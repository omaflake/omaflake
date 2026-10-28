// SPDX-License-Identifier: AGPL-3.0-or-later
//! The TUI's state and what keys do: which tab and page are open, the pages subscribed (`views.subscribe`), the
//! models as they come (`view.changed`), and an action's way from a key to the core – ask its values, confirm, send,
//! then reveal / open a terminal / go back (api.md §10.3–10.4). Everything that touches the terminal or the system
//! (a command in the terminal, opening a file) is an [`Effect`] the run loop carries out, so this module is tested
//! without a terminal.

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::Instant;

use chrono::Utc;
use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use serde_json::{json, Map, Value};

use crate::core::{Core, CoreError};
use crate::icons::IconSet;
use crate::model::{self, Ctx, Page};
use crate::paths;
use crate::text::{arr, b, fill, s, set_path, t, tr, trf};
use crate::theme::{Theme, Tone};

/// The tabs before `home` arrives
const TABS: [&str; 10] =
    ["vms", "images", "recipes", "activity", "recipeStudio", "modules", "looks", "credentials", "locales", "profiles"];

/// Keys every page has; a page's tools get the other lowercase letters, `home`'s actions uppercase ones
pub const RESERVED: &[char] = &['q', 'j', 'k', 'g', 'h', 'l', 'a', 'G'];

/// A page and the params it is asked with
#[derive(Debug, Clone, PartialEq)]
pub struct PageRef {
    pub page: String,
    pub params: Value,
}

impl PageRef {
    pub fn new(page: &str, params: Value) -> PageRef {
        let params = if params.is_object() { params } else { json!({}) };
        PageRef { page: page.into(), params }
    }

    fn key(&self) -> String {
        format!("{} {}", self.page, self.params)
    }
}

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

/// An action on its way: its params as they fill up
#[derive(Debug, Clone)]
pub struct Flow {
    pub action: Value,
    pub params: Value,
    /// The next ask to ask
    pub ask: usize,
    pub confirmed: bool,
}

/// Where a prompt's value goes
#[derive(Debug, Clone)]
pub enum Target {
    Ask(Flow),
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
    Confirm { flow: Flow, input: Option<String> },
    Choice { field: Value, index: usize },
    Menu { actions: Vec<Value>, index: usize },
    Filter { page: String, groups: Vec<Value>, values: Map<String, Value>, index: usize },
    Help,
}

/// What an answer is for
#[derive(Debug)]
enum Pending {
    Action(Flow),
    Check(Flow, String),
    Step,
    Log(String),
    Quiet,
}

