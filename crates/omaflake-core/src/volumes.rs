// SPDX-License-Identifier: AGPL-3.0-or-later
//! Volumes (plan B187; German "Platten"): folders of this computer that VMs share. A VM has none, one or several, and
//! a volume may be in several VMs – the user decides, as with networks. Each is a folder: `share/<name>` in the data
//! folder by default, or one the user chose. A guest sees each of its volumes on its own at `/Volumes/<mount name>`,
//! the same path in macOS and NixOS guests: the mount name is the user's per VM, by default the name of the volume's
//! folder. It is the device's tag (a virtio-fs device of Virtualization.framework, a virtio-9p device of QEMU), and the
//! guest mounts every tag it finds there (vm-guest.nix; the macOS platform's launch daemon).
//!
//! `volumes.json` keeps the volumes the user made; a volume without a record is one at its default folder (every VM
//! from before volumes had one named after itself – `InstanceConfig::volume_names`), and a VM from before with a
//! folder of its own (`sharePath`) gives that folder to its volume. Removing a volume takes it out of its VMs and
//! forgets it – its folder, the user's files, always stays.

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{IoContext, Result, codes};
use crate::file_format::{FileFormat, Step, write_atomic};
use crate::instance_store::InstanceStore;
use crate::json::Date;
use crate::models::{InstanceConfig, VolumeMount};
use crate::paths::Paths;
use crate::vm_err;

static VOLUMES_STEPS: [Step; 1] = [|_| Ok(())];

/// Most volumes a NixOS VM can have: one PCI slot each (qemu_command.rs `slot::VOLUMES`)
pub const MAX_PER_VM: usize = 8;
/// A volume's name (it is a folder's name in `share/`)
pub const MAX_NAME: usize = 31;
/// A mount name is a virtio-9p tag in NixOS guests: at most 31 bytes (Virtualization.framework takes 36)
pub const MAX_MOUNT: usize = 31;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Volume {
    /// Lowercase letters, digits and dashes – it is the folder's name in the guests
    pub name: String,
    /// The folder the user chose; None: `share/<name>` in the data folder
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    #[serde(with = "crate::json::iso8601")]
    pub created: Date,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, Default)]
struct VolumesFile {
    #[serde(default)]
    volumes: Vec<Volume>,
}

/// A volume as the lists show it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VolumeInfo {
    pub name: String,
    /// Its folder on this computer
    pub folder: PathBuf,
    /// The folder is one the user chose (not `share/<name>`)
    pub custom: bool,
    /// The VMs that have it
    pub vms: Vec<String>,
}

pub struct Volumes {
    paths: Paths,
}

impl Volumes {
    pub const FILE: &'static str = "volumes.json";
    pub const FILE_FORMAT: FileFormat = FileFormat::new("volumes", "volumes.json", 1, &VOLUMES_STEPS);
    /// Where every guest sees its volumes, each at `<GUEST_ROOT>/<mount name>`
    pub const GUEST_ROOT: &'static str = "/Volumes";

    pub fn new(paths: Paths) -> Volumes {
        Volumes { paths }
    }

    fn file(&self) -> PathBuf {
        self.paths.root.join(Self::FILE)
    }

    fn read(&self) -> VolumesFile {
        std::fs::read(self.file())
            .ok()
            .and_then(|d| Self::FILE_FORMAT.decode::<VolumesFile>(&d).ok())
            .map(|(f, _)| f)
            .unwrap_or_default()
    }

    fn write(&self, f: &VolumesFile) -> Result<()> {
        std::fs::create_dir_all(&self.paths.root).with_path(&self.paths.root)?;
        write_atomic(&self.file(), &Self::FILE_FORMAT.encode(f)?)
    }

