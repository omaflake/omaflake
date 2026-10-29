// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaflake's look in iced: every colour, spacing, radius and text size comes from the design tokens
//! (`omaflake-brand`, generated from brand/tokens.toml) – the same values as the macOS app's `BrandStyle`.

use iced::font::Weight as IcedWeight;
use iced::widget::{button, container, pick_list, progress_bar, scrollable, text_input, toggler};
use iced::{Background, Border, Color, Font, Shadow, Theme, Vector, border};
use omaflake_brand as brand;
use omaflake_core::api::views::common::{LampState, Tone};

/// A token colour as iced's colour
pub const fn rgba(c: brand::Color) -> Color {
    Color { r: c.rgba[0], g: c.rgba[1], b: c.rgba[2], a: c.rgba[3] }
}

pub fn alpha(c: Color, a: f32) -> Color {
    Color { a: c.a * a, ..c }
}

pub const WINDOW: Color = rgba(brand::color::WINDOW);
pub const ACCENT: Color = rgba(brand::color::ACCENT);
pub const DANGER: Color = rgba(brand::color::DANGER);
pub const WARNING: Color = rgba(brand::color::WARNING);
pub const SUCCESS: Color = rgba(brand::color::SUCCESS);
pub const TEXT: Color = rgba(brand::color::TEXT_PRIMARY);
pub const SECONDARY: Color = rgba(brand::color::TEXT_SECONDARY);
pub const CARD: Color = rgba(brand::color::CARD);
pub const CARD_BORDER: Color = rgba(brand::color::CARD_BORDER);
pub const CONSOLE: Color = rgba(brand::color::CONSOLE);
pub const SCRIM: Color = rgba(brand::color::SCRIM);
pub const RED: Color = rgba(brand::palette::RED);
pub const CYAN: Color = rgba(brand::palette::CYAN);

/// macOS' tertiary label: the secondary one, fainter (SwiftUI `.tertiary`; no token of its own)
pub fn tertiary() -> Color {
    alpha(TEXT, 0.25)
}

pub use brand::{icon as icon_size, radius, size, spacing};

/// Text sizes (macOS' system text styles, `[text]` in tokens.toml)
pub mod text_size {
    use omaflake_brand::text;
    pub const HEADLINE: f32 = text::HEADLINE.size;
    pub const BODY: f32 = text::BODY.size;
    pub const CALLOUT: f32 = text::CALLOUT.size;
    pub const SUBHEADLINE: f32 = text::SUBHEADLINE.size;
    pub const CAPTION: f32 = text::CAPTION.size;
    pub const CAPTION2: f32 = text::CAPTION2.size;
}

fn weight(w: brand::Weight) -> IcedWeight {
    match w {
        brand::Weight::Regular => IcedWeight::Normal,
        brand::Weight::Medium => IcedWeight::Medium,
        brand::Weight::Semibold => IcedWeight::Semibold,
        brand::Weight::Bold => IcedWeight::Bold,
        brand::Weight::Heavy => IcedWeight::ExtraBold,
    }
}

/// The UI family (`font.ui` = the system's sans serif). Linux: fontconfig's sans-serif, which has every weight. On
/// macOS (a development aid) the generic family falls back to a serif face for bold weights, so it is named.
pub const UI: Font = if cfg!(target_os = "macos") { Font::with_name("Helvetica Neue") } else { Font::DEFAULT };

/// The UI family in a weight
pub fn ui(w: brand::Weight) -> Font {
    Font { weight: weight(w), ..UI }
}

pub fn medium() -> Font {
    ui(brand::Weight::Medium)
}

pub fn semibold() -> Font {
    ui(brand::Weight::Semibold)
}

pub fn bold() -> Font {
    ui(brand::Weight::Bold)
}

/// A token font style as an iced font (monospaced or the UI family)
pub fn font(f: brand::FontStyle) -> Font {
    let base = if f.monospaced { Font::MONOSPACE } else { UI };
    Font { weight: weight(f.weight), ..base }
}

