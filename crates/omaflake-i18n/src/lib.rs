// SPDX-License-Identifier: AGPL-3.0-or-later
//! Translations of the Rust interfaces – `omaf ui` and the Linux app (plan B119, B121). The keys are the English texts
//! of the views (docs/api.md §10.2) and of the interfaces' own few words; the tables are generated from the macOS
//! app's `Localizable.strings` (`scripts/translations.py`), so every interface says the same. Filling in works as the
//! macOS app's `ViewTextRenderer`: `%@` takes a text, `%lld` (or `%d`) a number, `%1$@` a given position (a
//! translation may reorder), `%%` is a percent sign; a translation whose placeholders do not fit the args gives way to
//! the English key.
//!
//! The language: `OMAFLAKE_LANGUAGE` (`en`, `de`), else the locale's (`LC_ALL`, `LC_MESSAGES`, `LANG` – the first that
//! is set); English when there is no table for it.

#![forbid(unsafe_code)]

mod de;

/// A language omaflake's Rust interfaces speak
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Language {
    #[default]
    English,
    German,
}

impl Language {
    /// From a language code or a locale ("de", "de_DE.UTF-8", "de-AT"); English for anything without a table
    pub fn parse(code: &str) -> Language {
        let lang = code.split(['_', '-', '.', '@']).next().unwrap_or_default().to_ascii_lowercase();
        match lang.as_str() {
            "de" => Language::German,
            _ => Language::English,
        }
    }

    /// From the environment: `OMAFLAKE_LANGUAGE`, else the POSIX locale variables in their order
    pub fn from_env() -> Language {
        Self::from_vars(|name| std::env::var(name).ok())
    }

    /// `from_env` over any lookup (tests)
    pub fn from_vars(var: impl Fn(&str) -> Option<String>) -> Language {
        ["OMAFLAKE_LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"]
            .iter()
            .filter_map(|name| var(name))
            .find(|v| !v.is_empty())
            .map(|v| Self::parse(&v))
            .unwrap_or_default()
    }

    /// The code the core API takes (`initialize`'s `language`)
    pub fn code(self) -> &'static str {
        match self {
            Language::English => "en",
            Language::German => "de",
        }
    }

    fn table(self) -> &'static [(&'static str, &'static str)] {
        match self {
            Language::English => &[],
            Language::German => de::DE,
        }
    }

    /// The translation of `key`, None when the table has none (English has none: the key is the text)
    pub fn lookup(self, key: &str) -> Option<&'static str> {
        let table = self.table();
        table.binary_search_by(|(k, _)| k.as_bytes().cmp(key.as_bytes())).ok().map(|i| table[i].1)
    }

    /// `key` translated, without args (`%%` becomes `%`)
    pub fn tr(self, key: &str) -> String {
        self.format(key, &[])
    }

    /// `key` translated and filled in; `args` in order, `Arg::Int` for `%lld`. A translation that does not fit gives
    /// way to the key; a key that does not fit either comes back with its placeholders as they are.
    pub fn format(self, key: &str, args: &[Arg]) -> String {
        let values: Vec<&str> = args.iter().map(Arg::text).collect();
        let ints: Vec<bool> = args.iter().map(Arg::is_int).collect();
        self.lookup(key)
            .and_then(|t| fill(t, &values, &ints))
            .or_else(|| fill(key, &values, &ints))
            .unwrap_or_else(|| key.to_string())
    }

    /// `fill` of the translation, else of the key; None when neither fits (the caller shows its English text)
    pub fn fill(self, key: &str, values: &[String], ints: &[bool]) -> Option<String> {
        let values: Vec<&str> = values.iter().map(String::as_str).collect();
        self.lookup(key).and_then(|t| fill(t, &values, ints)).or_else(|| fill(key, &values, ints))
    }

    /// A size in decimal units, as the macOS app shows it: "12.3 GB" / "12,3 GB"
    pub fn bytes(self, bytes: i64) -> String {
        let b = bytes as f64;
        let (value, unit) = if b >= 1e12 {
            (b / 1e12, "TB")
        } else if b >= 1e9 {
            (b / 1e9, "GB")
        } else if b >= 1e6 {
            (b / 1e6, "MB")
        } else if b >= 1e3 {
            (b / 1e3, "KB")
        } else {
            return match self {
                Language::German => format!("{bytes} Byte"),
                Language::English => format!("{bytes} bytes"),
            };
        };
        let number = format!("{value:.1}");
        match self {
            Language::German => format!("{} {unit}", number.replace('.', ",")),
            Language::English => format!("{number} {unit}"),
        }
    }

    /// The `chrono` pattern of a date with time: "2026-10-21 10:00" / "21.10.2026 10:00"
    pub fn date_time_pattern(self) -> &'static str {
        match self {
            Language::English => "%Y-%m-%d %H:%M",
            Language::German => "%d.%m.%Y %H:%M",
        }
    }
}

/// How many keys a language's table has (the up-to-date test)
pub fn table_len(language: Language) -> usize {
    language.table().len()
}

/// An argument of a text: a number (`%lld`) or a text (`%@`)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Arg {
    Int(String),
    Text(String),
}

impl Arg {
    fn text(&self) -> &str {
        match self {
            Arg::Int(s) | Arg::Text(s) => s,
        }
    }
    fn is_int(&self) -> bool {
        matches!(self, Arg::Int(_))
    }
}

