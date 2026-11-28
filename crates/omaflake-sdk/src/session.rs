// SPDX-License-Identifier: AGPL-3.0-or-later
//! What every interface of the page models does the same way (docs/api.md §10), once: which pages are subscribed and
//! their models, an action's way from the click to the core – its asks with their checks, its confirmation, the call,
//! what follows –, the jobs, their logs and what is new in the activity. `omaf ui` and the Linux app are built on it;
//! an interface of your own can be too.
//!
//! A [`Session`] is a state machine without threads, sockets or a toolkit. It gets what happens – the connection
//! came or went, an event of the core, the answer to a request, what the user wants – and each time returns
//! [`Effect`]s: requests to send, and what changed to show. The interface carries them out:
//!
//! ```no_run
//! use std::collections::VecDeque;
//!
//! use omaflake_sdk::client::Client;
//! use omaflake_sdk::session::{Effect, Session};
//! use omaflake_sdk::views::PageRef;
//! use serde_json::{Value, json};
//!
//! let omaf = Client::connect(std::path::Path::new("/Users/me/.omaflake/run/omaf.sock"))?;
//! let init = omaf.call("initialize", json!({"client": "my-tool", "clientVersion": "1", "apiVersion": 1}))?;
//! let mut session = Session::new();
//! let mut effects = VecDeque::from(session.connected(&init));
//! effects.extend(session.show(vec![PageRef::new("vms", Value::Null)]));
//! while let Some(effect) = effects.pop_front() {
//!     match effect {
//!         // (an interface sends each from a thread of its own and feeds events with `Session::event`)
//!         Effect::Send(r) => effects.extend(session.answer(r.id, omaf.call(&r.method, r.params))),
//!         Effect::Model(page) => println!("{}: {:?}", page.page, session.model(&page)),
//!         _ => {}
//!     }
//! }
//! # Ok::<(), omaflake_sdk::protocol::RpcError>(())
//! ```
//!
//! What it takes care of:
//!
//! - **Order.** Requests leave as [`Effect::Send`], in the order they are to be sent. The core answers a connection's
//!   requests side by side, so what replaces state there – `views.subscribe`, `events.subscribe`, a field's `set` –
//!   goes one at a time: the next is sent when the one before is answered, and of several page sets wanted meanwhile
//!   only the last is subscribed.
//! - **A new connection.** Job ids start again with a new service, so nothing of the old one is kept: jobs, logs,
//!   requests on their way, a check that was running. The pages wanted are subscribed again.
//! - **Questions.** One action is asked at a time ([`Session::question`]); each has a [`FlowId`], and an answer, a
//!   confirmation or a check's verdict counts only for the question that is open. A cancelled action never goes on.
//! - **Logs.** A log is read with `jobs.log` only, from the line after the last one read (`next`); a `job.log` event
//!   only says that there is more. So no line comes twice and none is lost.
//! - **Bounds.** A page's model is kept while the page is wanted, and the newest of each page name; a log while a
//!   kept model shows it; a job while it waits or runs.

use std::collections::{BTreeMap, HashMap, VecDeque};

use serde_json::{Map, Value, json};

use crate::protocol::RpcError;
use crate::types::JobDto;
use crate::views::PageRef;
use crate::views::common::set_param;

/// The lines a log keeps – the newest, as the core does (api.md §5)
pub const LOG_LINES: usize = 5000;

/// A request to send to the core; its answer goes back with [`Session::answer`] and this `id`
#[derive(Debug, Clone, PartialEq)]
pub struct Request {
    pub id: u64,
    pub method: String,
    pub params: Value,
}

/// An action on its way (from [`Session::start`] until it is sent or cancelled). Never used twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct FlowId(u64);

/// What the interface does for the session
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    /// Send this request – in the order the effects come
    Send(Request),
    /// This page's model came or changed ([`Session::model`])
    Model(PageRef),
    /// The open question is another one now, or none ([`Session::question`]): asked, being checked, refused, closed
    Question,
    /// The jobs that wait or run changed ([`Session::jobs`])
    Jobs,
    /// This job's log has more lines ([`Session::log`])
    Log(String),
    /// The activity list has entries not seen before
    Activity(News),
    /// A verb the interface does itself (api.md §10.4: `show`, `back`, `filter`, `field`, `reveal`, `open`,
    /// `terminal`, `quit` – `step` is the session's): `ui.<verb>` with these params
    Ui { verb: String, params: Value },
    /// The core did this action: its `then` says what follows with the `result` (`reveal`, `terminal`, `back`)
    Done { action: Value, result: Value },
    /// The answer to a request of the interface's own ([`Session::call`])
    Answered { method: String, result: Value },
    /// Something to tell the user
    Notice(Notice),
}

