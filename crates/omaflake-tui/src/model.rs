// SPDX-License-Identifier: AGPL-3.0-or-later
//! From a page model (docs/api.md §10, `view.changed`) to what the terminal shows: a list of items – headings, rows,
//! form fields, hints – each with the actions it offers. The building blocks (texts, actions, lamps, badges, info
//! lines, hints, fields, list heads) are drawn the same on every page; a page only says in which order its parts
//! come. Parts a page has that this module does not know are left out (OPEN: a generic fallback once the model
//! grows parts without a TUI release).

use std::collections::{HashMap, HashSet};

use chrono::{DateTime, Utc};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use serde_json::Value;

use crate::icons::IconSet;
use crate::text::{arr, b, bytes_text, date_text, duration_text, parse_date, s, t, tr, trf};
use crate::theme::{Theme, Tone};

/// What the drawing needs besides the model
pub struct Ctx<'a> {
    pub theme: &'a Theme,
    pub icons: IconSet,
    /// Rows the user opened (their keys)
    pub expanded: &'a HashSet<String>,
    /// Rows the user closed that the model opens by itself (the Doctor's items with a problem)
    pub collapsed: &'a HashSet<String>,
    pub now: DateTime<Utc>,
    /// Counts up a few times a second: spinners
    pub tick: u64,
}

/// One entry of the page: what it shows and what can be done with it
#[derive(Debug, Clone, Default)]
pub struct Item {
    /// Stable across model updates (selection and expansion follow it)
    pub key: String,
    pub lines: Vec<Line<'static>>,
    /// Every action it offers, in the model's order (row actions, switches, detail actions)
    pub actions: Vec<Value>,
    /// The one Enter does: the model's `primary` (none: Enter opens the details)
    pub default: Option<usize>,
    /// A form field (Enter edits it)
    pub field: Option<Value>,
    pub selectable: bool,
    /// It has details to open (Space, →)
    pub expandable: bool,
    /// Its details are open now
    pub open: bool,
}

/// A page as the TUI draws it
#[derive(Debug, Clone, Default)]
pub struct Page {
    pub title: String,
    /// "1/2" while filtered
    pub count: Option<String>,
    /// The list's filter (`params.filter`)
    pub filter: Option<Value>,
    /// The page's tools (list head actions, a page's own actions) – each gets a letter
    pub tools: Vec<Value>,
    pub items: Vec<Item>,
    /// A job whose log fills the rest of the page (build, macOS base)
    pub log: Option<LogBox>,
}

/// A job's log (the model's LogRef `{job, from, lines, note?, file?, full?}`)
#[derive(Debug, Clone, Default)]
pub struct LogBox {
    pub job: String,
    /// The first line the core keeps (`jobs.log {id: job, from}`)
    pub from: u64,
    pub note: Option<String>,
}

impl LogBox {
    /// From a page's `log`; None when it is not a LogRef with a job
    pub fn from_model(log: &Value) -> Option<LogBox> {
        let job = s(log, "job");
        (!job.is_empty()).then(|| LogBox {
            job: job.to_string(),
            from: log.get("from").and_then(Value::as_u64).unwrap_or(0),
            note: log.get("note").map(t),
        })
    }
}

impl Item {
    fn line(key: impl Into<String>, line: Line<'static>) -> Item {
        Item { key: key.into(), lines: vec![line], ..Item::default() }
    }

    fn blank() -> Item {
        Item::line("", Line::raw(""))
    }

    /// A row with actions: Enter does the primary one
    fn row(key: String, lines: Vec<Line<'static>>, actions: Vec<Value>) -> Item {
        let default = primary(&actions);
        Item { key, lines, actions, default, selectable: true, ..Item::default() }
    }
}

/// The action Enter does: the one the model marks `primary` (api.md §10.3) – a disabled one too, so Enter says why
/// it cannot run. None: the row has no default (Enter opens its details).
pub fn primary(actions: &[Value]) -> Option<usize> {
    actions.iter().position(|a| b(a, "primary"))
}

// MARK: - Building blocks

fn tone_of(v: &Value) -> Tone {
    Tone::parse(v.get("tone").and_then(Value::as_str))
}

fn spinner(tick: u64) -> &'static str {
    ["◐", "◓", "◑", "◒"][(tick % 4) as usize]
}

/// The status lamp (plan B99)
fn lamp(ctx: &Ctx, lamp: &Value) -> Span<'static> {
    let th = ctx.theme;
    match s(lamp, "state") {
        "on" => Span::styled("●", th.tone(Tone::Success)),
        "attention" => Span::styled("●", th.tone(Tone::Warning)),
        "working" => Span::styled(spinner(ctx.tick), th.accent()),
        _ => Span::styled("○", th.secondary()),
    }
}

pub(crate) fn icon(ctx: &Ctx, name: &str, style: Style) -> Vec<Span<'static>> {
    let g = ctx.icons.get(name);
    if g.is_empty() {
        vec![]
    } else {
        vec![Span::styled(g.to_string(), style), Span::raw(" ")]
    }
}

/// A badge: its label where it has one (with its icon), else its icon – in its tone
fn badge(ctx: &Ctx, badge: &Value) -> Vec<Span<'static>> {
    let style = ctx.theme.tone(match (s(badge, "icon"), tone_of(badge)) {
        ("lab", Tone::Normal) => Tone::Lab,
        ("official", Tone::Normal) => Tone::Accent,
        (_, tone) => tone,
    });
    let g = ctx.icons.get(s(badge, "icon"));
    let label = badge.get("label").map(t).unwrap_or_default();
    let text = match (g.is_empty(), label.is_empty()) {
        (true, false) => label,
        (false, false) => format!("{g} {label}"),
        (false, true) => g.to_string(),
        (true, true) => t(&badge["help"]),
    };
    vec![Span::raw(" "), Span::styled(text, style)]
}

/// An action as a button: `icon label`, greyed out, red, a switch's state, a spinner or a count
pub fn button(ctx: &Ctx, a: &Value) -> Vec<Span<'static>> {
    let th = ctx.theme;
    let style = if !b(a, "enabled") {
        th.disabled()
    } else if b(a, "destructive") {
        th.tone(Tone::Danger)
    } else if b(a, "primary") {
        // The prominent button: the default action of its row or page
        th.text().add_modifier(Modifier::BOLD)
    } else {
        th.text()
    };
    let mut spans = Vec::new();
    if let Some(on) = a.get("on").and_then(Value::as_bool) {
        spans.push(Span::styled(if on { "[x] " } else { "[ ] " }, if on { th.accent() } else { style }));
    } else if b(a, "busy") {
        spans.push(Span::styled(format!("{} ", spinner(ctx.tick)), th.accent()));
    } else {
        spans.extend(icon(ctx, s(a, "icon"), style));
    }
    spans.push(Span::styled(t(&a["label"]), style));
    if let Some(n) = a.get("badge").and_then(Value::as_u64) {
        spans.push(Span::styled(format!(" ({n})"), th.tone(Tone::Warning)));
    }
    spans
}

