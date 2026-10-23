// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of AgentProtocol.swift – work package F2. Done: constants and types. Skeleton: the functions (`todo!`).
//!
//! The omaflake agent protocol (spec §11.9): frames over the virtio port org.omaflake.agent. The guest side is
//! nix/agent (Rust); both check the vectors in nix/agent/testdata/vectors.json (port the Swift tests in
//! Tests/OmaflakeCoreTests/AgentProtocolTests.swift, which read them).
//!
//! Frame: 12-byte header, little endian – magic "OMFA", type u16, flags u16 (bit 0: payload continues in the next
//! frame of the same type), payload length u32 (at most `MAX_PAYLOAD`) – then the payload. The protocol version is
//! agreed once per connection: the guest's hello names the range it speaks, the Mac's welcome the version chosen.
//!
//! Not 1:1: Swift's `AgentLink` took a raw descriptor and was both reader and (thread-safe) writer. Here it owns a
//! `UnixStream`; `run(&mut self)` reads, and `sender()` hands out an `AgentSender` (a clone of the stream behind a
//! shared lock) for writing from other threads – camera frames, clipboard content.
#![allow(unused_variables, dead_code)]

use std::collections::{HashMap, HashSet};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::Log;

/// Swift `enum AgentProtocol` (constants and frame encoding)
pub enum AgentProtocol {}

impl AgentProtocol {
    pub const PORT_NAME: &'static str = "org.omaflake.agent";
    pub const MAGIC: [u8; 4] = *b"OMFA";
    pub const HEADER_LENGTH: usize = 12;
    pub const MAX_PAYLOAD: usize = 2 * 1024 * 1024;
    pub const FLAG_MORE: u16 = 1;
    /// Large messages go in chunks of at most this size (a control message waits for one chunk at most)
    pub const CHUNK: usize = 256 * 1024;
    /// Clipboard: at most 32 MiB of content, as JSON with base64 a third more
    pub const MAX_CLIPBOARD: usize = 32 * 1024 * 1024;
    pub const MAX_CLIPBOARD_MESSAGE: usize = Self::MAX_CLIPBOARD / 3 * 4 + 1024;
    /// Camera: 1280×720 NV12, and the frames the guest may grant at once (spec §11.9, credits)
    pub const CAMERA_FRAME_BYTES: usize = 1280 * 720 * 3 / 2;
    pub const MAX_CAMERA_CREDITS: u32 = 4;

    /// Versions the Mac side speaks: the current one and, once there is one, the previous one (spec §11.9)
    pub const SUPPORTED: std::ops::RangeInclusive<u32> = 1..=1;
    pub const CURRENT: u32 = 1;

    /// A message as frames: one frame, or chunks of `CHUNK` bytes with MORE on all but the last
    pub fn encode_message(type_: u16, payload: &[u8]) -> Vec<u8> {
        todo!("port from AgentProtocol.swift")
    }

    /// The version both sides speak (the highest), None if their ranges do not overlap
    pub fn agree(agent_min: u32, agent_max: u32) -> Option<u32> {
        todo!("port from AgentProtocol.swift")
    }

    /// One frame. Panics when `payload` is longer than `MAX_PAYLOAD` (Swift: precondition)
    pub fn encode(type_: u16, flags: u16, payload: &[u8]) -> Vec<u8> {
        todo!("port from AgentProtocol.swift")
    }
}

/// Swift `AgentProtocol.Message` (the frame types)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[repr(u16)]
pub enum AgentMessage {
    /// guest → Mac
    Hello = 0x0001,
    /// Mac → guest
    Welcome = 0x0002,
    /// guest → Mac
    Status = 0x0003,
    /// guest → Mac
    Goodbye = 0x0004,
    /// both: {"mime", "data" (base64)} in chunks
    Clipboard = 0x0100,
    /// guest → Mac: {"action": "start" | "stop"}
    CameraControl = 0x0200,
    /// guest → Mac: {"credits": n}
    CameraCredit = 0x0201,
    /// Mac → guest: one NV12 frame
    CameraFrame = 0x0202,
    /// Mac → guest: {"status", "detail"}
    CameraStatus = 0x0203,
}

impl AgentMessage {
    /// Swift `Message(rawValue:)`
    pub fn from_u16(v: u16) -> Option<Self> {
        todo!("port from AgentProtocol.swift")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AgentFrame {
    pub type_: u16,
    pub flags: u16,
    pub payload: Vec<u8>,
}

/// Puts chunked messages back together; one that grows beyond its limit is dropped up to its last frame, so its
/// tail never passes for a message of its own (the same as the Rust side's Reassembler).
#[derive(Debug, Clone, Default)]
pub struct AgentReassembler {
    /// Per type: the message so far, or None while the rest of a dropped one is skipped
    parts: HashMap<u16, Option<Vec<u8>>>,
    pub dropped: usize,
}

impl AgentReassembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// The whole message once its last frame came, None before (and for a dropped one)
    pub fn push(&mut self, frame: &AgentFrame, limit: usize) -> Option<Vec<u8>> {
        todo!("port from AgentProtocol.swift")
    }
}

/// Incremental frame decoder. A bad header costs one error and the bytes up to the next magic (resynchronisation).
#[derive(Debug, Clone, Default)]
pub struct AgentDecoder {
    buffer: Vec<u8>,
    pub errors: usize,
}

impl AgentDecoder {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn feed(&mut self, bytes: &[u8]) {
        todo!("port from AgentProtocol.swift")
    }

