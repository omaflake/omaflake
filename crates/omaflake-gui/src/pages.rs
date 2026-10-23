// SPDX-License-Identifier: AGPL-3.0-or-later
//! The pages drawn from their models (api.md §10.5): the home tabs' lists (VMs, images, recipes, activity) and the
//! pages one level deeper (New VM, Build Image, Doctor, macOS base, Settings). Nothing here decides what a page
//! shows – only how.

use std::fmt;

use iced::widget::{button, column, container, pick_list, progress_bar, row, space, text, text_input, toggler, Column};
use iced::{Alignment, Color, Length};
use omaflake_core::api::views::activity::ActivityPage;
use omaflake_core::api::views::build::{BuildPage, LogRef, Outcome};
use omaflake_core::api::views::common::{Action, Blockers, Field, FieldKind, Filter, FilterKind, ListHead, Tone};
use omaflake_core::api::views::doctor::{DoctorItem, DoctorPage, Download, MacBasePage};
use omaflake_core::api::views::images::{BuildJobRow, ImageRowView, ImagesPage};
use omaflake_core::api::views::new_vm::NewVmPage;
use omaflake_core::api::views::recipes::{OfferedRow, RecipeRowView, RecipesPage};
use omaflake_core::api::views::settings::{SettingsPage, SettingsSection};
use omaflake_core::api::views::vms::{VmRowView, VmsPage};
use omaflake_core::api::views::{PageRef, Text};
use omaflake_core::prerequisites::PrerequisiteStatus;
use serde_json::{json, Value};

use crate::app::{App, Message, Model};
use crate::icons::icon;
use crate::style::{self, spacing, text_size};
use crate::text::{date_time, duration, s};
use crate::widgets::{
    self, action_button, action_icon, badge, caption, chip, detail_box, divider, expandable, hint, info_line, kind,
    lamp, primary_button, secondary, section_title, wrap, El,
};

/// While a page's model has not come yet
pub fn waiting<'a>(status: Option<&str>) -> El<'a> {
    container(
        row![
            widgets::lamp_dot(omaflake_core::api::views::common::LampState::Working, false),
            secondary(status.unwrap_or("Loading …").to_string()).size(text_size::CALLOUT),
        ]
        .spacing(spacing::L)
        .align_y(Alignment::Center),
    )
    .padding(spacing::CARD_PADDING)
    .width(Length::Fill)
    .style(style::card)
    .into()
}

fn error<'a>(message: &str) -> El<'a> {
    container(
        row![
            icon("warning", text_size::CALLOUT, style::WARNING),
            secondary(message.to_string()).size(text_size::CALLOUT)
        ]
        .spacing(spacing::L),
    )
    .padding(spacing::CARD_PADDING)
    .width(Length::Fill)
    .style(style::card)
    .into()
}

// MARK: Lists

/// The open tab's list
pub fn list<'a>(app: &'a App, tab: &str, m: &'a Model) -> El<'a> {
    match m {
        Model::Vms(p) => vms(app, p),
        Model::Images(p) => images(app, p),
        Model::Recipes(p) => recipes(app, p),
        Model::Activity(p) => activity(app, p),
        Model::Error(e) => error(e),
        _ => error(&format!("{tab}: not a list")),
    }
}

/// Title, count ("3/6" while filtered), the filter and the list's tools, then the rows in a rounded box
/// (`ListSection`)
fn section<'a>(app: &'a App, page: &str, head: &'a ListHead, rows: Vec<El<'a>>) -> El<'a> {
    let mut top =
        row![text(s(&head.title)).size(text_size::SUBHEADLINE).font(style::semibold()).color(style::SECONDARY)]
            .spacing(spacing::M)
            .align_y(Alignment::Center);
    match head.of {
        Some(of) if of != head.count => top = top.push(caption(format!("{}/{of}", head.count)).color(style::CYAN)),
        _ if head.count > 0 => top = top.push(caption(head.count.to_string()).color(style::tertiary())),
        _ => {}
    }
    if let Some(f) = &head.filter {
        let open = app.filter_menu.as_deref() == Some(page);
        let color = if f.active || open { style::CYAN } else { style::TEXT };
        top = top.push(widgets::icon_button(
            "filter",
            "Filter".into(),
            Some(Message::FilterMenu(Some(page.to_string()))),
            color,
            false,
        ));
    }
    top = top.push(space::horizontal());
    for a in &head.actions {
        top = top.push(action_icon(a, app.pulse));
    }
    let mut c = column![container(top).height(24).align_y(Alignment::Center)].spacing(spacing::M);
    if let (Some(f), true) = (&head.filter, app.filter_menu.as_deref() == Some(page)) {
        c = c.push(filter_menu(page, f));
    }
    let mut body = Column::new();
    if let Some(e) = &head.empty {
        body = body.push(hint(e, app.pulse));
    }
    for r in rows {
        body = body.push(r);
    }
    c.push(container(body.padding([spacing::XS, 0.0])).width(Length::Fill).style(style::card)).into()
}

/// The filter's choices (the macOS app's menu): one line per group, the chosen option highlighted
fn filter_menu<'a>(page: &str, f: &'a Filter) -> El<'a> {
    let mut c = column![].spacing(spacing::M);
    for g in &f.groups {
        let mut options: Vec<El<'a>> = Vec::new();
        match g.kind {
            FilterKind::Choice => {
                for o in &g.options {
                    let selected = o.value == g.value;
                    let mut label = row![].spacing(spacing::S).align_y(Alignment::Center);
                    if let Some(i) = &o.icon {
                        label = label.push(icon(i, text_size::CAPTION, style::SECONDARY));
                    }
                    label = label.push(caption(s(&o.label)));
                    options.push(
                        button(label)
                            .padding([spacing::XS, spacing::L])
                            .style(style::chip(selected))
                            .on_press(Message::SetFilter {
                                page: page.to_string(),
                                group: g.id.clone(),
                                value: o.value.clone(),
                            })
                            .into(),
                    );
                }
            }
            FilterKind::Toggle => {
                let on = g.value.as_bool().unwrap_or(false);
                options.push(
                    toggler(on)
                        .size(14)
                        .style(style::switch)
                        .on_toggle({
                            let (page, id) = (page.to_string(), g.id.clone());
                            move |v| Message::SetFilter { page: page.clone(), group: id.clone(), value: json!(v) }
                        })
                        .into(),
                );
            }
        }
        c = c.push(
            row![container(secondary(s(&g.label))).width(70), container(wrap(options)).width(Length::Fill)]
                .spacing(spacing::L)
                .align_y(Alignment::Center),
        );
    }
    if f.active {
        c = c.push(
            row![
                space::horizontal(),
                button(caption("Reset Filter"))
                    .padding([spacing::XS, spacing::L])
                    .style(style::action_button(style::TEXT, false))
                    .on_press(Message::Run(
                        Box::new(omaflake_core::api::views::common::Action::call(
                            "resetFilter",
                            Text::new("Reset Filter"),
                            None,
                            "ui.filter",
                            json!({"page": page, "filter": null}),
                        )),
                        None
                    ))
            ]
            .align_y(Alignment::Center),
        );
    }
    container(c).padding(spacing::CARD_PADDING).width(Length::Fill).style(style::card).into()
}

