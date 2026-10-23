// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of QEMURuntime.swift – work package F2. Done: `tail`, `InstanceLock`, `FileLock` (shared by F4) and their
//! tests. Skeleton: `QemuRuntime` (`todo!`).
//!
//! Not 1:1: Swift took the window's initial resolution from CoreGraphics (`CGDisplayCopyDisplayMode`) and wrote the
//! Dock icon itself (`Brand.dockIconFile`, AppKit). The Dock icon stays with the Swift app: QEMU gets
//! `OMAFLAKE_DOCK_ICON=<data>/cache/dock-icon.png` when that file exists (`Paths::dock_icon`, written by the app).
//! `main_screen_resolution` is OPEN for Rust (CoreGraphics FFI on macOS, or the caller passes it).
#![allow(unused_variables, dead_code)]

use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::agent_protocol::AgentState;
use crate::backend::Readiness;
use crate::error::{codes, os_text, Error, Result};
use crate::models::{DisplayMode, GlBackend, InstanceConfig, QemuCapabilities, StopResult, VmStatus};
use crate::paths::{InstancePaths, Paths};
use crate::spawn::Pid;

/// Starts, stops and monitors QEMU processes. QEMU runs in its own session (setsid)
/// and outlives CLI/app; state is derived only from files/sockets (plan §3).
#[derive(Debug, Clone)]
pub struct QemuRuntime {
    pub paths: Paths,
    pub qemu: String,
    pub firmware: String,
}

impl QemuRuntime {
    pub fn new(paths: Paths, qemu: &str, firmware: &str) -> Self {
        QemuRuntime { paths, qemu: qemu.into(), firmware: firmware.into() }
    }

    /// QEMU from VM_QEMU, config.json or PATH ("qemu-system-aarch64"); firmware (`../share/qemu/edk2-aarch64-code.fd`)
    /// and qemu-img live next to the binary (qemu-img else from PATH). Returns (runtime, qemu-img).
    /// OPEN (x86_64 Linux hosts): the binary and firmware names are aarch64's.
    pub fn locate(paths: &Paths) -> Result<(QemuRuntime, String)> {
        todo!("port from QEMURuntime.swift")
    }

    // MARK: Status

    /// PID of the instance's running QEMU, otherwise None (a stale pidfile is ignored; the process must be
    /// "qemu-system" – PID reuse).
    pub fn pid(&self, name: &str) -> Option<Pid> {
        todo!("port from QEMURuntime.swift")
    }

    pub fn status(&self, name: &str) -> VmStatus {
        todo!("port from QEMURuntime.swift")
    }

    // MARK: Start

    /// Capabilities of the binary: GL via `-device help` ("\"virtio-gpu-gl-pci\""), guest_owner via the option string
    /// "guest_owner_uid" in the binary.
    pub fn capabilities(&self) -> QemuCapabilities {
        todo!("port from QEMURuntime.swift")
    }

    /// Starts QEMU detached (`spawn_detached`, log qemu.log), after removing an earlier run's agent.json; ready as soon
    /// as QMP answers `query-status` (15 s), then the omaf-runner bridge is started. Errors: already running, base
    /// image missing, SSH port in use, no virtio-gpu-gl for `gpu`, "QEMU exited during startup:\n<tail of qemu.log>",
    /// "QEMU does not answer over QMP (<socket>)". The default share folder is opened up (0777) for 9p (plan B18).
    pub fn start(&self, config: &InstanceConfig, display: DisplayMode, gl: GlBackend) -> Result<Pid> {
        todo!("port from QEMURuntime.swift")
    }

    /// The Mac side of the agent, clipboard and camera (omaf-runner bridge), detached like QEMU: it ends when the VM
    /// closes a port. Without omaf-runner the VM simply runs without them. OMAFLAKE_CAMERA_TEST_PATTERN=1 sends a test
    /// pattern instead of the camera (tests without camera permission).
    pub(crate) fn start_bridge(&self, ip: &InstancePaths, clipboard: bool, camera: bool) {
        todo!("port from QEMURuntime.swift")
    }

    /// Initial resolution from the main screen (points × Retina factor, `QemuCommand::initial_resolution_for`), None
    /// without a screen. OPEN (see module docs).
    pub(crate) fn main_screen_resolution() -> Option<(u32, u32)> {
        todo!("port from QEMURuntime.swift")
    }

    // MARK: Stop

    /// ACPI powerdown → after `timeout` (Swift default 60 s) QMP `quit` → SIGKILL.
    pub fn stop(&self, name: &str, timeout: Duration) -> Result<StopResult> {
        todo!("port from QEMURuntime.swift")
    }

    /// What the instance's agent last reported – None while the VM is stopped or before the agent's hello.
    pub fn agent_state(&self, name: &str) -> Option<AgentState> {
        todo!("port from QEMURuntime.swift")
    }

    /// Ready once the agent says hello (also without network) or, for images without the agent, SSH answers
    /// (an image with the agent says hello as early as sshd: it gets a moment to win).
    pub fn wait_for_ready(&self, config: &InstanceConfig, timeout: Duration) -> Option<Readiness> {
        todo!("port from QEMURuntime.swift")
    }

