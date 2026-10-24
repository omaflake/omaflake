// SPDX-License-Identifier: AGPL-3.0-or-later
//! The generated tables match the macOS app's `Localizable.strings`: every pair there is in the table and nothing
//! else is. Fails after a change to the strings without `python3 scripts/translations.py`.

use omaflake_i18n::Language;
use std::collections::BTreeMap;

/// The pairs of a .strings file (the subset scripts/translations.py reads: comments, `"key" = "value";`, escapes)
fn parse(text: &str) -> BTreeMap<String, String> {
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    let mut pairs = BTreeMap::new();
    let skip = |mut i: usize| loop {
        if i < chars.len() && chars[i].is_whitespace() {
            i += 1;
        } else if chars[i..].starts_with(&['/', '*']) {
            i += 2;
            while !chars[i..].starts_with(&['*', '/']) {
                i += 1;
            }
            i += 2;
        } else if chars[i..].starts_with(&['/', '/']) {
            while i < chars.len() && chars[i] != '\n' {
                i += 1;
            }
        } else {
            return i;
        }
    };
    let string = |mut i: usize| {
        assert_eq!(chars[i], '"', "a string at {i}");
        i += 1;
        let mut out = String::new();
        loop {
            match chars[i] {
                '"' => return (out, i + 1),
                '\\' => {
                    match chars[i + 1] {
                        'n' => out.push('\n'),
                        't' => out.push('\t'),
                        'r' => out.push('\r'),
                        'u' | 'U' => {
                            let hex: String = chars[i + 2..i + 6].iter().collect();
                            out.push(char::from_u32(u32::from_str_radix(&hex, 16).unwrap()).unwrap());
                            i += 4;
                        }
                        c => out.push(c),
                    }
                    i += 2;
                }
                c => {
                    out.push(c);
                    i += 1;
                }
            }
        }
    };
    loop {
        i = skip(i);
        if i >= chars.len() {
            return pairs;
        }
        let (key, j) = string(i);
        i = skip(j);
        assert_eq!(chars[i], '=', "= after {key}");
        let (value, j) = string(skip(i + 1));
        i = skip(j);
        assert_eq!(chars[i], ';', "; after {key}");
        i += 1;
        pairs.insert(key, value);
    }
}

#[test]
fn german_is_up_to_date() {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../App/Resources/de.lproj/Localizable.strings");
    let Ok(text) = std::fs::read_to_string(path) else {
        // A source tree without the macOS app (the Nix package): nothing to compare with
        eprintln!("no {path} – skipped");
        return;
    };
    let pairs = parse(&text);
    let stale: Vec<&String> =
        pairs.iter().filter(|(k, v)| Language::German.lookup(k) != Some(v.as_str())).map(|(k, _)| k).collect();
    assert!(stale.is_empty(), "not in the table (run python3 scripts/translations.py): {stale:?}");
    assert_eq!(omaflake_i18n::table_len(Language::German), pairs.len(), "the table has more (run the script)");
}
