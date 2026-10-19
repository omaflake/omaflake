// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaflake agent (guest side): one process per guest, talking to omaflake on the Mac over the virtio port
//! org.omaflake.agent (spec §11.9): hello and periodic status, the clipboard (for the user's Wayland session, through
//! `omaflake-agent session`) and the Mac's camera as /dev/video42.
//!
//! The port reports POLLHUP while nothing listens on the Mac's end. Each time the Mac side connects, the agent
//! says hello (again every few seconds until a welcome comes back), then sends its status at the interval the
//! welcome asks for. The welcome says which features the Mac enabled for this VM; messages of other features are
//! dropped on both sides. Whatever comes from the Mac is only parsed, never executed.

/// A file in the agent's runtime directory (/run/omaflake-agent, macOS /var/run/omaflake-agent)
macro_rules! concat_runtime {
    ($name:literal) => {
        if cfg!(target_os = "macos") { concat!("/var/run/omaflake-agent/", $name) } else { concat!("/run/omaflake-agent/", $name) }
    };
}
pub(crate) use concat_runtime;

mod b64;
mod camera;
mod clip;
mod proto;
mod session;
mod status;

use camera::Camera;
use proto::*;
use serde::Deserialize;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

#[cfg(target_os = "linux")]
const PORT: &str = "/dev/virtio-ports/org.omaflake.agent";
/// macOS guests: the virtio console port omaf-runner attaches (Virtualization.framework names it after the port)
#[cfg(target_os = "macos")]
const PORT: &str = "/dev/cu.org.omaflake.agent";
const HELLO_RETRY: Duration = Duration::from_secs(5);
const DEFAULT_INTERVAL: u64 = 5;
/// The features the Mac enabled, readable by the users' tools (in the agent's runtime directory)
const WELCOME_FILE: &str = concat_runtime!("welcome.json");

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

fn log(msg: &str) {
    eprintln!("omaflake-agent: {msg}");
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct Welcome {
    protocol: u32,
    #[serde(default)]
    features: Vec<String>,
    status_interval: Option<u64>,
    host: Option<String>,
}

/// What this agent offers in its hello; the Mac enables a subset in its welcome
fn offered() -> Vec<&'static str> {
    let mut f = vec!["status", "clipboard"];
    if cfg!(target_os = "linux") && std::path::Path::new(camera::DEVICE).exists() {
        f.push("camera");
    }
    f
}

/// A session process (the clipboard of a user's Wayland session) connected over SOCKET
struct Session {
    stream: UnixStream,
    decoder: Decoder,
    parts: Reassembler,
}

struct Agent {
    port: File,
    decoder: Decoder,
    parts: Reassembler,
    connected: bool,
    welcomed: bool,
    /// Features the Mac enabled in its welcome
    features: Vec<String>,
    interval: Duration,
    next_hello: Instant,
    next_status: Instant,
    listener: Option<UnixListener>,
    sessions: Vec<Session>,
    camera: Option<Camera>,
    /// One read buffer for the agent's life: the Linux virtio port gives at most one 4 KiB buffer per read, so a
    /// camera frame takes ~340 reads – a fresh zeroed buffer each time cost more than the frames themselves
    chunk: Vec<u8>,
}

impl Agent {
    fn enabled(&self, feature: &str) -> bool {
        self.welcomed && self.features.iter().any(|f| f == feature)
    }

    fn send_raw(&mut self, bytes: &[u8]) {
        if let Err(e) = self.port.write_all(bytes) {
            log(&format!("write failed: {e}"));
        }
    }

    fn send(&mut self, kind: u16, json: &impl serde::Serialize) {
        let payload = serde_json::to_vec(json).expect("serializable");
        self.send_raw(&encode(kind, 0, &payload));
    }

    fn hello(&mut self) {
        self.send(MSG_HELLO, &status::hello(offered()));
        self.next_hello = Instant::now() + HELLO_RETRY;
    }

    /// The camera is in use: ask the Mac to start and grant it the frames it may send
    fn camera_start(&mut self) {
        self.send(MSG_CAMERA_CONTROL, &serde_json::json!({ "action": "start" }));
        self.send(MSG_CAMERA_CREDIT, &serde_json::json!({ "credits": camera::CREDITS }));
    }

    fn welcome(&mut self, w: Welcome) {
        self.interval = Duration::from_secs(w.status_interval.unwrap_or(DEFAULT_INTERVAL).clamp(1, 60));
        self.welcomed = true;
        self.features = w.features;
        self.next_status = Instant::now();
        // What is on for this VM, for the guest's own tools (e.g. the lab recipe's "omaflake Test")
        let _ = std::fs::write(WELCOME_FILE, serde_json::json!({ "features": self.features }).to_string());
        log(&format!(
            "welcome from {} (protocol {}, features {:?}, status every {} s)",
            w.host.as_deref().unwrap_or("the Mac"),
            w.protocol,
            self.features,
            self.interval.as_secs()
        ));
        if self.enabled("camera") {
            if self.camera.is_none() {
                match Camera::open() {
                    Ok(c) => self.camera = Some(c),
                    Err(e) => log(&format!("{}: {e} – no camera", camera::DEVICE)),
                }
            }
            if self.camera.as_ref().is_some_and(|c| c.active) {
                self.camera_start();
            }
        } else {
            self.camera = None;   // off for this VM: the device stays black
        }
    }

