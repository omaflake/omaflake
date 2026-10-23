// SPDX-License-Identifier: AGPL-3.0-or-later
//! Jobs (docs/api.md §5): work that takes minutes runs in the server and reports through events. The port of the
//! app's build queue (AppModel.swift `builds`, `BuildJob`, `schedule`, `cancel`, plan B60): builds wait in a queue,
//! one per guest kind at a time – NixOS and macOS side by side –, run a check right before they start (`checking`),
//! keep their log (the full one in a file, routine lines hidden from the shown one) and their progress.
//!
//! The queue knows nothing about builds: a job is a `JobSpec` with an optional check and the work as closures, so
//! the logic here is tested with fake work. The work runs on its own thread and gets a `JobCtx` for log lines,
//! progress and its `Cancellation`.

use std::collections::VecDeque;
use std::fs::File;
use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant};

use chrono::{DateTime, Utc};
use serde_json::Value;

use super::events::{Events, Topic};
use super::types::{JobChanged, JobDto, JobKind, JobLines, JobLog, JobProgress, JobState};
use crate::build_progress::BuildProgressTracker;
use crate::cancellation::Cancellation;
use crate::error::{codes, Error, Result};
use crate::json;
use crate::models::GuestKind;
use crate::{Log, Progress};

/// Lines a job keeps for `jobs.log` (older ones stay in its log file, if it has one)
pub(crate) const MAX_LINES: usize = 5000;
/// How often new log lines and progress go out as events (they come in bursts during builds)
const PUMP_INTERVAL: Duration = Duration::from_millis(250);

pub(crate) type Work = Box<dyn FnOnce(&JobCtx) -> Result<Value> + Send>;
pub(crate) type Check = Box<dyn FnOnce(&JobCtx) -> Result<()> + Send>;
pub(crate) type OnEnd = Box<dyn FnOnce(&JobDto) + Send>;

/// What a job is and does
pub(crate) struct JobSpec {
    pub kind: JobKind,
    pub title: String,
    pub recipe: Option<String>,
    pub guest_kind: Option<GuestKind>,
    /// Queued: one job of this queue at a time (builds: per guest kind). None: starts at once.
    pub queue: Option<GuestKind>,
    /// Every line, also the hidden ones, goes there (truncated when the job is created)
    pub log_file: Option<PathBuf>,
    /// Progress from the log lines (`BuildProgressTracker`)
    pub tracker: Option<GuestKind>,
    /// Lines left out of the shown log (`BuildLog::is_noise`)
    pub noise: Option<fn(&str) -> bool>,
    /// A running job can be stopped (its work watches the `Cancellation` or set a cancel hook)
    pub cancellable: bool,
    /// Runs right before the work, in state `checking`; an error fails the job
    pub check: Option<Check>,
    pub work: Work,
    /// After the job ended (also when it failed, not when it left the queue): messages, reloads
    pub on_end: Option<OnEnd>,
}

impl JobSpec {
    pub fn new(kind: JobKind, title: impl Into<String>, work: Work) -> Self {
        JobSpec {
            kind,
            title: title.into(),
            recipe: None,
            guest_kind: None,
            queue: None,
            log_file: None,
            tracker: None,
            noise: None,
            cancellable: true,
            check: None,
            work,
            on_end: None,
        }
    }
}

struct Pending {
    check: Option<Check>,
    work: Work,
    on_end: Option<OnEnd>,
}

struct JobData {
    state: JobState,
    /// Ended and its `on_end` ran: `wait` returns
    settled: bool,
    started: Option<DateTime<Utc>>,
    finished: Option<DateTime<Utc>>,
    last_output: Option<DateTime<Utc>>,
    lines: VecDeque<String>,
    /// Index of `lines[0]` among all shown lines
    first: usize,
    total: usize,
    hidden: usize,
    log_file: Option<PathBuf>,
    file: Option<File>,
    tracker: Option<BuildProgressTracker>,
    progress: Option<JobProgress>,
    noise: Option<fn(&str) -> bool>,
    result: Option<Value>,
    error: Option<String>,
    cancel_hook: Option<Box<dyn Fn() + Send>>,
    /// Lines not sent as `job.log` yet
    unsent: Vec<String>,
    /// Changed since the last `jobs.changed`
    dirty: bool,
}