/// "Label   value  note"
fn info_line(ctx: &Ctx, info: &Value, indent: usize) -> Line<'static> {
    let th = ctx.theme;
    let mut spans = vec![
        Span::raw(" ".repeat(indent)),
        Span::styled(format!("{:<14}", t(&info["label"])), th.secondary()),
        Span::raw(" "),
    ];
    let value = t(&info["value"]);
    match (info.get("note"), info.get("tone")) {
        (Some(note), _) => {
            spans.push(Span::styled(value, th.text()));
            spans.push(Span::styled(format!("  {}", t(note)), th.tone(tone_of(info))));
        }
        (None, Some(_)) => spans.push(Span::styled(value, th.tone(tone_of(info)))),
        (None, None) => spans.push(Span::styled(value, th.text())),
    }
    Line::from(spans)
}

/// A hint: an empty list, a filter that leaves nothing, a file that cannot be read; with its action selectable
fn hint(ctx: &Ctx, key: String, h: &Value) -> Item {
    let style = ctx.theme.tone(match tone_of(h) {
        Tone::Normal => Tone::Normal,
        tone => tone,
    });
    let mut spans = vec![Span::raw("  ")];
    spans.extend(icon(ctx, s(h, "icon"), style));
    spans.push(Span::styled(t(&h["text"]), style));
    match h.get("action") {
        Some(a) if a.is_object() => {
            spans.push(Span::raw("  "));
            spans.extend(button(ctx, a));
            Item::row(key, vec![Line::from(spans)], vec![a.clone()])
        }
        _ => Item::line(key, Line::from(spans)),
    }
}

fn heading(ctx: &Ctx, text: String) -> Item {
    Item::line("", Line::from(Span::styled(text, ctx.theme.title())))
}

fn secondary_line(ctx: &Ctx, text: String, indent: usize) -> Line<'static> {
    Line::from(vec![Span::raw(" ".repeat(indent)), Span::styled(text, ctx.theme.secondary())])
}

fn toned_line(ctx: &Ctx, text: String, tone: Tone, indent: usize) -> Line<'static> {
    Line::from(vec![Span::raw(" ".repeat(indent)), Span::styled(text, ctx.theme.tone(tone))])
}

/// A bar of actions (a page's buttons) as one selectable item
fn button_bar(ctx: &Ctx, key: &str, actions: &[Value]) -> Option<Item> {
    if actions.is_empty() {
        return None;
    }
    let mut spans = vec![Span::raw("  ")];
    for (i, a) in actions.iter().enumerate() {
        if i > 0 {
            spans.push(Span::styled("  ·  ", ctx.theme.border()));
        }
        spans.extend(button(ctx, a));
    }
    Some(Item::row(key.into(), vec![Line::from(spans)], actions.to_vec()))
}

/// "[####------] 62 %"
pub fn gauge(fraction: f64, width: usize) -> String {
    let f = fraction.clamp(0.0, 1.0);
    let done = (f * width as f64).round() as usize;
    format!("[{}{}] {:>3} %", "#".repeat(done), "-".repeat(width - done), (f * 100.0).round() as i64)
}

fn is_open(ctx: &Ctx, key: &str, by_default: bool) -> bool {
    if by_default {
        !ctx.collapsed.contains(key)
    } else {
        ctx.expanded.contains(key)
    }
}

/// The first line of a row: lamp, kind, name, what follows the name, badges, tags
fn row_head(ctx: &Ctx, row: &Value, name: String) -> Line<'static> {
    let th = ctx.theme;
    let mut spans = vec![Span::raw(" ")];
    if let Some(l) = row.get("lamp") {
        spans.push(lamp(ctx, l));
        spans.push(Span::raw(" "));
    }
    if let Some(k) = row.get("kind") {
        spans.extend(icon(ctx, s(k, "icon"), th.secondary()));
    } else if let Some(i) = row.get("icon").and_then(Value::as_str) {
        spans.extend(icon(ctx, i, th.secondary()));
    }
    spans.push(Span::styled(name, th.title()));
    for sec in arr(row, "secondary") {
        spans.push(Span::styled(format!("  {}", t(sec)), th.secondary()));
    }
    for bd in arr(row, "badges") {
        spans.extend(badge(ctx, bd));
    }
    for tag in arr(row, "tags") {
        spans.push(Span::styled(format!(" #{}", t(tag)), th.secondary()));
    }
    Line::from(spans)
}

/// A list row (VM, image, recipe, offered recipe, source): its head, subtitle and, opened, its details
fn list_row(ctx: &Ctx, prefix: &str, row: &Value, out: &mut Vec<Item>) {
    let name = s(row, "name").to_string();
    let key = format!("{prefix}:{}", row.get("id").and_then(Value::as_str).unwrap_or(&name));
    let detail = row.get("detail").filter(|d| d.is_object());
    let open = detail.is_some() && is_open(ctx, &key, false);
    let mut lines = vec![row_head(ctx, row, name)];
    if let Some(sub) = row.get("subtitle") {
        lines.push(secondary_line(ctx, t(sub), 4));
    }
    // A look's colours (plan B141): a strip of blocks in its own colours
    let swatches = arr(row, "swatches");
    if !swatches.is_empty() {
        let mut spans = vec![Span::raw("    ")];
        for c in swatches {
            let hex = c.as_str().unwrap_or("").trim_start_matches('#');
            let v = u32::from_str_radix(hex, 16).unwrap_or(0x808080);
            let color = Color::Rgb((v >> 16) as u8, (v >> 8) as u8, v as u8);
            spans.push(Span::styled("██", Style::default().fg(color)));
        }
        lines.push(Line::from(spans));
    }
    let mut actions: Vec<Value> = arr(row, "actions").to_vec();
    let main = actions.len();
    let mut sub_items = Vec::new();
    if let Some(d) = detail {
        actions.extend(arr(d, "switches").iter().cloned());
        actions.extend(arr(d, "actions").iter().cloned());
        if open {
            if let Some(desc) = d.get("description").map(t).filter(|s| !s.is_empty()) {
                lines.push(secondary_line(ctx, desc, 4));
            }
            if let Some(w) = d.get("warning") {
                lines.push(toned_line(ctx, t(w), Tone::Warning, 4));
            }
            let switches = arr(d, "switches");
            if !switches.is_empty() {
                let mut spans = vec![Span::raw("    ")];
                for sw in switches {
                    spans.extend(button(ctx, sw));
                    spans.push(Span::raw("   "));
                }
                lines.push(Line::from(spans));
            }
            if let Some(note) = d.get("switchesNote") {
                lines.push(secondary_line(ctx, t(note), 4));
            }
            for info in arr(d, "info") {
                lines.push(info_line(ctx, info, 4));
            }
            let detail_actions = arr(d, "actions");
            if !detail_actions.is_empty() {
                // Numbered as in the footer (the row's actions, the switches, then these)
                let first = main + arr(d, "switches").len();
                let mut spans = vec![Span::raw("    ")];
                for (i, a) in detail_actions.iter().enumerate() {
                    if first + i < 9 {
                        spans.push(Span::styled(format!("{} ", first + i + 1), ctx.theme.key()));
                    }
                    spans.extend(button(ctx, a));
                    spans.push(Span::raw("   "));
                }
                lines.push(Line::from(spans));
            }
            // What is missing or wrong (plan B142): hints, each with its action
            for (i, n) in arr(d, "notes").iter().enumerate() {
                sub_items.push(hint(ctx, format!("{key}/note{i}"), n));
            }
            for v in arr(d, "versions") {
                let vkey = format!("{key}/{}", s(v, "id"));
                let vline = Line::from(vec![
                    Span::raw("      "),
                    Span::styled(s(v, "words").to_string(), ctx.theme.text()),
                    Span::styled(format!("  {}", t(&v["state"])), ctx.theme.secondary()),
                ]);
                sub_items.push(Item::row(vkey, vec![vline], arr(v, "actions").to_vec()));
            }
        }
    }
    let mut item = Item::row(key, lines, actions);
    // Enter: the row's own primary button, not a detail action
    item.default = primary(&item.actions[..main]);
    item.expandable = detail.is_some();
    item.open = open;
    out.push(item);
    out.extend(sub_items);
}