#[derive(Debug, Clone, PartialEq)]
pub enum Notice {
    /// The core refused or failed this request
    Failed { method: String, error: RpcError },
    /// The action cannot be done now; `reason` is its text (api.md §10.2) when it has one
    Disabled { reason: Option<Value> },
    /// A step (`ui.step`) leads nowhere: the `reason` its page gives (a text), `name` what it was about
    Refused { reason: Option<Value>, name: String },
    /// Some of several chosen values were refused by their check; the action goes on with the others
    Skipped { refused: Vec<Refusal> },
    /// No connection to the core: this request was not sent
    NotConnected { method: String },
}

/// What an action asks before it is sent
#[derive(Debug, Clone, PartialEq)]
pub enum Question {
    /// A value (the action's `ask[index]`): answered with [`Session::reply`]
    Ask {
        flow: FlowId,
        action: Value,
        ask: Value,
        index: usize,
        /// The params as filled so far
        params: Value,
        /// The core checks the answer (`ask.check`): nothing to do but wait, cancel or change it
        /// ([`Session::retract`])
        checking: bool,
        /// Why the last answer was not taken; asked again
        refused: Vec<Refusal>,
    },
    /// The action's `confirm`, for these params: answered with [`Session::confirm`]
    Confirm { flow: FlowId, action: Value, confirm: Value, params: Value },
}

impl Question {
    pub fn flow(&self) -> FlowId {
        match self {
            Question::Ask { flow, .. } | Question::Confirm { flow, .. } => *flow,
        }
    }
}

/// A value its check did not accept: the check's `reason` (a text), or the error of the check itself
#[derive(Debug, Clone, PartialEq)]
pub struct Refusal {
    pub value: String,
    pub reason: Option<Value>,
    pub error: Option<RpcError>,
}

/// What is new in the activity list (api.md §10.6)
#[derive(Debug, Clone, PartialEq)]
pub struct News {
    /// The entries above the highest id seen so far, newest first
    pub fresh: Vec<Value>,
    /// The first list of this session: nothing in it is news
    pub first: bool,
    /// The ids started again (a new service): the mark was forgotten, `fresh` is its whole list
    pub restarted: bool,
    /// The highest id in the list
    pub newest: Option<u64>,
}

/// The filters of list pages, by page (`params.filter`); the interface asks its pages with them
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Filters(HashMap<String, Value>);

impl Filters {
    pub fn get(&self, page: &str) -> Option<&Value> {
        self.0.get(page)
    }

    /// Sets a page's whole filter; `null` or an empty one removes it (`ui.filter`)
    pub fn set(&mut self, page: &str, filter: Value) {
        if filter.is_null() || filter.as_object().is_some_and(Map::is_empty) {
            self.0.remove(page);
        } else {
            self.0.insert(page.to_string(), filter);
        }
    }

    /// Sets one group of a page's filter; `null`, `false` and `""` take it out. A new search starts on its first page.
    pub fn set_group(&mut self, page: &str, group: &str, value: Value) {
        let mut filter = match self.0.remove(page) {
            Some(Value::Object(f)) => f,
            _ => Map::new(),
        };
        if group == "search" {
            filter.remove("page");
        }
        if value.is_null() || value == json!(false) || value == json!("") {
            filter.remove(group);
        } else {
            filter.insert(group.to_string(), value);
        }
        self.set(page, Value::Object(filter));
    }
}

/// What an answer is for
#[derive(Debug)]
enum Purpose {
    Subscribe(Vec<PageRef>),
    /// Only its error is told
    Quiet,
    /// The interface's own call
    Own,
    Action(Value),
    Step,
    Log(String),
    Check {
        flow: FlowId,
        round: u64,
        index: usize,
    },
}

#[derive(Debug)]
struct Pending {
    method: String,
    purpose: Purpose,
}

/// An action being asked
#[derive(Debug)]
struct Flow {
    id: FlowId,
    action: Value,
    params: Value,
    /// The next ask
    ask: usize,
    /// The params sets still to confirm and send (several after an ask of `multiple`)
    queue: VecDeque<Value>,
    /// The asks are through: the queue holds what is sent
    asked: bool,
    /// The params set whose confirmation is open
    confirming: Option<Value>,
    check: Option<Check>,
    refused: Vec<Refusal>,
}