    /// Waits until the instance's SSH server sends its banner.
    pub fn wait_for_ssh(&self, config: &InstanceConfig, timeout: Duration) -> bool {
        todo!("port from QEMURuntime.swift")
    }
}

/// Last lines of a text file ("" when it cannot be read).
pub fn tail(path: &Path, lines: usize) -> String {
    let Ok(text) = std::fs::read_to_string(path) else { return String::new() };
    let all: Vec<&str> = text.split('\n').collect();
    all[all.len().saturating_sub(lines)..].join("\n")
}

/// An exclusive, non-blocking flock on `file` (created 0644, O_CLOEXEC: processes started meanwhile – QEMU, the
/// runner, nix, a shell – must not inherit the descriptor; a flock lasts while any process still has the file open,
/// plan B104). The descriptor is owned only once the lock is held, so a refused lock closes nothing twice.
fn lock_file(file: &Path, busy: &str) -> Result<File> {
    let f = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o644)
        .custom_flags(libc::O_CLOEXEC)
        .open(file)
        .map_err(|_| Error::new(format!("Cannot create lock file: {}", file.display())))?;
    // SAFETY: flock on a descriptor owned by `f`, which stays open while the lock is held
    if unsafe { libc::flock(f.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        let e = std::io::Error::last_os_error();
        // Busy only when someone holds the lock; anything else is an error of its own (not "busy")
        if e.raw_os_error() == Some(libc::EWOULDBLOCK) {
            return Err(Error::with_code(codes::IN_USE, busy));
        }
        return Err(Error::new(format!("Cannot lock {}: {}", file.display(), os_text(&e))));
    }
    Ok(f)
}

/// Exclusive lock per instance (CLI and app at the same time, plan §6). Released when dropped or the process exits.
#[derive(Debug)]
pub struct InstanceLock {
    _file: File,
}

impl InstanceLock {
    pub fn new(paths: &InstancePaths) -> Result<Self> {
        let lock = paths.lock_file();
        if let Some(dir) = lock.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        Ok(InstanceLock { _file: lock_file(&lock, "Instance is currently busy (locked)")? })
    }
}

/// Exclusive lock on a file (flock), released when it is dropped or the process exits – also across app and CLI.
#[derive(Debug)]
pub struct FileLock {
    file: File,
    path: PathBuf,
    /// Removes the lock file when released – for per-build locks whose names are never reused (plan B84)
    remove_when_done: bool,
}

impl FileLock {
    /// `busy`: the error message when someone else holds the lock (code `in-use`)
    pub fn new(path: &Path, busy: &str, remove_when_done: bool) -> Result<Self> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| Error::from(e).context(format!("{}: ", dir.display())))?;
        }
        let file = lock_file(path, busy)?;
        Ok(FileLock { file, path: path.to_path_buf(), remove_when_done })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Unlink while still holding the lock: no one can take a file that is going away
        if self.remove_when_done {
            let _ = std::fs::remove_file(&self.path);
        }
        // `self.file` closes after this, releasing the lock
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;
    use std::time::Instant;

    #[test]
    fn second_build_of_the_same_image_is_refused() {
        let (_t, p) = temp_paths();
        let file = p.images().join(".build-x.lock");
        {
            let first = FileLock::new(&file, "busy", false).unwrap();
            let refused = FileLock::new(&file, "busy", false).unwrap_err();
            assert_eq!((refused.message(), refused.code()), ("busy", Some(codes::IN_USE)));
            drop(first);
        }
        // Free again after the first is gone – within a moment (a process another test starts at the same time may
        // hold a copy of the descriptor until its exec closes it, plan B106)
        let deadline = Instant::now() + Duration::from_secs(1);
        let again = loop {
            match FileLock::new(&file, "busy", false) {
                Ok(l) => break l,
                Err(e) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
                Err(e) => panic!("{e}"),
            }
        };
        drop(again);
    }

    /// A refused lock must not touch the holder's lock file (plan B104: its deinit used to unlink it)
    #[test]
    fn refused_lock_keeps_the_holders_file() {
        let (_t, p) = temp_paths();
        let file = p.images().join(".build-y.lock");
        let holder = FileLock::new(&file, "busy", true).unwrap();
        assert!(FileLock::new(&file, "busy", true).is_err());
        assert!(file.exists());
        drop(holder);
        assert!(!file.exists());
    }

    #[test]
    fn instance_lock_and_tail() {
        let (_t, p) = temp_paths();
        let ip = p.instance("a");
        let l = InstanceLock::new(&ip).unwrap();
        assert_eq!(InstanceLock::new(&ip).unwrap_err().message(), "Instance is currently busy (locked)");
        drop(l);
        let log = p.root.join("x.log");
        std::fs::write(&log, "1\n2\n3\n").unwrap();
        assert_eq!(tail(&log, 2), "3\n");
        assert_eq!(tail(&log, 10), "1\n2\n3\n");
        assert_eq!(tail(&p.root.join("missing"), 3), "");
    }
}
