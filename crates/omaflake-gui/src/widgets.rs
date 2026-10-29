// SPDX-License-Identifier: AGPL-3.0-or-later
//! The building blocks of api.md §10.3 drawn in omaflake's look – the iced counterparts of the macOS app's
//! `IconButton`, `ActionButton`, `FeatureChip`, `StatusLamp`, `DetailBox`, `InfoLine`, `ExpandableRow`,
//! `ListSection`, `EmptyHint` and `CountBadge`.

use iced::widget::{Column, button, column, container, row, space, text, tooltip};
use iced::{Alignment, Color, Element, Length, Padding};
use omaflake_core::api::views::Text;
use omaflake_core::api::views::common::{Action, Badge, Hint, InfoLine, Lamp, LampState, Tone};

use crate::app::Message;
use crate::icons::icon;
use crate::style::{self, icon_size, size, spacing, text_size};
use crate::text::s;

pub type El<'a> = Element<'a, Message>;

/// The help switch (plan B148), as the home page says: explanations are drawn only while it is on. Set before each
/// view is drawn (`App::view`), read by every page.
static HELP: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn set_help(on: bool) {
    HELP.store(on, std::sync::atomic::Ordering::Relaxed);
}

pub fn help_on() -> bool {
    HELP.load(std::sync::atomic::Ordering::Relaxed)
}

/// An explanation in a box of its own (`HelpNote`, plan B148): a page's intro or footer, a field's help, a hint
/// marked help; nothing while the help switch is off
pub fn help_note<'a>(t: String) -> El<'a> {
    help_box(t, None)
}

fn help_box<'a>(t: String, action: Option<&Action>) -> El<'a> {
    if !help_on() || t.is_empty() {
        return column![].into();
    }
    let mut r = row![
        icon("help", text_size::CALLOUT, style::CYAN),
        text(t)
            .size(text_size::CAPTION)
            .color(style::SECONDARY)
            .width(Length::Fill)
            .wrapping(text::Wrapping::WordOrGlyph),
    ]
    .spacing(spacing::L)
    .align_y(Alignment::Center);
    if let Some(a) = action {
        r = r.push(action_button(a, None, false));
    }
    container(r).padding([spacing::L, spacing::XL]).width(Length::Fill).style(style::detail_box).into()
}

/// One key as caps (plan B148): each cap a box of its own – |Ctrl| |g| – chords a little apart, never "+"
pub fn key_caps<'a>(chords: &[Vec<String>]) -> El<'a> {
    let mut r = row![].spacing(spacing::M).align_y(Alignment::Center);
    for chord in chords {
        let mut c = row![].spacing(spacing::XS);
        for cap in chord {
            c = c.push(
                container(text(cap.clone()).size(text_size::CAPTION).font(style::semibold()))
                    .padding([1.0, spacing::S])
                    .style(style::card),
            );
        }
        r = r.push(c);
    }
    r.into()
}

/// Several keys for the same: |h| or |←|
pub fn key_alternatives<'a>(keys: &[Vec<Vec<String>>]) -> El<'a> {
    let mut r = row![].spacing(spacing::S).align_y(Alignment::Center);
    for (i, k) in keys.iter().enumerate() {
        if i > 0 {
            r = r.push(text(crate::text::tr("or")).size(text_size::CAPTION).color(style::tertiary()));
        }
        r = r.push(key_caps(k));
    }
    r.into()
}

/// A tooltip in the brand's look
pub fn tip<'a>(content: impl Into<El<'a>>, help: String) -> El<'a> {
    if help.is_empty() {
        return content.into();
    }
    tooltip(
        content,
        container(text(help).size(text_size::CAPTION).color(style::TEXT))
            .padding([spacing::S, spacing::L])
            .max_width(320)
            .style(style::sheet),
        tooltip::Position::Bottom,
    )
    .gap(4)
    .into()
}

pub fn caption<'a>(t: impl text::IntoFragment<'a>) -> iced::widget::Text<'a> {
    text(t).size(text_size::CAPTION)
}

pub fn secondary<'a>(t: impl text::IntoFragment<'a>) -> iced::widget::Text<'a> {
    text(t).size(text_size::CAPTION).color(style::SECONDARY)
}

/// A thin line between rows
pub fn divider<'a>() -> El<'a> {
    container(space()).width(Length::Fill).height(1).style(style::divider).into()
}

