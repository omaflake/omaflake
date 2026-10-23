// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of IPSWDownloader.swift – work package F4. Done: types, `gigabytes`. Skeleton: the rest (`todo!`).
//!
//! Not 1:1 (no URLSession): Swift kept URLSession's opaque resume data in `<build>.ipsw.resume`. The Rust downloader
//! keeps the bytes received so far in `<build>.ipsw.partial` and continues with an HTTP `Range` request (ureq);
//! a Swift `.resume` file cannot be used and is ignored. Prerequisites asks `IpswDownloader::resume_file_for`.
//! `KeepAwake` used `ProcessInfo.beginActivity`; OPEN for Rust: e.g. `caffeinate -i -w <pid>` on macOS, nothing
//! elsewhere. `free_space` used `volumeAvailableCapacityForImportantUsage` (counts purgeable space); OPEN for Rust:
//! `statvfs` (f_bavail × f_frsize) is the plain free space – smaller than what macOS can make available.
#![allow(unused_variables, dead_code)]

use std::path::{Path, PathBuf};

use crate::cancellation::Cancellation;
use crate::error::Result;
use crate::Progress;

/// A macOS release as Apple offers it for this Mac (`omaf-runner latest`, VZMacOSRestoreImage.latestSupported).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacOsRelease {
    pub version: String,
    pub build: String,
    /// https URL of the IPSW
    pub url: String,
    pub bytes: Option<i64>,
}

impl MacOsRelease {
    /// Output of `omaf-runner latest`:
    /// ```text
    /// macOS 27.0.1 (26A434)
    /// URL:   https://updates.cdn-apple.com/…/UniversalMac_27.0.1_26A434_Restore.ipsw
    /// Size:  26.6 GB
    /// ```
    /// None unless the first line is "macOS <version> (<build>)" and the URL is https; Size "<x> GB" → x × 1e9 bytes.
    pub fn parse(text: &str) -> Option<MacOsRelease> {
        todo!("port from IPSWDownloader.swift")
    }

    /// Asks Apple via the runner (`<runner> latest`, needs its entitlement); None when offline or the runner is
    /// unusable.
    pub fn latest(runner: &str) -> Option<MacOsRelease> {
        todo!("port from IPSWDownloader.swift")
    }
}

/// Downloads an IPSW from Apple into `ipsw/<build>.ipsw`. An interrupted or cancelled download keeps what it has in
/// `ipsw/<build>.ipsw.partial` and continues from there next time.
#[derive(Debug, Clone)]
pub struct IpswDownloader {
    pub release: MacOsRelease,
    pub destination: PathBuf,
    cancellation: Cancellation,
}

impl IpswDownloader {
    pub fn new(release: MacOsRelease, directory: &Path) -> Self {
        let destination = Self::file(&release, directory);
        IpswDownloader { release, destination, cancellation: Cancellation::new() }
    }

    /// `<directory>/<build>.ipsw`
    pub fn file(release: &MacOsRelease, directory: &Path) -> PathBuf {
        directory.join(format!("{}.ipsw", release.build))
    }

    /// Where an interrupted download waits: `<destination>.partial`
    pub fn resume_file(&self) -> PathBuf {
        Self::resume_file_for(&self.release, self.destination.parent().unwrap_or(Path::new(".")))
    }

    /// `<directory>/<build>.ipsw.partial`
    pub fn resume_file_for(release: &MacOsRelease, directory: &Path) -> PathBuf {
        directory.join(format!("{}.ipsw.partial", release.build))
    }

    /// Blocks until the file is complete (returns it) or the download failed/was cancelled (what was received is
    /// kept). `progress` gets (received, expected) bytes. Errors as in Swift: "Download paused – it continues from here
    /// next time" (cancelled, code `cancelled`), "Download interrupted (<reason>) – it continues from here next time".
    pub fn run(&self, progress: &Progress) -> Result<PathBuf> {
        todo!("port from IPSWDownloader.swift")
    }

    /// Stops the download (from another thread; clones share the token) and keeps what was received for the next `run`.
    pub fn cancel(&self) {
        self.cancellation.cancel()
    }
}

/// Keeps the Mac awake (no idle sleep) while long work runs: downloads, macOS installation, image builds.
pub enum KeepAwake {}

impl KeepAwake {
    pub fn during<T>(reason: &str, body: impl FnOnce() -> T) -> T {
        todo!("port from IPSWDownloader.swift")
    }
}

/// Free space for the volume of `path` (the nearest existing folder above it); the read-only Nix store volume reports
/// 0 for "important usage" in Swift, where the plain free space was used instead
pub fn free_space(path: &Path) -> Option<i64> {
    todo!("port from IPSWDownloader.swift")
}

/// "26.6 GB" (decimal gigabytes, one digit)
pub fn gigabytes(bytes: i64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}