impl From<&str> for Arg {
    fn from(s: &str) -> Arg {
        Arg::Text(s.to_string())
    }
}

impl From<String> for Arg {
    fn from(s: String) -> Arg {
        Arg::Text(s)
    }
}

impl From<i64> for Arg {
    fn from(n: i64) -> Arg {
        Arg::Int(n.to_string())
    }
}

impl From<usize> for Arg {
    fn from(n: usize) -> Arg {
        Arg::Int(n.to_string())
    }
}

/// `format` with its placeholders replaced (Swift `ViewTextRenderer.fill`): `%@` a text, `%lld`/`%d` a number,
/// optionally at a position (`%2$@`), `%%` a percent sign. None when they do not fit the values – a missing value,
/// the wrong kind, or a value left unused.
pub fn fill(format: &str, values: &[&str], ints: &[bool]) -> Option<String> {
    let mut out = String::with_capacity(format.len());
    let mut next = 0;
    let mut used = vec![false; values.len()];
    let mut rest = format;
    while let Some(i) = rest.find('%') {
        out.push_str(&rest[..i]);
        let mut after = &rest[i + 1..];
        if let Some(r) = after.strip_prefix('%') {
            out.push('%');
            rest = r;
            continue;
        }
        let digits = after.len() - after.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        let mut position = None;
        if digits > 0 && after[digits..].starts_with('$') {
            position = Some(after[..digits].parse::<usize>().ok()?.checked_sub(1)?);
            after = &after[digits + 1..];
        }
        let (is_int, r) = if let Some(r) = after.strip_prefix('@') {
            (false, r)
        } else if let Some(r) = after.strip_prefix("lld") {
            (true, r)
        } else {
            (true, after.strip_prefix('d')?)
        };
        let index = position.unwrap_or(next);
        next += 1;
        if index >= values.len() || ints.get(index) != Some(&is_int) {
            return None;
        }
        used[index] = true;
        out.push_str(values[index]);
        rest = r;
    }
    out.push_str(rest);
    used.iter().all(|u| *u).then_some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn languages_from_the_environment() {
        let env = |pairs: &'static [(&'static str, &'static str)]| {
            move |name: &str| pairs.iter().find(|(k, _)| *k == name).map(|(_, v)| v.to_string())
        };
        assert_eq!(Language::from_vars(env(&[("LANG", "de_DE.UTF-8")])), Language::German);
        assert_eq!(Language::from_vars(env(&[("LANG", "de_DE.UTF-8"), ("LC_ALL", "C")])), Language::English);
        assert_eq!(
            Language::from_vars(env(&[("LC_ALL", ""), ("LC_MESSAGES", "de_AT"), ("LANG", "en_US")])),
            Language::German
        );
        assert_eq!(Language::from_vars(env(&[("LANG", "de_DE"), ("OMAFLAKE_LANGUAGE", "en")])), Language::English);
        assert_eq!(Language::from_vars(env(&[("LANG", "fr_FR.UTF-8")])), Language::English);
        assert_eq!(Language::from_vars(env(&[])), Language::English);
        assert_eq!(Language::parse("de-CH").code(), "de");
    }

    #[test]
    fn keys_translated_and_filled() {
        let de = Language::German;
        assert_eq!(de.tr("Remove …"), "Entfernen …");
        assert_eq!(de.format("Upgrade “%@” to %@?", &["a".into(), "b".into()]), "„a“ auf b umstellen?");
        assert_eq!(de.format("CPU cores: %lld", &[4i64.into()]), "CPU-Kerne: 4");
        // no translation: the English key, filled in
        assert_eq!(de.format("Nothing like this %@", &["x".into()]), "Nothing like this x");
        assert_eq!(
            Language::English.format("%@ created (SSH port %lld)", &["a".into(), 2201i64.into()]),
            "a created (SSH port 2201)"
        );
        assert_eq!(Language::English.tr("100%% sure"), "100% sure");
        // a translation that does not fit (kinds) gives way to the key
        assert_eq!(de.format("CPU cores: %lld", &["four".into()]), "CPU cores: %lld");
    }

    #[test]
    fn placeholders_must_fit() {
        assert_eq!(fill("%2$@ / %1$@", &["a", "b"], &[false, false]).as_deref(), Some("b / a"));
        assert_eq!(fill("%1$lld of %2$@", &["3", "x"], &[true, false]).as_deref(), Some("3 of x"));
        assert_eq!(fill("%@", &["a", "b"], &[false, false]), None); // one unused
        assert_eq!(fill("%@ %@", &["a"], &[false]), None); // one missing
        assert_eq!(fill("%lld", &["a"], &[false]), None); // wrong kind
        assert_eq!(fill("%x", &[], &[]), None);
        assert_eq!(fill("50%% done", &[], &[]).as_deref(), Some("50% done"));
    }

    #[test]
    fn the_table_is_sorted() {
        assert!(de::DE.windows(2).all(|w| w[0].0.as_bytes() < w[1].0.as_bytes()));
        assert!(de::DE.len() > 500);
    }

    #[test]
    fn sizes_and_dates() {
        assert_eq!(Language::English.bytes(12_300_000_000), "12.3 GB");
        assert_eq!(Language::German.bytes(12_300_000_000), "12,3 GB");
        assert_eq!(Language::German.bytes(999), "999 Byte");
        assert_eq!(Language::German.date_time_pattern(), "%d.%m.%Y %H:%M");
    }
}
