// SPDX-License-Identifier: AGPL-3.0-or-later
//! Clipboard messages (spec §11.9, type 0x0100): {"mime": …, "data": <base64>}. Text and PNG, at most 32 MiB.

use crate::b64;
use serde::{Deserialize, Serialize};

pub const TEXT: &str = "text/plain;charset=utf-8";
pub const PNG: &str = "image/png";

#[derive(Serialize, Deserialize)]
struct Message {
    mime: String,
    data: String,
}

pub fn encode(mime: &str, data: &[u8]) -> Vec<u8> {
    serde_json::to_vec(&Message { mime: mime.to_string(), data: b64::encode(data) }).expect("serializable")
}

/// (mime, data) if it is one of the two kinds the Mac and the guest exchange
pub fn decode(payload: &[u8]) -> Option<(String, Vec<u8>)> {
    let m: Message = serde_json::from_slice(payload).ok()?;
    if m.mime != TEXT && m.mime != PNG {
        return None;
    }
    let data = b64::decode(&m.data)?;
    (data.len() <= crate::proto::MAX_CLIPBOARD).then_some((m.mime, data))
}

/// Largest JSON payload of a clipboard message (base64 grows by a third)
pub const MAX_MESSAGE: usize = crate::proto::MAX_CLIPBOARD / 3 * 4 + 1024;

/// A short fingerprint of a content, to recognise the echo of what this side just wrote
pub fn fingerprint(mime: &str, data: &[u8]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    mime.hash(&mut h);
    data.hash(&mut h);
    h.finish()
}

#[cfg(test)]
mod tests {
    #[test]
    fn round_trip_and_kinds() {
        let p = super::encode(super::TEXT, "hällo".as_bytes());
        assert_eq!(super::decode(&p).unwrap(), (super::TEXT.to_string(), "hällo".as_bytes().to_vec()));
        let other = br#"{"mime":"text/html","data":"aGk="}"#;
        assert!(super::decode(other).is_none());
    }
}
