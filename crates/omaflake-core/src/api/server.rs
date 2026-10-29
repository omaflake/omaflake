// SPDX-License-Identifier: AGPL-3.0-or-later
//! The server: one per user as the core service (`omaf serve --socket`, plan B120), one per UI session (`omaf serve`
//! on stdio) or per in-process UI. It holds what the app's `AppModel` held – the recipe list, the Doctor's results,
//! the catalogs, the VMs it is acting on or was asked to stop – and runs the jobs and the watcher. Requests are
//! dispatched in `methods.rs`.
//!
//! A `Server` value is one **connection** to that shared state: `Server::new` makes the state and its first
//! connection, `connect` adds one (each socket client of the service), `disconnect` ends it. A connection has its own
//! session (`initialize`, language, update feed) and subscription; jobs, the watcher, the build queue and the caches
//! are shared, and events go to every connection subscribed to their topic.
//!
//! Every method runs on the caller's thread and blocks until it is done; the stdio transport gives each request its
//! own thread, so a slow request never holds up a fast one. Shared state sits behind short-lived mutexes.

use std::collections::{HashMap, HashSet};
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard, RwLock};
use std::time::{Duration, Instant, SystemTime};

use serde::Serialize;
use serde_json::Value;

use super::events::{ConnId, Events, Notify, Topic};
use super::jobs::{Job, Jobs, panic_text};
use super::model::{self, Doctor, RecipeCache, Snapshot, View};
use super::protocol::{self, Incoming, RpcError};
use super::types::{JobDto, Via, VmEnded, VmReady};
use crate::backend::{Backends, Readiness};
use crate::error::{Error, codes};
use crate::instance_store::InstanceStore;
use crate::models::{GuestKind, NetworkMode, VmStatus};
use crate::paths::{Paths, Settings};
use crate::qemu_runtime::InstanceLock;
use crate::recipe_sources::RecipeCatalog;
use crate::sockets::Ports;
use crate::spawn::Pid;

/// The watcher looks at the data folder this often while a client is subscribed (api.md §6)
pub(crate) const WATCH_INTERVAL: Duration = Duration::from_secs(2);
/// While pages are subscribed, looks follow changes (jobs, activity) at most this often
const VIEW_SPACING: Duration = Duration::from_millis(300);
/// The recipe folders' modification stamp is compared at most this often (`AppModel.reloadRecipesIfChanged`)
const STAMP_INTERVAL: Duration = Duration::from_secs(5);

/// What a method answers: a value, or a job (or several) – `call_once` waits for jobs, a session gets them at once
pub(crate) enum Answer {
    Value(Value),
    Job(Arc<Job>),
    Jobs(Vec<Arc<Job>>),
}

impl Answer {
    pub fn of<T: Serialize>(value: T) -> Result<Answer, RpcError> {
        serde_json::to_value(value).map(Answer::Value).map_err(|e| RpcError::internal(e.to_string()))
    }
}

pub(crate) type Reply = Result<Answer, RpcError>;

#[derive(Default)]
pub(crate) struct Session {
    pub language: Option<String>,
    pub client_version: Option<String>,
    pub update_feed: Option<String>,
}

#[derive(Default)]
pub(crate) struct Freshness {
    /// Recipe → the store path a build would give now
    pub expected: HashMap<String, String>,
    running: bool,
    again: bool,
}

#[derive(Default)]
struct Stamp {
    value: Option<SystemTime>,
    checked: Option<Instant>,
}

#[derive(Default)]
pub(crate) struct Catalogs {
    pub catalogs: HashMap<String, RecipeCatalog>,
    pub errors: std::collections::BTreeMap<String, String>,
    pub fetched: bool,
    /// They are being fetched now
    pub fetching: bool,
}

#[derive(Default)]
struct Clipboard {
    /// VM → the runner process that got its clipboard agent (a restarted VM gets it again)
    started: HashMap<String, Pid>,
    pending: HashSet<String>,
}

#[derive(Default)]
struct Watch {
    running: bool,
    /// The last list sent per topic and language (as JSON, to compare); the language only for recipes
    last: HashMap<(Topic, Option<String>), Value>,
    /// VM statuses of the last look (for `vm.ended`)
    statuses: Option<Vec<(String, VmStatus)>>,
    poked: bool,
}

