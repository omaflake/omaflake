// SPDX-License-Identifier: AGPL-3.0-or-later
//! The generated `src/tokens.rs` matches `brand/tokens.toml`: every token there is in [`ALL`] with the same value,
//! and nothing else is. Fails after a token change without `python3 brand/tokens.py`.

use omaflake_brand::{ALL, Token, Weight};
use std::collections::BTreeMap;
use toml::{Table, Value};

fn tokens() -> Table {
    let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../brand/tokens.toml");
    std::fs::read_to_string(path).expect("brand/tokens.toml").parse().expect("tokens.toml parses")
}

fn number(v: &Value) -> f32 {
    v.as_float().or_else(|| v.as_integer().map(|i| i as f64)).expect("a number") as f32
}

fn hex(s: &str) -> [f32; 3] {
    let s = s.trim_start_matches('#');
    assert_eq!(s.len(), 6, "#RRGGBB: {s}");
    let c = |i: usize| u8::from_str_radix(&s[i..i + 2], 16).unwrap() as f32 / 255.0;
    [c(0), c(2), c(4)]
}

fn weight(name: &str) -> Weight {
    [Weight::Regular, Weight::Medium, Weight::Semibold, Weight::Bold, Weight::Heavy]
        .into_iter()
        .find(|w| w.name() == name)
        .unwrap_or_else(|| panic!("unknown weight {name}"))
}

fn close(a: f32, b: f32) -> bool {
    (a - b).abs() < 1e-5
}

/// Every token of tokens.toml as (path, expected value)
fn expected() -> Vec<(String, Token)> {
    let t = tokens();
    let table = |name: &str| t[name].as_table().unwrap_or_else(|| panic!("[{name}]")).clone();
    let palette = table("palette");
    let mut out = Vec::new();
    let color = |rgb: [f32; 3], a: f32| {
        let rgba = [rgb[0], rgb[1], rgb[2], a];
        Token::Color(omaflake_brand::Color { rgba8: rgba.map(|x| (x * 255.0).round() as u8), rgba })
    };
    for (k, v) in &palette {
        out.push((format!("palette.{k}"), color(hex(v.as_str().unwrap()), 1.0)));
    }
    for (k, spec) in &table("color") {
        let rgb = if let Some(p) = spec.get("palette") {
            hex(palette[p.as_str().unwrap()].as_str().unwrap())
        } else if let Some(h) = spec.get("hex") {
            hex(h.as_str().unwrap())
        } else {
            let rgb: Vec<f32> = spec["rgb"].as_array().unwrap().iter().map(number).collect();
            [rgb[0], rgb[1], rgb[2]]
        };
        out.push((format!("color.{k}"), color(rgb, spec.get("alpha").map(number).unwrap_or(1.0))));
    }
    for group in ["opacity", "spacing", "radius", "glow", "size", "icon"] {
        for (k, v) in &table(group) {
            out.push((format!("{group}.{k}"), Token::Number(number(v))));
        }
    }
    for (k, v) in &table("font") {
        out.push((format!("font.{k}"), Token::Text(v.as_str().unwrap().to_string().leak())));
    }
    for group in ["font_style", "text"] {
        for (k, s) in &table(group) {
            let font = omaflake_brand::FontStyle {
                size: number(&s["size"]),
                weight: weight(s["weight"].as_str().unwrap()),
                monospaced: s.get("design").and_then(Value::as_str) == Some("monospaced"),
            };
            out.push((format!("{group}.{k}"), Token::Font(font)));
        }
    }
    out
}

fn same(a: &Token, b: &Token) -> bool {
    match (a, b) {
        (Token::Color(a), Token::Color(b)) => {
            a.rgba8 == b.rgba8 && a.rgba.iter().zip(b.rgba).all(|(x, y)| close(*x, y))
        }
        (Token::Number(a), Token::Number(b)) => close(*a, *b),
        (Token::Text(a), Token::Text(b)) => a == b,
        (Token::Font(a), Token::Font(b)) => {
            close(a.size, b.size) && a.weight == b.weight && a.monospaced == b.monospaced
        }
        _ => false,
    }
}

#[test]
fn generated_tokens_match_tokens_toml() {
    let generated: BTreeMap<&str, Token> = ALL.iter().copied().collect();
    assert_eq!(generated.len(), ALL.len(), "duplicate paths in ALL");
    let expected = expected();
    let hint = "run `python3 brand/tokens.py`";
    for (path, want) in &expected {
        let got = generated.get(path.as_str()).unwrap_or_else(|| panic!("{path} missing in tokens.rs – {hint}"));
        assert!(same(got, want), "{path}: tokens.rs has {got:?}, tokens.toml {want:?} – {hint}");
    }
    assert_eq!(generated.len(), expected.len(), "tokens.rs has tokens that tokens.toml lacks – {hint}");
}
