// SPDX-License-Identifier: AGPL-3.0-or-later
//! The brand's design tokens (`omaflake-brand`, plan B119) as terminal colours: truecolor where the terminal says it
//! has it (`COLORTERM=truecolor|24bit`), else the nearest of the 256 xterm colours; none at all with `NO_COLOR`
//! (no-color.org) – then only bold, dim and reverse tell things apart. Colours with alpha are blended over the window
//! colour, as the app draws them over its background.

use omaflake_brand::{color, opacity, palette, Color as BrandColor};
use ratatui::style::{Color, Modifier, Style};

/// How many colours the terminal shows
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColorMode {
    TrueColor,
    Ansi256,
    None,
}

impl ColorMode {
    /// From the environment: `NO_COLOR` (any non-empty value) wins, then `COLORTERM`
    pub fn detect() -> ColorMode {
        Self::from_env(std::env::var("NO_COLOR").ok().as_deref(), std::env::var("COLORTERM").ok().as_deref())
    }

    pub fn from_env(no_color: Option<&str>, colorterm: Option<&str>) -> ColorMode {
        if no_color.is_some_and(|v| !v.is_empty()) {
            ColorMode::None
        } else if colorterm.is_some_and(|c| matches!(c.to_ascii_lowercase().as_str(), "truecolor" | "24bit")) {
            ColorMode::TrueColor
        } else {
            ColorMode::Ansi256
        }
    }
}

/// A view model's tone (api.md §10.3)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tone {
    Normal,
    Accent,
    Success,
    Warning,
    Danger,
    Lab,
}

impl Tone {
    pub fn parse(s: Option<&str>) -> Tone {
        match s {
            Some("accent") => Tone::Accent,
            Some("success") => Tone::Success,
            Some("warning") => Tone::Warning,
            Some("danger") => Tone::Danger,
            _ => Tone::Normal,
        }
    }
}

/// The styles the TUI draws with
#[derive(Debug, Clone, Copy)]
pub struct Theme {
    pub mode: ColorMode,
}

/// `c` over the window colour, alpha applied
fn blend(c: BrandColor, over: BrandColor) -> [u8; 3] {
    let a = c.rgba[3];
    let mix = |i: usize| (c.rgba8[i] as f32 * a + over.rgba8[i] as f32 * (1.0 - a)).round() as u8;
    [mix(0), mix(1), mix(2)]
}

/// `c` at `alpha` over the window colour
fn tint(c: BrandColor, alpha: f32) -> [u8; 3] {
    let mut c = c;
    c.rgba[3] = alpha;
    blend(c, color::WINDOW)
}

/// The nearest of the xterm 256 colours (the 6×6×6 cube and the 24 greys; the 16 system colours are left out – their
/// values depend on the terminal's theme)
pub fn nearest_256(rgb: [u8; 3]) -> u8 {
    const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let level = |v: u8| -> usize {
        LEVELS.iter().enumerate().min_by_key(|(_, &l)| (l as i32 - v as i32).abs()).map(|(i, _)| i).unwrap_or(0)
    };
    let (r, g, b) = (level(rgb[0]), level(rgb[1]), level(rgb[2]));
    let cube = [LEVELS[r], LEVELS[g], LEVELS[b]];
    let cube_index = 16 + 36 * r + 6 * g + b;
    let avg = (rgb[0] as u32 + rgb[1] as u32 + rgb[2] as u32) / 3;
    let grey_step = if avg < 8 { 0 } else { ((avg - 8) / 10).min(23) };
    let grey = 8 + 10 * grey_step as u8;
    let dist = |c: [u8; 3]| -> i32 { (0..3).map(|i| (c[i] as i32 - rgb[i] as i32).pow(2)).sum() };
    if dist([grey; 3]) < dist(cube) {
        232 + grey_step as u8
    } else {
        cube_index as u8
    }
}

impl Theme {
    pub fn new(mode: ColorMode) -> Theme {
        Theme { mode }
    }

    fn rgb(&self, rgb: [u8; 3]) -> Option<Color> {
        match self.mode {
            ColorMode::TrueColor => Some(Color::Rgb(rgb[0], rgb[1], rgb[2])),
            ColorMode::Ansi256 => Some(Color::Indexed(nearest_256(rgb))),
            ColorMode::None => None,
        }
    }

    fn fg(&self, c: BrandColor) -> Style {
        match self.rgb(blend(c, color::WINDOW)) {
            Some(c) => Style::new().fg(c),
            None => Style::new(),
        }
    }

