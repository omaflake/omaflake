// SPDX-License-Identifier: AGPL-3.0-or-later
//! The host's camera for the guest (spec §11.9, types `0x0200`–`0x0203`), as `Camera.swift` does it on a Mac: the camera
//! runs only between the guest's "start" and "stop" (an application in the guest opened `/dev/video42`), frames go
//! only against the guest's credits – the newest frame that found none waits for the next credit, older ones are
//! dropped (live video is never queued) – and no credit for 2 s stops the camera. Each frame is one 1280×720 NV12
//! picture (`convert`). Where the frames come from is a `FrameSource`: a V4L2 camera (`v4l2`, Linux) or the test
//! pattern (`--camera-test-pattern`, as the Mac's).

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::agent_protocol::{AgentMessage, AgentProtocol, AgentSender, CameraRequest};
use crate::host_bridge::convert::{FRAME_BYTES, HEIGHT, WIDTH};
use crate::Log;

/// Without a credit for this long while frames are waiting, the guest stopped reading: the camera goes off
pub const CREDIT_TIMEOUT: Duration = Duration::from_secs(2);

/// Somewhere 1280×720 NV12 frames come from, opened on "start" and closed on "stop"
pub trait FrameSource: Send {
    /// Starts capturing; Ok(what is shown as the camera status' detail, e.g. the device's name)
    fn open(&mut self) -> Result<String, String>;
    /// The next frame, None when none came within `timeout`; Err when the camera failed (it is closed then)
    fn frame(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, String>;
    fn close(&mut self);
}

/// What to do with a captured frame
#[derive(Debug, PartialEq, Eq)]
pub enum Gate {
    /// A credit was there: send it
    Send(Vec<u8>),
    /// No credit (yet): kept as the pending frame
    Held,
    /// No credit for `CREDIT_TIMEOUT`: stop the camera
    Stop,
}

/// The credit bookkeeping of `CameraBridge.sendAgent` (Swift), without the I/O
#[derive(Debug, Default)]
pub struct CreditGate {
    credits: u32,
    last_credit: Option<Instant>,
    pending: Option<Vec<u8>>,
}

impl CreditGate {
    /// The camera starts: credits from an earlier run do not count, the guest's first credit gets `CREDIT_TIMEOUT`
    /// (the Mac counts from its last credit; here the start counts as one, so a frame captured before the guest's
    /// credit message arrives does not stop the camera)
    pub fn start(&mut self, now: Instant) {
        *self = CreditGate { last_credit: Some(now), ..Default::default() };
    }

    pub fn reset(&mut self) {
        *self = CreditGate::default();
    }

    /// The guest grants `n` more frames (at most `MAX_CAMERA_CREDITS` in all); a pending frame goes out at once
    pub fn grant(&mut self, n: u32, now: Instant) -> Option<Vec<u8>> {
        self.credits = (self.credits + n).min(AgentProtocol::MAX_CAMERA_CREDITS);
        self.last_credit = Some(now);
        let frame = self.pending.take()?;
        self.credits -= 1;
        Some(frame)
    }

    pub fn frame(&mut self, frame: Vec<u8>, now: Instant) -> Gate {
        if self.credits > 0 {
            self.credits -= 1;
            return Gate::Send(frame);
        }
        if self.last_credit.is_none_or(|t| now.duration_since(t) > CREDIT_TIMEOUT) {
            self.pending = None;
            return Gate::Stop;
        }
        self.pending = Some(frame);
        Gate::Held
    }

    pub fn credits(&self) -> u32 {
        self.credits
    }
}

enum Command {
    UseAgent(AgentSender),
    Request(CameraRequest),
}

/// The camera side of the bridge: a thread owning the `FrameSource`; requests from the agent link reach it through
/// a channel, so the link's reader never waits for the camera
pub struct CameraBridge {
    commands: Sender<Command>,
}

impl CameraBridge {
    pub fn start(source: Box<dyn FrameSource>, label: &str, log: Arc<Log<'static>>) -> CameraBridge {
        let (tx, rx) = mpsc::channel();
        let label = label.to_string();
        std::thread::spawn(move || {
            Worker { source, label, log, sender: None, gate: CreditGate::default(), running: false }.run(rx)
        });
        CameraBridge { commands: tx }
    }

