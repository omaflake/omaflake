// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of AgentProtocol.swift.
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
//! shared lock) for writing from other threads – camera frames, clipboard content. Times between messages are
//! measured with `Instant` (monotonic) instead of `Date`.

use std::collections::{HashMap, HashSet};
use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::Log;
use crate::error::Result;
use crate::file_format::write_atomic;
use crate::json;

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
    /// MIDI: one data message at most (never chunked), a ping's payload at most (spec §11.9, plan B161)
    pub const MAX_MIDI_MESSAGE: usize = 64 * 1024;
    pub const MAX_MIDI_PING: usize = 64;
    /// MIDI from the guest: bytes a second it may send on average, and the burst above that (a SysEx dump is fine, a
    /// guest flooding the host's devices is cut off)
    pub const MIDI_RATE: usize = 1024 * 1024;
    pub const MIDI_BURST: usize = 512 * 1024;

    /// Versions the Mac side speaks: the current one and, once there is one, the previous one (spec §11.9)
    pub const SUPPORTED: std::ops::RangeInclusive<u32> = 1..=1;
    pub const CURRENT: u32 = 1;

    /// A message as frames: one frame, or chunks of `CHUNK` bytes with MORE on all but the last
    pub fn encode_message(type_: u16, payload: &[u8]) -> Vec<u8> {
        if payload.len() <= Self::CHUNK {
            return Self::encode(type_, 0, payload);
        }
        let mut out = Vec::with_capacity(payload.len() + (payload.len() / Self::CHUNK + 1) * Self::HEADER_LENGTH);
        let mut chunks = payload.chunks(Self::CHUNK).peekable();
        while let Some(chunk) = chunks.next() {
            let flags = if chunks.peek().is_some() { Self::FLAG_MORE } else { 0 };
            out.extend(Self::encode(type_, flags, chunk));
        }
        out
    }

    /// The version both sides speak (the highest), None if their ranges do not overlap
    pub fn agree(agent_min: u32, agent_max: u32) -> Option<u32> {
        let v = agent_max.min(*Self::SUPPORTED.end());
        (v >= agent_min.max(*Self::SUPPORTED.start())).then_some(v)
    }

    /// One frame. Panics when `payload` is longer than `MAX_PAYLOAD` (Swift: precondition)
    pub fn encode(type_: u16, flags: u16, payload: &[u8]) -> Vec<u8> {
        assert!(payload.len() <= Self::MAX_PAYLOAD, "payload too large");
        let mut out = Vec::with_capacity(Self::HEADER_LENGTH + payload.len());
        out.extend_from_slice(&Self::MAGIC);
        out.extend_from_slice(&type_.to_le_bytes());
        out.extend_from_slice(&flags.to_le_bytes());
        out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
        out.extend_from_slice(payload);
        out
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
    /// Mac → guest: {"endpoints": [...]}, the whole list after each change
    MidiEndpoints = 0x0500,
    /// both: MIDI records (`MidiRecord`)
    MidiData = 0x0501,
    /// guest → Mac: {"endpoints": n, "failed": [names]}
    MidiStatus = 0x0502,
    /// Mac → guest: at most `MAX_MIDI_PING` bytes, answered at once
    MidiPing = 0x0503,
    /// guest → Mac: the ping's payload unchanged
    MidiPong = 0x0504,
}

impl AgentMessage {
    /// Swift `Message(rawValue:)`
    pub fn from_u16(v: u16) -> Option<Self> {
        use AgentMessage::*;
        [
            Hello,
            Welcome,
            Status,
            Goodbye,
            Clipboard,
            CameraControl,
            CameraCredit,
            CameraFrame,
            CameraStatus,
            MidiEndpoints,
            MidiData,
            MidiStatus,
            MidiPing,
            MidiPong,
        ]
        .into_iter()
        .find(|m| *m as u16 == v)
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
        let last = frame.flags & AgentProtocol::FLAG_MORE == 0;
        match self.parts.get_mut(&frame.type_) {
            Some(None) => {} // skipping
            Some(Some(buf)) => {
                if buf.len() + frame.payload.len() > limit {
                    self.parts.insert(frame.type_, None);
                    self.dropped += 1;
                } else {
                    buf.extend_from_slice(&frame.payload);
                }
            }
            None => {
                if frame.payload.len() > limit {
                    self.parts.insert(frame.type_, None);
                    self.dropped += 1;
                } else {
                    self.parts.insert(frame.type_, Some(frame.payload.clone()));
                }
            }
        }
        if !last {
            return None;
        }
        self.parts.remove(&frame.type_).flatten()
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
        self.buffer.extend_from_slice(bytes);
    }

    pub fn next_frame(&mut self) -> Option<AgentFrame> {
        loop {
            if self.buffer.len() >= 4 && self.buffer[0..4] != AgentProtocol::MAGIC {
                self.errors += 1;
                self.skip_to_magic();
                continue;
            }
            if self.buffer.len() < AgentProtocol::HEADER_LENGTH {
                return None;
            }
            let b = &self.buffer;
            let length = u32::from_le_bytes([b[8], b[9], b[10], b[11]]) as usize;
            if length > AgentProtocol::MAX_PAYLOAD {
                self.errors += 1;
                self.skip_to_magic();
                continue;
            }
            if b.len() < AgentProtocol::HEADER_LENGTH + length {
                return None;
            }
            let type_ = u16::from_le_bytes([b[4], b[5]]);
            let flags = u16::from_le_bytes([b[6], b[7]]);
            let payload = b[AgentProtocol::HEADER_LENGTH..AgentProtocol::HEADER_LENGTH + length].to_vec();
            self.buffer.drain(..AgentProtocol::HEADER_LENGTH + length);
            return Some(AgentFrame { type_, flags, payload });
        }
    }

    /// Drops at least one byte, then everything before the next possible magic
    fn skip_to_magic(&mut self) {
        let b = &self.buffer;
        let start =
            (1..b.len()).find(|&i| AgentProtocol::MAGIC.starts_with(&b[i..b.len().min(i + 4)])).unwrap_or(b.len());
        self.buffer.drain(..start);
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
        let text = |o: &Option<String>| o.as_deref().map(Untrusted::text);
        AgentHello {
            agent: Untrusted::text(&self.agent),
            protocol: self.protocol.clone(),
            os: text(&self.os),
            os_name: text(&self.os_name),
            kernel: text(&self.kernel),
            hostname: text(&self.hostname),
            boot_id: text(&self.boot_id),
            features: self.features.as_ref().map(|f| f.iter().take(32).map(|s| Untrusted::text(s)).collect()),
        }
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
        AgentStatus {
            uptime: self.uptime.and_then(Untrusted::number),
            load: self.load.as_ref().map(|l| l.iter().take(3).filter_map(|d| Untrusted::number(*d)).collect()),
            memory: self.memory.clone(),
            disk: self.disk.as_ref().map(|d| AgentDisk {
                path: Untrusted::text(&d.path),
                total: d.total,
                free: d.free,
            }),
            addresses: self.addresses.as_ref().map(|a| {
                a.iter()
                    .take(64)
                    .map(|a| AgentAddress {
                        interface: Untrusted::text(&a.interface),
                        family: Untrusted::text(&a.family),
                        address: Untrusted::text(&a.address),
                    })
                    .collect()
            }),
            users: self.users.as_ref().map(|u| u.iter().take(64).map(|s| Untrusted::text(s)).collect()),
        }
    }
}

