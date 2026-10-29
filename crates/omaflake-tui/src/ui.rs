// SPDX-License-Identifier: AGPL-3.0-or-later
//! Drawing: the header with the tabs and home's actions, the page's title and tools, its items (and a job's log),
//! the selected item's actions, the status line, and the modals (questions, asks, menus, the filter, help).

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use serde_json::Value;

use crate::app::{App, Modal, RESERVED, Target};
use crate::model::{self, Page, button};
use crate::text::{arr, b, s, t, tr, trf};
use crate::theme::Tone;

pub fn draw(f: &mut Frame, app: &mut App) {
    let area = f.area();
    let th = app.theme;
    f.render_widget(Block::new().style(th.base()), area);
    let [header, title, body, footer, status] = Layout::vertical([
        Constraint::Length(1),
        Constraint::Length(1),
        Constraint::Min(1),
        Constraint::Length(1),
        Constraint::Length(1),
    ])
    .areas(area);
    let page = app.page();
    draw_header(f, app, header);
    draw_title(f, app, &page, title);
    let (list_area, log_area) = match &page.log {
        Some(_) if body.height > 8 => {
            let [a, b] = Layout::vertical([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(body);
            (a, Some(b))
        }
        _ => (body, None),
    };
    draw_items(f, app, &page, list_area);
    if let (Some(log), Some(area)) = (&page.log, log_area) {
        let ctx = app.ctx();
        let lines = model::log_lines(&ctx, &app.logs, log);
        let inner = area.height.saturating_sub(2) as usize;
        let skip = lines.len().saturating_sub(inner);
        let block =
            Block::new().borders(Borders::ALL).border_style(th.border()).title(Span::styled(" Log ", th.secondary()));
        f.render_widget(
            Paragraph::new(lines.into_iter().skip(skip).collect::<Vec<_>>()).style(th.console()).block(block),
            area,
        );
    }
    draw_footer(f, app, &page, footer);
    draw_status(f, app, status);
    if app.modal.is_some() {
        draw_modal(f, app, &page, area);
    }
}

fn key_span(app: &App, k: String) -> Span<'static> {
    Span::styled(k, app.theme.key())
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let th = app.theme;
    let ctx = app.ctx();
    // home's title ("omaflake", as the model says it)
    let name = app.home.as_ref().map(|h| t(&h["title"])).filter(|n| !n.is_empty()).unwrap_or_else(|| "omaflake".into());
    let mut spans = vec![Span::styled(format!(" {name} "), th.accent().add_modifier(Modifier::BOLD))];
    match &app.home {
        Some(home) => spans.extend(model::tabs(&ctx, home, &app.tab)),
        None => spans.push(Span::styled(format!(" {}", tr("connecting …")), th.secondary())),
    }
    let home = app.home_actions();
    let letters = App::letters(&home, true, RESERVED);
    // The header's buttons: key and label (the model's short one in a narrow header); where they do not fit next to
    // the tabs, the last ones (then all) with key and symbol only
    let buttons = |labelled: usize| -> Vec<Span<'static>> {
        let mut right: Vec<Span<'static>> = Vec::new();
        for (i, (a, l)) in home.iter().zip(letters.iter()).enumerate() {
            let compact = i >= labelled;
            if let Some(l) = l {
                right.push(key_span(app, l.to_string()));
                right.push(Span::raw(" "));
            }
            if compact {
                right.extend(model::icon(&ctx, a["icon"].as_str().unwrap_or(""), th.text()));
            } else {
                right.push(Span::styled(t(a.get("shortLabel").unwrap_or(&a["label"])), th.text()));
            }
            if let Some(n) = a.get("badge").and_then(Value::as_u64) {
                right.push(Span::styled(format!(" {n}"), th.tone(Tone::Warning)));
            }
            right.push(Span::raw(if compact { " " } else { "  " }));
        }
        right
    };
    let left: usize = spans.iter().map(|s| s.width()).sum();
    f.render_widget(Paragraph::new(Line::from(spans)), area);
    let free = (area.width as usize).saturating_sub(left + 2);
    let width = |b: &[Span<'static>]| b.iter().map(|s| s.width()).sum::<usize>();
    let Some(right) = (0..=home.len()).rev().map(buttons).find(|b| width(b) <= free) else {
        return;
    };
    let w = width(&right) as u16;
    let r = Rect { x: area.x + area.width - w, width: w, ..area };
    f.render_widget(Paragraph::new(Line::from(right)), r);
}

fn draw_title(f: &mut Frame, app: &App, page: &Page, area: Rect) {
    let th = app.theme;
    let mut spans = vec![Span::raw(" ")];
    if !app.stack.is_empty() {
        spans.push(Span::styled("‹ Esc  ", th.secondary()));
    }
    spans.push(Span::styled(page.title.clone(), th.title()));
    if let Some(c) = &page.count {
        spans.push(Span::styled(format!("  {c}"), th.secondary()));
    }
    if let Some(filter) = &page.filter {
        let active = b(filter, "active");
        spans.push(Span::raw("   "));
        spans.push(key_span(app, "/".into()));
        let chosen: Vec<String> = arr(filter, "groups")
            .iter()
            .filter(|g| !g["value"].is_null() && g["value"] != Value::Bool(false))
            .map(|g| {
                arr(g, "options")
                    .iter()
                    .find(|o| o["value"] == g["value"])
                    .map(|o| t(&o["label"]))
                    .unwrap_or_else(|| t(&g["label"]))
            })
            .collect();
        let text =
            if active { format!(" {} {}", app.icons.get("filter"), chosen.join(", ")) } else { " Filter".into() };
        spans.push(Span::styled(text, if active { th.accent() } else { th.secondary() }));
    }
    let letters = App::letters(&page.tools, false, RESERVED);
    for (a, l) in page.tools.iter().zip(letters) {
        spans.push(Span::raw("   "));
        if let Some(l) = l {
            spans.push(key_span(app, l.to_string()));
            spans.push(Span::raw(" "));
        }
        spans.extend(button(&app.ctx(), a));
    }
    f.render_widget(Paragraph::new(Line::from(spans)).style(th.text()), area);
}

fn draw_items(f: &mut Frame, app: &mut App, page: &Page, area: Rect) {
    let th = app.theme;
    if page.items.is_empty() {
        let text = if app.model(&app.current()).is_none() { tr("Loading …") } else { String::new() };
        f.render_widget(Paragraph::new(Span::styled(format!("  {text}"), th.secondary())), area);
        return;
    }
    let sel = app.selected_index(page);
    let width = area.width as usize;
    let mut lines: Vec<Line<'static>> = Vec::new();
    let (mut sel_start, mut sel_end) = (0, 0);
    app.item_heights.clear();
    for (i, item) in page.items.iter().enumerate() {
        let start = lines.len();
        for l in &item.lines {
            for mut l in wrap(l, width) {
                if Some(i) == sel {
                    l.style = th.selected();
                }
                lines.push(l);
            }
        }
        if Some(i) == sel {
            (sel_start, sel_end) = (start, lines.len());
        }
        app.item_heights.push(lines.len() - start);
    }
    let height = area.height as usize;
    app.page_height = height;
    let key = format!("{:?}", app.current());
    let mut top = app.scroll.get(&key).copied().unwrap_or(0);
    if sel_end > top + height {
        top = sel_end.saturating_sub(height);
    }
    if sel_start < top {
        top = sel_start;
    }
    top = top.min(lines.len().saturating_sub(1));
    app.scroll.insert(key, top);
    f.render_widget(Paragraph::new(lines).scroll((top as u16, 0)), area);
}

fn draw_footer(f: &mut Frame, app: &App, page: &Page, area: Rect) {
    let th = app.theme;
    let ctx = app.ctx();
    let item = app.selected_index(page).and_then(|i| page.items.get(i));
    let mut spans = vec![Span::raw(" ")];
    match item {
        Some(i) if i.field.is_some() => {
            let f = i.field.as_ref().expect("checked");
            let hint = tr(match s(f, "kind") {
                "toggle" => "Enter/Space switch",
                "number" => "←/→ change  Enter type",
                "choice" => "←/→ change  Enter choose",
                "path" => "Enter choose a path (Tab completes)",
                _ => "Enter edit",
            });
            spans.push(Span::styled(hint, th.secondary()));
        }
        Some(i) => {
            for (n, a) in i.actions.iter().enumerate().take(9) {
                spans.push(key_span(app, format!("{}", n + 1)));
                spans.push(Span::raw(" "));
                let mut b = button(&ctx, a);
                if Some(n) == i.default {
                    for sp in &mut b {
                        sp.style = sp.style.add_modifier(Modifier::BOLD);
                    }
                }
                spans.extend(b);
                spans.push(Span::raw("  "));
            }
            if i.expandable {
                spans.push(Span::styled(tr(if i.open { "Space close" } else { "Space details" }), th.secondary()));
            }
        }
        None => {}
    }
    spans.push(Span::styled(format!("   {}", tr("? help")), th.secondary()));
    f.render_widget(Paragraph::new(Line::from(spans)), area);
}

fn draw_status(f: &mut Frame, app: &App, area: Rect) {
    let th = app.theme;
    let line = match &app.status {
        Some((text, tone, _)) => Line::from(vec![Span::raw(" "), Span::styled(text.clone(), th.tone(*tone))]),
        None => Line::from(Span::styled(
            format!(
                " {}",
                trf("omaf ui %@ · %@ · Tab pages · q quit", &[omaflake_core::VERSION.into(), app.via.clone().into()])
            ),
            th.secondary(),
        )),
    };
    f.render_widget(Paragraph::new(line).style(th.border().patch(Style::new().add_modifier(Modifier::empty()))), area);
}

/// A box in the middle: `w` columns, `h` rows at most
fn centered(area: Rect, w: u16, h: u16) -> Rect {
    let w = w.min(area.width.saturating_sub(2));
    let h = h.min(area.height.saturating_sub(2));
    Rect { x: area.x + (area.width - w) / 2, y: area.y + (area.height - h) / 2, width: w, height: h }
}

fn draw_modal(f: &mut Frame, app: &App, page: &Page, area: Rect) {
    let th = app.theme;
    let ctx = app.ctx();
    let Some(modal) = &app.modal else { return };
    // The lines of the selected entry (first, last) – kept in view when the modal is taller than the terminal
    let mut focus = None;
    let (title, lines): (String, Vec<Line<'static>>) = match modal {
        Modal::Help => (tr("Keys"), help_lines(app, page)),
        Modal::Menu { actions, index } => {
            let mut lines = Vec::new();
            for (n, a) in actions.iter().enumerate() {
                let first = lines.len();
                let mut spans = vec![key_span(app, format!(" {} ", n + 1))];
                spans.extend(button(&ctx, a));
                let mut line = Line::from(spans);
                if n == *index {
                    line.style = th.selected();
                }
                lines.push(line);
                let why = if b(a, "enabled") { a.get("help") } else { a.get("reason") };
                if let Some(w) = why {
                    lines.push(Line::from(Span::styled(format!("     {}", t(w)), th.secondary())));
                }
                if n == *index {
                    focus = Some((first, lines.len() - 1));
                }
            }
            (tr("Actions"), lines)
        }
        Modal::Choice { field, index } => {
            let lines = arr(field, "choices")
                .iter()
                .enumerate()
                .map(|(n, c)| {
                    let mut line = Line::from(Span::styled(format!(" {}", t(&c["label"])), th.text()));
                    if n == *index {
                        line.style = th.selected();
                    }
                    line
                })
                .collect();
            focus = Some((*index, *index));
            (t(&field["label"]), lines)
        }
        Modal::Filter { groups, values, index, .. } => {
            let mut lines = Vec::new();
            for (n, g) in groups.iter().enumerate() {
                let id = s(g, "id");
                let now = values.get(id).cloned().unwrap_or(Value::Null);
                let shown = if s(g, "kind") == "toggle" {
                    if now.as_bool().unwrap_or(false) { "[x]".to_string() } else { "[ ]".to_string() }
                } else {
                    arr(g, "options").iter().find(|o| o["value"] == now).map(|o| t(&o["label"])).unwrap_or_default()
                };
                let mut line = Line::from(vec![
                    Span::styled(format!(" {:<22}", t(&g["label"])), th.text()),
                    Span::styled(format!("‹ {shown} ›"), th.accent()),
                ]);
                if n == *index {
                    line.style = th.selected();
                }
                lines.push(line);
            }
            lines.push(Line::raw(""));
            lines.push(Line::from(Span::styled(" ←/→ change · Enter apply · r reset · Esc cancel", th.secondary())));
            (tr("Filter"), lines)
        }
        Modal::Confirm { flow, input } => {
            let (title, text, button, destructive) = App::confirm_texts(flow);
            let mut lines = vec![Line::from(Span::styled(text, th.text())), Line::raw("")];
            if let Some(i) = input {
                lines.push(Line::from(vec![Span::styled("> ", th.accent()), Span::styled(i.clone(), th.text())]));
                lines.push(Line::raw(""));
            }
            let style = if destructive { th.tone(Tone::Danger).add_modifier(Modifier::BOLD) } else { th.key() };
            let mut keys = vec![Span::styled("Enter ", th.key()), Span::styled(button, style)];
            if input.is_none() {
                keys.push(Span::styled(" (y)", th.secondary()));
            }
            keys.push(Span::styled(format!("    {}", tr("Esc Cancel")), th.secondary()));
            lines.push(Line::from(keys));
            (title, lines)
        }
        Modal::Prompt(p) => {
            let mut lines = Vec::new();
            if let Some(pr) = &p.prompt {
                lines.push(Line::from(Span::styled(pr.clone(), th.secondary())));
                lines.push(Line::raw(""));
            }
            let shown = if p.kind == "password" { "•".repeat(p.input.chars().count()) } else { p.input.clone() };
            lines.push(Line::from(vec![
                Span::styled("> ", th.accent()),
                Span::styled(shown, th.text()),
                Span::styled("▏", th.accent()),
            ]));
            if !p.candidates.is_empty() {
                let shown: Vec<String> = p.candidates.iter().take(30).cloned().collect();
                lines.push(Line::from(Span::styled(shown.join("  "), th.secondary())));
            }
            if p.checking {
                lines.push(Line::from(Span::styled(tr("Checking …"), th.accent())));
            }
            if let Some(e) = &p.error {
                lines.push(Line::from(Span::styled(e.clone(), th.tone(Tone::Danger))));
            }
            lines.push(Line::raw(""));
            let tab = if matches!(p.kind.as_str(), "folder" | "file" | "package") {
                format!("{} · ", tr("Tab complete"))
            } else {
                String::new()
            };
            let what = tr(match &p.target {
                Target::Ask(_) => "Enter OK",
                Target::Field(_) => "Enter set",
            });
            lines.push(Line::from(Span::styled(format!("{tab}{what} · {}", tr("Esc cancel")), th.secondary())));
            (p.label.clone(), lines)
        }
    };
    let width = 72u16;
    let heights: Vec<u16> = lines.iter().map(|l| (l.width() as u16 / (width - 4)) + 1).collect();
    let r = centered(area, width, heights.iter().sum::<u16>() + 2);
    let (skip, more_below) = modal_scroll(&heights, r.height.saturating_sub(2), focus);
    f.render_widget(Clear, r);
    let mut block = Block::new()
        .borders(Borders::ALL)
        .border_style(th.accent())
        .title(Span::styled(format!(" {title} "), th.title()))
        .style(th.card());
    let more = match (skip > 0, more_below) {
        (true, true) => Some(" ↑↓ "),
        (true, false) => Some(" ↑ "),
        (false, true) => Some(" ↓ "),
        (false, false) => None,
    };
    if let Some(m) = more {
        block = block.title_bottom(Line::from(Span::styled(m, th.accent())).right_aligned());
    }
    let shown: Vec<Line<'static>> = lines.into_iter().skip(skip).collect();
    f.render_widget(Paragraph::new(shown).wrap(Wrap { trim: false }).block(block), r);
}

/// The lines a modal leaves out at its top so that the `focus` lines (first, last) fit into `room` rows, and whether
/// lines are left out below. Without a focus the top is shown.
fn modal_scroll(heights: &[u16], room: u16, focus: Option<(usize, usize)>) -> (usize, bool) {
    let mut skip = 0;
    if let Some((first, last)) = focus {
        let last = last.min(heights.len().saturating_sub(1));
        while skip < first && heights[skip..=last].iter().sum::<u16>() > room {
            skip += 1;
        }
    }
    let below = heights[skip.min(heights.len())..].iter().sum::<u16>() > room;
    (skip, below)
}

/// `?`: the keys, the selected row's actions with theirs, the page's tools
fn help_lines(app: &App, page: &Page) -> Vec<Line<'static>> {
    let th = app.theme;
    let ctx = app.ctx();
    let k = |keys: &str, what: &str| {
        Line::from(vec![Span::styled(format!(" {keys:<16}"), th.key()), Span::styled(what.to_string(), th.text())])
    };
    let mut lines = vec![
        k("j/k ↑/↓", &tr("move")),
        k("Tab/Shift-Tab", &tr("next / previous tab")),
        k("Enter", &tr("the row's default action (bold) · edit a field")),
        k("Space →/←", &tr("open / close details · change a field")),
        k("1-9", &tr("the row's actions")),
        k("a", &tr("all actions of the row, with why one is off")),
        k("/", &tr("filter the list")),
        k("Esc", &tr("back")),
        k("q · Ctrl-C", &tr("quit (VMs keep running)")),
    ];
    let home = app.home_actions();
    for (a, l) in home.iter().zip(App::letters(&home, true, RESERVED)) {
        if let Some(l) = l {
            lines.push(k(&l.to_string(), &t(&a["label"])));
        }
    }
    for (a, l) in page.tools.iter().zip(App::letters(&page.tools, false, RESERVED)) {
        if let Some(l) = l {
            lines.push(k(&l.to_string(), &t(&a["label"])));
        }
    }
    if let Some(item) = app.selected_index(page).and_then(|i| page.items.get(i))
        && !item.actions.is_empty()
    {
        lines.push(Line::raw(""));
        lines.push(Line::from(Span::styled(format!(" {}", tr("This row")), th.title())));
        for (n, a) in item.actions.iter().enumerate() {
            let key = if n < 9 { format!("{}", n + 1) } else { "a".into() };
            let mut spans = vec![Span::styled(format!(" {key:<16}"), th.key())];
            spans.extend(button(&ctx, a));
            if !b(a, "enabled")
                && let Some(r) = a.get("reason")
            {
                spans.push(Span::styled(format!(" – {}", t(r)), th.secondary()));
            }
            lines.push(Line::from(spans));
        }
    }
    lines
}