/// The colour of a tone (api.md §10.3): normal = secondary label, accent = cyan, …
pub fn tone(t: Tone) -> Color {
    match t {
        Tone::Normal => SECONDARY,
        Tone::Accent => ACCENT,
        Tone::Success => SUCCESS,
        Tone::Warning => WARNING,
        Tone::Danger => DANGER,
    }
}

/// The status lamp's colour (plan B99): green on, grey off, orange attention, cyan working
pub fn lamp(state: LampState) -> Color {
    match state {
        LampState::On => SUCCESS,
        LampState::Off => alpha(SECONDARY, 0.35),
        LampState::Attention => WARNING,
        LampState::Working => ACCENT,
    }
}

/// omaflake's dark theme: iced's own widgets (menus, scrollers) take these colours
pub fn theme() -> Theme {
    Theme::custom(
        "omaflake",
        iced::theme::Palette {
            background: WINDOW,
            text: TEXT,
            primary: ACCENT,
            success: SUCCESS,
            warning: WARNING,
            danger: DANGER,
        },
    )
}

// MARK: Containers

/// A list's rounded box (`ListSection`)
pub fn card(_: &Theme) -> container::Style {
    container::Style {
        background: Some(CARD.into()),
        border: Border { color: CARD_BORDER, width: 1.0, radius: radius::CARD.into() },
        ..Default::default()
    }
}

/// A row's details (`DetailBox`)
pub fn detail_box(_: &Theme) -> container::Style {
    container::Style {
        background: Some(alpha(TEXT, brand::opacity::SUBTLE_FILL).into()),
        border: border::rounded(radius::CARD),
        ..Default::default()
    }
}

/// The modal question (`ConfirmationOverlay`)
pub fn sheet(_: &Theme) -> container::Style {
    container::Style {
        background: Some(WINDOW.into()),
        text_color: Some(TEXT),
        border: Border { color: CARD_BORDER, width: 1.0, radius: radius::SHEET.into() },
        shadow: Shadow { color: alpha(Color::BLACK, 0.5), offset: Vector::new(0.0, 8.0), blur_radius: 20.0 },
        ..Default::default()
    }
}

pub fn scrim(_: &Theme) -> container::Style {
    container::Style { background: Some(SCRIM.into()), ..Default::default() }
}

/// The footer line with the newest activity (`FooterMessage`); `color` its border tint
pub fn toast(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(WINDOW.into()),
        text_color: Some(TEXT),
        border: Border { color: alpha(color, brand::opacity::NOTICE_BORDER), width: 1.0, radius: radius::CARD.into() },
        shadow: Shadow { color: alpha(Color::BLACK, 0.4), offset: Vector::new(0.0, 4.0), blur_radius: 12.0 },
        ..Default::default()
    }
}

/// The log console
pub fn console(_: &Theme) -> container::Style {
    container::Style {
        background: Some(CONSOLE.into()),
        text_color: Some(alpha(TEXT, 0.85)),
        border: Border {
            color: alpha(TEXT, brand::opacity::CONSOLE_BORDER * 0.5),
            width: 1.0,
            radius: radius::SMALL.into(),
        },
        ..Default::default()
    }
}

/// A tinted notice box (Doctor banner, outcome, warnings)
pub fn notice(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(alpha(color, brand::opacity::TINT_FILL * 0.5).into()),
        border: Border { color: alpha(color, brand::opacity::NOTICE_BORDER), width: 1.0, radius: radius::CARD.into() },
        ..Default::default()
    }
}

/// A small capsule with a count (`CountBadge`)
pub fn count_badge(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        text_color: Some(Color::BLACK),
        border: border::rounded(100.0),
        ..Default::default()
    }
}

/// A short label badge ("x86_64", "outdated")
pub fn label_badge(color: Color) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(alpha(color, brand::opacity::TINT_FILL).into()),
        text_color: Some(color),
        border: Border { color: alpha(color, brand::opacity::NOTICE_BORDER), width: 1.0, radius: radius::SMALL.into() },
        ..Default::default()
    }
}

/// A recipe's tag: a faint filled capsule
pub fn tag(_: &Theme) -> container::Style {
    container::Style {
        background: Some(alpha(TEXT, brand::opacity::HOVER).into()),
        border: border::rounded(radius::SMALL),
        ..Default::default()
    }
}