/// An answer the core is checking, value by value
#[derive(Debug)]
struct Check {
    round: u64,
    values: Vec<String>,
    verdicts: Vec<Option<Result<(), Refusal>>>,
}

#[derive(Debug)]
struct Held {
    page: PageRef,
    model: Value,
    /// It came over this connection (an older one may name jobs of a service that is gone)
    fresh: bool,
}

#[derive(Debug, Default)]
struct Log {
    lines: Vec<String>,
    /// The index of the next line to read
    next: usize,
    /// The `jobs.log` on its way
    request: Option<u64>,
    /// There is more than the request on its way asked for
    more: bool,
    loaded: bool,
    failed: bool,
}

/// The state every interface of the page models keeps – see the module
#[derive(Debug, Default)]
pub struct Session {
    connected: bool,
    init: Value,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    /// Requests sent one at a time, in this order
    lane: VecDeque<(String, Value, Purpose)>,
    /// The one of them on its way
    lane_busy: Option<u64>,
    wanted: Vec<PageRef>,
    /// What the core has subscribed for this connection, as far as it answered
    subscribed: Option<Vec<PageRef>>,
    /// The page set of the `views.subscribe` on its way
    subscribing: Option<Vec<PageRef>>,
    models: Vec<Held>,
    /// The newest model's page per page name
    latest: HashMap<String, PageRef>,
    pub filters: Filters,
    flow: Option<Flow>,
    next_flow: u64,
    next_round: u64,
    /// Actions of steps, waiting for the question that is open
    steps: VecDeque<Value>,
    jobs: BTreeMap<String, JobDto>,
    logs: HashMap<String, Log>,
    /// The highest activity id seen
    activity: Option<u64>,
    out: Vec<Effect>,
}

impl Session {
    pub fn new() -> Session {
        Session::default()
    }

    // MARK: The connection

    /// A connection to the core is there (`init`: what `initialize` answered). Nothing of an earlier one is kept –
    /// its jobs, logs and requests are gone, and job ids start again with a new service (api.md §5) –, the events of
    /// jobs and the pages wanted are subscribed.
    pub fn connected(&mut self, init: &Value) -> Vec<Effect> {
        self.turn(|s| {
            s.forget_connection();
            s.connected = true;
            s.init = init.clone();
            // Models of the connection before stay to look at until theirs come; pages nobody looks at go
            let wanted = s.wanted.clone();
            s.models.retain(|h| wanted.contains(&h.page));
            s.models.iter_mut().for_each(|h| h.fresh = false);
            let models = &s.models;
            s.latest.retain(|_, p| models.iter().any(|h| h.page == *p));
            s.lane.push_back(("events.subscribe".into(), json!({"topics": ["jobs"]}), Purpose::Quiet));
            s.subscribe();
            s.pump();
        })
    }

    /// The connection closed. Requests on their way get no answer that counts; a new connection starts afresh.
    pub fn disconnected(&mut self) -> Vec<Effect> {
        self.turn(|s| {
            s.forget_connection();
            s.connected = false;
        })
    }

    fn forget_connection(&mut self) {
        self.pending.clear();
        self.lane.clear();
        self.lane_busy = None;
        self.subscribed = None;
        self.subscribing = None;
        if !self.jobs.is_empty() {
            self.jobs.clear();
            self.out.push(Effect::Jobs);
        }
        self.logs.clear();
        if let Some(flow) = &mut self.flow {
            // Its answer will not come: the question is open again
            flow.check = None;
        }
    }

    pub fn is_connected(&self) -> bool {
        self.connected
    }

    /// What `initialize` answered on this connection
    pub fn init(&self) -> &Value {
        &self.init
    }

    // MARK: Pages

    /// The pages to follow from now on, with their params (a page with other params is another page). Subscribes them
    /// when they differ from what is subscribed; models of pages no longer wanted go, but for the newest of each
    /// page name.
    pub fn show(&mut self, pages: Vec<PageRef>) -> Vec<Effect> {
        self.turn(|s| {
            let mut wanted: Vec<PageRef> = Vec::with_capacity(pages.len());
            for mut p in pages {
                if !p.params.is_object() {
                    p.params = json!({});
                }
                if !wanted.contains(&p) {
                    wanted.push(p);
                }
            }
            s.wanted = wanted;
            s.subscribe();
            s.pump();
            s.prune();
        })
    }

    /// The pages wanted ([`Session::show`])
    pub fn wanted(&self) -> &[PageRef] {
        &self.wanted
    }

