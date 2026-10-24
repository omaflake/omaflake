// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Transfer.swift – work package F1.
//!
//! Export and import of packages (spec §11.10, plan B108/B109): a VM, an image or recipes leave the data folder as a
//! package (to an external disk, a NAS, a folder) and come back from one. An import is always a copy into the data
//! folder – the source is never changed; an export changes nothing in the data folder. A VM can also run from where its
//! package is (`use_external`).
//!
//! Who a package is for: a VM carries its user, SSH keys and data, so it goes back to the same person (a backup,
//! another Mac of theirs); an image carries the user it was built for. What people share are recipes.
//!
//! Not 1:1: `is_on_network_volume` used `URLResourceKey.volumeIsLocalKey`; Rust uses `statfs` (macOS: `MNT_LOCAL`,
//! Linux: the file system type, see there). Swift wrote an imported VM's instance.json without `$schema` (migrated
//! again on the next load); the port writes it with `InstanceConfig::FILE_FORMAT`. The entries of an imported VM
//! package are copied in sorted order.
use std::fs;
use std::path::{Path, PathBuf};

use crate::backend::Backends;
use crate::data_folder::DataFolder;
use crate::error::{codes, IoContext, Result};
use crate::file_format::write_atomic;
use crate::image_store::ImageStore;
use crate::instance_store::InstanceStore;
use crate::models::{GuestKind, InstanceConfig};
use crate::omaflake_id::OmaflakeId;
use crate::packages::{Manifest, Package, PackageKind};
use crate::paths::Paths;
use crate::platform::{standardize, Platform};
use crate::recipe_sources::RecipeLibrary;
use crate::shell::{RunOptions, Shell};
use crate::sockets::Ports;
use crate::trash::Disposal;
use crate::vm_err;
use crate::Log;

#[derive(Debug, Clone)]
pub struct Transfer {
    pub paths: Paths,
    pub qemu_img: String,
    /// omaf-runner, for a macOS VM imported as a copy (new machine identifier and MAC); None without one
    pub runner: Option<String>,
}

/// What an import brought (Swift `Transfer.Imported`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub kind: PackageKind,
    /// VM or image name; recipe IDs for recipes
    pub names: Vec<String>,
    pub notes: Vec<String>,
}

/// What is here already for a package – checked before an import, so the app or the CLI can ask first
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Conflict {
    None,
    /// A VM of that name is here: cancel, or import under another name (a second VM with its own identity)
    VmName(String),
    /// The same image is here (equal names mean equal content): cancel or replace it
    Image(String),
    // Recipes have none (plan B110): the same content is here already, other content is a recipe of its own
}

impl Transfer {
    /// Name of the image package a VM package carries: "image.omaflake-image"
    pub const EMBEDDED_IMAGE_NAME: &'static str = "image.omaflake-image";

    pub fn new(paths: Paths, qemu_img: &str, runner: Option<String>) -> Self {
        Transfer { paths, qemu_img: qemu_img.into(), runner }
    }

    /// From the backends: their store's qemu-img and the runner when macOS guests work
    pub fn from_backends(backends: &Backends) -> Self {
        Transfer::new(backends.paths.clone(), &backends.store.qemu_img, backends.vz().ok().map(|v| v.runner.clone()))
    }

    pub(crate) fn store(&self) -> InstanceStore {
        InstanceStore::new(self.paths.clone(), &self.qemu_img)
    }

    pub(crate) fn images(&self) -> ImageStore {
        ImageStore::new(self.paths.clone())
    }

    // MARK: - Export

    /// Whether a VM's package should carry its image: NixOS always (the overlay is built on it), macOS when it
    /// needs the image to start (ephemeral) – otherwise only for a reset, which `with_image` asks for
    pub fn carries_image(c: &InstanceConfig, with_image: bool) -> bool {
        c.guest_kind() == GuestKind::Nixos || c.ephemeral || with_image
    }

