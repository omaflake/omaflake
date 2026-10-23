// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of InstanceStore.swift – work package F1.
//!
//! Instances under ~/.omaflake/instances: create, load, reset, remove. instance.json is read and written with
//! `InstanceConfig::FILE_FORMAT` (Swift's `InstanceStore.encoder`/`decoder` – pretty, sorted, `.iso8601` dates – are
//! the format's `encode`/`decode` plus the serde attributes on `InstanceConfig`).
use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{codes, Error, IoContext, Result};
use crate::file_format::{write_atomic, UnreadableFile};
use crate::image_store::{Image, ImageStore};
use crate::json;
use crate::models::{InstanceConfig, RecipeDefaults};
use crate::omaflake_id::OmaflakeId;
use crate::packages::{Package, PackageKind};
use crate::paths::{Paths, Settings};
use crate::platform::{standardize, Platform};
use crate::shell::{RunOptions, Shell};
use crate::sockets::Ports;
use crate::trash::Disposal;
use crate::vm_err;

#[derive(Debug, Clone)]
pub struct InstanceStore {
    pub paths: Paths,
    /// qemu-img (absolute path or a name for `Shell::which`)
    pub qemu_img: String,
}

impl InstanceStore {
    pub const FIRST_SSH_PORT: u16 = 2222;
    /// must match the code firmware (64 MiB)
    pub const EFIVARS_SIZE: u64 = 64 * 1024 * 1024;

    /// What macOS guests and Finder leave in a shared folder on their own – not the user's files
    pub const SHARE_METADATA: [&'static str; 7] = [
        ".fseventsd",
        ".Trashes",
        ".DS_Store",
        ".Spotlight-V100",
        ".TemporaryItems",
        ".localized",
        ".DocumentRevisions-V100",
    ];

    pub fn new(paths: Paths, qemu_img: &str) -> Self {
        InstanceStore { paths, qemu_img: qemu_img.into() }
    }

    /// Lowercase letters, digits, hyphen, 1–32 characters, not starting with "-"; short enough for Unix sockets
    /// (< 104 bytes for the longest, runner.sock, plan B12). Errors: "Invalid name \"<name>\": 1–32 characters a–z,
    /// 0–9, \"-\"", "Path <socket> too long for a Unix socket (max. 103 bytes) – choose a shorter name or VM_HOME".
    pub fn validate(&self, name: &str) -> Result<()> {
        let ok = !name.is_empty()
            && name.chars().count() <= 32
            && !name.starts_with('-')
            && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-');
        if !ok {
            return Err(vm_err!("Invalid name \"{name}\": 1–32 characters a–z, 0–9, \"-\""));
        }
        // longest socket name of both guest kinds (qmp.sock, runner.sock)
        let socket = self.paths.instance(name).runner_socket();
        if socket.as_os_str().len() >= 104 {
            return Err(vm_err!(
                "Path {} too long for a Unix socket (max. 103 bytes) – choose a shorter name or VM_HOME",
                socket.display()
            ));
        }
        Ok(())
    }

    pub fn exists(&self, name: &str) -> bool {
        self.paths.instance(name).config().exists()
    }

    /// instance.json, brought up to date (and written back) if it is older (plan B114). Missing: "Instance \"<name>\"
    /// does not exist".
    pub fn load(&self, name: &str) -> Result<InstanceConfig> {
        let file = self.paths.instance(name).config();
        let Ok(data) = fs::read(&file) else {
            return Err(Error::with_code(codes::NOT_FOUND, format!("Instance \"{name}\" does not exist")));
        };
        let (config, migrated) = InstanceConfig::FILE_FORMAT.decode::<InstanceConfig>(&data)?;
        if migrated {
            let _ = self.save(&config);
        }
        Ok(config)
    }

    /// Refuses a newer format's file (`check_writable`), writes atomically
    pub fn save(&self, config: &InstanceConfig) -> Result<()> {
        let file = self.paths.instance(&config.name).config();
        InstanceConfig::FILE_FORMAT.check_writable(&file)?;
        write_atomic(&file, &InstanceConfig::FILE_FORMAT.encode(config)?)
    }