/// Cleaning for text and numbers from the guest before omaflake keeps or shows them
pub(crate) enum Untrusted {}

impl Untrusted {
    /// Control characters (Unicode general category Cc and the other Foundation `controlCharacters`) removed, at
    /// most 256 Unicode scalars
    pub(crate) fn text(s: &str) -> String {
        s.chars().filter(|c| !is_control_character(*c)).take(256).collect()
    }

    /// Finite, ≥ 0 and < 1e15, else None
    pub(crate) fn number(d: f64) -> Option<f64> {
        // NaN and the infinities are outside the range
        (0.0..1e15).contains(&d).then_some(d)
    }
}

/// Foundation's `CharacterSet.controlCharacters`: Unicode general categories Cc and Cf. The Cf ranges are those of
/// Unicode 15.1 (OPEN: Foundation follows the OS's Unicode version; a newer one may add format characters).
fn is_control_character(c: char) -> bool {
    const FORMAT: [(u32, u32); 21] = [
        (0x00AD, 0x00AD),
        (0x0600, 0x0605),
        (0x061C, 0x061C),
        (0x06DD, 0x06DD),
        (0x070F, 0x070F),
        (0x0890, 0x0891),
        (0x08E2, 0x08E2),
        (0x180E, 0x180E),
        (0x200B, 0x200F),
        (0x202A, 0x202E),
        (0x2060, 0x2064),
        (0x2066, 0x206F),
        (0xFEFF, 0xFEFF),
        (0xFFF9, 0xFFFB),
        (0x110BD, 0x110BD),
        (0x110CD, 0x110CD),
        (0x13430, 0x1343F),
        (0x1BCA0, 0x1BCA3),
        (0x1D173, 0x1D17A),
        (0xE0001, 0xE0001),
        (0xE0020, 0xE007F),
    ];
    let v = c as u32;
    c.is_control() || FORMAT.iter().any(|&(lo, hi)| (lo..=hi).contains(&v))
}

// MARK: MIDI

/// MIDI data on the agent's port (spec §11.9, plan B161): Universal MIDI Packets (MIDI 2.0's format, which carries MIDI
/// 1.0 too) in records naming their endpoint – `id u32, protocol u8 (1 | 2), flags u8, words u16`, then, when flags
/// bit 0 is set, `time u64` (the sender's monotonic nanoseconds when the data arrived from its MIDI system), then the
/// words (u32), all little endian, whole UMP messages only. The same as Swift's `MIDIRecord` and nix/agent/src/ump.rs;
/// all check the "midi" vectors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MidiRecord {
    /// An endpoint of the host's list: host → guest the source the data came from, guest → host the destination
    pub id: u32,
    /// 1 = MIDI 1.0 UMP, 2 = MIDI 2.0 UMP (CoreMIDI's MIDIProtocolID)
    pub protocol: u8,
    /// The sender's monotonic clock in nanoseconds when the data arrived from its MIDI system; None: "now"
    pub time: Option<u64>,
    pub words: Vec<u32>,
}

impl MidiRecord {
    pub const HEADER_LENGTH: usize = 8;
    /// Flags bit 0: 8 bytes of `time` follow the header; other bits are malformed
    pub const FLAG_TIME: u8 = 1;

    /// Words of the UMP message starting with `first`, by its message type (UMP spec 1.1)
    pub fn message_words(first: u32) -> usize {
        match first >> 28 {
            0x0 | 0x1 | 0x2 | 0x6 | 0x7 => 1,
            0x3 | 0x4 | 0x8 | 0x9 | 0xA => 2,
            0xB | 0xC => 3,
            _ => 4, // 0x5 (SysEx8, mixed data), 0xD (flex data), 0xE, 0xF (UMP stream)
        }
    }

    /// How many of `words` form whole messages (a message cut off at the end is left out)
    pub fn whole_messages(words: &[u32]) -> usize {
        let mut i = 0;
        while i < words.len() {
            let n = Self::message_words(words[i]);
            if i + n > words.len() {
                break;
            }
            i += n;
        }
        i
    }

    fn header_length(&self) -> usize {
        Self::HEADER_LENGTH + if self.time.is_some() { 8 } else { 0 }
    }

    /// Records as payloads of at most `MAX_MIDI_MESSAGE` bytes; long records split at message boundaries (each part
    /// keeps the record's time)
    pub fn encode(records: &[MidiRecord]) -> Vec<Vec<u8>> {
        let max = AgentProtocol::MAX_MIDI_MESSAGE;
        let mut out: Vec<Vec<u8>> = vec![];
        let mut cur: Vec<u8> = vec![];
        for r in records {
            let header = r.header_length();
            let max_words = ((max - header) / 4).min(u16::MAX as usize);
            let words = &r.words[..Self::whole_messages(&r.words)];
            let mut start = 0;
            while start < words.len() {
                let mut end = start;
                while end < words.len() && end + Self::message_words(words[end]) - start <= max_words {
                    end += Self::message_words(words[end]);
                }
                let part = &words[start..end];
                if !cur.is_empty() && cur.len() + header + part.len() * 4 > max {
                    out.push(std::mem::take(&mut cur));
                }
                cur.extend_from_slice(&r.id.to_le_bytes());
                cur.push(r.protocol);
                cur.push(if r.time.is_some() { Self::FLAG_TIME } else { 0 });
                cur.extend_from_slice(&(part.len() as u16).to_le_bytes());
                if let Some(t) = r.time {
                    cur.extend_from_slice(&t.to_le_bytes());
                }
                for w in part {
                    cur.extend_from_slice(&w.to_le_bytes());
                }
                start = end;
            }
        }
        if !cur.is_empty() {
            out.push(cur);
        }
        out
    }

    /// The records of a payload; None if it is malformed (too long, cut off, a protocol other than 1 or 2, unknown
    /// flags, or words that are not whole messages)
    pub fn decode(payload: &[u8]) -> Option<Vec<MidiRecord>> {
        if payload.len() > AgentProtocol::MAX_MIDI_MESSAGE {
            return None;
        }
        let mut records = vec![];
        let mut at = 0;
        while at < payload.len() {
            let head = payload.get(at..at + Self::HEADER_LENGTH)?;
            let id = u32::from_le_bytes([head[0], head[1], head[2], head[3]]);
            let (protocol, flags) = (head[4], head[5]);
            let count = u16::from_le_bytes([head[6], head[7]]) as usize;
            if !(protocol == 1 || protocol == 2) || flags & !Self::FLAG_TIME != 0 {
                return None;
            }
            at += Self::HEADER_LENGTH;
            let time = if flags & Self::FLAG_TIME != 0 {
                let t = payload.get(at..at + 8)?;
                at += 8;
                Some(u64::from_le_bytes(t.try_into().ok()?))
            } else {
                None
            };
            let body = payload.get(at..at + count * 4)?;
            let words: Vec<u32> = body.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
            if Self::whole_messages(&words) != words.len() {
                return None;
            }
            at += count * 4;
            records.push(MidiRecord { id, protocol, time, words });
        }
        Some(records)
    }
}