/// One job
pub(crate) struct Job {
    pub id: String,
    pub kind: JobKind,
    pub title: String,
    pub recipe: Option<String>,
    pub guest_kind: Option<GuestKind>,
    pub queue: Option<GuestKind>,
    cancellable: bool,
    cancel: Cancellation,
    data: Mutex<JobData>,
    ended: Condvar,
    pending: Mutex<Option<Pending>>,
    /// Held while events of this job go out, so they leave in order (not held while calling the sink's caller back)
    emitting: Mutex<()>,
    events: Arc<Events>,
}

impl Job {
    fn lock(&self) -> MutexGuard<'_, JobData> {
        self.data.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn state(&self) -> JobState {
        self.lock().state
    }

    pub fn is_active(&self) -> bool {
        self.state().is_active()
    }

    /// A log line: into the file, the progress tracker and – unless it is routine – the shown log
    pub fn append(&self, line: &str) {
        let mut d = self.lock();
        if let Some(t) = &d.tracker {
            if t.feed(line) {
                let p = t.progress();
                d.progress = Some(JobProgress {
                    step: p.step,
                    total: p.total,
                    title: p.title.clone(),
                    done: p.done as i64,
                    of: p.of as i64,
                    fraction: p.fraction(),
                });
                d.dirty = true;
            }
        }
        d.last_output = Some(json::now());
        if let Some(f) = d.file.as_mut() {
            let _ = f.write_all(format!("{line}\n").as_bytes());
        }
        if d.noise.is_some_and(|noise| noise(line)) {
            d.hidden += 1;
            d.dirty = true;
            return;
        }
        d.lines.push_back(line.to_string());
        d.total += 1;
        if d.lines.len() > MAX_LINES {
            d.lines.pop_front();
            d.first += 1;
        }
        d.unsent.push(line.to_string());
    }

    /// Progress in units (a download's bytes); keeps the title
    pub fn set_progress(&self, done: i64, of: i64, title: Option<&str>) {
        let mut d = self.lock();
        let title =
            title.map(str::to_string).or_else(|| d.progress.as_ref().map(|p| p.title.clone())).unwrap_or_default();
        let fraction = if of > 0 { (done as f64 / of as f64).clamp(0.0, 1.0) } else { 0.0 };
        d.progress = Some(JobProgress { step: 0, total: 0, title, done, of, fraction });
        d.dirty = true;
    }

    pub fn dto(&self) -> JobDto {
        let d = self.lock();
        self.dto_of(&d)
    }

    fn dto_of(&self, d: &JobData) -> JobDto {
        let cancellable = match d.state {
            JobState::Queued | JobState::Checking => true,
            JobState::Running => self.cancellable && !self.cancel.is_cancelled(),
            _ => false,
        };
        JobDto {
            id: self.id.clone(),
            kind: self.kind,
            title: self.title.clone(),
            recipe: self.recipe.clone(),
            guest_kind: self.guest_kind,
            state: d.state,
            progress: d.progress.clone(),
            started: d.started,
            finished: d.finished,
            last_output: d.last_output,
            log_lines: d.total,
            hidden_lines: d.hidden,
            log_file: d.log_file.as_ref().map(|p| p.to_string_lossy().into_owned()),
            cancellable,
            result: d.result.clone(),
            error: d.error.clone(),
        }
    }

    /// Lines from index `from` on (older ones than the kept `MAX_LINES` are gone: it starts at the oldest kept)
    pub fn log(&self, from: usize) -> JobLog {
        let d = self.lock();
        let from = from.max(d.first).min(d.total);
        JobLog { lines: d.lines.iter().skip(from - d.first).cloned().collect(), next: d.total }
    }

