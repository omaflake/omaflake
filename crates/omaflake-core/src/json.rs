// SPDX-License-Identifier: AGPL-3.0-or-later
//! JSON and dates as the Swift version writes them, so files stay interchangeable (no Swift counterpart file:
//! Foundation's `JSONEncoder`/`JSONSerialization` behaviour, ported once for every module).
//!
//! - `to_pretty` = Swift's `outputFormatting = [.prettyPrinted, .sortedKeys]`: two-space indent, `"key" : value`,
//!   keys sorted, empty arrays/objects as `[\n\n<indent>]`. `escape_slashes` = Swift's default of writing `/` as `\/`
//!   (`JSONEncoder` without `.withoutEscapingSlashes`); `FileFormat.encode` writes them unescaped.
//! - `iso8601` = Swift's `.iso8601` date strategy: `2026-10-21T10:00:00Z`, whole seconds.
//! - `iso8601_fractional` = the custom strategy of RecipeSources.swift (`.installed.json`, plan B111):
//!   `2026-10-21T10:00:00.123Z` written, with or without fractions read.
//!
//! Readers accept what Swift writes (escaped slashes, any key order, whole-number doubles like `0`).

use chrono::{DateTime, SecondsFormat, Timelike, Utc};
use serde::Serialize;
use serde_json::Value;

use crate::error::Result;

/// The current time in whole seconds – what survives a round trip through a Swift `.iso8601` file
pub fn now() -> DateTime<Utc> {
    let t = Utc::now();
    t.with_nanosecond(0).unwrap_or(t)
}

/// `value` as Swift's `JSONEncoder` with `[.prettyPrinted, .sortedKeys]` writes it
pub fn to_pretty<T: Serialize + ?Sized>(value: &T, escape_slashes: bool) -> Result<Vec<u8>> {
    let v = serde_json::to_value(value)?;
    Ok(value_to_pretty(&v, escape_slashes))
}

/// A `serde_json::Value` in Swift's pretty, sorted form (keys sort by bytes: `$schema` comes first)
pub fn value_to_pretty(v: &Value, escape_slashes: bool) -> Vec<u8> {
    let mut out = String::new();
    write_value(&mut out, v, 0, escape_slashes);
    out.into_bytes()
}

/// `value` compact with sorted keys (Swift `[.sortedKeys]`)
pub fn to_compact<T: Serialize + ?Sized>(value: &T) -> Result<Vec<u8>> {
    // serde_json's Map is a BTreeMap (no `preserve_order` feature): keys come out sorted
    Ok(serde_json::to_vec(&serde_json::to_value(value)?)?)
}

fn indent(out: &mut String, level: usize) {
    for _ in 0..level {
        out.push_str("  ");
    }
}

