// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of MacBaseInstaller.swift – work package F4. Done: types, constants, paths, `is_ready`. Skeleton: the rest
//! (`todo!`).
//!
//! A macOS base from Apple's IPSW (spec §6.5 step 1, docs/macos-base.md), shared by `omaf macos install` and the app:
//!   `omaf-runner install` → `bases/mac-fresh-<build>` (never started, read-only) → APFS clone `bases/mac-base-<build>`
//!   with a new identity → first boot with provisioning (user, automatic login, SSH; plan BM11) → your SSH public
//!   keys into the guest user's authorized_keys → shut down → read-only. No manual steps (plan BM29).
//! omaflake never distributes macOS itself: the IPSW comes from Apple, the base is built on the user's Mac.
//! Existing folders are reused, never overwritten. The password goes to the runner and ssh/sudo on stdin or via
//! an askpass helper that reads it from its environment – never into a file or an argument.
#![allow(unused_variables, dead_code)]

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::ipsw_downloader::{IpswDownloader, MacOsRelease};
use crate::paths::Paths;
use crate::shell::ShellResult;
use crate::vz_backend::VzBackend;
use crate::{Log, Progress};

#[derive(Debug, Clone)]
pub struct MacBaseInstaller {
    pub paths: Paths,
    pub vz: VzBackend,
}

/// What to install (Swift `MacBaseInstaller.Request`)
#[derive(Debug, Clone)]
pub struct BaseRequest {
    /// Local IPSW; None = the latest supported by this Mac, downloaded from Apple into `~/.omaflake/ipsw/`
    pub ipsw: Option<PathBuf>,
    pub cpus: u32,
    pub memory_gb: u32,
    pub disk_gb: u32,
    pub user: String,
    pub full_name: String,
    /// Password of the new guest user; None = stop after the clone, without the first boot
    pub password: Option<String>,
    /// Public keys for the guest user's authorized_keys (default: ~/.ssh/*.pub)
    pub authorized_keys: Vec<String>,
    /// Show the first boot in a window (headless otherwise)
    pub window: bool,
}

impl BaseRequest {
    /// Swift defaults: 4 CPUs, 8 GB memory, 80 GB disk, no IPSW, the default public keys, headless
    pub fn new(user: &str, full_name: &str, password: Option<String>) -> Self {
        BaseRequest {
            ipsw: None,
            cpus: 4,
            memory_gb: 8,
            disk_gb: 80,
            user: user.into(),
            full_name: full_name.into(),
            password,
            authorized_keys: MacBaseInstaller::default_public_keys(),
            window: false,
        }
    }
}

/// Swift `MacBaseInstaller.Outcome`
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BaseOutcome {
    /// The base for this build existed already; nothing was changed
    Existing(PathBuf),
    /// New base; `ready` = provisioned, keys installed, shut down and read-only
    Created { dir: PathBuf, ready: bool },
}

impl MacBaseInstaller {
    /// Marker of a finished base (keys installed, shut down, read-only); not one of the VM files, so clones skip it.
    /// It holds the guest user's name.
    pub const READY_MARKER: &'static str = ".omaflake-base-ready";

    pub fn new(paths: Paths, vz: VzBackend) -> Self {
        MacBaseInstaller { paths, vz }
    }

    /// The Mac user's SSH public keys (`~/.ssh/*.pub`, sorted, trimmed, only "ssh-…" and "ecdsa-…") – the keys
    /// `omaf ssh` and the image build log in with.
    pub fn default_public_keys() -> Vec<String> {
        todo!("port from MacBaseInstaller.swift")
    }

    /// The latest macOS this Mac can run as a VM (asks Apple, no download), None if unknown.
    pub fn latest_release(&self) -> Option<MacOsRelease> {
        todo!("port from MacBaseInstaller.swift")
    }

    /// Build of the latest IPSW supported by this Mac, None if unknown.
    pub fn latest_build(&self) -> Option<String> {
        todo!("port from MacBaseInstaller.swift")
    }