/// Name and badges, then the subtitle and the tags (`TagList`) of a row
fn name_line<'a>(name: String, badges: Vec<El<'a>>, subtitle: Option<String>, tags: Vec<El<'a>>) -> El<'a> {
    let mut first =
        row![text(name).size(text_size::BODY).font(style::medium())].spacing(spacing::M).align_y(Alignment::Center);
    for b in badges {
        first = first.push(b);
    }
    let mut c = column![first.wrap().vertical_spacing(spacing::XS)].spacing(spacing::XXS);
    let mut second = row![].spacing(spacing::S).align_y(Alignment::Center);
    let mut any = false;
    if let Some(sub) = subtitle.filter(|t| !t.is_empty()) {
        second = second.push(secondary(sub));
        any = true;
    }
    any |= !tags.is_empty();
    for t in tags {
        second = second.push(t);
    }
    if any {
        // Many tags wrap instead of running under the row's buttons
        c = c.push(second.wrap().vertical_spacing(spacing::XS));
    }
    c.into()
}

/// A row's or page's primary action (api.md §10.3): what a click on a build row does, drawn prominent on a page
pub fn primary_of(actions: &[Action]) -> Option<&Action> {
    actions.iter().find(|a| a.primary)
}

fn row_actions<'a>(app: &'a App, actions: &'a [Action]) -> El<'a> {
    let mut r = row![].spacing(spacing::XXS).align_y(Alignment::Center);
    for a in actions {
        r = r.push(action_icon(a, app.pulse));
    }
    r.into()
}

fn vms<'a>(app: &'a App, p: &'a VmsPage) -> El<'a> {
    let mut rows: Vec<El<'a>> = p.rows.iter().map(|r| vm_row(app, r)).collect();
    rows.extend(p.unreadable.iter().map(|h| hint(h, app.pulse)));
    section(app, "vms", &p.head, rows)
}

/// One VM (`InstanceRow`): lamp, kind, name with its feature icons, subtitle, the row's actions; its details
pub fn vm_row<'a>(app: &'a App, r: &'a VmRowView) -> El<'a> {
    let key = format!("vm:{}", r.name);
    let header = row![
        lamp(&r.lamp, app.pulse),
        kind(&r.kind),
        container(name_line(r.name.clone(), r.badges.iter().map(badge).collect(), Some(s(&r.subtitle)), vec![]))
            .width(Length::Fill),
        row_actions(app, &r.actions),
    ]
    .spacing(spacing::XL)
    .align_y(Alignment::Center);
    expandable(key.clone(), app.expanded.contains(&key), header.into(), || {
        let d = &r.detail;
        let mut c = Column::new();
        if !d.switches.is_empty() {
            c = c.push(wrap(d.switches.iter().map(chip).collect()));
        }
        if let Some(n) = &d.switches_note {
            c = c.push(secondary(s(n)));
        }
        c = c.push(Column::with_children(d.info.iter().map(info_line)).spacing(spacing::S));
        if !d.actions.is_empty() {
            c = c.push(wrap(d.actions.iter().map(|a| action_button(a, None, app.pulse)).collect()));
        }
        detail_box(c)
    })
}

fn images<'a>(app: &'a App, p: &'a ImagesPage) -> El<'a> {
    let mut rows: Vec<El<'a>> = p.jobs.iter().map(|j| job_row(app, j)).collect();
    rows.extend(p.rows.iter().map(|r| image_row(app, r)));
    rows.extend(p.unreadable.iter().map(|h| hint(h, app.pulse)));
    section(app, "images", &p.head, rows)
}

/// A build that waits, runs, failed or was cancelled (`BuildJobRow`); a click runs its primary action (`open`: the
/// build page)
fn job_row<'a>(app: &'a App, j: &'a BuildJobRow) -> El<'a> {
    use omaflake_core::api::types::JobState;
    let (state, color): (_, Color) = match j.state {
        JobState::Failed => ("failed", style::RED),
        JobState::Cancelled => ("cancelled", style::SECONDARY),
        JobState::Finished => ("finished", style::SUCCESS),
        _ => ("", style::ACCENT),
    };
    let status: El<'a> = if state.is_empty() {
        widgets::lamp_dot(omaflake_core::api::views::common::LampState::Working, app.pulse)
    } else {
        icon(state, text_size::CALLOUT, color).into()
    };
    let mut info =
        column![row![text(j.name.clone()).size(text_size::BODY).font(style::medium()), secondary(s(&j.title))]
            .spacing(spacing::M)
            .align_y(Alignment::Center)]
        .spacing(spacing::XS);
    if let Some(f) = j.fraction {
        let mut line = row![progress_bar(0.0..=1.0, f as f32).girth(4).style(style::progress(style::ACCENT))]
            .spacing(spacing::L)
            .align_y(Alignment::Center);
        if let Some(c) = &j.count {
            line = line.push(secondary(s(c)));
        }
        info = info.push(line);
    } else if let Some(c) = &j.count {
        info = info.push(secondary(s(c)));
    }
    let open = button(
        row![
            container(status).width(16).align_x(Alignment::Center),
            kind(&j.kind),
            container(info).width(Length::Fill)
        ]
        .spacing(spacing::XL)
        .align_y(Alignment::Center),
    )
    .padding(0)
    .style(|_, _| button::Style { text_color: style::TEXT, ..Default::default() })
    .on_press_maybe(
        primary_of(std::slice::from_ref(&j.open))
            .or_else(|| primary_of(&j.actions))
            .filter(|a| a.enabled)
            .map(|a| Message::Run(Box::new(a.clone()), None)),
    );
    container(row![open, row_actions(app, &j.actions)].spacing(spacing::XL).align_y(Alignment::Center))
        .padding([7.0, spacing::XL])
        .into()
}

pub fn image_row<'a>(app: &'a App, r: &'a ImageRowView) -> El<'a> {
    let key = format!("image:{}", r.name);
    let header = row![
        lamp(&r.lamp, app.pulse),
        kind(&r.kind),
        container(name_line(r.name.clone(), r.badges.iter().map(badge).collect(), Some(s(&r.subtitle)), vec![]))
            .width(Length::Fill),
        row_actions(app, &r.actions),
    ]
    .spacing(spacing::XL)
    .align_y(Alignment::Center);
    expandable(key.clone(), app.expanded.contains(&key), header.into(), || {
        let mut c = column![Column::with_children(r.detail.info.iter().map(info_line)).spacing(spacing::S)];
        if !r.detail.actions.is_empty() {
            c = c.push(wrap(r.detail.actions.iter().map(|a| action_button(a, None, app.pulse)).collect()));
        }
        detail_box(c)
    })
}

/// A recipe's tags as filled capsules: five, then "+N" (`TagList`)
fn tags<'a>(tags: &'a [String]) -> Vec<El<'a>> {
    const SHOWN: usize = 5;
    let mut out: Vec<El<'a>> = tags
        .iter()
        .take(SHOWN)
        .map(|t| {
            container(text(t.clone()).size(text_size::CAPTION2).color(style::SECONDARY))
                .padding([1.0, spacing::M])
                .style(style::tag)
                .into()
        })
        .collect();
    if tags.len() > SHOWN {
        out.push(caption(format!("+{}", tags.len() - SHOWN)).color(style::tertiary()).into());
    }
    out
}