    /// The guest's agent carries the camera (its welcome enabled it): status "ready", frames from now on go there
    pub fn use_agent(&self, sender: AgentSender) {
        let _ = self.commands.send(Command::UseAgent(sender));
    }

    /// Start, stop and credits from the agent
    pub fn request(&self, r: CameraRequest) {
        let _ = self.commands.send(Command::Request(r));
    }
}

struct Worker {
    source: Box<dyn FrameSource>,
    /// "camera" or "test pattern", for the "ready" status
    label: String,
    log: Arc<Log<'static>>,
    sender: Option<AgentSender>,
    gate: CreditGate,
    running: bool,
}

impl Worker {
    fn run(mut self, rx: Receiver<Command>) {
        loop {
            // Requests first (a credit or "stop" must not wait behind frames), then one frame while the camera runs
            let next = if self.running {
                match rx.try_recv() {
                    Ok(c) => Some(c),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => break,
                }
            } else {
                match rx.recv_timeout(Duration::from_secs(3600)) {
                    Ok(c) => Some(c),
                    Err(RecvTimeoutError::Timeout) => continue,
                    Err(RecvTimeoutError::Disconnected) => break,
                }
            };
            if let Some(command) = next {
                self.handle(command);
                continue;
            }
            match self.source.frame(Duration::from_millis(200)) {
                Ok(Some(frame)) if frame.len() == FRAME_BYTES => self.offer(frame),
                Ok(_) => {}
                Err(e) => {
                    (self.log)(&format!("camera: {e}"));
                    self.stop();
                    self.status("unavailable", &e);
                }
            }
        }
        self.stop();
    }

    fn handle(&mut self, command: Command) {
        match command {
            Command::UseAgent(sender) => {
                self.stop();
                self.gate.reset();
                self.sender = Some(sender);
                let label = self.label.clone();
                self.status("ready", &label);
            }
            Command::Request(CameraRequest::Start) => {
                if self.running || self.sender.is_none() {
                    return;
                }
                match self.source.open() {
                    Ok(detail) => {
                        (self.log)(&format!("camera: on ({detail})"));
                        self.running = true;
                        self.gate.start(Instant::now());
                        self.status("streaming", &detail);
                    }
                    Err(e) => {
                        (self.log)(&format!("camera: {e}"));
                        self.status("unavailable", &e);
                    }
                }
            }
            Command::Request(CameraRequest::Stop) => {
                self.stop();
                self.gate.reset();
            }
            Command::Request(CameraRequest::Credits(n)) => {
                if let Some(frame) = self.gate.grant(n, Instant::now()) {
                    self.send_frame(&frame);
                }
            }
        }
    }

    fn offer(&mut self, frame: Vec<u8>) {
        match self.gate.frame(frame, Instant::now()) {
            Gate::Send(f) => self.send_frame(&f),
            Gate::Held => {}
            Gate::Stop => {
                self.stop();
                self.status("stopped", "no credit from the guest for 2 s");
            }
        }
    }

    fn send_frame(&mut self, frame: &[u8]) {
        let sent = self.sender.as_ref().is_some_and(|s| s.send(AgentMessage::CameraFrame, frame));
        if !sent {
            self.stop();
        }
    }

    fn stop(&mut self) {
        if self.running {
            self.source.close();
            self.running = false;
            (self.log)("camera: off");
        }
    }