/// A form field: its label, value and what is wrong with it
fn field(ctx: &Ctx, prefix: &str, f: &Value) -> Item {
    let th = ctx.theme;
    let enabled = b(f, "enabled");
    let label_style = if enabled { th.text() } else { th.disabled() };
    let value = f.get("value").unwrap_or(&Value::Null);
    let placeholder = f.get("placeholder").map(t).unwrap_or_default();
    let shown = |v: &Value| -> Span<'static> {
        match v {
            Value::Null => Span::styled(placeholder.clone(), th.secondary()),
            Value::String(s) if s.is_empty() => Span::styled(placeholder.clone(), th.secondary()),
            v => Span::styled(t(v), if enabled { th.accent() } else { th.disabled() }),
        }
    };
    let mut spans = vec![Span::raw("  ")];
    match s(f, "kind") {
        "toggle" => {
            let on = value.as_bool().unwrap_or(false);
            spans.push(Span::styled(if on { "[x] " } else { "[ ] " }, if on { th.accent() } else { label_style }));
            spans.push(Span::styled(t(&f["label"]), label_style));
        }
        "number" => {
            spans.push(Span::styled(t(&f["label"]), label_style));
            spans.push(Span::styled("  ‹ ", th.secondary()));
            spans.push(shown(value));
            spans.push(Span::styled(" ›", th.secondary()));
        }
        "choice" => {
            spans.push(Span::styled(format!("{}: ", t(&f["label"])), label_style));
            let chosen = arr(f, "choices").iter().find(|c| c.get("value").unwrap_or(&Value::Null) == value);
            match chosen {
                Some(c) => spans.push(Span::styled(
                    format!("‹ {} ›", t(&c["label"])),
                    if enabled { th.accent() } else { th.disabled() },
                )),
                None => spans.push(shown(value)),
            }
        }
        "info" => {
            spans.push(Span::styled(format!("{}: ", t(&f["label"])), th.secondary()));
            spans.push(Span::styled(t(value), th.text()));
        }
        _ => {
            spans.push(Span::styled(format!("{}: ", t(&f["label"])), label_style));
            spans.push(shown(value));
        }
    }
    let mut lines = vec![Line::from(spans)];
    if let Some(e) = f.get("error") {
        lines.push(toned_line(ctx, t(e), Tone::Danger, 6));
    } else if let (false, Some(r)) = (enabled, f.get("reason")) {
        lines.push(secondary_line(ctx, t(r), 6));
    }
    let selectable = s(f, "kind") != "info";
    Item {
        key: format!("{prefix}:field:{}", s(f, "id")),
        lines,
        field: selectable.then(|| f.clone()),
        selectable,
        ..Item::default()
    }
}

/// "Not possible yet – the Doctor reports:" with its lines and the way to the Doctor
fn blockers(ctx: &Ctx, key: &str, bl: &Value, out: &mut Vec<Item>) {
    let mut lines = vec![toned_line(ctx, t(&bl["title"]), Tone::Warning, 2)];
    for it in arr(bl, "items") {
        lines.push(Line::from(vec![
            Span::raw("    • "),
            Span::styled(t(&it["title"]), ctx.theme.text()),
            Span::styled(format!(": {}", t(&it["hint"])), ctx.theme.secondary()),
        ]));
    }
    let actions: Vec<Value> = bl.get("action").into_iter().cloned().collect();
    if let Some(a) = actions.first() {
        let mut spans = vec![Span::raw("    ")];
        spans.extend(button(ctx, a));
        lines.push(Line::from(spans));
    }
    out.push(Item::row(key.into(), lines, actions));
}

fn list_head(page: &mut Page, model: &Value) {
    let count = model.get("count").and_then(Value::as_u64).unwrap_or(0);
    page.count = Some(match model.get("of").and_then(Value::as_u64) {
        Some(of) => format!("{count}/{of}"),
        None => count.to_string(),
    });
    page.filter = model.get("filter").cloned();
    page.tools = arr(model, "actions").to_vec();
}

// MARK: - Pages

/// The page `page` from its model
pub fn page(ctx: &Ctx, name: &str, model: &Value) -> Page {
    let mut p = Page::default();
    if let Some(e) = model.get("error") {
        p.title = name.to_string();
        // The message in the TUI's language when it has a key (api.md §3)
        let message = e.get("text").filter(|x| x.is_object()).unwrap_or(&e["message"]);
        p.items.push(Item::line("", toned_line(ctx, t(message), Tone::Danger, 2)));
        return p;
    }
    // Every page has its title (api.md §10.5); the TUI keeps no table of its own
    p.title = t(&model["title"]);
    match name {
        "vms" => vms(ctx, model, &mut p),
        "images" => images(ctx, model, &mut p),
        "recipes" => recipes(ctx, model, &mut p),
        "activity" => activity(ctx, model, &mut p),
        // Modules in the looks' shape (plan B142)
        "looks" | "modules" => looks(ctx, model, &mut p),
        "vm" | "image" => {
            let mut open = ctx.expanded.clone();
            let key = format!("{name}:{}", s(&model["row"], "name"));
            open.insert(key);
            let inner = Ctx { expanded: &open, ..*ctx };
            list_row(&inner, name, &model["row"], &mut p.items);
        }
        "build" => build(ctx, model, &mut p),
        "doctor" => doctor(ctx, model, &mut p),
        "macBase" => mac_base(ctx, model, &mut p),
        "newVm" => new_vm(ctx, model, &mut p),
        "settings" => settings(ctx, model, &mut p),
        "credentials" | "locales" | "profiles" => credentials(ctx, model, &mut p),
        "locale" => credential(ctx, "locale", model, &mut p),
        "profile" => credential(ctx, "profile", model, &mut p),
        // The recipe studio (plan B147)
        "recipeStudio" => studio(ctx, model, &mut p),
        "keymapStudio" => keymap_studio(ctx, model, &mut p),
        "credential" => credential(ctx, "credential", model, &mut p),
        // The credential of a macOS base from before bases kept theirs: the same form (plan B139)
        "baseCredential" => credential(ctx, "baseCredential", model, &mut p),
        "keys" => keys(ctx, model, &mut p),
        _ => {}
    }
    if p.title.is_empty() {
        p.title = name.to_string();
    }
    p
}