    /// The model of exactly this page
    pub fn model(&self, page: &PageRef) -> Option<&Value> {
        self.models.iter().find(|h| h.page == *page).map(|h| &h.model)
    }

    /// The model of this page, else the newest of a page of that name – what to show while the model for new params
    /// (a filter, a form value) is on its way
    pub fn model_or_latest(&self, page: &PageRef) -> Option<&Value> {
        self.model(page).or_else(|| self.model(self.latest.get(&page.page)?))
    }

    /// The page whose model is the newest of that page name
    pub fn latest(&self, name: &str) -> Option<&PageRef> {
        self.latest.get(name)
    }

    /// Is this page's model kept
    pub fn holds(&self, page: &PageRef) -> bool {
        self.models.iter().any(|h| h.page == *page)
    }

    /// The pages whose models are kept
    pub fn held(&self) -> impl Iterator<Item = &PageRef> {
        self.models.iter().map(|h| &h.page)
    }

    /// `views.subscribe` when the pages wanted are not what the core has, or will have once everything on its way is
    /// answered. At most one waits to be sent – the last one wanted.
    fn subscribe(&mut self) {
        if !self.connected {
            return;
        }
        self.lane.retain(|(_, _, purpose)| !matches!(purpose, Purpose::Subscribe(_)));
        let will_have = self.subscribing.as_ref().or(self.subscribed.as_ref());
        match will_have {
            Some(pages) if *pages == self.wanted => return,
            None if self.wanted.is_empty() => return,
            _ => {}
        }
        let pages = self.wanted.clone();
        self.lane.push_back(("views.subscribe".into(), json!({"pages": pages}), Purpose::Subscribe(pages)));
    }

    fn prune(&mut self) {
        let (wanted, latest) = (&self.wanted, &self.latest);
        self.models.retain(|h| wanted.contains(&h.page) || latest.get(&h.page.page) == Some(&h.page));
        let models = &self.models;
        self.logs.retain(|job, _| models.iter().any(|h| log_ref(&h.model).is_some_and(|l| l.0 == job)));
    }

    // MARK: What the core says

    /// An event of the core: `view.changed`, `jobs.changed`, `job.log` (others are not the session's)
    pub fn event(&mut self, method: &str, params: &Value) -> Vec<Effect> {
        self.turn(|s| match method {
            "view.changed" => {
                let page = PageRef::new(params["page"].as_str().unwrap_or_default(), params["params"].clone());
                s.on_model(page, params["model"].clone());
            }
            "jobs.changed" => {
                let Ok(job) = serde_json::from_value::<JobDto>(params["job"].clone()) else { return };
                // An ended job is the activity's and its page's; here are those that wait or run
                let changed = if job.state.is_active() {
                    s.jobs.insert(job.id.clone(), job);
                    true
                } else {
                    s.jobs.remove(&job.id).is_some()
                };
                if changed {
                    s.out.push(Effect::Jobs);
                }
            }
            "job.log" => {
                // Its lines are read with jobs.log: the event has no index, and lines it brings may be in an answer
                // already (or come before the answer they belong behind)
                let job = params["id"].as_str().unwrap_or_default();
                if s.logs.contains_key(job) {
                    s.read_log(job, true);
                }
            }
            _ => {}
        })
    }

    fn on_model(&mut self, page: PageRef, model: Value) {
        let log = log_ref(&model).map(|(job, from, lines)| (job.to_string(), from, lines));
        let activity = (page.page == "activity").then(|| self.news(&model));
        match self.models.iter_mut().find(|h| h.page == page) {
            Some(held) => {
                held.model = model;
                held.fresh = true;
            }
            None => self.models.push(Held { page: page.clone(), model, fresh: true }),
        }
        self.latest.insert(page.page.clone(), page.clone());
        self.prune();
        if let Some((job, from, lines)) = log
            && self.wanted.contains(&page)
        {
            match self.logs.get(&job) {
                None => {
                    self.logs.insert(job.clone(), Log { next: from, ..Log::default() });
                    self.read_log(&job, true);
                }
                Some(log) => {
                    let behind = lines.is_some_and(|n| n > log.next);
                    self.read_log(&job, behind);
                }
            }
        }
        self.out.push(Effect::Model(page));
        if let Some(news) = activity.flatten() {
            self.out.push(Effect::Activity(news));
        }
    }

