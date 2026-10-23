// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of QEMURuntime.swift.
//!
//! Not 1:1: Swift took the window's initial resolution from CoreGraphics (`CGDisplayCopyDisplayMode`) and wrote the
//! Dock icon itself (`Brand.dockIconFile`, AppKit). The core writes the Dock icon now: the brand's 512 px app icon is
//! embedded (`DOCK_ICON`) and written to `<data>/cache/dock-icon.png` (`Paths::dock_icon`) when QEMU starts with a
//! window; QEMU gets `OMAFLAKE_DOCK_ICON` pointing at it.
//!
//! The main screen: a UI can tell the core its size with `set_main_screen_resolution`, process-wide (OPEN: the API has
//! no param for it yet). Until then – the CLI, a UI that passes nothing – macOS asks `system_profiler SPDisplaysDataType -json` once (cached for the process): the main display's pixels
//! and its "looks like" size in points give the Retina factor, so the result is Swift's. Elsewhere there is no
//! screen (None: QEMU picks its default size).
//!
//! The start's error kinds: already running → `vm-running`, base image missing → `not-found`, SSH port in use →
//! `in-use`, no virtio-gpu-gl or no UEFI firmware → `prerequisite`.

use std::fs::File;
use std::os::fd::AsRawFd;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::Value;

