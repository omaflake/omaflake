// SPDX-License-Identifier: AGPL-3.0-or-later
//! A blocking client of the core service's socket (docs/api.md §1, plan B120) – what `omaf build`, `omaf call
//! --socket` and `omaf service` use, and the tests. One reader thread matches responses to requests by id and passes
//! notifications on as events.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{json, Map, Value};

use super::protocol::{RpcError, SERVER_ERROR};
use super::server::lock;
use super::types::{InitializeResult, JobDto};
use crate::error::codes;

/// How long a request waits for its answer (a job's method answers at once; nothing in the API takes longer)
const ANSWER: Duration = Duration::from_secs(600);

type Pending = Arc<Mutex<HashMap<u64, Sender<Value>>>>;

/// A connection to the service
pub struct Client {
    stream: UnixStream,
    writer: Mutex<UnixStream>,
    pending: Pending,
    next: AtomicU64,
    events: Mutex<Receiver<(String, Value)>>,
    open: Arc<AtomicBool>,
}

impl Client {
    /// Connects to the socket at `path` (no session yet: `initialize` is the first call)
    pub fn connect(path: &Path) -> crate::Result<Client> {
        let stream = crate::sockets::connect_unix(path, Duration::ZERO)?;
        let reader = stream.try_clone()?;
        let writer = stream.try_clone()?;
        let pending: Pending = Arc::default();
        let open = Arc::new(AtomicBool::new(true));
        let (tx, rx) = mpsc::channel();
        {
            let (pending, open) = (pending.clone(), open.clone());
            std::thread::spawn(move || {
                for line in BufReader::new(reader).lines() {
                    let Ok(line) = line else { break };
                    let Ok(value) = serde_json::from_str::<Value>(&line) else { continue };
                    if let Some(method) = value.get("method").and_then(Value::as_str) {
                        let params = value.get("params").cloned().unwrap_or(Value::Null);
                        let _ = tx.send((method.to_string(), params));
                    } else if let Some(id) = value.get("id").and_then(Value::as_u64) {
                        if let Some(waiting) = lock(&pending).remove(&id) {
                            let _ = waiting.send(value);
                        }
                    }
                }
                open.store(false, Ordering::SeqCst);
                // Whoever still waits learns that no answer comes
                lock(&pending).clear();
            });
        }
        Ok(Client {
            stream,
            writer: Mutex::new(writer),
            pending,
            next: AtomicU64::new(1),
            events: Mutex::new(rx),
            open,
        })
    }

    /// `initialize` as `client`, with this omaf's version
    pub fn initialize(&self, client: &str) -> Result<InitializeResult, RpcError> {
        let v = self.call(
            "initialize",
            json!({"client": client, "clientVersion": crate::VERSION, "apiVersion": super::types::API_VERSION}),
        )?;
        serde_json::from_value(v).map_err(|e| RpcError::internal(format!("initialize: {e}")))
    }

    /// The connection is still open (the service did not end)
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    /// One request; its result or its error
    pub fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        if !self.is_open() {
            return Err(gone());
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (tx, rx) = mpsc::channel();
        lock(&self.pending).insert(id, tx);
        let mut line = serde_json::to_string(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .map_err(|e| RpcError::internal(e.to_string()))?;
        line.push('\n');
        if lock(&self.writer).write_all(line.as_bytes()).is_err() {
            lock(&self.pending).remove(&id);
            return Err(gone());
        }
        let answer = rx.recv_timeout(ANSWER).map_err(|e| match e {
            RecvTimeoutError::Timeout => RpcError::from(crate::vm_err!("The service did not answer {method} in time")),
            RecvTimeoutError::Disconnected => gone(),
        })?;
        if let Some(error) = answer.get("error") {
            return Err(rpc_error(error));
        }
        Ok(answer.get("result").cloned().unwrap_or(Value::Null))
    }

    /// The next event, waiting at most `timeout`; None when none came (or the connection closed)
    pub fn next_event(&self, timeout: Duration) -> Option<(String, Value)> {
        lock(&self.events).recv_timeout(timeout).ok()
    }

    /// Follows a job until it ended: `on_lines` gets its new log lines (from `from` on), `cancelled` is asked
    /// between looks – true once sends `jobs.cancel`. Needs `events.subscribe` with `jobs` for prompt news; without it
    /// the job is looked at every half second.
    pub fn follow_job(
        &self,
        id: &str,
        mut from: usize,
        on_lines: &mut dyn FnMut(&[String]),
        cancelled: &dyn Fn() -> bool,
    ) -> Result<JobDto, RpcError> {
        let mut cancel_sent = false;
        let mut ended: Option<JobDto> = None;
        loop {
            if !cancel_sent && cancelled() {
                cancel_sent = true;
                self.call("jobs.cancel", json!({"id": id}))?;
            }
            // Its log so far (jobs.log counts lines from the job's start; a long log keeps only the newest)
            let log = self.call("jobs.log", json!({"id": id, "from": from}))?;
            let lines: Vec<String> = serde_json::from_value(log["lines"].clone()).unwrap_or_default();
            if !lines.is_empty() {
                on_lines(&lines);
            }
            from = log["next"].as_u64().map(|n| n as usize).unwrap_or(from + lines.len());
            if let Some(job) = ended {
                return Ok(job);
            }
            let jobs: Vec<JobDto> = serde_json::from_value(self.call("jobs.list", Value::Null)?)
                .map_err(|e| RpcError::internal(format!("jobs.list: {e}")))?;
            match jobs.into_iter().find(|j| j.id == id) {
                // One more look at the log, then done
                Some(job) if !job.state.is_active() => ended = Some(job),
                Some(_) => {
                    // Woken by any event (job.log, jobs.changed), else every half second
                    let _ = self.next_event(Duration::from_millis(500));
                }
                None => {
                    return Err(RpcError::from(crate::vm_err!(
                        codes::NOT_FOUND,
                        "Job {id} left the list (dismissed elsewhere) – its log file has its end"
                    )))
                }
            }
            if !self.is_open() {
                return Err(gone());
            }
        }
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        // Ends the reader thread too
        let _ = self.stream.shutdown(std::net::Shutdown::Both);
    }
}

fn gone() -> RpcError {
    RpcError::from(crate::vm_err!("The connection to the service closed"))
}

/// An `error` object back into an `RpcError` (its kind, if this omaflake knows it, and the other data)
fn rpc_error(error: &Value) -> RpcError {
    let code = error["code"].as_i64().unwrap_or(SERVER_ERROR);
    let message = error["message"].as_str().unwrap_or("Error").to_string();
    let mut data: Map<String, Value> = error.get("data").and_then(Value::as_object).cloned().unwrap_or_default();
    let kind = data.remove("kind").and_then(|k| k.as_str().and_then(codes::known));
    RpcError { code, message, kind, data }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn errors_come_back_with_their_kind() {
        let e =
            rpc_error(&json!({"code": -32000, "message": "x", "data": {"kind": "vm-running", "networkVolume": true}}));
        assert_eq!((e.code, e.kind), (-32000, Some(codes::VM_RUNNING)));
        assert_eq!(e.data["networkVolume"], json!(true));
        let e = rpc_error(&json!({"code": -32601, "message": "Unknown method"}));
        assert_eq!((e.code, e.kind, e.message.as_str()), (-32601, None, "Unknown method"));
        assert_eq!(rpc_error(&json!({"code": -32000, "message": "x", "data": {"kind": "new-kind"}})).kind, None);
    }
}