    /// Reads on in a job's log when there is `more` (or more was announced while nobody showed it) – one request at
    /// a time, and only while a page wanted shows it
    fn read_log(&mut self, job: &str, more: bool) {
        let shown = self
            .models
            .iter()
            .any(|h| h.fresh && self.wanted.contains(&h.page) && log_ref(&h.model).is_some_and(|l| l.0 == job));
        let Some(log) = self.logs.get_mut(job) else { return };
        log.more |= more;
        if !log.more || log.request.is_some() || log.failed || !shown {
            return;
        }
        let from = log.next;
        if let Some(id) = self.request("jobs.log", json!({"id": job, "from": from}), Purpose::Log(job.to_string()))
            && let Some(log) = self.logs.get_mut(job)
        {
            log.request = Some(id);
            log.more = false;
        }
    }

    /// What an activity list brings that was not seen: entries above the mark. The first list only sets the mark; a
    /// newest id below the mark is a new server's (its ids start at 1 again), so the mark is forgotten.
    fn news(&mut self, model: &Value) -> Option<News> {
        let entries = model["entries"].as_array().map(Vec::as_slice).unwrap_or_default();
        let id = |e: &Value| e.get("id").and_then(Value::as_u64);
        let newest = entries.iter().filter_map(id).max();
        let (seen, restarted) = match (self.activity, newest) {
            (Some(mark), Some(n)) if n < mark => (Some(0), true),
            (mark, _) => (mark, false),
        };
        self.activity = newest.or(seen);
        let Some(seen) = seen else {
            return Some(News { fresh: Vec::new(), first: true, restarted: false, newest });
        };
        let fresh: Vec<Value> = entries.iter().filter(|e| id(e).is_some_and(|i| i > seen)).cloned().collect();
        (restarted || !fresh.is_empty()).then_some(News { fresh, first: false, restarted, newest })
    }

    /// The answer to the request `id` ([`Effect::Send`]). An answer nobody waits for – of a connection that is gone,
    /// a cancelled action, a retracted check – changes nothing.
    pub fn answer(&mut self, id: u64, result: Result<Value, RpcError>) -> Vec<Effect> {
        self.turn(|s| {
            let Some(Pending { method, purpose }) = s.pending.remove(&id) else { return };
            if s.lane_busy == Some(id) {
                s.lane_busy = None;
            }
            s.answered(id, method, purpose, result);
            s.pump();
        })
    }

    fn answered(&mut self, id: u64, method: String, purpose: Purpose, result: Result<Value, RpcError>) {
        match (purpose, result) {
            (Purpose::Subscribe(pages), result) => {
                self.subscribing = None;
                match result {
                    Ok(_) => self.subscribed = Some(pages),
                    Err(error) => {
                        // What the core has now is not known: the next `show` subscribes again
                        self.subscribed = None;
                        self.out.push(Effect::Notice(Notice::Failed { method, error }));
                    }
                }
                // Pages wanted meanwhile
                self.subscribe_if_behind();
            }
            (Purpose::Check { flow, round, index }, result) => self.checked(flow, round, index, result),
            (Purpose::Log(job), result) => self.log_answered(id, job, method, result),
            (Purpose::Action(action), Ok(result)) => self.out.push(Effect::Done { action, result }),
            (Purpose::Step, Ok(page)) => match page.get("action").filter(|a| a.is_object()) {
                Some(action) if page.get("valid").and_then(Value::as_bool) != Some(false) => {
                    self.steps.push_back(action.clone());
                    self.next_step();
                }
                _ => {
                    let name = page["name"].as_str().unwrap_or_default().to_string();
                    self.out.push(Effect::Notice(Notice::Refused { reason: page.get("reason").cloned(), name }));
                }
            },
            (Purpose::Own, Ok(result)) => self.out.push(Effect::Answered { method, result }),
            (Purpose::Quiet, Ok(_)) => {}
            (_, Err(error)) => self.out.push(Effect::Notice(Notice::Failed { method, error })),
        }
    }

    /// After a failed subscription nothing is sent again by itself (it would fail again); after one that worked, the
    /// page set wanted meanwhile follows
    fn subscribe_if_behind(&mut self) {
        if self.subscribed.is_some() {
            self.subscribe();
        }
    }

    // MARK: Requests

    /// Registers a request and hands it out; None without a connection
    fn request(&mut self, method: &str, params: Value, purpose: Purpose) -> Option<u64> {
        if !self.connected {
            return None;
        }
        self.next_id += 1;
        let id = self.next_id;
        if let Purpose::Subscribe(pages) = &purpose {
            self.subscribing = Some(pages.clone());
        }
        self.pending.insert(id, Pending { method: method.to_string(), purpose });
        self.out.push(Effect::Send(Request { id, method: method.to_string(), params }));
        Some(id)
    }

