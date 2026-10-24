// SPDX-License-Identifier: AGPL-3.0-or-later
//! Texts of the views (docs/api.md §10.2) and the JSON helpers the TUI reads its models with. A text with a key is
//! translated into the language of the environment (`omaflake-i18n`: `OMAFLAKE_LANGUAGE`, `LC_ALL`, `LC_MESSAGES`,
//! `LANG`; English without a table) and filled in; the TUI's own words go through `tr`/`trf`. A text whose args name
//! an asked param (`{"param": …}`) is filled in with the value the user gave.

use std::sync::OnceLock;

use chrono::{DateTime, Utc};
pub use omaflake_i18n::Arg as I18nArg;
use serde_json::Value;

pub use omaflake_i18n::Language;

static LANG: OnceLock<Language> = OnceLock::new();

/// The language of the TUI: as set, else the environment's (its own unit tests: always English)
pub fn lang() -> Language {
    if cfg!(test) {
        return Language::English;
    }
    *LANG.get_or_init(Language::from_env)
}

/// Sets the language before the first text is shown (tests); later calls change nothing
pub fn set_language(language: Language) {
    let _ = LANG.set(language);
}

/// One of the TUI's own texts, translated
pub fn tr(key: &str) -> String {
    lang().tr(key)
}

/// One of the TUI's own texts with args (`%@`, `%lld`), translated
pub fn trf(key: &str, args: &[I18nArg]) -> String {
    lang().format(key, args)
}

/// A Text (`{key, text, args}`) as shown – translated –, a plain string or a number; "" for anything else
pub fn t(v: &Value) -> String {
    match v {
        Value::Object(o) if o.get("key").is_some_and(Value::is_string) => fill(v, &Value::Null),
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

/// A text translated and filled in, `{"param": …}` from `params` (what the user just chose); its English `text` when
/// the placeholders do not fit
pub fn fill(text: &Value, params: &Value) -> String {
    let english = || text.get("text").and_then(Value::as_str).unwrap_or_default().to_string();
    let Some(key) = text.get("key").and_then(Value::as_str) else { return t(text) };
    let args = arr(text, "args");
    let values: Vec<String> = args.iter().map(|a| arg_text(a, params)).collect();
    let ints: Vec<bool> = args.iter().map(Value::is_number).collect();
    lang().fill(key, &values, &ints).unwrap_or_else(english)
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

/// `key` translated, with its placeholders (`%@`, `%lld`, `%1$@`; `%%` a percent sign) filled in order
pub fn render(key: &str, args: &[Value], params: &Value) -> String {
    let values: Vec<String> = args.iter().map(|a| arg_text(a, params)).collect();
    let ints: Vec<bool> = args.iter().map(Value::is_number).collect();
    lang().fill(key, &values, &ints).unwrap_or_else(|| key.to_string())
}

/// "2026-10-21 10:00 UTC" (German "21.10.2026 10:00 UTC", as the core's texts in UTC)
pub fn date_text(iso: &str) -> String {
    match parse_date(iso) {
        Some(d) => format!("{} UTC", d.format(lang().date_time_pattern())),
        None => iso.to_string(),
    }
}

pub fn parse_date(iso: &str) -> Option<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(iso).ok().map(|d| d.with_timezone(&Utc))
}

/// "12.3 GB" (decimal units, as the core; "12,3 GB" in German)
pub fn bytes_text(bytes: i64) -> String {
    lang().bytes(bytes)
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
        let plain = json!({"key": "%@ of %@", "text": "1.0 GB of 2.0 GB", "args": [{"bytes": 1_000_000_000_i64}, {"bytes": 2_000_000_000_i64}]});
        assert_eq!(fill(&plain, &json!({})), "1.0 GB of 2.0 GB");
        assert_eq!(render("%lld%% · %@", &[json!(5), json!({"bytes": 2_000_000_000})], &json!({})), "5% · 2.0 GB");
        assert_eq!(duration_text(3909), "1:05:09");
        assert_eq!(duration_text(62), "1:02");
    }
}