/// One endpoint of the host as the guest gets it (`MidiEndpoints`; Swift `MIDIEndpointInfo`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MidiEndpointInfo {
    pub id: u32,
    /// "in": a source on the host, its data come into the guest; "out": a destination, the guest plays to it
    pub direction: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
    pub protocol: u8,
    /// What a MIDI 2.0 device says of itself (spec §11.9, "UMP endpoints"), where the host's MIDI system has it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ump: Option<UmpEndpointInfo>,
}

/// A MIDI 2.0 device's UMP endpoint (CoreMIDI's `MIDIUMPEndpoint`, ALSA's UMP endpoint info)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UmpEndpointInfo {
    pub name: String,
    #[serde(rename = "productInstanceId")]
    pub product_instance_id: String,
    /// The 3 bytes of the SysEx manufacturer id as hex
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub family: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<u16>,
    /// The 4 bytes of the software revision as hex
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(rename = "staticBlocks")]
    pub static_blocks: bool,
    pub blocks: Vec<UmpBlockInfo>,
}

/// A function block of a UMP endpoint
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UmpBlockInfo {
    /// 0–31
    pub id: u8,
    pub name: String,
    /// "in" | "out" | "both"
    pub direction: String,
    /// 0–15
    #[serde(rename = "firstGroup")]
    pub first_group: u8,
    /// 1–16
    pub groups: u8,
    /// "no" | "yes" | "low-speed"
    pub midi1: String,
    /// "unknown" | "receiver" | "sender" | "both"
    #[serde(rename = "uiHint")]
    pub ui_hint: String,
}

impl UmpEndpointInfo {
    /// At most this many blocks, names at most this many bytes (spec §11.9)
    pub const MAX_BLOCKS: usize = 32;
    pub const MAX_NAME: usize = 98;

    /// Within the spec's limits: at most 32 blocks, names cut to 98 bytes (at a character boundary), ids, groups and
    /// the first group in range (blocks out of range are left out)
    pub fn limited(mut self) -> UmpEndpointInfo {
        self.name = Self::cut(&self.name);
        self.product_instance_id = Self::cut(&self.product_instance_id);
        self.blocks.retain(|b| b.id < 32 && b.first_group < 16 && (1..=16).contains(&b.groups));
        self.blocks.truncate(Self::MAX_BLOCKS);
        for b in &mut self.blocks {
            b.name = Self::cut(&b.name);
        }
        self
    }

    fn cut(s: &str) -> String {
        let clean: String = s.chars().filter(|c| !c.is_control()).collect();
        let mut end = clean.len().min(Self::MAX_NAME);
        while !clean.is_char_boundary(end) {
            end -= 1;
        }
        clean[..end].to_string()
    }
}

/// The sender's clock mapped onto the receiver's (spec §11.9, "Timestamps"): `offset` is the smallest
/// `own_now − time` seen over the last 30 s – the transport's fastest delivery – and data are handed to the MIDI system
/// for `time + offset + 2 ms`. A time already past, or more than 1 s ahead, is delivered now. Pure: the caller passes
/// both clocks (nanoseconds).
#[derive(Debug, Clone, Default)]
pub struct ClockMapper {
    /// (own_now, offset), offsets strictly increasing from the front: the front is the window's minimum, and a newer
    /// sample with the same or a smaller offset replaces older ones (so the newest minimum is kept when it slides)
    window: std::collections::VecDeque<(u64, i128)>,
}

impl ClockMapper {
    pub const WINDOW: u64 = 30_000_000_000;
    pub const LEAD: u64 = 2_000_000;
    pub const MAX_AHEAD: u64 = 1_000_000_000;

    pub fn new() -> Self {
        Self::default()
    }

    /// One more sample: data stamped `time` by the sender arrived at `own_now`
    pub fn observe(&mut self, own_now: u64, time: u64) {
        let offset = own_now as i128 - time as i128;
        while self.window.back().is_some_and(|&(_, o)| o >= offset) {
            self.window.pop_back();
        }
        self.window.push_back((own_now, offset));
        while self.window.front().is_some_and(|&(at, _)| at.saturating_add(Self::WINDOW) < own_now) {
            self.window.pop_front();
        }
    }

    /// The current offset (own clock − sender's), None before the first sample
    pub fn offset(&self) -> Option<i128> {
        self.window.front().map(|&(_, o)| o)
    }

    /// When data stamped `time` should reach the MIDI system on the own clock; None: now (no offset yet, already
    /// past, or more than 1 s ahead)
    pub fn deliver_at(&self, own_now: u64, time: u64) -> Option<u64> {
        let target = time as i128 + self.offset()? + Self::LEAD as i128;
        let now = own_now as i128;
        (target > now && target <= now + Self::MAX_AHEAD as i128).then_some(target as u64)
    }

    /// `observe`, then `deliver_at`: what a receiver does with each timestamped record
    pub fn schedule(&mut self, own_now: u64, time: u64) -> Option<u64> {
        self.observe(own_now, time);
        self.deliver_at(own_now, time)
    }
}

/// This process's monotonic clock in nanoseconds (CLOCK_MONOTONIC): MIDI time stamps and the round trip's pings
pub fn monotonic_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid timespec to write; CLOCK_MONOTONIC exists on macOS and Linux
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    (ts.tv_sec as u64).saturating_mul(1_000_000_000).saturating_add(ts.tv_nsec as u64)
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
    /// MIDI (plan B161; kept by omaf-runner and `omaf bridge`): the guest's endpoints and the measured round trip
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub midi: Option<AgentMidiState>,
}

/// What a guest reported about its MIDI endpoints, and the round trip the host side measured (spec §11.9)
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct AgentMidiState {
    pub endpoints: u32,
    #[serde(default)]
    pub failed: Vec<String>,
    /// Milliseconds Mac → guest agent → Mac of the last pings
    #[serde(rename = "roundTripMs", default, skip_serializing_if = "Option::is_none")]
    pub round_trip_ms: Option<RoundTrip>,
}

