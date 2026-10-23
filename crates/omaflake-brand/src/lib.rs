// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaflake's design tokens for the Rust interfaces (the iced app, plan B119): the same colours, spacing, radii and
//! type as the macOS app. The values live in `brand/tokens.toml`; `python3 brand/tokens.py` generates
//! `src/tokens.rs` (and the app's `BrandTokens.swift`). Plain values, no UI toolkit: with iced, e.g.
//! `iced::Color::from_rgba(r, g, b, a)` from [`Color::rgba`] and `iced::font::Weight` from [`Weight`].

// Generated with its own layout; `cargo fmt` leaves it as it is
#[rustfmt::skip]
mod tokens;
pub use tokens::*;

/// A colour in sRGB
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Color {
    /// 0–255, alpha included
    pub rgba8: [u8; 4],
    /// 0.0–1.0
    pub rgba: [f32; 4],
}

/// Font weight
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Weight {
    Regular,
    Medium,
    Semibold,
    Bold,
    Heavy,
}

impl Weight {
    /// The numeric (CSS/OpenType) weight
    pub const fn value(self) -> u16 {
        match self {
            Weight::Regular => 400,
            Weight::Medium => 500,
            Weight::Semibold => 600,
            Weight::Bold => 700,
            Weight::Heavy => 800,
        }
    }

    /// The name used in tokens.toml
    pub const fn name(self) -> &'static str {
        match self {
            Weight::Regular => "regular",
            Weight::Medium => "medium",
            Weight::Semibold => "semibold",
            Weight::Bold => "bold",
            Weight::Heavy => "heavy",
        }
    }
}

/// A font: size in points (logical pixels), weight, monospaced or the UI family
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FontStyle {
    pub size: f32,
    pub weight: Weight,
    pub monospaced: bool,
}

/// Any token, for [`ALL`]
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Token {
    Color(Color),
    Number(f32),
    Text(&'static str),
    Font(FontStyle),
}
