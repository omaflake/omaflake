// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaflake agent (guest side): one process per guest, talking to omaflake on the Mac over the virtio port
//! org.omaflake.agent (spec §11.9): hello and periodic status, the clipboard (for the user's Wayland session, through
//! `omaflake-agent session`), the Mac's camera (Linux: /dev/video42; macOS: a stream for OBS until omaflake's camera
//! extension – camera_mac.rs, obs.rs), the Mac's MIDI endpoints (CoreMIDI on macOS, the
//! ALSA sequencer on Linux – midi.rs, midi_alsa.rs) and the guest's half of the VM diagnosis (diagnose.rs).
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
#[cfg_attr(target_os = "macos", path = "camera_mac.rs")]
mod camera;
mod clip;
mod diagnose;
mod midi;
#[cfg(any(not(target_os = "macos"), test))]
mod midi1;
#[cfg(not(target_os = "macos"))]
mod midi_alsa;
mod midi_clock;
mod obs;
mod proto;
mod session;
mod status;
mod ump;

use camera::Camera;
use midi::Midi;
use proto::*;
use serde::Deserialize;
use std::collections::VecDeque;
use std::fs::{File, OpenOptions};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::sync::atomic::AtomicUsize;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
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

/// The port's writing end, shared: the MIDI thread (CoreMIDI's, or the ALSA input thread on Linux) sends MIDI played in
/// the guest straight away (no detour through the poll loop), so every frame is written whole under this lock
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

/// Credits for the Mac's camera from the camera's own thread (macOS: per frame the reader took)
fn camera_credit(n: u32) {
    let payload = serde_json::to_vec(&serde_json::json!({ "credits": n })).expect("serializable");
    write_port(&encode(MSG_CAMERA_CREDIT, 0, &payload));
}

/// A guest app played to one of the virtual destinations / writable ports (the MIDI thread)
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

/// What this agent offers in its hello; the Mac enables a subset in its welcome. macOS guests keep their clipboard
/// helper until the agent's pasteboard is checked in a guest (PNG: OPEN, spec §11.9), so the agent there offers the
/// camera (plan B169), MIDI and the diagnosis. The diagnosis is offered always, on every guest kind (plan B162)
fn offered(midi: bool) -> Vec<&'static str> {
    let mut f = vec!["status", "diagnose"];
    if cfg!(target_os = "linux") {
        f.push("clipboard");
    }
    if camera::available() {
        f.push("camera");
    }
    if midi {
        f.push("midi");
    }
    f
}

