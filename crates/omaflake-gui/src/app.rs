// SPDX-License-Identifier: AGPL-3.0-or-later
//! The app's state and what it does with messages. It keeps only what api.md §10 leaves to a UI: which tab and page
//! are open, which rows are expanded, what the user types, whether the activity was seen.
//! Everything shown comes from the core's page models (`view.changed`); every action is sent as the model names it.
//! What every interface of the page models does the same way – the pages subscribed and their models, an action's
//! way to the core with its asks, checks and confirmation, the jobs, their logs, what is new in the activity – is the
//! SDK's [`Session`], shared with `omaf ui`; this module turns messages into what the user wants of it and carries
//! its effects out (calls as tasks, questions as dialogs).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use iced::widget::image;
use iced::{Size, Subscription, Task, window};
use omaflake_sdk::protocol::RpcError;
use omaflake_sdk::session::{self, FlowId, News, Notice, Question, Session};
use omaflake_sdk::types::JobDto;
use omaflake_sdk::views::activity::ActivityPage;
use omaflake_sdk::views::build::{BuildPage, LogRef};
use omaflake_sdk::views::common::{Action, Ask, AskKind, Confirm, Field, set_param};
use omaflake_sdk::views::credentials::{CredentialPage, CredentialsPage};
use omaflake_sdk::views::doctor::{DoctorPage, MacBasePage};
use omaflake_sdk::views::home::HomePage;
use omaflake_sdk::views::images::{ImagePage, ImagesPage};
use omaflake_sdk::views::new_vm::NewVmPage;
use omaflake_sdk::views::recipes::RecipesPage;
use omaflake_sdk::views::settings::SettingsPage;
use omaflake_sdk::views::vms::{VmPage, VmsPage};
use omaflake_sdk::views::{PageRef, Text};
use serde_json::{Map, Value, json};

use crate::backend::{self, CallError, Handle};
use crate::prefs::Prefs;
use crate::text::{lookup, s, tr, trf};
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
    /// A VM's diagnosis (plan B162)
    Diagnostics(Box<omaflake_sdk::views::diagnostics::DiagnosticsPage>),
    MacBase(Box<MacBasePage>),
    Settings(Box<SettingsPage>),
    /// Credentials (plan B137)
    Credentials(Box<CredentialsPage>),
    Credential(Box<CredentialPage>),
    /// Networks between VMs (plan B170): the credentials' list and the graph
    Networks(Box<omaflake_sdk::views::networks::NetworksPage>),
    /// Looks (plan B141)
    Looks(Box<omaflake_sdk::views::looks::LooksPage>),
    Activity(Box<ActivityPage>),
    Vm(Box<VmPage>),
    Image(Box<ImagePage>),
    /// A recipe's keys by module and mode (plan B147)
    Keys(Box<omaflake_sdk::views::keys::KeysPage>),
    /// The recipe studio (plan B147)
    Studio(Box<omaflake_sdk::views::recipe_studio::RecipeStudioPage>),
    /// The key binding studio (plan B147)
    KeymapStudio(Box<omaflake_sdk::views::keymap_studio::KeymapStudioPage>),
    LookStudio(Box<omaflake_sdk::views::look_studio::LookStudioPage>),
    /// The recipe sources (plan B147: a tab of the catalog)
    Sources(Box<omaflake_sdk::views::settings::SourcesPage>),
    /// A page that cannot be built (`model: {error}`) or that this app cannot read
    Error(String),
}

/// Does a model (its JSON) show something working: a lamp in state `working`, a `busy` action, `loading`, a job that
/// waits or runs
pub fn pulses(v: &Value) -> bool {
    match v {
        Value::Object(o) => o.iter().any(|(k, x)| match (k.as_str(), x) {
            ("busy" | "loading", Value::Bool(true)) => true,
            ("state", Value::String(s)) if matches!(s.as_str(), "working" | "running" | "queued" | "checking") => true,
            _ => pulses(x),
        }),
        Value::Array(a) => a.iter().any(pulses),
        Value::String(s) => s == "working",
        _ => false,
    }
}