fn recipes<'a>(app: &'a App, p: &'a RecipesPage) -> El<'a> {
    let mut rows: Vec<El<'a>> = Vec::new();
    if let (true, Some(t)) = (p.loading, &p.loading_text) {
        rows.push(
            container(
                row![
                    widgets::lamp_dot(omaflake_core::api::views::common::LampState::Working, app.pulse),
                    secondary(s(t))
                ]
                .spacing(spacing::L)
                .align_y(Alignment::Center),
            )
            .padding(spacing::CARD_PADDING)
            .into(),
        );
    }
    if let Some(u) = &p.updates {
        let mut title = row![
            icon("upgradable", text_size::CALLOUT, style::WARNING),
            text(s(&u.title)).size(text_size::CALLOUT).font(style::semibold()).width(Length::Fill),
        ]
        .spacing(spacing::L)
        .align_y(Alignment::Center);
        if let Some(a) = &u.all {
            title = title.push(action_button(a, None, app.pulse));
        }
        let mut c = column![title].spacing(spacing::M);
        for r in &u.rows {
            c = c.push(
                row![
                    kind(&r.kind),
                    text(r.name.clone()).size(text_size::CALLOUT).width(Length::Fill),
                    secondary(r.source.clone()),
                    action_button(&r.action, None, app.pulse),
                ]
                .spacing(spacing::L)
                .align_y(Alignment::Center),
            );
        }
        c = c.push(secondary(s(&u.note)));
        rows.push(container(c).padding(spacing::CARD_PADDING).into());
        rows.push(divider());
    }
    if let Some(h) = &p.suggestion {
        rows.push(hint(h, app.pulse));
    }
    rows.extend(p.rows.iter().map(|r| recipe_row(app, r)));
    if let (false, Some(t)) = (p.offered.is_empty(), &p.offered_title) {
        rows.push(container(section_title(t)).padding([spacing::M, spacing::XL]).into());
    }
    rows.extend(p.offered.iter().map(|r| offered_row(app, r)));
    rows.extend(p.unreadable.iter().map(|h| hint(h, app.pulse)));
    rows.extend(p.errors.iter().map(|h| hint(h, app.pulse)));
    if let (true, Some(h)) = (p.lab_hidden, &p.lab_hint) {
        rows.push(hint(h, app.pulse));
    }
    section(app, "recipes", &p.head, rows)
}

pub fn recipe_row<'a>(app: &'a App, r: &'a RecipeRowView) -> El<'a> {
    let key = format!("recipe:{}", r.line);
    let badges: Vec<El<'a>> = r.badges.iter().map(badge).collect();
    let header = row![
        lamp(&r.lamp, app.pulse),
        kind(&r.kind),
        container(name_line(r.name.clone(), badges, Some(r.secondary.join(" · ")), tags(&r.tags))).width(Length::Fill),
        row_actions(app, &r.actions),
    ]
    .spacing(spacing::XL)
    .align_y(Alignment::Center);
    expandable(key.clone(), app.expanded.contains(&key), header.into(), || {
        let d = &r.detail;
        let mut c = Column::new();
        if let Some(desc) = &d.description {
            c = c.push(text(desc.clone()).size(text_size::CAPTION));
        }
        c = c.push(Column::with_children(d.info.iter().map(info_line)).spacing(spacing::S));
        if !d.versions.is_empty() {
            let mut v = column![].spacing(spacing::XS);
            for ver in &d.versions {
                v = v.push(
                    row![
                        text(ver.words.clone()).size(text_size::CAPTION).font(iced::Font::MONOSPACE),
                        secondary(s(&ver.state)).width(Length::Fill),
                        row_actions(app, &ver.actions),
                    ]
                    .spacing(spacing::L)
                    .align_y(Alignment::Center),
                );
            }
            c = c.push(v);
        }
        if !d.actions.is_empty() {
            c = c.push(wrap(d.actions.iter().map(|a| action_button(a, None, app.pulse)).collect()));
        }
        detail_box(c)
    })
}

fn offered_row<'a>(app: &'a App, r: &'a OfferedRow) -> El<'a> {
    let key = format!("offered:{}", r.id);
    let badges: Vec<El<'a>> = r.badges.iter().map(badge).collect();
    let header = row![
        lamp(&r.lamp, app.pulse),
        kind(&r.kind),
        container(name_line(r.name.clone(), badges, Some(r.author.clone()), tags(&r.tags))).width(Length::Fill),
        row_actions(app, &r.actions),
    ]
    .spacing(spacing::XL)
    .align_y(Alignment::Center);
    expandable(key.clone(), app.expanded.contains(&key), header.into(), || {
        let d = &r.detail;
        let mut c = column![text(d.description.clone()).size(text_size::CAPTION)];
        c = c.push(Column::with_children(d.info.iter().map(info_line)).spacing(spacing::S));
        if let Some(w) = &d.warning {
            c = c.push(
                row![icon("warning", text_size::CAPTION, style::WARNING), caption(s(w)).color(style::WARNING)]
                    .spacing(spacing::M),
            );
        }
        detail_box(c)
    })
}

fn activity<'a>(app: &'a App, p: &'a ActivityPage) -> El<'a> {
    use omaflake_core::api::types::Level;
    let rows = p
        .entries
        .iter()
        .map(|e| {
            let color = match e.level {
                Level::Info => style::TEXT,
                Level::Warning => style::WARNING,
                Level::Error => style::RED,
            };
            container(
                row![
                    caption(crate::text::time(e.time)).font(iced::Font::MONOSPACE).color(style::tertiary()),
                    caption(s(&e.text)).color(color).width(Length::Fill),
                ]
                .spacing(spacing::L),
            )
            .padding([spacing::M, spacing::XL])
            .into()
        })
        .collect();
    section(app, "activity", &p.head, rows)
}

// MARK: Pages

/// A page one level deeper
pub fn page<'a>(app: &'a App, p: &PageRef, m: &'a Model) -> El<'a> {
    match m {
        Model::NewVm(n) => new_vm(app, &p.page, n),
        Model::Build(b) => build(app, b),
        Model::Doctor(d) => doctor(app, d),
        Model::MacBase(m) => mac_base(app, &p.page, m),
        Model::Settings(st) => settings(app, &p.page, st),
        Model::Vm(v) => detail_page(vm_row(app, &v.row)),
        Model::Image(i) => detail_page(image_row(app, &i.row)),
        Model::Error(e) => error(e),
        other => list(app, &p.page, other),
    }
}

