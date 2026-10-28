// SPDX-License-Identifier: AGPL-3.0-or-later
//! The window: the brand background with its red and cyan glow, the home page (header, tabs, the tab's list) or a
//! page one level deeper (back, title), the footer line and the modal question on top.

use iced::widget::{
    button, center, column, container, image, mouse_area, opaque, row, scrollable, space, stack, text, text_input,
};
use iced::{window, Alignment, Color, Length, Padding, Size};
use omaflake_core::api::views::common::LampState;
use omaflake_core::api::views::home::HomePage;
use omaflake_core::api::views::PageRef;
use serde_json::json;

use crate::app::{page_ref, App, Message, Modal, Model};
use crate::icons::icon;
use crate::pages;
use crate::style::{self, icon_size, spacing, text_size};
use crate::text::{s, show, tr};
use crate::widgets::{self, action_icon, count_badge, El};

/// The sidebar's width (`MainView.sidebarWidth`)
pub const SIDEBAR_WIDTH: f32 = 180.0;

/// The glow from the corners, like the hero mark's RGB split (`BrandStyle.background`): iced draws no radial
/// gradients, so it is an image of the window's size
pub fn glow_image(size: Size) -> image::Handle {
    let (w, h) = (size.width.max(1.0).round() as u32, size.height.max(1.0).round() as u32);
    let base = style::WINDOW;
    let red = omaflake_brand::color::GLOW_RED.rgba;
    let cyan = omaflake_brand::color::GLOW_CYAN.rgba;
    let (rr, cr) = (omaflake_brand::glow::RED_RADIUS, omaflake_brand::glow::CYAN_RADIUS);
    let mut px = Vec::with_capacity((w * h * 4) as usize);
    for y in 0..h {
        for x in 0..w {
            let (fx, fy) = (x as f32, y as f32);
            let d_red = (fx * fx + fy * fy).sqrt();
            let (dx, dy) = (w as f32 - fx, h as f32 - fy);
            let d_cyan = (dx * dx + dy * dy).sqrt();
            let a_red = red[3] * (1.0 - d_red / rr).max(0.0);
            let a_cyan = cyan[3] * (1.0 - d_cyan / cr).max(0.0);
            let mut c = [base.r, base.g, base.b];
            for i in 0..3 {
                c[i] = c[i] * (1.0 - a_red) + red[i] * a_red;
                c[i] = c[i] * (1.0 - a_cyan) + cyan[i] * a_cyan;
            }
            px.extend(c.iter().map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8));
            px.push(255);
        }
    }
    image::Handle::from_rgba(w, h, px)
}