    /// Sends what is new: `job.log` with the unsent lines, then `jobs.changed` when anything else changed
    pub fn flush(&self) {
        let _order = self.emitting.lock().unwrap_or_else(|e| e.into_inner());
        let (lines, changed) = {
            let mut d = self.lock();
            let lines = std::mem::take(&mut d.unsent);
            let changed = (d.dirty || !lines.is_empty()).then(|| self.dto_of(&d));
            d.dirty = false;
            (lines, changed)
        };
        if !lines.is_empty() {
            self.events.emit(Topic::Jobs, "job.log", &JobLines { id: self.id.clone(), lines });
        }
        if let Some(job) = changed {
            self.events.emit(Topic::Jobs, "jobs.changed", &JobChanged { job });
            self.events.changed();
        }
    }

    fn set_state(&self, state: JobState) {
        let mut d = self.lock();
        if state == JobState::Checking || (state == JobState::Running && d.started.is_none()) {
            d.started = Some(json::now());
        }
        d.state = state;
        d.dirty = true;
    }

    /// From `checking` to `running`, unless it was cancelled meanwhile
    fn begin_running(&self) -> bool {
        let mut d = self.lock();
        if !d.state.is_active() {
            return false;
        }
        if d.started.is_none() {
            d.started = Some(json::now());
        }
        d.state = JobState::Running;
        d.dirty = true;
        true
    }

    /// The end: a last line, the note about hidden lines, the state; wakes those waiting
    /// `on_end` runs before those waiting wake up – `wait` (e.g. `omaf call images.build`) returns only once it is done
    fn finish(
        &self,
        state: JobState,
        line: Option<&str>,
        result: Option<Value>,
        error: Option<String>,
        on_end: Option<OnEnd>,
    ) {
        if let Some(line) = line {
            self.append(line);
        }
        let note = {
            let d = self.lock();
            match (&d.log_file, d.hidden) {
                (Some(file), n) if n > 0 => {
                    Some(format!("({n} routine lines of the disk image tool hidden – full log: {})", file.display()))
                }
                _ => None,
            }
        };
        if let Some(note) = note {
            self.append(&note);
        }
        {
            let mut d = self.lock();
            d.file = None;
            d.state = state;
            d.finished = Some(json::now());
            d.result = result;
            d.error = error;
            d.cancel_hook = None;
            d.dirty = true;
        }
        self.flush();
        // Its end in the activity list, worded once for every UI
        if let Some((level, text, notification)) = super::views::activity::job_ended(&self.dto()) {
            self.events.record(level, text, notification);
        }
        if let Some(on_end) = on_end {
            on_end(&self.dto());
        }
        self.lock().settled = true;
        self.ended.notify_all();
    }

    /// Blocks until the job ended; returns it
    pub fn wait(&self) -> JobDto {
        let mut d = self.lock();
        while !d.settled {
            d = self.ended.wait(d).unwrap_or_else(|e| e.into_inner());
        }
        self.dto_of(&d)
    }

    /// Like `wait`, up to `timeout`; true = it ended
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        let mut d = self.lock();
        while !d.settled {
            let now = Instant::now();
            if now >= deadline {
                return false;
            }
            d = self.ended.wait_timeout(d, deadline - now).unwrap_or_else(|e| e.into_inner()).0;
        }
        true
    }
}

/// What the work of a job gets
pub(crate) struct JobCtx {
    pub job: Arc<Job>,
}

impl JobCtx {
    pub fn log(&self, line: &str) {
        self.job.append(line)
    }

    /// A log callback for the core (`&Log`)
    pub fn log_sink(&self) -> Arc<Log<'static>> {
        let job = self.job.clone();
        Arc::new(move |line: &str| job.append(line))
    }

    /// A progress callback for the core (`&Progress`): done/of in units
    pub fn progress_sink(&self, title: &str) -> Arc<Progress<'static>> {
        let job = self.job.clone();
        let title = title.to_string();
        Arc::new(move |done: i64, of: i64| job.set_progress(done, of, Some(&title)))
    }

    pub fn cancellation(&self) -> &Cancellation {
        &self.job.cancel
    }

    /// Runs `hook` when the job is cancelled while running (e.g. pausing a download)
    pub fn on_cancel(&self, hook: impl Fn() + Send + 'static) {
        let mut d = self.job.lock();
        if self.job.cancel.is_cancelled() {
            drop(d);
            hook();
            return;
        }
        d.cancel_hook = Some(Box::new(hook));
    }
}

