// SPDX-License-Identifier: AGPL-3.0-or-later
//! Texts of the views (docs/api.md §10.2) and the JSON helpers the TUI reads its models with. The TUI shows the
//! English `text`; translating by `key` is OPEN for the TUI (it would keep a table with the app's keys). A text whose
//! args name an asked param (`{"param": …}`) is filled in again with the value the user gave.

use chrono::{DateTime, Utc};
use serde_json::Value;

/// The English text of a Text (`{key, text, args}`), a plain string or a number; "" for anything else
pub fn t(v: &Value) -> String {
    match v {
        Value::Object(o) => o.get("text").and_then(Value::as_str).unwrap_or_default().to_string(),
        Value::String(s) => s.clone(),
        Value::Number(n) => n.to_string(),
        Value::Bool(b) => b.to_string(),
        _ => String::new(),
    }
}

/// `v[key]` as a string slice ("" when missing)
pub fn s<'a>(v: &'a Value, key: &str) -> &'a str {
    v.get(key).and_then(Value::as_str).unwrap_or_default()
}

/// `v[key]` as a bool (false when missing)
pub fn b(v: &Value, key: &str) -> bool {
    v.get(key).and_then(Value::as_bool).unwrap_or(false)
}

/// `v[key]` as an array (empty when missing)
pub fn arr<'a>(v: &'a Value, key: &str) -> &'a [Value] {
    v.get(key).and_then(Value::as_array).map(Vec::as_slice).unwrap_or_default()
}

/// The value at a dotted path (`params.path`)
pub fn get_path<'a>(v: &'a Value, path: &str) -> Option<&'a Value> {
    path.split('.').try_fold(v, |v, part| v.get(part))
}

/// Puts `value` at a dotted path, making objects on the way
pub fn set_path(v: &mut Value, path: &str, value: Value) {
    let mut cur = v;
    let parts: Vec<&str> = path.split('.').collect();
    for (i, part) in parts.iter().enumerate() {
        if !cur.is_object() {
            *cur = Value::Object(Default::default());
        }
        let obj = cur.as_object_mut().expect("made an object above");
        if i + 1 == parts.len() {
            obj.insert(part.to_string(), value);
            return;
        }
        cur = obj.entry(part.to_string()).or_insert_with(|| Value::Object(Default::default()));
    }
}

/// A text with its `{"param": …}` args filled from `params` (what the user just chose); else its `text`
pub fn fill(text: &Value, params: &Value) -> String {
    let args = arr(text, "args");
    let Some(key) = text.get("key").and_then(Value::as_str) else { return t(text) };
    if !args.iter().any(has_param) {
        return t(text);
    }
    render(key, args, params)
}

fn has_param(arg: &Value) -> bool {
    arg.get("param").is_some() || arr(arg, "args").iter().any(has_param)
}

fn arg_text(arg: &Value, params: &Value) -> String {
    match arg {
        Value::Number(n) => n.to_string(),
        Value::String(s) => s.clone(),
        Value::Object(o) => {
            if let Some(p) = o.get("param").and_then(Value::as_str) {
                get_path(params, p).map(t).filter(|s| !s.is_empty()).unwrap_or_else(|| format!("<{p}>"))
            } else if let Some(d) = o.get("date").and_then(Value::as_str) {
                date_text(d)
            } else if let Some(n) = o.get("bytes").and_then(Value::as_i64) {
                bytes_text(n)
            } else {
                fill(arg, params)
            }
        }
        _ => String::new(),
    }
}

/// `key` with its placeholders (`%@`, `%lld`; `%%` a percent sign) filled in order
pub fn render(key: &str, args: &[Value], params: &Value) -> String {
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
            out.push_str(&next.next().map(|a| arg_text(a, params)).unwrap_or_default());
            rest = r;
        } else {
            out.push('%');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// "2026-10-21 10:00 UTC" (as the core's English texts)
pub fn date_text(iso: &str) -> String {
    match parse_date(iso) {
        Some(d) => d.format("%Y-%m-%d %H:%M UTC").to_string(),
        None => iso.to_string(),
    }
}

pub fn parse_date(iso: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(iso).ok().map(|d| d.with_timezone(&Utc))
}

/// "12.3 GB" (decimal units, as the core)
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

/// "1:05:09", "4:02" – a running time
pub fn duration_text(secs: i64) -> String {
    let secs = secs.max(0);
    let (h, m, s) = (secs / 3600, secs / 60 % 60, secs % 60);
    if h > 0 {
        format!("{h}:{m:02}:{s:02}")
    } else {
        format!("{m}:{s:02}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn texts_and_paths() {
        assert_eq!(t(&json!({"key": "Run", "text": "Run"})), "Run");
        assert_eq!(t(&json!("plain")), "plain");
        let mut v = json!({"page": "import", "params": {"expect": "vm"}});
        set_path(&mut v, "params.path", json!("/x"));
        assert_eq!(v, json!({"page": "import", "params": {"expect": "vm", "path": "/x"}}));
        set_path(&mut v, "a.b", json!(1));
        assert_eq!(get_path(&v, "a.b"), Some(&json!(1)));
    }

    #[test]
    fn asked_params_fill_the_text() {
        let text = json!({"key": "Move the data folder to %@?", "text": "Move the data folder to <target>?",
                          "args": [{"param": "target"}]});
        assert_eq!(fill(&text, &json!({"target": "/Volumes/X"})), "Move the data folder to /Volumes/X?");
        assert_eq!(fill(&text, &json!({})), "Move the data folder to <target>?");
        let plain = json!({"key": "%@ of %@", "text": "1.0 GB of 2.0 GB", "args": [{"bytes": 1}, {"bytes": 2}]});
        assert_eq!(fill(&plain, &json!({})), "1.0 GB of 2.0 GB");
        assert_eq!(render("%lld%% · %@", &[json!(5), json!({"bytes": 2_000_000_000})], &json!({})), "5% · 2.0 GB");
        assert_eq!(duration_text(3909), "1:05:09");
        assert_eq!(duration_text(62), "1:02");
    }
}
