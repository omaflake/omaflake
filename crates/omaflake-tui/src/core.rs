// SPDX-License-Identifier: AGPL-3.0-or-later
//! The TUI's way to the core (docs/api.md §1): the per-user service over its socket (`api::client::Client`), or a
//! server in this process when the service cannot be reached. Both answer the same methods; their events come
//! through one channel.

use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Arc;
use std::time::Duration;

use omaflake_core::api::client::Client;
use omaflake_core::api::{RpcError, Server};
use serde_json::{json, Value};

/// A refused or failed request: the core's English message and its stable kind (api.md §3)
#[derive(Debug, Clone, PartialEq)]
pub struct CoreError {
    pub message: String,
    pub kind: Option<String>,
}

impl From<RpcError> for CoreError {
    fn from(e: RpcError) -> CoreError {
        CoreError { message: e.message, kind: e.kind.map(str::to_string) }
    }
}

impl std::fmt::Display for CoreError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

/// One request at a time per call; implementations are shared between threads
pub trait Core: Send + Sync {
    fn call(&self, method: &str, params: Value) -> Result<Value, CoreError>;
    /// The connection still works (a service that ended closes it)
    fn is_open(&self) -> bool {
        true
    }
    /// Ends this session (a private server also stops its jobs)
    fn close(&self) {}
}

/// An event of the core: its method and params (`view.changed`, `job.log`, …)
pub type Event = (String, Value);

/// A connected core with its events
pub struct Connection {
    pub core: Arc<dyn Core>,
    pub events: Receiver<Event>,
    /// How it is connected, for the status line ("service", "in-process")
    pub via: String,
}

impl Core for Client {
    fn call(&self, method: &str, params: Value) -> Result<Value, CoreError> {
        Client::call(self, method, params).map_err(CoreError::from)
    }

    fn is_open(&self) -> bool {
        Client::is_open(self)
    }

    fn close(&self) {
        let _ = Client::call(self, "shutdown", Value::Null);
    }
}

/// A client of the service: its events are passed on to the channel by a thread of their own
pub fn from_client(client: Client, via: &str) -> Connection {
    // The session in the TUI's language (initialize again: recipe descriptions, the pages' language)
    let _ = client.call(
        "initialize",
        json!({"client": "omaf ui", "clientVersion": omaflake_core::VERSION, "apiVersion": 1,
               "language": crate::text::lang().code()}),
    );
    let client = Arc::new(client);
    let (tx, rx) = mpsc::channel();
    let reader = client.clone();
    std::thread::spawn(move || {
        while reader.is_open() {
            if let Some(event) = reader.next_event(Duration::from_millis(500)) {
                if tx.send(event).is_err() {
                    return;
                }
            }
        }
    });
    Connection { core: client, events: rx, via: via.into() }
}

/// The in-process server (the fallback): initialized here
struct Local(Server);

impl Core for Local {
    fn call(&self, method: &str, params: Value) -> Result<Value, CoreError> {
        self.0.call(method, params).map_err(CoreError::from)
    }

    fn close(&self) {
        self.0.close(Duration::from_secs(10));
    }
}

/// A server of its own on `paths`, initialized as `client`
pub fn in_process(paths: omaflake_core::Paths, client: &str) -> Result<Connection, CoreError> {
    let (tx, rx): (Sender<Event>, Receiver<Event>) = mpsc::channel();
    let tx = std::sync::Mutex::new(tx);
    let server = Server::new(
        paths,
        Arc::new(move |method: &str, params: Value| {
            let _ = tx.lock().unwrap_or_else(|e| e.into_inner()).send((method.to_string(), params));
        }),
    );
    server.call(
        "initialize",
        json!({"client": client, "clientVersion": omaflake_core::VERSION, "apiVersion": 1,
               "language": crate::text::lang().code()}),
    )?;
    Ok(Connection { core: Arc::new(Local(server)), events: rx, via: "in-process".into() })
}