use crate::agent_protocol::AgentState;
use crate::backend::Readiness;
use crate::error::{codes, os_text, Error, IoContext, Result};
use crate::instance_store::InstanceStore;
use crate::models::{DisplayMode, GlBackend, InstanceConfig, NetworkMode, QemuCapabilities, StopResult, VmStatus};
use crate::paths::{InstancePaths, Paths, Settings};
use crate::platform::{is_executable_file, resolve_symlinks};
use crate::qemu_command::QemuCommand;
use crate::qmp_client::QmpClient;
use crate::shell::{RunOptions, Shell};
use crate::sockets::{self, Ports};
use crate::spawn::{live_pid, reap_if_exited, send_signal, spawn_detached, wait_for_exit, Pid};
use crate::ssh_command::SshCommand;
use crate::vm_err;
use crate::vz_backend::VzBackend;

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
        let settings = Settings::load(paths);
        let env = std::env::var("VM_QEMU").ok();
        let qemu = Shell::which(env.as_deref().or(settings.qemu.as_deref()).unwrap_or("qemu-system-aarch64"))?;
        let resolved = resolve_symlinks(Path::new(&qemu));
        let bin = resolved.parent().unwrap_or(Path::new("/"));
        let firmware = bin.parent().unwrap_or(Path::new("/")).join("share/qemu/edk2-aarch64-code.fd");
        if !firmware.exists() {
            return Err(Error::with_code(
                codes::PREREQUISITE,
                format!("UEFI firmware not found: {}", firmware.display()),
            ));
        }
        let qemu_img_next = bin.join("qemu-img");
        let qemu_img = if is_executable_file(&qemu_img_next) {
            qemu_img_next.to_string_lossy().into_owned()
        } else {
            Shell::which("qemu-img")?
        };
        Ok((QemuRuntime::new(paths.clone(), &qemu, &firmware.to_string_lossy()), qemu_img))
    }

    // MARK: Status

    /// PID of the instance's running QEMU, otherwise None (a stale pidfile is ignored; the process must be
    /// "qemu-system" – PID reuse).
    pub fn pid(&self, name: &str) -> Option<Pid> {
        let ip = self.paths.instance(name);
        // Rule out PID reuse: the process must be QEMU
        live_pid(&ip.pid_file(), "qemu-system")
    }

    pub fn status(&self, name: &str) -> VmStatus {
        if self.pid(name).is_none() {
            VmStatus::Stopped
        } else {
            VmStatus::Running
        }
    }

    // MARK: Start

    /// Capabilities of the binary: GL via `-device help` ("\"virtio-gpu-gl-pci\""), guest_owner via the option string
    /// "guest_owner_uid" in the binary.
    pub fn capabilities(&self) -> QemuCapabilities {
        let devices =
            Shell::run(&self.qemu, &["-device", "help"], &RunOptions::default()).map(|r| r.stdout).unwrap_or_default();
        let binary = std::fs::read(&self.qemu).unwrap_or_default();
        let needle = b"guest_owner_uid";
        QemuCapabilities {
            gl: devices.contains("\"virtio-gpu-gl-pci\""),
            guest_owner: binary.windows(needle.len()).any(|w| w == needle),
        }
    }

    /// Starts QEMU detached (`spawn_detached`, log qemu.log), after removing an earlier run's agent.json; ready as soon
    /// as QMP answers `query-status` (15 s), then the omaf-runner bridge is started. Errors: already running, base
    /// image missing, SSH port in use, no virtio-gpu-gl for `gpu`, "QEMU exited during startup:\n<tail of qemu.log>",
    /// "QEMU does not answer over QMP (<socket>)". The default share folder is opened up (0777) for 9p (plan B18).
    pub fn start(&self, config: &InstanceConfig, display: DisplayMode, gl: GlBackend) -> Result<Pid> {
        let ip = self.paths.instance(&config.name);
        if self.pid(&config.name).is_some() {
            return Err(Error::with_code(codes::VM_RUNNING, format!("\"{}\" is already running", config.name)));
        }
        if InstanceStore::new(self.paths.clone(), "qemu-img").backing_image(config).is_none() {
            return Err(Error::with_code(
                codes::NOT_FOUND,
                format!(
                    "Base image missing: {} – rebuild it (omaf build {}) or import it, then omaf reset",
                    config.image, config.recipe
                ),
            ));
        }
        std::fs::create_dir_all(&ip.run).with_path(&ip.run)?;
        if config.network != NetworkMode::None && !Ports::is_free(config.ssh_port) {
            return Err(Error::with_code(codes::IN_USE, format!("SSH port {} is in use", config.ssh_port)));
        }
        let caps = self.capabilities();
        if display == DisplayMode::Gpu && !caps.gl {
            return Err(Error::with_code(
                codes::PREREQUISITE,
                format!(
                    "{} does not support virtio-gpu-gl – use a custom build (qemu/build.sh, omaf config --qemu …) or --no-gl",
                    self.qemu
                ),
            ));
        }
        let mut share = None;
        if config.share {
            // The default folder is opened up for 9p (plan B18); a folder the user chose keeps its permissions
            share = Some(self.paths.prepare_share(config, Some(0o777))?.to_string_lossy().into_owned());
        }
        let mut cmd = QemuCommand::new(&self.firmware, ip.clone(), share, display);
        cmd.gl = gl;
        cmd.capabilities = caps;
        if display != DisplayMode::None {
            cmd.initial_resolution = Self::main_screen_resolution();
        }
        let args = cmd.arguments(config);
        // Dock icon for the window (patch qemu-cocoa-dock-icon; older builds ignore the variable)
        let icon = self.paths.dock_icon();
        let icon_text = icon.to_string_lossy().into_owned();
        let mut extra_env: Vec<(&str, &str)> = vec![];
        if display != DisplayMode::None && write_dock_icon(&icon) {
            extra_env.push(("OMAFLAKE_DOCK_ICON", &icon_text));
        }
        // What an earlier run's agent reported must not count for this one
        let _ = std::fs::remove_file(ip.agent_state());
        let child = spawn_detached(&self.qemu, &args, &ip.qemu_log(), &extra_env)?;

        // Ready as soon as QMP answers; if QEMU dies first, show the log
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline {
            if reap_if_exited(child) {
                return Err(vm_err!("QEMU exited during startup:\n{}", tail(&ip.qemu_log(), 15)));
            }
            if let Ok(mut qmp) = QmpClient::connect(&ip.qmp_socket(), Duration::from_secs(1)) {
                if qmp.status().is_ok() {
                    self.start_bridge(&ip, config.clipboard_on(), config.camera_on());
                    return Ok(self.pid(&config.name).unwrap_or(child));
                }
            }
            std::thread::sleep(Duration::from_millis(200));
        }
        Err(vm_err!("QEMU does not answer over QMP ({})", ip.qmp_socket().display()))
    }

    /// The Mac side of the agent, clipboard and camera (omaf-runner bridge), detached like QEMU: it ends when the VM
    /// closes a port. Without omaf-runner the VM simply runs without them. OMAFLAKE_CAMERA_TEST_PATTERN=1 sends a test
    /// pattern instead of the camera (tests without camera permission).
    pub(crate) fn start_bridge(&self, ip: &InstancePaths, clipboard: bool, camera: bool) {
        let Ok(runner) = VzBackend::locate_runner(&self.paths) else { return };
        let mut args: Vec<String> = vec![
            "bridge".into(),
            "--agent".into(),
            ip.agent_socket().to_string_lossy().into_owned(),
            "--agent-state".into(),
            ip.agent_state().to_string_lossy().into_owned(),
        ];
        if clipboard {
            args.extend(["--clipboard".into(), ip.clip_socket().to_string_lossy().into_owned()]);
        }
        if camera {
            args.extend(["--camera".into(), ip.camera_socket().to_string_lossy().into_owned()]);
        }
        if camera && std::env::var("OMAFLAKE_CAMERA_TEST_PATTERN").as_deref() == Ok("1") {
            args.push("--camera-test-pattern".into());
        }
        let _ = spawn_detached(&runner, &args, &ip.bridge_log(), &[]);
    }

    /// Initial resolution from the main screen (points × Retina factor, `QemuCommand::initial_resolution_for`), None
    /// without a screen: what `set_main_screen_resolution` stored, else (macOS) `system_profiler` – see the module docs.
    pub(crate) fn main_screen_resolution() -> Option<(u32, u32)> {
        let screen = screen_override().or_else(|| *PROFILED_SCREEN.get_or_init(profiled_screen))?;
        screen.initial_resolution()
    }

    // MARK: Stop

    /// ACPI powerdown → after `timeout` (Swift default 60 s) QMP `quit` → SIGKILL.
    pub fn stop(&self, name: &str, timeout: Duration) -> Result<StopResult> {
        let Some(pid) = self.pid(name) else { return Ok(StopResult::AlreadyStopped) };
        let socket = self.paths.instance(name).qmp_socket();
        if let Ok(mut qmp) = QmpClient::connect(&socket, QmpClient::DEFAULT_TIMEOUT) {
            let _ = qmp.execute("system_powerdown");
            if wait_for_exit(pid, timeout) {
                return Ok(StopResult::Powerdown);
            }
            let _ = qmp.execute("quit");
            if wait_for_exit(pid, Duration::from_secs(5)) {
                return Ok(StopResult::Quit);
            }
        }
        send_signal(pid, libc::SIGKILL);
        let _ = wait_for_exit(pid, Duration::from_secs(5));
        Ok(StopResult::Killed)
    }

    /// What the instance's agent last reported – None while the VM is stopped or before the agent's hello.
    pub fn agent_state(&self, name: &str) -> Option<AgentState> {
        self.pid(name)?;
        AgentState::load(&self.paths.instance(name).agent_state())
    }

    /// Ready once the agent says hello (also without network) or, for images without the agent, SSH answers
    /// (an image with the agent says hello as early as sshd: it gets a moment to win).
    pub fn wait_for_ready(&self, config: &InstanceConfig, timeout: Duration) -> Option<Readiness> {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(state) = self.agent_state(&config.name).filter(AgentState::is_ready) {
                return Some(Readiness::Agent(Box::new(state)));
            }
            if config.network != NetworkMode::None
                && Ports::ssh_banner(SshCommand::DEFAULT_HOST, config.ssh_port, Duration::from_secs(1)).is_some()
            {
                // An image with the agent says hello as early as sshd: give it a moment to win
                if let Some(state) = self.agent_state(&config.name).filter(AgentState::is_ready) {
                    return Some(Readiness::Agent(Box::new(state)));
                }
                return Some(Readiness::Ssh);
            }
            self.pid(&config.name)?;
            std::thread::sleep(Duration::from_secs(1));
        }
        None
    }

    /// Waits until the instance's SSH server sends its banner.
    pub fn wait_for_ssh(&self, config: &InstanceConfig, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if Ports::ssh_banner(SshCommand::DEFAULT_HOST, config.ssh_port, sockets::DEFAULT_TIMEOUT).is_some() {
                return true;
            }
            if self.pid(&config.name).is_none() {
                return false;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        false
    }
}

