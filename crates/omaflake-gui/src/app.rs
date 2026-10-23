// SPDX-License-Identifier: AGPL-3.0-or-later
//! The app's state and what it does with messages. It keeps only what api.md §10 leaves to a UI: which tab and page
//! are open, which rows are expanded, the filters it asked with, what the user types, whether the activity was seen.
//! Everything shown comes from the core's page models (`view.changed`); every action is sent as the model names it.

use std::collections::{BTreeMap, HashMap, HashSet, VecDeque};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::widget::image;
use iced::{window, Size, Subscription, Task};
use omaflake_core::api::types::JobDto;
use omaflake_core::api::views::activity::ActivityPage;
use omaflake_core::api::views::build::BuildPage;
use omaflake_core::api::views::common::{set_param, Action, Ask, AskKind, Confirm, Field, Then};
use omaflake_core::api::views::doctor::{DoctorPage, MacBasePage};
use omaflake_core::api::views::home::HomePage;
use omaflake_core::api::views::images::ImagesPage;
use omaflake_core::api::views::import::ImportPage;
use omaflake_core::api::views::new_vm::NewVmPage;
use omaflake_core::api::views::recipes::RecipesPage;
use omaflake_core::api::views::settings::SettingsPage;
use omaflake_core::api::views::vms::VmsPage;
use omaflake_core::api::views::PageRef;
use serde_json::{json, Map, Value};

use crate::backend::{self, CallError, Handle};
use crate::prefs::Prefs;
use crate::text::{lookup, s};
use crate::{system, tray};

/// A page's model as the core sent it
#[derive(Debug, Clone)]
pub enum Model {
    Home(Box<HomePage>),
    Vms(Box<VmsPage>),
    Images(Box<ImagesPage>),
    Recipes(Box<RecipesPage>),
    NewVm(Box<NewVmPage>),
    Build(Box<BuildPage>),
    Doctor(Box<DoctorPage>),
    MacBase(Box<MacBasePage>),
    Settings(Box<SettingsPage>),
    Activity(Box<ActivityPage>),
    /// A page that cannot be built (`model: {error}`) or that this app cannot read
    Error(String),
}

/// A page's model from its JSON
pub fn decode(page: &str, v: Value) -> Model {
    if let Some(e) = v.get("error") {
        return Model::Error(e["message"].as_str().unwrap_or("This page cannot be shown").to_string());
    }
    fn de<T: serde::de::DeserializeOwned>(v: Value, f: fn(Box<T>) -> Model) -> Model {
        match serde_json::from_value::<T>(v) {
            Ok(t) => f(Box::new(t)),
            Err(e) => Model::Error(format!("This app cannot read the page: {e}")),
        }
    }
    match page {
        "home" => de(v, Model::Home),
        "vms" => de(v, Model::Vms),
        "images" => de(v, Model::Images),
        "recipes" => de(v, Model::Recipes),
        "newVm" => de(v, Model::NewVm),
        "build" => de(v, Model::Build),
        "doctor" => de(v, Model::Doctor),
        "macBase" => de(v, Model::MacBase),
        "settings" => de(v, Model::Settings),
        "activity" => de(v, Model::Activity),
        other => Model::Error(format!("Unknown page {other}")),
    }
}

pub fn page_ref(page: &str, params: Value) -> PageRef {
    PageRef { page: page.into(), params: if params.is_null() { json!({}) } else { params } }
}

/// What an answer was for
#[derive(Debug, Clone)]
pub enum Purpose {
    /// An action's method; `then` what to do with its result
    Action { then: Option<Then> },
    /// `ui.step`: the page whose action runs now
    Step,
    /// A log's lines (`jobs.log`)
    Log(String),
    /// The core's settings (notifications on?)
    Settings,
    /// Nothing to do but report an error
    Quiet,
}

#[derive(Debug, Clone)]
pub enum Message {
    Core(backend::Event),
    #[cfg(target_os = "linux")]
    Tray(tray::Event),
    Answer(Purpose, Result<Value, CallError>),
    Tab(String),
    Expand(String),
    FilterMenu(Option<String>),
    SetFilter {
        page: String,
        group: String,
        value: Value,
    },
    Run(Box<Action>, Option<String>),
    /// A form field's value (newVm, macBase, the new source)
    FormSet {
        page: String,
        field: String,
        value: Value,
    },
    /// Choosing a form field's path (its `ask`)
    FormChoose {
        page: String,
        field: Box<Field>,
    },
    /// Typing in a Settings field that is stored on submit
    Edit {
        field: String,
        text: String,
    },
    /// A Settings field's value stored (`set`)
    Store(Box<Field>, Value),
    ModalInput(String),
    ModalRepeat(String),
    ModalOk,
    ModalCancel,
    Picked(Option<Vec<PathBuf>>),
    Checked(Result<Vec<String>, String>),
    Back,
    Tick(Instant),
    Focus(bool),
    Resized(Size),

    CloseRequested(window::Id),
    WindowOpened(window::Id),
    SetTray(bool),
    DismissToast,
}