fn empty_and_unreadable(ctx: &Ctx, model: &Value, p: &mut Page) {
    if let Some(e) = model.get("empty") {
        p.items.push(hint(ctx, "empty".into(), e));
    }
    for (i, u) in arr(model, "unreadable").iter().enumerate() {
        p.items.push(hint(ctx, format!("unreadable:{i}"), u));
    }
}

fn vms(ctx: &Ctx, model: &Value, p: &mut Page) {
    list_head(p, model);
    for row in arr(model, "rows") {
        list_row(ctx, "vm", row, &mut p.items);
    }
    empty_and_unreadable(ctx, model, p);
}

/// A build (or the macOS base being created) that waits, runs or ended: its state, progress and buttons
fn job_item(ctx: &Ctx, job: &Value) -> Item {
    let th = ctx.theme;
    let state = s(job, "state");
    let (symbol, tone) = match state {
        "queued" => ("◷".to_string(), Tone::Normal),
        "failed" => (ctx.icons.get("failed").to_string(), Tone::Danger),
        "cancelled" => (ctx.icons.get("cancelled").to_string(), Tone::Normal),
        "finished" => (ctx.icons.get("finished").to_string(), Tone::Success),
        _ => (spinner(ctx.tick).to_string(), Tone::Accent),
    };
    let mut spans = vec![Span::raw(" "), Span::styled(symbol, th.tone(tone)), Span::raw(" ")];
    spans.extend(icon(ctx, s(&job["kind"], "icon"), th.secondary()));
    spans.push(Span::styled(s(job, "name").to_string(), th.title()));
    spans.push(Span::styled(format!("  {}", t(&job["title"])), th.tone(tone)));
    if let Some(c) = job.get("count") {
        spans.push(Span::styled(format!("  {}", t(c)), th.secondary()));
    }
    let mut lines = vec![Line::from(spans)];
    if let Some(f) = job.get("fraction").and_then(Value::as_f64) {
        lines.push(Line::from(vec![Span::raw("    "), Span::styled(gauge(f, 30), th.accent())]));
    }
    // `open` (the build page) first, the row's own buttons follow; Enter does the primary one
    let mut actions: Vec<Value> = job.get("open").filter(|o| o.is_object()).into_iter().cloned().collect();
    actions.extend(arr(job, "actions").iter().cloned());
    Item::row(format!("job:{}", s(job, "id")), lines, actions)
}

fn images(ctx: &Ctx, model: &Value, p: &mut Page) {
    list_head(p, model);
    for job in arr(model, "jobs") {
        p.items.push(job_item(ctx, job));
    }
    for row in arr(model, "rows") {
        list_row(ctx, "image", row, &mut p.items);
    }
    empty_and_unreadable(ctx, model, p);
    // The macOS bases (on a Mac, plan B137)
    let bases = &model["bases"];
    if bases.is_object() {
        p.items.push(Item::blank());
        p.items.push(heading(ctx, t(&bases["title"])));
        // The base being created (plan B142): its progress; Enter opens its page
        if let Some(job) = bases.get("job").filter(|j| j.is_object()) {
            p.items.push(job_item(ctx, job));
        }
        for row in arr(bases, "rows") {
            list_row(ctx, "base", row, &mut p.items);
        }
        match bases.get("empty") {
            Some(e) => p.items.push(hint(ctx, "bases:empty".into(), e)),
            None => {
                if let Some(bar) = button_bar(ctx, "bases:create", std::slice::from_ref(&bases["create"])) {
                    p.items.push(bar);
                }
            }
        }
        p.items.push(Item::line("", secondary_line(ctx, t(&bases["footer"]), 2)));
    }
}

fn recipes(ctx: &Ctx, model: &Value, p: &mut Page) {
    list_head(p, model);
    if b(model, "loading") {
        let mut spans = vec![Span::raw("  "), Span::styled(spinner(ctx.tick), ctx.theme.accent())];
        if let Some(text) = model.get("loadingText") {
            spans.push(Span::styled(format!(" {}", t(text)), ctx.theme.secondary()));
        }
        p.items.push(Item::line("", Line::from(spans)));
    }
    for (i, e) in arr(model, "errors").iter().enumerate() {
        p.items.push(hint(ctx, format!("error:{i}"), e));
    }
    if let Some(sg) = model.get("suggestion") {
        p.items.push(hint(ctx, "suggestion".into(), sg));
    }
    if let Some(u) = model.get("updates") {
        p.items.push(heading(ctx, t(&u["title"])));
        for row in arr(u, "rows") {
            let mut spans = vec![Span::raw("   ")];
            spans.extend(icon(ctx, s(&row["kind"], "icon"), ctx.theme.secondary()));
            spans.push(Span::styled(s(row, "name").to_string(), ctx.theme.text()));
            spans.push(Span::styled(format!("  {}", s(row, "source")), ctx.theme.secondary()));
            spans.push(Span::raw("  "));
            spans.extend(button(ctx, &row["action"]));
            p.items.push(Item::row(
                format!("update:{}", s(row, "id")),
                vec![Line::from(spans)],
                vec![row["action"].clone()],
            ));
        }
        if let Some(all) = u.get("all") {
            if let Some(bar) = button_bar(ctx, "updates:all", std::slice::from_ref(all)) {
                p.items.push(bar);
            }
        }
        p.items.push(Item::line("", secondary_line(ctx, t(&u["note"]), 3)));
        p.items.push(Item::blank());
    }
    for row in arr(model, "rows") {
        list_row(ctx, "recipe", row, &mut p.items);
    }
    let offered = arr(model, "offered");
    if !offered.is_empty() {
        if let Some(title) = model.get("offeredTitle") {
            p.items.push(Item::blank());
            p.items.push(heading(ctx, t(title)));
        }
        for row in offered {
            list_row(ctx, "offered", row, &mut p.items);
        }
    }
    empty_and_unreadable(ctx, model, p);
    if let (true, Some(h)) = (b(model, "labHidden"), model.get("labHint")) {
        p.items.push(hint(ctx, "labHint".into(), h));
    }
}

fn activity(ctx: &Ctx, model: &Value, p: &mut Page) {
    list_head(p, model);
    let th = ctx.theme;
    for e in arr(model, "entries") {
        let (symbol, tone) = match s(e, "level") {
            "error" => (ctx.icons.get("failed"), Tone::Danger),
            "warning" => (ctx.icons.get("warning"), Tone::Warning),
            _ => ("·", Tone::Normal),
        };
        let time = parse_date(s(e, "time"))
            .map(|d| d.with_timezone(&chrono::Local).format("%H:%M:%S").to_string())
            .unwrap_or_default();
        let line = Line::from(vec![
            Span::styled(format!(" {time} "), th.secondary()),
            Span::styled(symbol.to_string(), th.tone(tone)),
            Span::raw(" "),
            Span::styled(t(&e["text"]), if tone == Tone::Normal { th.text() } else { th.tone(tone) }),
        ]);
        let id = e.get("id").map(|v| v.to_string()).unwrap_or_default();
        p.items.push(Item { key: format!("entry:{id}"), lines: vec![line], selectable: true, ..Item::default() });
    }
    empty_and_unreadable(ctx, model, p);
}