    /// A message from the Mac
    fn handle(&mut self, frame: Frame) {
        match frame.kind {
            MSG_WELCOME => match serde_json::from_slice::<Welcome>(&frame.payload) {
                Ok(w) if (PROTOCOL_MIN..=PROTOCOL_MAX).contains(&w.protocol) => self.welcome(w),
                Ok(w) => log(&format!("the Mac asked for protocol {}, this agent speaks {PROTOCOL_MIN}–{PROTOCOL_MAX}", w.protocol)),
                Err(e) => log(&format!("malformed welcome: {e}")),
            },
            MSG_CLIPBOARD if self.enabled("clipboard") => {
                let Some(payload) = self.parts.push(frame, clip::MAX_MESSAGE) else { return };
                if clip::decode(&payload).is_none() {
                    return;
                }
                // To every session (one per logged-in user); a session that does not take it is dropped
                let bytes = encode_message(MSG_CLIPBOARD, &payload);
                self.sessions.retain_mut(|s| s.stream.write_all(&bytes).is_ok());
            }
            MSG_CAMERA_FRAME if self.enabled("camera") => {
                let Some(cam) = self.camera.as_mut() else { return };
                if !cam.active {
                    return;   // late frame after the application closed the camera
                }
                if let Err(e) = cam.write_frame(&frame.payload) {
                    log(&format!("camera frame: {e}"));
                }
                // One buffer free again: one more frame may come
                self.send(MSG_CAMERA_CREDIT, &serde_json::json!({ "credits": 1 }));
            }
            MSG_CAMERA_STATUS if self.enabled("camera") => {
                log(&format!("Mac camera: {}", String::from_utf8_lossy(&frame.payload)));
            }
            // Unknown types, or a feature the Mac did not enable: skipped
            other => log(&format!("ignored message type {other:#06x}")),
        }
    }

    /// A message from a session process (its user's clipboard) – passed on only if the Mac enabled the clipboard
    fn handle_session(&mut self, i: usize, frame: Frame) {
        if frame.kind != MSG_CLIPBOARD {
            return;
        }
        let Some(payload) = self.sessions[i].parts.push(frame, clip::MAX_MESSAGE) else { return };
        if self.enabled("clipboard") && clip::decode(&payload).is_some() {
            let bytes = encode_message(MSG_CLIPBOARD, &payload);
            self.send_raw(&bytes);
        }
    }

    fn accept_sessions(&mut self) {
        let Some(listener) = &self.listener else { return };
        while let Ok((stream, _)) = listener.accept() {
            let _ = stream.set_write_timeout(Some(Duration::from_secs(2)));
            let _ = stream.set_nonblocking(false);
            log("a session connected (clipboard)");
            self.sessions.push(Session { stream, decoder: Decoder::default(), parts: Reassembler::default() });
        }
    }