impl App {
    pub fn view(&self, _: window::Id) -> El<'_> {
        widgets::set_help(self.home_model().and_then(|h| h.help.on).unwrap_or(false));
        let content: El<'_> = match self.current() {
            Some(page) => self.page_frame(&page),
            None => self.home(),
        };
        let background: El<'_> = match &self.glow {
            Some((_, h)) => {
                image(h.clone()).width(Length::Fill).height(Length::Fill).content_fit(iced::ContentFit::Fill).into()
            }
            None => container(space())
                .width(Length::Fill)
                .height(Length::Fill)
                .style(|_| container::Style { background: Some(style::WINDOW.into()), ..Default::default() })
                .into(),
        };
        let mut layers = stack![background, content].width(Length::Fill).height(Length::Fill);
        if let Some(t) = &self.toast {
            layers = layers.push(self.footer(&t.text, t.error));
        }
        if let Some(m) = &self.modal {
            layers = layers.push(self.modal_view(m));
        }
        container(layers).style(|_| container::Style { text_color: Some(style::TEXT), ..Default::default() }).into()
    }

    // MARK: Home

    fn home_model(&self) -> Option<&HomePage> {
        match self.models.get(&page_ref("home", json!({})).key()) {
            Some(Model::Home(h)) => Some(h),
            _ => None,
        }
    }

    fn home(&self) -> El<'_> {
        let header = self.header();
        let list: El<'_> = match self.model(&self.tab_page()) {
            Some(m) => pages::list(self, &self.tab, m),
            None => pages::waiting(self.status.as_deref()),
        };
        let mut main = column![];
        if let (Some(status), true) = (&self.status, self.core.is_some()) {
            main = main.push(container(widgets::secondary(status.clone()).color(style::WARNING)).padding(Padding {
                top: 0.0,
                right: style::spacing::MARGIN,
                bottom: spacing::L,
                left: style::spacing::MARGIN,
            }));
        }
        let main = main.push(
            scrollable(container(list).padding(Padding {
                top: spacing::S,
                right: style::spacing::MARGIN,
                bottom: 60.0,
                left: style::spacing::MARGIN,
            }))
            .height(Length::Fill)
            .style(style::scroller),
        );
        // The groups with their tabs in a sidebar, the open tab next to it (plan B147, B148), as in the Mac's window
        column![
            container(header).padding(Padding {
                top: style::spacing::MARGIN,
                right: style::spacing::MARGIN,
                bottom: spacing::XL,
                left: style::spacing::MARGIN
            }),
            row![self.sidebar(), widgets::vertical_divider(), main].height(Length::Fill),
        ]
        .into()
    }

    /// The brand glyph, the wordmark, then the header's actions (Doctor with its issues, Settings, data folder, Quit)
    fn header(&self) -> El<'_> {
        let glyph = image(image::Handle::from_bytes(
            &include_bytes!("../../../brand/png/glyph-small/omaflake-glyph-small-white-48.png")[..],
        ))
        .width(18)
        .height(18);
        let mut r = row![glyph, wordmark(), space::horizontal()].spacing(spacing::L).align_y(Alignment::Center);
        if let Some(h) = self.home_model() {
            r = r.push(self.help_switch(&h.help));
            for a in &h.actions {
                r = r.push(action_icon(a, self.pulse));
            }
        }
        r.into()
    }

    /// The help switch (plan B148): off by default; on, every page shows its explanations, each in a box of its own
    fn help_switch<'a>(&self, a: &'a omaflake_core::api::views::common::Action) -> El<'a> {
        let on = a.on.unwrap_or(false);
        let fg = if on { style::CYAN } else { style::SECONDARY };
        let label = row![
            icon("help", text_size::CALLOUT, fg),
            text(s(&a.label)).size(text_size::CALLOUT).font(if on { style::semibold() } else { style::UI }).color(fg),
        ]
        .spacing(spacing::S)
        .align_y(Alignment::Center);
        widgets::tip(
            button(container(label).height(26).align_y(Alignment::Center))
                .padding([0.0, spacing::L])
                .style(style::tab(on))
                .on_press(Message::Run(Box::new(a.clone()), None)),
            a.help.as_ref().map(s).unwrap_or_default(),
        )
    }

    /// The groups and their tabs with what needs attention (running VMs, a build, a changed recipe, new activity)
    fn sidebar(&self) -> El<'_> {
        let Some(h) = self.home_model() else {
            return container(space()).width(SIDEBAR_WIDTH).into();
        };
        let mut c = column![].spacing(spacing::XXL);
        for g in &h.groups {
            let mut items = column![text(s(&g.title).to_uppercase())
                .size(text_size::CAPTION2)
                .font(style::semibold())
                .color(style::tertiary())]
            .spacing(spacing::XS);
            for t in h.tabs.iter().filter(|t| t.group == g.id) {
                let selected = t.id == self.tab;
                let fg = if selected { style::CYAN } else { style::TEXT };
                let mut label = row![
                    container(icon(&t.icon, icon_size::TAB + 1.0, fg)).width(18).align_x(Alignment::Center),
                    text(s(&t.label))
                        .size(text_size::CALLOUT)
                        .font(if selected { style::semibold() } else { style::UI })
                        .color(fg)
                        .width(Length::Fill),
                ]
                .spacing(spacing::L)
                .align_y(Alignment::Center);
                if let Some(b) = &t.badge {
                    let badge: El<'_> = match b.kind.as_str() {
                        "count" => count_badge(b.count.unwrap_or(0), style::tone(b.tone)),
                        "working" => widgets::lamp_dot(LampState::Working, self.pulse),
                        "failed" => icon("failed", text_size::CAPTION2 + 1.0, style::RED).into(),
                        _ => dot(style::tone(b.tone)),
                    };
                    label = label.push(widgets::tip(badge, s(&b.help)));
                } else if t.id == "activity" && self.tab != "activity" && self.unseen_activity() {
                    label = label.push(widgets::tip(dot(style::CYAN), tr("New messages")));
                }
                items = items.push(
                    button(container(label).height(30).align_y(Alignment::Center))
                        .width(Length::Fill)
                        .padding([0.0, spacing::L])
                        .style(style::tab(selected))
                        .on_press(Message::Tab(t.id.clone())),
                );
            }
            c = c.push(items);
        }
        scrollable(container(c).padding(spacing::L).width(SIDEBAR_WIDTH)).height(Length::Fill).into()
    }

    pub fn unseen_activity(&self) -> bool {
        match self.models.get(&page_ref("activity", json!({})).key()) {
            Some(Model::Activity(a)) => a.entries.first().is_some_and(|e| e.id > self.seen_activity),
            _ => false,
        }
    }

    // MARK: Pages one level deeper

    fn page_frame<'a>(&'a self, page: &PageRef) -> El<'a> {
        // The model's title; none while it has not come
        let title = self.model(page).and_then(Model::title).map(s).unwrap_or_default();
        let back = widgets::icon_button("chevron-left", tr("Back"), Some(Message::Back), style::TEXT, false);
        let header = row![back, text(title).size(text_size::HEADLINE).font(style::bold())]
            .spacing(spacing::M)
            .align_y(Alignment::Center);
        let body: El<'a> = match self.model(page) {
            Some(m) => pages::page(self, page, m),
            None => pages::waiting(self.status.as_deref()),
        };
        column![
            container(header).padding(Padding {
                top: style::spacing::MARGIN - 6.0,
                right: style::spacing::MARGIN - 6.0,
                bottom: spacing::L,
                left: style::spacing::MARGIN - 6.0
            }),
            widgets::divider(),
            scrollable(container(body).padding(Padding {
                top: spacing::XXL,
                right: style::spacing::MARGIN,
                bottom: 60.0,
                left: style::spacing::MARGIN
            }))
            .height(Length::Fill)
            .style(style::scroller),
        ]
        .into()
    }

    // MARK: Overlays

    /// The newest message for a few seconds at the bottom (`FooterMessage`); a click opens the activity tab
    fn footer<'a>(&self, message: &str, error: bool) -> El<'a> {
        let color = if error { style::RED } else { style::CYAN };
        let line = row![
            icon(if error { "warning" } else { "info" }, text_size::CALLOUT, color),
            text(message.to_string()).size(text_size::CAPTION).width(Length::Fill),
        ]
        .spacing(spacing::L)
        .align_y(Alignment::Center);
        let b =
            button(container(line).padding([spacing::L, spacing::XXL]).width(Length::Fill).style(style::toast(color)))
                .padding(0)
                .style(|_, _| button::Style { text_color: style::TEXT, ..Default::default() })
                .on_press(if self.stack.is_empty() { Message::Tab("activity".into()) } else { Message::DismissToast });
        container(b)
            .width(Length::Fill)
            .height(Length::Fill)
            .align_y(Alignment::End)
            .padding(Padding {
                top: 0.0,
                right: style::spacing::MARGIN,
                bottom: spacing::XL,
                left: style::spacing::MARGIN,
            })
            .into()
    }

    /// A question over the window (`ConfirmationOverlay`): a confirmation or a value to type
    fn modal_view<'a>(&'a self, m: &'a Modal) -> El<'a> {
        let sheet: El<'a> = match m {
            Modal::Confirm { confirm, params, input } => {
                let mut c = column![
                    text(show(&confirm.title, Some(params))).size(text_size::HEADLINE).font(style::bold()),
                    text(show(&confirm.text, Some(params))).size(text_size::CALLOUT).color(style::SECONDARY),
                ]
                .spacing(spacing::XXL);
                if let Some(v) = input {
                    c = c.push(
                        text_input("", v)
                            .on_input(Message::ModalInput)
                            .on_submit(Message::ModalOk)
                            .padding(spacing::M)
                            .size(text_size::BODY)
                            .style(style::input),
                    );
                }
                let color = if confirm.destructive { style::RED } else { style::ACCENT };
                c.push(buttons(s(&confirm.button), color)).into()
            }
            Modal::Ask { ask, value, repeat, error } => {
                let secret = ask.kind == omaflake_core::api::views::common::AskKind::Password;
                let mut c =
                    column![text(s(&ask.label)).size(text_size::HEADLINE).font(style::bold())].spacing(spacing::XXL);
                if let Some(p) = &ask.prompt {
                    c = c.push(text(s(p)).size(text_size::CALLOUT).color(style::SECONDARY));
                }
                c = c.push(
                    text_input("", value)
                        .secure(secret)
                        .on_input(Message::ModalInput)
                        .on_submit(Message::ModalOk)
                        .padding(spacing::M)
                        .size(text_size::BODY)
                        .style(style::input),
                );
                if ask.repeat {
                    c = c.push(
                        text_input(&tr("Repeat"), repeat)
                            .secure(secret)
                            .on_input(Message::ModalRepeat)
                            .on_submit(Message::ModalOk)
                            .padding(spacing::M)
                            .size(text_size::BODY)
                            .style(style::input),
                    );
                }
                if let Some(e) = error {
                    c = c.push(widgets::caption(e.clone()).color(style::DANGER));
                }
                let button_text = ask.button.as_ref().map(s).unwrap_or_else(|| tr("Continue"));
                c.push(buttons(button_text, style::ACCENT)).into()
            }
        };
        let sheet = container(sheet).padding(style::spacing::MARGIN).max_width(400).style(style::sheet);
        opaque(
            mouse_area(center(opaque(sheet)).style(style::scrim).padding(style::spacing::MARGIN))
                .on_press(Message::ModalCancel),
        )
    }
}

