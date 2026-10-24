// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of DataFolder.swift – work package F1.
//!
//! Moves omaflake's data folder (default ~/.omaflake) to another place and back, e.g. to ~/Documents/omaflake-vms.
//! Only omaflake's own entries move (`Paths::DATA_ENTRIES`); the settings file stays in ~/.omaflake and records
//! the new place (`home`). Nothing is copied or deleted: the folders are renamed, which needs the same volume.
//!
//! Absolute paths that point into the data folder are rewritten afterwards: `image_path` of the instances
//! (informational), a share folder or QEMU/runner path inside the data folder. Images and VMs are self-contained
//! packages and the qcow2 overlays name their image by a relative path (plan B108), so they need nothing.
//!
//! Not 1:1: Swift compared `URLResourceKey.volumeIdentifierKey`; Rust compares the `st_dev` of both folders
//! (`MetadataExt::dev`).

use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::Path;

use crate::error::{codes, IoContext, Result};
use crate::instance_store::InstanceStore;
use crate::models::InstanceConfig;
use crate::paths::{Paths, Settings};
use crate::platform::resolve_symlinks;
use crate::spawn::live_pid;
use crate::vm_err;
use crate::Log;

/// Swift `enum DataFolder` (namespace)
pub enum DataFolder {}

impl DataFolder {
    /// Moves the data of `old` to `target` and stores `target` in `settings_file` (Swift default:
    /// `Paths::default_home()/config.json`); returns the new paths. Refuses while an instance is running, if
    /// `target` already holds omaflake data, or across volumes. Swift `move(from:to:settingsFile:log:)`.
    pub fn move_to(old: &Paths, target: &Path, settings_file: &Path, log: &Log) -> Result<Paths> {
        let from = resolve_symlinks(&old.root);
        let to = resolve_symlinks(target);
        let from_s = from.to_string_lossy().into_owned();
        let to_s = to.to_string_lossy().into_owned();
        if from_s == to_s {
            return Err(vm_err!(codes::EXISTS, "{to_s} is the data folder already"));
        }
        if to_s.starts_with(&format!("{from_s}/")) || from_s.starts_with(&format!("{to_s}/")) {
            return Err(vm_err!("The new data folder must not be inside the old one or contain it"));
        }
        let running: Vec<String> = InstanceStore::new(old.clone(), "qemu-img")
            .list()
            .into_iter()
            .filter(|c| Self::is_running(c, old))
            .map(|c| c.name)
            .collect();
        if !running.is_empty() {
            return Err(vm_err!(codes::VM_RUNNING, "Stop these VMs first: {}", running.join(", ")));
        }

        let entries: Vec<&str> = Paths::DATA_ENTRIES.iter().copied().filter(|e| from.join(e).exists()).collect();
        fs::create_dir_all(&to).with_path(&to)?;
        let clashes: Vec<&str> = entries.iter().copied().filter(|e| to.join(e).exists()).collect();
        if !clashes.is_empty() {
            return Err(vm_err!(
                codes::EXISTS,
                "{to_s} already contains omaflake data ({}) – nothing was moved",
                clashes.join(", ")
            ));
        }
        if !entries.is_empty() && Self::volume(&from)? != Self::volume(&to)? {
            // Copying would expand sparse disks and break APFS clones of macOS images (OPEN: copy with clonefile/sparse)
            return Err(vm_err!("{to_s} is on another volume – moving across volumes is not supported yet"));
        }

        // Settings first (read from the old file, which may be inside the old data folder)
        let mut settings = Settings::load(old);
        settings.home = if to == resolve_symlinks(&Paths::default_home()) { None } else { Some(to_s.clone()) };
        settings.qemu = settings.qemu.map(|q| Self::rebase(&q, &from, &to));
        settings.runner = settings.runner.map(|r| Self::rebase(&r, &from, &to));

        for e in &entries {
            fs::rename(from.join(e), to.join(e)).with_path(&from.join(e))?;
            log(&format!("moved {e}"));
        }
        settings.save_file(settings_file)?;
        let paths = Paths::new(to.clone(), Some(settings_file.to_path_buf()));
        log(&format!("settings: {} (data folder {to_s})", settings_file.display()));

        // Instances: image path (informational), share folder
        let store = InstanceStore::new(paths.clone(), "qemu-img");
        for mut c in store.list() {
            let image = Self::rebase(&c.image_path, &from, &to);
            let share = c.share_path.as_deref().map(|s| Self::rebase(s, &from, &to));
            if image == c.image_path && share == c.share_path {
                continue;
            }
            c.image_path = image.clone();
            c.share_path = share;
            store.save(&c)?;
            log(&format!("instance {} points to {image}", c.name));
        }
        Ok(paths)
    }