    /// One round: wait for input or the next due message, at most one second
    fn step(&mut self) {
        let now = Instant::now();
        let due = if !self.connected { now + Duration::from_secs(1) }
            else if self.welcomed { self.next_status } else { self.next_hello };
        let timeout = due.saturating_duration_since(now).min(Duration::from_secs(1));
        // port, listener, camera, then the sessions
        let mut fds = vec![libc::pollfd { fd: self.port.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
        let listener_at = self.listener.as_ref().map(|l| {
            fds.push(libc::pollfd { fd: l.as_raw_fd(), events: libc::POLLIN, revents: 0 });
            fds.len() - 1
        });
        let camera_at = self.camera.as_ref().map(|c| {
            fds.push(libc::pollfd { fd: c.fd(), events: libc::POLLPRI, revents: 0 });
            fds.len() - 1
        });
        let sessions_at = fds.len();
        for s in &self.sessions {
            fds.push(libc::pollfd { fd: s.stream.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        }
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout.as_millis() as libc::c_int) };
        if rc < 0 {
            return; // EINTR (a signal) – the caller checks STOP
        }

        if let Some(i) = camera_at {
            if fds[i].revents & libc::POLLPRI != 0 {
                if let Some(cam) = self.camera.as_mut() {
                    match cam.readers() {
                        Ok(now_active) if now_active != cam.active => {
                            cam.active = now_active;
                            if !now_active {
                                cam.black();
                            }
                            log(if now_active { "an application opened the camera" } else { "camera no longer in use" });
                            if now_active {
                                self.camera_start();
                            } else {
                                self.send(MSG_CAMERA_CONTROL, &serde_json::json!({ "action": "stop" }));
                            }
                        }
                        Ok(_) => {}
                        Err(e) => log(&format!("camera event: {e}")),
                    }
                }
            }
        }
        // Sessions: read what came, drop those that went away (from the back, so the indices stay valid). Only the
        // sessions that were polled – new ones are accepted below, after this loop
        let polled = fds.len() - sessions_at;
        for i in (0..polled).rev() {
            let revents = fds[sessions_at + i].revents;
            if revents == 0 {
                continue;
            }
            match self.sessions[i].stream.read(&mut self.chunk) {
                Ok(n) if n > 0 => {
                    self.sessions[i].decoder.feed(&self.chunk[..n]);
                    while let Some(frame) = self.sessions[i].decoder.next_frame() {
                        self.handle_session(i, frame);
                    }
                }
                Err(e) if e.kind() == ErrorKind::Interrupted => {}
                _ => {
                    log("a session went away");
                    self.sessions.remove(i);
                }
            }
        }

        if listener_at.is_some_and(|i| fds[i].revents & libc::POLLIN != 0) {
            self.accept_sessions();
        }

        let hup = fds[0].revents & (libc::POLLHUP | libc::POLLERR) != 0;
        if hup {
            if self.connected {
                log("the Mac side went away");
            }
            self.connected = false;
            self.welcomed = false;
            self.decoder = Decoder::default();
            self.parts = Reassembler::default();
            // POLLHUP returns at once – wait a little before looking again
            std::thread::sleep(Duration::from_secs(1));
            return;
        }
        if !self.connected {
            self.connected = true;
            log("the Mac side is there – saying hello");
            self.hello();
        }
        if fds[0].revents & libc::POLLIN != 0 {
            match self.port.read(&mut self.chunk) {
                Ok(0) => {}
                Ok(n) => {
                    self.decoder.feed(&self.chunk[..n]);
                    while let Some(frame) = self.decoder.next_frame() {
                        self.handle(frame);
                    }
                }
                Err(e) => log(&format!("read failed: {e}")),
            }
        }
        let now = Instant::now();
        if !self.welcomed && now >= self.next_hello {
            self.hello();
        } else if self.welcomed && now >= self.next_status {
            self.send(MSG_STATUS, &status::status());
            self.next_status = now + self.interval;
        }
    }
}

/// The socket session processes connect to; without it (no /run/omaflake-agent, e.g. started by hand) no clipboard
fn listen() -> Option<UnixListener> {
    // macOS has no RuntimeDirectory: the agent makes its folder (Linux: systemd does, for group "users")
    if cfg!(target_os = "macos") {
        let _ = std::fs::create_dir_all(session::RUNTIME);
        let _ = std::fs::set_permissions(session::RUNTIME, std::fs::Permissions::from_mode(0o755));
    }
    let _ = std::fs::remove_file(session::SOCKET);
    match UnixListener::bind(session::SOCKET) {
        Ok(l) => {
            // The users' sessions may connect: the service runs with group "users" (vm-guest.nix)
            let _ = std::fs::set_permissions(session::SOCKET, std::fs::Permissions::from_mode(0o660));
            let _ = l.set_nonblocking(true);
            Some(l)
        }
        Err(e) => {
            log(&format!("{}: {e} – no clipboard", session::SOCKET));
            None
        }
    }
}

fn main() {
    match std::env::args().nth(1).as_deref() {
        Some("--print") => {
            // Diagnostics: what the agent would send, without a port
            println!("{}", serde_json::to_string_pretty(&status::hello(offered())).unwrap());
            println!("{}", serde_json::to_string_pretty(&status::status()).unwrap());
            return;
        }
        Some("session") => return session::run(),
        _ => {}
    }
    let port = match OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOCTTY).open(PORT) {
        Ok(f) => f,
        Err(e) => {
            log(&format!("{PORT}: {e} – not running under omaflake, nothing to do"));
            return;
        }
    };
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        // A session that went away must not end the agent while it is written to
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    let now = Instant::now();
    let mut agent = Agent {
        port,
        decoder: Decoder::default(),
        parts: Reassembler::default(),
        connected: false,
        welcomed: false,
        features: vec![],
        interval: Duration::from_secs(DEFAULT_INTERVAL),
        next_hello: now,
        next_status: now,
        listener: listen(),
        sessions: vec![],
        camera: None,
        chunk: vec![0u8; 1 << 20],
    };
    log(&format!("version {}, protocol {PROTOCOL_MIN}–{PROTOCOL_MAX}, port {PORT}", env!("CARGO_PKG_VERSION")));
    while !STOP.load(Ordering::SeqCst) {
        agent.step();
    }
    if agent.connected {
        agent.send(MSG_GOODBYE, &serde_json::json!({ "reason": "stopping" }));
    }
}