/// One key as caps: "⌃+g", a sequence with spaces ("␣ f f")
fn caps(key: &Value) -> String {
    let list = |v: &Value| v.as_array().cloned().unwrap_or_default();
    list(key)
        .iter()
        .map(|chord| list(chord).iter().filter_map(Value::as_str).collect::<Vec<_>>().join("+"))
        .collect::<Vec<_>>()
        .join(" ")
}

/// A recipe's keys (plan B147): a heading per module, per mode its way there, a row per key with where it leads;
/// Enter on a row shows what it does in the program's own words
fn keys(ctx: &Ctx, model: &Value, p: &mut Page) {
    let th = ctx.theme;
    p.title = t(&model["title"]);
    p.items.push(Item::line("", secondary_line(ctx, t(&model["subtitle"]), 1)));
    p.items.push(Item::line("", secondary_line(ctx, t(&model["intro"]), 1)));
    for (i, n) in arr(model, "notes").iter().enumerate() {
        p.items.push(hint(ctx, format!("note:{i}"), n));
    }
    for m in arr(model, "modules") {
        p.items.push(Item::blank());
        p.items.push(heading(ctx, format!(" {}", s(m, "title"))));
        for (gi, g) in arr(m, "groups").iter().enumerate() {
            let mut head = vec![Span::raw("  "), Span::styled(t(&g["title"]), th.accent())];
            if g["start"].as_bool() == Some(true) {
                head.push(Span::styled(format!("  {}", tr("start")), th.secondary()));
            }
            let way: Vec<String> = arr(g, "way").iter().map(|st| caps(&st["caps"])).collect();
            if !way.is_empty() {
                head.push(Span::styled(format!("   {}", way.join(" › ")), th.key()));
            }
            p.items.push(Item::line("", Line::from(head)));
            for r in arr(g, "rows") {
                let key = format!("key:{}:{gi}:{}", s(m, "reference"), s(r, "action"));
                let keys: Vec<String> = arr(r, "caps").iter().map(caps).collect();
                let mut line = vec![
                    Span::raw("    "),
                    Span::styled(format!("{:<16}", keys.join(" / ")), th.key()),
                    Span::raw(" "),
                    Span::styled(
                        t(&r["title"]),
                        if r["switches"].as_bool() == Some(true) { th.secondary() } else { th.text() },
                    ),
                ];
                if let Some(after) = r.get("after") {
                    let back = r["backToStart"].as_bool() == Some(true);
                    let style = if back { th.secondary() } else { th.tone(Tone::Accent) };
                    line.push(Span::styled(format!("  {} {}", if back { "↩" } else { "→" }, t(after)), style));
                }
                let mut lines = vec![Line::from(line)];
                if is_open(ctx, &key, false) {
                    if let Some(does) = r.get("does").and_then(Value::as_str) {
                        lines.push(secondary_line(ctx, does.to_string(), 21));
                    }
                    if let Some(from) = r.get("from").and_then(Value::as_str) {
                        lines.push(secondary_line(ctx, trf("Key from %@", &[from.into()]), 21));
                    }
                }
                p.items.push(Item { key, lines, selectable: true, ..Item::default() });
            }
        }
    }
}

fn build(ctx: &Ctx, model: &Value, p: &mut Page) {
    let th = ctx.theme;
    p.tools = arr(model, "actions").to_vec();
    let mut head = vec![Span::raw(" ")];
    head.extend(icon(ctx, s(&model["kind"], "icon"), th.secondary()));
    head.push(Span::styled(s(model, "name").to_string(), th.title()));
    for bd in arr(model, "badges") {
        head.extend(badge(ctx, bd));
    }
    p.items.push(Item::line("", Line::from(head)));
    if let Some(d) = model.get("description").map(t).filter(|s| !s.is_empty()) {
        p.items.push(Item::line("", secondary_line(ctx, d, 3)));
    }
    // The credential and the look of the image (plan B137) – their values go into the build action
    for f in arr(model, "fields") {
        p.items.push(field(ctx, "build", f));
    }
    if let Some(bar) = button_bar(ctx, "build:actions", arr(model, "actions")) {
        p.items.push(bar);
    }
    if let Some(bl) = model.get("blockers") {
        blockers(ctx, "build:blockers", bl, &mut p.items);
    }
    if let Some(h) = model.get("macBase") {
        p.items.push(hint(ctx, "build:macBase".into(), h));
    }
    // What the recipe is made of and what is wrong (plan B142)
    for (i, n) in arr(model, "notes").iter().enumerate() {
        p.items.push(hint(ctx, format!("build:note{i}"), n));
    }
    if let Some(st) = model.get("status") {
        p.items.push(Item::line("", secondary_line(ctx, t(st), 2)));
    }
    if let Some(pr) = model.get("progress") {
        let mut spans = vec![
            Span::raw("  "),
            Span::styled(format!("{} ", t(&pr["step"])), th.secondary()),
            Span::styled(t(&pr["title"]), th.text()),
        ];
        if let Some(c) = pr.get("count") {
            spans.push(Span::styled(format!("  {}", t(c)), th.secondary()));
        }
        let mut lines = vec![Line::from(spans)];
        let mut second = vec![
            Span::raw("  "),
            Span::styled(gauge(pr.get("fraction").and_then(Value::as_f64).unwrap_or(0.0), 30), th.accent()),
        ];
        // The running time and the quiet time are the UI's to count (api.md §10.5)
        if let Some(start) = pr.get("started").and_then(Value::as_str).and_then(parse_date) {
            second.push(Span::styled(format!("  {}", duration_text((ctx.now - start).num_seconds())), th.secondary()));
        }
        if let Some(last) = pr.get("lastOutput").and_then(Value::as_str).and_then(parse_date) {
            let quiet = (ctx.now - last).num_seconds();
            let warn = pr.get("quietWarning").and_then(Value::as_i64).unwrap_or(i64::MAX);
            if quiet >= warn {
                second.push(Span::styled(format!("  quiet {}", duration_text(quiet)), th.tone(Tone::Warning)));
                lines.push(Line::from(second));
                lines.push(toned_line(ctx, t(&pr["quietHelp"]), Tone::Warning, 2));
            } else {
                lines.push(Line::from(second));
            }
        } else {
            lines.push(Line::from(second));
        }
        p.items.push(Item { key: "build:progress".into(), lines, ..Item::default() });
    }
    if let Some(o) = model.get("outcome") {
        let tone = tone_of(o);
        let mut spans = vec![Span::raw("  ")];
        spans.extend(icon(ctx, s(o, "icon"), th.tone(tone)));
        spans.push(Span::styled(t(&o["title"]), th.tone(tone).add_modifier(Modifier::BOLD)));
        let mut lines = vec![Line::from(spans)];
        if let Some(d) = o.get("detail") {
            lines.push(secondary_line(ctx, t(d), 4));
        }
        let actions: Vec<Value> = o.get("action").into_iter().cloned().collect();
        if let Some(a) = actions.first() {
            let mut spans = vec![Span::raw("    ")];
            spans.extend(button(ctx, a));
            lines.push(Line::from(spans));
        }
        p.items.push(Item::row("build:outcome".into(), lines, actions));
    }
    if let Some(n) = model.get("newest") {
        p.items.push(Item::line("", secondary_line(ctx, t(n), 2)));
    }
    if let Some(log) = model.get("log").filter(|l| l.is_object()) {
        if let Some(full) = log.get("full") {
            if let Some(bar) = button_bar(ctx, "build:log", std::slice::from_ref(full)) {
                p.items.push(bar);
            }
        }
        p.log = LogBox::from_model(log);
    }
}