/// A VM's or image's page: its row (opened with a click) in a card
fn detail_page(row: El<'_>) -> El<'_> {
    container(row).padding([spacing::XS, 0.0]).width(Length::Fill).style(style::card).into()
}

/// The Doctor's report against an action (`DoctorBanner`)
fn blockers<'a>(app: &'a App, b: &'a Blockers) -> El<'a> {
    let mut c = column![text(s(&b.title)).size(text_size::CALLOUT).font(style::semibold())].spacing(spacing::S);
    for i in &b.items {
        c = c.push(caption(format!("• {}: {}", s(&i.title), s(&i.hint))).color(style::SECONDARY));
    }
    c = c.push(action_button(&b.action, None, app.pulse));
    container(c).padding(spacing::CARD_PADDING).width(Length::Fill).style(style::notice(style::WARNING)).into()
}

fn page_head<'a>(kind_badge: El<'a>, name: String, badges: Vec<El<'a>>, sub: Option<String>) -> El<'a> {
    let mut first = row![kind_badge, text(name).size(text_size::HEADLINE).font(style::bold())]
        .spacing(spacing::M)
        .align_y(Alignment::Center);
    for b in badges {
        first = first.push(b);
    }
    let mut c = column![first].spacing(spacing::XS);
    if let Some(sub) = sub.filter(|s| !s.is_empty()) {
        c = c.push(secondary(sub));
    }
    c.into()
}

fn new_vm<'a>(app: &'a App, page: &str, n: &'a NewVmPage) -> El<'a> {
    let mut c = column![page_head(kind(&n.kind), s(&n.subtitle), vec![], None)].spacing(spacing::XXL);
    if let Some(d) = &n.description {
        c = c.push(text(d.clone()).size(text_size::CALLOUT).color(style::SECONDARY));
    }
    if let Some(b) = &n.blockers {
        c = c.push(blockers(app, b));
    }
    for sec in &n.sections {
        let mut f = Column::new().spacing(spacing::L);
        for field in &sec.fields {
            f = f.push(field_view(app, page, field, false));
        }
        let mut block = column![].spacing(spacing::M);
        if let Some(t) = &sec.title {
            block = block.push(section_title(t));
        }
        c = c.push(block.push(container(f).padding(spacing::CARD_PADDING).width(Length::Fill).style(style::card)));
    }
    for w in &n.warnings {
        c = c.push(
            row![icon("warning", text_size::CAPTION, style::WARNING), caption(s(w)).color(style::WARNING)]
                .spacing(spacing::M),
        );
    }
    c.push(page_actions(app, page, &n.actions)).into()
}

/// A page's actions at its end: the primary one prominent
fn page_actions<'a>(app: &'a App, page: &str, actions: &'a [Action]) -> El<'a> {
    let mut r = row![space::horizontal()].spacing(spacing::L).align_y(Alignment::Center);
    for a in actions {
        r = r.push(button_of(a, Some(page), app.pulse));
    }
    r.into()
}

/// An action as a button: the primary one filled, the others framed
fn button_of<'a>(a: &Action, form: Option<&str>, pulse: bool) -> El<'a> {
    if a.primary {
        primary_button(a, form, pulse)
    } else {
        action_button(a, form, pulse)
    }
}

fn build<'a>(app: &'a App, b: &'a BuildPage) -> El<'a> {
    let mut c = column![page_head(kind(&b.kind), b.name.clone(), b.badges.iter().map(badge).collect(), None)]
        .spacing(spacing::XXL);
    if let Some(d) = &b.description {
        c = c.push(text(d.clone()).size(text_size::CALLOUT).color(style::SECONDARY));
    }
    if let Some(bl) = &b.blockers {
        c = c.push(blockers(app, bl));
    }
    if let Some(h) = &b.mac_base {
        c = c.push(container(hint(h, app.pulse)).style(style::notice(style::WARNING)));
    }
    let mut status = row![].spacing(spacing::L).align_y(Alignment::Center);
    if let Some(st) = &b.status {
        status = status.push(text(s(st)).size(text_size::CALLOUT).color(style::SECONDARY).width(Length::Fill));
    } else {
        status = status.push(space::horizontal());
    }
    for a in &b.actions {
        status = status.push(button_of(a, None, app.pulse));
    }
    c = c.push(status);
    if let Some(p) = &b.progress {
        let now = chrono::Utc::now();
        let running = p.started.map(|t| (now - t).num_seconds()).unwrap_or(0);
        let quiet = p.last_output.map(|t| (now - t).num_seconds()).unwrap_or(0);
        let mut times =
            row![secondary(format!("{} · {}", s(&p.step), s(&p.title))).width(Length::Fill)].spacing(spacing::L);
        if let Some(count) = &p.count {
            times = times.push(secondary(s(count)));
        }
        times = times.push(secondary(duration(running)));
        let mut pc =
            column![times, progress_bar(0.0..=1.0, p.fraction as f32).girth(6).style(style::progress(style::ACCENT))]
                .spacing(spacing::M);
        if quiet >= p.quiet_warning as i64 {
            pc = pc.push(widgets::tip(
                caption(format!("No output for {}", duration(quiet))).color(style::WARNING),
                s(&p.quiet_help),
            ));
        }
        c = c.push(container(pc).padding(spacing::CARD_PADDING).width(Length::Fill).style(style::card));
    }
    if let Some(o) = &b.outcome {
        c = c.push(outcome(app, o));
    }
    if let Some(n) = &b.newest {
        c = c.push(secondary(s(n)));
    }
    if let Some(l) = &b.log {
        c = c.push(log_section(app, l));
    }
    c.into()
}

/// A job's log (build, macOS base): its note, the whole file, the newest lines
fn log_section<'a>(app: &'a App, l: &'a LogRef) -> El<'a> {
    let mut head = row![section_title(&Text::new("Log")), space::horizontal()].align_y(Alignment::Center);
    if let Some(full) = &l.full {
        head = head.push(action_button(full, None, app.pulse));
    }
    let mut lc = column![head].spacing(spacing::M);
    if let Some(n) = &l.note {
        lc = lc.push(secondary(s(n)));
    }
    lc.push(log_console(app, &l.job)).into()
}

fn outcome<'a>(app: &'a App, o: &'a Outcome) -> El<'a> {
    let color = style::tone(o.tone);
    let mut c = column![row![
        icon(&o.icon, text_size::HEADLINE, color),
        text(s(&o.title)).size(text_size::HEADLINE).font(style::bold())
    ]
    .spacing(spacing::L)
    .align_y(Alignment::Center)]
    .spacing(spacing::M);
    if let Some(d) = &o.detail {
        c = c.push(caption(s(d)).color(style::SECONDARY));
    }
    if let Some(a) = &o.action {
        c = c.push(button_of(a, None, app.pulse));
    }
    container(c).padding(spacing::CARD_PADDING).width(Length::Fill).style(style::notice(color)).into()
}

/// The newest lines of a job's log in the console
fn log_console<'a>(app: &'a App, job: &str) -> El<'a> {
    let lines = app.logs.get(job).map(|l| l.lines.as_slice()).unwrap_or_default();
    let shown = &lines[lines.len().saturating_sub(400)..];
    let body = if shown.is_empty() { "…".to_string() } else { shown.join("\n") };
    container(
        iced::widget::scrollable(
            container(
                text(body).size(text_size::CAPTION).font(iced::Font::MONOSPACE).wrapping(text::Wrapping::WordOrGlyph),
            )
            .padding(spacing::L)
            .width(Length::Fill),
        )
        .anchor_bottom()
        .height(260)
        .style(style::scroller),
    )
    .width(Length::Fill)
    .style(style::console)
    .into()
}