/// A thin line between the sidebar and the tab
pub fn vertical_divider<'a>() -> El<'a> {
    container(space()).width(1).height(Length::Fill).style(style::divider).into()
}

/// What a button's tooltip says: why it is greyed out, its help, else its label
pub fn help_of(a: &Action) -> String {
    if !a.enabled
        && let Some(r) = &a.reason
    {
        return s(r);
    }
    a.help.as_ref().map(s).unwrap_or_else(|| s(&a.label))
}

/// The message that runs an action (none while it is off)
pub fn message_of(a: &Action, form: Option<&str>) -> Option<Message> {
    a.enabled.then(|| Message::Run(Box::new(a.clone()), form.map(str::to_string)))
}

/// A pulsing dot in place of a spinner (iced has none): something is under way
pub fn working<'a>(pulse: bool) -> El<'a> {
    container(lamp_dot(LampState::Working, pulse))
        .width(size::ICON_BUTTON_WIDTH)
        .height(size::ICON_BUTTON_HEIGHT)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center)
        .into()
}

/// Small borderless icon button with a hover highlight and an optional warning badge (`IconButton`)
pub fn icon_button<'a>(name: &str, help: String, on: Option<Message>, color: Color, warn: bool) -> El<'a> {
    let glyph = container(icon(name, icon_size::TOOLBAR, color).font(crate::icons::FONT))
        .width(size::ICON_BUTTON_WIDTH)
        .height(size::ICON_BUTTON_HEIGHT)
        .align_x(Alignment::Center)
        .align_y(Alignment::Center);
    let content: El<'a> = if warn {
        iced::widget::stack![
            glyph,
            container(icon("warning", icon_size::SUB_BADGE + 1.0, style::WARNING))
                .width(size::ICON_BUTTON_WIDTH)
                .align_x(Alignment::End)
                .padding(Padding { top: 0.0, right: 1.0, bottom: 0.0, left: 0.0 })
        ]
        .into()
    } else {
        glyph.into()
    };
    let b = button(content).padding(0).style(style::icon_button(color)).on_press_maybe(on);
    tip(b, help)
}

/// An action as an icon button (row actions, a list's tools, the header): red when destructive; with a play
/// sub-icon for starts as the macOS app draws them
pub fn action_icon<'a>(a: &Action, pulse: bool) -> El<'a> {
    if a.busy {
        return tip(working(pulse), help_of(a));
    }
    let color = if a.destructive { style::RED } else { style::TEXT };
    let name = a.icon.clone().unwrap_or_default();
    let warn = a.badge.is_some_and(|n| n > 0) || name == "warning";
    let name = if name == "warning" { "doctor".to_string() } else { name };
    // Starts and builds: the thing with a small cyan sign at its bottom right (the macOS app's `subIcon`)
    let composite = match name.as_str() {
        "start-window" => Some(("vm", "play")),
        "start-headless" => Some(("terminal", "play")),
        "build" => Some(("image", "build")),
        _ => None,
    };
    let glyph = composite.map(|c| c.0).unwrap_or(name.as_str());
    if let Some((_, sub)) = composite {
        let mut content = iced::widget::stack![
            container(icon(glyph, icon_size::TOOLBAR, color))
                .width(size::ICON_BUTTON_WIDTH)
                .height(size::ICON_BUTTON_HEIGHT)
                .align_x(Alignment::Center)
                .align_y(Alignment::Center),
            container(container(icon(sub, icon_size::SUB_ICON + 2.0, style::CYAN)).padding(1.5).style(
                |_: &iced::Theme| container::Style {
                    background: Some(style::WINDOW.into()),
                    border: iced::border::rounded(100.0),
                    ..Default::default()
                }
            ))
            .width(size::ICON_BUTTON_WIDTH)
            .height(size::ICON_BUTTON_HEIGHT)
            .align_x(Alignment::End)
            .align_y(Alignment::End)
            .padding(Padding { top: 0.0, right: 1.0, bottom: 1.0, left: 0.0 }),
        ];
        // The Doctor's sign at the top right, as `icon_button` draws it (a build the Doctor blocks)
        if warn {
            content = content.push(
                container(icon("warning", icon_size::SUB_BADGE + 1.0, style::WARNING))
                    .width(size::ICON_BUTTON_WIDTH)
                    .align_x(Alignment::End)
                    .padding(Padding { top: 0.0, right: 1.0, bottom: 0.0, left: 0.0 }),
            );
        }
        let b = button(content).padding(0).style(style::icon_button(color)).on_press_maybe(message_of(a, None));
        return tip(b, help_of(a));
    }
    icon_button(glyph, help_of(a), message_of(a, None), color, warn)
}

