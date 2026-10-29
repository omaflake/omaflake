// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of ImageMigration.swift – work package F1. Methods on `ImageStore`.
//!
//! Not 1:1: when the migration of a legacy NixOS image link fails, the folder just created is removed with
//! `remove_dir` (only an empty folder) – Swift's `removeItem` could also remove the original link when the very first
//! step (moving it aside) had failed.

use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde_json::Value;

use crate::data_folder::DataFolder;
use crate::error::{IoContext, Result};
use crate::image_store::{ImageInfo, ImageStore};
use crate::instance_store::InstanceStore;
use crate::json;
use crate::mac_vm::MacVmBundle;
use crate::models::{GuestKind, RecipeId};
use crate::omaflake_id::OmaflakeId;
use crate::packages::Package;
use crate::paths::{InstancePaths, Settings};
use crate::platform::resolve_symlinks;
use crate::qemu_runtime::QemuRuntime;
use crate::recipe_sources::{RecipeLibrary, RecipeRef};
use crate::shell::{RunOptions, Shell};
use crate::spawn::live_pid;
use crate::trash::Disposal;

/// Entry names of a folder (empty when it cannot be read), sorted – Swift's `contentsOfDirectory` order is unspecified
fn entries(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// The last part of a path as a String
fn last_component(path: &str) -> String {
    Path::new(path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// A recipe ID with the full 12 words of its content hash
fn is_full(id: &str) -> bool {
    RecipeId::words(id).is_some_and(|w| OmaflakeId::is_words(w, true))
}

/// qemu-img of QEMU (`QemuRuntime::locate`), else from PATH
fn qemu_img_of(store: &ImageStore) -> String {
    QemuRuntime::locate(&store.paths).map(|r| r.1).unwrap_or_else(|_| "qemu-img".into())
}

impl ImageStore {
    /// Older layouts (before image.json, 2026-10-17): NixOS images were symlinks into the Nix store and recipes were
    /// called "minimal", "mac-minimal", "nixos-minimal", … Turns them into image folders with image.json (recipe
    /// "omaflake.<kind>.<name>"), renames `current/` links and points instances at the new paths (qcow2 overlays are
    /// rebased). Then everything becomes a package (plan B108, `migrate_packages`). Idempotent and cheap when there is
    /// nothing to do; returns what it changed. Starts with `Settings::migrate_recipes_folder`.
    pub fn migrate_legacy(&self) -> Vec<String> {
        let paths = &self.paths;
        let mut notes = Vec::new();
        if let Some(flake) = Settings::migrate_recipes_folder(paths) {
            notes.push(format!("Image flake moved to {flake} (plan B92, B107)"));
        }
        for name in entries(&paths.images()) {
            if name == "current" || name.starts_with('.') || Package::kind_of(Path::new(&name)).is_some() {
                continue;
            }
            let entry = paths.images().join(&name);
            if Self::read_info(&entry).is_some() {
                continue;
            }
            let legacy = Self::recipe_of_image(&name);
            let revision = Self::legacy_revision(&name, &legacy);
            // When the old image was made (not when it was migrated); the link itself, not its store target
            let created: DateTime<Utc> = fs::symlink_metadata(&entry)
                .and_then(|m| m.created())
                .map(DateTime::<Utc>::from)
                .unwrap_or_else(|_| json::now());
            let dest = fs::read_link(&entry).ok().map(|d| d.to_string_lossy().into_owned());
            if let Some(dest) = dest.filter(|d| d.starts_with("/nix/store/")) {
                // NixOS: the symlink becomes a folder with result → store path (registered as a GC root again)
                let parked = paths.images().join(format!(".legacy-{name}"));
                let result = (|| -> Result<()> {
                    fs::rename(&entry, &parked).with_path(&entry)?;
                    fs::create_dir_all(&entry).with_path(&entry)?;
                    let out_link = entry.join("result").to_string_lossy().into_owned();
                    Shell::check("nix", &["build", &dest, "--out-link", &out_link], &RunOptions::default())?;
                    let info = ImageInfo::new(
                        &Self::legacy_recipe_id(&legacy, GuestKind::Nixos),
                        None,
                        GuestKind::Nixos,
                        "aarch64-linux",
                        "",
                        &revision,
                        Some(dest.clone()),
                        created,
                    );
                    Self::write_info(&info, &entry)
                })();
                match result {
                    Ok(()) => {
                        let _ = Disposal::delete(&[&parked]); // only the old symlink; the store path has its new GC root
                        notes.push(format!("image {name}: folder with image.json"));
                    }
                    Err(e) => {
                        // Put the old link back so nothing is lost
                        if !entry.join("result").exists() {
                            let _ = fs::remove_dir(&entry); // the empty folder just created
                            let _ = fs::rename(&parked, &entry);
                        }
                        notes.push(format!("image {name}: not migrated ({e})"));
                    }
                }
            } else if MacVmBundle::new(&entry).config().exists() {
                let info = ImageInfo::new(
                    &Self::legacy_recipe_id(&legacy, GuestKind::Macos),
                    None,
                    GuestKind::Macos,
                    "aarch64-darwin",
                    "",
                    &revision,
                    None,
                    created,
                );
                let _ = Self::write_info(&info, &entry);
                notes.push(format!("image {name}: image.json"));
            }
        }
        // current/<legacy recipe> → current/<recipe ID>
        for link in entries(&paths.current_images()) {
            if RecipeId::parts(&link).is_some() {
                continue;
            }
            let old = paths.current_images().join(&link);
            let Ok(dest) = fs::read_link(&old) else { continue };
            let dest = dest.to_string_lossy().into_owned();
            let Some(info) = Self::read_info(&paths.images().join(last_component(&dest)))
                .or_else(|| Self::read_info(&paths.image_dir(&Self::image_name_of_link(&dest))))
            else {
                continue;
            };
            let new = paths.current_images().join(&info.recipe);
            if new.exists() || fs::read_link(&new).is_ok() {
                let _ = Disposal::delete(&[&old]);
            } else {
                let _ = fs::rename(&old, &new);
            }
            notes.push(format!("current/{link} → {}", info.recipe));
        }
        let qemu_img = qemu_img_of(self);
        notes.extend(self.migrate_packages(&qemu_img));
        // Instances: recipe ID; an overlay that still names its image by an absolute path gets the relative one
        let store = InstanceStore::new(paths.clone(), &qemu_img);
        for mut c in store.list() {
            let mut changed = false;
            let mut note = None;
            if RecipeId::parts(&c.recipe).is_none() {
                c.recipe = Self::legacy_recipe_id(&c.recipe, c.guest_kind());
                changed = true;
                note = Some(format!("instance {}: {}", c.name, c.recipe));
            }
            let ip = paths.instance(&c.name);
            if c.guest_kind() == GuestKind::Nixos && ip.disk().exists() && !DataFolder::is_running(&c, paths) {
                let absolute = Self::backing_file(&ip.disk(), &qemu_img).is_some_and(|b| b.starts_with('/'));
                if let Some(image) = store.backing_image(&c).filter(|_| absolute) {
                    let relative = Package::relative_path(&ip.dir, &image);
                    let disk = ip.disk().to_string_lossy().into_owned();
                    let opts = RunOptions { cwd: Some(&ip.dir), ..Default::default() };
                    if Shell::check(&qemu_img, &["rebase", "-u", "-F", "qcow2", "-b", &relative, &disk], &opts).is_ok()
                    {
                        c.image_path = image.to_string_lossy().into_owned();
                        changed = true;
                        notes.push(format!("instance {}: overlay on {relative}", c.name));
                    }
                }
            }
            if changed
                && store.save(&c).is_ok()
                && let Some(note) = note
            {
                notes.push(note);
            }
        }
        notes.extend(self.migrate_recipe_ids(&store));
        notes
    }

    /// Plan B110/B111: recipe IDs carry their content hash as 12 words and images their recipe's source (the line).
    /// Downloaded recipes move to `recipes/<id>/`; images, `current/` links and VMs of older IDs (none, or fewer
    /// words) get the ID and source of the recipe there is now – the one whose words they start with, else the first of
    /// that base; an image built from an older state is then outdated by its store path, as before. Recipes that are
    /// gone stay as they are. VMs get an identity of 12 words (a shorter one is extended, so it starts the same).
    pub(crate) fn migrate_recipe_ids(&self, store: &InstanceStore) -> Vec<String> {
        let paths = &self.paths;
        let mut notes = Vec::new();
        let library = RecipeLibrary::new(paths.clone());
        // Downloaded and imported recipes: recipes/<author>/<kind>/<name>/ or recipes/<shorter ID>/ → recipes/<id>/
        let installed = library.installed();
        let mut manifest = installed.clone();
        for (old, entry) in &installed {
            if is_full(old) {
                continue;
            }
            let Some((author, kind, name)) = RecipeRef::parts(RecipeId::base(old)) else { continue };
            let folder: PathBuf = if RecipeId::words(old).is_none() {
                paths.recipes().join(format!("{author}/{}/{name}", kind.as_str()))
            } else {
                paths.recipes().join(old)
            };
            let Ok(hash) = RecipeLibrary::content_hash(&folder) else { continue };
            let id = RecipeId::make_with_hash(RecipeId::base(old), &hash, OmaflakeId::LENGTH);
            let Some(target) = library.folder(&id) else { continue };
            if !target.exists() && fs::rename(&folder, &target).is_err() {
                continue;
            }
            manifest.remove(old);
            manifest.insert(id.clone(), entry.clone());
            notes.push(format!("recipe {}: {}", RecipeId::qualified(old), OmaflakeId::recipe(&id, 3)));
        }
        if manifest != library.installed() {
            let _ = library.save_manifest(&manifest);
        }
        let available = library.available(&Settings::load(paths));
        // The recipe there is now for an ID: (id, source)
        let current = |recipe: &str| -> Option<(String, String)> {
            if let Some(e) = available.iter().find(|e| e.id == recipe) {
                return Some((e.id.clone(), e.source.clone()));
            }
            let base = RecipeId::base(recipe);
            let found = match RecipeId::words(recipe) {
                Some(words) => available.iter().find(|e| {
                    RecipeId::base(&e.id) == base && RecipeId::words(&e.id).is_some_and(|w| w.starts_with(words))
                }),
                None => available.iter().find(|e| RecipeId::base(&e.id) == base),
            };
            found.map(|e| (e.id.clone(), e.source.clone()))
        };
        // Images
        for image in self.list() {
            if image.info.recipe_source.is_some() && is_full(image.recipe()) {
                continue;
            }
            let Some((id, source)) = current(image.recipe()) else { continue };
            let mut info = image.info.clone();
            info.recipe = id.clone();
            info.recipe_source = Some(source);
            if Self::write_info(&info, &paths.image_dir(&image.name)).is_ok() {
                notes.push(format!("image {}: {}", image.name, OmaflakeId::recipe(&id, 3)));
            }
        }
        // current/<old ID> → current/<line>
        for link in entries(&paths.current_images()) {
            if link.contains('~') {
                continue;
            }
            let url = paths.current_images().join(&link);
            let Ok(dest) = fs::read_link(&url) else { continue };
            let name = Self::image_name_of_link(&dest.to_string_lossy());
            if self.image(&name).is_err() || self.set_current(&name).is_err() {
                continue;
            }
            let _ = fs::remove_file(&url);
        }
        // VMs: the recipe of their image, else the one there is now; an identity of 12 words
        for mut c in store.list() {
            let short_identity = c.identity.as_deref().map(|i| !OmaflakeId::is_words(i, true)).unwrap_or(true);
            if is_full(&c.recipe) && !short_identity {
                continue;
            }
            let before = c.clone();
            if !is_full(&c.recipe) {
                let from_image = self.image(&c.image).ok().map(|i| i.info.recipe).filter(|r| is_full(r));
                if let Some(id) = from_image.or_else(|| current(&c.recipe).map(|x| x.0)) {
                    c.recipe = id;
                }
            }
            // Swift's split leaves out empty parts
            let have: Vec<String> = c
                .identity
                .as_deref()
                .map(|i| i.split('-').filter(|w| !w.is_empty()).map(str::to_string).collect())
                .unwrap_or_default();
            if have.len() != OmaflakeId::LENGTH {
                let more = OmaflakeId::random_words();
                let words: Vec<String> = have
                    .into_iter()
                    .chain(more.split('-').filter(|w| !w.is_empty()).map(str::to_string))
                    .take(OmaflakeId::LENGTH)
                    .collect();
                c.identity = Some(words.join("-"));
            }
            if c != before && store.save(&c).is_ok() {
                notes.push(format!("instance {}: {}", c.name, OmaflakeId::vm(&c, 3)));
            }
        }
        notes.extend(self.rename_images(store));
        notes
    }

    /// Plan B110/B111: image packages are named `<base>@<12 words of the store path>` – older names are renamed; the VMs on
    /// them follow (their `image`, and NixOS overlays their relative backing file). An image a running VM uses waits.
    pub(crate) fn rename_images(&self, store: &InstanceStore) -> Vec<String> {
        let paths = &self.paths;
        let mut notes = Vec::new();
        for image in self.list() {
            if RecipeId::words(&image.name).is_some_and(|w| OmaflakeId::is_words(w, true)) {
                continue;
            }
            let new = Self::image_name(image.recipe(), image.info.store_path.as_deref().unwrap_or(&image.name));
            let users: Vec<_> = store.list().into_iter().filter(|c| c.image == image.name).collect();
            if paths.image_dir(&new).exists() || users.iter().any(|c| DataFolder::is_running(c, paths)) {
                continue;
            }
            let was_current = self.current_name_in_line(&image.info.line()).as_deref() == Some(image.name.as_str());
            if fs::rename(paths.image_dir(&image.name), paths.image_dir(&new)).is_err() {
                continue;
            }
            if was_current {
                let _ = self.set_current(&new);
            }
            let qemu_img = qemu_img_of(self);
            for mut c in users {
                c.image = new.clone();
                let ip = paths.instance(&c.name);
                let embedded = ip.embedded_image().join("disk.qcow2");
                if c.guest_kind() == GuestKind::Nixos && !embedded.exists() && ip.disk().exists() {
                    let backing = paths.image_dir(&new).join("disk.qcow2");
                    c.image_path = backing.to_string_lossy().into_owned();
                    let relative = Package::relative_path(&ip.dir, &backing);
                    let disk = ip.disk().to_string_lossy().into_owned();
                    let opts = RunOptions { cwd: Some(&ip.dir), ..Default::default() };
                    let _ = Shell::check(&qemu_img, &["rebase", "-u", "-F", "qcow2", "-b", &relative, &disk], &opts);
                } else if c.guest_kind() == GuestKind::Macos {
                    c.image_path = paths.image_dir(&new).to_string_lossy().into_owned();
                }
                let _ = store.save(&c);
            }
            notes.push(format!("image {} → {new}", image.name));
        }
        notes
    }

    /// The backing file a qcow2 names (as written in it: `qemu-img info -U --output=json`, "backing-filename"), None
    /// without one
    pub(crate) fn backing_file(disk: &Path, qemu_img: &str) -> Option<String> {
        let disk = disk.to_string_lossy().into_owned();
        let json = Shell::check(qemu_img, &["info", "-U", "--output=json", &disk], &RunOptions::default()).ok()?;
        let obj: Value = serde_json::from_str(&json).ok()?;
        obj.as_object()?.get("backing-filename")?.as_str().map(str::to_string)
    }

    /// Plan B108: image folders → `<name>.omaflake-image` (NixOS: the qcow2 is copied out of the Nix store, the GC
    /// root goes), `current/` links follow, instance folders → `<name>.omaflake-vm` with their runtime files moved to
    /// `run/<name>/`. Skipped as a whole while a VM of the old layout runs (its files are open) – next start then.
    pub(crate) fn migrate_packages(&self, qemu_img: &str) -> Vec<String> {
        let _ = qemu_img; // Swift takes it too, without using it
        let paths = &self.paths;
        let mut notes = Vec::new();
        let old_instances: Vec<PathBuf> = entries(&paths.instances())
            .into_iter()
            .filter(|n| !n.starts_with('.') && Package::kind_of(Path::new(n)).is_none())
            .map(|n| paths.instances().join(n))
            .filter(|d| d.join("instance.json").exists())
            .collect();
        let running: Vec<String> = old_instances
            .iter()
            .filter(|d| {
                let ip = InstancePaths::new(d.to_path_buf(), None);
                live_pid(&ip.pid_file(), "qemu").is_some() || live_pid(&ip.runner_pid(), "omaf-runner").is_some()
            })
            .map(|d| last_component(&d.to_string_lossy()))
            .collect();
        if !running.is_empty() {
            return vec![format!("packages: not yet – stop {} first", running.join(", "))];
        }
        // Images
        for name in entries(&paths.images()) {
            if name == "current" || name.starts_with('.') || Package::kind_of(Path::new(&name)).is_some() {
                continue;
            }
            let old = paths.images().join(&name);
            let Some(info) = Self::read_info(&old) else { continue };
            let new = paths.image_dir(&name);
            if new.exists() {
                continue;
            }
            let result = (|| -> Result<()> {
                if info.kind == GuestKind::Nixos {
                    let result = old.join("result");
                    Self::copy_disk(&resolve_symlinks(&result), &old)?;
                    fs::rename(&old, &new).with_path(&old)?;
                    let _ = Disposal::delete(&[new.join("result")]); // only the GC root
                } else {
                    fs::rename(&old, &new).with_path(&old)?;
                }
                Ok(())
            })();
            match result {
                Ok(()) => notes.push(format!("image {name}: package")),
                Err(e) => notes.push(format!("image {name}: not a package yet ({e})")),
            }
        }
        // current/<recipe> → ../<name>.omaflake-image
        for link in entries(&paths.current_images()) {
            let url = paths.current_images().join(&link);
            let Ok(dest) = fs::read_link(&url) else { continue };
            if Package::kind_of(&dest).is_some() {
                continue;
            }
            let name = last_component(&dest.to_string_lossy());
            if paths.image_dir(&name).exists() {
                let _ = fs::remove_file(&url);
                let _ = std::os::unix::fs::symlink(format!("../{name}.{}", Package::IMAGE), &url);
            }
        }
        // Instances
        for old in old_instances {
            let name = last_component(&old.to_string_lossy());
            let new = paths.instances().join(format!("{name}.{}", Package::VM));
            if new.exists() {
                continue;
            }
            let result = (|| -> Result<()> {
                let run = paths.run().join(&name);
                fs::create_dir_all(&run).with_path(&run)?;
                for file in InstancePaths::RUNTIME_NAMES {
                    let from = old.join(file);
                    let is_link = fs::read_link(&from).is_ok();
                    if !(is_link || from.exists()) {
                        continue;
                    }
                    let _ = Disposal::delete(&[run.join(file)]);
                    fs::rename(&from, run.join(file)).with_path(&from)?;
                }
                fs::rename(&old, &new).with_path(&old)?;
                Ok(())
            })();
            match result {
                Ok(()) => notes.push(format!("instance {name}: package")),
                Err(e) => notes.push(format!("instance {name}: not a package yet ({e})")),
            }
        }
        notes
    }

    /// "minimal", "nixos-minimal", "mac-minimal", "macos-minimal" → "omaflake.<kind>.minimal"; IDs stay as they are
    pub(crate) fn legacy_recipe_id(recipe: &str, kind: GuestKind) -> String {
        if RecipeId::parts(recipe).is_some() {
            return recipe.to_string();
        }
        let prefixes: &[&str] = if kind == GuestKind::Macos { &["macos-", "mac-"] } else { &["nixos-"] };
        let name = prefixes.iter().find_map(|p| recipe.strip_prefix(p)).unwrap_or(recipe);
        RecipeId::make("omaflake", kind, name)
    }

    /// "<recipe>-<rev>[-dirty]-<hash>" → "<rev>[-dirty]"; "unknown" when the name does not start with the recipe
    pub(crate) fn legacy_revision(name: &str, recipe: &str) -> String {
        let Some(rest) = name.strip_prefix(&format!("{recipe}-")) else { return "unknown".into() };
        // Swift's split leaves out empty parts
        let mut parts: Vec<&str> = rest.split('-').filter(|p| !p.is_empty()).collect();
        if parts.len() > 1 {
            parts.pop(); // store hash
        }
        parts.join("-")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::tests::config;
    use crate::paths::tests::temp_paths;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn legacy_names_map_to_omaflake_ids() {
        assert_eq!(ImageStore::legacy_recipe_id("minimal", GuestKind::Nixos), "omaflake.nixos.minimal");
        assert_eq!(ImageStore::legacy_recipe_id("nixos-minimal", GuestKind::Nixos), "omaflake.nixos.minimal");
        assert_eq!(ImageStore::legacy_recipe_id("mac-minimal", GuestKind::Macos), "omaflake.macos.minimal");
        assert_eq!(ImageStore::legacy_recipe_id("macos-minimal", GuestKind::Macos), "omaflake.macos.minimal");
        assert_eq!(ImageStore::legacy_recipe_id("alice.nixos.dev", GuestKind::Nixos), "alice.nixos.dev");
        assert_eq!(
            ImageStore::legacy_revision("mac-minimal-46c254685a-dirty-8ymcgbkl", "mac-minimal"),
            "46c254685a-dirty"
        );
        assert_eq!(ImageStore::legacy_revision("other-1-2", "mac-minimal"), "unknown");
    }

    /// Swift VZBackendTests.imageRecipeFromName
    #[test]
    fn image_recipe_from_name() {
        assert_eq!(ImageStore::recipe_of_image("mac-minimal-d7ad2da948-dirty-3c4r9p1j"), "mac-minimal");
        assert_eq!(ImageStore::recipe_of_image("minimal-d1f95739ba-55pvcxp5"), "minimal");
        assert_eq!(ImageStore::recipe_of_image("dev-nogit-abcdef12"), "dev");
    }

    /// A stand-in for a macOS VM folder (MacVmBundle's config.json; its content is not read here)
    fn mac_folder(dir: &Path) {
        fs::create_dir_all(dir).unwrap();
        fs::write(MacVmBundle::new(dir).config(), "{}").unwrap();
    }

    #[test]
    fn migrates_legacy_mac_image_current_link_and_instance() {
        let (_t, p) = temp_paths();
        let img = p.images().join("mac-minimal-46c254685a-dirty-8ymcgbkl");
        mac_folder(&img);
        fs::set_permissions(&img, fs::Permissions::from_mode(0o555)).unwrap();
        fs::create_dir_all(p.current_images()).unwrap();
        std::os::unix::fs::symlink(
            format!("../{}", img.file_name().unwrap().to_string_lossy()),
            p.current_images().join("mac-minimal"),
        )
        .unwrap();
        let mut c = config("m1");
        c.kind = GuestKind::Macos;
        c.recipe = "mac-minimal".into();
        fs::create_dir_all(p.instance("m1").dir).unwrap();
        InstanceStore::new(p.clone(), "/usr/bin/false").save(&c).unwrap();

        let store = ImageStore::new(p.clone());
        assert!(!store.migrate_legacy().is_empty());
        let current = store.current("omaflake.macos.minimal").unwrap();
        assert_eq!(current.info.revision, "46c254685a-dirty");
        // stays read-only (the folder is a package now)
        assert!(!crate::platform::is_writable(&current.path));
        assert_eq!(
            InstanceStore::new(p.clone(), "/usr/bin/false").load("m1").unwrap().recipe,
            "omaflake.macos.minimal"
        );
        assert!(store.migrate_legacy().is_empty()); // idempotent
        fs::set_permissions(&current.path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// Plan B108: folders of the old layout become packages, runtime files go to run/<name>/ (Swift
    /// PackageTests.oldLayoutBecomesPackages)
    #[test]
    fn old_layout_becomes_packages() {
        let (_t, p) = temp_paths();
        // NixOS image: folder with image.json + result → a folder with the qcow2 (stands in for the store path)
        let store = p.root.join("fake-store");
        fs::create_dir_all(&store).unwrap();
        fs::write(store.join("nixos.qcow2"), "qcow").unwrap();
        let nix = p.images().join("omaflake.nixos.lab-abc-12345678");
        fs::create_dir_all(&nix).unwrap();
        std::os::unix::fs::symlink(&store, nix.join("result")).unwrap();
        let info =
            ImageInfo::new("omaflake.nixos.lab", None, GuestKind::Nixos, "aarch64-linux", "", "abc", None, json::now());
        ImageStore::write_info(&info, &nix).unwrap();
        fs::create_dir_all(p.current_images()).unwrap();
        std::os::unix::fs::symlink("../omaflake.nixos.lab-abc-12345678", p.current_images().join("omaflake.nixos.lab"))
            .unwrap();
        // VM of the old layout with a runtime file
        let old = p.instances().join("t1");
        fs::create_dir_all(&old).unwrap();
        fs::write(old.join("instance.json"), "{}").unwrap();
        fs::write(old.join("qemu.log"), "log").unwrap();
        fs::write(old.join("disk.qcow2"), "disk").unwrap();

        let images = ImageStore::new(p.clone());
        let notes = images.migrate_packages("/usr/bin/false");
        assert_eq!(notes.len(), 2, "{notes:?}");
        let image = images.image("omaflake.nixos.lab-abc-12345678").unwrap();
        assert_eq!(image.path, p.image_dir(&image.name).join("disk.qcow2"));
        assert_eq!(fs::read(&image.path).unwrap(), b"qcow");
        assert!(fs::read_link(p.image_dir(&image.name).join("result")).is_err());
        assert_eq!(
            fs::read_link(p.current_images().join("omaflake.nixos.lab")).unwrap(),
            PathBuf::from(format!("../{}.omaflake-image", image.name))
        );
        let pkg = p.instances().join("t1.omaflake-vm");
        assert!(pkg.join("disk.qcow2").exists());
        assert!(!pkg.join("qemu.log").exists());
        assert!(p.run().join("t1/qemu.log").exists());
        // idempotent
        assert!(images.migrate_packages("/usr/bin/false").is_empty());
    }

    #[test]
    fn backing_file_from_qemu_img_info() {
        let (_t, p) = temp_paths();
        let fake = p.root.join("qemu-img.sh");
        fs::write(&fake, "#!/bin/sh\necho '{\"backing-filename\": \"/abs/disk.qcow2\", \"format\": \"qcow2\"}'\n")
            .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let disk = p.root.join("disk.qcow2");
        assert_eq!(ImageStore::backing_file(&disk, &fake.to_string_lossy()).as_deref(), Some("/abs/disk.qcow2"));
        assert_eq!(ImageStore::backing_file(&disk, "/usr/bin/false"), None);
    }
}