// MARK: Main screen

/// The main screen: its size in pixels and, when known, in points (the Retina factor is pixels / points)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Screen {
    pub pixels: (u32, u32),
    pub points: Option<(u32, u32)>,
}

impl Screen {
    /// Swift's computation with points and Retina factor; with pixels only, the same on whole pixels (factor 1 – at
    /// most a few pixels off Swift's result, still 16:9 within rounding and even). None for an empty screen.
    pub(crate) fn initial_resolution(&self) -> Option<(u32, u32)> {
        let (w, h) = (self.pixels.0 as f64, self.pixels.1 as f64);
        match self.points {
            Some((pw, ph)) if pw > 0 && ph > 0 => {
                Some(QemuCommand::initial_resolution_for((pw as f64, ph as f64), w / pw as f64))
            }
            _ if w > 0.0 && h > 0.0 => Some(QemuCommand::initial_resolution_for((w, h), 1.0)),
            _ => None,
        }
    }
}

static SCREEN_OVERRIDE: Mutex<Option<Screen>> = Mutex::new(None);
static PROFILED_SCREEN: OnceLock<Option<Screen>> = OnceLock::new();

/// The main screen's size in pixels (e.g. NSScreen.main's frame × backingScaleFactor), for the initial size of QEMU
/// windows. Process-wide; the API server calls it from `initialize` with what the app passes. A later call replaces
/// it; 0 × 0 means "no screen" (QEMU picks its default size, `system_profiler` is not asked).
pub fn set_main_screen_resolution(width: u32, height: u32) {
    *SCREEN_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner()) = Some(Screen { pixels: (width, height), points: None });
}