/// The server's jobs: the list, the queue and the event pump
pub(crate) struct Jobs {
    list: Mutex<Vec<Arc<Job>>>,
    next: AtomicU64,
    events: Arc<Events>,
    pump: AtomicBool,
    closed: AtomicBool,
}

impl Jobs {
    pub fn new(events: Arc<Events>) -> Arc<Jobs> {
        Arc::new(Jobs {
            list: Mutex::new(Vec::new()),
            next: AtomicU64::new(1),
            events,
            pump: AtomicBool::new(false),
            closed: AtomicBool::new(false),
        })
    }

    fn lock(&self) -> MutexGuard<'_, Vec<Arc<Job>>> {
        self.list.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Adds a job: a queued one waits for its queue, any other starts at once
    pub fn submit(self: &Arc<Self>, spec: JobSpec) -> Arc<Job> {
        let id = format!("j{}", self.next.fetch_add(1, Ordering::Relaxed));
        let file = spec.log_file.as_ref().and_then(|f| {
            if let Some(dir) = f.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            File::create(f).ok()
        });
        let job = Arc::new(Job {
            id,
            kind: spec.kind,
            title: spec.title,
            recipe: spec.recipe,
            guest_kind: spec.guest_kind,
            queue: spec.queue,
            cancellable: spec.cancellable,
            cancel: Cancellation::new(),
            data: Mutex::new(JobData {
                state: JobState::Queued,
                settled: false,
                started: None,
                finished: None,
                last_output: None,
                lines: VecDeque::new(),
                first: 0,
                total: 0,
                hidden: 0,
                log_file: spec.log_file,
                file,
                tracker: spec.tracker.map(BuildProgressTracker::new),
                progress: None,
                noise: spec.noise,
                result: None,
                error: None,
                cancel_hook: None,
                unsent: Vec::new(),
                dirty: true,
            }),
            ended: Condvar::new(),
            pending: Mutex::new(Some(Pending { check: spec.check, work: spec.work, on_end: spec.on_end })),
            emitting: Mutex::new(()),
            events: self.events.clone(),
        });
        let waits = {
            let mut list = self.lock();
            let waits = job.queue.is_some_and(|q| list.iter().any(|j| j.queue == Some(q) && j.is_active()));
            list.push(job.clone());
            waits
        };
        if waits {
            let kind = if job.queue == Some(GuestKind::Macos) { "macOS" } else { "NixOS" };
            job.append(&format!("Waiting – another {kind} build runs first"));
        }
        self.start_pump();
        if job.queue.is_some() {
            self.schedule();
        } else {
            self.launch(job.clone(), false);
        }
        job.flush();
        job
    }

    /// Starts the next waiting job of each queue that has none running
    fn schedule(self: &Arc<Self>) {
        let mut start = Vec::new();
        {
            let list = self.lock();
            for kind in GuestKind::ALL {
                let busy = list
                    .iter()
                    .any(|j| j.queue == Some(kind) && matches!(j.state(), JobState::Checking | JobState::Running));
                if busy {
                    continue;
                }
                if let Some(next) = list.iter().find(|j| j.queue == Some(kind) && j.state() == JobState::Queued) {
                    // Marked under the list lock, so two schedules never start it twice
                    let checks = next.pending.lock().unwrap().as_ref().is_some_and(|p| p.check.is_some());
                    next.set_state(if checks { JobState::Checking } else { JobState::Running });
                    start.push((next.clone(), checks));
                }
            }
        }
        for (job, checks) in start {
            self.launch(job, checks);
        }
    }

    fn launch(self: &Arc<Self>, job: Arc<Job>, checks: bool) {
        if job.queue.is_none() {
            job.set_state(JobState::Running);
        }
        let jobs = self.clone();
        std::thread::spawn(move || jobs.run(job, checks));
    }