#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct RoundTrip {
    pub min: f64,
    pub median: f64,
    pub max: f64,
    pub samples: u32,
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
        let data = std::fs::read(path).ok()?;
        serde_json::from_slice(&data).ok()
    }

    /// Written atomically, pretty and sorted (`json::to_pretty(self, true)`)
    pub fn save(&self, path: &Path) -> Result<()> {
        write_atomic(path, &json::to_pretty(self, true)?)
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
/// `AgentLink::on_midi`: records the guest's apps played (checked: whole UMP messages, within the rate limit)
pub type MidiHandler = Box<dyn Fn(Vec<MidiRecord>) + Send + Sync>;

/// Writes to the guest from any thread (Swift `AgentLink.send`, thread-safe)
#[derive(Debug, Clone)]
pub struct AgentSender {
    stream: Arc<Mutex<UnixStream>>,
}

impl AgentSender {
    /// Sends a message to the guest; false when the port is gone. Large messages go in chunks – except camera frames:
    /// one frame each (at most 2 MiB, so a 1280×720 NV12 frame fits), the guest takes them whole
    pub fn send(&self, message: AgentMessage, payload: &[u8]) -> bool {
        let bytes = if message == AgentMessage::CameraFrame {
            AgentProtocol::encode(message as u16, 0, payload)
        } else {
            AgentProtocol::encode_message(message as u16, payload)
        };
        let stream = self.stream.lock().unwrap_or_else(|e| e.into_inner());
        (&*stream).write_all(&bytes).is_ok()
    }

    /// A ping for the MIDI round trip: its payload is the send time (`monotonic_ns`), the pong brings it back
    pub fn send_midi_ping(&self) -> bool {
        self.send(AgentMessage::MidiPing, &monotonic_ns().to_le_bytes())
    }

    /// Records to the guest (MIDI data messages of at most 64 KiB each); false when the port is gone
    pub fn send_midi(&self, records: &[MidiRecord]) -> bool {
        MidiRecord::encode(records).iter().all(|p| self.send(AgentMessage::MidiData, p))
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
    log: Arc<Log<'static>>,
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
    /// MIDI the guest's apps played, when MIDI is enabled; the receiver sends only to ids of its current list
    pub on_midi: Option<MidiHandler>,
    /// MIDI from the guest: the token bucket (bytes), when it was last filled, how much was dropped
    midi_tokens: f64,
    midi_filled: Instant,
    midi_dropped: usize,
    /// Round trips of the last pings, in milliseconds
    round_trips: Vec<f64>,
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
        log: Arc<Log<'static>>,
    ) -> Result<Self> {
        let writer = stream.try_clone()?;
        let mut allowed = allowed;
        allowed.insert("status".to_string());
        Ok(AgentLink {
            stream,
            sender: AgentSender { stream: Arc::new(Mutex::new(writer)) },
            state_file,
            host: host.to_string(),
            status_interval,
            log,
            state: AgentState::default(),
            decoder: AgentDecoder::new(),
            parts: AgentReassembler::new(),
            errors: 0,
            last_accepted: HashMap::new(),
            clipboard_times: Vec::new(),
            allowed,
            enabled: HashSet::new(),
            on_welcome: None,
            on_clipboard: None,
            on_camera: None,
            on_midi: None,
            midi_tokens: AgentProtocol::MIDI_BURST as f64,
            midi_filled: Instant::now(),
            midi_dropped: 0,
            round_trips: Vec::new(),
        })
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
        let mut chunk = vec![0u8; 65536];
        loop {
            match (&self.stream).read(&mut chunk) {
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Ok(0) | Err(_) => return LinkEnd::Closed,
                Ok(n) => {
                    if !self.receive(&chunk[..n]) {
                        return LinkEnd::Abandoned;
                    }
                }
            }
        }
    }

    /// Handles bytes from the guest; false once the error budget is spent (then `problem` is set and persisted:
    /// "too many malformed messages from the guest – omaflake stopped listening")
    pub(crate) fn receive(&mut self, bytes: &[u8]) -> bool {
        self.decoder.feed(bytes);
        while let Some(frame) = self.decoder.next_frame() {
            self.handle(&frame);
        }
        if self.errors + self.decoder.errors > Self::ERROR_BUDGET {
            let problem = "too many malformed messages from the guest – omaflake stopped listening";
            self.state.problem = Some(problem.to_string());
            self.persist();
            (self.log)(&format!("agent: {problem}"));
            return false;
        }
        true
    }

    pub(crate) fn handle(&mut self, frame: &AgentFrame) {
        let now = Instant::now();
        let Some(message) = AgentMessage::from_u16(frame.type_) else {
            self.errors += 1; // unknown to this version; a newer agent only sends what the welcome enabled
            return;
        };
        if matches!(
            message,
            AgentMessage::Hello | AgentMessage::Status | AgentMessage::Goodbye | AgentMessage::MidiStatus
        ) && let Some(last) = self.last_accepted.get(&frame.type_)
            && now.duration_since(*last) < Self::MIN_INTERVAL
        {
            return;
        }
        match message {
            AgentMessage::Hello => {
                let Ok(hello) = serde_json::from_slice::<AgentHello>(&frame.payload) else {
                    self.errors += 1;
                    return;
                };
                let hello = hello.sanitized();
                self.state =
                    AgentState { hello: Some(hello.clone()), hello_at: Some(json::now()), ..Default::default() };
                if let Some(v) = AgentProtocol::agree(hello.protocol.min, hello.protocol.max) {
                    self.state.protocol_version = Some(v);
                    self.welcome(v);
                    let on = hello.os_name.as_deref().or(hello.os.as_deref()).unwrap_or("?");
                    (self.log)(&format!("agent: hello from {} (protocol {v}) on {on}", hello.agent));
                } else {
                    let problem = format!(
                        "the agent speaks protocol {}–{}, this omaflake {}–{}",
                        hello.protocol.min,
                        hello.protocol.max,
                        AgentProtocol::SUPPORTED.start(),
                        AgentProtocol::SUPPORTED.end()
                    );
                    (self.log)(&format!("agent: {problem}"));
                    self.state.problem = Some(problem);
                }
            }
            AgentMessage::Status => {
                if self.state.protocol_version.is_none() {
                    return; // only after a welcome
                }
                let Ok(status) = serde_json::from_slice::<AgentStatus>(&frame.payload) else {
                    self.errors += 1;
                    return;
                };
                self.state.status = Some(status.sanitized());
                self.state.status_at = Some(json::now());
            }
            AgentMessage::Goodbye => {
                self.state.goodbye_at = Some(json::now());
                (self.log)("agent: goodbye");
            }
            AgentMessage::Clipboard => {
                if !self.enabled.contains("clipboard") {
                    return;
                }
                let Some(payload) = self.parts.push(frame, AgentProtocol::MAX_CLIPBOARD_MESSAGE) else { return };
                // At most 4 contents a second – a guest flooding the pasteboard is cut off
                self.clipboard_times.retain(|t| now.duration_since(*t) < Duration::from_secs(1));
                self.clipboard_times.push(now);
                if self.clipboard_times.len() > 4 {
                    return;
                }
                let Some((mime, data)) = Self::clipboard(&payload) else {
                    self.errors += 1;
                    return;
                };
                if let Some(on_clipboard) = &self.on_clipboard {
                    on_clipboard(&mime, &data);
                }
                return; // no state change to keep
            }
            AgentMessage::CameraControl => {
                if !self.enabled.contains("camera") {
                    return;
                }
                #[derive(Deserialize)]
                struct Control {
                    action: String,
                }
                let request = match serde_json::from_slice::<Control>(&frame.payload).ok().map(|c| c.action) {
                    Some(a) if a == "start" => CameraRequest::Start,
                    Some(a) if a == "stop" => CameraRequest::Stop,
                    _ => {
                        self.errors += 1;
                        return;
                    }
                };
                if let Some(on_camera) = &self.on_camera {
                    on_camera(request);
                }
                return;
            }
            AgentMessage::CameraCredit => {
                if !self.enabled.contains("camera") {
                    return;
                }
                #[derive(Deserialize)]
                struct Credit {
                    credits: i64,
                }
                let credits = match serde_json::from_slice::<Credit>(&frame.payload) {
                    Ok(c) if c.credits > 0 => c.credits,
                    _ => {
                        self.errors += 1;
                        return;
                    }
                };
                if let Some(on_camera) = &self.on_camera {
                    on_camera(CameraRequest::Credits(credits.min(AgentProtocol::MAX_CAMERA_CREDITS as i64) as u32));
                }
                return;
            }
            AgentMessage::MidiData => {
                if !self.enabled.contains("midi") {
                    return;
                }
                // Token bucket: `MIDI_RATE` bytes a second on average, `MIDI_BURST` at once – beyond that dropped
                let elapsed = now.duration_since(self.midi_filled).as_secs_f64();
                self.midi_tokens = (self.midi_tokens + elapsed * AgentProtocol::MIDI_RATE as f64)
                    .min(AgentProtocol::MIDI_BURST as f64);
                self.midi_filled = now;
                if frame.payload.len() as f64 > self.midi_tokens {
                    self.midi_dropped += 1;
                    if self.midi_dropped == 1 || self.midi_dropped.is_multiple_of(1000) {
                        let rate = AgentProtocol::MIDI_RATE / 1024;
                        (self.log)(&format!(
                            "agent: MIDI from the guest beyond {rate} KiB/s dropped ({}×)",
                            self.midi_dropped
                        ));
                    }
                    return;
                }
                self.midi_tokens -= frame.payload.len() as f64;
                let Some(records) = MidiRecord::decode(&frame.payload) else {
                    self.errors += 1;
                    return;
                };
                if let Some(on_midi) = &self.on_midi {
                    on_midi(records);
                }
                return;
            }
            AgentMessage::MidiStatus => {
                if !self.enabled.contains("midi") {
                    return;
                }
                #[derive(Deserialize)]
                struct Status {
                    endpoints: i64,
                    #[serde(default)]
                    failed: Option<Vec<String>>,
                }
                let s = match serde_json::from_slice::<Status>(&frame.payload) {
                    Ok(s) if s.endpoints >= 0 => s,
                    _ => {
                        self.errors += 1;
                        return;
                    }
                };
                let midi = self.state.midi.get_or_insert_with(AgentMidiState::default);
                midi.endpoints = s.endpoints.min(1024) as u32;
                midi.failed = s.failed.unwrap_or_default().iter().take(64).map(|f| Untrusted::text(f)).collect();
            }
            AgentMessage::MidiPong => {
                if !self.enabled.contains("midi") || frame.payload.len() != 8 {
                    return;
                }
                let sent = u64::from_le_bytes(frame.payload[..8].try_into().unwrap_or_default());
                let now_ns = monotonic_ns();
                if sent > now_ns || now_ns - sent >= 10_000_000_000 {
                    return; // a forged or stale time
                }
                self.round_trips.push((now_ns - sent) as f64 / 1e6);
                if self.round_trips.len() > 60 {
                    self.round_trips.remove(0);
                }
                let mut sorted = self.round_trips.clone();
                sorted.sort_by(f64::total_cmp);
                let midi = self.state.midi.get_or_insert_with(AgentMidiState::default);
                midi.round_trip_ms = Some(RoundTrip {
                    min: sorted[0],
                    median: sorted[sorted.len() / 2],
                    max: sorted[sorted.len() - 1],
                    samples: sorted.len() as u32,
                });
                return; // kept with the next status; no file write per pong
            }
            AgentMessage::Welcome
            | AgentMessage::CameraFrame
            | AgentMessage::CameraStatus
            | AgentMessage::MidiEndpoints
            | AgentMessage::MidiPing => {
                self.errors += 1; // the Mac's messages, never the guest's
                return;
            }
        }
        self.last_accepted.insert(frame.type_, now);
        self.persist();
    }

    pub(crate) fn welcome(&mut self, version: u32) {
        // What the agent offers and this VM allows
        let offered: HashSet<String> = self
            .state
            .hello
            .as_ref()
            .and_then(|h| h.features.clone())
            .unwrap_or_else(|| vec!["status".to_string()])
            .into_iter()
            .collect();
        self.enabled = self.allowed.intersection(&offered).cloned().collect();
        self.enabled.insert("status".to_string());
        let mut features: Vec<String> = self.enabled.iter().cloned().collect();
        features.sort();
        self.state.features = Some(features.clone());
        let w = AgentWelcome {
            protocol: version,
            features,
            status_interval: self.status_interval,
            host: self.host.clone(),
        };
        let Ok(json) = serde_json::to_vec(&w) else { return };
        self.sender.send(AgentMessage::Welcome, &json);
        if let Some(on_welcome) = &self.on_welcome {
            on_welcome(&self.enabled);
        }
    }

    /// (mime, data) of a clipboard message if it is text ("text/plain;charset=utf-8") or PNG ("image/png") within the
    /// size limit
    pub(crate) fn clipboard(payload: &[u8]) -> Option<(String, Vec<u8>)> {
        let obj: HashMap<String, String> = serde_json::from_slice(payload).ok()?;
        let mime = obj.get("mime")?;
        if !["text/plain;charset=utf-8", "image/png"].contains(&mime.as_str()) {
            return None;
        }
        let data = BASE64.decode(obj.get("data")?).ok()?;
        (data.len() <= AgentProtocol::MAX_CLIPBOARD).then(|| (mime.clone(), data))
    }

    pub(crate) fn persist(&self) {
        if let Err(e) = self.state.save(&self.state_file) {
            (self.log)(&format!("agent: cannot write {}: {e}", self.state_file.display()));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;

    fn bytes(hex: &str) -> Vec<u8> {
        (0..hex.len()).step_by(2).map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap()).collect()
    }

    /// The vectors shared with the guest agent (nix/agent/testdata/vectors.json)
    #[test]
    fn shared_vectors() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../nix/agent/testdata/vectors.json");
        let v: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        for f in v["frames"].as_array().unwrap() {
            let payload = f["payload"].as_str().unwrap().as_bytes();
            let type_ = f["type"].as_u64().unwrap() as u16;
            let flags = f["flags"].as_u64().unwrap() as u16;
            let wire = bytes(f["hex"].as_str().unwrap());
            assert_eq!(AgentProtocol::encode(type_, flags, payload), wire, "{}", f["name"]);
            let mut d = AgentDecoder::new();
            d.feed(&wire);
            assert_eq!(d.next_frame(), Some(AgentFrame { type_, flags, payload: payload.to_vec() }));
            assert_eq!(d.next_frame(), None);
        }
        for s in v["streams"].as_array().unwrap() {
            let mut d = AgentDecoder::new();
            d.feed(&bytes(s["hex"].as_str().unwrap()));
            let mut types = vec![];
            while let Some(f) = d.next_frame() {
                types.push(f.type_ as u64);
            }
            let want: Vec<u64> = s["types"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap()).collect();
            assert_eq!(types, want, "{}", s["name"]);
            assert_eq!(d.errors as u64, s["errors"].as_u64().unwrap(), "{}", s["name"]);
        }
    }

    #[test]
    fn version_agreement() {
        assert_eq!(AgentProtocol::agree(1, 1), Some(1));
        assert_eq!(AgentProtocol::agree(1, 9), Some(AgentProtocol::CURRENT));
        assert_eq!(AgentProtocol::agree(99, 100), None);
        assert_eq!(AgentProtocol::agree(0, 0), None);
        assert_eq!(AgentProtocol::CURRENT, *AgentProtocol::SUPPORTED.end());
    }

    #[test]
    fn message_types_round_trip() {
        for v in [
            0x0001, 0x0002, 0x0003, 0x0004, 0x0100, 0x0200, 0x0201, 0x0202, 0x0203, 0x0500, 0x0501, 0x0502, 0x0503,
            0x0504,
        ] {
            assert_eq!(AgentMessage::from_u16(v).map(|m| m as u16), Some(v));
        }
        assert_eq!(AgentMessage::from_u16(0x0005), None);
    }

    #[test]
    fn untrusted_values_are_cleaned() {
        assert_eq!(Untrusted::text("a\u{1b}b\u{7f}c\u{200b}d\u{feff}é"), "abcdé");
        assert_eq!(Untrusted::text(&"x".repeat(300)).chars().count(), 256);
        assert_eq!(Untrusted::number(1.5), Some(1.5));
        assert_eq!(Untrusted::number(-1.0), None);
        assert_eq!(Untrusted::number(f64::NAN), None);
        assert_eq!(Untrusted::number(1e15), None);
    }

    /// A socket pair: the test plays the guest on one end, AgentLink reads the other
    struct Harness {
        guest: Option<UnixStream>,
        link: AgentLink,
        state_file: PathBuf,
        _dir: tempfile::TempDir,
    }

    impl Harness {
        fn new(allowed: &[&str]) -> Harness {
            let (guest, mac) = UnixStream::pair().unwrap();
            let dir = tempfile::tempdir().unwrap();
            let state_file = dir.path().join("agent.json");
            let allowed = allowed.iter().map(|s| s.to_string()).collect();
            let link = AgentLink::new(mac, state_file.clone(), "omaflake", 7, allowed, Arc::new(|_: &str| {})).unwrap();
            Harness { guest: Some(guest), link, state_file, _dir: dir }
        }

        fn send(&mut self, message: AgentMessage, payload: &[u8]) -> bool {
            let frames = AgentProtocol::encode_message(message as u16, payload);
            self.link.receive(&frames)
        }

        /// What the Mac wrote back to the guest
        fn reply(&self) -> Vec<AgentFrame> {
            let guest = self.guest.as_ref().unwrap();
            guest.set_nonblocking(true).unwrap();
            let mut buf = vec![0u8; 4096];
            let mut d = AgentDecoder::new();
            if let Ok(n) = (&*guest).read(&mut buf) {
                d.feed(&buf[..n]);
            }
            std::iter::from_fn(|| d.next_frame()).collect()
        }

        fn state(&self) -> Option<AgentState> {
            AgentState::load(&self.state_file)
        }
    }

    const HELLO: &str = r#"{"agent":"0.1.0","protocol":{"min":1,"max":1},"os":"linux","osName":"NixOS\u001b[31m","features":["status"]}"#;

    #[test]
    fn hello_welcome_status() {
        let mut h = Harness::new(&["status"]);
        assert!(h.send(AgentMessage::Status, br#"{"uptime":1}"#)); // before the hello: ignored
        assert_eq!(h.state(), None);
        assert!(h.send(AgentMessage::Hello, HELLO.as_bytes()));
        let replies = h.reply();
        assert_eq!(replies.len(), 1);
        assert_eq!(replies[0].type_, AgentMessage::Welcome as u16);
        let welcome: AgentWelcome = serde_json::from_slice(&replies[0].payload).unwrap();
        assert!(welcome.protocol == 1 && welcome.status_interval == 7 && welcome.host == "omaflake");
        let state = h.state().unwrap();
        assert!(state.is_ready() && !state.agent_is_older());
        assert_eq!(state.hello.as_ref().unwrap().os_name.as_deref(), Some("NixOS[31m")); // control characters dropped
        assert!(h.send(
            AgentMessage::Status,
            br#"{"uptime":12.5,"load":[0.5,-1,0.1],"memory":{"total":4,"available":2},"addresses":[],"users":["alice"]}"#
        ));
        let state = h.state().unwrap();
        let status = state.status.unwrap();
        assert_eq!(status.uptime, Some(12.5));
        assert_eq!(status.load, Some(vec![0.5, 0.1])); // negative numbers are dropped
        assert_eq!(status.users, Some(vec!["alice".to_string()]));
        // The file is what Swift's JSONEncoder.agent writes: pretty, sorted, ISO 8601 dates
        let text = std::fs::read_to_string(&h.state_file).unwrap();
        assert!(text.starts_with("{\n  \"features\" : [\n    \"status\"\n  ],\n  \"hello\" : {"), "{text}");
        assert!(state.hello_at.is_some() && text.contains("\"helloAt\" : \"20"));
    }

    #[test]
    fn status_too_soon_is_dropped() {
        let mut h = Harness::new(&["status"]);
        assert!(h.send(AgentMessage::Hello, HELLO.as_bytes()));
        assert!(h.send(AgentMessage::Status, br#"{"uptime":1}"#));
        assert!(h.send(AgentMessage::Status, br#"{"uptime":2}"#)); // within MIN_INTERVAL
        assert_eq!(h.state().unwrap().status.unwrap().uptime, Some(1.0));
    }

    #[test]
    fn no_common_version() {
        let mut h = Harness::new(&["status"]);
        assert!(h.send(AgentMessage::Hello, br#"{"agent":"9.0","protocol":{"min":7,"max":8}}"#));
        assert!(h.reply().is_empty());
        let state = h.state().unwrap();
        assert!(!state.is_ready());
        assert!(state.problem.unwrap().contains("7–8"));
    }

    #[test]
    fn goodbye_ends_readiness() {
        let mut h = Harness::new(&["status"]);
        assert!(h.send(AgentMessage::Hello, HELLO.as_bytes()));
        assert!(h.send(AgentMessage::Goodbye, br#"{"reason":"stopping"}"#));
        assert!(!h.state().unwrap().is_ready());
    }

    #[test]
    fn garbage_spends_the_budget() {
        let mut h = Harness::new(&["status"]);
        let mut ok = true;
        for _ in 0..=AgentLink::ERROR_BUDGET {
            if !ok {
                break;
            }
            ok = h.link.receive(b"junk");
        }
        assert!(!ok);
        assert!(h.state().unwrap().problem.is_some());
    }

    /// Large messages go in chunks and come back whole; a message over its limit is dropped with its tail (B106)
    #[test]
    fn chunks_and_reassembly() {
        let big: Vec<u8> = (0..(AgentProtocol::CHUNK * 2 + 100)).map(|i| (i % 251) as u8).collect();
        let mut d = AgentDecoder::new();
        d.feed(&AgentProtocol::encode_message(AgentMessage::Clipboard as u16, &big));
        let mut r = AgentReassembler::new();
        let (mut frames, mut whole) = (0, None);
        while let Some(f) = d.next_frame() {
            frames += 1;
            if let Some(m) = r.push(&f, big.len()) {
                whole = Some(m);
            }
        }
        assert_eq!(frames, 3);
        assert_eq!(whole.unwrap(), big);
        d.feed(&AgentProtocol::encode_message(AgentMessage::Clipboard as u16, &big));
        while let Some(f) = d.next_frame() {
            assert_eq!(r.push(&f, AgentProtocol::CHUNK), None);
        }
        assert_eq!(r.dropped, 1);
        d.feed(&AgentProtocol::encode_message(AgentMessage::Clipboard as u16, b"small"));
        let small = d.next_frame().unwrap();
        assert_eq!(r.push(&small, AgentProtocol::CHUNK).unwrap(), b"small");
    }

    /// The welcome enables what the agent offers and this VM allows; other features' messages are dropped
    #[test]
    fn features_and_clipboard() {
        let offering = br#"{"agent":"0.2.0","protocol":{"min":1,"max":1},"features":["status","clipboard","camera"]}"#;
        type Clip = Option<(String, Vec<u8>)>;
        let got: Arc<Mutex<Clip>> = Arc::default();
        let mut h = Harness::new(&["clipboard"]);
        let g = got.clone();
        h.link.on_clipboard = Some(Box::new(move |mime, data| *g.lock().unwrap() = Some((mime.into(), data.into()))));
        assert!(h.send(AgentMessage::Hello, offering));
        let welcome: AgentWelcome = serde_json::from_slice(&h.reply()[0].payload).unwrap();
        assert_eq!(welcome.features, ["clipboard", "status"]);
        let enabled: HashSet<String> = ["clipboard", "status"].map(String::from).into();
        assert_eq!(h.link.enabled(), &enabled);
        let content = "x".repeat(600_000).into_bytes(); // more than two chunks
        let message = serde_json::to_vec(
            &serde_json::json!({"mime": "text/plain;charset=utf-8", "data": BASE64.encode(&content)}),
        )
        .unwrap();
        assert!(h.send(AgentMessage::Clipboard, &message));
        assert_eq!(*got.lock().unwrap(), Some(("text/plain;charset=utf-8".to_string(), content)));
        // the camera was not allowed for this VM: its requests go nowhere
        let camera: Arc<Mutex<Option<CameraRequest>>> = Arc::default();
        let c = camera.clone();
        h.link.on_camera = Some(Box::new(move |r| *c.lock().unwrap() = Some(r)));
        assert!(h.send(AgentMessage::CameraControl, br#"{"action":"start"}"#));
        assert_eq!(*camera.lock().unwrap(), None);
        // other kinds of content are refused
        *got.lock().unwrap() = None;
        assert!(h.send(AgentMessage::Clipboard, br#"{"mime":"text/html","data":"aGk="}"#));
        assert_eq!(*got.lock().unwrap(), None);
    }

    #[test]
    fn camera_credits_are_capped() {
        let offering = br#"{"agent":"0.2.0","protocol":{"min":1,"max":1},"features":["status","camera"]}"#;
        let mut h = Harness::new(&["camera"]);
        let requests: Arc<Mutex<Vec<CameraRequest>>> = Arc::default();
        let r = requests.clone();
        h.link.on_camera = Some(Box::new(move |req| r.lock().unwrap().push(req)));
        assert!(h.send(AgentMessage::Hello, offering));
        assert!(h.send(AgentMessage::CameraControl, br#"{"action":"start"}"#));
        assert!(h.send(AgentMessage::CameraCredit, br#"{"credits":1000}"#));
        assert!(h.send(AgentMessage::CameraCredit, br#"{"credits":-3}"#));
        assert_eq!(
            *requests.lock().unwrap(),
            [CameraRequest::Start, CameraRequest::Credits(AgentProtocol::MAX_CAMERA_CREDITS)]
        );
    }

    /// A camera frame (1.4 MB) goes out as one frame, not in chunks – the guest writes it to /dev/video42 whole
    #[test]
    fn camera_frames_are_not_chunked() {
        let h = Harness::new(&["camera"]);
        let sender = h.link.sender();
        let t = std::thread::spawn(move || {
            sender.send(AgentMessage::CameraFrame, &vec![7u8; AgentProtocol::CAMERA_FRAME_BYTES])
        });
        let guest = h.guest.as_ref().unwrap();
        guest.set_read_timeout(Some(Duration::from_secs(5))).unwrap();
        let mut d = AgentDecoder::new();
        let mut buf = vec![0u8; 1 << 20];
        let mut got = None;
        while got.is_none() {
            let n = (&*guest).read(&mut buf).unwrap();
            assert!(n > 0);
            d.feed(&buf[..n]);
            got = d.next_frame();
        }
        assert!(t.join().unwrap());
        let got = got.unwrap();
        assert_eq!(got.type_, AgentMessage::CameraFrame as u16);
        assert_eq!((got.flags, got.payload.len()), (0, AgentProtocol::CAMERA_FRAME_BYTES));
    }

    #[test]
    fn run_ends_when_the_port_closes() {
        let mut h = Harness::new(&["status"]);
        h.guest = None;
        assert_eq!(h.link.run(), LinkEnd::Closed);
        // writing to a closed port fails instead of blocking
        assert!(!h.link.sender().send(AgentMessage::Welcome, b"{}"));
    }

    #[test]
    fn run_abandons_a_misbehaving_guest() {
        let mut h = Harness::new(&["status"]);
        let mut guest = h.guest.take().unwrap();
        let t = std::thread::spawn(move || {
            // Frames of a type this version does not know: one error each
            let _ = guest.write_all(&AgentProtocol::encode(0x0999, 0, b"").repeat(AgentLink::ERROR_BUDGET + 2));
            guest // kept open until the link gave up
        });
        assert_eq!(h.link.run(), LinkEnd::Abandoned);
        drop(t.join().unwrap());
    }

    /// The "midi" vectors shared with omaf-runner and the guest agent: records with and without time stamps
    #[test]
    fn midi_vectors() {
        let file = concat!(env!("CARGO_MANIFEST_DIR"), "/../../nix/agent/testdata/vectors.json");
        let v: Value = serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        let cases = v["midi"].as_array().unwrap();
        assert!(cases.iter().any(|c| c["records"].as_array().is_some_and(|r| r.iter().any(|r| r["time"].is_u64()))));
        for case in cases {
            let wire = bytes(case["hex"].as_str().unwrap());
            match case["records"].as_array() {
                Some(want) => {
                    let records: Vec<MidiRecord> = want
                        .iter()
                        .map(|r| MidiRecord {
                            id: r["id"].as_u64().unwrap() as u32,
                            protocol: r["protocol"].as_u64().unwrap() as u8,
                            time: r["time"].as_u64(),
                            words: r["words"].as_array().unwrap().iter().map(|w| w.as_u64().unwrap() as u32).collect(),
                        })
                        .collect();
                    assert_eq!(MidiRecord::decode(&wire).as_ref(), Some(&records), "{}", case["name"]);
                    assert_eq!(MidiRecord::encode(&records), vec![wire.clone()], "{}", case["name"]);
                }
                None => assert_eq!(MidiRecord::decode(&wire), None, "{}", case["name"]),
            }
        }
    }

    #[test]
    fn long_midi_records_are_split_and_keep_their_time() {
        let words: Vec<u32> = (0..40_000u32).map(|i| if i % 2 == 0 { 0x3016_0000 } else { i }).collect();
        let parts = MidiRecord::encode(&[MidiRecord { id: 7, protocol: 1, time: Some(42), words: words.clone() }]);
        assert!(parts.len() > 1 && parts.iter().all(|p| p.len() <= AgentProtocol::MAX_MIDI_MESSAGE));
        let back: Vec<MidiRecord> = parts.iter().flat_map(|p| MidiRecord::decode(p).unwrap()).collect();
        assert!(back.iter().all(|r| r.time == Some(42) && r.id == 7));
        assert_eq!(back.into_iter().flat_map(|r| r.words).collect::<Vec<_>>(), words);
        // a cut-off message at the end is not sent
        let half = MidiRecord { id: 1, protocol: 2, time: None, words: vec![0x4090_3C00] };
        assert!(MidiRecord::encode(&[half]).is_empty());
        assert_eq!(MidiRecord::message_words(0x2090_3C64), 1);
        assert_eq!(MidiRecord::message_words(0x4090_3C00), 2);
        assert_eq!(MidiRecord::message_words(0xF000_0000), 4);
    }

    #[test]
    fn endpoint_list_json() {
        let e = MidiEndpointInfo {
            id: 0x1401,
            direction: "in".into(),
            name: "Synth: Port".into(),
            manufacturer: None,
            model: None,
            protocol: 2,
            ump: Some(
                UmpEndpointInfo {
                    name: "Synth\u{7}".into(),
                    product_instance_id: "x".repeat(200),
                    manufacturer: Some("00201f".into()),
                    family: Some(1),
                    model: None,
                    version: None,
                    static_blocks: true,
                    blocks: vec![UmpBlockInfo {
                        id: 0,
                        name: "Main".into(),
                        direction: "both".into(),
                        first_group: 0,
                        groups: 1,
                        midi1: "no".into(),
                        ui_hint: "both".into(),
                    }],
                }
                .limited(),
            ),
        };
        let json = serde_json::to_value(&e).unwrap();
        assert_eq!(json["ump"]["name"], "Synth");
        assert_eq!(json["ump"]["productInstanceId"].as_str().unwrap().len(), UmpEndpointInfo::MAX_NAME);
        assert_eq!(json["ump"]["staticBlocks"], true);
        assert_eq!(json["ump"]["blocks"][0]["firstGroup"], 0);
        assert_eq!(json["ump"]["blocks"][0]["uiHint"], "both");
        assert!(json.get("manufacturer").is_none() && json["ump"].get("model").is_none());
        let back: MidiEndpointInfo = serde_json::from_value(json).unwrap();
        assert_eq!(back, e);
    }

    const MS: u64 = 1_000_000;

    #[test]
    fn clock_mapper_takes_the_fastest_delivery() {
        let mut c = ClockMapper::new();
        assert_eq!(c.deliver_at(1000 * MS, 0), None, "no offset yet: now");
        // The sender's clock is 500 ms behind; transport 3 ms, then 1 ms, then 5 ms
        c.observe(1003 * MS, 500 * MS);
        assert_eq!(c.offset(), Some(503 * MS as i128));
        c.observe(1011 * MS, 510 * MS);
        assert_eq!(c.offset(), Some(501 * MS as i128));
        // 1 ms slower than the fastest: delivered at its time + the fastest offset + 2 ms – the spacing is kept
        assert_eq!(c.schedule(1022 * MS, 520 * MS), Some(1023 * MS));
        assert_eq!(c.offset(), Some(501 * MS as i128));
        // the fastest one itself: 2 ms later than it came
        assert_eq!(c.schedule(1031 * MS, 530 * MS), Some(1033 * MS));
        // already past (the transport stalled for more than 2 ms): now
        assert_eq!(c.deliver_at(1050 * MS, 535 * MS), None);
        // more than 1 s ahead (a forged time): now
        assert_eq!(c.deliver_at(1050 * MS, 2000 * MS), None);
    }

    #[test]
    fn clock_mapper_window_slides_and_keeps_the_newest_minimum() {
        let mut c = ClockMapper::new();
        let s = 1_000 * MS;
        c.observe(s, 0); // offset 1000 ms
        c.observe(s + 10_000 * MS, 10_000 * MS - 2 * MS); // offset 1002 ms
        c.observe(s + 20_000 * MS, 20_000 * MS - 3 * MS); // offset 1003 ms
        assert_eq!(c.offset(), Some(1000 * MS as i128));
        // 31 s later the first sample left the window: the smallest of the rest
        c.observe(s + 31_000 * MS, 31_000 * MS - 4 * MS);
        assert_eq!(c.offset(), Some(1002 * MS as i128));
        // a new sample equal to the minimum replaces the older one, so it stays when that one leaves
        c.observe(s + 32_000 * MS, 32_000 * MS - 2 * MS);
        c.observe(s + 41_000 * MS, 41_000 * MS - 5 * MS);
        assert_eq!(c.offset(), Some(1002 * MS as i128), "the newest minimum (at 32 s) is still in the window");
        c.observe(s + 63_000 * MS, 63_000 * MS - 6 * MS);
        assert_eq!(c.offset(), Some(1005 * MS as i128));
    }

    #[test]
    fn midi_from_the_guest() {
        let offering = br#"{"agent":"0.3.0","protocol":{"min":1,"max":1},"features":["status","midi"]}"#;
        let mut h = Harness::new(&["midi"]);
        let got: Arc<Mutex<Vec<MidiRecord>>> = Arc::default();
        let g = got.clone();
        h.link.on_midi = Some(Box::new(move |r| g.lock().unwrap().extend(r)));
        assert!(h.send(AgentMessage::Hello, offering));
        assert!(h.link.enabled().contains("midi"));
        let note = MidiRecord { id: 3, protocol: 1, time: Some(77), words: vec![0x2090_3C64] };
        assert!(h.send(AgentMessage::MidiData, &MidiRecord::encode(std::slice::from_ref(&note))[0]));
        assert_eq!(*got.lock().unwrap(), [note]);
        // malformed data cost an error; the host's own messages too
        assert!(h.send(AgentMessage::MidiData, &bytes("0100000001020100643c9020")));
        assert!(h.send(AgentMessage::MidiPing, b"x"));
        assert_eq!(h.link.errors, 2);
        // status and round trip go into agent.json
        assert!(h.send(AgentMessage::MidiStatus, br#"{"endpoints":5,"failed":["bad\u001bname"]}"#));
        let sent = monotonic_ns().to_le_bytes();
        assert!(h.send(AgentMessage::MidiPong, &sent));
        assert!(h.send(AgentMessage::MidiPong, &u64::MAX.to_le_bytes())); // a forged time: ignored
        let midi = h.link.state().midi.clone().unwrap();
        assert_eq!((midi.endpoints, midi.failed), (5, vec!["badname".to_string()]));
        assert_eq!(midi.round_trip_ms.unwrap().samples, 1);
        assert_eq!(h.state().unwrap().midi.unwrap().endpoints, 5);
    }

    #[test]
    fn midi_off_for_the_vm_is_dropped() {
        let offering = br#"{"agent":"0.3.0","protocol":{"min":1,"max":1},"features":["status","midi"]}"#;
        let mut h = Harness::new(&["status"]);
        let got: Arc<Mutex<usize>> = Arc::default();
        let g = got.clone();
        h.link.on_midi = Some(Box::new(move |_| *g.lock().unwrap() += 1));
        assert!(h.send(AgentMessage::Hello, offering));
        assert!(h.send(AgentMessage::MidiData, &bytes("0403020101000100643c9020")));
        assert!(h.send(AgentMessage::MidiStatus, br#"{"endpoints":5}"#));
        assert_eq!(*got.lock().unwrap(), 0);
        assert!(h.state().unwrap().midi.is_none());
    }
}
