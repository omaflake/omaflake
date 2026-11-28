// SPDX-License-Identifier: AGPL-3.0-or-later
//! The TUI's state and what keys do: which tab and page are open, what is selected and typed, and how the open
//! question is asked in a terminal. What every interface of the page models does the same way – the pages subscribed
//! and their models, an action's way to the core with its asks, checks and confirmation, the logs, what is new in
//! the activity – is the SDK's [`Session`]; this module turns keys into what the user wants of it and carries its
//! effects out. Everything that touches the terminal or the system (a command in the terminal, opening a file) is an
//! [`Effect`] the run loop carries out, so this module is tested without a terminal.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::Arc;
use std::sync::mpsc::Sender;
use std::time::Instant;

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use omaflake_sdk::protocol::RpcError;
use omaflake_sdk::session::{self, FlowId, News, Notice, Question, Session};
pub use omaflake_sdk::views::PageRef;
use serde_json::{Map, Value, json};

use crate::core::{Core, CoreError};
use crate::icons::IconSet;
use crate::model::{self, Ctx, Page};
use crate::paths;
use crate::text::{arr, b, fill, s, set_path, t, tr, trf};
use crate::theme::{Theme, Tone};

/// The tabs before `home` arrives
const TABS: [&str; 17] = [
    "vms",
    "images",
    "activity",
    "networks",
    "volumes",
    "recipes",
    "recipeStudio",
    "keymapStudio",
    "lookStudio",
    "recipeCatalog",
    "modules",
    "looks",
    "keymaps",
    "sources",
    "credentials",
    "locales",
    "profiles",
];

/// Keys every page has; a page's tools get the other lowercase letters, `home`'s actions uppercase ones
pub const RESERVED: &[char] = &['q', 'j', 'k', 'g', 'h', 'l', 'a', 'G'];

/// What the TUI gets from its threads
#[derive(Debug)]
pub enum Msg {
    /// An event of the core
    Event(String, Value),
    /// The answer to request `id`
    Answer(u64, Result<Value, CoreError>),
}