/// A page's model from its JSON
pub fn decode(page: &str, v: Value) -> Model {
    if let Some(e) = v.get("error") {
        // In the app's language when the message has a key (api.md §3)
        let text = e.get("text").and_then(|t| serde_json::from_value::<omaflake_sdk::text::Text>(t.clone()).ok());
        return Model::Error(match text {
            Some(t) => crate::text::s(&t),
            None => e["message"].as_str().map(str::to_string).unwrap_or_else(|| tr("This page cannot be shown")),
        });
    }
    fn de<T: serde::de::DeserializeOwned>(v: Value, f: fn(Box<T>) -> Model) -> Model {
        match serde_json::from_value::<T>(v) {
            Ok(t) => f(Box::new(t)),
            Err(e) => Model::Error(trf("This app cannot read the page: %@", &[e.to_string().into()])),
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
        "diagnostics" => de(v, Model::Diagnostics),
        "macBase" => de(v, Model::MacBase),
        "settings" => de(v, Model::Settings),
        "credentials" => de(v, Model::Credentials),
        "credential" => de(v, Model::Credential),
        // Locales (plan B141): the same page and form as credentials
        "locales" => de(v, Model::Credentials),
        "locale" => de(v, Model::Credential),
        // Plan B142: profiles in the credentials' shapes, the recipe editor in the credential form's, modules in the
        // looks'
        "profiles" => de(v, Model::Credentials),
        "profile" => de(v, Model::Credential),
        // Networks between VMs (plan B170): the list with its graph; the forms in the credential form's shape
        "networks" => de(v, Model::Networks),
        "network" | "networkMember" => de(v, Model::Credential),
        // Volumes (plan B187): the credentials' list and form
        "volumes" => de(v, Model::Credentials),
        "volume" | "volumeMount" => de(v, Model::Credential),
        // The first start's questions (plan B144): a credential form's shape
        "welcome" => de(v, Model::Credential),
        "recipeStudio" => de(v, Model::Studio),
        "keymapStudio" => de(v, Model::KeymapStudio),
        "lookStudio" => de(v, Model::LookStudio),
        // The catalog (plan B147): the sources, the recipes they offer (the recipes' shape), key bindings (the looks')
        "sources" => de(v, Model::Sources),
        "recipeCatalog" => de(v, Model::Recipes),
        "keymaps" => de(v, Model::Looks),
        "modules" => de(v, Model::Looks),
        // The credential of a macOS base from before bases kept theirs: the same form (plan B139)
        "baseCredential" => de(v, Model::Credential),
        "activity" => de(v, Model::Activity),
        "looks" => de(v, Model::Looks),
        "vm" => de(v, Model::Vm),
        "image" => de(v, Model::Image),
        "keys" => de(v, Model::Keys),
        other => Model::Error(trf("Unknown page %@", &[other.into()])),
    }
}

impl Model {
    /// The page's title as its model gives it (the window's for home, the header's for the others)
    pub fn title(&self) -> Option<&Text> {
        Some(match self {
            Model::Home(p) => &p.title,
            Model::Vms(p) => &p.head.title,
            Model::Images(p) => &p.head.title,
            Model::Recipes(p) => &p.head.title,
            Model::Activity(p) => &p.head.title,
            Model::Looks(p) => &p.head.title,
            Model::NewVm(p) => &p.title,
            Model::Build(p) => &p.title,
            Model::Doctor(p) => &p.title,
            Model::Diagnostics(p) => &p.title,
            Model::MacBase(p) => &p.title,
            Model::Settings(p) => &p.title,
            Model::Credentials(p) => &p.title,
            Model::Networks(p) => &p.list.title,
            Model::Credential(p) => &p.title,
            Model::Vm(p) => &p.title,
            Model::Image(p) => &p.title,
            Model::Keys(p) => &p.title,
            Model::Studio(p) => &p.title,
            Model::KeymapStudio(p) => &p.title,
            Model::LookStudio(p) => &p.title,
            Model::Sources(p) => &p.title,
            Model::Error(_) => return None,
        })
    }

    /// The job log the page shows (build, macOS base, a diagnosis)
    pub fn log(&self) -> Option<&LogRef> {
        match self {
            Model::Build(b) => b.log.as_ref(),
            Model::Diagnostics(d) => d.log.as_ref(),
            Model::MacBase(m) => m.log.as_ref(),
            _ => None,
        }
    }
}

/// What a guide opens: the bundled file (`path`, only inside omaflake.app), else its page on omaflake.dev (`url`)
pub fn guide_target(params: &Value) -> String {
    params["path"].as_str().or_else(|| params["url"].as_str()).unwrap_or_default().to_string()
}

/// Why a check (`packages.inspect`) refused a chosen value: its `reason` is a Text
pub fn refusal(value: &str, answer: &Value) -> String {
    let why =
        serde_json::from_value::<Text>(answer["reason"].clone()).map(|t| s(&t)).unwrap_or_else(|_| tr("not usable"));
    format!("{value}: {why}")
}

pub fn page_ref(page: &str, params: Value) -> PageRef {
    PageRef { page: page.into(), params: if params.is_null() { json!({}) } else { params } }
}

/// A text of the core (api.md §10.2) as it came in JSON, in the app's language
fn text_of(v: &Value) -> Option<String> {
    serde_json::from_value::<Text>(v.clone()).ok().map(|t| s(&t))
}

/// Why chosen values were not taken, one per line
fn refusals(refused: &[session::Refusal]) -> String {
    let line = |r: &session::Refusal| match &r.error {
        Some(e) => format!("{}: {}", r.value, e.message),
        None => refusal(&r.value, &json!({"reason": r.reason})),
    };
    refused.iter().map(line).collect::<Vec<_>>().join("\n")
}

#[derive(Debug, Clone)]
pub enum Message {
    Core(backend::Event),
    #[cfg(target_os = "linux")]
    Tray(tray::Event),
    /// The answer to a request of the session, by its id
    Answer(u64, Result<Value, CallError>),
    Tab(String),
    Expand(String),
    FilterMenu(Option<String>),
    SetFilter {
        page: String,
        group: String,
        value: Value,
    },
    Run(Box<Action>, Option<String>),
    /// A form field's value (newVm, macBase, the new source), kept under `values.<key>` (the field's `valueParam`);
    /// `null` drops it (back to the default)
    FormSet {
        page: String,
        key: String,
        value: Value,
    },
    /// Several form values of one page at once (`null` drops one): one request to the core
    FormSetMany {
        page: String,
        values: Vec<(String, Value)>,
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
    /// A file dialog closed: the action it was open for, and what was chosen
    Picked(FlowId, Option<Vec<PathBuf>>),
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

#[derive(Debug, Clone)]
pub struct Toast {
    pub text: String,
    pub error: bool,
    pub until: Instant,
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
    /// The key of the newest model per page name – shown while the model for changed params (a form value) comes
    pub latest: HashMap<String, String>,
    pub tab: String,
    pub tab_chosen: bool,
    pub stack: Vec<PageRef>,
    /// Pages, questions, jobs, logs, activity: what every interface keeps (the filters too)
    pub session: Session,
    pub filter_menu: Option<String>,
    pub expanded: HashSet<String>,
    /// Rows whose first state the model gives (Doctor items): what the user changed
    pub toggled: HashSet<String>,
    pub forms: HashMap<String, Map<String, Value>>,
    /// A form tab's choices while the same form is open as a page on top
    pub tab_forms: HashMap<String, Map<String, Value>>,
    pub edits: HashMap<String, String>,
    /// The session's question the app shows now: its action and, for an ask, which one (None: the confirmation)
    pub asked: Option<(FlowId, Option<usize>)>,
    /// The action a file dialog is open for
    pub picking: Option<FlowId>,
    pub modal: Option<Modal>,
    pub toast: Option<Toast>,
    pub seen_activity: u64,
    pub notifications: bool,
    pub catalogs_asked: std::collections::HashSet<String>,
    pub pulse: bool,
    /// The pages whose model shows something working (a pulsing lamp, a busy action, loading, a running job): only
    /// then – or with a toast – the app ticks; an idle window is not redrawn (plan B123)
    pub animated: HashSet<String>,
    /// Drawing fixture models, without a core (`OMAFLAKE_DEMO_VIEWS`)
    pub demo: bool,
    /// The demo's log lines, by job
    pub demo_log: Option<(String, Vec<String>)>,
    #[cfg(target_os = "linux")]
    pub tray: Option<tray::TrayHandle>,
}

/// At least as tall as the sidebar with every group and tab (plan B153), so it never has to scroll
pub const WINDOW_SIZE: Size = Size { width: 680.0, height: 720.0 };

fn window_settings() -> window::Settings {
    window::Settings {
        size: Size::new(780.0, 760.0),
        min_size: Some(WINDOW_SIZE),
        exit_on_close_request: false,
        icon: window::icon::from_file_data(include_bytes!("../assets/omaflake-icon-128.png"), None).ok(),
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
            size: Size::new(780.0, 760.0),
            glow: None,
            core: None,
            status: Some(tr("Connecting to omaflake's core …")),
            init: Value::Null,
            models: HashMap::new(),
            tab: "vms".into(),
            tab_chosen: false,
            stack: Vec::new(),
            session: Session::new(),
            filter_menu: None,
            expanded: HashSet::new(),
            toggled: HashSet::new(),
            forms: HashMap::new(),
            tab_forms: HashMap::new(),
            latest: HashMap::new(),
            edits: HashMap::new(),
            asked: None,
            picking: None,
            modal: None,
            toast: None,
            seen_activity: 0,
            notifications: true,
            catalogs_asked: std::collections::HashSet::new(),
            pulse: false,
            animated: HashSet::new(),
            demo: false,
            demo_log: None,
            #[cfg(target_os = "linux")]
            tray: None,
        };
        let mut app = app;
        app.update_glow();
        app.load_demo();
        (app, open.map(Message::WindowOpened))
    }

    pub fn title(&self, _: window::Id) -> String {
        match self.models.get(&page_ref("home", json!({})).key()).and_then(Model::title) {
            Some(t) => s(t),
            None => "omaflake".into(),
        }
    }

    /// Stores a page's model from its JSON and notes whether it shows something working
    pub fn set_model(&mut self, key: String, page: &str, v: Value) {
        if pulses(&v) {
            self.animated.insert(key.clone());
        } else {
            self.animated.remove(&key);
        }
        self.latest.insert(page.to_string(), key.clone());
        self.models.insert(key, decode(page, v));
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
        ];
        // The pulse and the toast's end – not when nothing moves: every tick redraws the window, which costs a core
        // with software or virtualised GPUs (plan B123)
        if self.toast.is_some() || !self.animated.is_empty() {
            subs.push(iced::time::every(Duration::from_millis(600)).map(Message::Tick));
        }
        #[cfg(target_os = "linux")]
        if self.prefs.tray {
            subs.push(Subscription::run(tray::stream).map(Message::Tray));
        }
        Subscription::batch(subs)
    }

    // MARK: Pages

    /// The page shown: the top of the stack, with the form values typed so far (`params.values`, keyed by the fields'
    /// `valueParam`)
    pub fn effective(&self, p: &PageRef) -> PageRef {
        let mut p = p.clone();
        if let Some(values) = self.forms.get(&p.page).filter(|v| !v.is_empty()) {
            p.params["values"] = Value::Object(values.clone());
        }
        p
    }

    /// The open tab's list page with its filter; a form as a tab (the studios) with what the user chose in it
    pub fn tab_page(&self) -> PageRef {
        let mut params = json!({});
        if let Some(f) = self.session.filters.get(&self.tab) {
            params["filter"] = f.clone();
        }
        self.effective(&page_ref(&self.tab, params))
    }

    pub fn current(&self) -> Option<PageRef> {
        self.stack.last().map(|p| self.effective(p))
    }

    pub fn model(&self, p: &PageRef) -> Option<&Model> {
        self.models.get(&p.key()).or_else(|| self.models.get(self.latest.get(&p.page)?))
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
        if let Some(p) = self.current()
            && p.page != "guide"
            && !pages.contains(&p)
        {
            pages.push(p);
        }
        pages
    }

    /// The pages shown now are the ones to follow: the session subscribes them when they changed (one page set at a
    /// time, the last one wanted last)
    fn resubscribe(&mut self) -> Task<Message> {
        let pages = self.wanted();
        let fx = self.session.show(pages);
        self.apply(fx)
    }

    /// A job's log as far as it was read
    pub fn log(&self, job: &str) -> &[String] {
        match &self.demo_log {
            Some((id, lines)) if id == job => lines,
            _ => self.session.log(job).unwrap_or_default(),
        }
    }

    // MARK: The session

    /// Carries out what the session asks for
    fn apply(&mut self, effects: Vec<session::Effect>) -> Task<Message> {
        let mut tasks = Vec::new();
        for effect in effects {
            match effect {
                session::Effect::Send(request) => {
                    let id = request.id;
                    tasks.push(match &self.core {
                        // A thread per call (`Handle::call`): the session sends in order what must not overtake
                        // each other
                        Some(core) => {
                            Task::perform(core.call(&request.method, request.params), move |r| Message::Answer(id, r))
                        }
                        None => Task::done(Message::Answer(
                            id,
                            Err(CallError { message: tr("Not connected to omaflake's core") }),
                        )),
                    });
                }
                session::Effect::Model(page) => tasks.push(self.model_arrived(&page)),
                session::Effect::Question => tasks.push(self.ask()),
                session::Effect::Jobs => self.update_tray(),
                // Drawn from the session
                session::Effect::Log(_) => {}
                session::Effect::Activity(news) => self.activity_arrived(&news),
                session::Effect::Ui { verb, params } => tasks.push(self.ui(&verb, params)),
                session::Effect::Done { action, result } => tasks.push(self.done(&action, &result)),
                session::Effect::Answered { method, result } => {
                    if method == "settings.get" {
                        self.notifications = result["settings"]["notifications"].as_bool().unwrap_or(true);
                    }
                }
                session::Effect::Notice(notice) => self.notice(notice),
            }
        }
        Task::batch(tasks)
    }

    fn notice(&mut self, notice: Notice) {
        match notice {
            // The activity list has refusals too; the toast shows it where the user looks
            Notice::Failed { error, .. } => self.say(error.message, true),
            Notice::Disabled { reason } => {
                if let Some(why) = reason.as_ref().and_then(text_of) {
                    self.say(why, false);
                }
            }
            Notice::Refused { reason, .. } => {
                let why = reason.as_ref().and_then(text_of).unwrap_or_else(|| tr("This cannot be imported"));
                self.say(why, true);
            }
            Notice::Skipped { refused } => self.say(refusals(&refused), true),
            Notice::NotConnected { .. } => self.say(tr("Not connected to omaflake's core"), true),
        }
    }

    /// A page's model came: decoded for drawing; only the models the session keeps stay (a form asks a new page with
    /// every value typed)
    fn model_arrived(&mut self, page: &PageRef) -> Task<Message> {
        let Some(v) = self.session.model(page).cloned() else { return Task::none() };
        self.set_model(page.key(), &page.page, v);
        let held: HashSet<String> = self.session.held().map(PageRef::key).collect();
        self.models.retain(|key, _| held.contains(key));
        self.animated.retain(|key| held.contains(key));
        self.after_model(&page.page)
    }

    /// The session's open question as a dialog: a sheet for a text or password and for a confirmation, a file dialog
    /// for a path. Without a question the sheet closes.
    fn ask(&mut self) -> Task<Message> {
        match self.session.question() {
            None => {
                self.asked = None;
                self.modal = None;
                Task::none()
            }
            Some(Question::Ask { flow, ask, index, checking, refused, .. }) => {
                let same = self.asked == Some((flow, Some(index)));
                let Ok(ask) = serde_json::from_value::<Ask>(ask) else {
                    self.say(tr("This app cannot read what the action asks"), true);
                    return self.give_up(flow);
                };
                match ask.kind {
                    AskKind::Text | AskKind::Password => {
                        if same && let Some(Modal::Ask { error, .. }) = &mut self.modal {
                            // Still this question: what was typed stays; a refused value says why
                            if !refused.is_empty() {
                                *error = Some(refusals(&refused));
                            }
                            return Task::none();
                        }
                        self.asked = Some((flow, Some(index)));
                        let value = ask.default.clone().unwrap_or_default();
                        let error = (!refused.is_empty()).then(|| refusals(&refused));
                        self.modal = Some(Modal::Ask { ask, value, repeat: String::new(), error });
                        Task::none()
                    }
                    AskKind::Folder | AskKind::File | AskKind::Path | AskKind::Package => {
                        if !refused.is_empty() {
                            // Nothing of what was chosen can be used: the action ends, saying why
                            self.say(refusals(&refused), true);
                            return self.give_up(flow);
                        }
                        if same {
                            return match (checking, self.picking == Some(flow)) {
                                // The core checks what was chosen, or the dialog is still open
                                (true, _) | (_, true) => Task::none(),
                                // The check's answer will not come (the connection closed meanwhile)
                                _ => self.give_up(flow),
                            };
                        }
                        self.asked = Some((flow, Some(index)));
                        self.modal = None;
                        self.picking = Some(flow);
                        Task::perform(pick(ask), move |paths| Message::Picked(flow, paths))
                    }
                }
            }
            Some(Question::Confirm { flow, confirm, params, .. }) => {
                if self.asked == Some((flow, None))
                    && matches!(&self.modal, Some(Modal::Confirm { params: p, .. }) if *p == params)
                {
                    return Task::none();
                }
                let Ok(confirm) = serde_json::from_value::<Confirm>(confirm) else {
                    self.say(tr("This app cannot read what the action asks"), true);
                    return self.give_up(flow);
                };
                self.asked = Some((flow, None));
                let input = confirm.input.as_ref().map(|i| i.default.clone());
                self.modal = Some(Modal::Confirm { confirm, params, input });
                Task::none()
            }
        }
    }

    /// The action of `flow` is given up: nothing more of it is sent, whatever answer still comes
    fn give_up(&mut self, flow: FlowId) -> Task<Message> {
        let fx = self.session.cancel(flow);
        self.apply(fx)
    }

    /// What follows an action the core did (its `then`)
    fn done(&mut self, action: &Value, result: &Value) -> Task<Message> {
        match action["then"].as_str() {
            Some("reveal") => {
                let r = system::reveal(lookup(result, "path").and_then(Value::as_str).unwrap_or_default());
                if let Err(e) = r {
                    self.say(e, true);
                }
                Task::none()
            }
            Some("terminal") => {
                let line = lookup(result, "shellLine").and_then(Value::as_str).unwrap_or_default().to_string();
                if let Err(e) = system::run_in_terminal(&line) {
                    self.say(e, true);
                }
                Task::none()
            }
            Some("back") => self.back(),
            _ => Task::none(),
        }
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
        // `OMAFLAKE_DEMO_VARIANT=linux,loading`: `<page>.<variant>.json` where there is one (home.linux, recipes.loading)
        let variants: Vec<String> = std::env::var("OMAFLAKE_DEMO_VARIANT")
            .map(|v| v.split(',').map(str::to_string).collect())
            .unwrap_or_default();
        for page in ["home", "vms", "images", "recipes", "activity"] {
            let v = variants
                .iter()
                .find_map(|x| read(&format!("{page}.{x}.json")))
                .or_else(|| read(&format!("{page}.json")));
            if let Some(v) = v {
                self.set_model(page_ref(page, json!({})).key(), page, v);
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
                self.set_model(r.key(), &name, v);
                self.stack.push(r);
            }
        }
        if let Ok(keys) = std::env::var("OMAFLAKE_DEMO_EXPAND") {
            self.expanded.extend(keys.split(',').map(str::to_string));
        }
        if let Some(l) = self.current().as_ref().and_then(|p| self.model(p)).and_then(Model::log).cloned() {
            let lines = (l.from + 1..=l.lines).map(|i| format!("[demo] log line {i}")).collect();
            self.demo_log = Some((l.job.clone(), lines));
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
            Message::Answer(id, result) => {
                // (the error's message is in the app's language already)
                let fx = self.session.answer(id, result.map_err(|e| RpcError::server(None, e.message)));
                self.apply(fx)
            }
            Message::Tab(tab) => {
                // The sidebar goes to the tab at once: pages open on top of the last one close (plan B152)
                while let Some(p) = self.stack.pop() {
                    self.forms.remove(&p.page);
                }
                for (page, v) in self.tab_forms.drain() {
                    self.forms.insert(page, v);
                }
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
                self.session.filters.set_group(&page, &group, value);
                self.resubscribe()
            }
            Message::Run(action, form) => self.start(*action, form),
            Message::FormSet { page, key, value } => {
                self.set_form(page, key, Some(value));
                self.resubscribe()
            }
            Message::FormSetMany { page, values } => {
                for (key, value) in values {
                    self.set_form(page.clone(), key, Some(value));
                }
                self.resubscribe()
            }
            Message::FormChoose { page, field } => {
                // The field's own ask, as a `ui.field` action of its page (api.md §10.4)
                let (Some(mut ask), Some(key)) = (field.ask.clone(), field.value_param.as_deref()) else {
                    return Task::none();
                };
                ask.param = "value".into();
                let action = Action::field(
                    &format!("choose-{}", field.id),
                    field.label.clone(),
                    None,
                    &page,
                    json!({}),
                    &field.id,
                    key,
                )
                .ask(ask);
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
                // One after the other, so that the last value stored is the one that stays
                let fx = self.session.set(&set.method, params);
                self.apply(fx)
            }
            Message::ModalInput(v) => {
                match &mut self.modal {
                    Some(Modal::Ask { value, error, .. }) => {
                        *value = v;
                        *error = None;
                        // Typed on while the core checks the value before: that check's answer is not for this text
                        if let Some((flow, _)) = self.asked {
                            let fx = self.session.retract(flow);
                            return self.apply(fx);
                        }
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
            Message::ModalCancel => self.cancel_question(),
            Message::Picked(flow, paths) => self.picked(flow, paths),
            Message::Back => {
                if self.modal.is_some() {
                    return self.cancel_question();
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
        match self.stack.pop() {
            Some(p) => {
                // A form starts afresh next time; a tab's own choices come back
                if !self.stack.iter().any(|q| q.page == p.page) {
                    self.forms.remove(&p.page);
                    if let Some(v) = self.tab_forms.remove(&p.page) {
                        self.forms.insert(p.page, v);
                    }
                }
            }
            // On a form tab (a studio's Cancel, or Save's `then: back`): its choices go, it starts afresh
            None => {
                self.forms.remove(&self.tab);
            }
        }
        self.resubscribe()
    }

    fn show(&mut self, page: &str, params: Value) -> Task<Message> {
        if page == "guide" {
            // A guide of the docs: the bundled file where the core has one (`path`), else its page on omaflake.dev
            // (`url`); the desktop opens it, so its `title` names no page here
            return match system::open(&guide_target(&params)) {
                Ok(()) => Task::none(),
                Err(e) => {
                    self.say(e, true);
                    Task::none()
                }
            };
        }
        self.filter_menu = None;
        // A form that is a tab too (the studios): opened as a page it starts afresh, the tab's choices wait aside and
        // come back when the page closes (they share the page's name, plan B154)
        if !self.stack.iter().any(|p| p.page == page) {
            if let Some(v) = self.forms.remove(page) {
                self.tab_forms.insert(page.to_string(), v);
            }
        } else {
            self.forms.remove(page);
        }
        self.stack.push(page_ref(page, params));
        if self.window.is_none() {
            return Task::batch([self.open_window(), self.resubscribe()]);
        }
        self.resubscribe()
    }

    /// A form value of a page under `values.<key>`; `None` or `null` drops it (back to the page's default)
    fn set_form(&mut self, page: String, key: String, value: Option<Value>) {
        match value.filter(|v| !v.is_null()) {
            Some(v) => {
                self.forms.entry(page).or_default().insert(key, v);
            }
            None => {
                if let Some(f) = self.forms.get_mut(&page) {
                    f.remove(&key);
                }
            }
        }
    }

    /// The open question is cancelled: its action never goes on (the next one waiting may ask)
    fn cancel_question(&mut self) -> Task<Message> {
        self.modal = None;
        match self.asked.take() {
            Some((flow, _)) => self.give_up(flow),
            None => Task::none(),
        }
    }

    fn mark_activity_seen(&mut self) {
        if let Some(Model::Activity(a)) = self.models.get(&page_ref("activity", json!({})).key()) {
            self.seen_activity = a.entries.first().map(|e| e.id).unwrap_or(0);
        }
    }

    /// The catalog tabs fetch the sources' catalogs once each when they open (as the macOS app does)
    fn refresh_catalogs_once(&mut self) -> Task<Message> {
        let tab = self.tab.clone();
        if !matches!(tab.as_str(), "recipes" | "recipeCatalog" | "modules" | "keymaps" | "looks")
            || self.catalogs_asked.contains(&tab)
        {
            return Task::none();
        }
        let (loaded, head) = match self.models.get(&self.tab_page().key()) {
            Some(Model::Recipes(r)) => (r.catalogs_loaded || r.loading, &r.head),
            Some(Model::Looks(l)) => (l.catalogs_loaded || l.loading, &l.head),
            _ => return Task::none(),
        };
        if loaded {
            self.catalogs_asked.insert(tab);
            return Task::none();
        }
        let refresh = head.actions.iter().find(|a| a.id == "refresh" && a.enabled).cloned();
        self.catalogs_asked.insert(tab);
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
                    backend::Via::InProcess(why) => Some(trf(
                        "Running the core in this app – no service (%@); jobs end with the app",
                        &[why.clone().into()],
                    )),
                };
                self.core = Some(handle);
                // A new connection: the session keeps nothing of the one before (jobs, logs, requests – a new
                // service counts its jobs from j1 again) and subscribes the jobs' events and the pages
                let pages = self.wanted();
                let mut fx = self.session.show(pages);
                fx.extend(self.session.connected(&init));
                fx.extend(self.session.call("settings.get", Value::Null));
                self.init = init;
                self.apply(fx)
            }
            backend::Event::Disconnected(why) => {
                if self.core.take().is_some() || self.status.is_none() {
                    self.status = Some(trf("Connecting to omaflake's core … (%@)", &[why.clone().into()]));
                }
                let fx = self.session.disconnected();
                self.apply(fx)
            }
            backend::Event::Notification(method, params) => {
                let fx = self.session.event(&method, &params);
                self.apply(fx)
            }
        }
    }

    /// What a new model brings along: the first tab, the catalogs, the tray
    fn after_model(&mut self, page: &str) -> Task<Message> {
        let mut tasks = Vec::new();
        match page {
            "home" => {
                if !self.tab_chosen {
                    if let Some(Model::Home(h)) = self.models.get(&page_ref("home", json!({})).key())
                        && h.first_tab != self.tab
                    {
                        self.tab = h.first_tab.clone();
                        tasks.push(self.resubscribe());
                    }
                    self.tab_chosen = true;
                }
            }
            "vms" => self.update_tray(),
            "recipes" | "modules" | "looks" => tasks.push(self.refresh_catalogs_once()),
            "settings" => {
                if let Some(Model::Settings(s)) = self.current().as_ref().and_then(|p| self.model(p))
                    && let Some(f) = s.general.fields.iter().find(|f| f.id == "notifications")
                {
                    self.notifications = f.value.as_bool().unwrap_or(true);
                }
            }
            _ => {}
        }
        Task::batch(tasks)
    }

    /// What is new in the activity list (which entries those are is the session's, api.md §10.6): notifications, a
    /// toast, the tab's dot
    fn activity_arrived(&mut self, news: &News) {
        let newest = news.newest.unwrap_or(0);
        if news.first {
            // What happened before this app opened is not news
            self.seen_activity = newest;
            return;
        }
        if news.restarted {
            // A new server (its ids start at 1 again): the mark is forgotten – its entries are unseen –, but what it
            // said before this app saw it is not news
            self.seen_activity = 0;
            return;
        }
        let Some(Model::Activity(a)) = self.models.get(&page_ref("activity", json!({})).key()) else { return };
        let ids: HashSet<u64> = news.fresh.iter().filter_map(|e| e["id"].as_u64()).collect();
        let fresh: Vec<_> = a.entries.iter().filter(|e| ids.contains(&e.id)).cloned().collect();
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
            let error = e.level == omaflake_sdk::types::Level::Error;
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
        let jobs: Vec<JobDto> = self.session.jobs().cloned().collect();
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
        let mut params = action.params.clone();
        if let Some(page) = form {
            self.apply_form(&page, &mut params);
        }
        self.filter_menu = None;
        // The session asks what the action needs (api.md §10.3: ask, then confirm, then send) – or says why it cannot
        // be done now
        let mut action = serde_json::to_value(&action).unwrap_or_default();
        action["params"] = params;
        let fx = self.session.start(&action);
        self.apply(fx)
    }

    /// Puts the form's typed values (`values.<valueParam>`) into the page action's params, at each field's `param`
    fn apply_form(&self, page: &str, params: &mut Value) {
        let Some(values) = self.forms.get(page) else { return };
        let fields: Vec<Field> = match self.current().as_ref().and_then(|p| self.model(p)) {
            Some(Model::NewVm(n)) => n.sections.iter().flat_map(|s| s.fields.clone()).collect(),
            Some(Model::MacBase(m)) => m.fields.clone(),
            Some(Model::Build(b)) => b.fields.clone(),
            Some(Model::Credential(c)) => c.fields.clone(),
            Some(Model::Sources(s)) => s.sources.add.fields.clone(),
            _ => vec![],
        };
        for f in fields {
            if let (Some(param), Some(v)) = (&f.param, f.value_param.as_ref().and_then(|k| values.get(k))) {
                set_param(params, param, v.clone());
            }
        }
    }

    fn modal_ok(&mut self) -> Task<Message> {
        let Some((flow, _)) = self.asked else {
            self.modal = None;
            return Task::none();
        };
        // The sheet stays until the session's question changes (the core may check the value first)
        let fx = match &mut self.modal {
            Some(Modal::Ask { ask, value, repeat, error }) => {
                if ask.repeat && value != repeat {
                    *error = Some("The two entries differ.".into());
                    return Task::none();
                }
                *error = None;
                self.session.reply(flow, vec![value.clone()])
            }
            Some(Modal::Confirm { input, .. }) => self.session.confirm(flow, input.clone()),
            None => return Task::none(),
        };
        self.apply(fx)
    }

    /// The file dialog closed: what was chosen answers the ask it was open for (each value checked by the core where
    /// the ask says so; several values run the action once each); nothing chosen gives the action up
    fn picked(&mut self, flow: FlowId, paths: Option<Vec<PathBuf>>) -> Task<Message> {
        if self.picking == Some(flow) {
            self.picking = None;
        }
        // (the dialog of an action that was given up meanwhile answers nothing: the session knows its flow no more)
        let values: Vec<String> =
            paths.unwrap_or_default().into_iter().map(|p| p.to_string_lossy().into_owned()).collect();
        let fx = self.session.reply(flow, values);
        self.apply(fx)
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
            "filter" => {
                let page = str_of("page").unwrap_or_else(|| self.tab.clone());
                self.session.filters.set(&page, p["filter"].clone());
                self.filter_menu = None;
                self.resubscribe()
            }
            "field" => {
                // `{page, params, field, valueParam, value?}`: `values.<valueParam>` of that page; `value: null` drops it
                // (the asks ran before, so an absent value means nothing was chosen)
                let (Some(page), Some(key)) = (str_of("page"), str_of("valueParam")) else {
                    self.say("ui.field without page or valueParam", true);
                    return Task::none();
                };
                let Some(v) = p.get("value") else { return Task::none() };
                // `also`: more values of the same page at once (plan B147: a key and the mode after it)
                for m in p.get("also").and_then(Value::as_array).cloned().unwrap_or_default() {
                    if let Some(k) = m.get("valueParam").and_then(Value::as_str) {
                        self.set_form(page.clone(), k.to_string(), m.get("value").cloned());
                    }
                }
                self.set_form(page, key, Some(v.clone()));
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
                self.say(trf("This app does not know ui.%@", &[other.into()]), true);
                Task::none()
            }
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
        d = d.add_filter(tr("macOS restore image"), &["ipsw"]);
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