fn screen_override() -> Option<Screen> {
    *SCREEN_OVERRIDE.lock().unwrap_or_else(|e| e.into_inner())
}

/// macOS: the main display from `system_profiler SPDisplaysDataType -json` (asked once per process)
fn profiled_screen() -> Option<Screen> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let r = Shell::run("/usr/sbin/system_profiler", &["SPDisplaysDataType", "-json"], &RunOptions::default()).ok()?;
    if r.status != 0 {
        return None;
    }
    main_display(r.stdout.as_bytes())
}

/// The main display ("spdisplays_main": "spdisplays_yes", else the first one) of `system_profiler
/// SPDisplaysDataType -json`: "_spdisplays_pixels" ("3024 x 1964") and "_spdisplays_resolution" ("1512 x 982 @
/// 120.00Hz", the size in points). OPEN: these keys are system_profiler's private output format (checked on
/// macOS 27), not an API.
pub(crate) fn main_display(json: &[u8]) -> Option<Screen> {
    fn size(v: &Value) -> Option<(u32, u32)> {
        let text = v.as_str()?;
        let mut parts = text.split(|c: char| !c.is_ascii_digit()).filter(|p| !p.is_empty());
        Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
    }
    let root: Value = serde_json::from_slice(json).ok()?;
    let displays: Vec<&Value> = root
        .get("SPDisplaysDataType")?
        .as_array()?
        .iter()
        .filter_map(|gpu| gpu.get("spdisplays_ndrvs").and_then(Value::as_array))
        .flatten()
        .filter(|d| d.get("_spdisplays_pixels").is_some())
        .collect();
    let main = displays
        .iter()
        .find(|d| d.get("spdisplays_main").and_then(Value::as_str) == Some("spdisplays_yes"))
        .or(displays.first())?;
    Some(Screen {
        pixels: size(main.get("_spdisplays_pixels")?)?,
        points: main.get("_spdisplays_resolution").and_then(size),
    })
}

/// Last lines of a text file ("" when it cannot be read).
/// The app icon QEMU's window shows in the Dock (brand/png/icon/omaflake-icon-512.png, made by brand/generate.py)
pub(crate) const DOCK_ICON: &[u8] = include_bytes!("../../../brand/png/icon/omaflake-icon-512.png");