    pub fn next_frame(&mut self) -> Option<AgentFrame> {
        todo!("port from AgentProtocol.swift")
    }

    /// Drops at least one byte, then everything before the next possible magic
    fn skip_to_magic(&mut self) {
        todo!("port from AgentProtocol.swift")
    }
}

// MARK: Messages

/// The protocol range in a hello
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProtocolRange {
    pub min: u32,
    pub max: u32,
}

/// The agent's hello. Everything in it comes from the guest – untrusted, cleaned by `sanitized()`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentHello {
    pub agent: String,
    pub protocol: ProtocolRange,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub os: Option<String>,
    #[serde(rename = "osName", default, skip_serializing_if = "Option::is_none")]
    pub os_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kernel: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hostname: Option<String>,
    #[serde(rename = "bootId", default, skip_serializing_if = "Option::is_none")]
    pub boot_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<String>>,
}

impl AgentHello {
    /// Every text through `Untrusted::text`, at most 32 features
    pub(crate) fn sanitized(&self) -> AgentHello {
        todo!("port from AgentProtocol.swift")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentWelcome {
    pub protocol: u32,
    pub features: Vec<String>,
    #[serde(rename = "statusInterval")]
    pub status_interval: u32,
    pub host: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentMemory {
    pub total: u64,
    pub available: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentDisk {
    pub path: String,
    pub total: u64,
    pub free: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentAddress {
    pub interface: String,
    pub family: String,
    pub address: String,
}

/// The agent's periodic status (guest view; untrusted, cleaned by `sanitized()`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AgentStatus {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub uptime: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub load: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub memory: Option<AgentMemory>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disk: Option<AgentDisk>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub addresses: Option<Vec<AgentAddress>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub users: Option<Vec<String>>,
}

impl AgentStatus {
    /// Numbers through `Untrusted::number` (at most 3 load values), texts through `Untrusted::text`, at most 64
    /// addresses and 64 users
    pub(crate) fn sanitized(&self) -> AgentStatus {
        todo!("port from AgentProtocol.swift")
    }
}

/// Cleaning for text and numbers from the guest before omaflake keeps or shows them
pub(crate) enum Untrusted {}

impl Untrusted {
    /// Control characters (Unicode general category Cc and the other Foundation `controlCharacters`) removed, at
    /// most 256 Unicode scalars
    pub(crate) fn text(s: &str) -> String {
        todo!("port from AgentProtocol.swift")
    }

    /// Finite, ≥ 0 and < 1e15, else None
    pub(crate) fn number(d: f64) -> Option<f64> {
        todo!("port from AgentProtocol.swift")
    }
}

// MARK: State

/// What omaflake knows from a VM's agent: kept by the Mac side of the link in the run folder (`agent.json`),
/// read by the CLI and the app. Removed when the VM starts, so a file left from an earlier run never counts.
/// JSON: camelCase keys (helloAt, protocolVersion, statusAt, goodbyeAt), `.iso8601` dates, pretty and sorted with
/// escaped slashes (Swift `JSONEncoder.agent`).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AgentState {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hello: Option<AgentHello>,
    #[serde(
        rename = "helloAt",
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::json::iso8601::option"
    )]
    pub hello_at: Option<DateTime<Utc>>,
    /// The version agreed in the welcome; None while there is none (no hello yet, or no common version)
    #[serde(rename = "protocolVersion", default, skip_serializing_if = "Option::is_none")]
    pub protocol_version: Option<u32>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<AgentStatus>,
    #[serde(
        rename = "statusAt",
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::json::iso8601::option"
    )]
    pub status_at: Option<DateTime<Utc>>,
    #[serde(
        rename = "goodbyeAt",
        default,
        skip_serializing_if = "Option::is_none",
        with = "crate::json::iso8601::option"
    )]
    pub goodbye_at: Option<DateTime<Utc>>,
    /// Why the link does not work (no common version, too many malformed messages)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub problem: Option<String>,
    /// What the welcome enabled: offered by the agent and switched on for this VM
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub features: Option<Vec<String>>,
}

impl AgentState {
    /// The agent said hello and both sides speak a common version: the VM is up
    pub fn is_ready(&self) -> bool {
        self.hello.is_some() && self.protocol_version.is_some() && self.goodbye_at.is_none()
    }

    /// The agent speaks only older protocol versions than this omaflake: the image should be rebuilt (spec §11.9)
    pub fn agent_is_older(&self) -> bool {
        self.hello.as_ref().map(|h| h.protocol.max).unwrap_or(AgentProtocol::CURRENT) < AgentProtocol::CURRENT
    }

    /// None when missing or unreadable
    pub fn load(path: &Path) -> Option<AgentState> {
        todo!("port from AgentProtocol.swift")
    }