/// The diagnosis check that runs now (one at a time, spec §11.9)
enum Running {
    /// On a worker thread, which answers the Mac itself; the counter goes up when it is done
    Worker(Arc<AtomicUsize>),
    /// Handed to the newest session process; answered `fail` by the agent after SESSION_WAIT
    Session { req: diagnose::Request, deadline: Instant },
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
    /// The Mac's last camera status ("status", "detail"), for the diagnosis
    camera_status: Option<(String, String)>,
    /// macOS: OBS's scene collection for the camera (obs.rs), as last looked at
    obs: Option<obs::State>,
    /// Why there is no MIDI here (Midi::new failed), for the diagnosis
    midi_error: Option<String>,
    /// MIDI endpoints mirrored from the Mac's list and the names that failed, as last reported to the Mac
    midi_state: (usize, Vec<String>),
    /// Diagnosis requests waiting for the one that runs
    diag_queue: VecDeque<diagnose::Request>,
    diag_running: Option<Running>,
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
                match Camera::open(camera_credit) {
                    Ok(c) => self.camera = Some(c),
                    Err(e) => log(&format!("{}: {e} – no camera", camera::DEVICE)),
                }
            }
            self.obs_prepare();
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

    /// macOS: OBS's scene for the camera (obs.rs) – at the welcome and with every status, until it is there
    fn obs_prepare(&mut self) {
        if !cfg!(target_os = "macos") || !self.enabled("camera") || matches!(self.obs, Some(obs::State::Ready { .. })) {
            return;
        }
        let state = obs::prepare();
        if self.obs.as_ref() != Some(&state) {
            log(&state.describe());
        }
        self.obs = Some(state);
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
        self.midi_state = (count, failed);
    }

    fn send_answer(&mut self, answer: &diagnose::Answer) {
        self.send_raw(&encode_message(MSG_DIAGNOSE_ANSWER, &diagnose::encode(answer)));
    }

    /// A diagnosis request from the Mac: queued, and started when nothing else runs
    fn diagnose_request(&mut self, payload: &[u8]) {
        match diagnose::parse_request(payload) {
            Ok(req) if self.diag_queue.len() >= diagnose::MAX_QUEUE => {
                let busy = diagnose::Outcome::new(diagnose::Status::Fail, "Too many diagnosis requests at once");
                self.send_answer(&busy.answer(&req));
            }
            Ok(req) => {
                self.diag_queue.push_back(req);
                self.diagnose_tick();
            }
            Err(Some(id)) => self.send_answer(&diagnose::malformed(id)),
            Err(None) => log("malformed diagnosis request"),
        }
    }

    /// The running check: done or overdue? Then the next one from the queue
    fn diagnose_tick(&mut self) {
        match &self.diag_running {
            Some(Running::Worker(done)) if done.load(Ordering::SeqCst) > 0 => self.diag_running = None,
            Some(Running::Session { req, deadline }) if Instant::now() >= *deadline => {
                let answer = diagnose::session_silent(req);
                self.diag_running = None;
                self.send_answer(&answer);
            }
            _ => {}
        }
        while self.diag_running.is_none()
            && let Some(req) = self.diag_queue.pop_front()
        {
            self.diagnose_start(req);
        }
    }

    fn diagnose_start(&mut self, req: diagnose::Request) {
        if !diagnose::known(&req.check) {
            return self.send_answer(&diagnose::unknown(&req));
        }
        match diagnose::place(&req.check) {
            diagnose::Place::Agent => {
                let answer = self.diagnose_here(&req);
                self.send_answer(&answer);
            }
            diagnose::Place::Worker => self.diagnose_worker(req),
            diagnose::Place::Session { fallback } => {
                let bytes = encode_message(MSG_DIAGNOSE_REQUEST, &serde_json::to_vec(&req).expect("serializable"));
                // The newest session: the user who logged in last
                if let Some(s) = self.sessions.last_mut()
                    && s.stream.write_all(&bytes).is_ok()
                {
                    self.diag_running =
                        Some(Running::Session { req, deadline: Instant::now() + diagnose::SESSION_WAIT });
                } else if fallback {
                    self.diagnose_worker(req);
                } else {
                    self.send_answer(&diagnose::no_session(&req));
                }
            }
        }
    }

    fn diagnose_worker(&mut self, req: diagnose::Request) {
        let done = Arc::new(AtomicUsize::new(0));
        self.diag_running = Some(Running::Worker(done.clone()));
        let ctx = diagnose::Context { sessions: self.sessions.len() };
        std::thread::spawn(move || {
            #[cfg(target_os = "macos")]
            let clipboard: Option<&dyn diagnose::Clipboard> = Some(&session::PlainClipboard);
            #[cfg(not(target_os = "macos"))]
            let clipboard: Option<&dyn diagnose::Clipboard> = None;
            let answer = diagnose::run(&req, ctx, clipboard);
            write_port(&encode_message(MSG_DIAGNOSE_ANSWER, &diagnose::encode(&answer)));
            done.fetch_add(1, Ordering::SeqCst);
        });
    }

    /// The checks answered from the agent's own state
    fn diagnose_here(&self, req: &diagnose::Request) -> diagnose::Answer {
        use diagnose::{Outcome, Status};
        let outcome = match req.check.as_str() {
            "camera" => self.diagnose_camera(),
            "midi" if !self.enabled("midi") => Outcome::new(Status::Skipped, "MIDI is off for this VM"),
            "midi" if self.midi.is_none() => Outcome::new(
                Status::Fail,
                format!(
                    "MIDI is on, but the guest's MIDI system is unavailable: {}",
                    self.midi_error.as_deref().unwrap_or("?")
                ),
            ),
            "midi" => {
                let (count, failed) = &self.midi_state;
                let data = serde_json::json!({ "endpoints": count, "failed": failed });
                if failed.is_empty() {
                    Outcome::new(Status::Pass, format!("{count} MIDI endpoints from the Mac in the guest")).with(data)
                } else {
                    let detail = format!("{} MIDI endpoints could not be made: {}", failed.len(), failed.join(", "));
                    Outcome::new(Status::Fail, detail).with(data)
                }
            }
            _ => return diagnose::unknown(req),
        };
        outcome.answer(req)
    }

    /// macOS (plan B169): the stream, OBS and its camera extension, whether OBS reads, the Mac's last camera status
    #[cfg(target_os = "macos")]
    fn diagnose_camera(&self) -> diagnose::Outcome {
        use diagnose::{Outcome, Status};
        if !self.enabled("camera") {
            return Outcome::new(Status::Skipped, "The camera is off for this VM");
        }
        let state = obs::prepare();
        let extension = obs::extension_state();
        let mac = self.camera_status.as_ref();
        let reading = self.camera.as_ref().is_some_and(|c| c.active);
        let data = serde_json::json!({
            "device": camera::DEVICE,
            "open": self.camera.is_some(),
            "inUse": reading,
            "obs": state.describe(),
            "obsExtension": extension,
            "macStatus": mac.map(|(s, _)| s),
            "macDetail": mac.map(|(_, d)| d),
        });
        let outcome = if self.camera.is_none() {
            Outcome::new(Status::Fail, format!("The agent could not open {}", camera::DEVICE))
        } else if state == obs::State::NotInstalled {
            Outcome::new(
                Status::Fail,
                "Install OBS (obsproject.com) in the VM: until omaflake's own camera extension, apps in the VM see \
                 the Mac's camera as \"OBS Virtual Camera\"",
            )
        } else if state == obs::State::NeverStarted {
            Outcome::new(Status::Fail, "Start OBS once in the VM; the agent then sets up its scene collection")
        } else if extension.as_deref() != Some("activated enabled") {
            Outcome::new(
                Status::Fail,
                match extension {
                    None => "In OBS, click \"Start Virtual Camera\" once to install OBS's camera extension".into(),
                    Some(s) => format!(
                        "OBS's camera extension is {s}: approve \"OBS Virtual Camera\" in System Settings → General → \
                         Login Items & Extensions → Camera Extensions, then start OBS's virtual camera again"
                    ),
                },
            )
        } else if let Some((s, d)) = mac.filter(|(s, _)| s == "unavailable") {
            Outcome::new(
                Status::Fail,
                format!("The Mac's camera is {s}{}", if d.is_empty() { String::new() } else { format!(": {d}") }),
            )
        } else if reading {
            Outcome::new(
                Status::Pass,
                format!(
                    "OBS reads the Mac's camera{}; apps in the VM choose \"OBS Virtual Camera\"",
                    mac.map(|(s, _)| format!(" ({s})")).unwrap_or_default()
                ),
            )
        } else {
            Outcome::new(
                Status::Pass,
                format!(
                    "Ready – {}. In OBS: the scene collection \"{}\", then \"Start Virtual Camera\"",
                    state.describe(),
                    obs::COLLECTION
                ),
            )
        };
        outcome.with(data)
    }

    /// NixOS: the loopback device, whether the agent feeds it, and the Mac's last camera status
    #[cfg(not(target_os = "macos"))]
    fn diagnose_camera(&self) -> diagnose::Outcome {
        use diagnose::{Outcome, Status};
        if !std::path::Path::new(camera::DEVICE).exists() {
            return Outcome::new(
                Status::Fail,
                format!("The camera device {} is missing (v4l2loopback)", camera::DEVICE),
            );
        }
        if !self.enabled("camera") {
            return Outcome::new(Status::Skipped, "The camera is off for this VM");
        }
        let mac = self.camera_status.as_ref();
        let data = serde_json::json!({
            "device": camera::DEVICE,
            "open": self.camera.is_some(),
            "inUse": self.camera.as_ref().is_some_and(|c| c.active),
            "macStatus": mac.map(|(s, _)| s),
            "macDetail": mac.map(|(_, d)| d),
        });
        let outcome = match (self.camera.is_some(), mac) {
            (false, _) => Outcome::new(Status::Fail, format!("The agent could not open {}", camera::DEVICE)),
            (true, Some((s, d))) if s == "unavailable" => Outcome::new(
                Status::Fail,
                format!(
                    "The Mac's camera is unavailable{}",
                    if d.is_empty() { String::new() } else { format!(": {d}") }
                ),
            ),
            (true, Some((s, _))) => {
                Outcome::new(Status::Pass, format!("{} is ready; the Mac's camera: {s}", camera::DEVICE))
            }
            (true, None) => Outcome::new(
                Status::Pass,
                format!(
                    "{} is ready; no camera status from the Mac yet (it comes when an app opens the camera)",
                    camera::DEVICE
                ),
            ),
        };
        outcome.with(data)
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
                // Linux: one buffer free again; macOS: a replaced frame's credit (the rest come from its thread)
                let credits = cam.write_frame(frame.payload).unwrap_or_else(|e| {
                    log(&format!("camera frame: {e}"));
                    1
                });
                if credits > 0 {
                    self.send(MSG_CAMERA_CREDIT, &serde_json::json!({ "credits": credits }));
                }
            }
            MSG_CAMERA_STATUS if self.enabled("camera") => {
                log(&format!("Mac camera: {}", String::from_utf8_lossy(&frame.payload)));
                if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&frame.payload) {
                    let text = |k: &str| midi::clean(v.get(k).and_then(|x| x.as_str()).unwrap_or(""));
                    self.camera_status = Some((text("status"), text("detail")));
                }
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
            MSG_DIAGNOSE_REQUEST if self.enabled("diagnose") => {
                let Some(payload) = self.parts.push(frame, MAX_DIAGNOSE_REQUEST) else { return };
                self.diagnose_request(&payload);
            }
            // Unknown types, or a feature the Mac did not enable: skipped
            other => log(&format!("ignored message type {other:#06x}")),
        }
    }

    /// A message from a session process (its user's clipboard) – passed on only if the Mac enabled the clipboard
    fn handle_session(&mut self, i: usize, frame: Frame) {
        if frame.kind == MSG_DIAGNOSE_ANSWER {
            return self.diagnose_session_answer(&frame.payload);
        }
        if frame.kind != MSG_CLIPBOARD {
            return;
        }
        let Some(payload) = self.sessions[i].parts.push(frame, clip::MAX_MESSAGE) else { return };
        if self.enabled("clipboard") && clip::decode(&payload).is_some() {
            let bytes = encode_message(MSG_CLIPBOARD, &payload);
            self.send_raw(&bytes);
        }
    }

    /// A session's answer to the check handed to it: passed on to the Mac (re-encoded, only for the running check)
    fn diagnose_session_answer(&mut self, payload: &[u8]) {
        let Some(Running::Session { req, .. }) = &self.diag_running else { return };
        match serde_json::from_slice::<diagnose::Answer>(payload) {
            Ok(mut a) if a.id == req.id => {
                a.check = req.check.clone();
                self.diag_running = None;
                self.send_answer(&a);
                self.diagnose_tick();
            }
            _ => log("a session sent a diagnosis answer nobody asked for"),
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
        let mut timeout = due.saturating_duration_since(now).min(Duration::from_secs(1));
        if !self.diag_queue.is_empty() {
            timeout = timeout.min(Duration::from_millis(50)); // the next check starts as soon as the running one is done
        }
        // port, listener, camera, then the sessions
        let mut fds = vec![libc::pollfd { fd: self.port.as_raw_fd(), events: libc::POLLIN, revents: 0 }];
        let listener_at = self.listener.as_ref().map(|l| {
            fds.push(libc::pollfd { fd: l.as_raw_fd(), events: libc::POLLIN, revents: 0 });
            fds.len() - 1
        });
        let camera_at = fds.len();
        if let Some(c) = self.camera.as_ref() {
            for (fd, events) in c.poll_fds() {
                fds.push(libc::pollfd { fd, events, revents: 0 });
            }
        }
        let sessions_at = fds.len();
        for s in &self.sessions {
            fds.push(libc::pollfd { fd: s.stream.as_raw_fd(), events: libc::POLLIN, revents: 0 });
        }
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout.as_millis() as libc::c_int) };
        if rc < 0 {
            return; // EINTR (a signal) – the caller checks STOP
        }

        if let Some(cam) = self.camera.as_mut() {
            let revents: Vec<i16> = fds[camera_at..sessions_at].iter().map(|f| f.revents).collect();
            match cam.events(&revents) {
                Ok(Some(now_active)) => {
                    log(if now_active { "an application opened the camera" } else { "camera no longer in use" });
                    if now_active {
                        self.camera_start();
                    } else {
                        self.send(MSG_CAMERA_CONTROL, &serde_json::json!({ "action": "stop" }));
                    }
                }
                Ok(None) => {}
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
            self.midi_state = (0, vec![]);
            // Checks for a Mac side that is gone are dropped; a worker that still runs finishes on its own
            self.diag_queue.clear();
            if matches!(self.diag_running, Some(Running::Session { .. })) {
                self.diag_running = None;
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
            self.obs_prepare();
        }
        if let Some(cam) = self.camera.as_mut() {
            cam.keep_fresh(now);
        }
        self.diagnose_tick();
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
    // Without MIDI the agent still runs: status and the diagnosis go on (macOS guests too)
    let (midi, midi_error) = match Midi::new(midi_out) {
        Ok(m) => (Some(m), None),
        Err(e) => {
            log(&format!("no MIDI: {e}"));
            (None, Some(e))
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
        camera_status: None,
        obs: None,
        midi_error,
        midi_state: (0, vec![]),
        diag_queue: VecDeque::new(),
        diag_running: None,
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
