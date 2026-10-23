// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of MacImageBuilder.swift – work package F4. Done: types and constants. Skeleton: the rest (`todo!`).
//!
//! Builds a macOS image from a base and a nix-darwin recipe (spec §6.5, plan M9/M10; formerly
//! `scripts/macos-build-image.sh`):
//!   build the system closure on the Mac → clone the base (new identity) → start headless → install Nix →
//!   nix copy → activate → shut down → `images/<name>.omaflake-image` (read-only) + `current/<line>`
//!
//! The guest user's password is needed once for sudo. It is never written to a file or passed as an argument:
//! it goes to `sudo -S` on stdin, which installs a temporary sudo rule that is removed again at the end.
//! Cancelling (the `&Cancellation` passed to `build`) forces the build VM off at once (`RunnerClient` "kill", else
//! SIGTERM) – its runner is detached and would outlive omaf otherwise.
#![allow(unused_variables, dead_code)]

use std::path::PathBuf;
use std::time::Duration;

use crate::cancellation::Cancellation;
use crate::error::Result;
use crate::image_store::Image;
use crate::paths::Paths;
use crate::shell::ShellResult;
use crate::spawn::Pid;
use crate::vz_backend::VzBackend;
use crate::Log;

#[derive(Debug, Clone)]
pub struct MacImageBuilder {
    pub paths: Paths,
    pub vz: VzBackend,
}

/// What to build (Swift `MacImageBuilder.Request`)
#[derive(Debug, Clone)]
pub struct BuildRequest {
    pub recipe: String,
    pub flake: String,
    /// Guest user of the base (the recipe's `user`)
    pub user: String,
    pub password: String,
    /// None = newest `bases/mac-base-*`
    pub base: Option<PathBuf>,
    /// Keep a failed build folder for inspection instead of deleting it
    pub keep_failed: bool,
}

impl MacImageBuilder {
    pub fn new(paths: Paths, vz: VzBackend) -> Self {
        MacImageBuilder { paths, vz }
    }

    /// Newest base (`bases/mac-base-<os-build>` with the ready marker), None if there is none.
    pub fn newest_base(&self) -> Option<PathBuf> {
        Self::newest_base_in(&self.paths)
    }

    /// Swift `static newestBase(_:)`: names starting with "mac-base-", sorted descending, the first that
    /// `MacBaseInstaller::is_ready`
    pub fn newest_base_in(paths: &Paths) -> Option<PathBuf> {
        todo!("port from MacImageBuilder.swift")
    }

    /// Runs the whole pipeline (keeping the Mac awake); progress goes to `log`, details (guest output) to
    /// `logs/build-<name>.log`. One build per image at a time (`FileLock` `images/.build-<name>.lock`, removed when
    /// done). Cancelled: "Build cancelled".
    pub fn build(&self, r: &BuildRequest, cancel: &Cancellation, log: &Log) -> Result<Image> {
        todo!("port from MacImageBuilder.swift")
    }

    /// Nix's macOS release tarball in <data>/cache, checked against the SHA-256 that nixos.org publishes next to it.
    pub(crate) fn nix_tarball(&self, version: &str, log: &Log) -> Result<PathBuf> {
        todo!("port from MacImageBuilder.swift")
    }

    /// `nix copy` into the build VM with a watchdog: a copy of ~9 GiB over ssh-ng once stood still after 632 MB with
    /// both sides idle (plan B75). No new path for `stall` (Swift default 300 s) → stop and copy again; paths already
    /// in the guest are skipped, later attempts use the older ssh:// protocol.
    pub(crate) fn copy_closure(
        &self,
        system: &str,
        stores: &[String],
        env: &[(&str, &str)],
        stall: Duration,
        cancel: &Cancellation,
        log: &Log,
    ) -> Result<()> {
        todo!("port from MacImageBuilder.swift")
    }

    /// Steps 3–6 inside the running build VM.
    fn provision(
        &self,
        vm: &mut BuildVm,
        r: &BuildRequest,
        system: &str,
        cancel: &Cancellation,
        log: &Log,
    ) -> Result<()> {
        todo!("port from MacImageBuilder.swift")
    }
}

/// The headless build VM: its runner process (our child) and SSH into it. Swift `MacImageBuilder.BuildVM`.
#[derive(Debug)]
pub(crate) struct BuildVm {
    pub child: Pid,
    pub socket: PathBuf,
    pub user: String,
    pub log_file: PathBuf,
    pub ip: Option<String>,
    done: bool,
}

impl BuildVm {
    /// ssh options for the build VM (also used by `MacBaseInstaller`)
    pub(crate) const SSH_OPTIONS: [&'static str; 10] = [
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "LogLevel=ERROR",
    ];

    pub(crate) fn new(child: Pid, socket: PathBuf, user: &str, log_file: PathBuf) -> Self {
        BuildVm { child, socket, user: user.into(), log_file, ip: None, done: false }
    }

    /// True once the runner has exited (reaps it, so no zombie is left).
    pub(crate) fn exited(&mut self) -> bool {
        todo!("port from MacImageBuilder.swift")
    }

    /// Runner "kill", else SIGTERM; after 15 s SIGKILL
    pub(crate) fn force_off(&mut self) {
        todo!("port from MacImageBuilder.swift")
    }

    /// One command in the guest (remote shell line); stdout/stderr are appended to the build log.
    pub(crate) fn ssh(&self, command: &str, input: Option<&[u8]>) -> Result<ShellResult> {
        todo!("port from MacImageBuilder.swift")
    }

    /// "<what> failed – see <log>" unless the command succeeds
    pub(crate) fn check(&self, command: &str, what: &str) -> Result<()> {
        todo!("port from MacImageBuilder.swift")
    }

    fn append_log(&self, text: &str) {
        todo!("port from MacImageBuilder.swift")
    }
}

/// The build VM's folder while it is built: `images/.build-<name>`
pub(crate) fn work_folder(paths: &Paths, name: &str) -> PathBuf {
    paths.images().join(format!(".build-{name}"))
}
