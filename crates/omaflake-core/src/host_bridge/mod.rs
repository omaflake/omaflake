// SPDX-License-Identifier: AGPL-3.0-or-later
//! The host side of a NixOS guest's omaflake agent on a Linux host (plan B125) – what `omaf-runner bridge`
//! (Bridge.swift, Camera.swift) is on a Mac: `omaf bridge`, one process per VM, started detached by
//! `QemuRuntime::start` and ending with the VM (QEMU closes the agent port). It answers the agent's hello
//! (`AgentLink`: the instance's `agent.json`, so readiness and `omaf agent` work as on a Mac) and, when the VM's
//! switches allow them, carries the host's clipboard (`clipboard`: Wayland or X11) and camera (`camera`, frames from
//! `v4l2` or the test pattern) over the agent. Only images whose agent carries clipboard and camera (plan B106 on) –
//! the old per-feature ports QEMU still attaches stay unserved on Linux.

pub mod camera;
pub mod clipboard;
pub mod convert;
#[cfg(target_os = "linux")]
pub mod v4l2;

use std::collections::HashSet;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::agent_protocol::{AgentLink, LinkEnd};
use crate::sockets::connect_unix;
use crate::Log;
use camera::{CameraBridge, FrameSource};
use clipboard::{ClipboardBackend, ClipboardBridge};

/// What `omaf bridge` is asked to do (its options)
#[derive(Debug, Clone)]
pub struct BridgeOptions {
    /// The agent port's Unix socket (QEMU's `agent.sock`)
    pub agent: PathBuf,
    /// Where the agent's report goes (`agent.json`)
    pub agent_state: PathBuf,
    /// The VM's clipboard switch
    pub clipboard: bool,
    /// The VM's camera switch
    pub camera: bool,
    /// Frames of a test pattern instead of the camera
    pub camera_test_pattern: bool,
}

/// The Mac side's host name in the welcome, as Bridge.swift sends it
pub const HOST: &str = "omaflake";
/// Seconds between the agent's status reports
pub const STATUS_INTERVAL: u32 = 5;

/// Runs the bridge until the VM closes the agent port (or the guest misbehaves). Connects to the socket QEMU created
/// (the VM may still be starting: up to 60 s), picks the clipboard and camera of this host and logs to stderr (the
/// instance's `bridge.log`).
pub fn run(options: &BridgeOptions) -> Result<LinkEnd, String> {
    let log: Arc<Log<'static>> = Arc::new(|line: &str| eprintln!("{} {line}", chrono::Local::now().format("%H:%M:%S")));
    let stream = connect(&options.agent, Duration::from_secs(60))?;
    let clipboard = if options.clipboard {
        match clipboard::detect() {
            Ok(backend) => {
                log(&format!("clipboard: {}", backend.describe()));
                Some(backend)
            }
            Err(why) => {
                log(&format!("clipboard: unavailable – {why}"));
                None
            }
        }
    } else {
        None
    };
    let camera: Option<(Box<dyn FrameSource>, &str)> = if !options.camera {
        None
    } else if options.camera_test_pattern {
        Some((Box::new(camera::TestPattern::new()), "test pattern"))
    } else {
        #[cfg(target_os = "linux")]
        {
            Some((Box::new(v4l2::V4l2Camera::new()), "camera"))
        }
        #[cfg(not(target_os = "linux"))]
        {
            log("camera: V4L2 exists on Linux only");
            None
        }
    };
    Bridge::new(stream, &options.agent_state, clipboard, camera, log).map(|b| b.run())
}

/// Connects to QEMU's socket, retrying every 0.5 s until `wait` ran out; reads then wait forever (a connect timeout
/// left on the socket made the Mac's bridge take 2 s of silence for "VM gone", plan B36)
pub fn connect(path: &Path, wait: Duration) -> Result<UnixStream, String> {
    let deadline = Instant::now() + wait;
    loop {
        match connect_unix(path, Duration::from_secs(2)) {
            Ok(s) => {
                s.set_read_timeout(None).map_err(|e| e.to_string())?;
                return Ok(s);
            }
            Err(e) if Instant::now() >= deadline => return Err(e.message().to_string()),
            Err(_) => std::thread::sleep(Duration::from_millis(500)),
        }
    }
}

/// One agent connection with the features it serves
pub struct Bridge {
    link: AgentLink,
}

