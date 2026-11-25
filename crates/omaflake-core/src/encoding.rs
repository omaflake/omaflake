// SPDX-License-Identifier: AGPL-3.0-or-later
//! Hex and Base64 – small enough not to need a crate (plan B177; the guest agent has the same Base64, `b64.rs`).

const HEX: &[u8; 16] = b"0123456789abcdef";
const BASE64: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

/// Lowercase hex of `data`
pub fn hex(data: impl AsRef<[u8]>) -> String {
    let data = data.as_ref();
    let mut out = String::with_capacity(data.len() * 2);
    for b in data {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 15) as usize] as char);
    }
    out
}

/// Uppercase hex of `data`
pub fn hex_upper(data: impl AsRef<[u8]>) -> String {
    hex(data).to_ascii_uppercase()
}

/// Base64, standard alphabet, with padding
pub fn base64(data: impl AsRef<[u8]>) -> String {
    let data = data.as_ref();
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = (b[0] as u32) << 16 | (b[1] as u32) << 8 | b[2] as u32;
        for i in 0..4 {
            if i <= chunk.len() {
                out.push(BASE64[(n >> (18 - 6 * i) & 63) as usize] as char);
            } else {
                out.push('=');
            }
        }
    }
    out
}

/// Base64 back to bytes (standard alphabet, padding required, whitespace ignored); None when it is not Base64
pub fn from_base64(text: &str) -> Option<Vec<u8>> {
    let bytes: Vec<u8> = text.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
    if !bytes.len().is_multiple_of(4) {
        return None;
    }
    let mut out = Vec::with_capacity(bytes.len() / 4 * 3);
    let last = bytes.len() / 4;
    for (k, chunk) in bytes.chunks(4).enumerate() {
        let mut n = 0u32;
        let mut pad = 0;
        for (i, &c) in chunk.iter().enumerate() {
            let v = match c {
                b'A'..=b'Z' => c - b'A',
                b'a'..=b'z' => c - b'a' + 26,
                b'0'..=b'9' => c - b'0' + 52,
                b'+' => 62,
                b'/' => 63,
                // padding only at the end of the last group
                b'=' if i >= 2 && k + 1 == last && chunk[i..].iter().all(|&p| p == b'=') => {
                    pad += 1;
                    0
                }
                _ => return None,
            };
            n = n << 6 | v as u32;
        }
        out.push((n >> 16) as u8);
        if pad < 2 {
            out.push((n >> 8) as u8);
        }
        if pad < 1 {
            out.push(n as u8);
        }
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_as_the_crate_wrote_it() {
        assert_eq!(hex([0x00, 0x0f, 0xa5, 0xff]), "000fa5ff");
        assert_eq!(hex_upper([0xab, 0x01]), "AB01");
        assert_eq!(hex(b""), "");
    }

    #[test]
    fn base64_vectors_of_rfc_4648() {
        for (plain, coded) in [
            ("", ""),
            ("f", "Zg=="),
            ("fo", "Zm8="),
            ("foo", "Zm9v"),
            ("foob", "Zm9vYg=="),
            ("fooba", "Zm9vYmE="),
            ("foobar", "Zm9vYmFy"),
        ] {
            assert_eq!(base64(plain), coded);
            assert_eq!(from_base64(coded).unwrap(), plain.as_bytes());
        }
        let all: Vec<u8> = (0..=255).collect();
        assert_eq!(from_base64(&base64(&all)).unwrap(), all);
        for bad in ["Zm9v!", "Zg=", "Z===", "Zg==Zg==", "Zm=v"] {
            assert!(from_base64(bad).is_none(), "{bad}");
        }
    }
}