    /// `path` inside `from` → the same place inside `to`; other paths unchanged.
    pub(crate) fn rebase(path: &str, from: &Path, to: &Path) -> String {
        let from = from.to_string_lossy();
        match path.strip_prefix(&format!("{from}/")) {
            Some(rest) => format!("{}/{rest}", to.to_string_lossy()),
            None => path.to_string(),
        }
    }

    /// QEMU (pid file, executable "qemu") or omaf-runner (runner pid file, "omaf-runner") of the VM is alive
    pub fn is_running(c: &InstanceConfig, paths: &Paths) -> bool {
        let ip = paths.instance(&c.name);
        live_pid(&ip.pid_file(), "qemu").is_some() || live_pid(&ip.runner_pid(), "omaf-runner").is_some()
    }

    /// The volume a folder is on ("Cannot determine the volume of <path>" when it cannot be read)
    pub(crate) fn volume(path: &Path) -> Result<u64> {
        fs::metadata(path).map(|m| m.dev()).map_err(|_| vm_err!("Cannot determine the volume of {}", path.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::tests::config;
    use crate::models::GuestKind;
    use crate::paths::tests::temp_paths;

    #[test]
    fn moves_own_entries_rewrites_paths_keeps_others() {
        let (_t, tp) = temp_paths();
        let tmp = tp.root.clone();
        let old = Paths::new(tmp.join("a"), None);
        let settings_file = tmp.join("home/config.json");
        let mut c = config("d1");
        c.kind = GuestKind::Macos;
        c.image_path = old.images().join("mac-x-abc-12345678").to_string_lossy().into_owned();
        c.share_path = Some(old.root.join("share/custom").to_string_lossy().into_owned());
        fs::create_dir_all(old.instance("d1").dir).unwrap();
        InstanceStore::new(old.clone(), "/usr/bin/false").save(&c).unwrap();
        Settings::with(
            Some("/f".into()),
            Some(old.root.join("qemu/current/bin/qemu-system-aarch64").to_string_lossy().into_owned()),
            None,
        )
        .save(&old)
        .unwrap();
        fs::write(old.root.join("agent.env"), "secret").unwrap();

        let target = tmp.join("b");
        let new = DataFolder::move_to(&old, &target, &settings_file, &|_| {}).unwrap();
        assert!(new.instance("d1").config().exists());
        assert!(!old.instances().exists());
        // not omaflake's: stays behind
        assert!(old.root.join("agent.env").exists());
        let moved = InstanceStore::new(new.clone(), "/usr/bin/false").load("d1").unwrap();
        assert_eq!(moved.image_path, new.images().join("mac-x-abc-12345678").to_string_lossy());
        assert_eq!(moved.share_path.as_deref(), Some(&*new.root.join("share/custom").to_string_lossy()));
        let s = Settings::load_file(&settings_file);
        assert_eq!(s.home.as_deref(), Some(&*new.root.to_string_lossy()));
        assert_eq!(s.flake.as_deref(), Some("/f"));
        assert_eq!(s.qemu.as_deref(), Some(&*new.root.join("qemu/current/bin/qemu-system-aarch64").to_string_lossy()));
    }

    #[test]
    fn refuses_clashes_and_nesting() {
        let (_t, tp) = temp_paths();
        let tmp = tp.root.clone();
        let old = Paths::new(tmp.join("a"), None);
        let settings_file = tmp.join("home/config.json");
        fs::create_dir_all(old.images()).unwrap();
        let target = tmp.join("b");
        fs::create_dir_all(target.join("images")).unwrap();
        let e = DataFolder::move_to(&old, &target, &settings_file, &|_| {}).unwrap_err();
        assert_eq!(e.code(), Some(codes::EXISTS));
        assert!(old.images().exists());
        assert!(DataFolder::move_to(&old, &old.root.join("x"), &settings_file, &|_| {}).is_err());
        assert!(DataFolder::move_to(&old, &old.root, &settings_file, &|_| {}).is_err());
    }

    #[test]
    fn rebase_only_inside() {
        let from = Path::new("/d/a");
        let to = Path::new("/e/b");
        assert_eq!(DataFolder::rebase("/d/a/images/x", from, to), "/e/b/images/x");
        assert_eq!(DataFolder::rebase("/d/ab/x", from, to), "/d/ab/x");
        assert_eq!(DataFolder::rebase("/d/a", from, to), "/d/a");
    }
}