    /// 1 to 31 characters a–z, 0–9, "-", not starting with "-"
    pub fn validate_name(name: &str) -> Result<()> {
        let ok = !name.is_empty()
            && name.len() <= MAX_NAME
            && !name.starts_with('-')
            && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            return Err(vm_err!("Invalid volume name \"{name}\": 1–31 characters a–z, 0–9, \"-\""));
        }
        Ok(())
    }

    /// A name in the guest: 1 to 31 bytes, no "/", "," or "=", not starting with "." or a space, not ending in a space
    pub fn validate_mount(name: &str) -> Result<()> {
        let ok = !name.is_empty()
            && name.len() <= MAX_MOUNT
            && !name.starts_with(['.', ' '])
            && !name.ends_with(' ')
            && !name.chars().any(|c| c.is_control() || matches!(c, '/' | ',' | '=' | ':'));
        if !ok {
            return Err(vm_err!(
                "Invalid name in the VM \"{name}\": 1–31 bytes, no \"/\", \",\", \"=\" or \":\", not starting with \".\""
            ));
        }
        Ok(())
    }

    /// The name the guest sees a volume under: the one the user chose, else its folder's name (the volume's name when
    /// that is no valid one)
    pub fn mount_name(m: &VolumeMount, folder: &Path) -> String {
        if let Some(n) = &m.mount {
            return n.clone();
        }
        folder
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .filter(|n| Self::validate_mount(n).is_ok())
            .unwrap_or_else(|| m.volume.clone())
    }

    /// The name of a VM's own volume: the VM's name, cut to a volume name's length
    pub fn own_name(vm: &str) -> String {
        vm.chars().take(MAX_NAME).collect::<String>().trim_end_matches('-').to_string()
    }

    /// A volume's folder: its record's, else – for a VM's own volume from before volumes – that VM's `sharePath`, else
    /// `share/<name>`
    pub fn folder(&self, name: &str, instances: &[InstanceConfig]) -> PathBuf {
        if let Some(v) = self.read().volumes.into_iter().find(|v| v.name == name) {
            return v.path.map(PathBuf::from).unwrap_or_else(|| self.paths.share(name));
        }
        instances
            .iter()
            .find(|c| Self::own_name(&c.name) == name && c.share_path.is_some())
            .and_then(|c| c.share_path.clone())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.paths.share(name))
    }

    /// Every volume: those the user made, those VMs have, and the folders in `share/` (each of them one)
    pub fn list(&self, instances: &[InstanceConfig]) -> Vec<VolumeInfo> {
        let mut names: Vec<String> = self.read().volumes.into_iter().map(|v| v.name).collect();
        for c in instances {
            names.extend(c.volume_names());
        }
        if let Ok(entries) = std::fs::read_dir(self.paths.shares()) {
            for e in entries.flatten() {
                let n = e.file_name().to_string_lossy().into_owned();
                if e.path().is_dir() && Self::validate_name(&n).is_ok() {
                    names.push(n);
                }
            }
        }
        names.sort();
        names.dedup();
        names
            .into_iter()
            .map(|name| {
                let folder = self.folder(&name, instances);
                VolumeInfo {
                    custom: folder != self.paths.share(&name),
                    vms: instances
                        .iter()
                        .filter(|c| c.volume_names().contains(&name))
                        .map(|c| c.name.clone())
                        .collect(),
                    folder,
                    name,
                }
            })
            .collect()
    }

    /// A VM's volumes for the API
    pub fn dtos_of(&self, c: &InstanceConfig, instances: &[InstanceConfig]) -> Vec<crate::api::types::VmVolumeDto> {
        c.volume_mounts()
            .into_iter()
            .map(|m| {
                let folder = self.folder(&m.volume, instances);
                crate::api::types::VmVolumeDto {
                    mount: Self::mount_name(&m, &folder),
                    named: m.mount.is_some(),
                    folder: folder.to_string_lossy().into_owned(),
                    volume: m.volume,
                }
            })
            .collect()
    }

    /// Every volume for the API (`volumes.list`)
    pub fn dtos(&self, instances: &[InstanceConfig]) -> Vec<crate::api::types::VolumeDto> {
        self.list(instances)
            .into_iter()
            .map(|v| crate::api::types::VolumeDto {
                vms: v
                    .vms
                    .iter()
                    .filter_map(|vm| instances.iter().find(|c| &c.name == vm))
                    .map(|c| crate::api::types::VolumeVmDto {
                        vm: c.name.clone(),
                        mount: c
                            .volume_mounts()
                            .iter()
                            .find(|m| m.volume == v.name)
                            .map(|m| Self::mount_name(m, &v.folder))
                            .unwrap_or_default(),
                    })
                    .collect(),
                name: v.name,
                folder: v.folder.to_string_lossy().into_owned(),
                custom: v.custom,
            })
            .collect()
    }

    /// A new volume; `path` a folder the user chose (it must exist), else `share/<name>` is made
    pub fn create(&self, name: &str, path: Option<&Path>, instances: &[InstanceConfig]) -> Result<VolumeInfo> {
        Self::validate_name(name)?;
        if self.list(instances).iter().any(|v| v.name == name) {
            return Err(vm_err!(codes::EXISTS, "There is a volume \"{name}\" already"));
        }
        if let Some(p) = path
            && !p.is_dir()
        {
            return Err(vm_err!(codes::NOT_FOUND, "No such folder: {}", p.display()));
        }
        let mut f = self.read();
        f.volumes.push(Volume {
            name: name.into(),
            path: path.map(|p| p.to_string_lossy().into_owned()),
            created: crate::json::now(),
        });
        self.write(&f)?;
        let folder = self.folder(name, instances);
        std::fs::create_dir_all(&folder).with_path(&folder)?;
        Ok(VolumeInfo { name: name.into(), custom: path.is_some(), folder, vms: vec![] })
    }

    /// Makes the folder of a volume that is about to be used (a folder the user chose must exist); `mode` for a folder
    /// omaflake makes (NixOS guests write through 9p as another user, plan B18)
    pub fn prepare(&self, name: &str, instances: &[InstanceConfig], mode: Option<u32>) -> Result<PathBuf> {
        let folder = self.folder(name, instances);
        if folder != self.paths.share(name) {
            if !folder.is_dir() {
                return Err(vm_err!("The folder of volume \"{name}\" is missing: {}", folder.display()));
            }
            return Ok(folder);
        }
        std::fs::create_dir_all(&folder).with_path(&folder)?;
        if let Some(mode) = mode {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(mode)).with_path(&folder)?;
        }
        Ok(folder)
    }

    fn instances_with(&self, c: &InstanceConfig) -> Vec<InstanceConfig> {
        let mut instances = InstanceStore::new(self.paths.clone(), "").list();
        instances.retain(|i| i.name != c.name);
        instances.push(c.clone());
        instances
    }

    /// A VM's volumes as they are: `(volume, name in the guest, folder)` each
    pub fn mounts(&self, c: &InstanceConfig) -> Vec<(String, String, PathBuf)> {
        let instances = self.instances_with(c);
        c.volume_mounts()
            .into_iter()
            .map(|m| {
                let folder = self.folder(&m.volume, &instances);
                (m.volume.clone(), Self::mount_name(&m, &folder), folder)
            })
            .collect()
    }

    /// A VM's volumes made ready for its start: `(name in the guest, folder)` each
    pub fn prepare_all(&self, c: &InstanceConfig, mode: Option<u32>) -> Result<Vec<(String, PathBuf)>> {
        let instances = self.instances_with(c);
        c.volume_mounts()
            .into_iter()
            .map(|m| {
                let folder = self.prepare(&m.volume, &instances, mode)?;
                Ok((Self::mount_name(&m, &folder), folder))
            })
            .collect()
    }

    /// Gives `vm` the volume (from its next start), in the guest under `mount` (None: its folder's name)
    pub fn attach(&self, store: &InstanceStore, volume: &str, vm: &str, mount: Option<&str>) -> Result<InstanceConfig> {
        let mut c = store.load(vm)?;
        let instances = store.list();
        if !self.list(&instances).iter().any(|v| v.name == volume) {
            return Err(vm_err!(codes::NOT_FOUND, "No volume \"{volume}\""));
        }
        let mut mounts = c.volume_mounts();
        if mounts.iter().any(|m| m.volume == volume) {
            return Err(vm_err!(codes::EXISTS, "{vm} has the volume \"{volume}\" already"));
        }
        if mounts.len() >= MAX_PER_VM {
            return Err(vm_err!("A VM has at most {} volumes", MAX_PER_VM));
        }
        let mount = mount.map(str::trim).filter(|m| !m.is_empty());
        if let Some(m) = mount {
            Self::validate_mount(m)?;
        }
        mounts.push(VolumeMount::new(volume, mount));
        Self::check_unique(&self.mounts_of(&c, &mounts, &instances))?;
        c.set_volumes(mounts);
        store.save(&c)?;
        Ok(c)
    }

    /// Changes the name `vm` sees the volume under (None: its folder's name)
    pub fn rename_mount(
        &self,
        store: &InstanceStore,
        volume: &str,
        vm: &str,
        mount: Option<&str>,
    ) -> Result<InstanceConfig> {
        let mut c = store.load(vm)?;
        let instances = store.list();
        let mut mounts = c.volume_mounts();
        let Some(m) = mounts.iter_mut().find(|m| m.volume == volume) else {
            return Err(vm_err!(codes::NOT_FOUND, "{vm} does not have the volume \"{volume}\""));
        };
        let mount = mount.map(str::trim).filter(|m| !m.is_empty());
        if let Some(n) = mount {
            Self::validate_mount(n)?;
        }
        m.mount = mount.map(String::from);
        Self::check_unique(&self.mounts_of(&c, &mounts, &instances))?;
        c.set_volumes(mounts);
        store.save(&c)?;
        Ok(c)
    }

    fn mounts_of(&self, c: &InstanceConfig, mounts: &[VolumeMount], instances: &[InstanceConfig]) -> Vec<String> {
        let mut all = instances.to_vec();
        all.retain(|i| i.name != c.name);
        all.push(c.clone());
        mounts.iter().map(|m| Self::mount_name(m, &self.folder(&m.volume, &all))).collect()
    }

    /// Two volumes of a VM cannot have the same name in it
    fn check_unique(names: &[String]) -> Result<()> {
        for (i, n) in names.iter().enumerate() {
            if names[..i].iter().any(|x| x.eq_ignore_ascii_case(n)) {
                return Err(vm_err!(codes::EXISTS, "Two volumes would be \"{n}\" in the VM – give one another name"));
            }
        }
        Ok(())
    }

    /// Takes the volume out of `vm` (from its next start); the folder stays
    pub fn detach(&self, store: &InstanceStore, volume: &str, vm: &str) -> Result<InstanceConfig> {
        let mut c = store.load(vm)?;
        let mut mounts = c.volume_mounts();
        if !mounts.iter().any(|m| m.volume == volume) {
            return Err(vm_err!(codes::NOT_FOUND, "{vm} does not have the volume \"{volume}\""));
        }
        mounts.retain(|m| m.volume != volume);
        c.set_volumes(mounts);
        store.save(&c)?;
        Ok(c)
    }

    /// Takes the volume out of every VM and forgets it; its folder – the user's files – stays
    pub fn remove(&self, store: &InstanceStore, name: &str) -> Result<()> {
        let instances = store.list();
        let Some(v) = self.list(&instances).into_iter().find(|v| v.name == name) else {
            return Err(vm_err!(codes::NOT_FOUND, "No volume \"{name}\""));
        };
        for vm in &v.vms {
            self.detach(store, name, vm)?;
        }
        let mut f = self.read();
        f.volumes.retain(|x| x.name != name);
        self.write(&f)?;
        // A default folder with nothing but what macOS leaves in it goes, so the list forgets it too
        if !v.custom {
            InstanceStore::remove_share_if_empty(&v.folder);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;

    fn vm(name: &str, volumes: Option<Vec<&str>>, share: bool) -> InstanceConfig {
        let mut c = crate::models::tests::config(name);
        c.share = share;
        c.volumes = volumes.map(|v| v.into_iter().map(|n| VolumeMount::new(n, None)).collect());
        c
    }

    #[test]
    fn mount_names() {
        for good in ["Projects", "My Code", "a.b", "Ähm"] {
            assert!(Volumes::validate_mount(good).is_ok(), "{good}");
        }
        for bad in ["", ".x", " x", "x ", "a/b", "a,b", "a=b", &"x".repeat(32)] {
            assert!(Volumes::validate_mount(bad).is_err(), "{bad}");
        }
        let m = VolumeMount::new("v", None);
        assert_eq!(Volumes::mount_name(&m, Path::new("/x/Work")), "Work");
        assert_eq!(Volumes::mount_name(&m, Path::new("/x/.hidden")), "v");
        assert_eq!(Volumes::mount_name(&VolumeMount::new("v", Some("Mine")), Path::new("/x/Work")), "Mine");
    }

    #[test]
    fn names() {
        assert!(Volumes::validate_name("projects-1").is_ok());
        for bad in ["", "-a", "A", "a b", "a_b", &"x".repeat(32)] {
            assert!(Volumes::validate_name(bad).is_err(), "{bad}");
        }
        assert_eq!(Volumes::own_name("work"), "work");
        assert_eq!(Volumes::own_name(&format!("{}-b", "a".repeat(30))), "a".repeat(30));
    }

    #[test]
    fn default_folder_is_made_with_mode_a_chosen_one_must_exist() {
        use std::os::unix::fs::PermissionsExt;
        let (_t, p) = temp_paths();
        let v = Volumes::new(p.clone());
        let c = vm("s1", None, true);
        let ready = v.prepare_all(&c, Some(0o777)).unwrap();
        assert_eq!(ready, [("s1".to_string(), p.share("s1"))], "named after its folder");
        assert_eq!(std::fs::metadata(p.share("s1")).unwrap().permissions().mode() & 0o7777, 0o777);
        let mut c = vm("s2", None, true);
        let mine = p.root.join("mine");
        c.share_path = Some(mine.to_string_lossy().into_owned());
        assert!(v.prepare_all(&c, None).is_err());
        std::fs::create_dir_all(&mine).unwrap();
        std::fs::set_permissions(&mine, std::fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(v.prepare_all(&c, Some(0o777)).unwrap(), [("mine".to_string(), mine.clone())]);
        assert_eq!(std::fs::metadata(&mine).unwrap().permissions().mode() & 0o7777, 0o700);
        assert!(!p.share("s2").exists());
    }

    #[test]
    fn a_vm_from_before_has_its_own_volume_at_its_folder() {
        let (_t, p) = temp_paths();
        let v = Volumes::new(p.clone());
        let mut old = vm("old", None, true);
        old.share_path = Some("/elsewhere".into());
        let plain = vm("plain", None, true);
        let none = vm("none", None, false);
        assert_eq!(old.volume_names(), ["old"]);
        assert!(none.volume_names().is_empty());
        let list = v.list(&[old.clone(), plain.clone(), none.clone()]);
        assert_eq!(list.iter().map(|x| x.name.as_str()).collect::<Vec<_>>(), ["old", "plain"]);
        assert_eq!(list[0].folder, PathBuf::from("/elsewhere"));
        assert!(list[0].custom && !list[1].custom);
        assert_eq!(list[1].folder, p.share("plain"));
        assert_eq!(list[1].vms, ["plain"]);
    }

    #[test]
    fn create_attach_detach_remove() {
        let (t, p) = temp_paths();
        let store = InstanceStore::new(p.clone(), "/usr/bin/false");
        let v = Volumes::new(p.clone());
        for name in ["a", "b"] {
            let c = vm(name, Some(vec![]), false);
            std::fs::create_dir_all(p.instance(name).dir).unwrap();
            store.save(&c).unwrap();
        }
        let projects = v.create("projects", None, &store.list()).unwrap();
        assert!(projects.folder.is_dir() && !projects.custom);
        assert!(v.create("projects", None, &store.list()).is_err(), "exists");
        let mine = t.path().join("mine");
        std::fs::create_dir_all(&mine).unwrap();
        let custom = v.create("mine", Some(&mine), &store.list()).unwrap();
        assert!(custom.custom && custom.folder == mine);
        assert!(v.create("gone", Some(&t.path().join("nope")), &store.list()).is_err(), "no such folder");

        v.attach(&store, "projects", "a", None).unwrap();
        v.attach(&store, "projects", "b", Some("Code")).unwrap();
        assert!(v.attach(&store, "mine", "b", Some("code")).is_err(), "one name twice in a VM");
        assert!(v.attach(&store, "mine", "a", Some("a/b")).is_err(), "no valid name");
        v.attach(&store, "mine", "a", None).unwrap();
        assert!(v.attach(&store, "projects", "a", None).is_err(), "twice");
        assert!(v.attach(&store, "nope", "a", None).is_err(), "no such volume");
        assert_eq!(store.load("a").unwrap().volume_names(), ["projects", "mine"]);
        let b = store.load("b").unwrap();
        assert_eq!(v.mounts(&b), [("projects".into(), "Code".into(), projects.folder.clone())]);
        v.rename_mount(&store, "projects", "b", None).unwrap();
        assert_eq!(v.mounts(&store.load("b").unwrap())[0].1, "projects", "back to the folder's name");
        let list = v.list(&store.list());
        assert_eq!(list.iter().find(|x| x.name == "projects").unwrap().vms, ["a", "b"]);

        v.detach(&store, "projects", "b").unwrap();
        assert!(store.load("b").unwrap().volume_names().is_empty());
        std::fs::write(projects.folder.join("keep.txt"), "x").unwrap();
        v.remove(&store, "projects").unwrap();
        assert!(store.load("a").unwrap().volume_names() == ["mine"], "taken out of its VMs");
        assert!(projects.folder.join("keep.txt").exists(), "the user's files stay");
        v.remove(&store, "mine").unwrap();
        assert!(mine.is_dir(), "a chosen folder stays");
    }
}
