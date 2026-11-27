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

use serde::Serialize;
use serde_json::Value;

use crate::error::Result;

// The times of the API (plan B192: in omaflake-sdk)

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
    if let Some(f) = n.as_f64().filter(|_| n.is_f64())
        && f.is_finite()
        && f.fract() == 0.0
        && f.abs() < 1e15
    {
        out.push_str(&format!("{}", f as i64));
        return;
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
}