/// Writes the Dock icon to `file` unless it is there already with these bytes; false when it cannot be written
pub(crate) fn write_dock_icon(file: &Path) -> bool {
    if std::fs::read(file).is_ok_and(|d| d == DOCK_ICON) {
        return true;
    }
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    crate::file_format::write_atomic(file, DOCK_ICON).is_ok()
}

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
    /// Held for the lock: closing it releases the lock
    _file: File,
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
        Ok(FileLock { _file: file, path: path.to_path_buf(), remove_when_done })
    }
}

impl Drop for FileLock {
    fn drop(&mut self) {
        // Unlink while still holding the lock: no one can take a file that is going away
        if self.remove_when_done {
            let _ = std::fs::remove_file(&self.path);
        }
        // `self._file` closes after this, releasing the lock
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;
    use std::time::Instant;

    #[test]
    fn the_dock_icon_is_written_once() {
        let (_dir, paths) = temp_paths();
        let file = paths.dock_icon();
        assert!(DOCK_ICON.starts_with(b"\x89PNG"));
        assert!(write_dock_icon(&file));
        assert_eq!(std::fs::read(&file).unwrap(), DOCK_ICON);
        std::fs::write(&file, b"old").unwrap();
        assert!(write_dock_icon(&file));
        assert_eq!(std::fs::read(&file).unwrap(), DOCK_ICON);
    }

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
                Err(_) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(5)),
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

    use crate::models::tests::config;
    use std::io::Write;
    use std::os::unix::fs::PermissionsExt;

    /// A fake QEMU installation: bin/qemu-system-aarch64 (a script answering `-device help`), bin/qemu-img and
    /// share/qemu/edk2-aarch64-code.fd
    fn fake_qemu(root: &Path, guest_owner: bool, firmware: bool) -> PathBuf {
        let bin = root.join("qemu/bin");
        std::fs::create_dir_all(&bin).unwrap();
        let qemu = bin.join("qemu-system-aarch64");
        let marker = if guest_owner { "# guest_owner_uid" } else { "" };
        std::fs::write(&qemu, format!("#!/bin/sh\necho 'name \"virtio-gpu-gl-pci\", bus PCI'\n{marker}\n")).unwrap();
        std::fs::set_permissions(&qemu, std::fs::Permissions::from_mode(0o755)).unwrap();
        let img = bin.join("qemu-img");
        std::fs::write(&img, "#!/bin/sh\n").unwrap();
        std::fs::set_permissions(&img, std::fs::Permissions::from_mode(0o755)).unwrap();
        if firmware {
            let share = root.join("qemu/share/qemu");
            std::fs::create_dir_all(&share).unwrap();
            std::fs::write(share.join("edk2-aarch64-code.fd"), "").unwrap();
        }
        qemu
    }

    #[test]
    fn locates_qemu_firmware_and_qemu_img() {
        if std::env::var("VM_QEMU").is_ok() {
            return; // the environment wins over config.json
        }
        let (_t, p) = temp_paths();
        let qemu = fake_qemu(&p.root, false, true);
        Settings { qemu: Some(qemu.to_string_lossy().into_owned()), ..Default::default() }.save(&p).unwrap();
        let (rt, img) = QemuRuntime::locate(&p).unwrap();
        assert_eq!(rt.qemu, qemu.to_string_lossy());
        let bin = resolve_symlinks(&qemu).parent().unwrap().to_path_buf();
        assert_eq!(img, bin.join("qemu-img").to_string_lossy());
        assert_eq!(rt.firmware, bin.parent().unwrap().join("share/qemu/edk2-aarch64-code.fd").to_string_lossy());

        let (_t, p) = temp_paths();
        let qemu = fake_qemu(&p.root, false, false);
        Settings { qemu: Some(qemu.to_string_lossy().into_owned()), ..Default::default() }.save(&p).unwrap();
        let e = QemuRuntime::locate(&p).unwrap_err();
        assert!(e.message().starts_with("UEFI firmware not found: "), "{e}");
        assert_eq!(e.code(), Some(codes::PREREQUISITE));
    }

