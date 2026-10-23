// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of InstanceStore.swift – work package F1. Done: the type, constants, `check_runs_here`,
//! `check_config_runs_here`. Skeleton: the rest (`todo!`).
//!
//! Instances under ~/.omaflake/instances: create, load, reset, remove. instance.json is read and written with
//! `InstanceConfig::FILE_FORMAT` (Swift's `InstanceStore.encoder`/`decoder` – pretty, sorted, `.iso8601` dates – are
//! the format's `encode`/`decode` plus the serde attributes on `InstanceConfig`).
#![allow(unused_variables, dead_code)]

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::file_format::UnreadableFile;
use crate::image_store::Image;
use crate::models::{InstanceConfig, RecipeDefaults};
use crate::paths::Paths;
use crate::platform::Platform;

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
        todo!("port from InstanceStore.swift")
    }

    pub fn exists(&self, name: &str) -> bool {
        todo!("port from InstanceStore.swift")
    }

    /// instance.json, brought up to date (and written back) if it is older (plan B114). Missing: "Instance \"<name>\"
    /// does not exist".
    pub fn load(&self, name: &str) -> Result<InstanceConfig> {
        todo!("port from InstanceStore.swift")
    }

    /// Refuses a newer format's file (`check_writable`), writes atomically
    pub fn save(&self, config: &InstanceConfig) -> Result<()> {
        todo!("port from InstanceStore.swift")
    }

    /// The VMs: packages in the data folder and the external ones whose volume is there, sorted by name
    pub fn list(&self) -> Vec<InstanceConfig> {
        todo!("port from InstanceStore.swift")
    }

    /// VM packages this omaflake cannot read – written by a much older or a newer omaflake, or damaged (plan B114)
    pub fn unreadable(&self) -> Vec<UnreadableFile> {
        todo!("port from InstanceStore.swift")
    }

    /// The VM's package lies outside the data folder (e.g. on an external disk, plan B108)
    pub fn is_external(&self, name: &str) -> bool {
        todo!("port from InstanceStore.swift")
    }

    /// The qcow2 a NixOS overlay is built on: the image inside the package (exported and external VMs carry it),
    /// otherwise the image's package in the data folder; None if neither is there
    pub fn backing_image(&self, c: &InstanceConfig) -> Option<PathBuf> {
        todo!("port from InstanceStore.swift")
    }

    /// A VM that carries its image (exported, external) carries the new one after an upgrade
    pub fn replace_embedded_image(&self, name: &str, image: &Image) -> Result<()> {
        todo!("port from InstanceStore.swift")
    }

    /// Runs an external VM package from where it is: registered in config.json (`externalVMs`)
    pub fn register(&self, package: &Path) -> Result<()> {
        todo!("port from InstanceStore.swift")
    }

    /// Stops running an external VM from its place – its package stays where it is (nothing is deleted). Works also
    /// while its volume is not there. Its run folder goes.
    pub fn forget(&self, name: &str) -> Result<()> {
        todo!("port from InstanceStore.swift")
    }

    /// Next free SSH port: not used by an instance and bindable locally (`is_free`: Swift default `Ports::is_free`).
    pub fn next_ssh_port(&self, is_free: &dyn Fn(u16) -> bool) -> u16 {
        todo!("port from InstanceStore.swift")
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
        todo!("port from InstanceStore.swift")
    }

    /// Overlay (thin, backing file = image) + empty efivars (`EFIVARS_SIZE`). The backing file is written as a path
    /// relative to the package (plan B108), so the data folder – or a package with its image – can move as a whole.
    pub(crate) fn create_disks(&self, config: &InstanceConfig) -> Result<()> {
        todo!("port from InstanceStore.swift")
    }

    /// Reset: the old state is deleted, a fresh overlay from the same image.
    pub fn reset(&self, name: &str) -> Result<()> {
        todo!("port from InstanceStore.swift")
    }

    /// Switch to another image (spec §4 "Update"): the old state is deleted, fresh overlay from the new image.
    /// Settings and SSH port are kept.
    pub fn upgrade(&self, name: &str, image: &Image) -> Result<()> {
        todo!("port from InstanceStore.swift")
    }

    /// Remove an instance: its folder is deleted. Its default share folder goes too when it holds nothing but file system
    /// metadata; with files in it, or when it is a folder the user chose, it is kept (plan B85).
    pub fn remove(&self, name: &str) -> Result<()> {
        todo!("port from InstanceStore.swift")
    }

    /// Deletes a share folder that holds only metadata (`SHARE_METADATA`); returns whether it did.
    pub(crate) fn remove_share_if_empty(dir: &Path) -> bool {
        todo!("port from InstanceStore.swift")
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
}