    /// A request of the user's: without a connection it is told
    fn send(&mut self, method: &str, params: Value, purpose: Purpose) -> Option<u64> {
        let id = self.request(method, params, purpose);
        if id.is_none() {
            self.out.push(Effect::Notice(Notice::NotConnected { method: method.to_string() }));
        }
        id
    }

    /// Sends the next of the requests that go one at a time
    fn pump(&mut self) {
        while self.connected && self.lane_busy.is_none() {
            let Some((method, params, purpose)) = self.lane.pop_front() else { return };
            self.lane_busy = self.request(&method, params, purpose);
        }
    }

    /// A request of the interface's own: its result comes as [`Effect::Answered`], its error as a notice
    pub fn call(&mut self, method: &str, params: Value) -> Vec<Effect> {
        self.turn(|s| {
            s.send(method, params, Purpose::Own);
        })
    }

    /// Stores a value in the core (a field's `set`): sent after the ones before it are answered, so that the last
    /// value set is the one that stays; only an error is told
    pub fn set(&mut self, method: &str, params: Value) -> Vec<Effect> {
        self.turn(|s| {
            if !s.connected {
                s.out.push(Effect::Notice(Notice::NotConnected { method: method.to_string() }));
                return;
            }
            s.lane.push_back((method.to_string(), params, Purpose::Quiet));
            s.pump();
        })
    }

    // MARK: Actions

    /// The user asks for `action` (an action of a page model, api.md §10.3): not enabled – its reason is told; with
    /// `ask` or `confirm` – it becomes the open question (in place of one that was open, which is cancelled); else it
    /// is sent at once, beside whatever is open.
    pub fn start(&mut self, action: &Value) -> Vec<Effect> {
        self.turn(|s| s.begin(action.clone()))
    }

    fn begin(&mut self, action: Value) {
        if action.get("enabled").and_then(Value::as_bool) != Some(true) {
            let reason = action.get("reason").filter(|r| !r.is_null()).cloned();
            self.out.push(Effect::Notice(Notice::Disabled { reason }));
            return;
        }
        let params = action.get("params").cloned().unwrap_or(Value::Null);
        if asks(&action) == 0 && !action["confirm"].is_object() {
            self.dispatch(&action, params);
            return;
        }
        self.next_flow += 1;
        self.flow = Some(Flow {
            id: FlowId(self.next_flow),
            action,
            params,
            ask: 0,
            queue: VecDeque::new(),
            asked: false,
            confirming: None,
            check: None,
            refused: Vec::new(),
        });
        self.advance();
    }

    /// The open question, if an action waits for the user
    pub fn question(&self) -> Option<Question> {
        let flow = self.flow.as_ref()?;
        if let Some(params) = &flow.confirming {
            return Some(Question::Confirm {
                flow: flow.id,
                action: flow.action.clone(),
                confirm: flow.action["confirm"].clone(),
                params: params.clone(),
            });
        }
        let ask = flow.action["ask"].get(flow.ask)?;
        Some(Question::Ask {
            flow: flow.id,
            action: flow.action.clone(),
            ask: ask.clone(),
            index: flow.ask,
            params: flow.params.clone(),
            checking: flow.check.is_some(),
            refused: flow.refused.clone(),
        })
    }

    /// The flow `id`, while it is the open one
    fn open_flow(&mut self, id: FlowId) -> Option<&mut Flow> {
        self.flow.as_mut().filter(|f| f.id == id)
    }

    /// The next step of the open action: its next ask, the confirmation of a params set, or sending it
    fn advance(&mut self) {
        while let Some(flow) = &mut self.flow {
            if flow.confirming.is_some() || flow.ask < asks(&flow.action) {
                return;
            }
            if !flow.asked {
                flow.asked = true;
                if flow.queue.is_empty() {
                    flow.queue.push_back(flow.params.clone());
                }
            }
            let Some(params) = flow.queue.pop_front() else {
                self.flow = None;
                break;
            };
            if flow.action["confirm"].is_object() {
                flow.confirming = Some(params);
                return;
            }
            let action = flow.action.clone();
            self.dispatch(&action, params);
        }
        self.next_step();
    }

    /// The action of a step that waited for the open question
    fn next_step(&mut self) {
        while self.flow.is_none() {
            let Some(action) = self.steps.pop_front() else { return };
            self.begin(action);
        }
    }

