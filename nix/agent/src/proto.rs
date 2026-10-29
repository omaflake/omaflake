// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaflake agent protocol: frames and messages (spec §11.9, "Wire protocol").
//!
//! A frame is a 12-byte header followed by its payload; all integers little endian:
//!
//! ```text
//! offset 0  magic   "OMFA"
//!        4  type    u16   message type (MSG_*)
//!        6  flags   u16   bit 0 = MORE: the payload continues in the next frame of the same type
//!        8  length  u32   payload bytes, at most MAX_PAYLOAD
//! ```
//!
//! The header never changes; the protocol version is agreed once per connection (hello/welcome).
//! The same encoding is implemented on the Mac in Sources/OmaflakeCore/AgentProtocol.swift; both test suites
//! check the vectors in testdata/vectors.json.

pub const MAGIC: [u8; 4] = *b"OMFA";
pub const HEADER_LEN: usize = 12;
/// Largest payload of one frame: a 1280×720 NV12 camera frame (1,382,400 bytes) plus its header fits
pub const MAX_PAYLOAD: usize = 2 * 1024 * 1024;
pub const FLAG_MORE: u16 = 1;
/// Large messages go in chunks of at most this size, so a control message waits for one chunk at most
pub const CHUNK: usize = 256 * 1024;

/// Protocol versions this agent speaks
pub const PROTOCOL_MIN: u32 = 1;
pub const PROTOCOL_MAX: u32 = 1;

// Core (0x00xx)
pub const MSG_HELLO: u16 = 0x0001; // guest → Mac, JSON
pub const MSG_WELCOME: u16 = 0x0002; // Mac → guest, JSON
pub const MSG_STATUS: u16 = 0x0003; // guest → Mac, JSON
pub const MSG_GOODBYE: u16 = 0x0004; // guest → Mac, JSON: the agent stops (shutdown, restart)

// Clipboard (0x01xx): {"mime": …, "data": <base64>} in MORE chunks – both directions
pub const MSG_CLIPBOARD: u16 = 0x0100;
/// Largest clipboard content (before base64)
pub const MAX_CLIPBOARD: usize = 32 * 1024 * 1024;

// Camera (0x02xx): the Mac's camera as /dev/video42, frames only against credits (spec §11.9)
pub const MSG_CAMERA_CONTROL: u16 = 0x0200; // guest → Mac, JSON {"action": "start" | "stop"}
pub const MSG_CAMERA_CREDIT: u16 = 0x0201; // guest → Mac, JSON {"credits": n}: n more frames may come
pub const MSG_CAMERA_FRAME: u16 = 0x0202; // Mac → guest, binary: one 1280×720 NV12 frame
pub const MSG_CAMERA_STATUS: u16 = 0x0203; // Mac → guest, JSON {"status", "detail"}

/// A message as frames: one frame, or chunks of CHUNK bytes with MORE on all but the last
pub fn encode_message(kind: u16, payload: &[u8]) -> Vec<u8> {
    if payload.len() <= CHUNK {
        return encode(kind, 0, payload);
    }
    let mut out = Vec::with_capacity(payload.len() + payload.len() / CHUNK * HEADER_LEN + HEADER_LEN);
    let mut chunks = payload.chunks(CHUNK).peekable();
    while let Some(chunk) = chunks.next() {
        out.extend(encode(kind, if chunks.peek().is_some() { FLAG_MORE } else { 0 }, chunk));
    }
    out
}

/// Puts chunked messages back together; drops one that grows beyond `limit` (and counts it) – up to its last
/// frame, so its tail never passes for a message of its own
#[derive(Default)]
pub struct Reassembler {
    /// Per type: the message so far, or None while the rest of a dropped one is skipped
    parts: std::collections::HashMap<u16, Option<Vec<u8>>>,
    pub dropped: u64,
}

