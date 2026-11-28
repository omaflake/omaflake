// SPDX-License-Identifier: AGPL-3.0-or-later
//! A blocking client of the core service's socket (docs/api.md §1, plan B120) – what `omaf build`, `omaf call
//! --socket`, `omaf service`, `omaf ui` and the Linux app use, and anyone's tool. One reader thread matches responses to requests by id and passes
//! notifications on as events.

use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Map, Value, json};

use crate::codes;
use crate::protocol::{RpcError, SERVER_ERROR};
use crate::text::{Arg, Text};
use crate::types::{InitializeResult, JobDto};

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
    pub fn connect(path: &Path) -> Result<Client, RpcError> {
        let failed = |e: std::io::Error| {
            error(
                None,
                "Connection to %@ failed: %@",
                vec![Arg::from(path.display().to_string()), Arg::from(e.to_string())],
            )
        };
        Client::over(UnixStream::connect(path).map_err(failed)?).map_err(failed)
    }

    /// A client on a connected stream
    fn over(stream: UnixStream) -> std::io::Result<Client> {
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
                    if line.trim().is_empty() {
                        continue;
                    }
                    let Ok(value) = serde_json::from_str::<Value>(&line) else {
                        // A line that is no JSON: whoever it was for must not wait for the full timeout. Its id is
                        // read where the line still shows it; without one nobody can tell whose answer it was, so
                        // every waiting call fails
                        fail(&pending, id_of_broken(&line), unreadable());
                        continue;
                    };
                    if let Some(method) = value.get("method").and_then(Value::as_str) {
                        let params = value.get("params").cloned().unwrap_or(Value::Null);
                        let _ = tx.send((method.to_string(), params));
                    } else if let Some(id) = value.get("id").and_then(Value::as_u64) {
                        // (an answer nobody waits for any more – its call timed out – is dropped)
                        if let Some(waiting) = lock(&pending).remove(&id) {
                            let _ = waiting.send(value);
                        }
                    } else if value.get("error").is_some() {
                        // An error without an id: the service could not read a request, so its caller gets no answer
                        // of its own – every waiting call fails with that error
                        fail(&pending, None, value);
                    } else {
                        fail(&pending, None, unreadable());
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
            json!({"client": client, "clientVersion": crate::VERSION, "apiVersion": crate::types::API_VERSION}),
        )?;
        serde_json::from_value(v).map_err(|e| RpcError::internal(format!("initialize: {e}")))
    }

    /// The connection is still open (the service did not end)
    pub fn is_open(&self) -> bool {
        self.open.load(Ordering::SeqCst)
    }

    /// One request; its result or its error
    pub fn call(&self, method: &str, params: Value) -> Result<Value, RpcError> {
        self.call_within(method, params, ANSWER)
    }

    fn call_within(&self, method: &str, params: Value, timeout: Duration) -> Result<Value, RpcError> {
        if !self.is_open() {
            return Err(gone());
        }
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let mut line = serde_json::to_string(&json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}))
            .map_err(|e| RpcError::internal(e.to_string()))?;
        line.push('\n');
        let (tx, rx) = mpsc::channel();
        lock(&self.pending).insert(id, tx);
        // A service that just closed the socket: the write fails (EPIPE) and the call says so. No signal ends the
        // process where SIGPIPE is ignored (Rust's default) or the socket has SO_NOSIGPIPE (std sets it on Apple
        // systems). OPEN: a Linux program that put SIGPIPE back to its default must ignore it before it uses a client
        // (`omaf ui` does) – this crate forbids `unsafe`, so it cannot pass MSG_NOSIGNAL itself.
        if lock(&self.writer).write_all(line.as_bytes()).is_err() {
            lock(&self.pending).remove(&id);
            return Err(gone());
        }
        let answer = match rx.recv_timeout(timeout) {
            Ok(answer) => answer,
            Err(e) => {
                // Nobody waits for this answer any more
                lock(&self.pending).remove(&id);
                return Err(match e {
                    RecvTimeoutError::Timeout => {
                        error(None, "The service did not answer %@ in time", vec![Arg::from(method)])
                    }
                    RecvTimeoutError::Disconnected => gone(),
                });
            }
        };
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
                    return Err(error(
                        Some(codes::NOT_FOUND),
                        "Job %@ left the list (dismissed elsewhere) – its log file has its end",
                        vec![Arg::from(id)],
                    ));
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
    error(None, "The connection to the service closed", Vec::new())
}

/// The answer a call gets whose line could not be read
fn unreadable() -> Value {
    json!({"error": error(None, "The service sent an answer that cannot be read", Vec::new()).to_value()})
}

/// Ends waiting calls with `answer`: the one with `id`, or all of them when the id is not known
fn fail(pending: &Pending, id: Option<u64>, answer: Value) {
    let mut pending = lock(pending);
    match id {
        Some(id) => {
            if let Some(waiting) = pending.remove(&id) {
                let _ = waiting.send(answer);
            }
        }
        None => {
            for (_, waiting) in pending.drain() {
                let _ = waiting.send(answer.clone());
            }
        }
    }
}

/// The id of a line that is not JSON, where it still shows one (`"id":7` – the service writes it without spaces,
/// others may not)
fn id_of_broken(line: &str) -> Option<u64> {
    let rest = line.split_once("\"id\"")?.1.trim_start().strip_prefix(':')?.trim_start();
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    // A number that goes on (1.5, 1e3) or a string is no id of this client
    let after = rest[digits.len()..].chars().next();
    if matches!(after, Some('.' | 'e' | 'E')) {
        return None;
    }
    digits.parse().ok()
}