/// A question on top of the window
#[derive(Debug, Clone)]
pub enum Modal {
    Confirm { confirm: Confirm, params: Value, input: Option<String> },
    Ask { ask: Ask, value: String, repeat: String, error: Option<String> },
}

/// An action on its way: asking, confirming, sending (api.md §10.3: ask, then confirm, then send)
#[derive(Debug, Clone)]
pub struct Flow {
    pub action: Action,
    pub params: Value,
    pub asks: VecDeque<Ask>,
    /// The params sets to send (several when an `ask` of `multiple` got several values)
    pub queue: VecDeque<Value>,
    /// The ask a file dialog is open for
    pub picking: Option<Ask>,
}

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: Instant,
}

/// A job's log as far as it was read
#[derive(Debug, Clone, Default)]
pub struct Log {
    pub lines: Vec<String>,
    pub loaded: bool,
}

pub struct App {
    pub prefs: Prefs,
    pub window: Option<window::Id>,
    pub focused: bool,
    pub size: Size,
    pub glow: Option<(Size, image::Handle)>,
    pub core: Option<Handle>,
    pub status: Option<String>,
    pub init: Value,
    pub models: HashMap<String, Model>,
    pub tab: String,
    pub tab_chosen: bool,
    pub stack: Vec<PageRef>,
    pub filters: HashMap<String, Value>,
    pub filter_menu: Option<String>,
    pub expanded: HashSet<String>,
    /// Rows whose first state the model gives (Doctor items): what the user changed
    pub toggled: HashSet<String>,
    pub forms: HashMap<String, Map<String, Value>>,
    pub edits: HashMap<String, String>,
    pub logs: HashMap<String, Log>,
    pub jobs: BTreeMap<String, JobDto>,
    pub flow: Option<Flow>,
    pub modal: Option<Modal>,
    pub toast: Option<Toast>,
    pub seen_activity: u64,
    pub notified: Option<u64>,
    pub notifications: bool,
    pub catalogs_asked: bool,
    pub pulse: bool,
    pub subscribed: Vec<PageRef>,
    /// Actions waiting for the one on its way (the imports of several packages, each with its question)
    pub pending: VecDeque<Action>,
    /// Drawing fixture models, without a core (`OMAFLAKE_DEMO_VIEWS`)
    pub demo: bool,
    #[cfg(target_os = "linux")]
    pub tray: Option<tray::TrayHandle>,
}

pub const WINDOW_SIZE: Size = Size { width: 500.0, height: 600.0 };

fn window_settings() -> window::Settings {
    window::Settings {
        size: Size::new(560.0, 680.0),
        min_size: Some(WINDOW_SIZE),
        exit_on_close_request: false,
        icon: window::icon::from_file_data(include_bytes!("../../../brand/png/icon/omaflake-icon-128.png"), None).ok(),
        #[cfg(target_os = "linux")]
        platform_specific: window::settings::PlatformSpecific {
            application_id: "omaflake".into(),
            ..Default::default()
        },
        ..Default::default()
    }
}

impl App {
    pub fn boot() -> (App, Task<Message>) {
        let (id, open) = window::open(window_settings());
        let app = App {
            prefs: Prefs::load(),
            window: Some(id),
            focused: true,
            size: Size::new(560.0, 680.0),
            glow: None,
            core: None,
            status: Some("Connecting to omaflake's core …".into()),
            init: Value::Null,
            models: HashMap::new(),
            tab: "vms".into(),
            tab_chosen: false,
            stack: Vec::new(),
            filters: HashMap::new(),
            filter_menu: None,
            expanded: HashSet::new(),
            toggled: HashSet::new(),
            forms: HashMap::new(),
            edits: HashMap::new(),
            logs: HashMap::new(),
            jobs: BTreeMap::new(),
            flow: None,
            modal: None,
            toast: None,
            seen_activity: 0,
            notified: None,
            notifications: true,
            catalogs_asked: false,
            pulse: false,
            subscribed: Vec::new(),
            pending: VecDeque::new(),
            demo: false,
            #[cfg(target_os = "linux")]
            tray: None,
        };
        let mut app = app;
        app.update_glow();
        app.load_demo();
        (app, open.map(Message::WindowOpened))
    }