impl Reassembler {
    /// The whole message once its last frame is in, else None
    pub fn push(&mut self, frame: Frame, limit: usize) -> Option<Vec<u8>> {
        let last = frame.flags & FLAG_MORE == 0;
        let slot = self.parts.entry(frame.kind).or_insert_with(|| Some(Vec::new()));
        match slot {
            None => {} // skipping a dropped message
            Some(buf) if buf.len() + frame.payload.len() > limit => {
                *slot = None;
                self.dropped += 1;
            }
            Some(buf) => buf.extend_from_slice(&frame.payload),
        }
        if !last {
            return None;
        }
        self.parts.remove(&frame.kind).flatten()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    pub kind: u16,
    pub flags: u16,
    pub payload: Vec<u8>,
}

pub fn encode(kind: u16, flags: u16, payload: &[u8]) -> Vec<u8> {
    assert!(payload.len() <= MAX_PAYLOAD, "payload too large");
    let mut out = Vec::with_capacity(HEADER_LEN + payload.len());
    out.extend_from_slice(&MAGIC);
    out.extend_from_slice(&kind.to_le_bytes());
    out.extend_from_slice(&flags.to_le_bytes());
    out.extend_from_slice(&(payload.len() as u32).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

/// Incremental decoder: feed bytes as they arrive, take complete frames out. On a bad header it drops bytes up to
/// the next magic and counts the error (resynchronisation), so one corrupt frame does not end the connection.
#[derive(Default)]
pub struct Decoder {
    buf: Vec<u8>,
    pub errors: u64,
}

impl Decoder {
    pub fn feed(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    pub fn next_frame(&mut self) -> Option<Frame> {
        loop {
            // The magic is checked as soon as it can be, so garbage is not held until a full header arrived
            if self.buf.len() >= 4 && self.buf[0..4] != MAGIC {
                self.errors += 1;
                self.skip_to_magic();
                continue;
            }
            if self.buf.len() < HEADER_LEN {
                return None;
            }
            let length = u32::from_le_bytes(self.buf[8..12].try_into().unwrap()) as usize;
            if length > MAX_PAYLOAD {
                self.errors += 1;
                self.skip_to_magic();
                continue;
            }
            if self.buf.len() < HEADER_LEN + length {
                return None;
            }
            let kind = u16::from_le_bytes([self.buf[4], self.buf[5]]);
            let flags = u16::from_le_bytes([self.buf[6], self.buf[7]]);
            let payload = self.buf[HEADER_LEN..HEADER_LEN + length].to_vec();
            self.buf.drain(..HEADER_LEN + length);
            return Some(Frame { kind, flags, payload });
        }
    }

    /// Drops at least one byte, then everything before the next possible magic
    fn skip_to_magic(&mut self) {
        let start = (1..self.buf.len())
            .find(|&i| MAGIC.starts_with(&self.buf[i..self.buf.len().min(i + 4)]))
            .unwrap_or(self.buf.len());
        self.buf.drain(..start);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    /// The vectors shared with the Mac side's tests
    #[test]
    fn vectors() {
        let text = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/vectors.json")).unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        for case in v["frames"].as_array().unwrap() {
            let payload = case["payload"].as_str().unwrap().as_bytes();
            let kind = case["type"].as_u64().unwrap() as u16;
            let flags = case["flags"].as_u64().unwrap() as u16;
            let bytes = hex(case["hex"].as_str().unwrap());
            assert_eq!(encode(kind, flags, payload), bytes, "{}", case["name"]);
            let mut d = Decoder::default();
            d.feed(&bytes);
            assert_eq!(d.next_frame(), Some(Frame { kind, flags, payload: payload.to_vec() }));
            assert_eq!(d.next_frame(), None);
        }
        for case in v["streams"].as_array().unwrap() {
            let mut d = Decoder::default();
            d.feed(&hex(case["hex"].as_str().unwrap()));
            let mut kinds = vec![];
            while let Some(f) = d.next_frame() {
                kinds.push(f.kind as u64);
            }
            let want: Vec<u64> = case["types"].as_array().unwrap().iter().map(|t| t.as_u64().unwrap()).collect();
            assert_eq!(kinds, want, "{}", case["name"]);
            assert_eq!(d.errors, case["errors"].as_u64().unwrap(), "{}", case["name"]);
        }
    }

    #[test]
    fn chunked_messages_come_back_whole() {
        let big: Vec<u8> = (0..(CHUNK * 2 + 100)).map(|i| (i % 251) as u8).collect();
        let bytes = encode_message(MSG_CLIPBOARD, &big);
        let mut d = Decoder::default();
        d.feed(&bytes);
        let mut r = Reassembler::default();
        let mut whole = None;
        let mut frames = 0;
        while let Some(f) = d.next_frame() {
            frames += 1;
            if let Some(m) = r.push(f, MAX_CLIPBOARD * 2) {
                whole = Some(m);
            }
        }
        assert_eq!(frames, 3);
        assert_eq!(whole.unwrap(), big);
        // a message beyond the limit is dropped, the next one arrives
        let mut r = Reassembler::default();
        let mut d = Decoder::default();
        d.feed(&encode_message(MSG_CLIPBOARD, &big));
        while let Some(f) = d.next_frame() {
            assert!(r.push(f, CHUNK).is_none());
        }
        assert_eq!(r.dropped, 1);
        d.feed(&encode_message(MSG_CLIPBOARD, b"small"));
        assert_eq!(r.push(d.next_frame().unwrap(), CHUNK).unwrap(), b"small");
    }

    #[test]
    fn split_delivery() {
        let bytes = encode(MSG_STATUS, 0, b"{\"uptime\":1}");
        let mut d = Decoder::default();
        for b in &bytes[..bytes.len() - 1] {
            d.feed(&[*b]);
            assert_eq!(d.next_frame(), None);
        }
        d.feed(&bytes[bytes.len() - 1..]);
        assert_eq!(d.next_frame().unwrap().kind, MSG_STATUS);
    }
}