    fn run(self: &Arc<Self>, job: Arc<Job>, checks: bool) {
        let Some(pending) = job.pending.lock().unwrap().take() else { return };
        let ctx = JobCtx { job: job.clone() };
        if checks {
            job.append("Checking …");
            job.flush();
            if let Some(check) = pending.check {
                let checked = guarded(|| check(&ctx));
                if job.state() == JobState::Cancelled {
                    self.schedule();
                    return;
                }
                if let Err(e) = checked {
                    job.finish(
                        JobState::Failed,
                        Some(e.message()),
                        None,
                        Some(e.message().to_string()),
                        pending.on_end,
                    );
                    self.schedule();
                    return;
                }
            }
        }
        if !job.begin_running() {
            self.schedule();
            return;
        }
        job.flush();
        let outcome = guarded(|| (pending.work)(&ctx));
        match outcome {
            Ok(value) => job.finish(JobState::Finished, None, Some(value), None, pending.on_end),
            Err(e) => {
                let cancelled = job.cancel.is_cancelled() || e.is_cancelled();
                let line = format!("✗ {e}");
                let error = (!cancelled).then(|| e.message().to_string());
                job.finish(
                    if cancelled { JobState::Cancelled } else { JobState::Failed },
                    Some(&line),
                    None,
                    error,
                    pending.on_end,
                );
            }
        }
        if job.queue.is_some() {
            self.schedule();
        }
    }

    /// Sends new lines and progress of all jobs every `PUMP_INTERVAL` (one thread per server, from the first job on)
    fn start_pump(self: &Arc<Self>) {
        if self.pump.swap(true, Ordering::SeqCst) {
            return;
        }
        let jobs = Arc::downgrade(self);
        std::thread::spawn(move || loop {
            std::thread::sleep(PUMP_INTERVAL);
            let Some(jobs) = jobs.upgrade() else { return };
            if jobs.closed.load(Ordering::SeqCst) {
                return;
            }
            let list: Vec<Arc<Job>> = jobs.lock().clone();
            for job in list {
                job.flush();
            }
        });
    }

    pub fn get(&self, id: &str) -> Option<Arc<Job>> {
        self.lock().iter().find(|j| j.id == id).cloned()
    }

    pub fn find(&self, id: &str) -> Result<Arc<Job>> {
        self.get(id).ok_or_else(|| Error::with_code(codes::NOT_FOUND, format!("Job \"{id}\" does not exist")))
    }

    pub fn list(&self) -> Vec<JobDto> {
        self.lock().iter().map(|j| j.dto()).collect()
    }

    /// The waiting or running job of a kind (for a recipe)
    pub fn active(&self, kind: JobKind, recipe: Option<&str>) -> Option<Arc<Job>> {
        self.lock()
            .iter()
            .rev()
            .find(|j| j.kind == kind && j.is_active() && (recipe.is_none() || j.recipe.as_deref() == recipe))
            .cloned()
    }

    /// Any job waits or runs
    pub fn any_running(&self) -> bool {
        self.lock().iter().any(|j| j.is_active())
    }

    /// Any waiting or running job of these kinds
    pub fn any_active(&self, kinds: &[JobKind]) -> Option<Arc<Job>> {
        self.lock().iter().find(|j| kinds.contains(&j.kind) && j.is_active()).cloned()
    }

    /// Ended jobs of a kind for a recipe leave the list (a new build of a recipe replaces its last log)
    pub fn drop_ended(&self, kind: JobKind, recipe: &str) {
        self.lock().retain(|j| j.is_active() || j.kind != kind || j.recipe.as_deref() != Some(recipe));
    }

    /// A waiting job leaves the queue; a running one is stopped (its work sees the `Cancellation`, its hook runs)
    pub fn cancel(self: &Arc<Self>, id: &str) -> Result<JobDto> {
        let job = self.find(id)?;
        match job.state() {
            JobState::Queued | JobState::Checking => {
                // Dropping the closures also drops a password a build was given
                drop(job.pending.lock().unwrap().take());
                job.finish(JobState::Cancelled, Some("Removed from the queue"), None, None, None);
                self.schedule();
            }
            JobState::Running if job.cancellable && !job.cancel.is_cancelled() => {
                job.append("Cancelling …");
                job.cancel.cancel();
                let hook = job.lock().cancel_hook.take();
                if let Some(hook) = hook {
                    hook();
                }
                {
                    job.lock().dirty = true;
                }
                job.flush();
            }
            _ => {}
        }
        Ok(job.dto())
    }