pub(crate) struct Inner {
    pub paths: RwLock<Paths>,
    pub events: Arc<Events>,
    pub jobs: Arc<Jobs>,
    pub busy: Mutex<HashSet<String>>,
    /// VMs this server was asked to stop – their end is no news
    pub stop_requested: Mutex<HashSet<String>>,
    pub recipes: Mutex<RecipeCache>,
    /// The last error reading the recipes – said once, not at every look
    recipes_error: Mutex<Option<String>>,
    pub freshness: Mutex<Freshness>,
    stamp: Mutex<Stamp>,
    pub catalogs: Mutex<Catalogs>,
    pub doctor: Mutex<Option<Doctor>>,
    /// The Doctor's checks running now (the Doctor page's spinner)
    pub checking: AtomicUsize,
    /// The last `updates.check` (the Doctor page's "Updates")
    pub update: Mutex<super::views::UpdateRecord>,
    /// The last model sent per connection and subscribed page (`view.changed` only when it changes)
    pub views_last: Mutex<HashMap<(ConnId, String), Value>>,
    clipboard: Mutex<Clipboard>,
    watch: Mutex<Watch>,
    wake: Condvar,
    /// The first `initialize` ran (it migrates older layouts once)
    pub migrated: AtomicBool,
    /// The server ends: jobs are cancelled, the watcher and waits stop
    pub shut_down: AtomicBool,
    /// `service.restart` was asked: the socket service ends once no job runs (api.md §1)
    pub restart: AtomicBool,
    next_conn: AtomicU64,
    /// `initialize` migrates older layouts (`ImageStore::migrate_legacy`) – off in protocol tests
    pub migrate_at_start: bool,
}

/// One connection's session: what `initialize` said, and whether it ended (`shutdown`)
#[derive(Default)]
pub(crate) struct Conn {
    pub id: ConnId,
    pub session: Mutex<Session>,
    pub initialized: AtomicBool,
    pub ended: AtomicBool,
}

/// The core API in-process: the methods and JSON shapes of docs/api.md without the pipe. A Rust UI (iced) creates
/// one with its own `notify`, sends `initialize` first and `events.subscribe` when it wants events. Cloning is cheap
/// (shared state, the same connection). `notify` is called from the server's threads and must not block on the
/// server.
#[derive(Clone)]
pub struct Server {
    pub(crate) inner: Arc<Inner>,
    pub(crate) conn: Arc<Conn>,
}

pub(crate) fn lock<T>(m: &Mutex<T>) -> MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Owned state the lists are built from (`View` borrows it)
pub(crate) struct State {
    pub paths: Paths,
    pub snap: Snapshot,
    pub recipes: RecipeCache,
    pub busy: HashSet<String>,
    pub expected: HashMap<String, String>,
    pub language: Option<String>,
    pub developer_mode: bool,
}

impl State {
    pub fn view(&self) -> View<'_> {
        View {
            paths: &self.paths,
            snap: &self.snap,
            recipes: &self.recipes,
            busy: &self.busy,
            expected: &self.expected,
            language: self.language.as_deref(),
            developer_mode: self.developer_mode,
        }
    }
}

/// Marks a VM busy while an action runs on it (`AppModel.perform`)
struct BusyGuard<'a> {
    server: &'a Server,
    name: String,
}

impl Drop for BusyGuard<'_> {
    fn drop(&mut self) {
        lock(&self.server.inner.busy).remove(&self.name);
        self.server.poke();
    }
}

impl Server {
    /// A server on `paths` (`Paths::standard()` for the user's data folder); notifications go to `notify`
    pub fn new(paths: Paths, notify: Arc<Notify>) -> Server {
        Self::bare(paths, notify).with_view_hook()
    }

    fn bare(paths: Paths, notify: Arc<Notify>) -> Server {
        let events = Arc::new(Events::new());
        events.attach(0, notify);
        Server {
            conn: Arc::new(Conn::default()),
            inner: Arc::new(Inner {
                paths: RwLock::new(paths),
                jobs: Jobs::new(events.clone()),
                events,
                busy: Mutex::default(),
                stop_requested: Mutex::default(),
                recipes: Mutex::default(),
                recipes_error: Mutex::default(),
                freshness: Mutex::default(),
                stamp: Mutex::default(),
                catalogs: Mutex::default(),
                doctor: Mutex::default(),
                checking: AtomicUsize::new(0),
                update: Mutex::default(),
                views_last: Mutex::default(),
                clipboard: Mutex::default(),
                watch: Mutex::default(),
                wake: Condvar::new(),
                migrated: AtomicBool::new(false),
                shut_down: AtomicBool::new(false),
                restart: AtomicBool::new(false),
                next_conn: AtomicU64::new(1),
                migrate_at_start: true,
            }),
        }
    }