fn doctor<'a>(app: &'a App, d: &'a DoctorPage) -> El<'a> {
    let mut versions = column![info_line(&omaflake_core::api::views::common::InfoLine {
        label: Text::verbatim("App"),
        value: Text::verbatim(env!("CARGO_PKG_VERSION")),
        note: None,
        tone: None,
        full: None,
    })]
    .spacing(spacing::S);
    for v in &d.versions {
        versions = versions.push(info_line(v));
    }
    let u = &d.update;
    let mut update = row![
        container(secondary(s(&u.label)).align_x(Alignment::End)).width(80).align_x(Alignment::End),
        caption(s(&u.text))
            .color(if u.tone == Tone::Normal { style::TEXT } else { style::tone(u.tone) })
            .width(Length::Fill),
    ]
    .spacing(spacing::XL)
    .align_y(Alignment::Center);
    if u.busy {
        update = update.push(widgets::working(app.pulse));
    }
    for a in &u.actions {
        update = update.push(action_button(a, None, app.pulse));
    }
    let mut head = row![text(s(&d.intro)).size(text_size::CALLOUT).color(style::SECONDARY).width(Length::Fill)]
        .spacing(spacing::L)
        .align_y(Alignment::Center);
    if d.checking {
        head = head.push(widgets::working(app.pulse));
    }
    for a in &d.actions {
        head = head.push(action_button(a, None, app.pulse));
    }
    let mut c = column![
        container(column![versions, update].spacing(spacing::S))
            .padding(spacing::CARD_PADDING)
            .width(Length::Fill)
            .style(style::card),
        head,
    ]
    .spacing(spacing::XXL);
    if let Some(t) = d.checked {
        c = c.push(secondary(format!("Checked {}", date_time(t))));
    }
    for sec in &d.sections {
        let items = Column::with_children(sec.items.iter().map(|i| doctor_item(app, &sec.id, i)));
        c = c.push(
            column![
                section_title(&sec.title),
                container(items.padding([spacing::XS, 0.0])).width(Length::Fill).style(style::card)
            ]
            .spacing(spacing::M),
        );
    }
    c.into()
}

/// A check (`RequirementRow`): its status, title, and when opened what was found, the hint and the actions
fn doctor_item<'a>(app: &'a App, section: &str, i: &'a DoctorItem) -> El<'a> {
    let (glyph, color) = match i.status {
        PrerequisiteStatus::Ok => ("ok", style::SUCCESS),
        PrerequisiteStatus::Limited => ("warning", style::WARNING),
        PrerequisiteStatus::Missing => ("failed", style::RED),
    };
    let item_id = serde_json::to_value(i.item).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_default();
    let key = format!("doctor:{section}:{item_id}");
    // The model gives the first state; a click turns it over
    let open = i.expanded != app.toggled.contains(&key);
    let mut head =
        row![icon(glyph, text_size::BODY, color), text(s(&i.title)).size(text_size::BODY).width(Length::Fill),]
            .spacing(spacing::L)
            .align_y(Alignment::Center);
    if i.busy {
        head = head.push(widgets::working(app.pulse));
    }
    let detail = || {
        let mut c = Column::new();
        if let Some(d) = &i.detail {
            c = c.push(caption(s(d)).color(style::SECONDARY).wrapping(text::Wrapping::WordOrGlyph));
        }
        if let Some(h) = &i.hint {
            c = c.push(caption(s(h)).color(color));
        }
        if let Some(t) = &i.task {
            c = c.push(caption(s(t)).color(style::ACCENT));
        }
        if let Some(dl) = &i.download {
            c = c.push(download(dl));
        }
        if !i.actions.is_empty() {
            c = c.push(wrap(i.actions.iter().map(|a| action_button(a, None, app.pulse)).collect()));
        }
        detail_box(c)
    };
    if !i.expandable {
        return container(head).padding([7.0, spacing::XL]).into();
    }
    expandable(key, open, head.into(), detail)
}

fn download<'a>(d: &'a Download) -> El<'a> {
    let f = if d.of > 0 { d.done as f32 / d.of as f32 } else { 0.0 };
    column![progress_bar(0.0..=1.0, f).girth(4).style(style::progress(style::ACCENT)), secondary(s(&d.text))]
        .spacing(spacing::XS)
        .into()
}

fn mac_base<'a>(app: &'a App, page: &str, m: &'a MacBasePage) -> El<'a> {
    let mut c = column![text(s(&m.intro)).size(text_size::CALLOUT).color(style::SECONDARY)].spacing(spacing::XXL);
    if let Some(h) = &m.base {
        c = c.push(container(hint(h, app.pulse)).width(Length::Fill).style(style::card));
    }
    let mut ipsw =
        column![row![container(secondary(s(&m.ipsw.label))).width(Length::Fill), caption(s(&m.ipsw.text)),]
            .spacing(spacing::L)]
        .spacing(spacing::M);
    if let Some(d) = &m.ipsw.download {
        ipsw = ipsw.push(download(d));
    }
    if !m.ipsw.actions.is_empty() {
        ipsw = ipsw.push(wrap(m.ipsw.actions.iter().map(|a| action_button(a, Some(page), app.pulse)).collect()));
    }
    c = c.push(container(ipsw).padding(spacing::CARD_PADDING).width(Length::Fill).style(style::card));
    let mut f = Column::new().spacing(spacing::L);
    for field in &m.fields {
        f = f.push(field_view(app, page, field, false));
    }
    c = c.push(container(f).padding(spacing::CARD_PADDING).width(Length::Fill).style(style::card));
    if m.installing {
        c = c.push(row![widgets::working(app.pulse), secondary("Creating the base …")].align_y(Alignment::Center));
    }
    c = c.push(page_actions(app, page, &m.actions));
    if let Some(l) = &m.log {
        c = c.push(log_section(app, l));
    }
    c.into()
}

fn settings<'a>(app: &'a App, page: &str, st: &'a SettingsPage) -> El<'a> {
    let src = &st.sources;
    let mut rows = Column::new();
    for r in &src.rows {
        let mut line = row![
            icon(&r.icon, text_size::BODY, style::SECONDARY),
            container(name_line(r.name.clone(), r.badges.iter().map(badge).collect(), Some(s(&r.subtitle)), vec![]))
                .width(Length::Fill),
        ]
        .spacing(spacing::XL)
        .align_y(Alignment::Center);
        for a in &r.actions {
            line = line.push(match a.on {
                Some(on) => widgets::tip(
                    toggler(on).size(16).style(style::switch).on_toggle_maybe(a.enabled.then(|| {
                        let a = a.clone();
                        move |_| Message::Run(Box::new(a.clone()), None)
                    })),
                    widgets::help_of(a),
                ),
                None => action_icon(a, app.pulse),
            });
        }
        rows = rows.push(container(line).padding([7.0, spacing::XL]));
    }
    let mut add = Column::new().spacing(spacing::L);
    for field in &src.add.fields {
        add = add.push(field_view(app, page, field, false));
    }
    add = add.push(row![space::horizontal(), button_of(&src.add_action, Some(page), app.pulse)]);
    let mut c = column![column![
        section_title(&src.title),
        container(column![rows, divider(), container(add).padding(spacing::CARD_PADDING)])
            .width(Length::Fill)
            .style(style::card),
        secondary(s(&src.footer)),
    ]
    .spacing(spacing::M)]
    .spacing(spacing::XXL);
    c = c.push(settings_section(app, page, &st.general));
    c = c.push(settings_section(app, page, &st.guest));
    let df = &st.data_folder;
    let mut actions = row![text(df.location.clone()).size(text_size::CAPTION).width(Length::Fill)]
        .spacing(spacing::M)
        .align_y(Alignment::Center);
    if df.moving {
        actions = actions.push(widgets::working(app.pulse));
    }
    for a in &df.actions {
        actions = actions.push(action_button(a, None, app.pulse));
    }
    c = c.push(
        column![
            section_title(&df.title),
            container(actions).padding(spacing::CARD_PADDING).width(Length::Fill).style(style::card),
            secondary(s(&df.note)),
        ]
        .spacing(spacing::M),
    );
    // The app's own settings (not the core's)
    if cfg!(target_os = "linux") {
        c = c.push(
            column![
                section_title(&Text::verbatim("This app")),
                container(
                    row![
                        column![
                            text("Tray icon").size(text_size::BODY),
                            secondary("VMs, builds and Quit in the panel; closing the window keeps omaflake there"),
                        ]
                        .spacing(spacing::XXS)
                        .width(Length::Fill),
                        toggler(app.prefs.tray).size(16).style(style::switch).on_toggle(Message::SetTray),
                    ]
                    .align_y(Alignment::Center)
                )
                .padding(spacing::CARD_PADDING)
                .width(Length::Fill)
                .style(style::card),
            ]
            .spacing(spacing::M),
        );
    }
    c.into()
}