/// An action with symbol and name in a thin frame (`ActionButton`); `form` for a page's own action
pub fn action_button<'a>(a: &Action, form: Option<&str>, pulse: bool) -> El<'a> {
    let color = if a.destructive { style::RED } else { style::TEXT };
    let mut r = row![].spacing(spacing::S).align_y(Alignment::Center);
    if a.busy {
        r = r.push(lamp_dot(LampState::Working, pulse));
    } else if let Some(name) = &a.icon {
        r = r.push(icon(name, icon_size::BUTTON, color));
    }
    r = r.push(caption(s(&a.label)));
    let b = button(container(r).height(22).align_y(Alignment::Center))
        .padding([0.0, spacing::L])
        .style(style::action_button(color, a.destructive))
        .on_press_maybe(message_of(a, form));
    tip(b, if a.enabled { a.help.as_ref().map(s).unwrap_or_default() } else { help_of(a) })
}

/// The page's main action (Create, Build, Check and Add): filled in cyan, red when it removes
pub fn primary_button<'a>(a: &Action, form: Option<&str>, pulse: bool) -> El<'a> {
    let color = if a.destructive { style::RED } else { style::ACCENT };
    let fg = if a.enabled { Color::BLACK } else { style::SECONDARY };
    let mut r = row![].spacing(spacing::M).align_y(Alignment::Center);
    if a.busy {
        r = r.push(lamp_dot(LampState::Working, pulse));
    } else if let Some(name) = &a.icon {
        r = r.push(icon(name, icon_size::BUTTON + 1.0, fg));
    }
    r = r.push(text(s(&a.label)).size(text_size::CALLOUT).font(style::semibold()));
    let b = button(container(r).height(26).align_y(Alignment::Center))
        .padding([0.0, spacing::XXL])
        .style(style::primary_button(color))
        .on_press_maybe(message_of(a, form));
    tip(b, if a.enabled { a.help.as_ref().map(s).unwrap_or_default() } else { help_of(a) })
}

/// A switch action as a chip: cyan while on (`FeatureChip`)
pub fn chip<'a>(a: &Action) -> El<'a> {
    let on = a.on.unwrap_or(false);
    let color = if on { style::TEXT } else { style::SECONDARY };
    let mut r = row![].spacing(spacing::S).align_y(Alignment::Center);
    if let Some(name) = &a.icon {
        r = r.push(icon(name, icon_size::BUTTON, color));
    }
    r = r.push(caption(s(&a.label)));
    let b = button(container(r).height(22).align_y(Alignment::Center))
        .padding([0.0, spacing::L])
        .style(style::chip(on))
        .on_press_maybe(message_of(a, None));
    tip(b, help_of(a))
}

/// A dot in a lamp's colour; `working` pulses with the tick
pub fn lamp_dot<'a>(state: LampState, pulse: bool) -> El<'a> {
    let mut c = style::lamp(state);
    if state == LampState::Working && pulse {
        c = style::alpha(c, 0.35);
    }
    container(space()).width(8).height(8).style(style::lamp_dot(c, state != LampState::Off)).into()
}

/// The status lamp that opens a row (plan B99)
pub fn lamp<'a>(l: &Lamp, pulse: bool) -> El<'a> {
    tip(
        container(lamp_dot(l.state, pulse)).width(16).height(18).align_x(Alignment::Center).align_y(Alignment::Center),
        s(&l.help),
    )
}

/// A small symbol next to a name, or its short label in a frame
pub fn badge<'a>(b: &Badge) -> El<'a> {
    let color = style::tone(b.tone);
    let content: El<'a> = match &b.label {
        Some(label) => container(text(s(label)).size(icon_size::BADGE + 0.5).font(style::semibold()))
            .padding([0.0, spacing::S])
            .style(style::label_badge(color))
            .into(),
        None => icon(&b.icon, icon_size::BADGE + 1.0, color).into(),
    };
    tip(content, s(&b.help))
}

