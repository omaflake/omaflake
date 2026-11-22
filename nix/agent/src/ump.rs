// SPDX-License-Identifier: AGPL-3.0-or-later
//! MIDI data on the agent's port (spec §11.9, plan B161): Universal MIDI Packets – the format of MIDI 2.0, which carries
//! MIDI 1.0 as well (UMP message types 0x1, 0x2, 0x3) – in records that name the endpoint they belong to.
//!
//! A MIDI data payload is records back to back:
//!
//! ```text
//! offset 0  id        u32   an endpoint of the Mac's list: Mac → guest the source the data came from,
//!                           guest → Mac the destination it goes to
//!        4  protocol  u8    1 = MIDI 1.0 UMP, 2 = MIDI 2.0 UMP (CoreMIDI's MIDIProtocolID)
//!        5  0         u8
//!        6  words     u16   how many UMP words follow
//!        8  words           u32 each, little endian; whole UMP messages only
//! ```
//!
//! The same encoding is in Sources/OmaflakeKit/Runner/AgentProtocol.swift (`MIDIRecord`); both test suites check the
//! "midi" vectors in testdata/vectors.json.

use crate::proto::MAX_MIDI_MESSAGE;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Record {
    pub id: u32,
    pub protocol: u8,
    pub words: Vec<u32>,
}

pub const RECORD_HEADER: usize = 8;

/// Words of the UMP message that starts with `first`: by its message type, the top four bits (UMP spec 1.1)
pub fn message_words(first: u32) -> usize {
    match first >> 28 {
        0x0 | 0x1 | 0x2 | 0x6 | 0x7 => 1,
        0x3 | 0x4 | 0x8 | 0x9 | 0xA => 2,
        0xB | 0xC => 3,
        _ => 4, // 0x5 (SysEx8, mixed data), 0xD (flex data), 0xE, 0xF (UMP stream)
    }
}

/// How many of `words` form whole UMP messages – a message cut off at the end is left out
pub fn whole_messages(words: &[u32]) -> usize {
    let mut i = 0;
    while i < words.len() {
        let n = message_words(words[i]);
        if i + n > words.len() {
            break;
        }
        i += n;
    }
    i
}

/// Records as MIDI data payloads of at most MAX_MIDI_MESSAGE bytes each; a long record is split at message boundaries
pub fn encode(records: &[Record]) -> Vec<Vec<u8>> {
    let max_words = ((MAX_MIDI_MESSAGE - RECORD_HEADER) / 4).min(u16::MAX as usize);
    let mut out: Vec<Vec<u8>> = vec![];
    let mut cur: Vec<u8> = vec![];
    for r in records {
        let words = &r.words[..whole_messages(&r.words)];
        let mut start = 0;
        while start < words.len() {
            let mut end = start;
            while end < words.len() && end + message_words(words[end]) - start <= max_words {
                end += message_words(words[end]);
            }
            let part = &words[start..end];
            if !cur.is_empty() && cur.len() + RECORD_HEADER + part.len() * 4 > MAX_MIDI_MESSAGE {
                out.push(std::mem::take(&mut cur));
            }
            cur.extend_from_slice(&r.id.to_le_bytes());
            cur.push(r.protocol);
            cur.push(0);
            cur.extend_from_slice(&(part.len() as u16).to_le_bytes());
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

/// The records of a MIDI data payload; None if it is malformed: too long, a record cut off, a protocol other than 1
/// or 2, or words that are not whole UMP messages
pub fn decode(payload: &[u8]) -> Option<Vec<Record>> {
    if payload.len() > MAX_MIDI_MESSAGE {
        return None;
    }
    let mut records = vec![];
    let mut at = 0;
    while at < payload.len() {
        let head = payload.get(at..at + RECORD_HEADER)?;
        let id = u32::from_le_bytes(head[0..4].try_into().unwrap());
        let protocol = head[4];
        let count = u16::from_le_bytes([head[6], head[7]]) as usize;
        if protocol != 1 && protocol != 2 {
            return None;
        }
        at += RECORD_HEADER;
        let body = payload.get(at..at + count * 4)?;
        let words: Vec<u32> = body.as_chunks::<4>().0.iter().map(|c| u32::from_le_bytes(*c)).collect();
        if whole_messages(&words) != words.len() {
            return None;
        }
        at += count * 4;
        records.push(Record { id, protocol, words });
    }
    Some(records)
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
        for case in v["midi"].as_array().unwrap() {
            let bytes = hex(case["hex"].as_str().unwrap());
            match case["records"].as_array() {
                Some(want) => {
                    let records: Vec<Record> = want
                        .iter()
                        .map(|r| Record {
                            id: r["id"].as_u64().unwrap() as u32,
                            protocol: r["protocol"].as_u64().unwrap() as u8,
                            words: r["words"].as_array().unwrap().iter().map(|w| w.as_u64().unwrap() as u32).collect(),
                        })
                        .collect();
                    assert_eq!(decode(&bytes).as_ref(), Some(&records), "{}", case["name"]);
                    assert_eq!(encode(&records), vec![bytes.clone()], "{}", case["name"]);
                }
                None => assert_eq!(decode(&bytes), None, "{}", case["name"]),
            }
        }
    }

    #[test]
    fn message_sizes() {
        assert_eq!(message_words(0x2090_3C64), 1); // MIDI 1.0 note on
        assert_eq!(message_words(0x4090_3C00), 2); // MIDI 2.0 note on
        assert_eq!(message_words(0x3016_7E7F), 2); // SysEx7
        assert_eq!(message_words(0x5000_0000), 4); // SysEx8
        assert_eq!(message_words(0xF000_0000), 4); // UMP stream
        assert_eq!(whole_messages(&[0x2090_3C64, 0x4090_3C00]), 1);
    }

    #[test]
    fn long_data_is_split_and_comes_back_whole() {
        let words: Vec<u32> = (0..40_000u32).map(|i| if i % 2 == 0 { 0x3016_0000 } else { i }).collect();
        let parts = encode(&[Record { id: 7, protocol: 1, words: words.clone() }]);
        assert!(parts.len() > 1 && parts.iter().all(|p| p.len() <= MAX_MIDI_MESSAGE));
        let back: Vec<u32> = parts.iter().flat_map(|p| decode(p).unwrap()).flat_map(|r| r.words).collect();
        assert_eq!(back, words);
        // a cut-off message at the end is not sent
        assert_eq!(encode(&[Record { id: 1, protocol: 2, words: vec![0x4090_3C00] }]), Vec::<Vec<u8>>::new());
    }
}