    /// Written atomically, pretty and sorted (`json::to_pretty(self, true)`)
    pub fn save(&self, path: &Path) -> Result<()> {
        todo!("port from AgentProtocol.swift")
    }
}

// MARK: Link

/// How `AgentLink::run` ended
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkEnd {
    /// The VM closed the port
    Closed,
    /// The guest misbehaved (error budget spent)
    Abandoned,
}

/// What the guest asks of the Mac's camera
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CameraRequest {
    Start,
    Stop,
    Credits(u32),
}

/// `AgentLink::on_welcome`: the features the welcome enabled
pub type WelcomeHandler = Box<dyn Fn(&HashSet<String>) + Send + Sync>;
/// `AgentLink::on_clipboard`: (mime, data)
pub type ClipboardHandler = Box<dyn Fn(&str, &[u8]) + Send + Sync>;
/// `AgentLink::on_camera`
pub type CameraHandler = Box<dyn Fn(CameraRequest) + Send + Sync>;

/// Writes to the guest from any thread (Swift `AgentLink.send`, thread-safe)
#[derive(Debug, Clone)]
pub struct AgentSender {
    stream: Arc<Mutex<UnixStream>>,
}

impl AgentSender {
    /// Sends a message to the guest; false when the port is gone. Large messages go in chunks – except camera frames:
    /// one frame each (at most 2 MiB, so a 1280×720 NV12 frame fits), the guest takes them whole
    pub fn send(&self, message: AgentMessage, payload: &[u8]) -> bool {
        todo!("port from AgentProtocol.swift")
    }
}

/// The Mac side of one agent connection: reads frames from the stream (QEMU: the port's Unix socket), answers the
/// hello with a welcome and keeps what the agent reports in `state_file`. Accepts only the messages above, with
/// limits: the guest is untrusted (spec §11.9). Nothing the guest sends makes the Mac do more than update that file.
pub struct AgentLink {
    stream: UnixStream,
    sender: AgentSender,
    state_file: PathBuf,
    host: String,
    status_interval: u32,
    log: Arc<Log>,
    state: AgentState,
    decoder: AgentDecoder,
    parts: AgentReassembler,
    errors: usize,
    last_accepted: HashMap<u16, Instant>,
    clipboard_times: Vec<Instant>,
    /// Features the Mac side allows for this VM (its switches); the welcome enables those the agent offers too
    allowed: HashSet<String>,
    /// What the welcome enabled – messages of other features are dropped
    enabled: HashSet<String>,
    /// Called on the link's thread: the welcome went out with these features (also again after a new hello)
    pub on_welcome: Option<WelcomeHandler>,
    /// Clipboard content from the guest (mime, data – text or PNG, checked), when the clipboard is enabled
    pub on_clipboard: Option<ClipboardHandler>,
    /// The guest's camera requests, when the camera is enabled
    pub on_camera: Option<CameraHandler>,
}

impl AgentLink {
    /// Bad frames, malformed JSON or unknown types tolerated before the link stops listening
    pub const ERROR_BUDGET: usize = 64;
    /// Messages faster than this are dropped (a guest flooding the Mac side)
    pub const MIN_INTERVAL: Duration = Duration::from_millis(500);

    /// Swift defaults: host "omaflake", status interval 5, allowed ["status"] (always added), log ignored
    pub fn new(
        stream: UnixStream,
        state_file: PathBuf,
        host: &str,
        status_interval: u32,
        allowed: HashSet<String>,
        log: Arc<Log>,
    ) -> Result<Self> {
        todo!("port from AgentProtocol.swift")
    }

    /// A handle for writing to the guest from other threads
    pub fn sender(&self) -> AgentSender {
        self.sender.clone()
    }

    /// What the welcome enabled
    pub fn enabled(&self) -> &HashSet<String> {
        &self.enabled
    }

    pub fn state(&self) -> &AgentState {
        &self.state
    }

    /// Reads until the VM closes the port (`Closed`) or the guest misbehaves (`Abandoned`); blocking.
    pub fn run(&mut self) -> LinkEnd {
        todo!("port from AgentProtocol.swift")
    }

    /// Handles bytes from the guest; false once the error budget is spent (then `problem` is set and persisted:
    /// "too many malformed messages from the guest – omaflake stopped listening")
    pub(crate) fn receive(&mut self, bytes: &[u8]) -> bool {
        todo!("port from AgentProtocol.swift")
    }

    pub(crate) fn handle(&mut self, frame: &AgentFrame) {
        todo!("port from AgentProtocol.swift")
    }

    pub(crate) fn welcome(&mut self, version: u32) {
        todo!("port from AgentProtocol.swift")
    }

    /// (mime, data) of a clipboard message if it is text ("text/plain;charset=utf-8") or PNG ("image/png") within the
    /// size limit
    pub(crate) fn clipboard(payload: &[u8]) -> Option<(String, Vec<u8>)> {
        todo!("port from AgentProtocol.swift")
    }

    pub(crate) fn persist(&self) {
        todo!("port from AgentProtocol.swift")
    }
}
