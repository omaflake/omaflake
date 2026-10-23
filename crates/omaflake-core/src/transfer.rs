// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Transfer.swift – work package F1. Done: the types and constants. Skeleton: the functions (`todo!`).
//!
//! Export and import of packages (spec §11.10, plan B108/B109): a VM, an image or recipes leave the data folder as a
//! package (to an external disk, a NAS, a folder) and come back from one. An import is always a copy into the data
//! folder – the source is never changed; an export changes nothing in the data folder. A VM can also run from where its
//! package is (`use_external`).
//!
//! Who a package is for: a VM carries its user, SSH keys and data, so it goes back to the same person (a backup,
//! another Mac of theirs); an image carries the user it was built for. What people share are recipes.
//!
//! Not 1:1: `is_on_network_volume` used `URLResourceKey.volumeIsLocalKey`; Rust: OPEN – `statfs` `f_flags & MNT_LOCAL`
//! on macOS. Swift wrote an imported VM's instance.json without `$schema` (migrated again on the next load); the port
//! may write it with `InstanceConfig::FILE_FORMAT`.
#![allow(unused_variables, dead_code)]

use std::path::{Path, PathBuf};

use crate::backend::Backends;
use crate::error::Result;
use crate::image_store::ImageStore;
use crate::instance_store::InstanceStore;
use crate::models::InstanceConfig;
use crate::packages::PackageKind;
use crate::paths::Paths;
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
        todo!("port from Transfer.swift")
    }

    /// A stopped VM → `<folder>/<name>.omaflake-vm` (a snapshot: a running VM would change while it is copied),
    /// via `.<name>.omaflake-vm.partial`.
    pub fn export_vm(&self, name: &str, folder: &Path, with_image: bool, log: &Log) -> Result<PathBuf> {
        todo!("port from Transfer.swift")
    }

    /// An image → `<folder>/<name>.omaflake-image`
    pub fn export_image(&self, name: &str, folder: &Path, log: &Log) -> Result<PathBuf> {
        todo!("port from Transfer.swift")
    }

    // MARK: - Import: always a copy into the data folder; the source is never changed

    /// The kind of what `path` holds: by extension, a plain folder with recipes counts as recipes
    pub fn kind_of(path: &Path) -> PackageKind {
        todo!("port from Transfer.swift")
    }

    pub fn conflict(&self, path: &Path) -> Result<Conflict> {
        todo!("port from Transfer.swift")
    }

    /// Imports a package by its kind. A conflict (see `conflict`) is an error unless it is resolved: a VM under
    /// another `name`, an image with `replace`. Recipes whose content is here already are skipped.
    pub fn import_package(&self, path: &Path, name: Option<&str>, replace: bool, log: &Log) -> Result<Imported> {
        todo!("port from Transfer.swift")
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
        todo!("port from Transfer.swift")
    }

    /// instance.json of a VM package (migrated in memory); "<path> is not a VM package (no instance.json)"
    pub fn read_config(package: &Path) -> Result<InstanceConfig> {
        todo!("port from Transfer.swift")
    }

    /// The name of the registered external VM whose package is `path`
    pub(crate) fn external_vm(&self, path: &Path) -> Option<String> {
        todo!("port from Transfer.swift")
    }

    /// A VM package → the data folder. Under its own name it is the same VM again (a backup coming back, the next Mac
    /// of the same person); under another name a second VM with its own identity (SSH port, macOS machine identifier
    /// and MAC). The image it carries becomes an image of the data folder unless that is here already.
    pub(crate) fn import_vm(&self, path: &Path, name: Option<&str>, log: &Log) -> Result<Imported> {
        todo!("port from Transfer.swift")
    }

    /// A macOS VM folder gets a new machine identifier and MAC (omaf-runner clone on the same volume: instant)
    pub(crate) fn new_mac_identity(&self, vm: &Path) -> Result<()> {
        todo!("port from Transfer.swift")
    }

    /// The overlay's backing file → `image`, written relative to the package (`qemu-img rebase -u -F qcow2 -b <rel>`)
    pub(crate) fn rebase(&self, disk: &Path, image: &Path, package: &Path) -> Result<()> {
        todo!("port from Transfer.swift")
    }

    // MARK: - External VMs: run from where they are

    /// On a network volume (NAS): runs, but slower, and the VM breaks when the connection does
    pub fn is_on_network_volume(path: &Path) -> bool {
        todo!("port from Transfer.swift")
    }

    /// Runs a VM package from where it is (an external disk, a stick used on several Macs, a NAS, a folder): it must
    /// carry what it needs (NixOS: its image); its SSH port moves if this Mac uses it already. Nothing is copied, the
    /// package is registered in config.json (`externalVMs`). Returns the VM's name.
    pub fn use_external(&self, path: &Path) -> Result<String> {
        todo!("port from Transfer.swift")
    }
}