fn settings_section<'a>(app: &'a App, page: &str, sec: &'a SettingsSection) -> El<'a> {
    let mut f = Column::new().spacing(spacing::L);
    for field in &sec.fields {
        f = f.push(field_view(app, page, field, true));
    }
    let mut c = column![
        section_title(&sec.title),
        container(f).padding(spacing::CARD_PADDING).width(Length::Fill).style(style::card)
    ]
    .spacing(spacing::M);
    if let Some(foot) = &sec.footer {
        c = c.push(secondary(s(foot)));
    }
    c.into()
}

// MARK: Fields

/// A choice as `pick_list` shows it
#[derive(Debug, Clone, PartialEq)]
pub struct ChoiceItem {
    pub value: Value,
    pub label: String,
}

impl fmt::Display for ChoiceItem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.label)
    }
}

/// A field's value now: what was typed into the form (`values.<valueParam>`), else the model's
pub fn field_value(app: &App, page: &str, field: &Field) -> Value {
    field.value_param.as_ref().and_then(|k| app.forms.get(page)?.get(k)).cloned().unwrap_or_else(|| field.value.clone())
}

/// One form field (api.md §10.3 Field). `store`: Settings – a change is stored at once (`set`); else the value goes
/// into the page's params.
pub fn field_view<'a>(app: &'a App, page: &str, field: &'a Field, store: bool) -> El<'a> {
    let value = field_value(app, page, field);
    let page_s = page.to_string();
    // A form keeps only fields with a `valueParam` (the others are shown, not kept)
    let kept = store || field.value_param.is_some();
    let enabled = field.enabled && kept;
    let set = {
        let (page, key) = (page_s.clone(), field.value_param.clone().unwrap_or_default());
        move |v: Value| -> Message {
            if store {
                Message::Store(Box::new(field.clone()), v)
            } else {
                Message::FormSet { page: page.clone(), key: key.clone(), value: v }
            }
        }
    };
    let label = text(s(&field.label)).size(text_size::CALLOUT).color(if field.enabled {
        style::TEXT
    } else {
        style::SECONDARY
    });
    let control: El<'a> = match field.kind {
        FieldKind::Toggle => {
            let on = value.as_bool().unwrap_or(false);
            let t = toggler(on)
                .size(16)
                .style(style::switch)
                .on_toggle_maybe(enabled.then_some(move |v: bool| set(json!(v))));
            row![container(label).width(Length::Fill), t].align_y(Alignment::Center).into()
        }
        FieldKind::Text => {
            let current = if store {
                app.edits.get(&field.id).cloned().unwrap_or_else(|| value.as_str().unwrap_or_default().to_string())
            } else {
                value.as_str().unwrap_or_default().to_string()
            };
            let placeholder = field.placeholder.as_ref().map(s).unwrap_or_default();
            let mut input = text_input(&placeholder, &current)
                .padding(spacing::M)
                .size(text_size::CALLOUT)
                .style(if field.error.is_some() { style::input_error } else { style::input });
            if enabled {
                if store {
                    let id = field.id.clone();
                    let submitted = current.clone();
                    input = input.on_input(move |t| Message::Edit { field: id.clone(), text: t }).on_submit(
                        Message::Store(
                            Box::new(field.clone()),
                            if submitted.trim().is_empty() { Value::Null } else { json!(submitted) },
                        ),
                    );
                } else {
                    input = input.on_input(move |t| set(json!(t)));
                }
            }
            let mut r = row![container(label).width(150), input].spacing(spacing::L).align_y(Alignment::Center);
            if field.ask.is_some() && enabled {
                r = r.push(choose_button(page, field));
            }
            r.into()
        }
        FieldKind::Number => {
            let n = value.as_i64().unwrap_or(0);
            let step = field.step.unwrap_or(1);
            let (min, max) = (field.min.unwrap_or(i64::MIN), field.max.unwrap_or(i64::MAX));
            let minus = (enabled && n - step >= min).then(|| set(json!(n - step)));
            let plus = (enabled && n + step <= max).then(|| set(json!(n + step)));
            row![
                container(label).width(Length::Fill),
                stepper_button("−", minus),
                container(text(n.to_string()).size(text_size::CALLOUT).font(style::medium()))
                    .width(36)
                    .align_x(Alignment::Center),
                stepper_button("+", plus),
            ]
            .spacing(spacing::S)
            .align_y(Alignment::Center)
            .into()
        }
        FieldKind::Choice => {
            let items: Vec<ChoiceItem> =
                field.choices.iter().map(|c| ChoiceItem { value: c.value.clone(), label: s(&c.label) }).collect();
            let selected = items.iter().find(|c| c.value == value).cloned();
            let list = pick_list(items, selected, move |c: ChoiceItem| set(c.value))
                .text_size(text_size::CALLOUT)
                .padding([spacing::S, spacing::L])
                .style(style::picker);
            row![container(label).width(Length::Fill), list].spacing(spacing::L).align_y(Alignment::Center).into()
        }
        FieldKind::Path => {
            let shown =
                value.as_str().map(str::to_string).or_else(|| field.placeholder.as_ref().map(s)).unwrap_or_default();
            let color = if value.is_null() { style::tertiary() } else { style::TEXT };
            let mut r = row![
                container(label).width(150),
                text(shown)
                    .size(text_size::CAPTION)
                    .color(color)
                    .width(Length::Fill)
                    .wrapping(text::Wrapping::WordOrGlyph),
            ]
            .spacing(spacing::L)
            .align_y(Alignment::Center);
            if !value.is_null() && enabled {
                let reset = field.help.as_ref().map(s).unwrap_or_else(|| "Default".into());
                r = r.push(
                    button(caption(reset))
                        .padding([spacing::XS, spacing::L])
                        .style(style::action_button(style::TEXT, false))
                        .on_press(set(Value::Null)),
                );
            }
            if field.ask.is_some() && enabled {
                r = r.push(choose_button(page, field));
            }
            r.into()
        }
        FieldKind::Info => {
            let t: Option<Text> = serde_json::from_value(value.clone()).ok();
            let shown = t.as_ref().map(s).unwrap_or_else(|| value.as_str().unwrap_or_default().to_string());
            row![container(label).width(Length::Fill), caption(shown).color(style::SECONDARY)]
                .align_y(Alignment::Center)
                .into()
        }
    };
    let mut c = column![control].spacing(spacing::XXS);
    if let Some(e) = &field.error {
        c = c.push(caption(s(e)).color(style::DANGER));
    } else if !field.enabled {
        if let Some(r) = &field.reason {
            c = c.push(secondary(s(r)));
        }
    } else if let (Some(h), false) = (&field.help, field.kind == FieldKind::Path) {
        c = c.push(secondary(s(h)));
    }
    c.into()
}

