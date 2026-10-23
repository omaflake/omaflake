// SPDX-License-Identifier: AGPL-3.0-or-later
//! Texts of the views (docs/api.md §10.2): a message key, its English text and the values filled in. The key is the
//! English source string of the macOS app (`String(localized:)`, `LocalizedStringKey`) with `%@` for a text, a date or a
//! size and `%lld` for a whole number – so the app's `de.lproj/Localizable.strings` translates them as they are; other
//! UIs keep their own tables under the same keys. A text without a key is data (a name, a path) and is shown as it is.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// A user-facing text: `key` (absent: shown as it is), the English `text`, and the `args` its placeholders take, in order
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Text {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    pub text: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub args: Vec<Arg>,
}

/// A value of a placeholder. Numbers go into `%lld`; the others into `%@`: a plain text (data, not translated), a time
/// and a size (formatted by the UI in its language), a text of its own (translated by its key, then filled in), and the
/// value the UI asked for an action's `param` (a confirmation that names the folder just chosen).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Arg {
    Int(i64),
    Str(String),
    Date {
        #[serde(with = "crate::json::iso8601")]
        date: DateTime<Utc>,
    },
    Bytes {
        bytes: i64,
    },
    Param {
        param: String,
    },
    Text(Box<Text>),
}

impl From<&str> for Arg {
    fn from(s: &str) -> Arg {
        Arg::Str(s.to_string())
    }
}

impl From<String> for Arg {
    fn from(s: String) -> Arg {
        Arg::Str(s)
    }
}

impl From<&String> for Arg {
    fn from(s: &String) -> Arg {
        Arg::Str(s.clone())
    }
}

impl From<i64> for Arg {
    fn from(n: i64) -> Arg {
        Arg::Int(n)
    }
}

impl From<usize> for Arg {
    fn from(n: usize) -> Arg {
        Arg::Int(n as i64)
    }
}

impl From<u32> for Arg {
    fn from(n: u32) -> Arg {
        Arg::Int(n as i64)
    }
}

impl From<Text> for Arg {
    fn from(t: Text) -> Arg {
        Arg::Text(Box::new(t))
    }
}

impl Arg {
    pub fn date(date: DateTime<Utc>) -> Arg {
        Arg::Date { date }
    }

    pub fn bytes(bytes: i64) -> Arg {
        Arg::Bytes { bytes }
    }

    pub fn param(param: &str) -> Arg {
        Arg::Param { param: param.into() }
    }

    /// In English
    fn english(&self) -> String {
        match self {
            Arg::Int(n) => n.to_string(),
            Arg::Str(s) => s.clone(),
            Arg::Date { date } => date.format("%Y-%m-%d %H:%M UTC").to_string(),
            Arg::Bytes { bytes } => bytes_text(*bytes),
            Arg::Param { param } => format!("<{param}>"),
            Arg::Text(t) => t.text.clone(),
        }
    }
}

impl Text {
    /// A text without placeholders
    pub fn new(key: &str) -> Text {
        Text { key: Some(key.into()), text: key.into(), args: Vec::new() }
    }

    /// A text with placeholders (`%@`, `%lld`) filled from `args` in order
    pub fn fmt(key: &str, args: Vec<Arg>) -> Text {
        Text { key: Some(key.into()), text: render(key, &args), args }
    }

    /// Data shown as it is (a name, a path, a version) – no key, nothing to translate
    pub fn verbatim(text: impl Into<String>) -> Text {
        Text { key: None, text: text.into(), args: Vec::new() }
    }

    /// Several texts after each other with " · " between them (each translated on its own)
    pub fn joined(parts: Vec<Text>) -> Text {
        if parts.len() == 1 {
            return parts.into_iter().next().unwrap();
        }
        let key = vec!["%@"; parts.len()].join(" · ");
        Text::fmt(&key, parts.into_iter().map(Arg::from).collect())
    }
}

/// `key` with its placeholders replaced by the English values (`%%` is a percent sign)
pub(crate) fn render(key: &str, args: &[Arg]) -> String {
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
            out.push_str(&next.next().map(Arg::english).unwrap_or_default());
            rest = r;
        } else {
            out.push('%');
            rest = after;
        }
    }
    out.push_str(rest);
    out
}

/// "12.3 GB", "512.0 MB", "999 bytes" (decimal units, as the app's `gigabytes`)
pub(crate) fn bytes_text(bytes: i64) -> String {
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

/// `Text::fmt` with arguments of any type that becomes an `Arg`
macro_rules! tf {
    ($key:expr $(, $arg:expr)* $(,)?) => {
        $crate::api::views::text::Text::fmt($key, vec![$($crate::api::views::text::Arg::from($arg)),*])
    };
}
pub(crate) use tf;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn placeholders_are_filled_in_order() {
        let t = tf!("Upgrade “%@” to %@?", "research", "dev@2");
        assert_eq!(t.text, "Upgrade “research” to dev@2?");
        assert_eq!(tf!("%@ created (SSH port %lld)", "a", 2201_i64).text, "a created (SSH port 2201)");
        assert_eq!(render("100%% sure", &[]), "100% sure");
        let nested = tf!("Open the Doctor first: %@", Text::new("Nix"));
        assert_eq!(nested.text, "Open the Doctor first: Nix");
        assert_eq!(
            Text::fmt("%@ of %@ RAM free", vec![Arg::bytes(6_000_000_000), Arg::bytes(8_000_000_000)]).text,
            "6.0 GB of 8.0 GB RAM free"
        );
        assert_eq!(Text::joined(vec![Text::new("Sound"), Text::new("Clipboard")]).text, "Sound · Clipboard");
    }

    #[test]
    fn json_shape() {
        assert_eq!(serde_json::to_value(Text::verbatim("research")).unwrap(), json!({"text": "research"}));
        let t = Text::fmt(
            "%@ · %@ · %@ · %@",
            vec![
                Arg::from(3_i64),
                Arg::date(crate::json::parse_date("2026-10-21T10:00:00Z").unwrap()),
                Arg::bytes(5),
                Arg::from(Text::new("no VM")),
            ],
        );
        let v = serde_json::to_value(&t).unwrap();
        assert_eq!(
            v["args"],
            json!([3, {"date": "2026-10-21T10:00:00Z"}, {"bytes": 5}, {"key": "no VM", "text": "no VM"}])
        );
        assert_eq!(serde_json::from_value::<Text>(v).unwrap(), t);
        let p: Arg = serde_json::from_value(json!({"param": "target"})).unwrap();
        assert_eq!(p, Arg::param("target"));
    }
}
