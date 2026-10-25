// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of ImageStore.swift – work package F1.
//!
//! Images ("Vorlagen") under <data folder>/images: one package per image (`<name>.omaflake-image`, plan B108) with an
//! `image.json` that says what it is (recipe, author, guest kind, name, flake, revision, time). Nothing is derived
//! from folder names. Packages are self-contained, so they can be copied to a stick and back:
//!   NixOS: `image.json` + `disk.qcow2` (copied out of the Nix store, read-only)
//!   macOS: the VM folder (read-only) + `image.json`
//! `<name>` = `<author>.<kind>.<name>@<words>` (plan B110), the file form of its omaflake ID – the words come from the
//! store path, so the name is unique per content and several images of one recipe live side by side;
//! `current/<line>` points to the newest of a recipe's line.
use std::collections::{HashMap, HashSet};
use std::fs;
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::cancellation::Cancellation;
use crate::error::{codes, IoContext, Result};
use crate::file_format::{write_atomic, FileFormat, Step, UnreadableFile};
use crate::instance_store::InstanceStore;
use crate::ipsw_downloader::KeepAwake;
use crate::json;
use crate::models::{GuestKind, RecipeDefaults, RecipeId};
use crate::omaflake_id::OmaflakeId;
use crate::packages::{Package, PackageKind};
use crate::paths::{Paths, Settings};
use crate::platform::{is_writable, resolve_symlinks, Platform};
use crate::prerequisites::Prerequisites;
use crate::recipe_sources::RecipeLibrary;
use crate::shell::{RunOptions, Shell, ShellResult, StreamOptions};
use crate::spawn::Pid;
use crate::trash::Disposal;
use crate::vm_err;
use crate::Log;

/// image.json, schema `image` (plan B114). 0 → 1: "flavor" became "recipe" (plan B92), `system` added – every
/// image before it was built on a Mac with Apple silicon
fn image_v0_to_v1(o: &mut Map<String, Value>) -> Result<()> {
    FileFormat::rename_key("flavor", "recipe", o);
    let kind = o.get("kind").and_then(Value::as_str).and_then(GuestKind::parse).unwrap_or(GuestKind::Nixos);
    o.insert("system".into(), Value::String(Platform::legacy_system(kind).into()));
    Ok(())
}

/// 1 → 2 adds `look` (plan B135): nothing to convert – an image built before looks has none
fn image_v1_to_v2(_: &mut Map<String, Value>) -> Result<()> {
    Ok(())
}

static IMAGE_STEPS: [Step; 2] = [image_v0_to_v1, image_v1_to_v2];

/// Contents of `image.json` (Swift `ImageStore.Info`). Keys: recipe, recipeSource, author, kind, system, name,
/// source, revision, storePath, look, created (`.iso8601`); None fields left out.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImageInfo {
    /// Recipe ID "<author>.<kind>.<name>@<hash>" (plan B110; older images: without the hash)
    pub recipe: String,
    /// The recipe source it was built from (`RecipeSource::id`) – with the recipe's base its line (plan B110)
    #[serde(rename = "recipeSource", default, skip_serializing_if = "Option::is_none")]
    pub recipe_source: Option<String>,
    pub author: String,
    pub kind: GuestKind,
    /// What it was built for (`Platform`): "aarch64-linux", "x86_64-linux", "aarch64-darwin"
    pub system: String,
    pub name: String,
    /// Flake the image was built from (folder or URL)
    pub source: String,
    /// The recipe's state (plan B107): from a local folder its git revision ("-dirty" with uncommitted changes in
    /// the recipe's folder, "nogit" outside git), downloaded the start of its catalog version; images from before
    /// B107 carry the image flake's revision
    pub revision: String,
    /// NixOS: the store path of the image; macOS: the nix-darwin system it was activated with
    #[serde(rename = "storePath", default, skip_serializing_if = "Option::is_none")]
    pub store_path: Option<String>,
    /// The guest look it was built with (`omaflake.look`, plan B135); None: built before looks, or from a flake
    /// without them
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<String>,
    #[serde(with = "crate::json::iso8601")]
    pub created: DateTime<Utc>,
}

impl ImageInfo {
    /// image.json (see `image_v0_to_v1`, `image_v1_to_v2`)
    pub const FILE_FORMAT: FileFormat = FileFormat::new("image", "image.json", 0, &IMAGE_STEPS);

    /// Author and name come from the recipe ID ("local" and the ID itself when it has no parts); pass
    /// `json::now()` for Swift's default `created: Date()`
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        recipe: &str,
        recipe_source: Option<String>,
        kind: GuestKind,
        system: &str,
        source: &str,
        revision: &str,
        store_path: Option<String>,
        created: DateTime<Utc>,
    ) -> Self {
        let p = RecipeId::parts(recipe);
        ImageInfo {
            recipe: recipe.into(),
            recipe_source,
            author: p.as_ref().map(|p| p.author.clone()).unwrap_or_else(|| "local".into()),
            kind,
            system: system.into(),
            name: p.map(|p| p.name).unwrap_or_else(|| recipe.to_string()),
            source: source.into(),
            revision: revision.into(),
            store_path,
            look: None,
            created,
        }
    }

    /// The recipe's line: images of one line replace each other (current, outdated, upgrade, retention)
    pub fn line(&self) -> String {
        RecipeId::line_key(&self.recipe, self.recipe_source.as_deref().unwrap_or(""))
    }
}