fn choose_button<'a>(page: &str, field: &'a Field) -> El<'a> {
    let label = field.ask.as_ref().map(|a| s(&a.label)).unwrap_or_else(|| "Choose …".into());
    button(
        row![icon("folder", text_size::CAPTION, style::TEXT), caption(label)]
            .spacing(spacing::S)
            .align_y(Alignment::Center),
    )
    .padding([spacing::XS, spacing::L])
    .style(style::action_button(style::TEXT, false))
    .on_press(Message::FormChoose { page: page.to_string(), field: Box::new(field.clone()) })
    .into()
}

fn stepper_button<'a>(label: &'a str, on: Option<Message>) -> El<'a> {
    button(container(text(label).size(text_size::BODY)).width(22).height(20).center_x(22).center_y(20))
        .padding(0)
        .style(style::action_button(style::TEXT, false))
        .on_press_maybe(on)
        .into()
}

#[cfg(test)]
pub mod tests {
    //! Model → view: every golden page model of the core (testdata/api/views) decodes into its page and is drawn
    //! without a panic; the parts the drawing relies on are checked.

    use std::path::PathBuf;

    use serde::de::DeserializeOwned;
    use serde_json::Value;

    use super::*;
    use crate::app::{decode, page_ref};

    fn dir() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../omaflake-core/testdata/api/views")
    }

    pub fn golden_files() -> Vec<(String, Value)> {
        let mut out: Vec<(String, Value)> = std::fs::read_dir(dir())
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "json"))
            .map(|p| {
                let v = serde_json::from_slice(&std::fs::read(&p).unwrap()).unwrap();
                (p.file_name().unwrap().to_string_lossy().into_owned(), v)
            })
            .collect();
        out.sort_by(|a, b| a.0.cmp(&b.0));
        out
    }

    pub fn golden<T: DeserializeOwned>(name: &str) -> T {
        let v: Value = serde_json::from_slice(&std::fs::read(dir().join(format!("{name}.json"))).unwrap()).unwrap();
        serde_json::from_value(v).unwrap()
    }

    /// The page a golden file is a model of
    fn page_of(file: &str) -> Option<&'static str> {
        Some(match file.split('.').next()? {
            "home" => "home",
            "vms" => "vms",
            "images" => "images",
            "recipes" => "recipes",
            "newVm" => "newVm",
            "build" => "build",
            "doctor" => "doctor",
            "macBase" => "macBase",
            "settings" => "settings",
            "activity" => "activity",
            "vm" => "vm",
            "image" => "image",
            _ => return None,
        })
    }

    fn app_with(page: &str, params: Value, model: Model, stack: bool) -> App {
        let (mut app, _) = App::boot();
        let r = page_ref(page, params);
        app.models.insert(r.key(), model);
        if stack {
            app.stack.push(r);
        } else {
            app.tab = page.to_string();
        }
        app
    }

    #[test]
    fn every_golden_page_decodes_and_draws() {
        let mut drawn = 0;
        for (file, v) in golden_files() {
            let Some(page) = page_of(&file) else { continue };
            let model = decode(page, v);
            assert!(!matches!(model, Model::Error(_)), "{file}: {model:?}");
            let list = matches!(page, "vms" | "images" | "recipes" | "activity");
            let mut app = app_with(page, serde_json::json!({}), model, !list && page != "home");
            // Open every row, so the details are drawn too
            for k in ["vm:research", "vm:mac", "image:omaflake.nixos.dev@cedar-mango-lift"] {
                app.expanded.insert(k.into());
            }
            let _ = app.view(iced::window::Id::unique());
            drawn += 1;
        }
        // home.linux, settings.linux and recipes.loading among them
        assert!(drawn >= 18, "{drawn}");
    }

    #[test]
    fn titles_come_from_the_models() {
        for (file, v) in golden_files() {
            let Some(page) = page_of(&file) else { continue };
            let model = decode(page, v);
            assert!(model.title().is_some_and(|t| !s(t).is_empty()), "{file}");
        }
        let home = decode("home", golden("home"));
        assert_eq!(home.title().map(s).as_deref(), Some("omaflake"));
        let vm: omaflake_core::api::views::vms::VmPage = golden("vm");
        assert_eq!(s(&vm.title), vm.row.name);
        let (app, _) = App::boot();
        let mut app = app;
        app.models.insert(page_ref("home", serde_json::json!({})).key(), home);
        assert_eq!(app.title(iced::window::Id::unique()), "omaflake");
        // A VM's page is drawn with its row
        let mut app = app_with("vm", serde_json::json!({"name": vm.row.name}), Model::Vm(Box::new(vm)), true);
        app.expanded.insert("vm:research".into());
        let _ = app.view(iced::window::Id::unique());
    }

    #[test]
    fn the_primary_action_is_the_models() {
        let vms: VmsPage = golden("vms");
        // A row without one (a VM that is starting) has no default action
        let primaries: Vec<&str> =
            vms.rows.iter().filter_map(|r| primary_of(&r.actions)).map(|a| a.id.as_str()).collect();
        assert!(!primaries.is_empty());
        assert!(primaries.iter().all(|id| matches!(*id, "terminal" | "startWindow" | "doctor")), "{primaries:?}");
        // A build row's click is its `open`
        let images: ImagesPage = golden("images");
        let j = images.jobs.first().unwrap();
        assert!(j.open.primary && primary_of(&j.actions).is_none());
        // Build is the build page's primary; Cancel (while it runs) is not
        let b: BuildPage = golden("build.failed");
        assert_eq!(primary_of(&b.actions).map(|a| a.id.as_str()), Some("build"));
        let b: BuildPage = golden("build");
        assert!(primary_of(&b.actions).is_none());
        let n: NewVmPage = golden("newVm");
        assert_eq!(primary_of(&n.actions).map(|a| a.id.as_str()), Some("create"));
        let st: SettingsPage = golden("settings");
        assert!(st.sources.add_action.primary);
        // The home header's short labels
        let home: omaflake_core::api::views::home::HomePage = golden("home");
        let short: Vec<String> = home.actions.iter().filter_map(|a| a.short_label.as_ref().map(s)).collect();
        assert_eq!(short, ["Doctor", "Data Folder", "Quit"]);
    }

    #[test]
    fn recipes_say_what_they_load_and_hide() {
        let r: RecipesPage = golden("recipes.loading");
        assert!(r.loading && r.loading_text.is_some());
        assert!(r.lab_hidden && r.lab_hint.as_ref().and_then(|h| h.action.as_ref()).is_some());
        let r: RecipesPage = golden("recipes");
        assert_eq!(r.offered_title.is_some(), !r.offered.is_empty());
    }

    #[test]
    fn mac_base_shows_the_ipsw_once_and_its_log() {
        let m: MacBasePage = golden("macBase");
        assert!(m.fields.iter().all(|f| f.id != "ipsw" && f.value_param.is_some()));
        let model = Model::MacBase(Box::new(m.clone()));
        let log = model.log().unwrap();
        assert_eq!(log.job, m.log.as_ref().unwrap().job);
        assert!(log.from <= log.lines);
    }

    #[test]
    fn ui_field_sets_the_value_of_its_page() {
        let m: MacBasePage = golden("macBase");
        let mut app = app_with("macBase", serde_json::json!({}), Model::MacBase(Box::new(m.clone())), true);
        let mut choose = omaflake_core::api::views::common::Action::field(
            "chooseIpsw",
            Text::new("Choose IPSW …"),
            None,
            "macBase",
            serde_json::json!({}),
            "ipsw",
            "ipsw",
        );
        choose.params["value"] = serde_json::json!("/tmp/x.ipsw");
        let _ = app.update(Message::Run(Box::new(choose), None));
        assert_eq!(app.forms["macBase"]["ipsw"], serde_json::json!("/tmp/x.ipsw"));
        assert_eq!(app.current().unwrap().params["values"]["ipsw"], serde_json::json!("/tmp/x.ipsw"));
        // "Use Apple's": value null – back to the default
        let apple = m.ipsw.actions.iter().find(|a| a.id == "useApple").cloned().unwrap_or_else(|| {
            let mut a = omaflake_core::api::views::common::Action::field(
                "useApple",
                Text::new("Use Apple's"),
                None,
                "macBase",
                serde_json::json!({}),
                "ipsw",
                "ipsw",
            );
            a.params["value"] = Value::Null;
            a
        });
        assert_eq!(apple.params["page"], "macBase");
        let _ = app.update(Message::Run(Box::new(apple), None));
        assert!(!app.forms["macBase"].contains_key("ipsw"));
        // Typed values are kept by valueParam
        let user = m.fields.iter().find(|f| f.id == "user").unwrap();
        let _ = app.update(Message::FormSet {
            page: "macBase".into(),
            key: user.value_param.clone().unwrap(),
            value: serde_json::json!("guest"),
        });
        assert_eq!(field_value(&app, "macBase", user), serde_json::json!("guest"));
    }

    #[test]
    fn guides_open_their_file_or_url() {
        let d: DoctorPage = golden("doctor");
        let guide = d
            .sections
            .iter()
            .flat_map(|s| &s.items)
            .flat_map(|i| &i.actions)
            .find(|a| a.id == "guide")
            .expect("a guide in the doctor");
        let params = &guide.params["params"];
        assert!(params["title"]["text"].is_string());
        assert_eq!(crate::app::guide_target(params), "https://omaflake.dev/linux-builder/");
        let bundled = serde_json::json!({"url": "https://omaflake.dev/linux-builder/", "path": "/x/linux-builder.md"});
        assert_eq!(crate::app::guide_target(&bundled), "/x/linux-builder.md");
    }

    #[test]
    fn a_refusal_reads_its_reason_as_text() {
        let v: Value =
            serde_json::from_slice(&std::fs::read(dir().join("../packages.inspect.invalid.json")).unwrap()).unwrap();
        let answer = v.get("result").cloned().unwrap_or(v);
        let why = crate::app::refusal("/x", &answer);
        assert!(why.starts_with("/x: ") && !why.ends_with("not usable"), "{why}");
        let import: omaflake_core::api::views::import::ImportPage = golden("import");
        assert!(!s(&import.title).is_empty());
    }

    #[test]
    fn activity_marks_are_forgotten_with_a_new_server() {
        let a: Value = golden("activity");
        let newest = a["entries"][0]["id"].as_u64().unwrap();
        let (mut app, _) = App::boot();
        app.seen_activity = newest + 100;
        app.notified = Some(newest + 100);
        let _ = app.update(Message::Core(crate::backend::Event::Notification(
            "view.changed".into(),
            serde_json::json!({"page": "activity", "params": {}, "model": a}),
        )));
        assert_eq!(app.seen_activity, 0);
        assert!(app.unseen_activity());
        assert_eq!(app.notified, Some(newest));
    }

    #[test]
    fn the_view_changed_event_carries_a_page() {
        let v: Value = golden("event.view.changed");
        let page = v["page"].as_str().unwrap();
        assert!(!matches!(decode(page, v["model"].clone()), Model::Error(_)));
        assert!(
            matches!(decode("vms", serde_json::json!({"error": {"message": "gone"}})), Model::Error(m) if m == "gone")
        );
        assert!(matches!(decode("vms", serde_json::json!({"rows": 3})), Model::Error(_)));
    }

    #[test]
    fn rows_map_their_model() {
        let vms: VmsPage = golden("vms");
        let running =
            vms.rows.iter().find(|r| r.lamp.state == omaflake_core::api::views::common::LampState::On).unwrap();
        // A running VM's row offers terminal and stop; its details hold switches and greyed-out removals
        let ids: Vec<&str> = running.actions.iter().map(|a| a.id.as_str()).collect();
        assert!(ids.contains(&"terminal") && ids.contains(&"stop"), "{ids:?}");
        assert!(running.detail.switches.iter().all(|a| a.on.is_some()));
        assert!(running.detail.actions.iter().any(|a| !a.enabled && a.reason.is_some()));
        assert!(crate::icons::glyph(running.kind.icon.as_str()).is_some());
        let filtered: VmsPage = golden("vms.filtered");
        assert!(filtered.head.filter.as_ref().unwrap().active);
    }

    #[test]
    fn form_values_go_into_the_page_action() {
        let n: NewVmPage = golden("newVm");
        let mut app = app_with("newVm", serde_json::json!({"image": n.image}), Model::NewVm(Box::new(n.clone())), true);
        let cpus = n.sections.iter().flat_map(|s| &s.fields).find(|f| f.id == "cpus").unwrap().clone();
        assert_eq!(field_value(&app, "newVm", &cpus), serde_json::json!(4));
        app.forms.entry("newVm".into()).or_default().insert("cpus".into(), serde_json::json!(6));
        assert_eq!(field_value(&app, "newVm", &cpus), serde_json::json!(6));
        // The page is asked again with the values
        let shown = app.current().unwrap();
        assert_eq!(shown.params["values"]["cpus"], serde_json::json!(6));
    }

    #[test]
    fn choices_show_their_labels() {
        let n: NewVmPage = golden("newVm");
        let network = n.sections.iter().flat_map(|s| &s.fields).find(|f| f.kind == FieldKind::Choice).unwrap();
        let items: Vec<ChoiceItem> =
            network.choices.iter().map(|c| ChoiceItem { value: c.value.clone(), label: s(&c.label) }).collect();
        assert!(items.iter().any(|i| i.value == network.value));
        assert!(items.iter().all(|i| !i.to_string().is_empty()));
    }

    /// The app ticks (and redraws) only while a page shows something working (plan B123)
    #[test]
    fn only_working_pages_tick() {
        let v = |n: &str| -> Value { golden(n) };
        assert!(crate::app::pulses(&v("build")));
        assert!(crate::app::pulses(&serde_json::json!({"rows": [{"lamp": {"state": "working"}}]})));
        assert!(crate::app::pulses(&serde_json::json!({"actions": [{"id": "stop", "busy": true}]})));
        assert!(crate::app::pulses(&serde_json::json!({"loading": true})));
        assert!(!crate::app::pulses(&serde_json::json!({"rows": [{"lamp": {"state": "on"}}], "loading": false})));
        assert!(!crate::app::pulses(&v("settings")));
        let (mut app, _) = App::boot();
        app.set_model("a".into(), "build", v("build"));
        assert!(app.animated.contains("a"));
        app.set_model("a".into(), "settings", v("settings"));
        assert!(app.animated.is_empty());
    }
}