    fn status(&self, status: &str, detail: &str) {
        if let Some(sender) = &self.sender {
            let json = serde_json::json!({ "status": status, "detail": detail });
            sender.send(AgentMessage::CameraStatus, json.to_string().as_bytes());
        }
    }
}

/// Moving gray bars instead of a camera, at 15 fps or OMAFLAKE_CAMERA_TEST_FPS (1–60) – the Mac's test pattern
pub struct TestPattern {
    fps: f64,
    tick: usize,
    next: Instant,
}

impl TestPattern {
    pub fn new() -> TestPattern {
        let fps = std::env::var("OMAFLAKE_CAMERA_TEST_FPS").ok().and_then(|v| v.parse::<f64>().ok()).unwrap_or(15.0);
        TestPattern { fps: fps.clamp(1.0, 60.0), tick: 0, next: Instant::now() }
    }

    /// One frame of the pattern: a row of bars shifted by `tick`, copied to every row; chroma neutral
    pub fn render(tick: usize) -> Vec<u8> {
        let row: Vec<u8> = (0..WIDTH).map(|x| (16 + ((x + tick * 8) / 40 % 2) * 200) as u8).collect();
        let mut frame = Vec::with_capacity(FRAME_BYTES);
        for _ in 0..HEIGHT {
            frame.extend_from_slice(&row);
        }
        frame.resize(FRAME_BYTES, 128);
        frame
    }
}

impl Default for TestPattern {
    fn default() -> Self {
        Self::new()
    }
}

impl FrameSource for TestPattern {
    fn open(&mut self) -> Result<String, String> {
        self.next = Instant::now();
        Ok("test pattern".into())
    }

    fn frame(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, String> {
        let now = Instant::now();
        if self.next > now {
            let wait = self.next - now;
            if wait > timeout {
                std::thread::sleep(timeout);
                return Ok(None);
            }
            std::thread::sleep(wait);
        }
        self.next = self.next.max(Instant::now() - Duration::from_secs(1)) + Duration::from_secs_f64(1.0 / self.fps);
        self.tick += 1;
        Ok(Some(Self::render(self.tick)))
    }

    fn close(&mut self) {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn credits_bound_what_goes_out() {
        let t0 = Instant::now();
        let mut g = CreditGate::default();
        g.start(t0);
        assert_eq!(g.grant(2, t0), None);
        assert_eq!(g.frame(vec![1], t0), Gate::Send(vec![1]));
        assert_eq!(g.frame(vec![2], t0), Gate::Send(vec![2]));
        // no credit left: the newest frame waits, an older one is replaced
        assert_eq!(g.frame(vec![3], t0), Gate::Held);
        assert_eq!(g.frame(vec![4], t0 + Duration::from_millis(100)), Gate::Held);
        assert_eq!(g.grant(1, t0 + Duration::from_millis(150)), Some(vec![4]));
        assert_eq!(g.credits(), 0);
        // at most four credits however many the guest grants
        g.grant(100, t0);
        assert_eq!(g.credits(), AgentProtocol::MAX_CAMERA_CREDITS);
    }

    #[test]
    fn no_credit_for_two_seconds_stops() {
        let t0 = Instant::now();
        let mut g = CreditGate::default();
        g.start(t0);
        assert_eq!(g.frame(vec![1], t0 + Duration::from_secs(1)), Gate::Held);
        assert_eq!(g.frame(vec![2], t0 + Duration::from_millis(2100)), Gate::Stop);
        assert_eq!(g.grant(1, t0 + Duration::from_secs(3)), None, "the pending frame went with the stop");
        // never started (no credit ever): stop
        assert_eq!(CreditGate::default().frame(vec![1], t0), Gate::Stop);
    }

    #[test]
    fn test_pattern_frames() {
        let f = TestPattern::render(1);
        assert_eq!(f.len(), FRAME_BYTES);
        assert_eq!((f[0], f[40 - 8]), (16, 216), "bars 40 px wide, shifted by 8 px a tick");
        assert_eq!(f[WIDTH * HEIGHT], 128);
        assert_ne!(TestPattern::render(1), TestPattern::render(3), "the bars move");
    }
}