/// A `-32000` error with its text (`data.text`, docs/api.md §3) – a UI translates it by its key, as the core's errors
fn error(kind: Option<&'static str>, key: &str, args: Vec<Arg>) -> RpcError {
    let text = Text::fmt(key, args);
    RpcError::server(kind, text.text.clone()).with_data("text", serde_json::to_value(&text).unwrap_or_default())
}

/// The lock, also after a thread panicked while holding it
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
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
    /// A client and the other end of its socket (the service's side)
    fn pair() -> (Client, UnixStream) {
        let (ours, theirs) = UnixStream::pair().unwrap();
        (Client::over(ours).unwrap(), theirs)
    }

    /// Reads one request from the service's side and gives its id
    fn request(service: &UnixStream) -> u64 {
        // Byte by byte: a buffered reader would take the next request along
        let mut line = Vec::new();
        let mut byte = [0u8];
        let mut service = service;
        while std::io::Read::read(&mut service, &mut byte).unwrap() == 1 && byte[0] != b'\n' {
            line.push(byte[0]);
        }
        serde_json::from_slice::<Value>(&line).unwrap()["id"].as_u64().unwrap()
    }

    #[test]
    fn a_call_gets_its_answer_and_events_pass() {
        let (client, mut service) = pair();
        let answering = std::thread::spawn(move || {
            let id = request(&service);
            writeln!(service, r#"{{"jsonrpc":"2.0","method":"job.log","params":{{"id":"j1","lines":["a"]}}}}"#)
                .unwrap();
            writeln!(service, r#"{{"jsonrpc":"2.0","id":{id},"result":{{"ok":true}}}}"#).unwrap();
            service
        });
        assert_eq!(client.call("x", Value::Null).unwrap(), json!({"ok": true}));
        let _service = answering.join().unwrap();
        assert_eq!(client.next_event(Duration::from_secs(5)).unwrap().0, "job.log");
        assert!(lock(&client.pending).is_empty());
    }

    #[test]
    fn a_timed_out_call_leaves_nothing_waiting() {
        let (client, service) = pair();
        let e = client.call_within("slow", Value::Null, Duration::from_millis(50)).unwrap_err();
        assert!(e.message.contains("did not answer slow in time"), "{e}");
        assert!(lock(&client.pending).is_empty(), "the timed-out call is still waiting");
        // Its late answer is dropped; the next call works
        let id = request(&service);
        let mut service = service;
        writeln!(service, r#"{{"jsonrpc":"2.0","id":{id},"result":1}}"#).unwrap();
        let answering = std::thread::spawn(move || {
            let id = request(&service);
            writeln!(service, r#"{{"jsonrpc":"2.0","id":{id},"result":2}}"#).unwrap();
            service
        });
        assert_eq!(client.call("next", Value::Null).unwrap(), json!(2));
        drop(answering.join().unwrap());
    }

    #[test]
    fn an_unreadable_answer_fails_its_call_at_once() {
        // The id can still be read: only that call fails
        let (client, service) = pair();
        let answering = std::thread::spawn(move || {
            let mut service = service;
            let id = request(&service);
            writeln!(service, r#"{{"jsonrpc":"2.0","id":{id},"result":{{"broken"#).unwrap();
            service
        });
        let started = std::time::Instant::now();
        let e = client.call("x", Value::Null).unwrap_err();
        assert!(e.message.contains("cannot be read"), "{e}");
        assert!(started.elapsed() < Duration::from_secs(30));
        assert!(lock(&client.pending).is_empty());
        let mut service = answering.join().unwrap();
        // No id to read: every waiting call fails
        let client = Arc::new(client);
        let calls: Vec<_> = (0..2)
            .map(|_| {
                let client = client.clone();
                std::thread::spawn(move || client.call("y", Value::Null))
            })
            .collect();
        request(&service);
        request(&service);
        writeln!(service, "not json").unwrap();
        for call in calls {
            let e = call.join().unwrap().unwrap_err();
            assert!(e.message.contains("cannot be read"), "{e}");
        }
        // An error without an id (the service could not read a request): its message reaches the callers
        let waiting = {
            let client = client.clone();
            std::thread::spawn(move || client.call("z", Value::Null))
        };
        request(&service);
        writeln!(service, r#"{{"jsonrpc":"2.0","id":null,"error":{{"code":-32700,"message":"Parse error"}}}}"#)
            .unwrap();
        let e = waiting.join().unwrap().unwrap_err();
        assert_eq!((e.code, e.message.as_str()), (crate::protocol::PARSE_ERROR, "Parse error"));
        assert!(client.is_open());
    }

    #[test]
    fn the_id_of_a_broken_line() {
        assert_eq!(id_of_broken(r#"{"jsonrpc":"2.0","id":7,"result":{"x"#), Some(7));
        assert_eq!(id_of_broken(r#"{"id" : 12 ,"result"#), Some(12));
        assert_eq!(id_of_broken(r#"{"id":"a7","result"#), None);
        assert_eq!(id_of_broken(r#"{"id":1.5"#), None);
        assert_eq!(id_of_broken("not json"), None);
    }

    #[test]
    fn a_call_on_a_closed_socket_fails_without_ending_the_process() {
        let (client, service) = pair();
        drop(service);
        // Whether the reader saw the end already or the write meets the closed socket: an error, no signal
        for _ in 0..3 {
            let e = client.call("x", Value::Null).unwrap_err();
            assert!(e.message.contains("closed"), "{e}");
        }
        assert!(lock(&client.pending).is_empty());
    }
}
