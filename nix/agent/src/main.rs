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
        if cfg!(target_os = "macos") {
            concat!("/var/run/omaflake-agent/", $name)
        } else {
            concat!("/run/omaflake-agent/", $name)
        }
    };
}
pub(crate) use concat_runtime;

mod b64;
mod camera;
mod clip;
mod midi;
mod proto;
mod session;
mod status;
mod ump;

use camera::Camera;
use midi::Midi;
use proto::*;
use serde::Deserialize;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::Mutex;
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

/// The port's writing end, shared: CoreMIDI's thread sends MIDI played in the guest straight away (no detour through
/// the poll loop), so every frame is written whole under this lock
static PORT_OUT: Mutex<Option<File>> = Mutex::new(None);
/// The Mac enabled MIDI in its welcome – MIDI from the guest's apps goes out only then
static MIDI_ON: AtomicBool = AtomicBool::new(false);

fn write_port(bytes: &[u8]) {
    let mut out = PORT_OUT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(port) = out.as_mut()
        && let Err(e) = port.write_all(bytes)
    {
        log(&format!("write failed: {e}"));
    }
}

/// A guest app played to one of the virtual destinations (CoreMIDI's thread)
fn midi_out(record: ump::Record) {
    if !MIDI_ON.load(Ordering::SeqCst) {
        return;
    }
    for payload in ump::encode(&[record]) {
        write_port(&encode(MSG_MIDI_DATA, 0, &payload));
    }
}

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
/// What this agent offers in its hello; the Mac enables a subset in its welcome. macOS guests keep their clipboard
/// helper until the agent's pasteboard is checked in a guest (PNG: OPEN, spec §11.9), so the agent there offers MIDI only
fn offered(midi: bool) -> Vec<&'static str> {
    let mut f = vec!["status"];
    if cfg!(target_os = "linux") {
        f.push("clipboard");
        if std::path::Path::new(camera::DEVICE).exists() {
            f.push("camera");
        }
    }
    if midi {
        f.push("midi");
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
    /// The guest's MIDI endpoints for the Mac's (None: no MIDI here – see midi.rs)
    midi: Option<Midi>,
    /// One read buffer for the agent's life: the Linux virtio port gives at most one 4 KiB buffer per read, so a
    /// camera frame takes ~340 reads – a fresh zeroed buffer each time cost more than the frames themselves
    chunk: Vec<u8>,
}

impl Agent {
    fn enabled(&self, feature: &str) -> bool {
        self.welcomed && self.features.iter().any(|f| f == feature)
    }

    fn send_raw(&mut self, bytes: &[u8]) {
        write_port(bytes);
    }

    fn send(&mut self, kind: u16, json: &impl serde::Serialize) {
        let payload = serde_json::to_vec(json).expect("serializable");
        self.send_raw(&encode(kind, 0, &payload));
    }

