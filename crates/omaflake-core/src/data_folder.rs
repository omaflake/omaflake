// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of DataFolder.swift – work package F1. Skeleton (`todo!`).
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
#![allow(unused_variables, dead_code)]

use std::path::Path;

use crate::error::Result;
use crate::models::InstanceConfig;
use crate::paths::Paths;
use crate::Log;

/// Swift `enum DataFolder` (namespace)
pub enum DataFolder {}

impl DataFolder {
    /// Moves the data of `old` to `target` and stores `target` in `settings_file` (Swift default:
    /// `Paths::default_home()/config.json`); returns the new paths. Refuses while an instance is running, if
    /// `target` already holds omaflake data, or across volumes. Swift `move(from:to:settingsFile:log:)`.
    pub fn move_to(old: &Paths, target: &Path, settings_file: &Path, log: &Log) -> Result<Paths> {
        todo!("port from DataFolder.swift")
    }

    /// `path` inside `from` → the same place inside `to`; other paths unchanged.
    pub(crate) fn rebase(path: &str, from: &Path, to: &Path) -> String {
        todo!("port from DataFolder.swift")
    }

    /// QEMU (pid file, executable "qemu") or omaf-runner (runner pid file, "omaf-runner") of the VM is alive
    pub fn is_running(c: &InstanceConfig, paths: &Paths) -> bool {
        todo!("port from DataFolder.swift")
    }

    /// The volume a folder is on ("Cannot determine the volume of <path>" when it cannot be read)
    pub(crate) fn volume(path: &Path) -> Result<u64> {
        todo!("port from DataFolder.swift")
    }
}