    /// An ended job leaves the list
    pub fn dismiss(&self, id: &str) -> Result<()> {
        let job = self.find(id)?;
        if job.is_active() {
            return Err(Error::with_code(codes::IN_USE, format!("Job \"{id}\" is still running – cancel it first")));
        }
        self.lock().retain(|j| j.id != id);
        Ok(())
    }

    /// The server ends: every job is cancelled; waits up to `timeout` for them to stop
    pub fn close(self: &Arc<Self>, timeout: Duration) {
        let ids: Vec<String> = self.lock().iter().filter(|j| j.is_active()).map(|j| j.id.clone()).collect();
        for id in ids {
            let _ = self.cancel(&id);
        }
        let deadline = Instant::now() + timeout;
        let list: Vec<Arc<Job>> = self.lock().clone();
        for job in list {
            let left = deadline.saturating_duration_since(Instant::now());
            job.wait_timeout(left);
        }
        self.closed.store(true, Ordering::SeqCst);
    }
}

/// Runs `f`, turning a panic (a bug, or a part of the core not ported yet) into an `internal` error
pub(crate) fn guarded<T>(f: impl FnOnce() -> Result<T>) -> Result<T> {
    match catch_unwind(AssertUnwindSafe(f)) {
        Ok(r) => r,
        Err(panic) => Err(Error::with_code(codes::INTERNAL, format!("Internal error: {}", panic_text(&panic)))),
    }
}