    /// Job changes and new activity wake the watcher while pages are subscribed (`views.subscribe`)
    fn with_view_hook(self) -> Server {
        let weak = Arc::downgrade(&self.inner);
        self.inner.events.set_on_change(Arc::new(move || {
            if let Some(inner) = weak.upgrade() {
                Server { inner, conn: Arc::new(Conn::default()) }.poke();
            }
        }));
        self
    }

    #[cfg(test)]
    pub(crate) fn for_tests(paths: Paths, notify: Arc<Notify>) -> Server {
        let mut s = Self::bare(paths, notify);
        Arc::get_mut(&mut s.inner).unwrap().migrate_at_start = false;
        s.with_view_hook()
    }

    /// A new connection to the same state (a client of the service): its own session and subscription, the same
    /// jobs, watcher and caches. Its events go to `notify`.
    pub fn connect(&self, notify: Arc<Notify>) -> Server {
        let id = self.inner.next_conn.fetch_add(1, Ordering::SeqCst);
        self.inner.events.attach(id, notify);
        Server { inner: self.inner.clone(), conn: Arc::new(Conn { id, ..Conn::default() }) }
    }

    /// The connection is gone: no more events go to it. What it started (jobs, VM actions) goes on.
    pub fn disconnect(&self) {
        self.conn.ended.store(true, Ordering::SeqCst);
        self.inner.events.detach(self.conn.id);
        // The watcher ends by itself when nobody is subscribed any more
        self.inner.wake.notify_all();
    }

    /// Answers one JSON-RPC request object (None for a notification, which gets no answer)
    pub fn handle(&self, request: Value) -> Option<Value> {
        self.answer(protocol::parse_value(request))
    }

    pub(crate) fn answer(&self, incoming: Incoming) -> Option<Value> {
        match incoming {
            Incoming::Invalid { id, error } => Some(protocol::response_err(id, &error)),
            Incoming::Notification { method, params } => {
                let _ = self.call(&method, params);
                None
            }
            Incoming::Request { id, method, params } => Some(match self.call(&method, params) {
                Ok(v) => protocol::response_ok(id, v),
                Err(e) => protocol::response_err(id, &e),
            }),
        }
    }