    /// Names of the VM packages in the data folder and of the external ones whose volume is there (unsorted)
    fn package_names(&self) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(self.paths.instances())
            .map(|d| d.flatten().filter_map(|e| Package::name_of(Path::new(&e.file_name()), PackageKind::Vm)).collect())
            .unwrap_or_default();
        for url in self.paths.external_instances() {
            if let Some(n) = Package::name_of(&url, PackageKind::Vm) {
                if !names.contains(&n) {
                    names.push(n);
                }
            }
        }
        names.sort();
        names
    }

    /// The VMs: packages in the data folder and the external ones whose volume is there, sorted by name
    pub fn list(&self) -> Vec<InstanceConfig> {
        self.package_names().iter().filter_map(|n| self.load(n).ok()).collect()
    }

    /// VM packages this omaflake cannot read – written by a much older or a newer omaflake, or damaged (plan B114)
    pub fn unreadable(&self) -> Vec<UnreadableFile> {
        self.package_names()
            .into_iter()
            .filter(|name| self.exists(name))
            .filter_map(|name| match self.load(&name) {
                Ok(_) => None,
                Err(e) => Some(UnreadableFile::new(name, e.to_string())),
            })
            .collect()
    }

    /// The VM's package lies outside the data folder (e.g. on an external disk, plan B108)
    pub fn is_external(&self, name: &str) -> bool {
        let dir = standardize(&self.paths.instance(name).dir);
        let instances = standardize(&self.paths.instances());
        !dir.to_string_lossy().starts_with(&format!("{}/", instances.to_string_lossy()))
    }

    /// The qcow2 a NixOS overlay is built on: the image inside the package (exported and external VMs carry it),
    /// otherwise the image's package in the data folder; None if neither is there
    pub fn backing_image(&self, c: &InstanceConfig) -> Option<PathBuf> {
        let embedded = self.paths.instance(&c.name).embedded_image().join("disk.qcow2");
        if embedded.exists() {
            return Some(embedded);
        }
        ImageStore::new(self.paths.clone()).image(&c.image).ok().map(|i| i.path)
    }

    /// A VM that carries its image (exported, external) carries the new one after an upgrade
    pub fn replace_embedded_image(&self, name: &str, image: &Image) -> Result<()> {
        let embedded = self.paths.instance(name).embedded_image();
        if !embedded.exists() {
            return Ok(());
        }
        Disposal::delete(&[&embedded])?;
        Package::copy(&self.paths.image_dir(&image.name), &embedded)
    }

    /// Runs an external VM package from where it is: registered in config.json (`externalVMs`)
    pub fn register(&self, package: &Path) -> Result<()> {
        let Some(name) = Package::name_of(package, PackageKind::Vm) else {
            return Err(vm_err!("Not a VM package: {}", package.display()));
        };
        if self.exists(&name) {
            return Err(Error::with_code(codes::EXISTS, format!("A VM named \"{name}\" exists already")));
        }
        let path = package.to_string_lossy().into_owned();
        let mut s = Settings::load_file(&self.paths.config_file);
        let mut list: Vec<String> = s.external_vms.unwrap_or_default().into_iter().filter(|p| *p != path).collect();
        list.push(path);
        s.external_vms = Some(list);
        s.save_file(&self.paths.config_file)
    }

    /// Stops running an external VM from its place – its package stays where it is (nothing is deleted). Works also
    /// while its volume is not there. Its run folder goes.
    pub fn forget(&self, name: &str) -> Result<()> {
        let mut s = Settings::load_file(&self.paths.config_file);
        let is_it = |path: &String| Package::name_of(Path::new(path), PackageKind::Vm).as_deref() == Some(name);
        if !s.external_vms.as_ref().is_some_and(|l| l.iter().any(is_it)) {
            return Err(Error::with_code(codes::NOT_FOUND, format!("{name} is not an external VM")));
        }
        s.external_vms = s.external_vms.map(|l| l.into_iter().filter(|p| !is_it(p)).collect());
        s.save_file(&self.paths.config_file)?;
        let _ = Disposal::delete(&[self.paths.instance(name).run]);
        Ok(())
    }

    /// Next free SSH port: not used by an instance and bindable locally (`is_free`: Swift default `Ports::is_free`).
    pub fn next_ssh_port(&self, is_free: &dyn Fn(u16) -> bool) -> u16 {
        let used: HashSet<u16> = self.list().iter().map(|c| c.ssh_port).collect();
        let mut port = Self::FIRST_SSH_PORT;
        while used.contains(&port) || !is_free(port) {
            port += 1;
        }
        port
    }

    /// Create the instance folder, overlay and UEFI variables; the recipe its image was built from and a new identity.
    /// `ssh_port` None = `next_ssh_port(Ports::is_free)`.
    pub fn create(
        &self,
        name: &str,
        recipe: &str,
        image: &Image,
        settings: &RecipeDefaults,
        ssh_port: Option<u16>,
    ) -> Result<InstanceConfig> {
        self.validate(name)?;
        if self.exists(name) {
            return Err(Error::with_code(codes::EXISTS, format!("Instance \"{name}\" already exists")));
        }
        Self::check_runs_here(image)?;
        let ip = self.paths.instance(name);
        fs::create_dir_all(&ip.dir).with_path(&ip.dir)?;
        fs::create_dir_all(&ip.run).with_path(&ip.run)?;
        // The recipe its image was built from (with its words, plan B110) and a new identity
        let mut config = InstanceConfig::new(
            name,
            if image.recipe().is_empty() { recipe } else { image.recipe() },
            &image.name,
            &image.path.to_string_lossy(),
            &image.info.system,
            settings,
            ssh_port.unwrap_or_else(|| self.next_ssh_port(&Ports::is_free)),
            json::now(),
        );
        config.identity = Some(OmaflakeId::random_words());
        self.create_disks(&config)?;
        self.save(&config)?;
        Ok(config)
    }

    /// Overlay (thin, backing file = image) + empty efivars (`EFIVARS_SIZE`). The backing file is written as a path
    /// relative to the package (plan B108), so the data folder – or a package with its image – can move as a whole.
    pub(crate) fn create_disks(&self, config: &InstanceConfig) -> Result<()> {
        let ip = self.paths.instance(&config.name);
        let Some(backing) = self.backing_image(config) else {
            return Err(Error::with_code(
                codes::NOT_FOUND,
                format!(
                    "Base image missing: {} – rebuild it (omaf build {}) or import it",
                    config.image, config.recipe
                ),
            ));
        };
        let disk = ip.disk().to_string_lossy().into_owned();
        let size = format!("{}G", config.disk_gb);
        let relative = Package::relative_path(&ip.dir, &backing);
        Shell::check(
            &self.qemu_img,
            &["create", "-q", "-f", "qcow2", "-b", &relative, "-F", "qcow2", &disk, &size],
            &RunOptions { cwd: Some(&ip.dir), ..Default::default() },
        )?;
        let efivars = ip.efivars();
        let file = fs::File::create(&efivars).map_err(|_| vm_err!("Could not create {}", efivars.display()))?;
        file.set_len(Self::EFIVARS_SIZE).map_err(|_| vm_err!("Could not set efivars to 64 MiB"))?;
        Ok(())
    }

    /// Reset: the old state is deleted, a fresh overlay from the same image.
    pub fn reset(&self, name: &str) -> Result<()> {
        let config = self.load(name)?;
        let ip = self.paths.instance(name);
        Disposal::delete(&[ip.disk(), ip.efivars(), ip.known_hosts()])?;
        self.create_disks(&config)
    }

    /// Switch to another image (spec §4 "Update"): the old state is deleted, fresh overlay from the new image.
    /// Settings and SSH port are kept.
    pub fn upgrade(&self, name: &str, image: &Image) -> Result<()> {
        let mut config = self.load(name)?;
        if config.image == image.name {
            return Err(vm_err!("\"{name}\" already uses {}", image.name));
        }
        let ip = self.paths.instance(name);
        Disposal::delete(&[ip.disk(), ip.efivars(), ip.known_hosts()])?;
        config.image = image.name.clone();
        config.image_path = image.path.to_string_lossy().into_owned();
        self.replace_embedded_image(name, image)?;
        self.create_disks(&config)?;
        self.save(&config)
    }

    /// Remove an instance: its folder is deleted. Its default share folder goes too when it holds nothing but file system
    /// metadata; with files in it, or when it is a folder the user chose, it is kept (plan B85).
    pub fn remove(&self, name: &str) -> Result<()> {
        if !self.exists(name) {
            return Err(Error::with_code(codes::NOT_FOUND, format!("Instance \"{name}\" does not exist")));
        }
        // Swift: `(try? load(name))?.sharePath == nil` – also true when it cannot be read
        let own_share = self.load(name).map(|c| c.share_path.is_none()).unwrap_or(true);
        let external = self.is_external(name);
        let ip = self.paths.instance(name);
        Disposal::delete(&[&ip.dir, &ip.run])?;
        if external {
            let dir = ip.dir.to_string_lossy().into_owned();
            let mut s = Settings::load_file(&self.paths.config_file);
            s.external_vms = s.external_vms.map(|l| l.into_iter().filter(|p| *p != dir).collect());
            s.save_file(&self.paths.config_file)?;
        }
        if own_share {
            Self::remove_share_if_empty(&self.paths.share(name));
        }
        Ok(())
    }

    /// Deletes a share folder that holds only metadata (`SHARE_METADATA`); returns whether it did.
    pub(crate) fn remove_share_if_empty(dir: &Path) -> bool {
        let Ok(entries) = fs::read_dir(dir) else { return false };
        let only_metadata =
            entries.flatten().all(|e| Self::SHARE_METADATA.iter().any(|m| e.file_name().to_str() == Some(m)));
        only_metadata && Disposal::delete(&[dir]).is_ok()
    }

    /// An image built for another architecture (plan B114) gives no VM here
    pub fn check_runs_here(image: &Image) -> Result<()> {
        if Platform::runs_here(&image.info.system) {
            Ok(())
        } else {
            Err(Platform::not_here(&image.name, &image.info.system))
        }
    }

    /// A VM of another architecture does not start here (Swift `checkRunsHere(_: InstanceConfig)`)
    pub fn check_config_runs_here(c: &InstanceConfig) -> Result<()> {
        if Platform::runs_here(&c.system) {
            Ok(())
        } else {
            Err(Platform::not_here(&format!("\"{}\"", c.name), &c.system))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_store::ImageInfo;
    use crate::models::tests::config;
    use crate::models::GuestKind;

    #[test]
    fn other_architectures_do_not_run_here() {
        let foreign = if Platform::host_system(GuestKind::Nixos) == Some("x86_64-linux") {
            "aarch64-linux"
        } else {
            "x86_64-linux"
        };
        let mut c = config("t1");
        c.system = foreign.into();
        assert!(InstanceStore::check_config_runs_here(&c).is_err());
        let info = ImageInfo::new(
            "omaflake.nixos.minimal",
            None,
            GuestKind::Nixos,
            foreign,
            "",
            "r",
            None,
            crate::json::now(),
        );
        let image = Image { name: "x".into(), info, path: PathBuf::from("/x") };
        assert!(InstanceStore::check_runs_here(&image).is_err());
    }

    use crate::paths::tests::temp_paths;
    use crate::transfer::Transfer;
    use std::os::unix::fs::PermissionsExt;

    #[test]
    fn name_validation() {
        let (_t, p) = temp_paths();
        let store = InstanceStore::new(p, "/usr/bin/false");
        for ok in ["dev", "lab-2", "a1"] {
            store.validate(ok).unwrap();
        }
        for bad in ["", "Dev", "-x", "a b", "ä", &"x".repeat(33)] {
            assert!(store.validate(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn socket_path_limit() {
        let long = Paths::new(format!("/{}", "d".repeat(80)), None);
        let store = InstanceStore::new(long, "/usr/bin/false");
        assert!(store.validate(&"x".repeat(20)).is_err());
    }

    #[test]
    fn save_load_and_ports() {
        let (_t, p) = temp_paths();
        let store = InstanceStore::new(p.clone(), "/usr/bin/false");
        let c = config("t1");
        fs::create_dir_all(p.instance("t1").dir).unwrap();
        store.save(&c).unwrap();
        assert_eq!(store.load("t1").unwrap(), c);
        // 2222 free → first port; ports in use and instance ports are skipped
        assert_eq!(store.next_ssh_port(&|_| true), 2222);
        assert_eq!(store.next_ssh_port(&|p| p >= 2225), 2225);
        let mut c2 = config("t2");
        c2.ssh_port = 2222;
        fs::create_dir_all(p.instance("t2").dir).unwrap();
        store.save(&c2).unwrap();
        assert_eq!(store.next_ssh_port(&|_| true), 2223);
        assert_eq!(store.list().iter().map(|c| c.name.as_str()).collect::<Vec<_>>(), ["t1", "t2"]);
        assert_eq!(store.load("nope").unwrap_err().code(), Some(codes::NOT_FOUND));
    }

    /// Swift RecipeRenameTests.oldFlavorKeyIsRead: instance.json and image.json from before plan B92 say "flavor" (and
    /// have no format, plan B114)
    #[test]
    fn old_flavor_key_is_read() {
        let (_t, p) = temp_paths();
        let store = InstanceStore::new(p.clone(), "/usr/bin/false");
        fs::create_dir_all(p.instance("t1").dir).unwrap();
        store.save(&config("t1")).unwrap();
        let file = p.instance("t1").config();
        let written = fs::read_to_string(&file).unwrap();
        assert!(written.contains("\"recipe\""));
        fs::write(
            &file,
            written
                .replace("\"recipe\"", "\"flavor\"")
                .replace(r#""$schema" : "https://omaflake.dev/schema/instance/v1.json","#, ""),
        )
        .unwrap();
        assert!(!fs::read_to_string(&file).unwrap().contains("$schema"));
        let loaded = store.load("t1").unwrap();
        assert_eq!(loaded.recipe, "dev");
        assert_eq!(loaded, config("t1"));

        let dir = p.images().join("img");
        fs::create_dir_all(&dir).unwrap();
        let info = ImageInfo::new(
            "omaflake.nixos.minimal",
            None,
            GuestKind::Nixos,
            "aarch64-linux",
            "/x",
            "abc",
            None,
            crate::json::now(),
        );
        ImageStore::write_info(&info, &dir).unwrap();
        let json = ImageStore::info_file(&dir);
        let text = fs::read_to_string(&json)
            .unwrap()
            .replace("\"recipe\"", "\"flavor\"")
            .replace(r#""$schema" : "https://omaflake.dev/schema/image/v1.json","#, "");
        assert!(!text.contains("$schema"));
        fs::write(&json, text).unwrap();
        assert_eq!(ImageStore::read_info(&dir).unwrap().recipe, "omaflake.nixos.minimal");
    }

    /// Swift ShareCleanupTests.onlyMetadataGoes
    #[test]
    fn only_metadata_goes() {
        let (_t, p) = temp_paths();
        let (empty, used) = (p.share("a"), p.share("b"));
        fs::create_dir_all(empty.join(".fseventsd")).unwrap();
        fs::write(empty.join(".DS_Store"), "").unwrap();
        fs::create_dir_all(&used).unwrap();
        fs::write(used.join("thesis.tex"), "x").unwrap();
        assert!(InstanceStore::remove_share_if_empty(&empty));
        assert!(!empty.exists());
        assert!(!InstanceStore::remove_share_if_empty(&used));
        assert!(used.join("thesis.tex").exists());
    }

    /// A NixOS image package in the data folder
    fn nixos_image(p: &Paths, name: &str) -> Image {
        let dir = p.image_dir(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("disk.qcow2"), "qcow").unwrap();
        let info = ImageInfo::new(
            "omaflake.nixos.minimal",
            None,
            GuestKind::Nixos,
            Platform::host_system(GuestKind::Nixos).unwrap_or("aarch64-linux"),
            "/x",
            "abc",
            Some("/nix/store/x".into()),
            crate::json::now(),
        );
        ImageStore::write_info(&info, &dir).unwrap();
        ImageStore::new(p.clone()).image(name).unwrap()
    }

    /// A qemu-img stand-in: `create` writes the disk (9th argument) and its arguments next to it
    fn fake_qemu_img(p: &Paths) -> String {
        let fake = p.root.join("qemu-img.sh");
        fs::write(&fake, "#!/bin/sh\n[ \"$1\" = create ] && echo \"$@\" > \"$9.args\" && echo disk > \"$9\"\nexit 0\n")
            .unwrap();
        fs::set_permissions(&fake, fs::Permissions::from_mode(0o755)).unwrap();
        fake.to_string_lossy().into_owned()
    }

    #[test]
    fn create_reset_upgrade_remove() {
        if Platform::host_system(GuestKind::Nixos).is_none() {
            return;
        }
        let (_t, p) = temp_paths();
        let image = nixos_image(&p, "omaflake.nixos.minimal@a");
        let store = InstanceStore::new(p.clone(), &fake_qemu_img(&p));
        let c = store.create("v1", "ignored", &image, &crate::models::tests::dev_defaults(), Some(2300)).unwrap();
        assert_eq!(c.recipe, "omaflake.nixos.minimal");
        assert_eq!(c.ssh_port, 2300);
        assert!(c.identity.as_deref().is_some_and(|i| OmaflakeId::is_words(i, true)));
        let ip = p.instance("v1");
        let args = fs::read_to_string(ip.dir.join("disk.qcow2.args")).unwrap();
        assert!(args.contains("-b ../../images/omaflake.nixos.minimal@a.omaflake-image/disk.qcow2"), "{args}");
        assert!(args.trim_end().ends_with("100G"), "{args}");
        assert_eq!(fs::metadata(ip.efivars()).unwrap().len(), InstanceStore::EFIVARS_SIZE);
        assert_eq!(
            store.create("v1", "x", &image, &crate::models::tests::dev_defaults(), Some(2301)).unwrap_err().code(),
            Some(codes::EXISTS)
        );
        // reset: fresh state
        fs::write(ip.known_hosts(), "k").unwrap();
        store.reset("v1").unwrap();
        assert!(!ip.known_hosts().exists() && ip.disk().exists());
        // upgrade: to another image only
        assert!(store.upgrade("v1", &image).is_err());
        let newer = nixos_image(&p, "omaflake.nixos.minimal@b");
        store.upgrade("v1", &newer).unwrap();
        assert_eq!(store.load("v1").unwrap().image, "omaflake.nixos.minimal@b");
        assert!(fs::read_to_string(ip.dir.join("disk.qcow2.args")).unwrap().contains("minimal@b.omaflake-image"));
        // remove: the package, the run folder and the empty default share go
        fs::create_dir_all(p.share("v1")).unwrap();
        store.remove("v1").unwrap();
        assert!(!ip.dir.exists() && !ip.run.exists() && !p.share("v1").exists());
        assert_eq!(store.remove("v1").unwrap_err().code(), Some(codes::NOT_FOUND));
    }

    #[test]
    fn backing_image_prefers_the_embedded_one() {
        let (_t, p) = temp_paths();
        let store = InstanceStore::new(p.clone(), "/usr/bin/false");
        let mut c = config("v1");
        c.image = "omaflake.nixos.minimal@a".into();
        assert_eq!(store.backing_image(&c), None);
        let image = nixos_image(&p, "omaflake.nixos.minimal@a");
        assert_eq!(store.backing_image(&c), Some(image.path));
        let embedded = p.instance("v1").embedded_image();
        fs::create_dir_all(&embedded).unwrap();
        fs::write(embedded.join("disk.qcow2"), "x").unwrap();
        assert_eq!(store.backing_image(&c), Some(embedded.join("disk.qcow2")));
        // an upgrade puts the new image inside the package
        let newer = nixos_image(&p, "omaflake.nixos.minimal@b");
        store.replace_embedded_image("v1", &newer).unwrap();
        assert_eq!(ImageStore::read_info(&embedded).unwrap(), newer.info);
    }

    #[test]
    fn register_and_forget() {
        let (_t, p) = temp_paths();
        let store = InstanceStore::new(p.clone(), "/usr/bin/false");
        let pkg = p.root.join("stick/e1.omaflake-vm");
        fs::create_dir_all(&pkg).unwrap();
        let mut c = config("e1");
        c.ssh_port = 2240;
        fs::write(pkg.join("instance.json"), InstanceConfig::FILE_FORMAT.encode(&c).unwrap()).unwrap();
        assert!(store.register(&p.root.join("stick/x")).is_err());
        store.register(&pkg).unwrap();
        assert_eq!(p.instance("e1").dir, pkg);
        c.ssh_port = 2241;
        store.save(&c).unwrap();
        assert_eq!(Transfer::read_config(&pkg).unwrap().ssh_port, 2241);
        assert!(store.is_external("e1") && store.exists("e1"));
        assert_eq!(store.register(&pkg).unwrap_err().code(), Some(codes::EXISTS));
        fs::create_dir_all(p.instance("e1").run).unwrap();
        store.forget("e1").unwrap();
        assert!(!p.instance("e1").run.exists() && pkg.join("instance.json").exists());
        assert_eq!(Settings::load_file(&p.config_file).external_vms, Some(vec![]));
        assert_eq!(store.forget("e1").unwrap_err().code(), Some(codes::NOT_FOUND));
    }

    #[test]
    fn unreadable_instances_are_listed() {
        let (_t, p) = temp_paths();
        let store = InstanceStore::new(p.clone(), "/usr/bin/false");
        let dir = p.instance("n1").dir;
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("instance.json"), r#"{"$schema":"https://omaflake.dev/schema/instance/v9.json"}"#).unwrap();
        assert!(store.list().is_empty());
        let u = store.unreadable();
        assert_eq!(u.iter().map(|u| u.name.as_str()).collect::<Vec<_>>(), ["n1"]);
        assert!(u[0].reason.contains("update omaflake"), "{}", u[0].reason);
    }
}