fn doctor(ctx: &Ctx, model: &Value, p: &mut Page) {
    let th = ctx.theme;
    p.tools = arr(model, "actions").to_vec();
    // "omaflake" with this UI's and the core's version (the core says both), then the update right below
    for v in arr(model, "versions") {
        p.items.push(Item::line("", info_line(ctx, v, 2)));
    }
    let up = &model["update"];
    if up.is_object() {
        let mut spans = vec![
            Span::raw("  "),
            Span::styled(format!("{:<14} ", t(&up["label"])), th.secondary()),
            Span::styled(t(&up["text"]), th.tone(tone_of(up))),
        ];
        if b(up, "busy") {
            spans.push(Span::styled(format!(" {}", spinner(ctx.tick)), th.accent()));
        }
        let actions = arr(up, "actions").to_vec();
        for a in &actions {
            spans.push(Span::raw("  "));
            spans.extend(button(ctx, a));
        }
        p.items.push(Item::row("doctor:update".into(), vec![Line::from(spans)], actions));
    }
    p.items.push(Item::blank());
    p.items.push(Item::line("", secondary_line(ctx, t(&model["intro"]), 2)));
    let mut checked = vec![Span::raw("  ")];
    if b(model, "checking") {
        checked.push(Span::styled(format!("{} ", spinner(ctx.tick)), th.accent()));
    }
    if let Some(c) = model.get("checked").and_then(Value::as_str) {
        checked.push(Span::styled(date_text(c), th.secondary()));
    }
    p.items.push(Item::line("", Line::from(checked)));
    for sec in arr(model, "sections") {
        p.items.push(Item::blank());
        p.items.push(heading(ctx, t(&sec["title"])));
        for it in arr(sec, "items") {
            let key = format!("doctor:{}", s(it, "item"));
            let (symbol, tone) = match s(it, "status") {
                "ok" => (ctx.icons.get("ok"), Tone::Success),
                "limited" => (ctx.icons.get("warning"), Tone::Warning),
                _ => (ctx.icons.get("failed"), Tone::Danger),
            };
            let mut spans = vec![Span::raw(" ")];
            if b(it, "busy") {
                spans.push(Span::styled(spinner(ctx.tick), th.accent()));
            } else {
                spans.push(Span::styled(symbol, th.tone(tone)));
            }
            spans.push(Span::raw(" "));
            spans.push(Span::styled(t(&it["title"]), th.text()));
            if let Some(d) = it.get("detail") {
                spans.push(Span::styled(format!("  {}", t(d)), th.secondary()));
            }
            let mut lines = vec![Line::from(spans)];
            let expandable = b(it, "expandable");
            let open = expandable && is_open(ctx, &key, b(it, "expanded"));
            if open {
                if let Some(h) = it.get("hint") {
                    lines.push(toned_line(ctx, t(h), tone, 4));
                }
                if let Some(task) = it.get("task") {
                    lines.push(toned_line(ctx, t(task), Tone::Accent, 4));
                }
                if let Some(d) = it.get("download") {
                    let of = d.get("of").and_then(Value::as_f64).unwrap_or(0.0);
                    let done = d.get("done").and_then(Value::as_f64).unwrap_or(0.0);
                    let f = if of > 0.0 { done / of } else { 0.0 };
                    lines.push(Line::from(vec![
                        Span::raw("    "),
                        Span::styled(gauge(f, 24), th.accent()),
                        Span::styled(format!("  {}", t(&d["text"])), th.secondary()),
                    ]));
                }
                let acts = arr(it, "actions");
                if !acts.is_empty() {
                    let mut spans = vec![Span::raw("    ")];
                    for (i, a) in acts.iter().enumerate() {
                        if i > 0 {
                            spans.push(Span::styled("  ·  ", th.border()));
                        }
                        spans.extend(button(ctx, a));
                    }
                    lines.push(Line::from(spans));
                }
            }
            let mut item = Item::row(key, lines, arr(it, "actions").to_vec());
            item.expandable = expandable;
            item.open = open;
            p.items.push(item);
        }
    }
}

fn mac_base(ctx: &Ctx, model: &Value, p: &mut Page) {
    let th = ctx.theme;
    p.tools = arr(model, "actions").to_vec();
    p.items.push(Item::line("", secondary_line(ctx, t(&model["intro"]), 2)));
    if let Some(h) = model.get("base") {
        p.items.push(hint(ctx, "macBase:base".into(), h));
    }
    let ipsw = &model["ipsw"];
    if ipsw.is_object() {
        let mut lines = vec![Line::from(vec![
            Span::raw("  "),
            Span::styled(format!("{}: ", t(&ipsw["label"])), th.text()),
            Span::styled(t(&ipsw["text"]), th.accent()),
        ])];
        if let Some(d) = ipsw.get("download") {
            let of = d.get("of").and_then(Value::as_f64).unwrap_or(0.0);
            let done = d.get("done").and_then(Value::as_f64).unwrap_or(0.0);
            lines.push(Line::from(vec![
                Span::raw("    "),
                Span::styled(gauge(if of > 0.0 { done / of } else { 0.0 }, 24), th.accent()),
                Span::styled(format!("  {}", t(&d["text"])), th.secondary()),
            ]));
        }
        let actions = arr(ipsw, "actions").to_vec();
        if !actions.is_empty() {
            let mut spans = vec![Span::raw("    ")];
            for a in &actions {
                spans.extend(button(ctx, a));
                spans.push(Span::raw("   "));
            }
            lines.push(Line::from(spans));
        }
        p.items.push(Item::row("macBase:ipsw".into(), lines, actions));
    }
    for f in arr(model, "fields") {
        p.items.push(field(ctx, "macBase", f));
    }
    if let Some(bar) = button_bar(ctx, "macBase:actions", arr(model, "actions")) {
        p.items.push(bar);
    }
    // The IPSW is shown once, in its line above (it is no field); the log as the build page's
    if let Some(log) = model.get("log").filter(|l| l.is_object()) {
        if let Some(full) = log.get("full") {
            if let Some(bar) = button_bar(ctx, "macBase:log", std::slice::from_ref(full)) {
                p.items.push(bar);
            }
        }
        p.log = LogBox::from_model(log);
    }
}