/// The status lamp: a dot with a soft glow
pub fn lamp_dot(color: Color, glow: bool) -> impl Fn(&Theme) -> container::Style {
    move |_| container::Style {
        background: Some(color.into()),
        border: border::rounded(100.0),
        shadow: if glow {
            Shadow { color: alpha(color, 0.7), offset: Vector::ZERO, blur_radius: 6.0 }
        } else {
            Shadow::default()
        },
        ..Default::default()
    }
}

pub fn divider(_: &Theme) -> container::Style {
    container::Style { background: Some(CARD_BORDER.into()), ..Default::default() }
}

// MARK: Buttons

/// Borderless icon button with a hover highlight (`IconButton`)
pub fn icon_button(color: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let disabled = status == button::Status::Disabled;
        button::Style {
            background: hover.then(|| Background::Color(alpha(TEXT, brand::opacity::HOVER))),
            text_color: if disabled { alpha(color, brand::opacity::DISABLED) } else { color },
            border: border::rounded(radius::CARD),
            ..Default::default()
        }
    }
}

/// An action in a row's details: symbol and name in a thin frame, red for removing (`ActionButton`)
pub fn action_button(color: Color, framed_in_color: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let disabled = status == button::Status::Disabled;
        let border_color = if framed_in_color { alpha(color, brand::opacity::TINTED_BORDER) } else { CARD_BORDER };
        let o = if disabled { brand::opacity::DISABLED } else { 1.0 };
        button::Style {
            background: (hover && !disabled).then(|| Background::Color(alpha(color, brand::opacity::SELECTED))),
            text_color: alpha(color, o),
            border: Border { color: alpha(border_color, o), width: 1.0, radius: radius::SMALL.into() },
            ..Default::default()
        }
    }
}

/// A prominent button (the page's main action: Create, Build): cyan, or red when it removes
pub fn primary_button(color: Color) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let disabled = status == button::Status::Disabled;
        let fill = if disabled {
            alpha(color, brand::opacity::TINT_FILL)
        } else if hover {
            alpha(color, 0.85)
        } else {
            color
        };
        button::Style {
            background: Some(fill.into()),
            text_color: if disabled { alpha(TEXT, brand::opacity::DISABLED) } else { Color::BLACK },
            border: border::rounded(radius::SMALL),
            ..Default::default()
        }
    }
}

/// A switch as a small chip: cyan while on (`FeatureChip`)
pub fn chip(on: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let disabled = status == button::Status::Disabled;
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let o = if disabled { 0.45 } else { 1.0 };
        button::Style {
            background: if on {
                Some(alpha(CYAN, brand::opacity::TOGGLE_ON_FILL).into())
            } else if hover {
                Some(alpha(TEXT, brand::opacity::HOVER).into())
            } else {
                None
            },
            text_color: alpha(if on { TEXT } else { SECONDARY }, o),
            border: Border {
                color: alpha(if on { alpha(CYAN, brand::opacity::TOGGLE_ON_BORDER) } else { CARD_BORDER }, o),
                width: 1.0,
                radius: 100.0.into(),
            },
            ..Default::default()
        }
    }
}

/// A tab of the home page: selected ones get a light fill
pub fn tab(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_, status| {
        let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
        button::Style {
            background: if selected {
                Some(alpha(TEXT, brand::opacity::SELECTED).into())
            } else if hover {
                Some(alpha(TEXT, brand::opacity::SUBTLE_FILL).into())
            } else {
                None
            },
            text_color: if selected { TEXT } else { SECONDARY },
            border: border::rounded(radius::TAB),
            ..Default::default()
        }
    }
}

/// A whole row as a button (opening it); no frame
pub fn row_button(_: &Theme, status: button::Status) -> button::Style {
    let hover = matches!(status, button::Status::Hovered | button::Status::Pressed);
    button::Style {
        background: hover.then(|| Background::Color(alpha(TEXT, brand::opacity::SUBTLE_FILL))),
        text_color: TEXT,
        border: border::rounded(radius::CARD),
        ..Default::default()
    }
}

// MARK: Inputs