/// A line longer than `width` broken into several, the next ones indented as the first (by its leading spaces)
pub fn wrap(line: &Line<'static>, width: usize) -> Vec<Line<'static>> {
    if width < 8 || line.width() <= width {
        return vec![line.clone()];
    }
    let indent = line.spans.first().map(|s| s.content.chars().take_while(|c| *c == ' ').count()).unwrap_or(0);
    let indent = (indent + 2).min(width / 2);
    let mut out: Vec<Line<'static>> = vec![];
    let mut cur: Vec<Span<'static>> = vec![];
    let mut used = 0;
    for span in &line.spans {
        let mut text = String::new();
        for c in span.content.chars() {
            let w = unicode_width(c);
            if used + w > width {
                if !text.is_empty() {
                    cur.push(Span::styled(std::mem::take(&mut text), span.style));
                }
                out.push(Line::from(std::mem::take(&mut cur)));
                cur.push(Span::raw(" ".repeat(indent)));
                used = indent;
                if c == ' ' {
                    continue;
                }
            }
            text.push(c);
            used += w;
        }
        if !text.is_empty() {
            cur.push(Span::styled(text, span.style));
        }
    }
    if !cur.is_empty() {
        out.push(Line::from(cur));
    }
    out
}

fn unicode_width(c: char) -> usize {
    Span::raw(c.to_string()).width()
}