impl Bridge {
    /// `clipboard`: the host's clipboard (None: off for this VM, or none in this session); `camera`: a frame source
    /// and its label for the "ready" status (None: off)
    pub fn new(
        stream: UnixStream,
        state_file: &Path,
        clipboard: Option<Arc<dyn ClipboardBackend>>,
        camera: Option<(Box<dyn FrameSource>, &str)>,
        log: Arc<Log<'static>>,
    ) -> Result<Bridge, String> {
        let mut allowed: HashSet<String> = HashSet::from(["status".to_string()]);
        if clipboard.is_some() {
            allowed.insert("clipboard".into());
        }
        if camera.is_some() {
            allowed.insert("camera".into());
        }
        let mut link = AgentLink::new(stream, state_file.to_path_buf(), HOST, STATUS_INTERVAL, allowed, log.clone())
            .map_err(|e| e.message().to_string())?;
        let clipboard = clipboard.map(|backend| {
            let bridge = ClipboardBridge::new(backend, log.clone());
            bridge.start();
            bridge
        });
        let camera = camera.map(|(source, label)| Arc::new(CameraBridge::start(source, label, log.clone())));
        let sender = link.sender();
        let (clip, cam) = (clipboard.clone(), camera.clone());
        // Each welcome (also after a new hello – the guest restarted its agent) hands the features it enabled the link
        link.on_welcome = Some(Box::new(move |features| {
            if let Some(c) = clip.as_ref().filter(|_| features.contains("clipboard")) {
                c.use_agent(sender.clone());
            }
            if let Some(c) = cam.as_ref().filter(|_| features.contains("camera")) {
                c.use_agent(sender.clone());
            }
        }));
        if let Some(c) = clipboard {
            link.on_clipboard = Some(Box::new(move |mime, data| c.apply(mime, data)));
        }
        if let Some(c) = camera {
            link.on_camera = Some(Box::new(move |request| c.request(request)));
        }
        Ok(Bridge { link })
    }

    /// Reads the agent until the VM closes the port (`Closed`) or the guest spent its error budget (`Abandoned`)
    pub fn run(mut self) -> LinkEnd {
        self.link.run()
    }
}

#[cfg(test)]
mod tests {
    use super::camera::TestPattern;
    use super::clipboard::tests::FakeClipboard;
    use super::*;
    use crate::agent_protocol::{AgentDecoder, AgentFrame, AgentMessage, AgentProtocol, AgentReassembler, AgentState};
    use base64::Engine;
    use std::io::{Read, Write};

    /// The guest's end of the agent port
    struct Guest {
        stream: UnixStream,
        decoder: AgentDecoder,
        parts: AgentReassembler,
    }

    impl Guest {
        fn send(&mut self, message: AgentMessage, json: serde_json::Value) {
            let bytes = AgentProtocol::encode_message(message as u16, json.to_string().as_bytes());
            self.stream.write_all(&bytes).unwrap();
        }

        /// The next whole message of `type_` from the host, skipping others (5 s at most)
        fn expect(&mut self, message: AgentMessage) -> Vec<u8> {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut chunk = vec![0u8; 1 << 20];
            loop {
                while let Some(frame) = self.decoder.next_frame() {
                    let f: AgentFrame = frame;
                    if f.type_ == message as u16 {
                        if let Some(whole) = self.parts.push(&f, 64 << 20) {
                            return whole;
                        }
                    }
                }
                assert!(Instant::now() < deadline, "no {message:?} from the host");
                match self.stream.read(&mut chunk) {
                    Ok(0) => panic!("the host closed the port"),
                    Ok(n) => self.decoder.feed(&chunk[..n]),
                    Err(_) => {}
                }
            }
        }

        fn json(&mut self, message: AgentMessage) -> serde_json::Value {
            serde_json::from_slice(&self.expect(message)).unwrap()
        }
    }

    fn hello(features: &[&str]) -> serde_json::Value {
        serde_json::json!({
            "agent": "0.2.0", "protocol": {"min": 1, "max": 1}, "os": "linux", "osName": "NixOS",
            "hostname": "guest", "features": features,
        })
    }

    fn start(
        clipboard: Option<Arc<dyn ClipboardBackend>>,
        camera: Option<Box<dyn FrameSource>>,
    ) -> (Guest, std::thread::JoinHandle<LinkEnd>, PathBuf, tempfile::TempDir) {
        let (guest, host) = UnixStream::pair().unwrap();
        guest.set_read_timeout(Some(Duration::from_millis(200))).unwrap();
        let dir = tempfile::tempdir().unwrap();
        let state = dir.path().join("agent.json");
        let bridge =
            Bridge::new(host, &state, clipboard, camera.map(|c| (c, "test pattern")), Arc::new(|_: &str| {})).unwrap();
        let t = std::thread::spawn(move || bridge.run());
        (Guest { stream: guest, decoder: AgentDecoder::new(), parts: AgentReassembler::new() }, t, state, dir)
    }