fn new_vm(ctx: &Ctx, model: &Value, p: &mut Page) {
    let th = ctx.theme;
    p.tools = arr(model, "actions").to_vec();
    let mut head = vec![Span::raw(" ")];
    head.extend(icon(ctx, s(&model["kind"], "icon"), th.secondary()));
    head.push(Span::styled(s(model, "image").to_string(), th.title()));
    if b(model, "lab") {
        head.push(Span::styled(format!(" {}", ctx.icons.get("lab")), th.tone(Tone::Lab)));
    }
    p.items.push(Item::line("", Line::from(head)));
    p.items.push(Item::line("", secondary_line(ctx, t(&model["subtitle"]), 3)));
    if let Some(d) = model.get("description").map(t).filter(|s| !s.is_empty()) {
        p.items.push(Item::line("", secondary_line(ctx, d, 3)));
    }
    if let Some(bl) = model.get("blockers") {
        blockers(ctx, "newVm:blockers", bl, &mut p.items);
    }
    for sec in arr(model, "sections") {
        p.items.push(Item::blank());
        if let Some(title) = sec.get("title") {
            p.items.push(heading(ctx, t(title)));
        }
        for f in arr(sec, "fields") {
            p.items.push(field(ctx, "newVm", f));
        }
    }
    for w in arr(model, "warnings") {
        p.items.push(Item::line("", toned_line(ctx, t(w), Tone::Warning, 2)));
    }
    p.items.push(Item::blank());
    if let Some(bar) = button_bar(ctx, "newVm:actions", arr(model, "actions")) {
        p.items.push(bar);
    }
}

/// The Credentials page (plan B137): what they are, one row each with its actions, "New Credential …", the footer
/// The Looks tab (plan B141): the looks here with a strip of their colours, what the sources offer besides
fn looks(ctx: &Ctx, model: &Value, p: &mut Page) {
    list_head(p, model);
    if b(model, "loading") {
        let mut spans = vec![Span::raw("  "), Span::styled(spinner(ctx.tick), ctx.theme.accent())];
        if let Some(text) = model.get("loadingText") {
            spans.push(Span::styled(format!(" {}", t(text)), ctx.theme.secondary()));
        }
        p.items.push(Item::line("", Line::from(spans)));
    }
    for row in arr(model, "rows") {
        list_row(ctx, "look", row, &mut p.items);
    }
    if let Some(title) = model.get("offeredTitle") {
        p.items.push(heading(ctx, t(title)));
        for row in arr(model, "offered") {
            list_row(ctx, "offered-look", row, &mut p.items);
        }
    }
    for (i, h) in arr(model, "unreadable").iter().enumerate() {
        p.items.push(hint(ctx, format!("unreadable-look:{i}"), h));
    }
    if let Some(f) = model.get("footer") {
        p.items.push(Item::blank());
        p.items.push(Item::line("", secondary_line(ctx, t(f), 2)));
    }
}

fn credentials(ctx: &Ctx, model: &Value, p: &mut Page) {
    p.items.push(Item::line("", secondary_line(ctx, t(&model["intro"]), 2)));
    p.items.push(Item::blank());
    for row in arr(model, "rows") {
        list_row(ctx, "credential", row, &mut p.items);
    }
    if let Some(bar) = button_bar(ctx, "credentials:add", std::slice::from_ref(&model["add"])) {
        p.items.push(bar);
    }
    if let Some(f) = model.get("footer") {
        p.items.push(Item::line("", secondary_line(ctx, t(f), 2)));
    }
}

/// One credential's form (plan B137): what it is for, why only the name can change, the fields, Cancel and Save
fn credential(ctx: &Ctx, page: &str, model: &Value, p: &mut Page) {
    p.items.push(Item::line("", secondary_line(ctx, t(&model["intro"]), 2)));
    if let Some(l) = model.get("locked") {
        p.items.push(Item::line("", toned_line(ctx, t(l), Tone::Warning, 2)));
    }
    p.items.push(Item::blank());
    for f in arr(model, "fields") {
        p.items.push(field(ctx, page, f));
    }
    if let Some(bar) = button_bar(ctx, &format!("{page}:actions"), arr(model, "actions")) {
        p.items.push(bar);
    }
}

/// The recipe studio (plan B147): the recipe's fields, its modules with what is wrong, the catalog (search, filters,
/// modules with Add), look and keys, the VM; a module row's Enter adds or removes it
fn studio(ctx: &Ctx, model: &Value, p: &mut Page) {
    let th = ctx.theme;
    let page = "recipeStudio";
    let module = |m: &Value, at: &str| -> Item {
        let badges: Vec<String> = arr(m, "badges").iter().filter_map(|b| b.get("label").map(t)).collect();
        let mut head = vec![Span::raw("  "), Span::styled(s(m, "title").to_string(), th.title())];
        if !badges.is_empty() {
            head.push(Span::styled(format!("  {}", badges.join(" · ")), th.secondary()));
        }
        let lines = vec![Line::from(head), secondary_line(ctx, t(&m["subtitle"]), 4)];
        Item::row(format!("{at}:{}", s(m, "reference")), lines, vec![m["action"].clone()])
    };
    p.items.push(Item::line("", secondary_line(ctx, t(&model["intro"]), 2)));
    p.items.push(heading(ctx, format!(" {}", t(&model["titles"]["about"]))));
    for f in arr(model, "about") {
        p.items.push(field(ctx, page, f));
    }
    let recipe = &model["recipe"];
    p.items.push(Item::blank());
    p.items.push(heading(ctx, format!(" {}", t(&recipe["title"]))));
    if let Some(bar) = recipe.get("keys").and_then(|k| button_bar(ctx, "studio:keys", std::slice::from_ref(k))) {
        p.items.push(bar);
    }
    for (i, h) in arr(recipe, "hints").iter().enumerate() {
        p.items.push(hint(ctx, format!("studio:hint:{i}"), h));
    }
    if let Some(e) = recipe.get("empty") {
        p.items.push(Item::line("", secondary_line(ctx, t(e), 2)));
    }
    for m in arr(recipe, "rows") {
        p.items.push(module(m, "in"));
    }
    let catalog = &model["catalog"];
    p.items.push(Item::blank());
    p.items.push(Item::line(
        "",
        Line::from(vec![
            Span::styled(format!(" {}", t(&catalog["title"])), th.title()),
            Span::styled(format!("  {}", t(&catalog["count"])), th.secondary()),
        ]),
    ));
    p.items.push(field(ctx, page, &catalog["search"]));
    for f in arr(catalog, "filters") {
        p.items.push(field(ctx, page, f));
    }
    if let Some(e) = catalog.get("empty") {
        p.items.push(Item::line("", secondary_line(ctx, t(e), 2)));
    }
    for m in arr(catalog, "rows") {
        p.items.push(module(m, "catalog"));
    }
    for (title, list) in [("dress", "dress"), ("vm", "vm")] {
        p.items.push(Item::blank());
        p.items.push(heading(ctx, format!(" {}", t(&model["titles"][title]))));
        for f in arr(model, list) {
            p.items.push(field(ctx, page, f));
        }
    }
    if let Some(bar) = button_bar(ctx, &format!("{page}:actions"), arr(model, "actions")) {
        p.items.push(bar);
    }
}