fn write_value(out: &mut String, v: &Value, level: usize, esc: bool) {
    match v {
        Value::Null => out.push_str("null"),
        Value::Bool(b) => out.push_str(if *b { "true" } else { "false" }),
        Value::Number(n) => write_number(out, n),
        Value::String(s) => write_string(out, s, esc),
        Value::Array(a) => {
            out.push('[');
            out.push('\n');
            if a.is_empty() {
                out.push('\n');
            }
            for (i, item) in a.iter().enumerate() {
                indent(out, level + 1);
                write_value(out, item, level + 1, esc);
                if i + 1 < a.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(out, level);
            out.push(']');
        }
        Value::Object(o) => {
            out.push('{');
            out.push('\n');
            if o.is_empty() {
                out.push('\n');
            }
            let mut keys: Vec<&String> = o.keys().collect();
            keys.sort();
            for (i, k) in keys.iter().enumerate() {
                indent(out, level + 1);
                write_string(out, k, esc);
                out.push_str(" : ");
                write_value(out, &o[k.as_str()], level + 1, esc);
                if i + 1 < keys.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            indent(out, level);
            out.push('}');
        }
    }
}

/// Whole-number doubles as integers (Swift writes `0`, not `0.0`)
fn write_number(out: &mut String, n: &serde_json::Number) {
    if let Some(f) = n.as_f64().filter(|_| n.is_f64()) {
        if f.is_finite() && f.fract() == 0.0 && f.abs() < 1e15 {
            out.push_str(&format!("{}", f as i64));
            return;
        }
    }
    out.push_str(&n.to_string());
}

fn write_string(out: &mut String, s: &str, esc: bool) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '/' if esc => out.push_str("\\/"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{08}' => out.push_str("\\b"),
            '\u{0c}' => out.push_str("\\f"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
}

/// Parses an ISO 8601 date as Swift's `ISO8601DateFormatter` writes it (with or without fractional seconds)
pub fn parse_date(text: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(text).ok().map(|d| d.with_timezone(&Utc))
}

/// Swift's `.iso8601` date strategy: `2026-10-21T10:00:00Z` (whole seconds, UTC). Use as
/// `#[serde(with = "crate::json::iso8601")]`; `iso8601::option` for `Option<DateTime<Utc>>`.
pub mod iso8601 {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn format(d: &DateTime<Utc>) -> String {
        d.to_rfc3339_opts(SecondsFormat::Secs, true)
    }

    pub fn serialize<S: Serializer>(d: &DateTime<Utc>, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&format(d))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<DateTime<Utc>, D::Error> {
        let text = String::deserialize(d)?;
        parse_date(&text)
            .ok_or_else(|| serde::de::Error::custom(format!("Expected date string to be ISO8601-formatted: {text}")))
    }

    /// For `Option<DateTime<Utc>>` fields (add `#[serde(default, skip_serializing_if = "Option::is_none")]`)
    pub mod option {
        use super::*;

        pub fn serialize<S: Serializer>(d: &Option<DateTime<Utc>>, s: S) -> std::result::Result<S::Ok, S::Error> {
            match d {
                Some(d) => s.serialize_str(&format(d)),
                None => s.serialize_none(),
            }
        }

        pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<Option<DateTime<Utc>>, D::Error> {
            match Option::<String>::deserialize(d)? {
                None => Ok(None),
                Some(text) => parse_date(&text).map(Some).ok_or_else(|| {
                    serde::de::Error::custom(format!("Expected date string to be ISO8601-formatted: {text}"))
                }),
            }
        }
    }
}

/// The date strategy of RecipeSources.swift's `.installed.json`: written with milliseconds
/// (`.withFractionalSeconds`, so versions of a line keep their order, plan B111), read with or without them
pub mod iso8601_fractional {
    use super::*;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn format(d: &DateTime<Utc>) -> String {
        d.to_rfc3339_opts(SecondsFormat::Millis, true)
    }

    pub fn serialize<S: Serializer>(d: &DateTime<Utc>, s: S) -> std::result::Result<S::Ok, S::Error> {
        s.serialize_str(&format(d))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> std::result::Result<DateTime<Utc>, D::Error> {
        let text = String::deserialize(d)?;
        parse_date(&text).ok_or_else(|| serde::de::Error::custom(format!("date {text}")))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn pretty_output_matches_swift() {
        let v = json!({"b": [1, 2], "$schema": "https://x/y", "a": {"z": [], "e": {}}, "d": 0.0, "f": 1.5});
        let text = String::from_utf8(value_to_pretty(&v, false)).unwrap();
        let expected = "{\n  \"$schema\" : \"https://x/y\",\n  \"a\" : {\n    \"e\" : {\n\n    },\n    \"z\" : [\n\n    ]\n  },\n  \"b\" : [\n    1,\n    2\n  ],\n  \"d\" : 0,\n  \"f\" : 1.5\n}";
        assert_eq!(text, expected);
        let escaped = String::from_utf8(value_to_pretty(&json!({"tz": "Europe/Berlin"}), true)).unwrap();
        assert_eq!(escaped, "{\n  \"tz\" : \"Europe\\/Berlin\"\n}");
        // Swift's escaped slashes read back as slashes
        let back: Value = serde_json::from_str(&escaped).unwrap();
        assert_eq!(back["tz"], "Europe/Berlin");
    }

    #[test]
    fn dates_as_swift_writes_them() {
        let d = parse_date("2026-10-21T10:00:00Z").unwrap();
        assert_eq!(iso8601::format(&d), "2026-10-21T10:00:00Z");
        let f = parse_date("2026-10-18T17:13:59.619Z").unwrap();
        assert_eq!(iso8601_fractional::format(&f), "2026-10-18T17:13:59.619Z");
        assert_eq!(iso8601::format(&f), "2026-10-18T17:13:59Z");
        assert!(parse_date("2026-10-18").is_none());
        assert_eq!(now().nanosecond(), 0);
    }
}