    #[test]
    fn hello_welcome_status_and_report() {
        let (mut guest, t, state, _dir) = start(None, None);
        guest.send(AgentMessage::Hello, hello(&["status", "clipboard", "camera"]));
        let welcome = guest.json(AgentMessage::Welcome);
        // clipboard and camera are off for this VM: only status is enabled
        assert_eq!(welcome["features"], serde_json::json!(["status"]));
        assert_eq!((welcome["protocol"].as_u64(), welcome["host"].as_str()), (Some(1), Some(HOST)));
        assert_eq!(welcome["statusInterval"].as_u64(), Some(STATUS_INTERVAL as u64));
        guest.send(AgentMessage::Status, serde_json::json!({"uptime": 12.5, "users": ["u"]}));
        let deadline = Instant::now() + Duration::from_secs(5);
        let report = loop {
            if let Some(s) = AgentState::load(&state).filter(|s| s.status.is_some()) {
                break s;
            }
            assert!(Instant::now() < deadline, "no status in agent.json");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert!(report.is_ready());
        assert_eq!(report.hello.unwrap().hostname.as_deref(), Some("guest"));
        assert_eq!(report.status.unwrap().uptime, Some(12.5));
        drop(guest);
        assert_eq!(t.join().unwrap(), LinkEnd::Closed, "the VM closing the port ends the bridge");
    }

    #[test]
    fn clipboard_both_ways() {
        let clip = FakeClipboard::new();
        clip.copy(clipboard::TEXT, b"there before"); // never sent: only what is copied from now on
        let (mut guest, _t, _state, _dir) = start(Some(clip.clone()), None);
        guest.send(AgentMessage::Hello, hello(&["status", "clipboard"]));
        assert_eq!(guest.json(AgentMessage::Welcome)["features"], serde_json::json!(["clipboard", "status"]));
        std::thread::sleep(Duration::from_millis(100)); // the welcome reached the bridge's handlers

        // host → guest, also large (chunked): 600 KB of PNG bytes
        let png: Vec<u8> = (0..600_000u32).map(|i| (i % 253) as u8).collect();
        clip.copy(clipboard::PNG, &png);
        let m = guest.json(AgentMessage::Clipboard);
        assert_eq!(m["mime"], clipboard::PNG);
        assert_eq!(base64::engine::general_purpose::STANDARD.decode(m["data"].as_str().unwrap()).unwrap(), png);

        // guest → host: written, and its echo (the clipboard reports the change) not sent back
        let text = "Grüße aus dem Gast";
        let b64 = base64::engine::general_purpose::STANDARD.encode(text);
        guest.send(AgentMessage::Clipboard, serde_json::json!({"mime": clipboard::TEXT, "data": b64}));
        let deadline = Instant::now() + Duration::from_secs(5);
        while clip.written.lock().unwrap().is_empty() {
            assert!(Instant::now() < deadline, "nothing written");
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(clip.written.lock().unwrap()[0], (clipboard::TEXT.to_string(), text.as_bytes().to_vec()));
        clip.copy(clipboard::TEXT, b"next");
        let m = guest.json(AgentMessage::Clipboard);
        assert_eq!(m["data"], base64::engine::general_purpose::STANDARD.encode("next"), "the echo was skipped");
    }

    #[test]
    fn camera_frames_against_credits() {
        let (mut guest, _t, _state, _dir) = start(None, Some(Box::new(TestPattern::new())));
        guest.send(AgentMessage::Hello, hello(&["status", "camera"]));
        assert_eq!(guest.json(AgentMessage::Welcome)["features"], serde_json::json!(["camera", "status"]));
        assert_eq!(guest.json(AgentMessage::CameraStatus)["status"], "ready");
        guest.send(AgentMessage::CameraControl, serde_json::json!({"action": "start"}));
        guest.send(AgentMessage::CameraCredit, serde_json::json!({"credits": 2}));
        let s = guest.json(AgentMessage::CameraStatus);
        assert_eq!((s["status"].as_str(), s["detail"].as_str()), (Some("streaming"), Some("test pattern")));
        for _ in 0..2 {
            assert_eq!(guest.expect(AgentMessage::CameraFrame).len(), AgentProtocol::CAMERA_FRAME_BYTES);
        }
        // No credit for 2 s: the host stops its camera
        let s = guest.json(AgentMessage::CameraStatus);
        assert_eq!(s["status"], "stopped");
        // Asked again: frames again
        guest.send(AgentMessage::CameraControl, serde_json::json!({"action": "start"}));
        guest.send(AgentMessage::CameraCredit, serde_json::json!({"credits": 1}));
        assert_eq!(guest.json(AgentMessage::CameraStatus)["status"], "streaming");
        assert_eq!(guest.expect(AgentMessage::CameraFrame).len(), AgentProtocol::CAMERA_FRAME_BYTES);
        guest.send(AgentMessage::CameraControl, serde_json::json!({"action": "stop"}));
    }

    #[test]
    fn camera_off_for_the_vm_drops_requests() {
        let (mut guest, t, _state, _dir) = start(None, None);
        guest.send(AgentMessage::Hello, hello(&["status", "camera"]));
        guest.json(AgentMessage::Welcome);
        guest.send(AgentMessage::CameraControl, serde_json::json!({"action": "start"}));
        guest.send(AgentMessage::CameraCredit, serde_json::json!({"credits": 2}));
        std::thread::sleep(Duration::from_millis(300));
        let mut buf = [0u8; 64];
        assert!(guest.stream.read(&mut buf).is_err(), "nothing comes back (read times out)");
        drop(guest);
        assert_eq!(t.join().unwrap(), LinkEnd::Closed);
    }

    #[test]
    fn connect_waits_for_the_socket() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("agent.sock");
        assert!(connect(&path, Duration::from_millis(100)).is_err());
        let p = path.clone();
        let server = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            let l = std::os::unix::net::UnixListener::bind(&p).unwrap();
            l.accept().unwrap()
        });
        let s = connect(&path, Duration::from_secs(5)).unwrap();
        assert_eq!(s.read_timeout().unwrap(), None);
        server.join().unwrap();
    }
}