/// A guest kind's symbol (snowflake, apple) in a fixed column
pub fn kind<'a>(b: &Badge) -> El<'a> {
    tip(container(icon(&b.icon, 13.0, style::SECONDARY)).width(16).align_x(Alignment::Center), s(&b.help))
}

/// "Label  value" with a fixed label column; the note after the value in its tone (`InfoLine`)
pub fn info_line<'a>(l: &InfoLine) -> El<'a> {
    let mut value =
        row![text(s(&l.value)).size(text_size::CAPTION).wrapping(text::Wrapping::WordOrGlyph)].spacing(spacing::M);
    let tone = l.tone.map(style::tone).unwrap_or(style::SECONDARY);
    if let Some(n) = &l.note {
        value = value.push(text(s(n)).size(text_size::CAPTION).color(tone));
    }
    let line = row![
        container(secondary(s(&l.label)).align_x(Alignment::End)).width(80).align_x(Alignment::End),
        container(value).width(Length::Fill),
    ]
    .spacing(spacing::XL);
    match &l.full {
        Some(full) => tip(line, full.clone()),
        None => line.into(),
    }
}

/// A hint line: an empty list, a filter that leaves nothing, an unreadable file (`EmptyHint`)
pub fn hint<'a>(h: &Hint, pulse: bool) -> El<'a> {
    if h.help {
        return help_box(s(&h.text), h.action.as_ref());
    }
    let tone = if h.tone == Tone::Normal { style::tertiary() } else { style::tone(h.tone) };
    let mut r = row![
        icon(&h.icon, text_size::CALLOUT, tone),
        text(s(&h.text))
            .size(text_size::CALLOUT)
            .color(style::SECONDARY)
            .width(Length::Fill)
            .wrapping(text::Wrapping::WordOrGlyph),
    ]
    .spacing(spacing::L)
    .align_y(Alignment::Center);
    if !h.keys.is_empty() {
        r = r.push(key_alternatives(&h.keys));
    }
    if let Some(a) = &h.action {
        // A hint's primary action (the suggestion's Install) is the prominent one
        r = r.push(if a.primary { primary_button(a, None, pulse) } else { action_button(a, None, pulse) });
    }
    container(r).padding(spacing::CARD_PADDING).width(Length::Fill).into()
}

/// The box that holds a row's details (`DetailBox`)
pub fn detail_box<'a>(content: Column<'a, Message>) -> El<'a> {
    container(content.spacing(spacing::L))
        .padding(spacing::CARD_PADDING)
        .width(Length::Fill)
        .style(style::detail_box)
        .into()
}

/// A row that opens its details with a click (`ExpandableRow`)
pub fn expandable<'a>(key: String, open: bool, header: El<'a>, detail: impl FnOnce() -> El<'a>) -> El<'a> {
    let chevron = icon(if open { "chevron-down" } else { "chevron-right" }, 11.0, style::SECONDARY);
    let head = button(
        row![container(header).width(Length::Fill), container(chevron).width(14).align_x(Alignment::Center)]
            .spacing(spacing::XL)
            .align_y(Alignment::Center),
    )
    .padding([7.0, spacing::XL])
    .width(Length::Fill)
    .style(style::row_button)
    .on_press(Message::Expand(key));
    let mut c = column![head];
    if open {
        c = c.push(container(detail()).padding(Padding {
            top: 0.0,
            right: spacing::XL,
            bottom: spacing::XL,
            left: 36.0,
        }));
    }
    c.into()
}

/// A tab's count in a capsule (`CountBadge`)
pub fn count_badge<'a>(n: usize, color: Color) -> El<'a> {
    container(text(n.to_string()).size(text_size::CAPTION2).font(style::bold()).color(Color::BLACK))
        .padding([0.0, 5.0])
        .style(style::count_badge(color))
        .into()
}

/// A section title ("General", "Recipe sources")
pub fn section_title<'a>(t: &Text) -> El<'a> {
    text(s(t)).size(text_size::SUBHEADLINE).font(style::semibold()).color(style::SECONDARY).into()
}

/// A row of buttons that wraps onto further lines
pub fn wrap<'a>(items: Vec<El<'a>>) -> El<'a> {
    iced::widget::Row::with_children(items).spacing(spacing::M).wrap().vertical_spacing(spacing::M).into()
}
