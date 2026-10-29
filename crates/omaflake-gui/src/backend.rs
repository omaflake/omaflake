// SPDX-License-Identifier: AGPL-3.0-or-later
//! The connection to omaflake's core (docs/api.md §1): the per-user service on its Unix socket, started on demand
//! with `omaf service start --json` like the macOS app does; when that fails, the core in this process
//! (`api::Server`) – then jobs end with the app. A connection thread forwards the core's events to the app and
//! connects again when the service ends (it restarts after an update or idle exit).

use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use iced::futures::channel::mpsc::Sender;
use iced::futures::{SinkExt, Stream};
use omaflake_core::Paths;
use omaflake_core::api::client::Client;
use omaflake_core::api::{RpcError, Server};
use serde_json::{Value, json};

/// A failed call: what to show
#[derive(Debug, Clone)]
pub struct CallError {
    pub message: String,
}

impl From<RpcError> for CallError {
    fn from(e: RpcError) -> CallError {
        // In the app's language when the message has a key (api.md §3)
        CallError { message: e.text().map(|t| crate::text::s(&t)).unwrap_or(e.message) }
    }
}

trait Core: Send + Sync {
    fn call(&self, method: &str, params: Value) -> Result<Value, RpcError>;
    fn is_open(&self) -> bool;
}

impl Core for Client {
    fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        Client::call(self, method, params)
    }
    fn is_open(&self) -> bool {
        Client::is_open(self)
    }
}

impl Core for Server {
    fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        Server::call(self, method, params)
    }
    fn is_open(&self) -> bool {
        true
    }
}

/// How the app reaches the core
#[derive(Debug, Clone, PartialEq)]
pub enum Via {
    /// The service on this socket
    Service(PathBuf),
    /// In this process (no service could be started)
    InProcess(String),
}

/// A cheap handle to the core: calls block, so the app runs them off its thread (`call`)
#[derive(Clone)]
pub struct Handle {
    core: Arc<dyn Core>,
    pub via: Via,
}

impl std::fmt::Debug for Handle {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Handle({:?})", self.via)
    }
}

impl Handle {
    pub fn call_blocking(&self, method: &str, params: Value) -> Result<Value, CallError> {
        self.core.call(method, params).map_err(CallError::from)
    }

    /// A call on a thread of its own, as a future for `Task::perform`
    pub fn call(
        &self,
        method: &str,
        params: Value,
    ) -> impl std::future::Future<Output = Result<Value, CallError>> + use<> {
        let (tx, rx) = iced::futures::channel::oneshot::channel();
        let (me, method) = (self.clone(), method.to_string());
        std::thread::spawn(move || {
            let _ = tx.send(me.call_blocking(&method, params));
        });
        async move {
            rx.await.unwrap_or_else(|_| Err(CallError { message: crate::text::tr("The call ended without an answer") }))
        }
    }
}

/// What the connection tells the app
#[derive(Debug, Clone)]
pub enum Event {
    Connected(Handle, Value),
    /// A notification of the core (`view.changed`, `jobs.changed`, `job.log`, …)
    Notification(String, Value),
    Disconnected(String),
}

/// The connection as a stream for `Subscription::run`
pub fn connection() -> impl Stream<Item = Event> {
    iced::stream::channel(256, async |out: Sender<Event>| {
        std::thread::Builder::new()
            .name("omaflake-core".into())
            .spawn(move || run(out))
            .expect("thread for the core connection");
        std::future::pending::<()>().await
    })
}

fn send(out: &mut Sender<Event>, e: Event) -> bool {
    iced::futures::executor::block_on(out.send(e)).is_ok()
}

fn run(mut out: Sender<Event>) {
    loop {
        let (tx, rx) = mpsc::channel::<(String, Value)>();
        let connected = connect(tx);
        match connected {
            Ok((handle, init)) => {
                if !send(&mut out, Event::Connected(handle.clone(), init)) {
                    return;
                }
                loop {
                    match rx.recv_timeout(Duration::from_millis(500)) {
                        Ok((m, p)) => {
                            if !send(&mut out, Event::Notification(m, p)) {
                                return;
                            }
                        }
                        Err(mpsc::RecvTimeoutError::Timeout) if handle.core.is_open() => {}
                        Err(_) => break,
                    }
                }
                if !send(&mut out, Event::Disconnected(crate::text::tr("The connection to the service closed"))) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(500));
            }
            Err(e) => {
                if !send(&mut out, Event::Disconnected(e)) {
                    return;
                }
                std::thread::sleep(Duration::from_secs(3));
            }
        }
    }
}

fn initialize(core: &dyn Core) -> Result<Value, String> {
    core.call(
        "initialize",
        json!({"client": "omaflake-linux", "clientVersion": omaflake_core::VERSION, "apiVersion": 1, "language": crate::text::lang().code()}),
    )
    .map_err(|e| e.message)
}

/// The service (started on demand), else the core in-process. `OMAFLAKE_IN_PROCESS=1` skips the service.
fn connect(events: mpsc::Sender<(String, Value)>) -> Result<(Handle, Value), String> {
    let force_local = std::env::var_os("OMAFLAKE_IN_PROCESS").is_some_and(|v| v == "1");
    let service_error = if force_local {
        "OMAFLAKE_IN_PROCESS=1".to_string()
    } else {
        match service(events.clone()) {
            Ok(c) => return Ok(c),
            Err(e) => e,
        }
    };
    eprintln!("omaflake: no core service ({service_error}) – running the core in this process");
    let server = Server::new(
        Paths::standard(),
        Arc::new(move |m: &str, p: Value| {
            let _ = events.send((m.to_string(), p));
        }),
    );
    let init = initialize(&server)?;
    Ok((Handle { core: Arc::new(server), via: Via::InProcess(service_error) }, init))
}

fn service(events: mpsc::Sender<(String, Value)>) -> Result<(Handle, Value), String> {
    let omaf = find_omaf().ok_or("omaf not found next to omaflake or on PATH")?;
    let out = Command::new(&omaf)
        .args(["service", "start", "--json"])
        .output()
        .map_err(|e| format!("{}: {e}", omaf.display()))?;
    if !out.status.success() {
        return Err(String::from_utf8_lossy(&out.stderr).trim().to_string());
    }
    let answer: Value = serde_json::from_slice(&out.stdout).map_err(|e| format!("omaf service start: {e}"))?;
    let socket = PathBuf::from(answer["socket"].as_str().ok_or("omaf service start: no socket")?);
    let client = Arc::new(Client::connect(&socket).map_err(|e| e.to_string())?);
    let init = initialize(client.as_ref())?;
    let pump = client.clone();
    std::thread::spawn(move || {
        while pump.is_open() {
            if let Some(e) = pump.next_event(Duration::from_millis(500))
                && events.send(e).is_err()
            {
                return;
            }
        }
    });
    Ok((Handle { core: client, via: Via::Service(socket) }, init))
}

/// `omaf` next to this program (the package installs both), else on PATH
pub fn find_omaf() -> Option<PathBuf> {
    let here = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("omaf")));
    if let Some(p) = here.filter(|p| p.is_file()) {
        return Some(p);
    }
    which("omaf")
}

/// A program on PATH
pub fn which(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(name)).find(|p| is_executable(p))
}

fn is_executable(p: &Path) -> bool {
    use std::os::unix::fs::PermissionsExt;
    p.metadata().is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
}