    pub fn title(&self, _: window::Id) -> String {
        "omaflake".into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        #[allow(unused_mut)]
        let mut subs = vec![
            if self.demo { Subscription::none() } else { Subscription::run(backend::connection).map(Message::Core) },
            window::close_requests().map(Message::CloseRequested),
            window::resize_events().map(|(_, s)| Message::Resized(s)),
            iced::event::listen_with(|event, _, _| match event {
                iced::Event::Window(window::Event::Focused) => Some(Message::Focus(true)),
                iced::Event::Window(window::Event::Unfocused) => Some(Message::Focus(false)),
                iced::Event::Keyboard(iced::keyboard::Event::KeyPressed {
                    key: iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape),
                    ..
                }) => Some(Message::Back),
                _ => None,
            }),
            iced::time::every(Duration::from_millis(600)).map(Message::Tick),
        ];
        #[cfg(target_os = "linux")]
        if self.prefs.tray {
            subs.push(Subscription::run(tray::stream).map(Message::Tray));
        }
        Subscription::batch(subs)
    }

    // MARK: Pages

    /// The page shown: the top of the stack, with the form values typed so far
    pub fn effective(&self, p: &PageRef) -> PageRef {
        let mut p = p.clone();
        if let Some(values) = self.forms.get(&p.page).filter(|v| !v.is_empty()) {
            if matches!(p.page.as_str(), "newVm" | "macBase" | "settings") {
                p.params["values"] = Value::Object(values.clone());
            }
        }
        p
    }

    /// The open tab's list page with its filter
    pub fn tab_page(&self) -> PageRef {
        let mut params = json!({});
        if let Some(f) = self.filters.get(&self.tab) {
            params["filter"] = f.clone();
        }
        page_ref(&self.tab, params)
    }

    pub fn current(&self) -> Option<PageRef> {
        self.stack.last().map(|p| self.effective(p))
    }

    pub fn model(&self, p: &PageRef) -> Option<&Model> {
        self.models.get(&p.key())
    }

    /// A model of a page with any params (the newest one the core sent for that page name)
    fn wanted(&self) -> Vec<PageRef> {
        let mut pages = vec![page_ref("home", json!({})), page_ref("activity", json!({}))];
        let tab = self.tab_page();
        if !pages.contains(&tab) {
            pages.push(tab);
        }
        // The tray lists every VM
        let all_vms = page_ref("vms", json!({}));
        if self.prefs.tray && !pages.contains(&all_vms) {
            pages.push(all_vms);
        }
        if let Some(p) = self.current() {
            if p.page != "guide" && !pages.contains(&p) {
                pages.push(p);
            }
        }
        pages
    }

    /// `views.subscribe` when the pages wanted changed
    fn resubscribe(&mut self) -> Task<Message> {
        let Some(core) = &self.core else { return Task::none() };
        let wanted = self.wanted();
        if wanted == self.subscribed {
            return Task::none();
        }
        self.subscribed = wanted.clone();
        let core = core.clone();
        Task::perform(async move { core.call("views.subscribe", json!({"pages": wanted})).await }, |r| {
            Message::Answer(Purpose::Quiet, r)
        })
    }

    fn call(&self, method: &str, params: Value, purpose: Purpose) -> Task<Message> {
        let Some(core) = &self.core else {
            return Task::done(Message::Answer(
                purpose,
                Err(CallError { message: "Not connected to omaflake's core".into() }),
            ));
        };
        let fut = core.call(method, params);
        Task::perform(fut, move |r| Message::Answer(purpose.clone(), r))
    }

    /// Development aid: `OMAFLAKE_DEMO_VIEWS=<folder of page models>` (e.g. the core's testdata/api/views) draws those
    /// models without a core – for looking at the pages without VMs. `OMAFLAKE_DEMO_TAB` picks the tab,
    /// `OMAFLAKE_DEMO_PAGE` opens a page (its file `<page>.json`, or `<page>.<variant>.json` as `page.variant`),
    /// `OMAFLAKE_DEMO_EXPAND` lists row keys to open (`vm:research,image:…`).
    fn load_demo(&mut self) {
        let Some(dir) = std::env::var_os("OMAFLAKE_DEMO_VIEWS").map(PathBuf::from) else { return };
        self.demo = true;
        self.status = None;
        let read = |file: &str| -> Option<Value> { serde_json::from_slice(&std::fs::read(dir.join(file)).ok()?).ok() };
        for page in ["home", "vms", "images", "recipes", "activity"] {
            if let Some(v) = read(&format!("{page}.json")) {
                self.models.insert(page_ref(page, json!({})).key(), decode(page, v));
            }
        }
        if let Ok(tab) = std::env::var("OMAFLAKE_DEMO_TAB") {
            self.tab = tab;
            self.tab_chosen = true;
        }
        if let Ok(page) = std::env::var("OMAFLAKE_DEMO_PAGE") {
            let name = page.split('.').next().unwrap_or_default().to_string();
            if let Some(v) = read(&format!("{page}.json")) {
                let r = page_ref(&name, json!({}));
                self.models.insert(r.key(), decode(&name, v));
                self.stack.push(r);
            }
        }
        if let Ok(keys) = std::env::var("OMAFLAKE_DEMO_EXPAND") {
            self.expanded.extend(keys.split(',').map(str::to_string));
        }
        if let Some(Model::Build(b)) = self.current().as_ref().and_then(|p| self.model(p)) {
            if let Some(l) = &b.log {
                let lines = (1..=l.lines).map(|i| format!("[demo] log line {i}")).collect();
                self.logs.insert(l.job.clone(), Log { lines, loaded: true });
            }
        }
    }

    /// The background's glow, drawn again when the window's size changed noticeably (else it stretches)
    fn update_glow(&mut self) {
        let stale = self.glow.as_ref().is_none_or(|(s, _)| {
            (s.width - self.size.width).abs() > 24.0 || (s.height - self.size.height).abs() > 24.0
        });
        if stale {
            self.glow = Some((self.size, crate::view::glow_image(self.size)));
        }
    }

    fn say(&mut self, text: impl Into<String>, error: bool) {
        self.toast = Some(Toast { text: text.into(), error, until: Instant::now() + Duration::from_secs(6) });
    }

    // MARK: Update

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Core(e) => self.core_event(e),
            #[cfg(target_os = "linux")]
            Message::Tray(e) => self.tray_event(e),
            Message::Answer(purpose, result) => self.answered(purpose, result),
            Message::Tab(tab) => {
                self.tab = tab;
                self.tab_chosen = true;
                self.filter_menu = None;
                if self.tab == "activity" {
                    self.mark_activity_seen();
                }
                let refresh = self.refresh_catalogs_once();
                Task::batch([self.resubscribe(), refresh])
            }
            Message::Expand(key) => {
                if !self.expanded.remove(&key) {
                    self.expanded.insert(key.clone());
                }
                if !self.toggled.remove(&key) {
                    self.toggled.insert(key);
                }
                Task::none()
            }
            Message::FilterMenu(page) => {
                self.filter_menu = if self.filter_menu == page { None } else { page };
                Task::none()
            }
            Message::SetFilter { page, group, value } => {
                let mut f = self.filters.get(&page).cloned().unwrap_or_else(|| json!({}));
                if value.is_null() || value == json!(false) {
                    if let Some(m) = f.as_object_mut() {
                        m.remove(&group);
                    }
                } else {
                    f[&group] = value;
                }
                if f.as_object().is_some_and(|m| m.is_empty()) {
                    self.filters.remove(&page);
                } else {
                    self.filters.insert(page, f);
                }
                self.resubscribe()
            }
            Message::Run(action, form) => self.start(*action, form),
            Message::FormSet { page, field, value } => {
                self.forms.entry(page).or_default().insert(field, value);
                self.resubscribe()
            }
            Message::FormChoose { page, field } => {
                let Some(mut ask) = field.ask.clone() else { return Task::none() };
                ask.param = "value".into();
                let action = Action {
                    id: format!("choose-{}", field.id),
                    label: field.label.clone(),
                    icon: None,
                    method: "ui.field".into(),
                    params: json!({"field": field.id, "page": page}),
                    enabled: true,
                    reason: None,
                    help: None,
                    destructive: false,
                    on: None,
                    busy: false,
                    badge: None,
                    ask: vec![ask],
                    confirm: None,
                    then: None,
                };
                self.start(action, None)
            }
            Message::Edit { field, text } => {
                self.edits.insert(field, text);
                Task::none()
            }
            Message::Store(field, value) => {
                self.edits.remove(&field.id);
                let Some(set) = &field.set else { return Task::none() };
                let mut params = set.params.clone();
                set_param(&mut params, &set.param, value);
                self.call(&set.method, params, Purpose::Quiet)
            }
            Message::ModalInput(v) => {
                match &mut self.modal {
                    Some(Modal::Ask { value, error, .. }) => {
                        *value = v;
                        *error = None;
                    }
                    Some(Modal::Confirm { input: Some(i), .. }) => *i = v,
                    _ => {}
                }
                Task::none()
            }
            Message::ModalRepeat(v) => {
                if let Some(Modal::Ask { repeat, error, .. }) = &mut self.modal {
                    *repeat = v;
                    *error = None;
                }
                Task::none()
            }
            Message::ModalOk => self.modal_ok(),
            Message::ModalCancel => {
                self.modal = None;
                self.flow = None;
                self.next_pending()
            }
            Message::Picked(paths) => self.picked(paths),
            Message::Checked(result) => match result {
                Ok(values) if !values.is_empty() => self.chosen(values),
                Ok(_) => {
                    self.flow = None;
                    Task::none()
                }
                Err(reason) => {
                    self.flow = None;
                    self.say(reason, true);
                    Task::none()
                }
            },
            Message::Back => {
                if self.modal.is_some() {
                    self.modal = None;
                    self.flow = None;
                } else if self.filter_menu.is_some() {
                    self.filter_menu = None;
                } else {
                    return self.back();
                }
                Task::none()
            }
            Message::Tick(now) => {
                self.pulse = !self.pulse;
                if self.toast.as_ref().is_some_and(|t| t.until <= now) {
                    self.toast = None;
                }
                Task::none()
            }
            Message::Focus(f) => {
                self.focused = f;
                Task::none()
            }
            Message::Resized(size) => {
                self.size = size;
                self.update_glow();
                Task::none()
            }
            Message::WindowOpened(id) => {
                self.window = Some(id);
                self.focused = true;
                Task::none()
            }
            Message::CloseRequested(id) => {
                if self.keeps_running() {
                    self.window = None;
                    self.focused = false;
                    window::close(id)
                } else {
                    self.quit()
                }
            }
            Message::SetTray(on) => {
                self.prefs.tray = on;
                self.prefs.save();
                #[cfg(target_os = "linux")]
                if !on {
                    if let Some(t) = self.tray.take() {
                        t.shutdown();
                    }
                }
                self.resubscribe()
            }
            Message::DismissToast => {
                self.toast = None;
                Task::none()
            }
        }
    }

    /// Closing the window keeps the app in the tray while the tray is on (and runs)
    fn keeps_running(&self) -> bool {
        #[cfg(target_os = "linux")]
        return self.prefs.tray && self.tray.is_some();
        #[cfg(not(target_os = "linux"))]
        false
    }

    fn quit(&mut self) -> Task<Message> {
        // The service goes on for its other clients and its jobs; VMs keep running
        if let Some(core) = self.core.take() {
            let _ = core.call_blocking("shutdown", Value::Null);
        }
        #[cfg(target_os = "linux")]
        if let Some(t) = self.tray.take() {
            t.shutdown();
        }
        iced::exit()
    }

    fn open_window(&mut self) -> Task<Message> {
        match self.window {
            Some(id) => window::gain_focus(id),
            None => {
                let (id, open) = window::open(window_settings());
                self.window = Some(id);
                open.map(Message::WindowOpened)
            }
        }
    }

    fn back(&mut self) -> Task<Message> {
        if let Some(p) = self.stack.pop() {
            // A form starts afresh next time
            if !self.stack.iter().any(|q| q.page == p.page) {
                self.forms.remove(&p.page);
            }
        }
        self.resubscribe()
    }

    fn show(&mut self, page: &str, params: Value) -> Task<Message> {
        if page == "guide" {
            // The guide is the macOS app's (the Linux builder); the docs site has it
            let file = params["file"].as_str().unwrap_or("linux-builder").to_string();
            return match system::open(&format!("https://omaflake.dev/{file}/")) {
                Ok(()) => Task::none(),
                Err(e) => {
                    self.say(e, true);
                    Task::none()
                }
            };
        }
        self.filter_menu = None;
        self.forms.remove(page);
        self.stack.push(page_ref(page, params));
        if self.window.is_none() {
            return Task::batch([self.open_window(), self.resubscribe()]);
        }
        self.resubscribe()
    }

    fn mark_activity_seen(&mut self) {
        if let Some(Model::Activity(a)) = self.models.get(&page_ref("activity", json!({})).key()) {
            self.seen_activity = a.entries.first().map(|e| e.id).unwrap_or(0);
        }
    }

    /// The recipes tab fetches the sources' catalogs once when it opens (as the macOS app did)
    fn refresh_catalogs_once(&mut self) -> Task<Message> {
        if self.tab != "recipes" || self.catalogs_asked {
            return Task::none();
        }
        let Some(Model::Recipes(r)) = self.models.get(&self.tab_page().key()) else { return Task::none() };
        if r.catalogs_loaded || r.loading {
            self.catalogs_asked = true;
            return Task::none();
        }
        let refresh = r.head.actions.iter().find(|a| a.id == "refresh" && a.enabled).cloned();
        self.catalogs_asked = true;
        match refresh {
            Some(a) => self.start(a, None),
            None => Task::none(),
        }
    }

    // MARK: The core's events

    fn core_event(&mut self, e: backend::Event) -> Task<Message> {
        match e {
            backend::Event::Connected(handle, init) => {
                self.status = match &handle.via {
                    backend::Via::Service(_) => None,
                    backend::Via::InProcess(why) => {
                        Some(format!("Running the core in this app – no service ({why}); jobs end with the app"))
                    }
                };
                self.core = Some(handle);
                self.init = init;
                self.subscribed.clear();
                Task::batch([
                    self.call("events.subscribe", json!({"topics": ["jobs"]}), Purpose::Quiet),
                    self.call("settings.get", Value::Null, Purpose::Settings),
                    self.resubscribe(),
                ])
            }
            backend::Event::Disconnected(why) => {
                if self.core.take().is_some() || self.status.is_none() {
                    self.status = Some(format!("Connecting to omaflake's core … ({why})"));
                }
                self.subscribed.clear();
                Task::none()
            }
            backend::Event::Notification(method, params) => self.notification(&method, params),
        }
    }

    fn notification(&mut self, method: &str, params: Value) -> Task<Message> {
        match method {
            "view.changed" => {
                let page = params["page"].as_str().unwrap_or_default().to_string();
                let r = page_ref(&page, params["params"].clone());
                let model = decode(&page, params["model"].clone());
                self.models.insert(r.key(), model);
                self.after_model(&page)
            }
            "jobs.changed" => {
                if let Ok(job) = serde_json::from_value::<JobDto>(params["job"].clone()) {
                    self.jobs.insert(job.id.clone(), job);
                }
                self.update_tray();
                Task::none()
            }
            "job.log" => {
                let id = params["id"].as_str().unwrap_or_default();
                if let Some(log) = self.logs.get_mut(id).filter(|l| l.loaded) {
                    let lines: Vec<String> = serde_json::from_value(params["lines"].clone()).unwrap_or_default();
                    log.lines.extend(lines);
                    let over = log.lines.len().saturating_sub(5000);
                    log.lines.drain(..over);
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    /// What a new model brings along: the first tab, logs to read, notifications, the tray
    fn after_model(&mut self, page: &str) -> Task<Message> {
        let mut tasks = Vec::new();
        match page {
            "home" => {
                if !self.tab_chosen {
                    if let Some(Model::Home(h)) = self.models.get(&page_ref("home", json!({})).key()) {
                        if h.first_tab != self.tab {
                            self.tab = h.first_tab.clone();
                            tasks.push(self.resubscribe());
                        }
                    }
                    self.tab_chosen = true;
                }
            }
            "activity" => self.activity_arrived(),
            "vms" => self.update_tray(),
            "recipes" => tasks.push(self.refresh_catalogs_once()),
            "settings" => {
                if let Some(Model::Settings(s)) = self.current().as_ref().and_then(|p| self.model(p)) {
                    if let Some(f) = s.general.fields.iter().find(|f| f.id == "notifications") {
                        self.notifications = f.value.as_bool().unwrap_or(true);
                    }
                }
            }
            _ => {}
        }
        // Logs the page shows
        let log_job = match self.current().as_ref().and_then(|p| self.model(p)) {
            Some(Model::Build(b)) => b.log.as_ref().map(|l| l.job.clone()),
            Some(Model::MacBase(m)) => m.log.clone(),
            _ => None,
        };
        if let Some(id) = log_job {
            if !self.logs.contains_key(&id) {
                self.logs.insert(id.clone(), Log::default());
                tasks.push(self.call("jobs.log", json!({"id": id}), Purpose::Log(id)));
            }
        }
        Task::batch(tasks)
    }

    fn activity_arrived(&mut self) {
        let Some(Model::Activity(a)) = self.models.get(&page_ref("activity", json!({})).key()) else { return };
        let newest = a.entries.first().map(|e| e.id).unwrap_or(0);
        let Some(last) = self.notified else {
            // What happened before this app opened is not news
            self.notified = Some(newest);
            self.seen_activity = newest;
            return;
        };
        let fresh: Vec<_> = a.entries.iter().filter(|e| e.id > last).cloned().collect();
        self.notified = Some(newest.max(last));
        if fresh.is_empty() {
            return;
        }
        let window_in_front = self.window.is_some() && self.focused;
        if self.notifications && !window_in_front {
            for e in fresh.iter().rev() {
                if let Some(n) = &e.notification {
                    system::notify(&s(&n.title), &s(&n.body));
                }
            }
        }
        if self.tab == "activity" && self.stack.is_empty() {
            self.seen_activity = newest;
        } else if let Some(e) = fresh.first() {
            let error = e.level == omaflake_core::api::types::Level::Error;
            self.say(s(&e.text), error);
        }
    }

    fn update_tray(&mut self) {
        #[cfg(target_os = "linux")]
        if let Some(t) = &self.tray {
            t.set(self.tray_model());
        }
    }

    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub fn tray_model(&self) -> tray::TrayModel {
        let vms = match self.models.get(&page_ref("vms", json!({})).key()) {
            Some(Model::Vms(v)) => Some(v.as_ref()),
            _ => None,
        };
        let jobs: Vec<JobDto> = self.jobs.values().cloned().collect();
        tray::model(vms, &jobs)
    }

    #[cfg(target_os = "linux")]
    fn tray_event(&mut self, e: tray::Event) -> Task<Message> {
        match e {
            tray::Event::Ready(h) => {
                h.set(self.tray_model());
                self.tray = Some(h);
                Task::none()
            }
            tray::Event::Failed(why) => {
                eprintln!("omaflake: no tray icon: {why}");
                Task::none()
            }
            tray::Event::Command(tray::Command::Open) => self.open_window(),
            tray::Event::Command(tray::Command::Quit) => self.quit(),
            tray::Event::Command(tray::Command::Run(a)) => self.start(*a, None),
        }
    }

    // MARK: Actions

    /// Starts an action: its asks, its confirmation, then sending it. `form`: the page whose typed values go into its
    /// params (the page's own action: New VM's Create, the macOS base's Create, Settings' new source).
    pub fn start(&mut self, action: Action, form: Option<String>) -> Task<Message> {
        if !action.enabled {
            if let Some(r) = &action.reason {
                self.say(s(r), false);
            }
            return Task::none();
        }
        let mut params = action.params.clone();
        if let Some(page) = form {
            self.apply_form(&page, &mut params);
        }
        self.filter_menu = None;
        self.flow = Some(Flow {
            asks: action.ask.iter().cloned().collect(),
            action,
            params,
            queue: VecDeque::new(),
            picking: None,
        });
        self.advance()
    }

    fn next_pending(&mut self) -> Task<Message> {
        if self.flow.is_some() || self.modal.is_some() {
            return Task::none();
        }
        match self.pending.pop_front() {
            Some(a) => self.start(a, None),
            None => Task::none(),
        }
    }

    /// Puts the form's typed values into the page action's params, at each field's `param`
    fn apply_form(&self, page: &str, params: &mut Value) {
        let Some(values) = self.forms.get(page) else { return };
        let fields: Vec<Field> = match self.current().as_ref().and_then(|p| self.model(p)) {
            Some(Model::NewVm(n)) => n.sections.iter().flat_map(|s| s.fields.clone()).collect(),
            Some(Model::MacBase(m)) => m.fields.clone(),
            Some(Model::Settings(s)) => s.sources.add.fields.clone(),
            _ => vec![],
        };
        for f in fields {
            if let (Some(param), Some(v)) = (&f.param, values.get(&f.id)) {
                set_param(params, param, v.clone());
            }
        }
    }

    fn advance(&mut self) -> Task<Message> {
        let Some(flow) = &mut self.flow else { return Task::none() };
        if let Some(ask) = flow.asks.pop_front() {
            return match ask.kind {
                AskKind::Text | AskKind::Password => {
                    let value = ask.default.clone().unwrap_or_default();
                    self.modal = Some(Modal::Ask { ask, value, repeat: String::new(), error: None });
                    Task::none()
                }
                AskKind::Folder | AskKind::File | AskKind::Package => {
                    flow.picking = Some(ask.clone());
                    Task::perform(pick(ask), Message::Picked)
                }
            };
        }
        if flow.queue.is_empty() {
            flow.queue.push_back(flow.params.clone());
        }
        self.confirm_next()
    }

    fn confirm_next(&mut self) -> Task<Message> {
        let Some(flow) = &mut self.flow else { return Task::none() };
        let Some(params) = flow.queue.pop_front() else {
            self.flow = None;
            return self.next_pending();
        };
        if let Some(c) = &flow.action.confirm {
            let input = c.input.as_ref().map(|i| i.default.clone());
            self.modal = Some(Modal::Confirm { confirm: c.clone(), params, input });
            return Task::none();
        }
        let send = self.send(params);
        Task::batch([send, self.confirm_next()])
    }

    fn modal_ok(&mut self) -> Task<Message> {
        match self.modal.take() {
            Some(Modal::Ask { ask, value, repeat, .. }) => {
                if ask.repeat && value != repeat {
                    self.modal = Some(Modal::Ask { ask, value, repeat, error: Some("The two entries differ.".into()) });
                    return Task::none();
                }
                if let Some(flow) = &mut self.flow {
                    set_param(&mut flow.params, &ask.param, json!(value));
                }
                self.advance()
            }
            Some(Modal::Confirm { confirm, mut params, input }) => {
                if let (Some(i), Some(text)) = (&confirm.input, input) {
                    set_param(&mut params, &i.param, json!(text));
                }
                let send = self.send(params);
                Task::batch([send, self.confirm_next()])
            }
            None => Task::none(),
        }
    }

    fn picked(&mut self, paths: Option<Vec<PathBuf>>) -> Task<Message> {
        let Some(ask) = self.flow.as_mut().and_then(|f| f.picking.take()) else { return Task::none() };
        let values: Vec<String> =
            paths.unwrap_or_default().into_iter().map(|p| p.to_string_lossy().into_owned()).collect();
        if values.is_empty() {
            self.flow = None;
            return Task::none();
        }
        if let Some(flow) = &mut self.flow {
            flow.picking = Some(ask.clone());
        }
        let Some(check) = ask.check.clone() else { return self.chosen(values) };
        let Some(core) = self.core.clone() else { return self.chosen(values) };
        // Each chosen value is checked before it counts (packages.inspect); refused ones say why
        Task::perform(
            async move {
                let mut ok = Vec::new();
                let mut refused = Vec::new();
                for v in values {
                    let mut params = check.params.clone();
                    set_param(&mut params, &check.param, json!(v));
                    match core.call(&check.method, params).await {
                        Ok(r) if r["valid"] == json!(false) => {
                            refused.push(format!("{v}: {}", r["reason"].as_str().unwrap_or("not usable")))
                        }
                        Ok(_) => ok.push(v),
                        Err(e) => refused.push(format!("{v}: {}", e.message)),
                    }
                }
                if ok.is_empty() && !refused.is_empty() {
                    Err(refused.join("\n"))
                } else {
                    Ok(ok)
                }
            },
            Message::Checked,
        )
    }

    /// The values chosen for the ask a dialog was open for
    fn chosen(&mut self, values: Vec<String>) -> Task<Message> {
        let Some(flow) = &mut self.flow else { return Task::none() };
        let Some(ask) = flow.picking.take() else { return Task::none() };
        if ask.multiple && values.len() > 1 {
            for v in &values {
                let mut p = flow.params.clone();
                set_param(&mut p, &ask.param, json!(v));
                flow.queue.push_back(p);
            }
        } else {
            set_param(&mut flow.params, &ask.param, json!(values[0]));
        }
        self.advance()
    }

    /// Sends one params set: a `ui.*` verb here, anything else to the core
    fn send(&mut self, params: Value) -> Task<Message> {
        let Some(flow) = &self.flow else { return Task::none() };
        let method = flow.action.method.clone();
        let then = flow.action.then;
        if let Some(verb) = method.strip_prefix("ui.") {
            return self.ui(verb, params);
        }
        self.call(&method, params, Purpose::Action { then })
    }

    /// The verbs a UI does itself (api.md §10.4)
    fn ui(&mut self, verb: &str, p: Value) -> Task<Message> {
        let str_of = |k: &str| p[k].as_str().map(str::to_string);
        let done = |r: Result<(), String>, app: &mut App| {
            if let Err(e) = r {
                app.say(e, true);
            }
            Task::none()
        };
        match verb {
            "show" => self.show(&str_of("page").unwrap_or_default(), p["params"].clone()),
            "back" => self.back(),
            "step" => self.call("views.get", json!({"page": p["page"], "params": p["params"]}), Purpose::Step),
            "filter" => {
                let page = str_of("page").unwrap_or_else(|| self.tab.clone());
                if p["filter"].is_null() {
                    self.filters.remove(&page);
                } else {
                    self.filters.insert(page, p["filter"].clone());
                }
                self.filter_menu = None;
                self.resubscribe()
            }
            "field" => {
                let page = str_of("page").or_else(|| self.stack.last().map(|s| s.page.clone())).unwrap_or_default();
                let field = str_of("field").unwrap_or_default();
                self.forms.entry(page).or_default().insert(field, p.get("value").cloned().unwrap_or(Value::Null));
                self.resubscribe()
            }
            "reveal" => done(system::reveal(&str_of("path").unwrap_or_default()), self),
            "open" => {
                let target = str_of("path").or_else(|| str_of("url")).unwrap_or_default();
                done(system::open(&target), self)
            }
            "terminal" => done(system::run_in_terminal(&str_of("line").unwrap_or_default()), self),
            "quit" => self.quit(),
            other => {
                self.say(format!("This app does not know ui.{other}"), true);
                Task::none()
            }
        }
    }

    fn answered(&mut self, purpose: Purpose, result: Result<Value, CallError>) -> Task<Message> {
        let value = match result {
            Ok(v) => v,
            Err(e) => {
                if let Purpose::Log(id) = &purpose {
                    self.logs.remove(id);
                }
                // The activity list has refusals too; the toast shows it where the user looks
                self.say(e.message, true);
                return Task::none();
            }
        };
        match purpose {
            Purpose::Action { then } => match then {
                Some(Then::Reveal) => {
                    let r = system::reveal(lookup(&value, "path").and_then(Value::as_str).unwrap_or_default());
                    if let Err(e) = r {
                        self.say(e, true);
                    }
                    Task::none()
                }
                Some(Then::Terminal) => {
                    let line = lookup(&value, "shellLine").and_then(Value::as_str).unwrap_or_default().to_string();
                    if let Err(e) = system::run_in_terminal(&line) {
                        self.say(e, true);
                    }
                    Task::none()
                }
                Some(Then::Back) => self.back(),
                None => Task::none(),
            },
            Purpose::Step => match serde_json::from_value::<ImportPage>(value) {
                Ok(page) => match page.action {
                    Some(a) => {
                        self.pending.push_back(a);
                        self.next_pending()
                    }
                    None => {
                        self.say(page.reason.map(|r| s(&r)).unwrap_or_else(|| "This cannot be imported".into()), true);
                        Task::none()
                    }
                },
                Err(e) => {
                    self.say(format!("This app cannot read the import page: {e}"), true);
                    Task::none()
                }
            },
            Purpose::Log(id) => {
                let lines: Vec<String> = serde_json::from_value(value["lines"].clone()).unwrap_or_default();
                self.logs.insert(id, Log { lines, loaded: true });
                Task::none()
            }
            Purpose::Settings => {
                self.notifications = value["settings"]["notifications"].as_bool().unwrap_or(true);
                Task::none()
            }
            Purpose::Quiet => Task::none(),
        }
    }
}

/// A file dialog for an ask (desktop portals on Linux, through rfd)
async fn pick(ask: Ask) -> Option<Vec<PathBuf>> {
    let mut d = rfd::AsyncFileDialog::new();
    if let Some(p) = ask.prompt.as_ref().or(Some(&ask.label)) {
        d = d.set_title(s(p));
    }
    if let Some(dir) = ask.default.as_ref().map(PathBuf::from).filter(|p| p.is_dir()) {
        d = d.set_directory(dir);
    }
    if ask.accept.as_deref() == Some("ipsw") {
        d = d.add_filter("macOS restore image", &["ipsw"]);
    }
    let picked = match ask.kind {
        // omaflake packages (.omaflake-vm, -image, -recipe) are folders, and so is a recipe folder
        AskKind::Folder | AskKind::Package if ask.multiple => d.pick_folders().await,
        AskKind::Folder | AskKind::Package => d.pick_folder().await.map(|f| vec![f]),
        _ if ask.multiple => d.pick_files().await,
        _ => d.pick_file().await.map(|f| vec![f]),
    };
    picked.map(|v| v.into_iter().map(|h| h.path().to_path_buf()).collect())
}