    #[test]
    fn capabilities_of_the_binary() {
        let (_t, p) = temp_paths();
        let with = fake_qemu(&p.root, true, true);
        let caps = QemuRuntime::new(p.clone(), &with.to_string_lossy(), "/fw.fd").capabilities();
        assert_eq!(caps, QemuCapabilities { gl: true, guest_owner: true });
        let (_t, p) = temp_paths();
        let without = fake_qemu(&p.root, false, true);
        let caps = QemuRuntime::new(p.clone(), &without.to_string_lossy(), "/fw.fd").capabilities();
        assert_eq!(caps, QemuCapabilities { gl: true, guest_owner: false });
        let caps = QemuRuntime::new(p.clone(), "/nonexistent/qemu", "/fw.fd").capabilities();
        assert_eq!(caps, QemuCapabilities::default());
    }

    /// A pid file of a process that is not QEMU (this test) counts as stopped (PID reuse)
    #[test]
    fn status_needs_a_qemu_process() {
        let (_t, p) = temp_paths();
        let rt = QemuRuntime::new(p.clone(), "qemu-system-aarch64", "/fw.fd");
        assert_eq!(rt.status("t1"), VmStatus::Stopped);
        let ip = p.instance("t1");
        std::fs::create_dir_all(&ip.run).unwrap();
        std::fs::write(ip.pid_file(), format!("{}\n", std::process::id())).unwrap();
        assert_eq!(rt.pid("t1"), None);
        assert_eq!(rt.status("t1"), VmStatus::Stopped);
        assert_eq!(rt.stop("t1", Duration::from_secs(1)).unwrap(), StopResult::AlreadyStopped);
        // The agent's report counts only while the VM runs
        AgentState::default().save(&ip.agent_state()).unwrap();
        assert_eq!(rt.agent_state("t1"), None);
        let mut c = config("t1");
        c.network = NetworkMode::None;
        assert_eq!(rt.wait_for_ready(&c, Duration::from_secs(5)), None);
    }

    #[test]
    fn ssh_banner_means_ready() {
        let (_t, p) = temp_paths();
        let rt = QemuRuntime::new(p.clone(), "qemu-system-aarch64", "/fw.fd");
        let l = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let mut c = config("t1");
        c.ssh_port = l.local_addr().unwrap().port();
        let t = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut conn, _) = l.accept().unwrap();
                let _ = conn.write_all(b"SSH-2.0-OpenSSH_10.0\r\n");
            }
        });
        assert!(rt.wait_for_ssh(&c, Duration::from_secs(5)));
        assert_eq!(rt.wait_for_ready(&c, Duration::from_secs(5)), Some(Readiness::Ssh));
        t.join().unwrap();
    }

    #[test]
    fn main_display_from_system_profiler() {
        let json = br#"{"SPDisplaysDataType":[{"_name":"Apple M1 Max","spdisplays_ndrvs":[
            {"_name":"Color LCD","_spdisplays_pixels":"3024 x 1964","_spdisplays_resolution":"1512 x 982 @ 120.00Hz"},
            {"_name":"5K","_spdisplays_pixels":"5120 x 2880","_spdisplays_resolution":"2560 x 1440 @ 60.00Hz",
             "spdisplays_main":"spdisplays_yes"}]}]}"#;
        let screen = main_display(json).unwrap();
        assert_eq!(screen, Screen { pixels: (5120, 2880), points: Some((2560, 1440)) });
        assert_eq!(screen.initial_resolution(), Some((3840, 2160)));
        // Without a main display the first one; its Retina factor gives Swift's result
        let first = main_display(&String::from_utf8_lossy(json).replace("spdisplays_main", "x").into_bytes()).unwrap();
        assert_eq!(first.initial_resolution(), Some((2240, 1260)));
        assert_eq!(main_display(br#"{"SPDisplaysDataType":[{"_name":"no display"}]}"#), None);
        assert_eq!(main_display(b"not json"), None);
    }

    #[test]
    fn screen_size_from_the_ui() {
        // Pixels only: computed on whole pixels
        let s = Screen { pixels: (3840, 2160), points: None };
        assert_eq!(s.initial_resolution(), Some((2880, 1620)));
        assert_eq!(Screen { pixels: (0, 0), points: None }.initial_resolution(), None);
        set_main_screen_resolution(3840, 2160);
        assert_eq!(QemuRuntime::main_screen_resolution(), Some((2880, 1620)));
        set_main_screen_resolution(0, 0);
        assert_eq!(QemuRuntime::main_screen_resolution(), None);
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