pub struct App {
    pub core: Arc<dyn Core>,
    tx: Sender<Msg>,
    pub theme: Theme,
    pub icons: IconSet,
    pub via: String,
    pub tab: String,
    pub stack: Vec<PageRef>,
    filters: HashMap<String, Value>,
    models: Vec<(PageRef, Value)>,
    last_by_name: HashMap<String, Value>,
    pub home: Option<Value>,
    pub logs: HashMap<String, Vec<String>>,
    selected: HashMap<String, String>,
    pub scroll: HashMap<String, usize>,
    pub expanded: HashSet<String>,
    pub collapsed: HashSet<String>,
    pub modal: Option<Modal>,
    pub status: Option<(String, Tone, Instant)>,
    pub effects: VecDeque<Effect>,
    pub quit: bool,
    pending: HashMap<u64, Pending>,
    next_id: u64,
    subscribed: Vec<PageRef>,
    last_activity: Option<u64>,
    catalogs_asked: bool,
    /// The Looks tab looked into the catalogs once (plan B141)
    looks_asked: bool,
    pub tick: u64,
    /// Lines the list area showed last (page up/down)
    pub page_height: usize,
    /// How many lines each item took when drawn last (clicks)
    pub item_heights: Vec<usize>,
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
            filters: HashMap::new(),
            models: vec![],
            last_by_name: HashMap::new(),
            home: None,
            logs: HashMap::new(),
            selected: HashMap::new(),
            scroll: HashMap::new(),
            expanded: HashSet::new(),
            collapsed: HashSet::new(),
            modal: None,
            status: None,
            effects: VecDeque::new(),
            quit: false,
            pending: HashMap::new(),
            next_id: 1,
            subscribed: vec![],
            last_activity: None,
            catalogs_asked: false,
            looks_asked: false,
            tick: 0,
            page_height: 20,
            item_heights: vec![],
        }
    }

    /// The first requests: job logs as they come, then the pages
    pub fn start(&mut self) {
        self.request("events.subscribe", json!({"topics": ["jobs"]}), Pending::Quiet);
        self.resubscribe();
    }

    /// After a new connection: everything asked again
    pub fn reconnect(&mut self, core: Arc<dyn Core>, via: &str) {
        self.core = core;
        self.via = via.into();
        self.subscribed.clear();
        self.pending.clear();
        self.start();
    }

    // MARK: Requests

    fn request(&mut self, method: &str, params: Value, then: Pending) {
        let id = self.next_id;
        self.next_id += 1;
        self.pending.insert(id, then);
        let (core, tx, method) = (self.core.clone(), self.tx.clone(), method.to_string());
        std::thread::spawn(move || {
            let answer = core.call(&method, params);
            let _ = tx.send(Msg::Answer(id, answer));
        });
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

    fn tab_page(&self, tab: &str) -> PageRef {
        match self.filters.get(tab) {
            Some(f) => PageRef::new(tab, json!({"filter": f})),
            None => PageRef::new(tab, json!({})),
        }
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

    pub fn resubscribe(&mut self) {
        let wanted = self.wanted();
        if wanted == self.subscribed {
            return;
        }
        let pages: Vec<Value> = wanted.iter().map(|p| json!({"page": p.page, "params": p.params})).collect();
        self.subscribed = wanted;
        self.request("views.subscribe", json!({"pages": pages}), Pending::Quiet);
    }

    /// The model of a page: as asked, else the latest of that page (while the new one is on its way)
    pub fn model(&self, page: &PageRef) -> Option<&Value> {
        self.models.iter().find(|(p, _)| p == page).map(|(_, m)| m).or_else(|| self.last_by_name.get(&page.page))
    }

    pub fn ctx(&self) -> Ctx<'_> {
        Ctx {
            theme: &self.theme,
            icons: self.icons,
            expanded: &self.expanded,
            collapsed: &self.collapsed,
            now: Utc::now(),
            tick: self.tick,
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
        match msg {
            Msg::Event(method, params) => self.event(&method, params),
            Msg::Answer(id, answer) => {
                if let Some(p) = self.pending.remove(&id) {
                    self.answer(p, answer);
                }
            }
        }
    }

    fn event(&mut self, method: &str, params: Value) {
        match method {
            "view.changed" => {
                let page = PageRef::new(s(&params, "page"), params["params"].clone());
                let model = params["model"].clone();
                self.on_model(&page, &model);
                self.last_by_name.insert(page.page.clone(), model.clone());
                match self.models.iter_mut().find(|(p, _)| *p == page) {
                    Some(slot) => slot.1 = model,
                    None => self.models.push((page, model)),
                }
                // Keep only what is subscribed (and the page on top)
                let keep = self.wanted();
                self.models.retain(|(p, _)| keep.contains(p));
            }
            "job.log" => {
                let id = s(&params, "id").to_string();
                if let Some(lines) = self.logs.get_mut(&id) {
                    lines.extend(arr(&params, "lines").iter().map(t));
                    let over = lines.len().saturating_sub(5000);
                    lines.drain(..over);
                }
            }
            _ => {}
        }
    }

    fn on_model(&mut self, page: &PageRef, model: &Value) {
        match page.page.as_str() {
            "home" => {
                if self.home.is_none() {
                    if let Some(first) = model.get("firstTab").and_then(Value::as_str) {
                        self.tab = first.to_string();
                        self.resubscribe();
                    }
                }
                self.home = Some(model.clone());
            }
            "activity" => self.notify_activity(model),
            "looks" | "modules" if !b(model, "catalogsLoaded") && !b(model, "loading") && !self.looks_asked => {
                self.looks_asked = true;
                if let Some(refresh) = arr(model, "actions").iter().find(|a| s(a, "id") == "refresh").cloned() {
                    self.run_action(&refresh);
                }
            }
            "recipes" if !b(model, "catalogsLoaded") && !b(model, "loading") && !self.catalogs_asked => {
                // As the app: the tab looks into the remote catalogs once when it opens (api.md §10.5)
                self.catalogs_asked = true;
                if let Some(refresh) = arr(model, "actions").iter().find(|a| s(a, "id") == "refresh").cloned() {
                    self.run_action(&refresh);
                }
            }
            _ => {}
        }
        // A job's log (build, macBase: a LogRef): what the core keeps from `from` on, then `job.log`
        let log = match page.page.as_str() {
            "build" | "macBase" => model.get("log").and_then(model::LogBox::from_model),
            _ => None,
        };
        if let Some(log) = log.filter(|l| !self.logs.contains_key(&l.job)) {
            self.logs.insert(log.job.clone(), vec![]);
            self.request("jobs.log", json!({"id": log.job, "from": log.from}), Pending::Log(log.job.clone()));
        }
    }

    /// New activity lines go to the status line (a TUI's notifications, api.md §10.6). Seen or not is the UI's: it
    /// remembers the highest entry `id` it has shown (`last_activity`); entries above it are unseen. Ids are stable
    /// and rise; a cleared list keeps the mark. When the newest id in the list is lower than the mark, the server is
    /// a new one (its ids start at 1 again): the mark is forgotten, so its entries count as unseen. The first list
    /// after the start only sets the mark (nothing was shown before).
    fn notify_activity(&mut self, model: &Value) {
        let entries = arr(model, "entries");
        let newest = entries.iter().filter_map(|e| e.get("id").and_then(Value::as_u64)).max();
        let seen = match (self.last_activity, newest) {
            (Some(mark), Some(n)) if n < mark => Some(0),
            (mark, _) => mark,
        };
        self.last_activity = newest.or(seen);
        let Some(seen) = seen else { return };
        if let Some(e) = entries.iter().find(|e| e.get("id").and_then(Value::as_u64).is_some_and(|id| id > seen)) {
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

    fn answer(&mut self, pending: Pending, answer: Result<Value, CoreError>) {
        match (pending, answer) {
            (Pending::Action(flow), Ok(result)) => self.then(&flow, &result),
            (Pending::Check(flow, value), Ok(result)) => {
                if result.get("valid").and_then(Value::as_bool) == Some(false) {
                    // `reason` is a text (§10.2)
                    let reason = result.get("reason").map(t).unwrap_or_else(|| tr("Not accepted"));
                    if let Some(Modal::Prompt(p)) = &mut self.modal {
                        p.checking = false;
                        p.error = Some(reason);
                    }
                } else {
                    self.modal = None;
                    self.asked(flow, Value::String(value));
                }
            }
            (Pending::Check(_, _), Err(e)) => {
                if let Some(Modal::Prompt(p)) = &mut self.modal {
                    p.checking = false;
                    p.error = Some(e.message);
                }
            }
            (Pending::Step, Ok(model)) => match model.get("action").filter(|a| a.is_object()) {
                Some(a) if b(&model, "valid") => {
                    let a = a.clone();
                    self.run_action(&a)
                }
                _ => {
                    let reason = model.get("reason").map(t).unwrap_or_else(|| s(&model, "name").to_string());
                    self.set_status(reason, Tone::Danger)
                }
            },
            (Pending::Log(job), Ok(log)) => {
                let mut lines: Vec<String> = arr(&log, "lines").iter().map(t).collect();
                // Lines that came as job.log meanwhile follow
                if let Some(got) = self.logs.get(&job) {
                    lines.extend(got.iter().cloned());
                }
                self.logs.insert(job, lines);
            }
            (Pending::Quiet, Ok(_)) => {}
            (_, Err(e)) => self.set_status(e.message, Tone::Danger),
        }
    }

    // MARK: Actions

    /// A key or click asks for `action`
    pub fn run_action(&mut self, action: &Value) {
        if !b(action, "enabled") {
            let why = action.get("reason").map(t).unwrap_or_else(|| tr("Not possible now"));
            self.set_status(why, Tone::Warning);
            return;
        }
        let flow = Flow { action: action.clone(), params: action["params"].clone(), ask: 0, confirmed: false };
        self.go_on(flow);
    }

    /// The next step of an action: its next ask, its confirmation, or sending it
    fn go_on(&mut self, flow: Flow) {
        let asks = arr(&flow.action, "ask");
        if let Some(ask) = asks.get(flow.ask) {
            let kind = s(ask, "kind").to_string();
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
                error: None,
                first: None,
                checking: false,
                target: Target::Ask(flow),
            }));
            return;
        }
        if let (Some(c), false) = (flow.action.get("confirm").filter(|c| c.is_object()), flow.confirmed) {
            let input = c.get("input").map(|i| s(i, "default").to_string());
            self.modal = Some(Modal::Confirm { flow, input });
            return;
        }
        self.send(flow);
    }

    /// An ask was answered
    fn asked(&mut self, mut flow: Flow, value: Value) {
        let ask = &arr(&flow.action, "ask")[flow.ask];
        let param = s(ask, "param").to_string();
        set_path(&mut flow.params, &param, value);
        flow.ask += 1;
        self.go_on(flow);
    }

    fn send(&mut self, flow: Flow) {
        let method = s(&flow.action, "method").to_string();
        if let Some(verb) = method.strip_prefix("ui.") {
            self.verb(verb, flow);
        } else {
            let params = flow.params.clone();
            self.request(&method, params, Pending::Action(flow));
        }
    }

    /// What the UI does itself (api.md §10.4)
    fn verb(&mut self, verb: &str, flow: Flow) {
        let p = &flow.params;
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
            "step" => {
                let params = json!({"page": s(p, "page"), "params": p["params"]});
                self.request("views.get", params, Pending::Step);
            }
            "filter" => {
                let page = s(p, "page").to_string();
                match p.get("filter").filter(|f| !f.is_null()) {
                    Some(f) => self.filters.insert(page, f.clone()),
                    None => self.filters.remove(&page),
                };
                self.resubscribe();
            }
            "field" => {
                // `values.<valueParam>` of the form page `page` with `params`; its `ask` filled `value` already,
                // `value: null` puts the default back
                let value = p.get("value").cloned().unwrap_or(Value::Null);
                let target = PageRef::new(s(p, "page"), p["params"].clone());
                self.set_value(Some(&target), s(p, "valueParam"), value);
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
    fn then(&mut self, flow: &Flow, result: &Value) {
        match s(&flow.action, "then") {
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
            self.request(s(set, "method"), params, Pending::Quiet);
        } else if let Some(key) = f.get("valueParam").and_then(Value::as_str) {
            // A form field of the page shown now
            self.set_value(None, key, value);
        } else {
            self.set_status(tr("This value is not kept"), Tone::Warning);
        }
    }

    /// `values.<key>` of a form page: `target` (its params without `values`), else the page on top – then asked again
    fn set_value(&mut self, target: Option<&PageRef>, key: &str, value: Value) {
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

    pub fn home_actions(&self) -> Vec<Value> {
        self.home.as_ref().map(|h| arr(h, "actions").to_vec()).unwrap_or_default()
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
            KeyCode::Char('?') => self.modal = Some(Modal::Help),
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
            Modal::Help => {}
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
                    self.filters.remove(&page);
                    self.resubscribe();
                }
                KeyCode::Enter => {
                    if values.is_empty() {
                        self.filters.remove(&page);
                    } else {
                        self.filters.insert(page, Value::Object(values));
                    }
                    self.resubscribe();
                }
                _ => self.modal = Some(Modal::Filter { page, groups, values, index }),
            },
            Modal::Confirm { mut flow, input } => match key.code {
                KeyCode::Enter => {
                    if let (Some(text), Some(i)) = (&input, flow.action["confirm"].get("input")) {
                        set_path(&mut flow.params, s(i, "param"), json!(text));
                    }
                    flow.confirmed = true;
                    self.go_on(flow);
                }
                KeyCode::Esc => self.set_status(tr("Cancelled"), Tone::Normal),
                KeyCode::Char('y') if input.is_none() => {
                    flow.confirmed = true;
                    self.go_on(flow);
                }
                KeyCode::Char('n') if input.is_none() => self.set_status(tr("Cancelled"), Tone::Normal),
                KeyCode::Char(c) if input.is_some() => {
                    let mut text = input.unwrap_or_default();
                    text.push(c);
                    self.modal = Some(Modal::Confirm { flow, input: Some(text) });
                }
                KeyCode::Backspace if input.is_some() => {
                    let mut text = input.unwrap_or_default();
                    text.pop();
                    self.modal = Some(Modal::Confirm { flow, input: Some(text) });
                }
                _ => self.modal = Some(Modal::Confirm { flow, input }),
            },
            Modal::Prompt(p) => self.prompt_key(p, key),
        }
    }

    fn prompt_key(&mut self, mut p: Prompt, key: KeyEvent) {
        let is_path = matches!(p.kind.as_str(), "folder" | "file" | "package");
        match key.code {
            KeyCode::Esc => return self.set_status(tr("Cancelled"), Tone::Normal),
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
        self.modal = Some(Modal::Prompt(p));
    }

    fn submit(&mut self, mut p: Prompt) {
        let is_path = matches!(p.kind.as_str(), "folder" | "file" | "package");
        let value = if is_path { paths::normalize(&p.input) } else { p.input.clone() };
        if is_path {
            if let Err(e) = paths::fits(&p.kind, std::path::Path::new(&value)) {
                p.error = Some(e);
                self.modal = Some(Modal::Prompt(p));
                return;
            }
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
            Target::Ask(flow) => {
                let ask = arr(&flow.action, "ask")[flow.ask].clone();
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
                if let Some(check) = ask.get("check").filter(|c| c.is_object()) {
                    let mut params = check.get("params").filter(|p| p.is_object()).cloned().unwrap_or(json!({}));
                    set_path(&mut params, s(check, "param"), json!(value));
                    p.checking = true;
                    let method = s(check, "method").to_string();
                    self.modal = Some(Modal::Prompt(p));
                    self.request(&method, params, Pending::Check(flow, value));
                    return;
                }
                self.asked(flow, json!(value));
            }
        }
    }

    /// A confirmation's texts with the asked values filled in
    pub fn confirm_texts(flow: &Flow) -> (String, String, String, bool) {
        let c = &flow.action["confirm"];
        (fill(&c["title"], &flow.params), fill(&c["text"], &flow.params), t(&c["button"]), b(c, "destructive"))
    }
}

fn home_slash() -> String {
    std::env::var("HOME").map(|h| format!("{h}/")).unwrap_or_default()
}