/// What the run loop does for the app
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Leave the screen, run this in the terminal, come back
    Terminal(Command),
    /// Show in the file manager (or print the path)
    Reveal(String),
    /// Open with the system's default app (or print it)
    Open(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Command {
    Argv(Vec<String>),
    Shell(String),
}

/// Where a prompt's value goes
#[derive(Debug, Clone)]
pub enum Target {
    /// The open ask of an action (the session's question)
    Ask {
        flow: FlowId,
        index: usize,
        ask: Value,
    },
    Field(Value),
}

#[derive(Debug, Clone)]
pub struct Prompt {
    pub target: Target,
    /// text, password, folder, file, package, number
    pub kind: String,
    pub label: String,
    pub prompt: Option<String>,
    pub input: String,
    pub candidates: Vec<String>,
    pub error: Option<String>,
    /// A password's first entry, while it is asked again
    pub first: Option<String>,
    pub checking: bool,
}

#[derive(Debug, Clone)]
pub enum Modal {
    Prompt(Prompt),
    /// The open confirmation of an action (the session's question), for these params
    Confirm {
        flow: FlowId,
        action: Value,
        params: Value,
        input: Option<String>,
    },
    Choice {
        field: Value,
        index: usize,
    },
    Menu {
        actions: Vec<Value>,
        index: usize,
    },
    Filter {
        page: String,
        groups: Vec<Value>,
        values: Map<String, Value>,
        index: usize,
    },
    /// The keys; `top` = the lines left out at its top (scrolled with the arrows, plan B132)
    Help {
        top: usize,
    },
}

pub struct App {
    pub core: Arc<dyn Core>,
    tx: Sender<Msg>,
    pub theme: Theme,
    pub icons: IconSet,
    pub via: String,
    pub tab: String,
    pub stack: Vec<PageRef>,
    /// Pages, questions, logs, activity: what every interface keeps (the filters too)
    pub session: Session,
    /// The form values of a form shown as a tab (the studios), by tab
    tab_values: HashMap<String, Value>,
    /// Several values are being set at once (`ui.field` with `also`): one page set afterwards, not one each
    holding: bool,
    pub home: Option<Value>,
    selected: HashMap<String, String>,
    pub scroll: HashMap<String, usize>,
    pub expanded: HashSet<String>,
    pub collapsed: HashSet<String>,
    pub modal: Option<Modal>,
    pub status: Option<(String, Tone, Instant)>,
    pub effects: VecDeque<Effect>,
    pub quit: bool,
    catalogs_asked: bool,
    /// The Looks tab looked into the catalogs once (plan B141)
    looks_asked: bool,
    pub tick: u64,
    /// Lines the list area showed last (page up/down)
    pub page_height: usize,
    /// How many lines each item took when drawn last (clicks)
    pub item_heights: Vec<usize>,
    /// The most lines the help can leave out at its top when drawn last (0: it fits)
    pub help_max_top: usize,
}

impl App {
    pub fn new(core: Arc<dyn Core>, tx: Sender<Msg>, theme: Theme, icons: IconSet, via: &str) -> App {
        App {
            core,
            tx,
            theme,
            icons,
            via: via.into(),
            tab: "vms".into(),
            stack: vec![],
            session: Session::new(),
            tab_values: HashMap::new(),
            holding: false,
            home: None,
            selected: HashMap::new(),
            scroll: HashMap::new(),
            expanded: HashSet::new(),
            collapsed: HashSet::new(),
            modal: None,
            status: None,
            effects: VecDeque::new(),
            quit: false,
            catalogs_asked: false,
            looks_asked: false,
            tick: 0,
            page_height: 20,
            item_heights: vec![],
            help_max_top: 0,
        }
    }

    /// The connection is there: job logs as they come, then the pages
    pub fn start(&mut self) {
        let pages = self.wanted();
        let fx = self.session.show(pages);
        self.apply(fx);
        let fx = self.session.connected(&Value::Null);
        self.apply(fx);
    }

    /// After a new connection: nothing of the old one counts (its jobs, logs, answers), everything is asked again
    pub fn reconnect(&mut self, core: Arc<dyn Core>, via: &str) {
        let fx = self.session.disconnected();
        self.apply(fx);
        self.core = core;
        self.via = via.into();
        self.start();
    }

    // MARK: The session

    /// Carries out what the session asks for
    fn apply(&mut self, effects: Vec<session::Effect>) {
        for effect in effects {
            match effect {
                session::Effect::Send(request) => {
                    // A thread per request: the session sends in order what must not overtake each other
                    let (core, tx) = (self.core.clone(), self.tx.clone());
                    std::thread::spawn(move || {
                        let answer = core.call(&request.method, request.params);
                        let _ = tx.send(Msg::Answer(request.id, answer));
                    });
                }
                session::Effect::Model(page) => self.on_model(&page),
                session::Effect::Question => self.ask(),
                session::Effect::Activity(news) => self.notify_activity(&news),
                session::Effect::Ui { verb, params } => self.verb(&verb, &params),
                session::Effect::Done { action, result } => self.then(&action, &result),
                session::Effect::Notice(notice) => self.notice(notice),
                // The log is drawn from the session; jobs are rows of the pages
                session::Effect::Log(_) | session::Effect::Jobs | session::Effect::Answered { .. } => {}
            }
        }
    }

    fn notice(&mut self, notice: Notice) {
        match notice {
            Notice::Failed { error, .. } => self.set_status(error.message, Tone::Danger),
            Notice::Disabled { reason } => {
                let why = reason.as_ref().map(t).unwrap_or_else(|| tr("Not possible now"));
                self.set_status(why, Tone::Warning);
            }
            Notice::Refused { reason, name } => self.set_status(reason.as_ref().map(t).unwrap_or(name), Tone::Danger),
            Notice::Skipped { refused } => {
                if let Some(r) = refused.first() {
                    self.set_status(format!("{}: {}", r.value, refusal(r)), Tone::Warning);
                }
            }
            Notice::NotConnected { .. } => self.set_status(tr("The service ended – connecting again …"), Tone::Warning),
        }
    }

    /// The session's open question as this TUI's modal: a prompt for an ask, the question of a confirmation. A
    /// prompt that is open for the same ask keeps what was typed; without a question, its modal closes.
    fn ask(&mut self) {
        match self.session.question() {
            None => {
                if matches!(
                    &self.modal,
                    Some(Modal::Confirm { .. } | Modal::Prompt(Prompt { target: Target::Ask { .. }, .. }))
                ) {
                    self.modal = None;
                }
            }
            Some(Question::Ask { flow, ask, index, checking, refused, .. }) => {
                if let Some(Modal::Prompt(p)) = &mut self.modal
                    && matches!(&p.target, Target::Ask { flow: f, index: i, .. } if *f == flow && *i == index)
                {
                    p.checking = checking;
                    if let Some(r) = refused.first() {
                        p.error = Some(refusal(r));
                    }
                    return;
                }
                let kind = s(&ask, "kind").to_string();
                let input = match &kind[..] {
                    "password" => String::new(),
                    "folder" | "file" | "package" => {
                        ask.get("default").and_then(Value::as_str).map(str::to_string).unwrap_or_else(home_slash)
                    }
                    _ => ask.get("default").and_then(Value::as_str).unwrap_or_default().to_string(),
                };
                self.modal = Some(Modal::Prompt(Prompt {
                    label: t(&ask["label"]),
                    prompt: ask.get("prompt").map(t),
                    kind,
                    input,
                    candidates: vec![],
                    error: refused.first().map(refusal),
                    first: None,
                    checking,
                    target: Target::Ask { flow, index, ask },
                }));
            }
            Some(Question::Confirm { flow, action, confirm, params }) => {
                if matches!(&self.modal, Some(Modal::Confirm { flow: f, params: p, .. }) if *f == flow && *p == params)
                {
                    return;
                }
                let input = confirm.get("input").map(|i| s(i, "default").to_string());
                self.modal = Some(Modal::Confirm { flow, action, params, input });
            }
        }
    }

    pub fn set_status(&mut self, text: impl Into<String>, tone: Tone) {
        self.status = Some((text.into(), tone, Instant::now()));
    }

    // MARK: Pages

    /// The page shown now
    pub fn current(&self) -> PageRef {
        match self.stack.last() {
            Some(p) => p.clone(),
            None => self.tab_page(&self.tab),
        }
    }

    /// A tab's page: its filter, and – a form as a tab (the studios) – what the user chose in it
    fn tab_page(&self, tab: &str) -> PageRef {
        let mut params = json!({});
        if let Some(f) = self.session.filters.get(tab) {
            params["filter"] = f.clone();
        }
        if let Some(v) = self.tab_values.get(tab).filter(|v| v.as_object().is_some_and(|o| !o.is_empty())) {
            params["values"] = v.clone();
        }
        PageRef::new(tab, params)
    }

    pub fn tab_ids(&self) -> Vec<String> {
        match &self.home {
            Some(h) if !arr(h, "tabs").is_empty() => arr(h, "tabs").iter().map(|t| s(t, "id").to_string()).collect(),
            _ => TABS.iter().map(|s| s.to_string()).collect(),
        }
    }

    /// home, activity, the tab's page and the page on top
    fn wanted(&self) -> Vec<PageRef> {
        let mut pages = vec![PageRef::new("home", json!({})), PageRef::new("activity", json!({}))];
        for p in [self.tab_page(&self.tab), self.current()] {
            if !pages.contains(&p) {
                pages.push(p);
            }
        }
        pages
    }

    /// The pages shown now are the ones to follow
    pub fn resubscribe(&mut self) {
        if self.holding {
            return;
        }
        let pages = self.wanted();
        let fx = self.session.show(pages);
        self.apply(fx);
    }

    /// The model of a page: as asked, else the latest of that page (while the new one is on its way)
    pub fn model(&self, page: &PageRef) -> Option<&Value> {
        self.session.model_or_latest(page)
    }

    /// A job's log as far as it was read
    pub fn log(&self, job: &str) -> &[String] {
        self.session.log(job).unwrap_or_default()
    }

    pub fn ctx(&self) -> Ctx<'_> {
        Ctx {
            theme: &self.theme,
            icons: self.icons,
            expanded: &self.expanded,
            collapsed: &self.collapsed,
            now: omaflake_sdk::time::Date::now(),
            tick: self.tick,
            help: self.home.as_ref().and_then(|h| h["help"]["on"].as_bool()).unwrap_or(false),
        }
    }

    /// The page shown now, as items
    pub fn page(&self) -> Page {
        let cur = self.current();
        match self.model(&cur) {
            Some(m) => model::page(&self.ctx(), &cur.page, m),
            None => Page { title: cur.page.clone(), ..Page::default() },
        }
    }

    // MARK: Messages

    pub fn handle(&mut self, msg: Msg) {
        let fx = match msg {
            Msg::Event(method, params) => self.session.event(&method, &params),
            // (the error's message is in the TUI's language already)
            Msg::Answer(id, answer) => self.session.answer(
                id,
                answer.map_err(|e| RpcError::server(e.kind.as_deref().and_then(omaflake_sdk::codes::known), e.message)),
            ),
        };
        self.apply(fx);
    }

    /// What a new model brings along: the first tab, a look into the catalogs
    fn on_model(&mut self, page: &PageRef) {
        let Some(model) = self.session.model(page).cloned() else { return };
        match page.page.as_str() {
            "home" => {
                let first = self.home.is_none().then(|| model.get("firstTab").and_then(Value::as_str)).flatten();
                if let Some(first) = first {
                    self.tab = first.to_string();
                }
                self.home = Some(model.clone());
                if first.is_some() {
                    self.resubscribe();
                }
            }
            "looks" | "modules" | "keymaps"
                if !b(&model, "catalogsLoaded") && !b(&model, "loading") && !self.looks_asked =>
            {
                self.looks_asked = true;
                if let Some(refresh) = arr(&model, "actions").iter().find(|a| s(a, "id") == "refresh").cloned() {
                    self.run_action(&refresh);
                }
            }
            "recipes" | "recipeCatalog"
                if !b(&model, "catalogsLoaded") && !b(&model, "loading") && !self.catalogs_asked =>
            {
                // As the app: the tab looks into the remote catalogs once when it opens (api.md §10.5)
                self.catalogs_asked = true;
                if let Some(refresh) = arr(&model, "actions").iter().find(|a| s(a, "id") == "refresh").cloned() {
                    self.run_action(&refresh);
                }
            }
            _ => {}
        }
    }

    /// New activity lines go to the status line (a TUI's notifications, api.md §10.6): the newest entry not seen
    /// before. Which those are is the session's – the first list after the start is no news, a new server's list is.
    fn notify_activity(&mut self, news: &News) {
        if let Some(e) = news.fresh.first() {
            let tone = match s(e, "level") {
                "error" => Tone::Danger,
                "warning" => Tone::Warning,
                _ => Tone::Accent,
            };
            let text = match e.get("notification") {
                Some(n) => format!("{}: {}", t(&n["title"]), t(&n["body"])),
                None => t(&e["text"]),
            };
            self.set_status(text, tone);
        }
    }

    // MARK: Actions

    /// A key or click asks for `action`: the session asks what it needs (a prompt, a confirmation) and sends it
    pub fn run_action(&mut self, action: &Value) {
        let fx = self.session.start(action);
        self.apply(fx);
    }

    /// What the UI does itself (api.md §10.4)
    fn verb(&mut self, verb: &str, p: &Value) {
        match verb {
            "show" => {
                let page = s(p, "page");
                if page == "guide" {
                    // A guide of the docs: a terminal cannot show the bundled file (`path`, only inside omaflake.app)
                    // itself, so it opens its page on omaflake.dev – or prints it where there is no desktop
                    let g = &p["params"];
                    match g.get("url").and_then(Value::as_str) {
                        Some(url) => {
                            self.effects.push_back(Effect::Open(url.into()));
                        }
                        None => self.set_status(trf("%@: no page to open", &[t(&g["title"]).into()]), Tone::Warning),
                    }
                } else {
                    self.stack.push(PageRef::new(page, p["params"].clone()));
                    self.resubscribe();
                }
            }
            "back" => self.back(),
            "filter" => {
                self.session.filters.set(s(p, "page"), p.get("filter").cloned().unwrap_or(Value::Null));
                self.resubscribe();
            }
            "field" => {
                // `values.<valueParam>` of the form page `page` with `params`; its `ask` filled `value` already,
                // `value: null` puts the default back
                let value = p.get("value").cloned().unwrap_or(Value::Null);
                let target = PageRef::new(s(p, "page"), p["params"].clone());
                // `also`: more values of the same page at once (plan B147: a key and the mode after it)
                self.holding = true;
                for m in arr(p, "also") {
                    self.set_value(Some(&target), s(m, "valueParam"), m.get("value").cloned().unwrap_or(Value::Null));
                }
                self.set_value(Some(&target), s(p, "valueParam"), value);
                self.holding = false;
                self.resubscribe();
            }
            "reveal" => match p.get("path").and_then(Value::as_str) {
                Some(path) => self.effects.push_back(Effect::Reveal(path.into())),
                None => self.set_status(tr("Nothing to show yet"), Tone::Normal),
            },
            "open" => match p.get("path").or_else(|| p.get("url")).and_then(Value::as_str) {
                Some(target) => self.effects.push_back(Effect::Open(target.into())),
                None => self.set_status(tr("Nothing to open"), Tone::Normal),
            },
            "terminal" => self.effects.push_back(Effect::Terminal(Command::Shell(s(p, "line").into()))),
            "quit" => self.quit = true,
            other => self.set_status(trf("ui.%@ is not known to this omaf ui", &[other.into()]), Tone::Warning),
        }
    }

    /// The answer of a core method: what the action does with it (`then`)
    fn then(&mut self, action: &Value, result: &Value) {
        match s(action, "then") {
            "reveal" => match result.get("path").and_then(Value::as_str) {
                Some(path) => self.effects.push_back(Effect::Reveal(path.into())),
                None => self.set_status(tr("No path in the answer"), Tone::Warning),
            },
            "terminal" => {
                let argv: Vec<String> = arr(result, "argv").iter().map(t).collect();
                let cmd = if argv.is_empty() {
                    Command::Shell(s(result, "shellLine").to_string())
                } else {
                    Command::Argv(argv)
                };
                self.effects.push_back(Effect::Terminal(cmd));
            }
            "back" => self.back(),
            _ => {}
        }
    }

    pub fn back(&mut self) {
        if self.stack.pop().is_some() {
            self.resubscribe();
        }
    }

    // MARK: Fields

    /// A form field changed: stored at once (`set`), or into the page's params (`values`) and asked again
    fn change_field(&mut self, f: &Value, value: Value) {
        if !b(f, "enabled") {
            let why = f.get("reason").map(t).unwrap_or_else(|| tr("Not possible now"));
            self.set_status(why, Tone::Warning);
            return;
        }
        if let Some(set) = f.get("set").filter(|s| s.is_object()) {
            let mut params = set.get("params").filter(|p| p.is_object()).cloned().unwrap_or_else(|| json!({}));
            set_path(&mut params, s(set, "param"), value);
            if s(set, "method").starts_with("ui.") {
                // The UI's own verb (a catalog tab's search goes into its filter, plan B148)
                self.run_action(&json!({"method": s(set, "method"), "params": params, "enabled": true}));
            } else {
                // One after the other, so that the last value set is the one that stays
                let fx = self.session.set(s(set, "method"), params);
                self.apply(fx);
            }
        } else if let Some(key) = f.get("valueParam").and_then(Value::as_str) {
            // A form field of the page shown now
            self.set_value(None, key, value);
        } else {
            self.set_status(tr("This value is not kept"), Tone::Warning);
        }
    }

    /// `values.<key>` of a form page: `target` (its params without `values`), else the page on top – then asked again
    fn set_value(&mut self, target: Option<&PageRef>, key: &str, value: Value) {
        // On a tab (the studios): its values are the tab's own – also for an action of the page (`ui.field`) while no
        // page of that name is open on top
        let on_tab = match target {
            Some(t) => t.page == self.tab && !self.stack.iter().any(|p| p.page == t.page),
            None => self.stack.is_empty(),
        };
        if on_tab && !key.is_empty() {
            let values = self.tab_values.entry(self.tab.clone()).or_insert_with(|| json!({}));
            if let Some(obj) = values.as_object_mut() {
                if value.is_null() {
                    obj.remove(key);
                } else {
                    obj.insert(key.into(), value);
                }
            }
            self.resubscribe();
            return;
        }
        let without_values = |p: &PageRef| {
            let mut params = p.params.clone();
            if let Some(o) = params.as_object_mut() {
                o.remove("values");
            }
            PageRef::new(&p.page, params)
        };
        let slot = match target {
            Some(t) => {
                let t = without_values(t);
                self.stack.iter_mut().rev().find(|p| without_values(p) == t)
            }
            None => self.stack.last_mut(),
        };
        let (Some(page), false) = (slot, key.is_empty()) else {
            self.set_status(tr("No form here"), Tone::Warning);
            return;
        };
        let values = page.params.as_object_mut().expect("params are an object").entry("values").or_insert(json!({}));
        if !values.is_object() {
            *values = json!({});
        }
        let obj = values.as_object_mut().expect("made an object");
        if value.is_null() {
            obj.remove(key);
        } else {
            obj.insert(key.into(), value);
        }
        self.resubscribe();
    }

    fn edit_field(&mut self, f: &Value) {
        if !b(f, "enabled") {
            return self.change_field(f, Value::Null);
        }
        let value = f.get("value").cloned().unwrap_or(Value::Null);
        match s(f, "kind") {
            "toggle" => self.change_field(f, json!(!value.as_bool().unwrap_or(false))),
            "choice" => {
                let index = arr(f, "choices").iter().position(|c| c["value"] == value).unwrap_or(0);
                self.modal = Some(Modal::Choice { field: f.clone(), index });
            }
            kind => {
                let (kind, input) = match kind {
                    "path" => {
                        let k = f.get("ask").map(|a| s(a, "kind")).unwrap_or("folder").to_string();
                        (k, value.as_str().map(str::to_string).unwrap_or_else(home_slash))
                    }
                    "number" => ("number".into(), t(&value)),
                    _ => ("text".into(), value.as_str().unwrap_or_default().to_string()),
                };
                self.modal = Some(Modal::Prompt(Prompt {
                    label: t(&f["label"]),
                    prompt: f.get("help").or_else(|| f.get("ask").and_then(|a| a.get("prompt"))).map(t),
                    kind,
                    input,
                    candidates: vec![],
                    error: None,
                    first: None,
                    checking: false,
                    target: Target::Field(f.clone()),
                }));
            }
        }
    }

    /// ← / → on a field: a number one step, a choice the next
    fn step_field(&mut self, f: &Value, by: i64) {
        let value = f.get("value").cloned().unwrap_or(Value::Null);
        match s(f, "kind") {
            "number" => {
                let step = f.get("step").and_then(Value::as_i64).unwrap_or(1);
                let n = value.as_i64().unwrap_or(0) + by * step;
                let min = f.get("min").and_then(Value::as_i64).unwrap_or(i64::MIN);
                let max = f.get("max").and_then(Value::as_i64).unwrap_or(i64::MAX);
                self.change_field(f, json!(n.clamp(min, max)));
            }
            "choice" => {
                let choices = arr(f, "choices");
                if choices.is_empty() {
                    return;
                }
                let i = choices.iter().position(|c| c["value"] == value).unwrap_or(0) as i64;
                let next = (i + by).rem_euclid(choices.len() as i64) as usize;
                self.change_field(f, choices[next]["value"].clone());
            }
            "toggle" => self.change_field(f, json!(by > 0)),
            _ => {}
        }
    }

    // MARK: Selection

    fn page_key(&self) -> String {
        self.current().key()
    }

    /// The selected item's index in `page.items`
    pub fn selected_index(&self, page: &Page) -> Option<usize> {
        let key = self.selected.get(&self.page_key());
        key.and_then(|k| page.items.iter().position(|i| i.selectable && &i.key == k))
            .or_else(|| page.items.iter().position(|i| i.selectable))
    }

    fn select(&mut self, page: &Page, index: usize) {
        if let Some(item) = page.items.get(index) {
            self.selected.insert(self.page_key(), item.key.clone());
        }
    }

    fn move_selection(&mut self, page: &Page, by: i64) {
        let selectable: Vec<usize> =
            page.items.iter().enumerate().filter(|(_, i)| i.selectable).map(|(n, _)| n).collect();
        if selectable.is_empty() {
            return;
        }
        let at = self.selected_index(page).and_then(|i| selectable.iter().position(|&n| n == i)).unwrap_or(0) as i64;
        let to = (at + by).clamp(0, selectable.len() as i64 - 1) as usize;
        self.select(page, selectable[to]);
    }

    /// Selects the item a click hit
    pub fn click(&mut self, index: usize) {
        let page = self.page();
        if page.items.get(index).is_some_and(|i| i.selectable) {
            self.select(&page, index);
        }
    }

    // MARK: Keys

    /// Letters for actions: the first letter of its label that is free (`upper`: uppercase ones)
    pub fn letters(actions: &[Value], upper: bool, taken: &[char]) -> Vec<Option<char>> {
        let mut used: Vec<char> = taken.to_vec();
        actions
            .iter()
            .map(|a| {
                if s(a, "method") == "ui.quit" {
                    return Some('q');
                }
                // The letter is one of what is written: the short label where there is one
                let label = t(a.get("shortLabel").unwrap_or(&a["label"]));
                let c = label
                    .chars()
                    .filter(|c| c.is_ascii_alphabetic())
                    .map(|c| if upper { c.to_ascii_uppercase() } else { c.to_ascii_lowercase() })
                    .find(|c| !used.contains(c));
                if let Some(c) = c {
                    used.push(c);
                }
                c
            })
            .collect()
    }

    /// The header's actions, the help switch first (plan B148: a letter of its own like the others)
    pub fn home_actions(&self) -> Vec<Value> {
        let Some(h) = self.home.as_ref() else { return vec![] };
        let mut out: Vec<Value> = h.get("help").filter(|a| a.is_object()).cloned().into_iter().collect();
        out.extend(arr(h, "actions").iter().cloned());
        out
    }

    pub fn key(&mut self, key: KeyEvent) {
        if key.modifiers.contains(KeyModifiers::CONTROL) && matches!(key.code, KeyCode::Char('c')) {
            self.quit = true;
            return;
        }
        if self.modal.is_some() {
            return self.modal_key(key);
        }
        let page = self.page();
        let sel = self.selected_index(&page);
        let item = sel.and_then(|i| page.items.get(i)).cloned();
        match key.code {
            KeyCode::Char('q') => match self.home_actions().into_iter().find(|a| s(a, "method") == "ui.quit") {
                Some(quit) => self.run_action(&quit),
                None => self.quit = true,
            },
            KeyCode::Char('?') => self.modal = Some(Modal::Help { top: 0 }),
            KeyCode::Char('/') => match &page.filter {
                Some(f) => {
                    let mut values = Map::new();
                    for g in arr(f, "groups") {
                        if !g["value"].is_null() {
                            values.insert(s(g, "id").into(), g["value"].clone());
                        }
                    }
                    self.modal = Some(Modal::Filter {
                        page: self.current().page,
                        groups: arr(f, "groups").to_vec(),
                        values,
                        index: 0,
                    });
                }
                None => self.set_status(tr("This page has no filter"), Tone::Normal),
            },
            KeyCode::Down | KeyCode::Char('j') => self.move_selection(&page, 1),
            KeyCode::Up | KeyCode::Char('k') => self.move_selection(&page, -1),
            KeyCode::PageDown => self.move_selection(&page, (self.page_height / 2).max(1) as i64),
            KeyCode::PageUp => self.move_selection(&page, -((self.page_height / 2).max(1) as i64)),
            KeyCode::Home | KeyCode::Char('g') => self.move_selection(&page, -100_000),
            KeyCode::End | KeyCode::Char('G') => self.move_selection(&page, 100_000),
            KeyCode::Tab | KeyCode::BackTab => {
                let tabs = self.tab_ids();
                let at = tabs.iter().position(|t| *t == self.tab).unwrap_or(0) as i64;
                let by = if key.code == KeyCode::Tab { 1 } else { -1 };
                self.tab = tabs[(at + by).rem_euclid(tabs.len() as i64) as usize].clone();
                self.stack.clear();
                self.resubscribe();
            }
            KeyCode::Esc | KeyCode::Backspace => self.back(),
            KeyCode::Enter => match item {
                Some(i) if i.field.is_some() => self.edit_field(i.field.as_ref().expect("checked")),
                Some(i) => match i.default {
                    Some(d) => self.run_action(&i.actions[d]),
                    None if i.expandable => self.toggle_open(&i.key, None),
                    None if !i.actions.is_empty() => self.modal = Some(Modal::Menu { actions: i.actions, index: 0 }),
                    None => {}
                },
                None => {}
            },
            KeyCode::Char(' ') => match item {
                Some(i) if i.field.is_some() => self.edit_field(i.field.as_ref().expect("checked")),
                Some(i) if i.expandable => self.toggle_open(&i.key, None),
                _ => {}
            },
            KeyCode::Right | KeyCode::Char('l') => match item {
                Some(i) if i.field.is_some() => self.step_field(i.field.as_ref().expect("checked"), 1),
                Some(i) if i.expandable => self.toggle_open(&i.key, Some(true)),
                _ => {}
            },
            KeyCode::Left | KeyCode::Char('h') => match item {
                Some(i) if i.field.is_some() => self.step_field(i.field.as_ref().expect("checked"), -1),
                Some(i) if i.expandable => self.toggle_open(&i.key, Some(false)),
                _ => {}
            },
            KeyCode::Char('a') => match item {
                Some(i) if !i.actions.is_empty() => self.modal = Some(Modal::Menu { actions: i.actions, index: 0 }),
                _ => self.set_status(tr("No actions here"), Tone::Normal),
            },
            KeyCode::Char(c @ '1'..='9') => {
                let n = c as usize - '1' as usize;
                match item.and_then(|i| i.actions.get(n).cloned()) {
                    Some(a) => self.run_action(&a),
                    None => self.set_status(trf("No action %@ here", &[c.to_string().into()]), Tone::Normal),
                }
            }
            KeyCode::Char(c) => {
                let home = self.home_actions();
                let home_letters = Self::letters(&home, true, RESERVED);
                if let Some(i) = home_letters.iter().position(|l| *l == Some(c)) {
                    return self.run_action(&home[i]);
                }
                let tools = Self::letters(&page.tools, false, RESERVED);
                if let Some(i) = tools.iter().position(|l| *l == Some(c)) {
                    self.run_action(&page.tools[i]);
                }
            }
            _ => {}
        }
    }

    /// Opens or closes a row's details; `to` None flips it. Rows the model opens itself (the Doctor's items with a
    /// problem) are closed through `collapsed`.
    fn toggle_open(&mut self, key: &str, to: Option<bool>) {
        let page = self.page();
        let open_now = page.items.iter().find(|i| i.key == key).is_some_and(|i| i.open);
        let open = to.unwrap_or(!open_now);
        if open {
            self.expanded.insert(key.into());
            self.collapsed.remove(key);
        } else {
            self.expanded.remove(key);
            self.collapsed.insert(key.into());
        }
    }

    fn modal_key(&mut self, key: KeyEvent) {
        let Some(modal) = self.modal.take() else { return };
        match modal {
            // The help scrolls with the arrows, j/k, the page keys, Home and End; any other key closes it
            Modal::Help { top } => {
                let max = self.help_max_top;
                let top = top.min(max);
                let to = match key.code {
                    KeyCode::Down | KeyCode::Char('j') => Some((top + 1).min(max)),
                    KeyCode::Up | KeyCode::Char('k') => Some(top.saturating_sub(1)),
                    KeyCode::PageDown | KeyCode::Char(' ') => Some((top + 10).min(max)),
                    KeyCode::PageUp => Some(top.saturating_sub(10)),
                    KeyCode::Home | KeyCode::Char('g') => Some(0),
                    KeyCode::End | KeyCode::Char('G') => Some(max),
                    _ => None,
                };
                if let Some(top) = to {
                    self.modal = Some(Modal::Help { top });
                }
            }
            Modal::Menu { actions, index } => match key.code {
                KeyCode::Esc => {}
                KeyCode::Down | KeyCode::Char('j') => {
                    self.modal = Some(Modal::Menu { index: (index + 1).min(actions.len() - 1), actions })
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.modal = Some(Modal::Menu { index: index.saturating_sub(1), actions })
                }
                KeyCode::Enter => self.run_action(&actions[index]),
                KeyCode::Char(c @ '1'..='9') => {
                    if let Some(a) = actions.get(c as usize - '1' as usize) {
                        self.run_action(a)
                    }
                }
                _ => self.modal = Some(Modal::Menu { actions, index }),
            },
            Modal::Choice { field, index } => {
                let n = arr(&field, "choices").len();
                match key.code {
                    KeyCode::Esc => {}
                    KeyCode::Down | KeyCode::Char('j') => {
                        self.modal = Some(Modal::Choice { index: (index + 1).min(n.saturating_sub(1)), field })
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        self.modal = Some(Modal::Choice { index: index.saturating_sub(1), field })
                    }
                    KeyCode::Enter => {
                        let v = arr(&field, "choices").get(index).map(|c| c["value"].clone()).unwrap_or(Value::Null);
                        self.change_field(&field, v);
                    }
                    _ => self.modal = Some(Modal::Choice { field, index }),
                }
            }
            Modal::Filter { page, groups, mut values, index } => match key.code {
                KeyCode::Esc => {}
                KeyCode::Down | KeyCode::Char('j') => {
                    self.modal = Some(Modal::Filter {
                        index: (index + 1).min(groups.len().saturating_sub(1)),
                        page,
                        groups,
                        values,
                    })
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    self.modal = Some(Modal::Filter { index: index.saturating_sub(1), page, groups, values })
                }
                KeyCode::Left | KeyCode::Right | KeyCode::Char(' ') | KeyCode::Char('h') | KeyCode::Char('l') => {
                    if let Some(g) = groups.get(index) {
                        let id = s(g, "id").to_string();
                        let now = values.get(&id).cloned().unwrap_or(Value::Null);
                        let next = if s(g, "kind") == "toggle" {
                            json!(!now.as_bool().unwrap_or(false))
                        } else {
                            let opts = arr(g, "options");
                            let by = if matches!(key.code, KeyCode::Left | KeyCode::Char('h')) { -1 } else { 1 };
                            let at = opts.iter().position(|o| o["value"] == now).unwrap_or(0) as i64;
                            opts.get((at + by).rem_euclid(opts.len().max(1) as i64) as usize)
                                .map(|o| o["value"].clone())
                                .unwrap_or(Value::Null)
                        };
                        if next.is_null() || next == json!(false) {
                            values.remove(&id);
                        } else {
                            values.insert(id, next);
                        }
                    }
                    self.modal = Some(Modal::Filter { page, groups, values, index });
                }
                KeyCode::Char('r') | KeyCode::Backspace => {
                    self.session.filters.set(&page, Value::Null);
                    self.resubscribe();
                }
                KeyCode::Enter => {
                    self.session.filters.set(&page, Value::Object(values));
                    self.resubscribe();
                }
                _ => self.modal = Some(Modal::Filter { page, groups, values, index }),
            },
            Modal::Confirm { flow, action, params, input } => match key.code {
                // Enter opened this question (the actions menu): a second Enter, or the key held down, must not
                // delete anything – what destroys is confirmed with y, or with Enter after the text asked for
                KeyCode::Enter if input.is_none() && Self::confirm_texts(&action, &params).3 => {
                    self.modal = Some(Modal::Confirm { flow, action, params, input })
                }
                KeyCode::Enter => self.confirm(flow, input),
                KeyCode::Esc => self.cancel(flow),
                KeyCode::Char('y') if input.is_none() => self.confirm(flow, None),
                KeyCode::Char('n') if input.is_none() => self.cancel(flow),
                KeyCode::Char(c) if input.is_some() => {
                    let mut text = input.unwrap_or_default();
                    text.push(c);
                    self.modal = Some(Modal::Confirm { flow, action, params, input: Some(text) });
                }
                KeyCode::Backspace if input.is_some() => {
                    let mut text = input.unwrap_or_default();
                    text.pop();
                    self.modal = Some(Modal::Confirm { flow, action, params, input: Some(text) });
                }
                _ => self.modal = Some(Modal::Confirm { flow, action, params, input }),
            },
            Modal::Prompt(p) => self.prompt_key(p, key),
        }
    }

    /// Yes to the open confirmation: the session sends the action (with the text the question asked for)
    fn confirm(&mut self, flow: FlowId, input: Option<String>) {
        let fx = self.session.confirm(flow, input);
        self.apply(fx);
    }

    /// The user gives the action up: whatever answer still comes for it, nothing more is sent
    fn cancel(&mut self, flow: FlowId) {
        let fx = self.session.cancel(flow);
        self.apply(fx);
        self.set_status(tr("Cancelled"), Tone::Normal);
    }

    fn prompt_key(&mut self, mut p: Prompt, key: KeyEvent) {
        let is_path = matches!(p.kind.as_str(), "folder" | "file" | "package");
        let before = p.input.clone();
        match key.code {
            KeyCode::Esc => {
                return match p.target {
                    Target::Ask { flow, .. } => self.cancel(flow),
                    Target::Field(_) => self.set_status(tr("Cancelled"), Tone::Normal),
                };
            }
            KeyCode::Tab if is_path => {
                let (done, cands) = paths::complete(&p.input, p.kind == "folder");
                p.input = done;
                p.candidates = cands;
            }
            KeyCode::Backspace => {
                p.input.pop();
                p.error = None;
            }
            KeyCode::Char('u') if key.modifiers.contains(KeyModifiers::CONTROL) => p.input.clear(),
            KeyCode::Char(c) => {
                p.input.push(c);
                p.error = None;
            }
            KeyCode::Enter if !p.checking => return self.submit(p),
            _ => {}
        }
        // Typed on while the core checks the value before: that check's answer is not for this text any more
        let retract = match &p.target {
            Target::Ask { flow, .. } if p.checking && p.input != before => Some(*flow),
            _ => None,
        };
        self.modal = Some(Modal::Prompt(p));
        if let Some(flow) = retract {
            let fx = self.session.retract(flow);
            self.apply(fx);
        }
    }

    fn submit(&mut self, mut p: Prompt) {
        let is_path = matches!(p.kind.as_str(), "folder" | "file" | "package");
        let value = if is_path { paths::normalize(&p.input) } else { p.input.clone() };
        if is_path && let Err(e) = paths::fits(&p.kind, std::path::Path::new(&value)) {
            p.error = Some(e);
            self.modal = Some(Modal::Prompt(p));
            return;
        }
        match p.target.clone() {
            Target::Field(f) => {
                let v = match p.kind.as_str() {
                    "number" => match value.trim().parse::<i64>() {
                        Ok(n) => json!(n),
                        Err(_) => {
                            p.error = Some(tr("A whole number"));
                            self.modal = Some(Modal::Prompt(p));
                            return;
                        }
                    },
                    _ if value.is_empty() => Value::Null,
                    _ => json!(value),
                };
                self.change_field(&f, v);
            }
            Target::Ask { flow, ask, .. } => {
                if p.kind == "password" && b(&ask, "repeat") {
                    match p.first.take() {
                        None => {
                            p.first = Some(value);
                            p.input.clear();
                            p.label = format!("{} (again)", p.label);
                            self.modal = Some(Modal::Prompt(p));
                            return;
                        }
                        Some(first) if first != value => {
                            p.error = Some(tr("The two entries differ – type it again"));
                            p.input.clear();
                            self.modal = Some(Modal::Prompt(p));
                            return;
                        }
                        Some(_) => {}
                    }
                }
                // The prompt stays while the session has the core check the value (`ask.check`); it closes, or the
                // next question opens, with the session's question
                p.error = None;
                self.modal = Some(Modal::Prompt(p));
                let fx = self.session.reply(flow, vec![value]);
                self.apply(fx);
            }
        }
    }

    /// A confirmation's texts with the asked values filled in
    pub fn confirm_texts(action: &Value, params: &Value) -> (String, String, String, bool) {
        let c = &action["confirm"];
        (fill(&c["title"], params), fill(&c["text"], params), t(&c["button"]), b(c, "destructive"))
    }
}

/// Why a value was not taken: the check's reason, or what went wrong with the check
fn refusal(r: &session::Refusal) -> String {
    match (&r.reason, &r.error) {
        (Some(reason), _) => t(reason),
        (None, Some(e)) => e.message.clone(),
        (None, None) => tr("Not accepted"),
    }
}

fn home_slash() -> String {
    std::env::var("HOME").map(|h| format!("{h}/")).unwrap_or_default()
}