    /// The answer to the open ask of `flow`: one value, or several for an ask of `multiple` (the action is then done
    /// once per value); none cancels. With a `check`, the core is asked first: values it refuses are left out, and
    /// when none is left the ask stays open with the refusals. An answer to another question than the open one, or
    /// while a check runs, changes nothing.
    pub fn reply(&mut self, flow: FlowId, values: Vec<String>) -> Vec<Effect> {
        self.turn(|s| {
            let Some(f) = s.open_flow(flow).filter(|f| f.confirming.is_none() && f.check.is_none()) else { return };
            let Some(ask) = f.action["ask"].get(f.ask).cloned() else { return };
            if values.is_empty() {
                s.flow = None;
                s.next_step();
                return;
            }
            f.refused.clear();
            let Some(check) = ask.get("check").filter(|c| c.is_object()) else {
                s.apply(values);
                return;
            };
            if !s.connected {
                s.out.push(Effect::Notice(Notice::NotConnected {
                    method: check["method"].as_str().unwrap_or_default().to_string(),
                }));
                return;
            }
            s.next_round += 1;
            let round = s.next_round;
            for (index, value) in values.iter().enumerate() {
                let mut params = check.get("params").filter(|p| p.is_object()).cloned().unwrap_or_else(|| json!({}));
                set_param(&mut params, check["param"].as_str().unwrap_or_default(), json!(value));
                let method = check["method"].as_str().unwrap_or_default();
                s.request(method, params, Purpose::Check { flow, round, index });
            }
            if let Some(f) = s.open_flow(flow) {
                f.check = Some(Check { round, verdicts: vec![None; values.len()], values });
            }
        })
    }

    /// A check's verdict on one value – for the check that runs, of the question that is open; any other is dropped
    fn checked(&mut self, flow: FlowId, round: u64, index: usize, result: Result<Value, RpcError>) {
        let Some(f) = self.open_flow(flow) else { return };
        let Some(check) = f.check.as_mut().filter(|c| c.round == round) else { return };
        let Some(value) = check.values.get(index).cloned() else { return };
        check.verdicts[index] = Some(match result {
            Ok(v) if v.get("valid").and_then(Value::as_bool) == Some(false) => {
                Err(Refusal { value, reason: v.get("reason").filter(|r| !r.is_null()).cloned(), error: None })
            }
            Ok(_) => Ok(()),
            Err(error) => Err(Refusal { value, reason: None, error: Some(error) }),
        });
        if check.verdicts.iter().any(Option::is_none) {
            return;
        }
        let Some(check) = f.check.take() else { return };
        let (mut accepted, mut refused) = (Vec::new(), Vec::new());
        for (value, verdict) in check.values.into_iter().zip(check.verdicts) {
            match verdict {
                Some(Err(refusal)) => refused.push(refusal),
                _ => accepted.push(value),
            }
        }
        if accepted.is_empty() {
            f.refused = refused;
            return;
        }
        if !refused.is_empty() {
            self.out.push(Effect::Notice(Notice::Skipped { refused }));
        }
        self.apply(accepted);
    }

    /// The open ask got its values
    fn apply(&mut self, values: Vec<String>) {
        let Some(f) = &mut self.flow else { return };
        let ask = &f.action["ask"][f.ask];
        let param = ask["param"].as_str().unwrap_or_default().to_string();
        if ask["multiple"] == json!(true) && values.len() > 1 {
            // Once per value: each with what was asked before; what is asked after goes into all of them
            let sets: Vec<Value> =
                if f.queue.is_empty() { vec![f.params.clone()] } else { f.queue.drain(..).collect() };
            for set in sets {
                for v in &values {
                    let mut p = set.clone();
                    set_param(&mut p, &param, json!(v));
                    f.queue.push_back(p);
                }
            }
        } else {
            set_param(&mut f.params, &param, json!(values[0]));
            for p in f.queue.iter_mut() {
                set_param(p, &param, json!(values[0]));
            }
        }
        f.ask += 1;
        self.advance();
    }

    /// The user changes the answer the core is checking, or one that was refused: the check's verdict no longer
    /// counts, the refusals go
    pub fn retract(&mut self, flow: FlowId) -> Vec<Effect> {
        self.turn(|s| {
            if let Some(f) = s.open_flow(flow) {
                f.check = None;
                f.refused.clear();
            }
        })
    }