pub fn input(_: &Theme, status: text_input::Status) -> text_input::Style {
    let focused = matches!(status, text_input::Status::Focused { .. });
    let disabled = status == text_input::Status::Disabled;
    text_input::Style {
        background: alpha(TEXT, if disabled { 0.02 } else { brand::opacity::SUBTLE_FILL }).into(),
        border: Border {
            color: if focused { alpha(ACCENT, brand::opacity::TINTED_BORDER) } else { CARD_BORDER },
            width: 1.0,
            radius: radius::SMALL.into(),
        },
        icon: SECONDARY,
        placeholder: tertiary(),
        value: if disabled { SECONDARY } else { TEXT },
        selection: alpha(ACCENT, 0.35),
    }
}

/// An input whose value is wrong: a red frame
pub fn input_error(t: &Theme, status: text_input::Status) -> text_input::Style {
    let mut s = input(t, status);
    s.border.color = alpha(DANGER, brand::opacity::TINTED_BORDER);
    s
}

pub fn picker(_: &Theme, status: pick_list::Status) -> pick_list::Style {
    let hover = matches!(status, pick_list::Status::Hovered | pick_list::Status::Opened { .. });
    pick_list::Style {
        text_color: TEXT,
        placeholder_color: tertiary(),
        handle_color: SECONDARY,
        background: alpha(TEXT, if hover { brand::opacity::HOVER } else { brand::opacity::SUBTLE_FILL }).into(),
        border: Border { color: CARD_BORDER, width: 1.0, radius: radius::SMALL.into() },
    }
}

pub fn switch(_: &Theme, status: toggler::Status) -> toggler::Style {
    let (on, disabled) = match status {
        toggler::Status::Active { is_toggled } | toggler::Status::Hovered { is_toggled } => (is_toggled, false),
        toggler::Status::Disabled { is_toggled } => (is_toggled, true),
    };
    let o = if disabled { brand::opacity::DISABLED } else { 1.0 };
    toggler::Style {
        background: alpha(if on { ACCENT } else { alpha(TEXT, 0.18) }, o).into(),
        background_border_width: 0.0,
        background_border_color: Color::TRANSPARENT,
        foreground: alpha(if on { Color::BLACK } else { TEXT }, o).into(),
        foreground_border_width: 0.0,
        foreground_border_color: Color::TRANSPARENT,
        text_color: Some(TEXT),
        border_radius: None,
        padding_ratio: 0.1,
    }
}

pub fn progress(color: Color) -> impl Fn(&Theme) -> progress_bar::Style {
    move |_| progress_bar::Style {
        background: alpha(TEXT, brand::opacity::SELECTED).into(),
        bar: color.into(),
        border: border::rounded(100.0),
    }
}

/// Thin overlay scrollers that only show while scrolling is possible (`OverlayScrollers`)
pub fn scroller(_: &Theme, status: scrollable::Status) -> scrollable::Style {
    let active = !matches!(status, scrollable::Status::Active { .. });
    let rail = scrollable::Rail {
        background: None,
        border: Border::default(),
        scroller: scrollable::Scroller {
            background: alpha(TEXT, if active { 0.35 } else { 0.18 }).into(),
            border: border::rounded(100.0),
        },
    };
    scrollable::Style {
        container: container::Style::default(),
        vertical_rail: rail,
        horizontal_rail: rail,
        gap: None,
        auto_scroll: scrollable::AutoScroll {
            background: WINDOW.into(),
            border: border::rounded(100.0),
            shadow: Shadow::default(),
            icon: TEXT,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colours_come_from_the_tokens() {
        assert_eq!(ACCENT.into_rgba8(), [0x00, 0xE5, 0xFF, 0xFF]);
        assert_eq!(WINDOW.into_rgba8(), [0x12, 0x12, 0x12, 0xFF]);
        assert_eq!(tone(Tone::Danger), DANGER);
        assert_eq!(lamp(LampState::On), SUCCESS);
        assert!((CARD.a - 0.045).abs() < 1e-6);
        assert_eq!(font(brand::font::WORDMARK).family, Font::MONOSPACE.family);
    }
}
