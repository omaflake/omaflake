// SPDX-License-Identifier: AGPL-3.0-or-later
//! Texts of the views (docs/api.md §10.2) as this app shows them: translated by their key into the language of the
//! environment (`omaflake-i18n`: `OMAFLAKE_LANGUAGE`, `LC_ALL`, `LC_MESSAGES`, `LANG`; English without a table), filled in
//! with times in local time, sizes in decimal units, and `{"param": …}` with the value asked for. The app's own few
//! words go through `tr`/`trf` with the same tables.

use std::sync::OnceLock;

use chrono::{DateTime, Local, Utc};
use omaflake_core::api::views::{Arg, Text};
pub use omaflake_i18n::Arg as I18nArg;
use omaflake_i18n::Language;
use serde_json::Value;

/// The language of this app (tests: always English)
pub fn lang() -> Language {
    static LANG: OnceLock<Language> = OnceLock::new();
    if cfg!(test) {
        return Language::English;
    }
    *LANG.get_or_init(Language::from_env)
}

/// One of the app's own texts, translated
pub fn tr(key: &str) -> String {
    lang().tr(key)
}

/// One of the app's own texts with args (`%@`, `%lld`), translated
pub fn trf(key: &str, args: &[I18nArg]) -> String {
    lang().format(key, args)
}

/// A text as shown; `asked` holds the values asked for an action (a confirmation naming the folder just chosen)
pub fn show(t: &Text, asked: Option<&Value>) -> String {
    let Some(key) = &t.key else { return t.text.clone() };
    let values: Vec<String> = t.args.iter().map(|a| arg(a, asked)).collect();
    let ints: Vec<bool> = t.args.iter().map(|a| matches!(a, Arg::Int(_))).collect();
    lang().fill(key, &values, &ints).unwrap_or_else(|| t.text.clone())
}

/// `show` without asked values
pub fn s(t: &Text) -> String {
    show(t, None)
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

/// "2026-10-21 10:00" (German "21.10.2026 10:00") in local time
pub fn date_time(d: DateTime<Utc>) -> String {
    d.with_timezone(&Local).format(lang().date_time_pattern()).to_string()
}

/// "10:00" in local time (the activity list)
pub fn time(d: DateTime<Utc>) -> String {
    d.with_timezone(&Local).format("%H:%M").to_string()
}

/// "12.3 GB" (decimal units, as the core and the macOS app; "12,3 GB" in German)
pub fn bytes_text(bytes: i64) -> String {
    lang().bytes(bytes)
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
        assert_eq!(s(&text(json!({"key": "100%% sure", "text": "x"}))), "100% sure");
        // placeholders that do not fit the args: the English text
        assert_eq!(s(&text(json!({"key": "%lld VMs", "text": "two VMs", "args": ["two"]}))), "two VMs");
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
