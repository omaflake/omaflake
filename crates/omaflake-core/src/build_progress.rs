// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of BuildProgress.swift – work package F2. Done: the types and tables. Skeleton: the logic (`todo!`).
#![allow(unused_variables, dead_code)]

use std::sync::Mutex;

use crate::models::GuestKind;

/// Progress of an image build, read from its log lines: the step ("2 of 4") and, while Nix builds or downloads,
/// how many of the announced packages are done. Both build paths log through here (CLI and app).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildProgress {
    pub step: u32,
    pub total: u32,
    pub title: String,
    /// Packages done / announced in the current step (Nix: "these N derivations will be built", "… paths will be fetched")
    pub done: u32,
    pub of: u32,
}

impl BuildProgress {
    pub fn new(step: u32, total: u32, title: &str) -> Self {
        BuildProgress { step, total, title: title.into(), done: 0, of: 0 }
    }

    /// 0…1 over all steps; the current step counts by its packages when Nix announced some
    pub fn fraction(&self) -> f64 {
        todo!("port from BuildProgress.swift")
    }
}

/// Turns build log lines into `BuildProgress`. Steps follow the log messages of `ImageStore::build` (NixOS) and
/// `MacImageBuilder::build` (macOS); Nix's own lines give the package counts. Thread-safe (`feed` takes `&self`).
#[derive(Debug)]
pub struct BuildProgressTracker {
    progress: Mutex<BuildProgress>,
    kind: GuestKind,
}

impl BuildProgressTracker {
    /// macOS steps: the builder's log message that starts each one (prefix, title)
    pub const MAC_STEPS: [(&'static str, &'static str); 7] = [
        ("Building darwinConfigurations", "Building the macOS system on the Mac"),
        ("Cloning ", "Cloning the macOS base"),
        ("Waiting for the VM", "Starting the build VM"),
        ("Installing Nix", "Installing Nix in the guest"),
        ("Copying the closure", "Copying the system into the guest"),
        ("Activating", "Activating the system"),
        ("Shutting down", "Shutting down and storing the image"),
    ];

    /// NixOS: step 1 of 4 "Evaluating the recipe"; macOS: step 1 of 7 "Building the macOS system on the Mac"
    pub fn new(kind: GuestKind) -> Self {
        todo!("port from BuildProgress.swift")
    }

    pub fn progress(&self) -> BuildProgress {
        todo!("port from BuildProgress.swift")
    }

    /// Reads one log line; returns true when the progress changed.
    pub fn feed(&self, line: &str) -> bool {
        todo!("port from BuildProgress.swift")
    }

    /// "these 57 derivations will be built:", "this derivation will be built:", "these 12 paths will be fetched (…)"
    pub(crate) fn announced(line: &str) -> Option<u32> {
        todo!("port from BuildProgress.swift")
    }
}

/// Lines of a build log that say nothing on their own: the disk image tool reports >100 000 "error while reading
/// directory" lines and one "copying path" per store path on every successful NixOS build; Nix lists and downloads
/// thousands of packages one line each – the progress counts those (BuildProgressTracker sees every line). They are
/// left out of what the app and the CLI show; the full log keeps them (plan B83).
pub enum BuildLog {}

impl BuildLog {
    pub fn is_noise(line: &str) -> bool {
        todo!("port from BuildProgress.swift")
    }
}