/// Cancel and the answer
fn buttons<'a>(answer: String, color: Color) -> El<'a> {
    let cancel = button(text(tr("Cancel")).size(text_size::CALLOUT))
        .padding([spacing::S, spacing::XXL])
        .style(style::action_button(style::TEXT, false))
        .on_press(Message::ModalCancel);
    let ok = button(text(answer).size(text_size::CALLOUT).font(style::semibold()))
        .padding([spacing::S, spacing::XXL])
        .style(style::primary_button(color))
        .on_press(Message::ModalOk);
    row![space::horizontal(), cancel, ok].spacing(spacing::L).into()
}

fn dot<'a>(c: Color) -> El<'a> {
    container(space())
        .width(omaflake_brand::size::DOT)
        .height(omaflake_brand::size::DOT)
        .style(move |_| container::Style {
            background: Some(c.into()),
            border: iced::border::rounded(100.0),
            ..Default::default()
        })
        .into()
}

/// "omaflake" in the monospaced wordmark font with the hero's red/cyan offset (`Wordmark`)
pub fn wordmark<'a>() -> El<'a> {
    let f = style::font(omaflake_brand::font::WORDMARK);
    let size = omaflake_brand::font::WORDMARK.size;
    let t = |c: Color| text("omaflake").font(f).size(size).color(c);
    stack![
        container(t(style::RED)).padding(Padding { left: 0.0, ..Padding::ZERO }),
        container(t(style::CYAN)).padding(Padding { left: 2.0, ..Padding::ZERO }),
        container(t(style::TEXT)).padding(Padding { left: 1.0, ..Padding::ZERO }),
    ]
    .into()
}