    /// A stopped VM → `<folder>/<name>.omaflake-vm` (a snapshot: a running VM would change while it is copied),
    /// via `.<name>.omaflake-vm.partial`.
    pub fn export_vm(&self, name: &str, folder: &Path, with_image: bool, log: &Log) -> Result<PathBuf> {
        let c = self.store().load(name)?;
        if DataFolder::is_running(&c, &self.paths) {
            return Err(vm_err!(
                codes::VM_RUNNING,
                "{name} is running – shut it down first: an export is a snapshot of a stopped VM"
            ));
        }
        let ip = self.paths.instance(name);
        let dest = folder.join(format!("{name}.{}", Package::VM));
        if dest.exists() {
            return Err(vm_err!(codes::EXISTS, "{} exists already", dest.display()));
        }
        let partial = folder.join(format!(".{name}.{}.partial", Package::VM));
        Disposal::delete(&[&partial])?;
        let result = (|| -> Result<()> {
            log(&format!("Copying {name}"));
            Package::copy(&ip.dir, &partial)?;
            let embedded = partial.join(Self::EMBEDDED_IMAGE_NAME);
            if Self::carries_image(&c, with_image) && !embedded.exists() {
                let image = self.paths.image_dir(&c.image);
                if !image.exists() {
                    return Err(vm_err!(
                        codes::NOT_FOUND,
                        "The image {} is missing – the VM cannot be exported without it",
                        c.image
                    ));
                }
                log(&format!("Copying its image {}", c.image));
                Package::copy(&image, &embedded)?;
                Package::writable(&embedded, || {
                    let _ = fs::remove_file(embedded.join("manifest.json"));
                    Ok(())
                })?;
            }
            if c.guest_kind() == GuestKind::Nixos {
                self.rebase(&partial.join("disk.qcow2"), &embedded.join("disk.qcow2"), &partial)?;
            }
            Manifest::new(
                PackageKind::Vm,
                name,
                Some(c.guest_kind()),
                Some(c.recipe.clone()),
                Some(c.image.clone()),
                Some(embedded.exists()),
            )
            .write(&partial)?;
            fs::rename(&partial, &dest).with_path(&dest)?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = Disposal::delete(&[&partial]);
            return Err(e);
        }
        log(&format!("Exported to {}", dest.display()));
        Ok(dest)
    }

    /// An image → `<folder>/<name>.omaflake-image`
    pub fn export_image(&self, name: &str, folder: &Path, log: &Log) -> Result<PathBuf> {
        let image = self.images().image(name)?;
        let dest = folder.join(format!("{name}.{}", Package::IMAGE));
        if dest.exists() {
            return Err(vm_err!(codes::EXISTS, "{} exists already", dest.display()));
        }
        let partial = folder.join(format!(".{name}.{}.partial", Package::IMAGE));
        Disposal::delete(&[&partial])?;
        let result = (|| -> Result<()> {
            log(&format!("Copying {name}"));
            Package::copy(&self.paths.image_dir(name), &partial)?;
            Package::writable(&partial, || {
                Manifest::new(
                    PackageKind::Image,
                    name,
                    Some(image.kind()),
                    Some(image.recipe().to_string()),
                    Some(name.to_string()),
                    None,
                )
                .write(&partial)
            })?;
            fs::rename(&partial, &dest).with_path(&dest)?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = Disposal::delete(&[&partial]);
            return Err(e);
        }
        log(&format!("Exported to {}", dest.display()));
        Ok(dest)
    }

    // MARK: - Import: always a copy into the data folder; the source is never changed

    /// The kind of what `path` holds: by extension, a plain folder with recipes counts as recipes
    pub fn kind_of(path: &Path) -> PackageKind {
        Package::kind_of(path).unwrap_or(PackageKind::Recipe)
    }

    pub fn conflict(&self, path: &Path) -> Result<Conflict> {
        let url = standardize(path);
        match Self::kind_of(&url) {
            PackageKind::Vm => {
                let c = Self::read_config(&url)?;
                // taking back a VM that runs from there
                if self.external_vm(&url).as_deref() == Some(c.name.as_str()) {
                    return Ok(Conflict::None);
                }
                Ok(if self.store().exists(&c.name) { Conflict::VmName(c.name) } else { Conflict::None })
            }
            PackageKind::Image => {
                let name = Package::name_of(&url, PackageKind::Image).filter(|_| ImageStore::read_info(&url).is_some());
                let Some(name) = name else {
                    return Err(vm_err!("{} is not an image package (no image.json)", url.display()));
                };
                Ok(if self.paths.image_dir(&name).exists() { Conflict::Image(name) } else { Conflict::None })
            }
            PackageKind::Recipe => {
                RecipeLibrary::new(self.paths.clone()).recipes_in(&url)?;
                Ok(Conflict::None)
            }
        }
    }

    /// Imports a package by its kind. A conflict (see `conflict`) is an error unless it is resolved: a VM under
    /// another `name`, an image with `replace`. Recipes whose content is here already are skipped.
    pub fn import_package(&self, path: &Path, name: Option<&str>, replace: bool, log: &Log) -> Result<Imported> {
        let url = standardize(path);
        match Self::kind_of(&url) {
            PackageKind::Vm => self.import_vm(&url, name, log),
            PackageKind::Image => {
                let (n, notes) = self.import_image(&url, None, replace, log)?;
                Ok(Imported { kind: PackageKind::Image, names: vec![n], notes })
            }
            PackageKind::Recipe => {
                let ids = RecipeLibrary::new(self.paths.clone()).import_recipes(&url)?;
                let notes =
                    if ids.is_empty() { vec!["These recipes are here already, unchanged".to_string()] } else { vec![] };
                Ok(Imported { kind: PackageKind::Recipe, names: ids, notes })
            }
        }
    }

    /// An image package → the data folder (under `named`, else its own name). `replace` puts it in place of the same
    /// image (refused while a VM on it runs). Returns (name, notes).
    pub(crate) fn import_image(
        &self,
        path: &Path,
        named: Option<&str>,
        replace: bool,
        log: &Log,
    ) -> Result<(String, Vec<String>)> {
        let info = ImageStore::read_info(path);
        let name = named.map(str::to_string).or_else(|| Package::name_of(path, PackageKind::Image));
        let (Some(info), Some(name)) = (info, name) else {
            return Err(vm_err!("{} is not an image package (no image.json)", path.display()));
        };
        if !Platform::runs_here(&info.system) {
            return Err(Platform::not_here(&name, &info.system));
        }
        let dest = self.paths.image_dir(&name);
        let exists = dest.exists();
        if exists {
            if !replace {
                return Err(vm_err!(codes::EXISTS, "The image {name} is here already – replace it or cancel"));
            }
            let running: Vec<String> = self
                .store()
                .list()
                .into_iter()
                .filter(|c| c.image == name && DataFolder::is_running(c, &self.paths))
                .map(|c| c.name)
                .collect();
            if !running.is_empty() {
                return Err(vm_err!(codes::VM_RUNNING, "{} run on {name} – shut them down first", running.join(", ")));
            }
        }
        let images = self.paths.images();
        fs::create_dir_all(&images).with_path(&images)?;
        let partial = images.join(format!(".{name}.partial"));
        Disposal::delete(&[&partial])?;
        let result = (|| -> Result<()> {
            log(&format!("Copying {name}"));
            Package::copy(path, &partial)?;
            Package::writable(&partial, || {
                let _ = fs::remove_file(partial.join("manifest.json"));
                Ok(())
            })?;
            if exists {
                Disposal::delete(&[&dest])?;
            }
            fs::rename(&partial, &dest).with_path(&dest)?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = Disposal::delete(&[&partial]);
            return Err(e);
        }
        let store = self.images();
        if store.current_name_in_line(&info.line()).is_none() {
            store.set_current(&name)?;
        }
        let note = if exists { format!("image {name} replaced") } else { format!("image {name} imported") };
        Ok((name, vec![note]))
    }

    /// instance.json of a VM package (migrated in memory); "<path> is not a VM package (no instance.json)"
    pub fn read_config(package: &Path) -> Result<InstanceConfig> {
        let Ok(data) = fs::read(package.join("instance.json")) else {
            return Err(vm_err!("{} is not a VM package (no instance.json)", package.display()));
        };
        Ok(InstanceConfig::FILE_FORMAT.decode::<InstanceConfig>(&data)?.0)
    }

    /// The name of the registered external VM whose package is `path`
    pub(crate) fn external_vm(&self, path: &Path) -> Option<String> {
        let store = self.store();
        let url = standardize(path);
        store
            .list()
            .into_iter()
            .find(|c| store.is_external(&c.name) && standardize(&self.paths.instance(&c.name).dir) == url)
            .map(|c| c.name)
    }

    /// A VM package → the data folder. Under its own name it is the same VM again (a backup coming back, the next Mac
    /// of the same person); under another name a second VM with its own identity (SSH port, macOS machine identifier
    /// and MAC). The image it carries becomes an image of the data folder unless that is here already.
    pub(crate) fn import_vm(&self, path: &Path, name: Option<&str>, log: &Log) -> Result<Imported> {
        let store = self.store();
        let mut c = Self::read_config(path)?;
        InstanceStore::check_config_runs_here(&c)?;
        let mut notes: Vec<String> = Vec::new();
        // Taking back a VM that runs from this place: it stays registered until it is back
        let external = self.external_vm(path);
        if let Some(external) = &external {
            if store.load(external).is_ok_and(|e| DataFolder::is_running(&e, &self.paths)) {
                return Err(vm_err!(codes::VM_RUNNING, "{external} is running – shut it down first"));
            }
        }
        let new_name = name.map(str::to_string).unwrap_or_else(|| c.name.clone());
        let new_identity = new_name != c.name;
        let taking_back = external.as_deref() == Some(new_name.as_str());
        store.validate(&new_name)?;
        if !taking_back && store.exists(&new_name) {
            return Err(vm_err!(
                codes::EXISTS,
                "A VM named \"{new_name}\" is here already – cancel, or import it under another name"
            ));
        }
        if new_identity && c.guest_kind() == GuestKind::Macos && self.runner.is_none() {
            return Err(vm_err!(
                codes::PREREQUISITE,
                "omaf-runner is missing – a macOS VM cannot get a new identity without it"
            ));
        }
        let instances = self.paths.instances();
        fs::create_dir_all(&instances).with_path(&instances)?;
        let dest = instances.join(format!("{new_name}.{}", Package::VM));
        // Next to `dest`, so paths relative to the package are the same in both
        let partial = instances.join(format!(".{new_name}.{}.partial", Package::VM));
        Disposal::delete(&[&partial])?;
        let result = (|| -> Result<()> {
            log(&format!("Copying {}", c.name));
            // Everything but the manifest and the image it carries: that becomes an image of the data folder
            fs::create_dir_all(&partial).with_path(&partial)?;
            let mut items: Vec<String> = fs::read_dir(path)
                .with_path(path)?
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .collect();
            items.sort();
            for item in items {
                if item == "manifest.json" || item == Self::EMBEDDED_IMAGE_NAME {
                    continue;
                }
                Package::copy(&path.join(&item), &partial.join(&item))?;
            }
            let embedded = path.join(Self::EMBEDDED_IMAGE_NAME);
            if embedded.exists() && !self.paths.image_dir(&c.image).exists() {
                let image = c.image.clone();
                notes.extend(self.import_image(&embedded, Some(&image), false, log)?.1);
            }
            c.name = new_name.clone();
            if c.guest_kind() == GuestKind::Nixos {
                let image = self.paths.image_dir(&c.image).join("disk.qcow2");
                if !image.exists() {
                    return Err(vm_err!(
                        codes::NOT_FOUND,
                        "The VM's image {} is missing – import the image (or a package with it) first",
                        c.image
                    ));
                }
                self.rebase(&partial.join("disk.qcow2"), &image, &partial)?;
                c.image_path = image.to_string_lossy().into_owned();
                if new_identity || store.list().iter().any(|o| o.name != new_name && o.ssh_port == c.ssh_port) {
                    c.ssh_port = store.next_ssh_port(&Ports::is_free);
                    notes.push(format!("SSH port {}", c.ssh_port));
                }
            } else {
                c.image_path = self.paths.image_dir(&c.image).to_string_lossy().into_owned();
                if new_identity {
                    self.new_mac_identity(&partial.join("vm"))?;
                }
            }
            if new_identity || c.identity.is_none() {
                c.identity = Some(OmaflakeId::random_words());
            }
            write_atomic(&partial.join("instance.json"), &InstanceConfig::FILE_FORMAT.encode(&c)?)?;
            fs::rename(&partial, &dest).with_path(&dest)?;
            Ok(())
        })();
        if let Err(e) = result {
            let _ = Disposal::delete(&[&partial]);
            return Err(e);
        }
        if taking_back {
            if let Some(external) = &external {
                store.forget(external)?;
            }
        }
        notes.push(format!("VM {new_name} imported"));
        Ok(Imported { kind: PackageKind::Vm, names: vec![new_name], notes })
    }

    /// A macOS VM folder gets a new machine identifier and MAC (omaf-runner clone on the same volume: instant)
    pub(crate) fn new_mac_identity(&self, vm: &Path) -> Result<()> {
        let Some(runner) = &self.runner else {
            return Err(vm_err!(
                codes::PREREQUISITE,
                "omaf-runner is missing – a macOS VM cannot get a new identity without it"
            ));
        };
        if !vm.exists() {
            return Ok(());
        }
        let fresh = vm.parent().unwrap_or(Path::new("/")).join("vm.new");
        Disposal::delete(&[&fresh])?;
        let (from, to) = (vm.to_string_lossy().into_owned(), fresh.to_string_lossy().into_owned());
        Shell::check(runner, &["clone", &from, &to], &RunOptions::default())?;
        Disposal::delete(&[vm])?;
        fs::rename(&fresh, vm).with_path(vm)?;
        Ok(())
    }

    /// The overlay's backing file → `image`, written relative to the package (`qemu-img rebase -u -F qcow2 -b <rel>`)
    pub(crate) fn rebase(&self, disk: &Path, image: &Path, package: &Path) -> Result<()> {
        if !disk.exists() {
            return Ok(());
        }
        let relative = Package::relative_path(package, image);
        let disk = disk.to_string_lossy().into_owned();
        Shell::check(
            &self.qemu_img,
            &["rebase", "-u", "-F", "qcow2", "-b", &relative, &disk],
            &RunOptions { cwd: Some(package), ..Default::default() },
        )?;
        Ok(())
    }

    // MARK: - External VMs: run from where they are

    /// On a network volume (NAS): runs, but slower, and the VM breaks when the connection does. False when the
    /// volume cannot be read (as Swift: `volumeIsLocal == false`).
    ///
    /// macOS: `statfs` and `MNT_LOCAL` not set in `f_flags` (what `volumeIsLocalKey` reports). Linux: `statfs`
    /// `f_type` is one of the known network file systems (NFS, SMB/CIFS/SMB2, AFS, Coda, 9P, Ceph, NCP; FUSE file
    /// systems like sshfs are not recognized – OPEN). Other systems: false.
    pub fn is_on_network_volume(path: &Path) -> bool {
        network_volume(path)
    }

    /// Runs a VM package from where it is (an external disk, a stick used on several Macs, a NAS, a folder): it must
    /// carry what it needs (NixOS: its image); its SSH port moves if this Mac uses it already. Nothing is copied, the
    /// package is registered in config.json (`externalVMs`). Returns the VM's name.
    pub fn use_external(&self, path: &Path) -> Result<String> {
        let store = self.store();
        let url = standardize(path);
        let Some(name) = Package::name_of(&url, PackageKind::Vm) else {
            return Err(vm_err!("{} is not a VM package", url.display()));
        };
        let mut c = Self::read_config(&url)?;
        InstanceStore::check_config_runs_here(&c)?;
        if c.name != name {
            return Err(vm_err!("The package is named {name}, the VM inside {} – rename it back first", c.name));
        }
        if self.external_vm(&url).is_some() {
            return Err(vm_err!(codes::EXISTS, "{name} runs from there already"));
        }
        if store.exists(&name) {
            return Err(vm_err!(
                codes::EXISTS,
                "A VM named \"{name}\" is here already – remove or rename that one first"
            ));
        }
        let embedded = url.join(Self::EMBEDDED_IMAGE_NAME);
        if c.guest_kind() == GuestKind::Nixos {
            if !embedded.join("disk.qcow2").exists() {
                return Err(vm_err!(
                    "{name} does not carry its image – export it again (omaf export vm) to run it from there"
                ));
            }
            self.rebase(&url.join("disk.qcow2"), &embedded.join("disk.qcow2"), &url)?;
        }
        store.register(&url)?;
        if c.guest_kind() == GuestKind::Nixos && store.list().iter().any(|o| o.name != name && o.ssh_port == c.ssh_port)
        {
            c.ssh_port = store.next_ssh_port(&Ports::is_free);
            store.save(&c)?;
        }
        Ok(name)
    }
}

/// `statfs(path)`, None when it fails
fn stat_fs(path: &Path) -> Option<libc::statfs> {
    use std::os::unix::ffi::OsStrExt;
    let c = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    // SAFETY: a valid NUL-terminated path and a zeroed statfs as the out-parameter
    unsafe {
        let mut s: libc::statfs = std::mem::zeroed();
        (libc::statfs(c.as_ptr(), &mut s) == 0).then_some(s)
    }
}

#[cfg(target_os = "macos")]
fn network_volume(path: &Path) -> bool {
    stat_fs(path).is_some_and(|s| s.f_flags & (libc::MNT_LOCAL as u32) == 0)
}

#[cfg(target_os = "linux")]
fn network_volume(path: &Path) -> bool {
    // Magic numbers of linux/magic.h and the file systems' own headers
    const NETWORK: [i64; 10] = [
        0x6969,     // NFS
        0x517B,     // SMB
        0xFF534D42, // CIFS
        0xFE534D42, // SMB2
        0x5346414F, // AFS (kAFS)
        0x73757245, // Coda
        0x01021997, // 9P (v9fs)
        0x00C36400, // Ceph
        0x564C,     // NCP
        0x6B414653, // AFS (OpenAFS)
    ];
    #[allow(clippy::unnecessary_cast)]
    stat_fs(path).is_some_and(|s| NETWORK.contains(&(s.f_type as i64)))
}

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
fn network_volume(_path: &Path) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_store::ImageInfo;
    use crate::models::{NetworkMode, RecipeDefaults};
    use crate::packages::Manifest;
    use crate::paths::tests::temp_paths;
    use crate::paths::Settings;
    use std::os::unix::fs::PermissionsExt;

    fn quiet() -> &'static Log<'static> {
        &|_| {}
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // its tests are macOS only
    fn mac_vm(p: &Paths, name: &str) -> InstanceConfig {
        let mut s = RecipeDefaults::new(GuestKind::Macos, None, 4, 8, 80, NetworkMode::Shared, false, false, None);
        s.user = Some("test".into());
        let c = InstanceConfig::new(
            name,
            "omaflake.macos.minimal",
            "img-1",
            "/img",
            "aarch64-darwin",
            &s,
            22,
            crate::json::parse_date("1970-01-01T00:00:00Z").unwrap(),
        );
        let ip = p.instance(name);
        fs::create_dir_all(ip.vm()).unwrap();
        fs::write(ip.vm().join("disk.img"), "disk").unwrap();
        InstanceStore::new(p.clone(), "/usr/bin/false").save(&c).unwrap();
        c
    }

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // its tests are macOS only
    fn names(store: &InstanceStore) -> Vec<String> {
        store.list().into_iter().map(|c| c.name).collect()
    }

    /// Backup and back: a stopped VM goes out as a package and comes back as the same VM
    // A macOS guest package: only a Mac runs it (create/import refuse another system)
    #[cfg(target_os = "macos")]
    #[test]
    fn vm_goes_out_and_comes_back() {
        let (_t, p) = temp_paths();
        let out = p.root.join("stick");
        fs::create_dir_all(&out).unwrap();
        mac_vm(&p, "m1");
        let t = Transfer::new(p.clone(), "/usr/bin/false", None);
        let pkg = t.export_vm("m1", &out, false, quiet()).unwrap();
        assert_eq!(pkg.file_name().unwrap(), "m1.omaflake-vm");
        let manifest = Manifest::read(&pkg).unwrap();
        assert!(manifest.kind == PackageKind::Vm && manifest.contains_image == Some(false));
        assert_eq!(t.export_vm("m1", &out, false, quiet()).unwrap_err().code(), Some(codes::EXISTS));
        // the same VM cannot come in twice; under another name a macOS VM needs omaf-runner (new identity)
        assert_eq!(t.conflict(&pkg).unwrap(), Conflict::VmName("m1".into()));
        assert!(t.import_package(&pkg, None, false, quiet()).is_err());
        assert!(t.import_package(&pkg, Some("m2"), false, quiet()).is_err());
        assert!(!p.instances().join("m2.omaflake-vm").exists());
        Disposal::delete(&[p.instance("m1").dir]).unwrap();
        assert_eq!(t.conflict(&pkg).unwrap(), Conflict::None);
        let r = t.import_package(&pkg, None, false, quiet()).unwrap();
        assert!(r.kind == PackageKind::Vm && r.names == ["m1"]);
        let store = InstanceStore::new(p.clone(), "/usr/bin/false");
        assert_eq!(store.load("m1").unwrap().name, "m1");
        assert!(!p.instance("m1").dir.join("manifest.json").exists());
        assert_eq!(fs::read(p.instance("m1").vm().join("disk.img")).unwrap(), b"disk");
        // under another name with omaf-runner standing in (the clone itself is checked with a real runner)
        let fake_runner = p.root.join("runner.sh");
        fs::write(&fake_runner, "#!/bin/sh\n[ \"$1\" = clone ] && cp -R \"$2\" \"$3\"\n").unwrap();
        fs::set_permissions(&fake_runner, fs::Permissions::from_mode(0o755)).unwrap();
        let runner = Some(fake_runner.to_string_lossy().into_owned());
        let r2 = Transfer::new(p.clone(), "/usr/bin/false", runner)
            .import_package(&pkg, Some("m2"), false, quiet())
            .unwrap();
        assert_eq!(r2.names, ["m2"]);
        let m2 = store.load("m2").unwrap();
        assert_eq!(m2.name, "m2");
        assert_ne!(m2.identity, store.load("m1").unwrap().identity);
        assert_eq!(fs::read(p.instance("m2").vm().join("disk.img")).unwrap(), b"disk");
        assert_eq!(names(&store), ["m1", "m2"]);
    }

    /// A VM package registered in place shows up while its folder is there and can be forgotten while it is not
    // A macOS guest package: only a Mac runs it (create/import refuse another system)
    #[cfg(target_os = "macos")]
    #[test]
    fn external_vm_is_listed_while_there() {
        let (_t, p) = temp_paths();
        let out = p.root.join("stick");
        fs::create_dir_all(&out).unwrap();
        mac_vm(&p, "m1");
        let t = Transfer::new(p.clone(), "/usr/bin/false", None);
        let pkg = t.export_vm("m1", &out, false, quiet()).unwrap();
        Disposal::delete(&[p.instance("m1").dir]).unwrap();
        assert_eq!(t.use_external(&pkg).unwrap(), "m1");
        assert!(t.use_external(&pkg).is_err()); // runs from there already
        let store = InstanceStore::new(p.clone(), "/usr/bin/false");
        assert!(names(&store) == ["m1"] && store.is_external("m1"));
        assert_eq!(standardize(&p.instance("m1").dir), standardize(&pkg));
        // imported from its place: back in the data folder, no longer external; then external again
        assert_eq!(t.conflict(&pkg).unwrap(), Conflict::None);
        t.import_package(&pkg, None, false, quiet()).unwrap();
        assert!(!store.is_external("m1"));
        assert_eq!(Settings::load_file(&p.config_file).external_vms, Some(vec![]));
        assert!(t.use_external(&pkg).is_err()); // the name is taken now
        Disposal::delete(&[p.instance("m1").dir]).unwrap();
        t.use_external(&pkg).unwrap();
        // the stick is pulled: gone from the list, still forgettable
        let away = p.root.join("away");
        fs::rename(&out, &away).unwrap();
        assert!(store.list().is_empty());
        store.forget("m1").unwrap();
        assert_eq!(Settings::load_file(&p.config_file).external_vms, Some(vec![]));
    }

    /// Swift `images.currentName(recipe:)` goes through the recipe's line (F3's `RecipeLibrary::available`); here the
    /// line of the image itself is asked, which is what the import sets.
    // A macOS guest package: only a Mac runs it (create/import refuse another system)
    #[cfg(target_os = "macos")]
    #[test]
    fn image_goes_out_and_comes_back() {
        let (_t, p) = temp_paths();
        let dir = p.image_dir("img-1");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("config.json"), "{}").unwrap();
        let info = ImageInfo::new(
            "omaflake.macos.minimal",
            None,
            GuestKind::Macos,
            "aarch64-darwin",
            "",
            "r",
            None,
            crate::json::now(),
        );
        ImageStore::write_info(&info, &dir).unwrap();
        fs::set_permissions(&dir, fs::Permissions::from_mode(0o555)).unwrap();
        let out = p.root.join("stick");
        fs::create_dir_all(&out).unwrap();
        let t = Transfer::new(p.clone(), "/usr/bin/false", None);
        let pkg = t.export_image("img-1", &out, quiet()).unwrap();
        assert_eq!(Manifest::read(&pkg).unwrap().recipe.as_deref(), Some("omaflake.macos.minimal"));
        Disposal::delete(&[&dir]).unwrap();
        let r = t.import_package(&pkg, None, false, quiet()).unwrap();
        assert!(r.kind == PackageKind::Image && r.names == ["img-1"]);
        let images = ImageStore::new(p.clone());
        assert_eq!(images.list().into_iter().map(|i| i.name).collect::<Vec<_>>(), ["img-1"]);
        assert_eq!(images.current_name_in_line(&info.line()).as_deref(), Some("img-1"));
        assert!(!dir.join("manifest.json").exists());
        // there already: an error unless it is replaced
        assert_eq!(t.conflict(&pkg).unwrap(), Conflict::Image("img-1".into()));
        assert_eq!(t.import_package(&pkg, None, false, quiet()).unwrap_err().code(), Some(codes::EXISTS));
        assert_eq!(t.import_package(&pkg, None, true, quiet()).unwrap().notes, ["image img-1 replaced"]);
        assert_eq!(images.list().into_iter().map(|i| i.name).collect::<Vec<_>>(), ["img-1"]);
        for d in [&dir, &pkg] {
            let _ = fs::set_permissions(d, fs::Permissions::from_mode(0o755));
        }
    }

    /// A NixOS VM carries its image; the overlay is rebased onto it (qemu-img stand-in records the call)
    #[test]
    fn nixos_vm_carries_its_image_and_runs_from_there() {
        if Platform::host_system(GuestKind::Nixos).is_none() {
            return;
        }
        let (_t, p) = temp_paths();
        let system = Platform::host_system(GuestKind::Nixos).unwrap();
        let image_dir = p.image_dir("omaflake.nixos.minimal@a");
        fs::create_dir_all(&image_dir).unwrap();
        fs::write(image_dir.join("disk.qcow2"), "qcow").unwrap();
        let info =
            ImageInfo::new("omaflake.nixos.minimal", None, GuestKind::Nixos, system, "", "r", None, crate::json::now());
        ImageStore::write_info(&info, &image_dir).unwrap();
        let mut c = crate::models::tests::config("n1");
        c.image = "omaflake.nixos.minimal@a".into();
        c.system = system.into();
        let ip = p.instance("n1");
        fs::create_dir_all(&ip.dir).unwrap();
        fs::write(ip.disk(), "overlay").unwrap();
        InstanceStore::new(p.clone(), "/usr/bin/false").save(&c).unwrap();
        let fake = p.root.join("qemu-img.sh");
        let calls = p.root.join("calls");
        fs::write(&fake, format!("#!/bin/sh\necho \"$PWD $@\" >> '{}'\n", calls.display())).unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        let t = Transfer::new(p.clone(), &fake.to_string_lossy(), None);
        let out = p.root.join("stick");
        fs::create_dir_all(&out).unwrap();
        let pkg = t.export_vm("n1", &out, false, quiet()).unwrap();
        assert_eq!(Manifest::read(&pkg).unwrap().contains_image, Some(true));
        let embedded = pkg.join(Transfer::EMBEDDED_IMAGE_NAME);
        assert!(embedded.join("disk.qcow2").exists() && !embedded.join("manifest.json").exists());
        let logged = fs::read_to_string(&calls).unwrap();
        assert!(logged.contains("rebase -u -F qcow2 -b image.omaflake-image/disk.qcow2"), "{logged}");
        // run from the stick: its SSH port moves while another VM here uses it
        Disposal::delete(&[&ip.dir]).unwrap();
        let mut other = crate::models::tests::config("n2");
        other.ssh_port = c.ssh_port;
        fs::create_dir_all(p.instance("n2").dir).unwrap();
        InstanceStore::new(p.clone(), "/usr/bin/false").save(&other).unwrap();
        assert_eq!(t.use_external(&pkg).unwrap(), "n1");
        let moved = InstanceStore::new(p.clone(), "/usr/bin/false").load("n1").unwrap();
        assert_ne!(moved.ssh_port, c.ssh_port);
        // a package without its image is refused
        Disposal::delete(&[&embedded]).unwrap();
        InstanceStore::new(p.clone(), "/usr/bin/false").forget("n1").unwrap();
        assert!(t.use_external(&pkg).is_err());
    }

    #[test]
    fn kinds_and_image_carrying() {
        assert_eq!(Transfer::kind_of(Path::new("/a/x.omaflake-vm")), PackageKind::Vm);
        assert_eq!(Transfer::kind_of(Path::new("/a/x.omaflake-image")), PackageKind::Image);
        assert_eq!(Transfer::kind_of(Path::new("/a/folder")), PackageKind::Recipe);
        let mut c = crate::models::tests::config("x");
        assert!(Transfer::carries_image(&c, false));
        c.kind = GuestKind::Macos;
        assert!(!Transfer::carries_image(&c, false) && Transfer::carries_image(&c, true));
        c.ephemeral = true;
        assert!(Transfer::carries_image(&c, false));
    }

    #[test]
    fn local_folders_are_not_network_volumes() {
        let (_t, p) = temp_paths();
        assert!(!Transfer::is_on_network_volume(&p.root));
        assert!(!Transfer::is_on_network_volume(&p.root.join("missing")));
    }

    #[test]
    fn recipes_travel_with_what_they_require() {
        let (_t, p) = temp_paths();
        let src = p.root.join("src/recipes/me/nixos");
        for (name, req) in [("base", ""), ("desk", "requires = [\"me/nixos/base\"]")] {
            let d = src.join(name);
            fs::create_dir_all(&d).unwrap();
            fs::write(
                d.join("recipe.toml"),
                format!("\"$schema\" = \"https://omaflake.dev/schema/recipe/v1.json\"\ndescription = \"x\"\n{req}\n"),
            )
            .unwrap();
            fs::write(d.join("default.nix"), "{ ... }: { }\n").unwrap();
        }
        let library = RecipeLibrary::new(p.clone());
        // one recipe folder without the one it requires is refused
        assert!(library.import_recipes(&src.join("desk")).is_err());
        let mut ids = library.import_recipes(&p.root.join("src")).unwrap();
        ids.sort();
        assert_eq!(
            ids.iter().map(|i| crate::models::RecipeId::base(i)).collect::<Vec<_>>(),
            ["me.nixos.base", "me.nixos.desk"]
        );
        assert!(ids.iter().all(|i| crate::models::RecipeId::words(i).is_some_and(|w| OmaflakeId::is_words(w, true))));
        let desk = ids[1].clone();
        assert_eq!(library.installed()[&desk].requires, Some(vec!["me.nixos.base".to_string()]));
        assert_eq!(library.installed()[&desk].source, RecipeLibrary::IMPORTED_SOURCE);
        assert!(p.recipes().join(format!("{desk}/recipe.toml")).exists());
        let out = p.root.join("stick");
        fs::create_dir_all(&out).unwrap();
        let pkg = library.export_recipe(&desk, &out, &Settings::load(&p)).unwrap();
        assert_eq!(pkg.file_name().unwrap(), "me.nixos.desk.omaflake-recipe");
        let folders: Vec<String> = RecipeLibrary::recipe_folders(&pkg).into_iter().map(|f| f.0).collect();
        assert_eq!(folders, ["me.nixos.base", "me.nixos.desk"]);
        // into another data folder: the same content gives the same IDs
        let (_t2, q) = temp_paths();
        let tq = Transfer::new(q.clone(), "/usr/bin/false", None);
        assert_eq!(tq.conflict(&pkg).unwrap(), Conflict::None);
        let r = tq.import_package(&pkg, None, false, quiet()).unwrap();
        let mut got = r.names.clone();
        got.sort();
        assert!(r.kind == PackageKind::Recipe && got == ids);
        // a second time, identical: nothing written
        assert!(tq.import_package(&pkg, None, false, quiet()).unwrap().names.is_empty());
        // changed: a recipe of its own next to the first (plan B110) – kept with the setting (B111)
        let mut keep = Settings::load(&q);
        keep.keep_recipe_versions = Some(true);
        keep.save(&q).unwrap();
        fs::write(pkg.join("recipes/me/nixos/desk/default.nix"), "{ ... }: { networking.hostName = \"x\"; }\n")
            .unwrap();
        assert_eq!(tq.conflict(&pkg).unwrap(), Conflict::None);
        let second = tq.import_package(&pkg, None, false, quiet()).unwrap().names;
        assert!(second.len() == 1 && crate::models::RecipeId::base(&second[0]) == "me.nixos.desk" && second[0] != desk);
        let lq = RecipeLibrary::new(q.clone());
        let available: Vec<String> = lq.available(&Settings::load(&q)).into_iter().map(|a| a.id).collect();
        assert_eq!(available.iter().filter(|i| crate::models::RecipeId::base(i) == "me.nixos.desk").count(), 2);
        assert!(lq.distinctions(&Settings::load(&q)).is_empty());
        let staged = lq.stage(&Settings::load(&q)).unwrap();
        let mapping: std::collections::HashMap<String, String> =
            serde_json::from_slice(&fs::read(staged.join("recipes.json")).unwrap()).unwrap();
        let keys: std::collections::HashSet<&String> = mapping.keys().collect();
        assert_eq!(keys, available.iter().collect());
        assert_eq!(mapping[&second[0]], format!("{}/me/nixos/desk", second[0]));
        assert!(staged.join(format!("{}/me/nixos/base/default.nix", second[0])).exists());
    }
}