    /// Runs one method; a job is answered at once (as it is now)
    pub fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        match self.dispatch_guarded(method, params)? {
            Answer::Value(v) => Ok(v),
            Answer::Job(job) => to_value(&job.dto()),
            Answer::Jobs(jobs) => to_value(&jobs.iter().map(|j| j.dto()).collect::<Vec<_>>()),
        }
    }

    /// Runs one method and waits for the jobs it started; returns them ended
    pub fn call_and_wait(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        match self.dispatch_guarded(method, params)? {
            Answer::Value(v) => Ok(v),
            Answer::Job(job) => to_value(&job.wait()),
            Answer::Jobs(jobs) => to_value(&jobs.iter().map(|j| j.wait()).collect::<Vec<JobDto>>()),
        }
    }

    fn dispatch_guarded(&self, method: &str, params: Value) -> Reply {
        if !self.conn.initialized.load(Ordering::SeqCst) && !matches!(method, "initialize" | "shutdown") {
            return Err(RpcError::not_initialized());
        }
        // What an action did goes into the activity list (views/activity.rs), worded once for every UI
        let action = super::views::activity::is_action(method).then(|| params.clone());
        let reply = match catch_unwind(AssertUnwindSafe(|| self.dispatch(method, params))) {
            Ok(r) => r,
            Err(panic) => Err(RpcError::internal(format!("Internal error: {}", panic_text(&panic)))),
        };
        if let Some(p) = action {
            let outcome = match &reply {
                Ok(Answer::Value(v)) => Ok(v.clone()),
                Ok(_) => Ok(Value::Null),
                Err(e) => Err(e.clone()),
            };
            if let Some((level, text)) = super::views::activity::call_done(method, &p, &outcome) {
                self.events().record(level, text, None);
            }
        }
        reply
    }

    /// `shutdown` was answered on this connection, or the server ends: the transport ends this session
    pub fn is_shut_down(&self) -> bool {
        self.conn.ended.load(Ordering::SeqCst) || self.is_closing()
    }

    /// The whole server ends (`close`): background work stops
    pub(crate) fn is_closing(&self) -> bool {
        self.inner.shut_down.load(Ordering::SeqCst)
    }

    /// `service.restart` was asked
    pub(crate) fn restart_requested(&self) -> bool {
        self.inner.restart.load(Ordering::SeqCst)
    }

    /// A job waits or runs
    pub(crate) fn busy(&self) -> bool {
        self.inner.jobs.any_running()
    }

    /// Ends the session: jobs are cancelled (waiting up to `timeout` for them to stop), the watcher stops. VMs keep
    /// running – they live in their own session.
    pub fn close(&self, timeout: Duration) {
        self.inner.shut_down.store(true, Ordering::SeqCst);
        self.inner.wake.notify_all();
        self.inner.jobs.close(timeout);
    }

    // MARK: State

    pub(crate) fn paths(&self) -> Paths {
        self.inner.paths.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    pub(crate) fn set_paths(&self, paths: Paths) {
        *self.inner.paths.write().unwrap_or_else(|e| e.into_inner()) = paths;
    }

    /// This connection's language (`initialize`)
    pub(crate) fn language(&self) -> Option<String> {
        lock(&self.conn.session).language.clone()
    }

    pub(crate) fn events(&self) -> &Events {
        &self.inner.events
    }

    /// Wakes the watcher for a look now (after an action)
    pub(crate) fn poke(&self) {
        lock(&self.inner.watch).poked = true;
        self.inner.wake.notify_all();
    }

    /// The recipes, read once if they were not yet; an error is kept as the list's state
    pub(crate) fn recipes(&self) -> Result<RecipeCache, Error> {
        {
            let r = lock(&self.inner.recipes);
            if r.loaded {
                return Ok(r.clone());
            }
        }
        self.reload_recipes()?;
        Ok(lock(&self.inner.recipes).clone())
    }

    /// Reads the recipes again (`AppModel.loadRecipes`); on an error the old list stays and the error is a message
    pub(crate) fn reload_recipes(&self) -> Result<(), Error> {
        match RecipeCache::load(&self.paths()) {
            Ok(cache) => {
                *lock(&self.inner.recipes) = cache;
                *lock(&self.inner.recipes_error) = None;
                self.poke();
                Ok(())
            }
            Err(e) => {
                let new =
                    lock(&self.inner.recipes_error).replace(e.message().to_string()).as_deref() != Some(e.message());
                if new {
                    self.events().error(e.message());
                }
                Err(e)
            }
        }
    }

    /// `reload_recipes` on a thread of its own, then the freshness check
    pub(crate) fn reload_recipes_later(&self) {
        let s = self.clone();
        std::thread::spawn(move || {
            let _ = super::jobs::guarded(|| {
                let _ = s.reload_recipes();
                s.check_freshness();
                Ok(())
            });
        });
    }

    /// The store paths a build would give now (`nix eval`, slow) – on a thread; a second call while it runs makes it
    /// run once more after (`AppModel.checkFreshness`)
    pub(crate) fn check_freshness(&self) {
        {
            let mut f = lock(&self.inner.freshness);
            if f.running {
                f.again = true;
                return;
            }
            f.running = true;
        }
        let s = self.clone();
        std::thread::spawn(move || {
            loop {
                let paths = s.paths();
                let defaults = lock(&s.inner.recipes).defaults.clone();
                let expected = super::jobs::guarded(|| {
                    let flake = Settings::load(&paths).flake_path()?;
                    Ok(crate::image_store::ImageStore::new(paths.clone()).expected_store_paths(&defaults, &flake))
                })
                .unwrap_or_default();
                let again = {
                    let mut f = lock(&s.inner.freshness);
                    f.expected = expected;
                    let again = f.again;
                    f.again = false;
                    f.running = again;
                    again
                };
                s.poke();
                if !again {
                    return;
                }
            }
        });
    }

    /// The recipe list follows edits of the recipes: the newest modification in the flake, the guest values and the
    /// recipe folders is compared every 5 s at most; `nix eval` runs only after a change (plan B73, B82)
    pub(crate) fn reload_recipes_if_changed(&self) {
        {
            let mut st = lock(&self.inner.stamp);
            if st.checked.is_some_and(|c| c.elapsed() < STAMP_INTERVAL) {
                return;
            }
            st.checked = Some(Instant::now());
        }
        let stamp = model::newest_change(&model::watched_folders(&self.paths()));
        let first = {
            let mut st = lock(&self.inner.stamp);
            if st.value == Some(stamp) {
                return;
            }
            let first = st.value.is_none();
            st.value = Some(stamp);
            first
        };
        if !first {
            let _ = self.reload_recipes();
        }
        self.check_freshness();
    }

    /// The lists as they are now, in this connection's language
    pub(crate) fn state(&self) -> Result<State, Error> {
        self.state_in(self.language())
    }

    /// The lists as they are now, recipe texts in `language`
    pub(crate) fn state_in(&self, language: Option<String>) -> Result<State, Error> {
        let paths = self.paths();
        let recipes = self.recipes().unwrap_or_else(|_| lock(&self.inner.recipes).clone());
        let snap = Snapshot::read(&paths, &recipes)?;
        Ok(State {
            busy: lock(&self.inner.busy).clone(),
            expected: lock(&self.inner.freshness).expected.clone(),
            language,
            developer_mode: Settings::load(&paths).developer_mode.unwrap_or(false),
            paths,
            snap,
            recipes,
        })
    }

    /// Runs `work` for a VM (`AppModel.perform`): refused while another action runs on it; `locked` holds the
    /// instance lock – only for existing VMs, the lock file lives in their folder
    pub(crate) fn act<T>(
        &self,
        name: &str,
        locked: bool,
        work: impl FnOnce(&Backends, &Paths) -> crate::Result<T>,
    ) -> Result<T, RpcError> {
        if !lock(&self.inner.busy).insert(name.to_string()) {
            return Err(RpcError::from(crate::vm_err!(codes::IN_USE, "{name} is busy – an action on it runs")));
        }
        let _guard = BusyGuard { server: self, name: name.to_string() };
        self.poke();
        let paths = self.paths();
        let backends = Backends::new(paths.clone())?;
        let _lock = if locked { Some(InstanceLock::new(&paths.instance(name))?) } else { None };
        Ok(work(&backends, &paths)?)
    }

    // MARK: Doctor

    /// Runs the Doctor's checks, keeps and sends them, and reads the recipes again (`AppModel.checkAll`)
    pub(crate) fn check_doctor(&self) -> Doctor {
        self.inner.checking.fetch_add(1, Ordering::SeqCst);
        self.poke();
        let d = super::jobs::guarded(|| Ok(model::check_doctor(&self.paths())));
        self.inner.checking.fetch_sub(1, Ordering::SeqCst);
        let d = d.unwrap_or_else(|e| panic!("{}", e.message()));
        *lock(&self.inner.doctor) = Some(d.clone());
        self.poke();
        self.events().emit(Topic::Doctor, "doctor.changed", &model::doctor_report(&d));
        let _ = self.reload_recipes();
        d
    }

    /// The Doctor after an action – only when someone listens (it runs nix)
    pub(crate) fn check_doctor_later(&self) {
        if !self.events().is_subscribed(Topic::Doctor) {
            return;
        }
        let s = self.clone();
        std::thread::spawn(move || {
            let _ = super::jobs::guarded(|| {
                s.check_doctor();
                Ok(())
            });
        });
    }

    // MARK: VMs

    /// After a start: once the VM's agent says hello (NixOS, spec §11.9) or SSH answers, `vm.ready` (plan B91; the UI
    /// writes its own message). Creating comes before the start, so it first waits up to two minutes for the VM to run. VMs without
    /// network and without an agent can't be reached – nothing then.
    pub(crate) fn when_reachable(&self, name: &str) {
        let s = self.clone();
        let name = name.to_string();
        std::thread::spawn(move || {
            let _ = super::jobs::guarded(|| {
                let backends = Backends::new(s.paths())?;
                let mut running = None;
                for _ in 0..60 {
                    std::thread::sleep(Duration::from_secs(2));
                    if s.is_closing() {
                        return Ok(());
                    }
                    if let Ok(c) = backends.store.load(&name)
                        && backends.backend_for(&c).is_ok_and(|b| b.status(&c) == VmStatus::Running)
                    {
                        running = Some(c);
                        break;
                    }
                }
                let Some(c) = running else { return Ok(()) };
                if c.network == NetworkMode::None && c.kind != GuestKind::Nixos {
                    return Ok(());
                }
                let backend = backends.backend_for(&c)?;
                let Some(ready) = backend.wait_for_ready(&c, Duration::from_secs(300)) else { return Ok(()) };
                if backend.status(&c) != VmStatus::Running {
                    return Ok(());
                }
                let via = match ready {
                    Readiness::Ssh => Via::Ssh,
                    Readiness::Agent(_) => Via::Agent,
                };
                s.events().emit(Topic::Vms, "vm.ready", &VmReady { name: name.clone(), via });
                let (level, text, notification) = super::views::activity::vm_ready(&name, via);
                s.events().record(level, text, notification);
                s.poke();
                Ok(())
            });
        });
    }

    /// macOS guests get their clipboard agent over SSH once they answer (darwin-guest.nix) – checked at every look,
    /// so it also comes after a server restart or a first attempt that found no SSH yet (plan B64); remembered per
    /// runner process: a restarted VM gets it again
    fn ensure_clipboard_agents(&self, snap: &Snapshot) {
        for row in &snap.rows {
            let c = &row.config;
            if c.kind != GuestKind::Macos
                || row.status != VmStatus::Running
                || !c.clipboard_on()
                || c.network == NetworkMode::None
            {
                continue;
            }
            let name = c.name.clone();
            let known = {
                let mut cb = lock(&self.inner.clipboard);
                if !cb.pending.insert(name.clone()) {
                    continue;
                }
                cb.started.get(&name).copied()
            };
            let s = self.clone();
            std::thread::spawn(move || {
                let started = super::jobs::guarded(|| {
                    let backends = Backends::new(s.paths())?;
                    let vz = backends.vz()?;
                    let c = vz.store.load(&name)?;
                    let Some(pid) = vz.pid(&c) else { return Ok(None) };
                    if Some(pid) == known {
                        return Ok(Some(pid));
                    }
                    let Some(host) = vz.address(&c) else { return Ok(None) };
                    if Ports::ssh_banner(&host, 22, Duration::from_secs(3)).is_none() {
                        return Ok(None);
                    }
                    Ok(vz.start_clipboard_agent(&c, &host).then_some(pid))
                })
                .ok()
                .flatten();
                let mut cb = lock(&s.inner.clipboard);
                if let Some(pid) = started {
                    cb.started.insert(name.clone(), pid);
                }
                cb.pending.remove(&name);
            });
        }
    }

    // MARK: Watcher (api.md §6)

    /// Starts the watcher thread unless it runs; it ends when nothing is subscribed any more or the server ends
    pub(crate) fn start_watcher(&self, fresh: &HashSet<Topic>) {
        {
            let mut w = lock(&self.inner.watch);
            w.last.retain(|(t, _), _| !fresh.contains(t));
            w.poked = true;
            if w.running {
                drop(w);
                self.inner.wake.notify_all();
                return;
            }
            w.running = true;
        }
        let s = self.clone();
        std::thread::spawn(move || {
            loop {
                if s.is_closing() || !s.events().any() {
                    break;
                }
                if let Err(e) = super::jobs::guarded(|| {
                    s.tick();
                    Ok(())
                }) {
                    eprintln!("omaf serve: watcher: {e}");
                }
                if s.events().any_views() {
                    std::thread::sleep(VIEW_SPACING);
                }
                let w = lock(&s.inner.watch);
                let (mut w, _) = s
                    .inner
                    .wake
                    .wait_timeout_while(w, WATCH_INTERVAL, |w| !w.poked && !s.is_closing())
                    .unwrap_or_else(|e| e.into_inner());
                w.poked = false;
            }
            lock(&s.inner.watch).running = false;
        });
    }

    /// One look: whole lists of the subscribed topics when they changed, VMs that ended, clipboard agents, recipe
    /// edits on disk (`AppModel.refresh`)
    pub(crate) fn tick(&self) {
        let events = self.events();
        if lock(&self.inner.recipes).loaded {
            self.reload_recipes_if_changed();
        }
        let Ok(mut state) = self.state_in(None) else { return };
        let view = state.view();
        let statuses: Vec<(String, VmStatus)> =
            state.snap.rows.iter().map(|r| (r.config.name.clone(), r.status)).collect();
        let previous = lock(&self.inner.watch).statuses.replace(statuses.clone());
        if let Some(previous) = previous {
            for name in model::ended(&previous, &statuses) {
                let requested = lock(&self.inner.stop_requested).remove(&name);
                events.emit(Topic::Vms, "vm.ended", &VmEnded { name: name.clone(), requested });
                if !requested {
                    let (level, text, notification) = super::views::activity::vm_ended(&name);
                    events.record(level, text, notification);
                }
            }
        }
        lock(&self.inner.stop_requested).retain(|n| statuses.iter().any(|(m, s)| m == n && *s == VmStatus::Running));
        if events.is_subscribed(Topic::Vms) {
            self.send_if_changed(Topic::Vms, "vms.changed", &view.vms());
        }
        if events.is_subscribed(Topic::Images) {
            self.send_if_changed(Topic::Images, "images.changed", &view.images());
        }
        self.ensure_clipboard_agents(&state.snap);
        if state.recipes.loaded {
            // Recipe descriptions come in each client's language
            for language in events.languages(Topic::Recipes) {
                state.language = language.clone();
                let list = state.view().recipes(false);
                self.send_if_changed_in(Topic::Recipes, language, "recipes.changed", &list);
            }
        }
        self.send_views();
    }

    /// Sends a topic's whole list when it differs from the last one sent
    pub(crate) fn send_if_changed<T: Serialize>(&self, topic: Topic, method: &str, list: &T) -> bool {
        let Ok(value) = serde_json::to_value(list) else { return false };
        if !self.remember(topic, None, &value) {
            return false;
        }
        self.events().emit(topic, method, &value);
        true
    }

    /// The same for a list in one language, sent to the clients of that language
    fn send_if_changed_in<T: Serialize>(&self, topic: Topic, language: Option<String>, method: &str, list: &T) -> bool {
        let Ok(value) = serde_json::to_value(list) else { return false };
        if !self.remember(topic, language.clone(), &value) {
            return false;
        }
        self.events().emit_in(&language, topic, method, &value);
        true
    }

    /// Keeps `value` as the last list sent; false when it is that already
    fn remember(&self, topic: Topic, language: Option<String>, value: &Value) -> bool {
        let mut w = lock(&self.inner.watch);
        let key = (topic, language);
        if w.last.get(&key) == Some(value) {
            return false;
        }
        w.last.insert(key, value.clone());
        true
    }

    pub(crate) fn store(&self) -> InstanceStore {
        InstanceStore::new(self.paths(), "qemu-img")
    }
}