/// An image (Swift `ImageStore.Image`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Image {
    pub name: String,
    pub info: ImageInfo,
    /// NixOS: the .qcow2 file; macOS: the VM folder (spec §5)
    pub path: PathBuf,
}

impl Image {
    pub fn recipe(&self) -> &str {
        &self.info.recipe
    }

    pub fn kind(&self) -> GuestKind {
        self.info.kind
    }
}

#[derive(Debug, Clone)]
pub struct ImageStore {
    pub paths: Paths,
}

impl ImageStore {
    /// Retention (plan B52): after a build, a recipe keeps its current image and the one before it
    pub const KEEP_PREVIOUS: usize = 1;

    pub fn new(paths: Paths) -> Self {
        ImageStore { paths }
    }

    /// `<dir>/image.json`
    pub fn info_file(dir: &Path) -> PathBuf {
        dir.join("image.json")
    }

    /// image.json, brought up to date (and written back) if it is older; None if there is none or it cannot be read
    pub fn read_info(dir: &Path) -> Option<ImageInfo> {
        Self::checked_info(dir).ok().flatten()
    }

    /// image.json as `read_info`, but with the reason when it cannot be read (too old, too new, damaged); None if there
    /// is none
    pub(crate) fn checked_info(dir: &Path) -> Result<Option<ImageInfo>> {
        let Ok(data) = fs::read(Self::info_file(dir)) else { return Ok(None) };
        let (info, migrated) = ImageInfo::FILE_FORMAT.decode::<ImageInfo>(&data)?;
        if migrated {
            let _ = Self::write_info(&info, dir);
        }
        Ok(Some(info))
    }