/// The key binding studio (plan B147): its fields, the module and search, what overlaps, then the module's keys by
/// mode – each row a key field (and the mode afterwards), with what overlaps and Undo
fn keymap_studio(ctx: &Ctx, model: &Value, p: &mut Page) {
    let th = ctx.theme;
    let page = "keymapStudio";
    p.items.push(Item::line("", secondary_line(ctx, t(&model["intro"]), 2)));
    for f in arr(model, "about") {
        p.items.push(field(ctx, page, f));
    }
    p.items.push(Item::blank());
    p.items.push(field(ctx, page, &model["module"]));
    p.items.push(field(ctx, page, &model["search"]));
    p.items.push(Item::line("", secondary_line(ctx, t(&model["changed"]), 2)));
    for (i, h) in arr(model, "hints").iter().enumerate() {
        p.items.push(hint(ctx, format!("keymapStudio:hint:{i}"), h));
    }
    for (gi, g) in arr(model, "groups").iter().enumerate() {
        p.items.push(Item::blank());
        let way: Vec<String> = arr(g, "way").iter().map(caps).collect();
        let mut head = vec![Span::raw(" "), Span::styled(t(&g["title"]), th.accent())];
        if g["start"].as_bool() == Some(true) {
            head.push(Span::styled(format!("  {}", tr("start")), th.secondary()));
        }
        if !way.is_empty() {
            head.push(Span::styled(format!("   {}", way.join(" › ")), th.key()));
        }
        p.items.push(Item::line("", Line::from(head)));
        for r in arr(g, "rows") {
            p.items.push(field(ctx, page, &r["key"]));
            if r["after"].is_object() {
                p.items.push(field(ctx, page, &r["after"]));
            }
            if let Some(c) = r.get("conflict") {
                p.items.push(Item::line("", toned_line(ctx, t(c), Tone::Warning, 4)));
            }
            if let Some(bar) = r.get("reset").and_then(|a| {
                button_bar(ctx, &format!("keymapStudio:reset:{gi}:{}", s(r, "action")), std::slice::from_ref(a))
            }) {
                p.items.push(bar);
            }
        }
    }
    if let Some(bar) = button_bar(ctx, &format!("{page}:actions"), arr(model, "actions")) {
        p.items.push(bar);
    }
}

fn settings(ctx: &Ctx, model: &Value, p: &mut Page) {
    let th = ctx.theme;
    let src = &model["sources"];
    p.items.push(heading(ctx, t(&src["title"])));
    for row in arr(src, "rows") {
        list_row(ctx, "source", row, &mut p.items);
    }
    let add = &src["add"];
    if add.is_object() {
        p.items.push(Item::line("", secondary_line(ctx, t(&add["title"]), 2)));
        for f in arr(add, "fields") {
            p.items.push(field(ctx, "source", f));
        }
    }
    if let Some(bar) = button_bar(ctx, "source:add", std::slice::from_ref(&src["addAction"])) {
        if src["addAction"].is_object() {
            p.items.push(bar);
        }
    }
    if let Some(f) = src.get("footer") {
        p.items.push(Item::line("", secondary_line(ctx, t(f), 2)));
    }
    for id in ["general", "guest"] {
        let sec = &model[id];
        if !sec.is_object() {
            continue;
        }
        p.items.push(Item::blank());
        p.items.push(heading(ctx, t(&sec["title"])));
        for f in arr(sec, "fields") {
            p.items.push(field(ctx, id, f));
        }
        if let Some(f) = sec.get("footer") {
            p.items.push(Item::line("", secondary_line(ctx, t(f), 2)));
        }
    }
    let df = &model["dataFolder"];
    if df.is_object() {
        p.items.push(Item::blank());
        p.items.push(heading(ctx, t(&df["title"])));
        let mut spans = vec![Span::raw("  "), Span::styled(s(df, "location").to_string(), th.accent())];
        if b(df, "moving") {
            spans.push(Span::styled(format!(" {}", spinner(ctx.tick)), th.accent()));
        }
        p.items.push(Item::line("", Line::from(spans)));
        if let Some(bar) = button_bar(ctx, "dataFolder", arr(df, "actions")) {
            p.items.push(bar);
        }
        p.items.push(Item::line("", secondary_line(ctx, t(&df["note"]), 2)));
    }
}

/// The tabs of `home` with their badges, as spans (the selected one highlighted)
pub fn tabs(ctx: &Ctx, home: &Value, current: &str) -> Vec<Span<'static>> {
    let th = ctx.theme;
    let mut spans = Vec::new();
    // The open tab's group shows its tabs, the other group its name (plan B147); Tab goes on into it
    let group = arr(home, "tabs").iter().find(|t| s(t, "id") == current).map(|t| s(t, "group")).unwrap_or("run");
    let others: Vec<String> = arr(home, "groups")
        .iter()
        .filter(|g| s(g, "id") != group)
        .map(|g| {
            let dot = if g["attention"].as_bool() == Some(true) { " •" } else { "" };
            format!("{}{dot} ›", t(&g["title"]))
        })
        .collect();
    for tab in arr(home, "tabs").iter().filter(|t| s(t, "group") == group || s(t, "group").is_empty()) {
        let id = s(tab, "id");
        let style =
            if id == current { th.accent().add_modifier(Modifier::BOLD | Modifier::UNDERLINED) } else { th.text() };
        spans.push(Span::raw(" "));
        spans.extend(icon(ctx, s(tab, "icon"), style));
        spans.push(Span::styled(t(&tab["label"]), style));
        if let Some(bd) = tab.get("badge") {
            let tone = th.tone(tone_of(bd));
            let text = match s(bd, "kind") {
                "count" => format!(" {}", bd.get("count").and_then(Value::as_u64).unwrap_or(0)),
                "working" => format!(" {}", spinner(ctx.tick)),
                "failed" => format!(" {}", ctx.icons.get("failed")),
                _ => " •".to_string(),
            };
            spans.push(Span::styled(text, tone));
        }
        spans.push(Span::raw("  "));
    }
    for o in others {
        spans.push(Span::styled(format!("│ {o}  "), th.secondary()));
    }
    spans
}

/// The log lines of a job, as the console shows them
pub fn log_lines(ctx: &Ctx, logs: &HashMap<String, Vec<String>>, log: &LogBox) -> Vec<Line<'static>> {
    let mut lines: Vec<Line<'static>> = logs
        .get(&log.job)
        .map(|l| l.iter().map(|s| Line::from(Span::styled(s.clone(), ctx.theme.console()))).collect())
        .unwrap_or_default();
    if let Some(n) = &log.note {
        lines.push(Line::from(Span::styled(n.clone(), ctx.theme.secondary())));
    }
    lines
}

/// What a byte count says ("12.3 GB")
pub fn size(bytes: i64) -> String {
    bytes_text(bytes)
}