    /// Downloads the IPSW of `release` into `ipsw/` (continues an interrupted download); `started` gets the
    /// downloader so the caller can pause it (`IpswDownloader::cancel`). Checks free space first (IPSW size or 27 GB,
    /// plus `Prerequisites::MAC_SPACE_NEEDED`). Progress goes to `log` in whole percent ("Download: <n> %").
    pub fn download_ipsw(
        &self,
        release: &MacOsRelease,
        started: &dyn Fn(&IpswDownloader),
        progress: &Progress,
        log: &Log,
    ) -> Result<PathBuf> {
        todo!("port from MacBaseInstaller.swift")
    }

    /// "macOS 27.0.1 (26A434)" in the first line → "26A434"
    pub fn build_from_latest(output: &str) -> Option<String> {
        todo!("port from MacBaseInstaller.swift")
    }

    /// `bases/mac-fresh-<build>`
    pub fn fresh(&self, build: &str) -> PathBuf {
        self.paths.bases().join(format!("mac-fresh-{build}"))
    }

    /// `bases/mac-base-<build>`
    pub fn base(&self, build: &str) -> PathBuf {
        self.paths.bases().join(format!("mac-base-{build}"))
    }

    /// The base that `install` would end at, if it exists already (only known up front without `ipsw`).
    pub fn existing_base(&self, r: &BaseRequest) -> Option<PathBuf> {
        todo!("port from MacBaseInstaller.swift")
    }

    /// Runs all steps (keeping the Mac awake); progress (including the runner's) goes to `log` line by line. Blocks
    /// until the first-boot VM is shut down.
    pub fn install(
        &self,
        r: &BaseRequest,
        started: &dyn Fn(&IpswDownloader),
        progress: &Progress,
        log: &Log,
    ) -> Result<BaseOutcome> {
        todo!("port from MacBaseInstaller.swift")
    }

    pub fn is_ready(dir: &Path) -> bool {
        dir.join(Self::READY_MARKER).exists()
    }

    /// Guest user of a base: written into its ready marker; for older bases from the log of its first boot
    /// (`logs/first-boot-<base folder>.log`).
    pub fn user_of(dir: &Path, logs: &Path) -> Option<String> {
        todo!("port from MacBaseInstaller.swift")
    }

    /// "[00:12:48] Provisioning: user alice, auto-login on, SSH on" → "alice" (the last such line)
    pub(crate) fn user_from_first_boot_log(log: &str) -> Option<String> {
        todo!("port from MacBaseInstaller.swift")
    }

    /// Step 3: boot (the first one with provisioning), keys, shutdown, read-only. The first boot after an
    /// installation sometimes brings SSH up only much later (BM30); then the VM is restarted once – provisioning is
    /// done by then. The runner runs as a child (`Command`, stdin piped for `--password-stdin`, output appended to the first-boot log).
    fn finish(&self, dir: &Path, r: &BaseRequest, password: &str, provision: bool, log: &Log) -> Result<()> {
        todo!("port from MacBaseInstaller.swift")
    }

    /// ssh with password authentication: an askpass helper (`cache/askpass.sh`, no secret in it) prints the password
    /// from its environment (`OMAFLAKE_ASKPASS`), which only this ssh process and its helper get.
    fn password_ssh(&self, host: &str, user: &str, password: &str, command: &str, input: &[u8]) -> Result<ShellResult> {
        todo!("port from MacBaseInstaller.swift")
    }

    /// `omaf-runner install` into `bases/.install-<unix time>`, renamed to mac-fresh-<build> and made read-only.
    fn install_fresh(&self, r: &BaseRequest, ipsw: &Path, log: &Log) -> Result<PathBuf> {
        todo!("port from MacBaseInstaller.swift")
    }
}

/// Reports a percentage only when it changed (log lines in whole percent). Swift `LastPercent`.
#[derive(Debug, Default)]
pub(crate) struct LastPercent {
    last: std::sync::Mutex<Option<i64>>,
}

impl LastPercent {
    pub(crate) fn next(&self, pct: i64) -> Option<i64> {
        todo!("port from MacBaseInstaller.swift")
    }
}