    /// Names of the image packages in the data folder (not `current/`, not hidden ones like `.build-*`), sorted
    fn package_names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.paths.images())
            .map(|d| {
                d.flatten()
                    .map(|e| e.file_name().to_string_lossy().into_owned())
                    .filter(|n| !n.starts_with('.'))
                    .filter_map(|n| Package::name_of(Path::new(&n), PackageKind::Image))
                    .collect()
            })
            .unwrap_or_default();
        names.sort();
        names
    }

    /// Image packages this omaflake cannot read – written by a much older or a newer omaflake, or damaged (plan B114)
    pub fn unreadable(&self) -> Vec<UnreadableFile> {
        self.package_names()
            .into_iter()
            .filter_map(|name| match Self::checked_info(&self.paths.image_dir(&name)) {
                Ok(_) => None,
                Err(e) => Some(UnreadableFile::new(name, e.to_string())),
            })
            .collect()
    }

    /// Writes image.json (`ImageInfo::FILE_FORMAT.encode`); a read-only (macOS) folder is opened for it (0755) and
    /// closed again (0555), the file then 0444.
    pub fn write_info(info: &ImageInfo, dir: &Path) -> Result<()> {
        let read_only = !is_writable(dir);
        if read_only {
            fs::set_permissions(dir, fs::Permissions::from_mode(0o755)).with_path(dir)?;
        }
        let result = (|| {
            let file = Self::info_file(dir);
            write_atomic(&file, &ImageInfo::FILE_FORMAT.encode(info)?)?;
            if read_only {
                fs::set_permissions(&file, fs::Permissions::from_mode(0o444)).with_path(&file)?;
            }
            Ok(())
        })();
        if read_only {
            let _ = fs::set_permissions(dir, fs::Permissions::from_mode(0o555));
        }
        result
    }

    /// Recipe defaults from the flake (`nix eval --json <flakeRef>#lib.recipeDefaults` + `guest_arguments`).
    pub fn recipe_defaults(&self, flake: &str) -> Result<HashMap<String, RecipeDefaults>> {
        let mut args =
            vec!["eval".to_string(), "--json".into(), format!("{}#lib.recipeDefaults", Self::flake_ref(flake))];
        args.extend(self.guest_arguments(flake));
        let json = Shell::check("nix", &args, &RunOptions::default())?;
        Ok(serde_json::from_str(&json)?)
    }

    /// The store path a build of `recipe` would produce now – evaluation only, nothing is built. NixOS: the disk image,
    /// macOS: the darwin system; both are what image.json records as `storePath` (plan B82).
    pub fn expected_store_path(&self, recipe: &str, kind: GuestKind, flake: &str) -> Result<String> {
        let attr = if kind == GuestKind::Macos {
            format!("darwinConfigurations.\"{recipe}\".system")
        } else {
            Self::image_attribute(recipe)?
        };
        let mut args = vec!["eval".to_string(), "--raw".into(), format!("{}#{attr}.outPath", Self::flake_ref(flake))];
        args.extend(self.guest_arguments(flake));
        Ok(Shell::check("nix", &args, &RunOptions::default())?.trim().to_string())
    }

    /// The look a build of `recipe` gets now (`omaflake.look` of its configuration with the guest values, plan B135)
    /// – evaluation only; None when the flake has no looks or the evaluation fails
    pub fn look_of(&self, recipe: &str, kind: GuestKind, flake: &str) -> Option<String> {
        let attr = if kind == GuestKind::Macos {
            format!("darwinConfigurations.\"{recipe}\"")
        } else {
            format!("nixosConfigurations.\"vm-{}-{recipe}\"", Platform::host_system(GuestKind::Nixos)?)
        };
        let mut args =
            vec!["eval".to_string(), "--raw".into(), format!("{}#{attr}.config.omaflake.look", Self::flake_ref(flake))];
        args.extend(self.guest_arguments(flake));
        let look = Shell::check("nix", &args, &RunOptions::default()).ok()?.trim().to_string();
        (!look.is_empty()).then_some(look)
    }

    /// What a build of each recipe (with an image) would give now – recipes that fail to evaluate are left out
    pub fn expected_store_paths(
        &self,
        recipes: &HashMap<String, RecipeDefaults>,
        flake: &str,
    ) -> HashMap<String, String> {
        let mut result = HashMap::new();
        for (id, d) in recipes {
            if self.images_of(id).is_empty() {
                continue;
            }
            if let Ok(path) = self.expected_store_path(id, d.kind, flake) {
                result.insert(id.clone(), path);
            }
        }
        result
    }

    /// Recipes whose current image no longer matches the recipe (its files, the modules it uses, nixpkgs, the guest
    /// settings): a build now would give another store path.
    pub fn outdated(&self, recipes: &HashMap<String, RecipeDefaults>, flake: &str) -> HashSet<String> {
        self.expected_store_paths(recipes, flake)
            .into_iter()
            .filter(|(id, path)| self.current(id).is_some_and(|i| i.info.store_path.as_deref() != Some(path.as_str())))
            .map(|(id, _)| id)
            .collect()
    }

    /// The flake output of a NixOS recipe's image for this host: `images.<system>."<recipe>"` (plan B114)
    pub(crate) fn image_attribute(recipe: &str) -> Result<String> {
        let Some(system) = Platform::host_system(GuestKind::Nixos) else {
            return Err(vm_err!(codes::UNSUPPORTED_SYSTEM, "NixOS guests do not run on this computer"));
        };
        Ok(format!("images.{system}.\"{recipe}\""))
    }

    /// `nix build <flake>#images.<system>.<recipe>`, copy the disk out of the store, switch `current/<line>`.
    /// The build log goes line by line to `log`; `started` hands over the nix process for cancelling; `cancel` stops
    /// it. A build is run a second time when the Linux builder collected garbage during the first ("running
    /// auto-GC", plan B79). Exit status 15 or a cancelled token: "Build cancelled".
    pub fn build(
        &self,
        recipe: &str,
        flake: &str,
        started: Option<&dyn Fn(Pid)>,
        cancel: &Cancellation,
        log: &Log,
    ) -> Result<Image> {
        // The Linux builder collects garbage when it runs low on space (min-free) – mid-build that has removed paths
        // the build still needed ("dependencies failed", "is not valid"); a second run then finds the space (plan B79)
        let attribute = Self::image_attribute(recipe)?;
        let gc_hit = AtomicBool::new(false);
        let mut args = vec![
            "build".to_string(),
            format!("{}#{attribute}", Self::flake_ref(flake)),
            "--no-link".into(),
            "--print-out-paths".into(),
            "-L".into(),
        ];
        args.extend(self.guest_arguments(flake));
        let mut r = ShellResult { status: -1, ..Default::default() };
        for run in 1..=2 {
            if run == 2 {
                if r.status == 0 || !gc_hit.load(Ordering::SeqCst) || cancel.is_cancelled() {
                    break;
                }
                log("The Linux builder cleaned up during the build – building again");
            }
            let opts = StreamOptions { started, cancel: Some(cancel), ..Default::default() };
            r = KeepAwake::during("Building a NixOS image", || {
                Shell::stream("nix", &args, &opts, &|line: &str| {
                    if line.contains("running auto-GC") {
                        gc_hit.store(true, Ordering::SeqCst);
                    }
                    log(line)
                })
            })?;
        }
        if r.status != 0 {
            let cancelled = r.status == 15 || cancel.is_cancelled();
            return Err(if cancelled {
                vm_err!(codes::CANCELLED, "Build cancelled")
            } else {
                vm_err!("nix build failed ({}) – see log", r.status)
            });
        }
        // Swift's split leaves out empty lines
        let Some(store_path) = r.stdout.split('\n').rfind(|l| !l.is_empty()).map(str::to_string) else {
            return Err(vm_err!("nix build returned no store path"));
        };
        if !store_path.starts_with("/nix/store/") {
            return Err(vm_err!("nix build returned no store path"));
        }
        let rev = self.recipe_revision(recipe, flake);
        let name = Self::image_name(recipe, &store_path);
        let library = RecipeLibrary::new(self.paths.clone());
        let recipe_source =
            library.available(&Settings::load(&self.paths)).into_iter().find(|e| e.id == recipe).map(|e| e.source);
        let dir = self.paths.image_dir(&name);
        fs::create_dir_all(&dir).with_path(&dir)?;
        if Self::read_info(&dir).is_none() {
            log("Copying the image out of the Nix store");
            Self::copy_disk(Path::new(&store_path), &dir)?;
            let mut info = ImageInfo::new(
                recipe,
                recipe_source,
                GuestKind::Nixos,
                Platform::host_system(GuestKind::Nixos).unwrap_or(""),
                flake,
                &rev,
                Some(store_path.clone()),
                json::now(),
            );
            info.look = self.look_of(recipe, GuestKind::Nixos, flake);
            Self::write_info(&info, &dir)?;
        }
        self.set_current(&name)?;
        // The recipe version it is built from stays (plan B111); older ones may go now
        library.keep(recipe, &Settings::load(&self.paths));
        library.prune_versions();
        self.image(&name)
    }

    /// The qcow2 of a NixOS store path → `<package>/disk.qcow2` (read-only). The package then needs no GC root and
    /// no Nix store – it can be copied anywhere (plan B108). Written under a temporary name first, so an interrupted
    /// copy never looks like an image.
    pub(crate) fn copy_disk(from_store: &Path, into: &Path) -> Result<()> {
        let mut files: Vec<String> = fs::read_dir(from_store)
            .map(|d| d.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        files.sort();
        let Some(file) = files.into_iter().find(|f| f.ends_with(".qcow2")) else {
            return Err(vm_err!("No .qcow2 in {}", from_store.display()));
        };
        let disk = into.join("disk.qcow2");
        let partial = into.join(".disk.qcow2.partial");
        if fs::symlink_metadata(&partial).is_ok() {
            fs::remove_file(&partial).with_path(&partial)?;
        }
        Package::copy_item(&from_store.join(&file), &partial)?;
        fs::set_permissions(&partial, fs::Permissions::from_mode(0o444)).with_path(&partial)?;
        fs::rename(&partial, &disk).with_path(&disk)?;
        Ok(())
    }

    /// Flake reference for `nix`: a folder without its own git repository (like `recipes/` in the
    /// checkout) becomes `path:<folder>`, so Nix also sees its git-ignored `local/` settings.
    /// A git reference would copy only tracked files. Flake URLs (`path:`, `github:`, …) stay as they are.
    pub fn flake_ref(flake: &str) -> String {
        if !flake.starts_with('/') {
            return flake.to_string();
        }
        if Path::new(flake).join(".git").exists() {
            flake.to_string()
        } else {
            format!("path:{flake}")
        }
    }

    /// "omaflake.nixos.minimal" + a store path → "omaflake.nixos.minimal@cedar-mango-lift-…" (12 words, plan B111; the
    /// revision is in image.json)
    pub fn image_name(recipe: &str, store_path: &str) -> String {
        format!("{}@{}", RecipeId::base(recipe), OmaflakeId::words_of(store_path, OmaflakeId::LENGTH))
    }

    /// The state of a recipe for the image's name and image.json: where the recipe comes from decides (plan B107);
    /// unknown recipes fall back to the image flake's revision
    pub fn recipe_revision(&self, recipe: &str, flake: &str) -> String {
        let library = RecipeLibrary::new(self.paths.clone());
        if let Some(entry) = library.available(&Settings::load(&self.paths)).into_iter().find(|e| e.id == recipe) {
            if let Some(installed) = library.installed().get(recipe) {
                if installed.source == entry.source {
                    return installed.version.chars().take(10).collect();
                }
            }
            return self.git_rev(&entry.folder.to_string_lossy());
        }
        self.git_rev(flake)
    }

    /// Short git hash of the folder's repository, "-dirty" with uncommitted changes in that folder, "nogit" without
    /// git (never /usr/bin/git without developer tools: `Prerequisites::locate_git`)
    pub(crate) fn git_rev(&self, folder: &str) -> String {
        let dir = PathBuf::from(folder.strip_prefix("path:").unwrap_or(folder));
        // Never /usr/bin/git without developer tools: it would ask to install them (Prerequisites::locate_git)
        let Some(git) = Prerequisites::locate_git() else { return "nogit".into() };
        let opts = RunOptions { cwd: Some(&dir), ..Default::default() };
        let Ok(rev) = Shell::check(&git, &["rev-parse", "--short=10", "HEAD"], &opts) else {
            return "nogit".into();
        };
        let rev = rev.trim();
        let dirty = Shell::check(&git, &["status", "--porcelain", "--", "."], &opts).is_ok_and(|s| !s.is_empty());
        if dirty {
            format!("{rev}-dirty")
        } else {
            rev.to_string()
        }
    }

    /// The line of a recipe ID (plan B110): from the recipe there is, else from an image built from it
    pub fn line_of(&self, recipe: &str) -> String {
        let library = RecipeLibrary::new(self.paths.clone());
        if let Some(entry) = library.available(&Settings::load(&self.paths)).into_iter().find(|e| e.id == recipe) {
            return RecipeId::line_key(recipe, &entry.source);
        }
        self.list()
            .into_iter()
            .find(|i| i.recipe() == recipe)
            .map(|i| i.info.line())
            .unwrap_or_else(|| RecipeId::line_key(recipe, ""))
    }

    /// Points `current/<line>` to an image – the newest of its recipe's line, used for new VMs (plan B56, B110).
    /// The link is relative: `../<name>.omaflake-image`.
    pub fn set_current(&self, image_name: &str) -> Result<()> {
        let image = self.image(image_name)?;
        let current = self.paths.current_images();
        fs::create_dir_all(&current).with_path(&current)?;
        let link = current.join(image.info.line());
        // Replacing the symlink loses no data: the target stays under images/
        if fs::read_link(&link).is_ok() {
            fs::remove_file(&link).with_path(&link)?;
        }
        std::os::unix::fs::symlink(format!("../{image_name}.{}", Package::IMAGE), &link).with_path(&link)
    }

    /// Name of the current image of a recipe's line, of any guest kind (macOS images are VM folders, not qcow2).
    pub fn current_name(&self, recipe: &str) -> Option<String> {
        self.current_name_in_line(&self.line_of(recipe))
    }

    /// Swift `currentName(line:)`
    pub fn current_name_in_line(&self, line: &str) -> Option<String> {
        let dest = fs::read_link(self.paths.current_images().join(line)).ok()?;
        Some(Self::image_name_of_link(&dest.to_string_lossy()))
    }

    /// Link target "../<name>.omaflake-image" → "<name>" (links from before B108 have no extension)
    pub(crate) fn image_name_of_link(dest: &str) -> String {
        let path = Path::new(dest);
        Package::name_of(path, PackageKind::Image)
            .unwrap_or_else(|| path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default())
    }

    /// Current image of a recipe (None if never built).
    pub fn current(&self, recipe: &str) -> Option<Image> {
        self.current_name(recipe).and_then(|n| self.image(&n).ok())
    }

    pub fn image(&self, name: &str) -> Result<Image> {
        self.image_at(&self.paths.image_dir(name))
    }

    /// An image package anywhere (in the data folder, inside a VM package, on a stick). Swift `image(at:)`.
    pub fn image_at(&self, dir: &Path) -> Result<Image> {
        let name = Package::name_of(dir, PackageKind::Image)
            .unwrap_or_else(|| dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default());
        let Some(info) = Self::read_info(dir) else {
            return Err(vm_err!(codes::NOT_FOUND, "Image {name} has no image.json ({})", dir.display()));
        };
        if info.kind == GuestKind::Macos {
            return Ok(Image { name, info, path: dir.to_path_buf() });
        }
        let disk = dir.join("disk.qcow2");
        if !disk.exists() {
            return Err(vm_err!(codes::NOT_FOUND, "Image {name}: no disk.qcow2 in {} – build it again", dir.display()));
        }
        Ok(Image { name, info, path: disk })
    }

    /// Legacy folder names `<recipe>-<git-rev>[-dirty]-<hash>` → recipe part (used only to migrate old images).
    pub(crate) fn recipe_of_image(name: &str) -> String {
        let mut parts: Vec<&str> = name.split('-').collect();
        if parts.len() < 3 {
            return name.to_string();
        }
        parts.pop(); // store hash
        if parts.last() == Some(&"dirty") {
            parts.pop();
        }
        parts.pop(); // git rev or "nogit"
        parts.join("-")
    }

    /// Space the image takes on disk: allocated blocks (sparse disks count what they use; `st_blocks × 512`),
    /// symlinks followed. None when it can't be read.
    pub fn disk_usage(image: &Image) -> Option<i64> {
        let path = resolve_symlinks(&image.path);
        let meta = fs::metadata(&path).ok()?;
        if !meta.is_dir() {
            return Some(meta.blocks() as i64 * 512);
        }
        // Like FileManager's enumerator: into folders, not through links; regular files only
        fn walk(dir: &Path) -> i64 {
            let Ok(entries) = fs::read_dir(dir) else { return 0 };
            entries
                .flatten()
                .map(|e| match fs::symlink_metadata(e.path()) {
                    Ok(m) if m.is_dir() => walk(&e.path()),
                    Ok(m) if m.is_file() => m.blocks() as i64 * 512,
                    _ => 0,
                })
                .sum()
        }
        Some(walk(&path))
    }

    /// Images of a recipe's line, newest first. Swift `images(of:)`.
    pub fn images_of(&self, recipe: &str) -> Vec<Image> {
        self.images_in_line(&self.line_of(recipe))
    }

    /// Swift `images(inLine:)`, newest first
    pub fn images_in_line(&self, line: &str) -> Vec<Image> {
        let mut images: Vec<Image> = self.list().into_iter().filter(|i| i.info.line() == line).collect();
        images.sort_by_key(|a| std::cmp::Reverse(a.info.created));
        images
    }

    /// Deletes an image (NixOS: its GC root goes with it, the store path can then be collected). Refused while
    /// instances use it: NixOS overlays are built on it, macOS instances clone it again on reset (ephemeral ones on
    /// every start). If it was the recipe's current image, `current/` moves to the next newest image of that recipe.
    pub fn remove(&self, name: &str, instances: &InstanceStore) -> Result<()> {
        let image = self.image(name)?;
        let mut users: Vec<String> = instances.list().into_iter().filter(|c| c.image == name).map(|c| c.name).collect();
        users.sort();
        if !users.is_empty() {
            return Err(vm_err!(
                codes::IN_USE,
                "{name} is used by {} – remove or upgrade them first",
                users.join(", ")
            ));
        }
        let line = image.info.line();
        let was_current = self.current_name_in_line(&line).as_deref() == Some(name);
        Disposal::delete(&[self.paths.image_dir(name)])?;
        let result = (|| {
            if was_current {
                let link = self.paths.current_images().join(&line);
                if let Some(next) = self.images_in_line(&line).first() {
                    self.set_current(&next.name)?;
                } else {
                    let _ = Disposal::delete(&[link]);
                }
            }
            Ok(())
        })();
        // its recipe version may go now (plan B111) – Swift `defer`
        RecipeLibrary::new(self.paths.clone()).prune_versions();
        result
    }

    /// Retention (plan B52): after a build, a recipe keeps its current image, the one before it (to go back to) and
    /// every image a VM still uses; older ones are deleted. `used` must list the images of *all* instances – callers
    /// skip pruning when they cannot read them. Returns the deleted names.
    pub fn prune(&self, recipe: &str, used: &HashSet<String>) -> Vec<String> {
        let line = self.line_of(recipe);
        let current = self.current_name_in_line(&line);
        let older: Vec<Image> =
            self.images_in_line(&line).into_iter().filter(|i| Some(&i.name) != current.as_ref()).collect();
        let mut removed = Vec::new();
        for image in older.into_iter().skip(Self::KEEP_PREVIOUS).filter(|i| !used.contains(&i.name)) {
            if Disposal::delete(&[self.paths.image_dir(&image.name)]).is_ok() {
                removed.push(image.name);
            }
        }
        if !removed.is_empty() {
            RecipeLibrary::new(self.paths.clone()).prune_versions();
        }
        removed
    }

    /// All image packages with an image.json (not `current/` and not unfinished macOS builds `.build-*`), by name.
    pub fn list(&self) -> Vec<Image> {
        self.package_names().iter().filter_map(|n| self.image(n).ok()).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::tests::config;
    use crate::paths::tests::temp_paths;
    use crate::platform::is_writable;

    fn info(recipe: &str, kind: GuestKind, created: DateTime<Utc>) -> ImageInfo {
        let system = if kind == GuestKind::Macos { "aarch64-darwin" } else { "aarch64-linux" };
        ImageInfo::new(recipe, None, kind, system, "/f", "r", Some("/nix/store/x".into()), created)
    }

    fn at(seconds: i64) -> DateTime<Utc> {
        DateTime::from_timestamp(seconds, 0).unwrap()
    }

    /// A macOS image package (a stand-in VM folder, read-only like a real one)
    fn mac_image(p: &Paths, name: &str, recipe: &str, created: DateTime<Utc>) {
        let dir = p.image_dir(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), "{}").unwrap();
        ImageStore::write_info(&info(recipe, GuestKind::Macos, created), &dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
    }

    /// Read-only folders opened again, so the temporary folder can go
    fn open_up(p: &Paths) {
        for name in ImageStore::new(p.clone()).package_names() {
            let _ = fs::set_permissions(p.image_dir(&name), fs::Permissions::from_mode(0o755));
        }
    }

    #[test]
    fn image_name_is_unique_per_store_path() {
        let a = "/nix/store/55pvcxp567f41g0yxxf7blrqicjc3afm-nixos-disk-image";
        let n = ImageStore::image_name("omaflake.nixos.dev@able-acid-also", a);
        assert_eq!(n, format!("omaflake.nixos.dev@{}", OmaflakeId::words_of(a, OmaflakeId::LENGTH)));
        let words = n.split('@').nth(1).unwrap();
        assert!(OmaflakeId::is_words(words, true));
        assert_ne!(ImageStore::image_name("omaflake.nixos.dev", &format!("{a}x")), n);
        assert_eq!(words.split('-').count(), 12);
    }

    #[test]
    fn flake_ref_uses_path_for_folders_without_own_repo() {
        let (_t, p) = temp_paths();
        let dir = p.root.to_string_lossy().into_owned();
        assert_eq!(ImageStore::flake_ref(&dir), format!("path:{dir}"));
        fs::create_dir_all(p.root.join(".git")).unwrap();
        assert_eq!(ImageStore::flake_ref(&dir), dir);
        assert_eq!(ImageStore::flake_ref("github:omaflake/omaflake"), "github:omaflake/omaflake");
        assert_eq!(ImageStore::flake_ref("path:/x"), "path:/x");
    }

    #[test]
    fn disk_usage_follows_symlinks_and_sums_folders() {
        let (_t, p) = temp_paths();
        let dir = p.root.clone();
        let file = dir.join("disk.img");
        fs::write(&file, vec![1u8; 64 * 1024]).unwrap();
        let link = dir.join("link.qcow2");
        std::os::unix::fs::symlink(&file, &link).unwrap();
        let i = info("omaflake.nixos.minimal", GuestKind::Nixos, at(0));
        let via_link = ImageStore::disk_usage(&Image { name: "a".into(), info: i.clone(), path: link }).unwrap();
        assert!(via_link >= 64 * 1024);
        let folder = ImageStore::disk_usage(&Image { name: "b".into(), info: i.clone(), path: dir.clone() }).unwrap();
        assert!(folder >= via_link);
        assert_eq!(ImageStore::disk_usage(&Image { name: "c".into(), info: i, path: dir.join("gone") }), None);
    }

    /// Swift VZBackendTests.macImageIsAVMFolder
    #[test]
    fn mac_image_is_a_vm_folder() {
        let (_t, p) = temp_paths();
        let name = "omaflake.macos.minimal-abc1234567-dirty-12345678";
        let dir = p.image_dir(name);
        assert_eq!(dir.file_name().unwrap().to_string_lossy(), format!("{name}.omaflake-image"));
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), "{}").unwrap();
        // without image.json it is not an image
        assert!(ImageStore::new(p.clone()).image(name).is_err());
        let mut i = info("omaflake.macos.minimal", GuestKind::Macos, json::now());
        i.revision = "abc1234567-dirty".into();
        ImageStore::write_info(&i, &dir).unwrap();
        let image = ImageStore::new(p.clone()).image(name).unwrap();
        assert!(image.kind() == GuestKind::Macos && image.recipe() == "omaflake.macos.minimal" && image.path == dir);
        assert!(image.info.author == "omaflake" && image.info.name == "minimal");
        assert_eq!(ImageStore::new(p).list().iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), [name]);
    }

    #[test]
    fn nixos_image_needs_its_disk() {
        let (_t, p) = temp_paths();
        let dir = p.image_dir("omaflake.nixos.minimal@a");
        fs::create_dir_all(&dir).unwrap();
        ImageStore::write_info(&info("omaflake.nixos.minimal", GuestKind::Nixos, at(0)), &dir).unwrap();
        let store = ImageStore::new(p.clone());
        assert_eq!(store.image("omaflake.nixos.minimal@a").unwrap_err().code(), Some(codes::NOT_FOUND));
        // the disk comes out of a (stand-in) store path, read-only
        let nix = p.root.join("store");
        fs::create_dir_all(&nix).unwrap();
        fs::write(nix.join("nixos.qcow2"), "qcow").unwrap();
        ImageStore::copy_disk(&nix, &dir).unwrap();
        let image = store.image("omaflake.nixos.minimal@a").unwrap();
        assert_eq!(image.path, dir.join("disk.qcow2"));
        assert!(!is_writable(&image.path));
        assert!(ImageStore::copy_disk(&p.root.join("nothing"), &dir).is_err());
    }

    #[test]
    fn write_info_opens_and_closes_read_only_folders() {
        let (_t, p) = temp_paths();
        mac_image(&p, "m@a", "omaflake.macos.minimal", at(1));
        let dir = p.image_dir("m@a");
        assert!(!is_writable(&dir));
        let mut i = ImageStore::read_info(&dir).unwrap();
        i.revision = "changed".into();
        ImageStore::write_info(&i, &dir).unwrap();
        assert_eq!(ImageStore::read_info(&dir).unwrap().revision, "changed");
        assert!(!is_writable(&dir) && !is_writable(&ImageStore::info_file(&dir)));
        open_up(&p);
    }

    /// `current/<line>` is a relative link; the newest of a line comes first
    #[test]
    fn current_link_and_lines() {
        let (_t, p) = temp_paths();
        let store = ImageStore::new(p.clone());
        mac_image(&p, "x@old", "omaflake.macos.x", at(1));
        mac_image(&p, "x@new", "omaflake.macos.x", at(2));
        mac_image(&p, "y@one", "omaflake.macos.y", at(3));
        let line = info("omaflake.macos.x", GuestKind::Macos, at(0)).line();
        assert_eq!(store.current_name_in_line(&line), None);
        store.set_current("x@old").unwrap();
        store.set_current("x@new").unwrap();
        assert_eq!(fs::read_link(p.current_images().join(&line)).unwrap(), PathBuf::from("../x@new.omaflake-image"));
        assert_eq!(store.current_name_in_line(&line).as_deref(), Some("x@new"));
        let names: Vec<String> = store.images_in_line(&line).into_iter().map(|i| i.name).collect();
        assert_eq!(names, ["x@new", "x@old"]);
        assert!(store.set_current("missing").is_err());
        assert_eq!(ImageStore::image_name_of_link("../a@b.omaflake-image"), "a@b");
        assert_eq!(ImageStore::image_name_of_link("../minimal-abc-123"), "minimal-abc-123");
        open_up(&p);
    }

    #[test]
    fn removing_an_image_is_refused_while_used_and_moves_current() {
        let (_t, p) = temp_paths();
        let store = ImageStore::new(p.clone());
        mac_image(&p, "old", "omaflake.macos.minimal", at(1));
        mac_image(&p, "new", "omaflake.macos.minimal", at(2));
        store.set_current("new").unwrap();
        let instances = InstanceStore::new(p.clone(), "/usr/bin/false");
        let mut c = config("m1");
        c.image = "new".into();
        fs::create_dir_all(p.instance("m1").dir).unwrap();
        instances.save(&c).unwrap();
        assert_eq!(store.remove("new", &instances).unwrap_err().code(), Some(codes::IN_USE));
        instances.remove("m1").unwrap();
        store.remove("new", &instances).unwrap();
        assert_eq!(store.current_name("omaflake.macos.minimal").as_deref(), Some("old"));
        store.remove("old", &instances).unwrap();
        assert!(store.current_name("omaflake.macos.minimal").is_none() && store.list().is_empty());
    }

    /// Swift RetentionTests: current + one before + used ones stay; the rest goes
    #[test]
    fn prune_keeps_current_previous_and_used() {
        let (_t, p) = temp_paths();
        let store = ImageStore::new(p.clone());
        for (i, name) in ["a1", "a2", "a3", "a4", "a5"].iter().enumerate() {
            let dir = p.image_dir(name);
            fs::create_dir_all(&dir).unwrap();
            ImageStore::write_info(&info("omaflake.macos.x", GuestKind::Macos, at(1000 + i as i64)), &dir).unwrap();
        }
        store.set_current("a5").unwrap();
        let removed: HashSet<String> =
            store.prune("omaflake.macos.x", &HashSet::from(["a2".to_string()])).into_iter().collect();
        assert_eq!(removed, HashSet::from(["a1".to_string(), "a3".to_string()]));
        let left: HashSet<String> = store.images_of("omaflake.macos.x").into_iter().map(|i| i.name).collect();
        assert_eq!(left, HashSet::from(["a5".to_string(), "a4".to_string(), "a2".to_string()]));
    }

    /// Swift FileFormatTests.imageVersionZeroIsMigratedAndWrittenBack (with the shallow schema check)
    #[test]
    fn image_version_zero_is_migrated_and_written_back() {
        let (_t, p) = temp_paths();
        let dir = p.image_dir("omaflake.macos.minimal@a");
        fs::create_dir_all(&dir).unwrap();
        let old = r#"{"flavor":"omaflake.macos.minimal","author":"omaflake","kind":"macos","name":"minimal","source":"","revision":"r","created":"2026-10-01T00:00:00Z"}"#;
        fs::write(dir.join("image.json"), old).unwrap();
        let i = ImageStore::read_info(&dir).unwrap();
        assert!(i.recipe == "omaflake.macos.minimal" && i.system == "aarch64-darwin");
        let written = fs::read_to_string(dir.join("image.json")).unwrap();
        assert!(written.contains("https://omaflake.dev/schema/image/v2.json") && !written.contains("flavor"));
        // the keys are the schema's, its required ones are there
        let schema_file = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/schema/image/v2.json");
        let schema: Value = serde_json::from_slice(&fs::read(schema_file).unwrap()).unwrap();
        let object: Map<String, Value> = serde_json::from_str(&written).unwrap();
        let properties = schema["properties"].as_object().unwrap();
        assert!(object.keys().all(|k| properties.contains_key(k)), "{written}");
        let required = schema["required"].as_array().unwrap();
        assert!(required.iter().all(|r| object.contains_key(r.as_str().unwrap())), "{written}");
    }

    #[test]
    fn unreadable_images_are_listed() {
        let (_t, p) = temp_paths();
        let dir = p.image_dir("omaflake.nixos.minimal@new");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("image.json"), r#"{"$schema":"https://omaflake.dev/schema/image/v9.json"}"#).unwrap();
        let images = ImageStore::new(p);
        assert!(images.list().is_empty());
        let u = images.unreadable();
        assert_eq!(u.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["omaflake.nixos.minimal@new"]);
    }
}