pub(crate) fn to_value<T: Serialize>(v: &T) -> Result<Value, RpcError> {
    serde_json::to_value(v).map_err(|e| RpcError::internal(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::events::tests::recorder;
    use crate::paths::tests::temp_paths;
    use serde_json::json;

    #[test]
    fn lists_are_sent_only_when_they_change() {
        let (_dir, paths) = temp_paths();
        let (sink, got) = recorder();
        let server = Server::for_tests(paths, sink);
        server.events().subscribe(0, [Topic::Images].into());
        assert!(server.send_if_changed(Topic::Images, "images.changed", &json!({"images": []})));
        assert!(!server.send_if_changed(Topic::Images, "images.changed", &json!({"images": []})));
        assert!(server.send_if_changed(Topic::Images, "images.changed", &json!({"images": [1]})));
        assert_eq!(got.lock().unwrap().len(), 2);
        // A new subscription sends the list again
        lock(&server.inner.watch).last.remove(&(Topic::Images, None));
        assert!(server.send_if_changed(Topic::Images, "images.changed", &json!({"images": [1]})));
    }

    #[test]
    fn a_busy_vm_refuses_a_second_action() {
        let (_dir, paths) = temp_paths();
        let (sink, _) = recorder();
        let server = Server::for_tests(paths, sink);
        lock(&server.inner.busy).insert("a".into());
        let e = server.act("a", false, |_, _| Ok(())).unwrap_err();
        assert_eq!(e.kind, Some(codes::IN_USE));
        // The guard frees the name again, also when the work fails early
        lock(&server.inner.busy).clear();
        let _ = std::panic::catch_unwind(AssertUnwindSafe(|| server.act("b", false, |_, _| Ok(()))));
        assert!(lock(&server.inner.busy).is_empty());
    }
}