    /// Yes to the open confirmation of `flow`; `input` is the text its question asked for (`confirm.input`). The
    /// action is sent.
    pub fn confirm(&mut self, flow: FlowId, input: Option<String>) -> Vec<Effect> {
        self.turn(|s| {
            let Some(f) = s.open_flow(flow) else { return };
            let Some(mut params) = f.confirming.take() else { return };
            let action = f.action.clone();
            if let (Some(i), Some(text)) = (action["confirm"].get("input").filter(|i| i.is_object()), input) {
                set_param(&mut params, i["param"].as_str().unwrap_or_default(), json!(text));
            }
            s.dispatch(&action, params);
            s.advance();
        })
    }

    /// The user gives the action of `flow` up: nothing more of it is sent, whatever answer still comes
    pub fn cancel(&mut self, flow: FlowId) -> Vec<Effect> {
        self.turn(|s| {
            if s.open_flow(flow).is_some() {
                s.flow = None;
                s.next_step();
            }
        })
    }

    /// Sends an action with its params: a core method, the step the session does, or a verb for the interface
    fn dispatch(&mut self, action: &Value, params: Value) {
        let method = action["method"].as_str().unwrap_or_default();
        match method.strip_prefix("ui.") {
            Some("step") => {
                let params = json!({"page": params["page"], "params": params["params"]});
                self.send("views.get", params, Purpose::Step);
            }
            Some(verb) => self.out.push(Effect::Ui { verb: verb.to_string(), params }),
            None => {
                self.send(method, params, Purpose::Action(action.clone()));
            }
        }
    }

    // MARK: Jobs and logs

    /// The jobs that wait or run, as the core told them (`jobs.changed`), by id
    pub fn jobs(&self) -> impl Iterator<Item = &JobDto> {
        self.jobs.values()
    }

    /// A job's log as far as it was read (the newest [`LOG_LINES`]); None until its first lines came. Read for the
    /// jobs whose log a page wanted shows (the model's `log`, a LogRef).
    pub fn log(&self, job: &str) -> Option<&[String]> {
        self.logs.get(job).filter(|l| l.loaded).map(|l| l.lines.as_slice())
    }

    fn log_answered(&mut self, id: u64, job: String, method: String, result: Result<Value, RpcError>) {
        let Some(log) = self.logs.get_mut(&job).filter(|l| l.request == Some(id)) else { return };
        log.request = None;
        match result {
            Ok(answer) => {
                let lines: Vec<String> = answer["lines"]
                    .as_array()
                    .map(|a| {
                        a.iter().map(|l| l.as_str().map(str::to_string).unwrap_or_else(|| l.to_string())).collect()
                    })
                    .unwrap_or_default();
                // The lines end right before `next`. Those before what is here already are left out (a core that
                // answers from further back than asked); a gap – lines the core no longer keeps – is just skipped.
                let next = answer["next"].as_u64().map(|n| n as usize).unwrap_or(log.next + lines.len());
                let first = next.saturating_sub(lines.len());
                let known = log.next.saturating_sub(first).min(lines.len());
                log.lines.extend(lines.into_iter().skip(known));
                let over = log.lines.len().saturating_sub(LOG_LINES);
                log.lines.drain(..over);
                log.next = log.next.max(next);
                log.loaded = true;
                self.out.push(Effect::Log(job.clone()));
                self.read_log(&job, false);
            }
            Err(error) => {
                // Not asked again on this connection (a job that is gone would fail at every look)
                log.failed = true;
                self.out.push(Effect::Notice(Notice::Failed { method, error }));
            }
        }
    }

    // MARK: Turns

    /// One input: what `f` does, and whether the open question differs afterwards
    fn turn(&mut self, f: impl FnOnce(&mut Session)) -> Vec<Effect> {
        let before = self.question();
        f(self);
        if self.question() != before {
            self.out.push(Effect::Question);
        }
        std::mem::take(&mut self.out)
    }
}

/// How many values an action asks
fn asks(action: &Value) -> usize {
    action["ask"].as_array().map(Vec::len).unwrap_or(0)
}

/// The job log a page's model shows (its `log`, a LogRef `{job, from, lines}`): the job, the first line the core
/// still has, the lines so far
fn log_ref(model: &Value) -> Option<(&str, usize, Option<usize>)> {
    let log = model.get("log")?;
    let job = log.get("job")?.as_str().filter(|j| !j.is_empty())?;
    let n = |key: &str| log.get(key).and_then(Value::as_u64).map(|n| n as usize);
    Some((job, n("from").unwrap_or(0), n("lines")))
}

#[cfg(test)]
mod tests;
