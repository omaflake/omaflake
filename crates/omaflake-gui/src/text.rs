// SPDX-License-Identifier: AGPL-3.0-or-later
//! Texts of the views (docs/api.md §10.2) as this app shows them: English (no translation table yet – OPEN), filled
//! in from the key with times in local time, sizes in decimal units, and `{"param": …}` with the value asked for.

use chrono::{DateTime, Local, Utc};
use omaflake_core::api::views::{Arg, Text};
use serde_json::Value;

/// A text as shown; `asked` holds the values asked for an action (a confirmation naming the folder just chosen)
pub fn show(t: &Text, asked: Option<&Value>) -> String {
    match &t.key {
        Some(key) if !t.args.is_empty() => render(key, &t.args, asked),
        Some(key) => key.clone(),
        None => t.text.clone(),
    }
}

/// `show` without asked values
pub fn s(t: &Text) -> String {
    show(t, None)
}

fn render(key: &str, args: &[Arg], asked: Option<&Value>) -> String {
    let mut out = String::new();
    let mut rest = key;
    let mut next = args.iter();
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let after = &rest[i + 1..];
        if let Some(r) = after.strip_prefix('%') {
            out.push('%');
            rest = r;
        } else if let Some(r) = after.strip_prefix('@').or_else(|| after.strip_prefix("lld")) {
            out.push_str(&next.next().map(|a| arg(a, asked)).unwrap_or_default());
            rest = r;
        } else {
            out.push('%');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

fn arg(a: &Arg, asked: Option<&Value>) -> String {
    match a {
        Arg::Int(n) => n.to_string(),
        Arg::Str(s) => s.clone(),
        Arg::Date { date } => date_time(*date),
        Arg::Bytes { bytes } => bytes_text(*bytes),
        Arg::Param { param } => asked
            .and_then(|p| lookup(p, param))
            .map(|v| v.as_str().map(str::to_string).unwrap_or_else(|| v.to_string()))
            .unwrap_or_else(|| format!("<{param}>")),
        Arg::Text(t) => show(t, asked),
    }
}

/// The value at a dotted path (`params.path`)
pub fn lookup<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |at, p| at.get(p))
}

/// "2026-10-21 10:00" in local time
pub fn date_time(d: DateTime<Utc>) -> String {
    d.with_timezone(&Local).format("%Y-%m-%d %H:%M").to_string()
}

/// "10:00" in local time (the activity list)
pub fn time(d: DateTime<Utc>) -> String {
    d.with_timezone(&Local).format("%H:%M").to_string()
}

/// "12.3 GB" (decimal units, as the core and the macOS app)
pub fn bytes_text(bytes: i64) -> String {
    let b = bytes as f64;
    if b >= 1e9 {
        format!("{:.1} GB", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} MB", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.1} KB", b / 1e3)
    } else {
        format!("{bytes} bytes")
    }
}

/// "1:05:09" / "4:09" – a running time
pub fn duration(secs: i64) -> String {
    let s = secs.max(0);
    if s >= 3600 {
        format!("{}:{:02}:{:02}", s / 3600, s % 3600 / 60, s % 60)
    } else {
        format!("{}:{:02}", s / 60, s % 60)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn text(v: Value) -> Text {
        serde_json::from_value(v).unwrap()
    }

    #[test]
    fn keys_are_filled_in() {
        let t = text(json!({"key": "Upgrade “%@” to %@?", "text": "x", "args": ["research", "dev@2"]}));
        assert_eq!(s(&t), "Upgrade “research” to dev@2?");
        let t = text(json!({"key": "%@ created (SSH port %lld)", "text": "x", "args": ["a", 2201]}));
        assert_eq!(s(&t), "a created (SSH port 2201)");
        let t = text(
            json!({"key": "%@ · %@", "text": "x", "args": [{"bytes": 12_300_000_000_i64}, {"key": "no VM", "text": "no VM"}]}),
        );
        assert_eq!(s(&t), "12.3 GB · no VM");
        assert_eq!(s(&text(json!({"text": "/data"}))), "/data");
        assert_eq!(s(&text(json!({"key": "100%% sure", "text": "x", "args": [1]}))), "100% sure");
    }

    #[test]
    fn asked_values_fill_params() {
        let t = text(json!({"key": "Move to %@?", "text": "x", "args": [{"param": "target"}]}));
        assert_eq!(show(&t, Some(&json!({"target": "/Volumes/X"}))), "Move to /Volumes/X?");
        assert_eq!(show(&t, None), "Move to <target>?");
        assert_eq!(lookup(&json!({"params": {"path": "/a"}}), "params.path"), Some(&json!("/a")));
    }

    #[test]
    fn durations() {
        assert_eq!(duration(249), "4:09");
        assert_eq!(duration(3909), "1:05:09");
        assert_eq!(bytes_text(999), "999 bytes");
    }
}