    fn hello(&mut self) {
        let offer = offered(self.midi.is_some());
        self.send(MSG_HELLO, &status::hello(offer));
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
            self.camera = None; // off for this VM: the device stays black
        }
        let midi = self.enabled("midi");
        MIDI_ON.store(midi, Ordering::SeqCst);
        if !midi && let Some(m) = self.midi.as_mut() {
            m.clear(); // off for this VM: the Mac's devices leave the guest
        }
    }

    /// The Mac's MIDI endpoints changed: the guest's follow, the Mac hears how many exist
    fn midi_endpoints(&mut self, payload: &[u8]) {
        let Some(m) = self.midi.as_mut() else { return };
        let list = match serde_json::from_slice::<midi::EndpointList>(payload) {
            Ok(l) if midi::valid(&l.endpoints) => l.endpoints,
            Ok(_) => return log("MIDI: endpoint list rejected (too long, duplicate ids, unknown values)"),
            Err(e) => return log(&format!("MIDI: malformed endpoint list: {e}")),
        };
        let (count, failed) = m.sync(&list);
        log(&format!("MIDI: {count} endpoints from the Mac{}", if failed.is_empty() { "" } else { " (some failed)" }));
        self.send(MSG_MIDI_STATUS, &serde_json::json!({ "endpoints": count, "failed": failed }));
    }

    /// A message from the Mac
    fn handle(&mut self, frame: Frame) {
        match frame.kind {
            MSG_WELCOME => match serde_json::from_slice::<Welcome>(&frame.payload) {
                Ok(w) if (PROTOCOL_MIN..=PROTOCOL_MAX).contains(&w.protocol) => self.welcome(w),
                Ok(w) => log(&format!(
                    "the Mac asked for protocol {}, this agent speaks {PROTOCOL_MIN}–{PROTOCOL_MAX}",
                    w.protocol
                )),
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
                    return; // late frame after the application closed the camera
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
            MSG_MIDI_ENDPOINTS if self.enabled("midi") => self.midi_endpoints(&frame.payload),
            MSG_MIDI_DATA if self.enabled("midi") => {
                let Some(m) = self.midi.as_mut() else { return };
                match ump::decode(&frame.payload) {
                    Some(records) => {
                        for r in &records {
                            m.deliver(r); // an id no longer in the list: data that crossed a change, dropped
                        }
                    }
                    None => log("MIDI: malformed data from the Mac"),
                }
            }
            MSG_MIDI_PING if self.enabled("midi") && frame.payload.len() <= MAX_MIDI_PING => {
                self.send_raw(&encode(MSG_MIDI_PONG, 0, &frame.payload));
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
        let due = if !self.connected {
            now + Duration::from_secs(1)
        } else if self.welcomed {
            self.next_status
        } else {
            self.next_hello
        };
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

        if let Some(i) = camera_at
            && fds[i].revents & libc::POLLPRI != 0
            && let Some(cam) = self.camera.as_mut()
        {
            match cam.readers() {
                Ok(now_active) if now_active != cam.active => {
                    cam.active = now_active;
                    if !now_active {
                        cam.black(); // kept fresh from now on (`keep_fresh`, B130)
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
            MIDI_ON.store(false, Ordering::SeqCst);
            if let Some(m) = self.midi.as_mut() {
                m.clear(); // the Mac's list comes again with the next welcome
            }
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
        if let Some(cam) = self.camera.as_mut() {
            cam.keep_fresh(now);
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
            let midi = Midi::new(midi_out).is_ok();
            println!("{}", serde_json::to_string_pretty(&status::hello(offered(midi))).unwrap());
            println!("{}", serde_json::to_string_pretty(&status::status()).unwrap());
            return;
        }
        Some("session") => return session::run(),
        _ => {}
    }
    // OMAFLAKE_AGENT_PORT: another port, for checks on a Mac (`omaf-runner midi-check` hands it a pseudo-terminal)
    let path = std::env::var("OMAFLAKE_AGENT_PORT").unwrap_or_else(|_| PORT.to_string());
    let port = match OpenOptions::new().read(true).write(true).custom_flags(libc::O_NOCTTY).open(&path) {
        Ok(f) => f,
        Err(e) => {
            log(&format!("{path}: {e} – not running under omaflake, nothing to do"));
            return;
        }
    };
    unsafe {
        libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
        libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
        // A session that went away must not end the agent while it is written to
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
    // macOS names the port a serial tty (/dev/cu.…): without raw mode the line discipline would echo, translate
    // newlines and eat control bytes of the binary frames (the clipboard helper does the same, plan B48)
    if cfg!(target_os = "macos") {
        // SAFETY: termios is plain data; tcgetattr fills it for the open port, cfmakeraw and tcsetattr change it
        unsafe {
            let mut t: libc::termios = std::mem::zeroed();
            if libc::tcgetattr(port.as_raw_fd(), &mut t) == 0 {
                libc::cfmakeraw(&mut t);
                libc::tcsetattr(port.as_raw_fd(), libc::TCSANOW, &t);
            }
        }
    }
    match port.try_clone() {
        Ok(out) => *PORT_OUT.lock().unwrap_or_else(|e| e.into_inner()) = Some(out),
        Err(e) => return log(&format!("{path}: {e}")),
    }
    let midi = match Midi::new(midi_out) {
        Ok(m) => Some(m),
        Err(e) => {
            log(&format!("no MIDI: {e}"));
            None
        }
    };
    let now = Instant::now();
    let mut agent = Agent {
        midi,
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
    log(&format!("version {}, protocol {PROTOCOL_MIN}–{PROTOCOL_MAX}, port {path}", env!("CARGO_PKG_VERSION")));
    while !STOP.load(Ordering::SeqCst) {
        agent.step();
    }
    if agent.connected {
        agent.send(MSG_GOODBYE, &serde_json::json!({ "reason": "stopping" }));
    }
}