pub(crate) fn panic_text(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| panic.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_else(|| "panic".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::events::tests::recorder;
    use serde_json::json;
    use std::sync::mpsc;

    fn jobs() -> (Arc<Jobs>, crate::api::events::tests::Recorded) {
        let (sink, got) = recorder();
        let events = Arc::new(Events::new());
        events.attach(0, sink);
        events.subscribe(0, [Topic::Jobs].into());
        (Jobs::new(events), got)
    }

    /// A build that runs until `release` says so
    fn gated_build(kind: GuestKind, name: &str) -> (JobSpec, mpsc::Sender<()>) {
        let (tx, rx) = mpsc::channel::<()>();
        let rx = Mutex::new(rx);
        let mut spec = JobSpec::new(
            JobKind::Build,
            format!("Build {name}"),
            Box::new(move |ctx: &JobCtx| {
                ctx.log("building");
                loop {
                    ctx.cancellation().check()?;
                    if rx.lock().unwrap().recv_timeout(Duration::from_millis(10)).is_ok() {
                        return Ok(json!({"image": "img"}));
                    }
                }
            }),
        );
        spec.recipe = Some(name.into());
        spec.guest_kind = Some(kind);
        spec.queue = Some(kind);
        (spec, tx)
    }

    fn wait_for(job: &Job, state: JobState) {
        let deadline = Instant::now() + Duration::from_secs(5);
        while job.state() != state {
            assert!(Instant::now() < deadline, "{} stays {:?}, wanted {state:?}", job.id, job.state());
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[test]
    fn one_build_per_guest_kind_side_by_side() {
        let (jobs, _) = jobs();
        let (a, release_a) = gated_build(GuestKind::Nixos, "a");
        let (b, release_b) = gated_build(GuestKind::Nixos, "b");
        let (m, release_m) = gated_build(GuestKind::Macos, "m");
        let a = jobs.submit(a);
        let b = jobs.submit(b);
        let m = jobs.submit(m);
        wait_for(&a, JobState::Running);
        wait_for(&m, JobState::Running);
        assert_eq!(b.state(), JobState::Queued);
        assert_eq!(b.log(0).lines, vec!["Waiting – another NixOS build runs first".to_string()]);
        release_a.send(()).unwrap();
        assert_eq!(a.wait().state, JobState::Finished);
        assert_eq!(a.dto().result, Some(json!({"image": "img"})));
        wait_for(&b, JobState::Running);
        release_b.send(()).unwrap();
        release_m.send(()).unwrap();
        assert_eq!(b.wait().state, JobState::Finished);
        assert_eq!(m.wait().state, JobState::Finished);
        let started = a.dto().started.unwrap();
        assert!(a.dto().finished.unwrap() >= started);
    }

    #[test]
    fn cancelling_a_waiting_build_takes_it_out_of_the_queue() {
        let (jobs, _) = jobs();
        let (a, release_a) = gated_build(GuestKind::Nixos, "a");
        let (b, _release_b) = gated_build(GuestKind::Nixos, "b");
        let (c, release_c) = gated_build(GuestKind::Nixos, "c");
        let a = jobs.submit(a);
        let b = jobs.submit(b);
        let c = jobs.submit(c);
        wait_for(&a, JobState::Running);
        let dto = jobs.cancel(&b.id).unwrap();
        assert_eq!(dto.state, JobState::Cancelled);
        assert_eq!(b.log(0).lines.last().unwrap(), "Removed from the queue");
        release_a.send(()).unwrap();
        wait_for(&c, JobState::Running);
        release_c.send(()).unwrap();
        assert_eq!(c.wait().state, JobState::Finished);
    }

    #[test]
    fn cancelling_a_running_job_stops_its_work_and_runs_the_hook() {
        let (jobs, _) = jobs();
        let hooked = Arc::new(AtomicBool::new(false));
        let h = hooked.clone();
        let spec = JobSpec::new(
            JobKind::IpswDownload,
            "Download",
            Box::new(move |ctx: &JobCtx| {
                let h = h.clone();
                ctx.on_cancel(move || h.store(true, Ordering::SeqCst));
                loop {
                    ctx.cancellation().check()?;
                    std::thread::sleep(Duration::from_millis(5));
                }
            }),
        );
        let job = jobs.submit(spec);
        wait_for(&job, JobState::Running);
        assert!(job.dto().cancellable);
        let dto = jobs.cancel(&job.id).unwrap();
        assert!(!dto.cancellable);
        let end = job.wait();
        assert_eq!(end.state, JobState::Cancelled);
        assert_eq!(end.error, None);
        assert!(hooked.load(Ordering::SeqCst));
        assert!(job.log(0).lines.contains(&"Cancelling …".to_string()));
        assert!(jobs.dismiss(&job.id).is_ok());
        assert!(jobs.get(&job.id).is_none());
    }

    #[test]
    fn a_failing_check_fails_the_build_and_the_next_one_starts() {
        let (jobs, _) = jobs();
        let ended = Arc::new(Mutex::new(None));
        let e = ended.clone();
        let (mut a, _) = gated_build(GuestKind::Nixos, "a");
        a.check = Some(Box::new(|_| Err(Error::new("Doctor: Nix – fix it there first"))));
        a.on_end = Some(Box::new(move |dto| *e.lock().unwrap() = Some(dto.state)));
        let (b, release_b) = gated_build(GuestKind::Nixos, "b");
        let a = jobs.submit(a);
        let b = jobs.submit(b);
        let end = a.wait();
        assert_eq!(end.state, JobState::Failed);
        assert_eq!(end.error.as_deref(), Some("Doctor: Nix – fix it there first"));
        assert_eq!(a.log(0).lines, vec!["Checking …".to_string(), "Doctor: Nix – fix it there first".to_string()]);
        assert_eq!(*ended.lock().unwrap(), Some(JobState::Failed));
        wait_for(&b, JobState::Running);
        release_b.send(()).unwrap();
        b.wait();
    }

    #[test]
    fn errors_and_panics_fail_a_job() {
        let (jobs, _) = jobs();
        let failing = jobs.submit(JobSpec::new(JobKind::Export, "x", Box::new(|_| Err(Error::new("disk full")))));
        let end = failing.wait();
        assert_eq!((end.state, end.error.as_deref()), (JobState::Failed, Some("disk full")));
        assert_eq!(failing.log(0).lines, vec!["✗ disk full".to_string()]);
        let panicking = jobs.submit(JobSpec::new(JobKind::Import, "y", Box::new(|_| todo!("port from X.swift"))));
        let end = panicking.wait();
        assert_eq!(end.state, JobState::Failed);
        assert!(end.error.unwrap().contains("port from X.swift"));
        assert!(jobs.dismiss(&panicking.id).is_ok());
    }

    #[test]
    fn routine_lines_are_hidden_but_kept_in_the_log_file() {
        let (jobs, _) = jobs();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("logs/image-build-x.log");
        let mut spec = JobSpec::new(
            JobKind::Build,
            "Build x",
            Box::new(|ctx: &JobCtx| {
                for line in ["one", "noise 1", "two", "noise 2"] {
                    ctx.log(line);
                }
                Ok(Value::Null)
            }),
        );
        spec.log_file = Some(file.clone());
        spec.noise = Some(|l: &str| l.starts_with("noise"));
        let job = jobs.submit(spec);
        let end = job.wait();
        assert_eq!(end.hidden_lines, 2);
        let lines = job.log(0).lines;
        assert_eq!(&lines[..2], &["one".to_string(), "two".to_string()]);
        assert!(lines[2].starts_with("(2 routine lines"));
        assert_eq!(end.log_lines, 3);
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(text.starts_with("one\nnoise 1\ntwo\nnoise 2\n"));
        assert_eq!(job.log(2).lines.len(), 1);
        assert_eq!(job.log(99), JobLog { lines: vec![], next: 3 });
    }

    #[test]
    fn the_log_keeps_the_newest_lines() {
        let (jobs, _) = jobs();
        let job = jobs.submit(JobSpec::new(
            JobKind::Export,
            "big",
            Box::new(|ctx: &JobCtx| {
                for i in 0..MAX_LINES + 10 {
                    ctx.log(&i.to_string());
                }
                Ok(Value::Null)
            }),
        ));
        job.wait();
        let log = job.log(0);
        assert_eq!(log.lines.len(), MAX_LINES);
        assert_eq!(log.lines[0], "10");
        assert_eq!(log.next, MAX_LINES + 10);
    }

    #[test]
    fn events_tell_lines_before_the_end() {
        let (jobs, got) = jobs();
        let job = jobs.submit(JobSpec::new(
            JobKind::Export,
            "e",
            Box::new(|ctx: &JobCtx| {
                ctx.log("a");
                ctx.job.set_progress(5, 10, Some("Copying"));
                Ok(json!({"path": "/x"}))
            }),
        ));
        job.wait();
        let got = got.lock().unwrap();
        let methods: Vec<&str> = got.iter().map(|(m, _)| m.as_str()).collect();
        assert_eq!(methods.first(), Some(&"jobs.changed"));
        let log_at = got.iter().position(|(m, v)| m == "job.log" && v["lines"] == json!(["a"])).unwrap();
        let (_, last) = got.iter().rev().find(|(m, _)| m == "jobs.changed").unwrap();
        assert_eq!(last["job"]["state"], json!("finished"));
        assert_eq!(last["job"]["progress"]["fraction"], json!(0.5));
        let end_at = got.iter().rposition(|(m, _)| m == "jobs.changed").unwrap();
        assert!(log_at < end_at);
    }

    #[test]
    fn active_jobs_cannot_be_dismissed_and_close_cancels_them() {
        let (jobs, _) = jobs();
        let (a, _keep) = gated_build(GuestKind::Macos, "a");
        let a = jobs.submit(a);
        wait_for(&a, JobState::Running);
        assert_eq!(jobs.dismiss(&a.id).unwrap_err().code(), Some(codes::IN_USE));
        assert!(jobs.active(JobKind::Build, Some("a")).is_some());
        assert!(jobs.active(JobKind::Build, Some("b")).is_none());
        jobs.close(Duration::from_secs(5));
        assert_eq!(a.state(), JobState::Cancelled);
        jobs.drop_ended(JobKind::Build, "a");
        assert!(jobs.list().is_empty());
        assert_eq!(jobs.cancel("j999").unwrap_err().code(), Some(codes::NOT_FOUND));
    }
}