    /// The whole screen: the window colour under primary text
    pub fn base(&self) -> Style {
        match (self.rgb(blend(color::WINDOW, palette::BLACK)), self.rgb(blend(color::TEXT_PRIMARY, color::WINDOW))) {
            (Some(bg), Some(fg)) => Style::new().bg(bg).fg(fg),
            _ => Style::new(),
        }
    }

    pub fn text(&self) -> Style {
        self.fg(color::TEXT_PRIMARY)
    }

    pub fn secondary(&self) -> Style {
        match self.mode {
            ColorMode::None => Style::new().add_modifier(Modifier::DIM),
            _ => self.fg(color::TEXT_SECONDARY),
        }
    }

    pub fn tone(&self, tone: Tone) -> Style {
        match tone {
            Tone::Normal => self.secondary(),
            Tone::Accent => self.fg(color::ACCENT),
            Tone::Success => self.fg(color::SUCCESS),
            Tone::Warning => self.fg(color::WARNING).add_modifier(self.plain_emphasis()),
            Tone::Danger => self.fg(color::DANGER).add_modifier(self.plain_emphasis()),
            Tone::Lab => self.fg(color::LAB),
        }
    }

    /// Without colours, warnings and dangers are at least bold
    fn plain_emphasis(&self) -> Modifier {
        if self.mode == ColorMode::None {
            Modifier::BOLD
        } else {
            Modifier::empty()
        }
    }

    pub fn accent(&self) -> Style {
        self.tone(Tone::Accent)
    }

    pub fn title(&self) -> Style {
        self.text().add_modifier(Modifier::BOLD)
    }

    /// A greyed-out action
    pub fn disabled(&self) -> Style {
        match self.mode {
            ColorMode::None => Style::new().add_modifier(Modifier::DIM),
            _ => match self.rgb(tint(color::TEXT_PRIMARY, opacity::DISABLED * 0.55 + 0.1)) {
                Some(c) => Style::new().fg(c),
                None => Style::new(),
            },
        }
    }

    /// The selected row
    pub fn selected(&self) -> Style {
        match self.rgb(tint(color::ACCENT, opacity::TINT_FILL)) {
            Some(c) => Style::new().bg(c),
            None => Style::new().add_modifier(Modifier::REVERSED),
        }
    }

    /// Borders, separators
    pub fn border(&self) -> Style {
        match self.rgb(blend(color::WINDOW_BORDER, color::WINDOW)) {
            Some(c) => Style::new().fg(c),
            None => Style::new().add_modifier(Modifier::DIM),
        }
    }

    /// The key hint in front of an action ("1", "s")
    pub fn key(&self) -> Style {
        self.accent().add_modifier(Modifier::BOLD)
    }

    /// The build log's box
    pub fn console(&self) -> Style {
        match self.rgb(blend(color::CONSOLE, palette::BLACK)) {
            Some(c) => self.text().bg(c),
            None => Style::new(),
        }
    }

    /// The modal's background (a card over the scrim)
    pub fn card(&self) -> Style {
        match self.rgb(tint(color::TEXT_PRIMARY, opacity::HOVER)) {
            Some(c) => self.text().bg(c),
            None => Style::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colour_mode_follows_the_environment() {
        assert_eq!(ColorMode::from_env(Some("1"), Some("truecolor")), ColorMode::None);
        assert_eq!(ColorMode::from_env(Some(""), Some("truecolor")), ColorMode::TrueColor);
        assert_eq!(ColorMode::from_env(None, Some("24bit")), ColorMode::TrueColor);
        assert_eq!(ColorMode::from_env(None, None), ColorMode::Ansi256);
    }

    #[test]
    fn brand_colours_map_to_the_nearest_of_256() {
        // Cyan #00E5FF → cube (0, 4, 5) = 45; red #FF3B3B → (5, 1, 1) = 203; the window #121212 → grey 233
        assert_eq!(nearest_256([0, 229, 255]), 45);
        assert_eq!(nearest_256([255, 59, 59]), 203);
        assert_eq!(nearest_256([18, 18, 18]), 233);
        assert_eq!(nearest_256([255, 255, 255]), 231);
        let t = Theme::new(ColorMode::TrueColor);
        assert_eq!(t.accent().fg, Some(Color::Rgb(0, 229, 255)));
        // Secondary text: white at 55 % over the window
        assert_eq!(t.secondary().fg, Some(Color::Rgb(148, 148, 148)));
        let plain = Theme::new(ColorMode::None);
        assert_eq!(plain.accent().fg, None);
        assert!(plain.tone(Tone::Danger).add_modifier.contains(Modifier::BOLD));
    }
}
