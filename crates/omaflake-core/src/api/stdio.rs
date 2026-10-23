// SPDX-License-Identifier: AGPL-3.0-or-later
//! The stdio transport of `omaf serve` (docs/api.md §1): one JSON object per line in and out. A reader thread reads
//! stdin, each request runs on a thread of its own, and one writer (a mutex around stdout) keeps lines whole. It ends
//! when stdin closes or after `shutdown`; jobs still running are cancelled then, VMs keep running.

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::events::Notify;
use super::protocol;
use super::server::{lock, Server};
use crate::paths::Paths;

/// How long the end waits for requests still running (their answers go out if they make it)
const DRAIN: Duration = Duration::from_secs(5);
/// How long the end waits for cancelled jobs to stop (a build deletes its half-built image)
const JOBS_STOP: Duration = Duration::from_secs(30);

enum Input {
    Line(String),
    End,
}

/// The single writer: whole lines, flushed one by one
#[derive(Clone)]
pub(crate) struct Writer {
    out: Arc<Mutex<Box<dyn Write + Send>>>,
}

impl Writer {
    pub fn new(out: impl Write + Send + 'static) -> Self {
        Writer { out: Arc::new(Mutex::new(Box::new(out))) }
    }

    pub fn line(&self, value: &Value) {
        // serde_json never writes a newline inside compact JSON
        let Ok(mut text) = serde_json::to_string(value) else { return };
        text.push('\n');
        let mut out = lock(&self.out);
        if out.write_all(text.as_bytes()).and_then(|_| out.flush()).is_err() {
            // The client is gone; stdin closes too and the loop ends
        }
    }
}

/// Serves `input` → `output` on the data folder `paths` until the input ends or `shutdown`
pub(crate) fn serve(paths: Paths, input: impl BufRead + Send + 'static, output: impl Write + Send + 'static) {
    let writer = Writer::new(output);
    let w = writer.clone();
    let notify: Arc<Notify> =
        Arc::new(move |method: &str, params: Value| w.line(&protocol::notification(method, params)));
    let server = Server::new(paths, notify);
    run(server, writer, input);
}

pub(crate) fn run(server: Server, writer: Writer, input: impl BufRead + Send + 'static) {
    let (tx, rx) = mpsc::channel::<Input>();
    let reader_tx = tx.clone();
    std::thread::spawn(move || {
        for line in input.lines() {
            match line {
                Ok(l) => {
                    if reader_tx.send(Input::Line(l)).is_err() {
                        return;
                    }
                }
                Err(e) => {
                    eprintln!("omaf serve: reading stdin: {e}");
                    break;
                }
            }
        }
        let _ = reader_tx.send(Input::End);
    });
    let in_flight = Arc::new((Mutex::new(0usize), Condvar::new()));
    while let Ok(Input::Line(line)) = rx.recv() {
        if line.trim().is_empty() {
            continue;
        }
        let (server, writer, tx, in_flight) = (server.clone(), writer.clone(), tx.clone(), in_flight.clone());
        *lock(&in_flight.0) += 1;
        std::thread::spawn(move || {
            if let Some(answer) = server.answer(protocol::parse_line(&line)) {
                writer.line(&answer);
            }
            *lock(&in_flight.0) -= 1;
            in_flight.1.notify_all();
            if server.is_shut_down() {
                let _ = tx.send(Input::End);
            }
        });
    }
    // The end: answers still being worked on get a moment, then jobs are cancelled
    let deadline = Instant::now() + DRAIN;
    let mut n = lock(&in_flight.0);
    while *n > 0 {
        let left = deadline.saturating_duration_since(Instant::now());
        if left.is_zero() {
            break;
        }
        n = in_flight.1.wait_timeout(n, left).unwrap_or_else(|e| e.into_inner()).0;
    }
    drop(n);
    server.close(JOBS_STOP);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;
    use serde_json::json;
    use std::io::Cursor;

    /// A `Write` whose bytes the test reads afterwards
    #[derive(Clone, Default)]
    struct Shared(Arc<Mutex<Vec<u8>>>);

    impl Write for Shared {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(buf);
            Ok(buf.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn session(lines: &[&str]) -> Vec<Value> {
        let (_dir, paths) = temp_paths();
        let out = Shared::default();
        let writer = Writer::new(out.clone());
        let w = writer.clone();
        let notify: Arc<Notify> = Arc::new(move |m: &str, p: Value| w.line(&protocol::notification(m, p)));
        let server = Server::for_tests(paths, notify);
        let input = Cursor::new(lines.join("\n").into_bytes());
        run(server, writer, input);
        let text = String::from_utf8(out.0.lock().unwrap().clone()).unwrap();
        text.lines().map(|l| serde_json::from_str(l).expect("every line is one JSON object")).collect()
    }

    fn by_id(answers: &[Value], id: i64) -> &Value {
        answers.iter().find(|a| a["id"] == json!(id)).unwrap_or_else(|| panic!("no answer {id}: {answers:?}"))
    }

    #[test]
    fn a_session_over_lines() {
        let answers = session(&[
            r#"{"jsonrpc":"2.0","id":1,"method":"vms.list"}"#,
            "not json",
            r#"{"jsonrpc":"2.0","id":2,"method":"initialize","params":{"client":"test","clientVersion":"0","apiVersion":1,"language":"de-CH","updateFeed":"https://example.invalid/feed.json"}}"#,
        ]);
        // Requests run concurrently: the list before initialize is refused – unless initialize happened to run first
        let first = by_id(&answers, 1);
        assert!(first["error"]["code"] == json!(-32002) || first["result"].is_object(), "{first}");
        assert!(answers.iter().any(|a| a["id"].is_null() && a["error"]["code"] == json!(-32700)));
        let init = &by_id(&answers, 2)["result"];
        assert_eq!(init["apiVersion"], json!(1));
        assert_eq!(init["omafVersion"], json!(crate::VERSION));
        assert!(init["dataFolder"].as_str().unwrap().contains("vmtest-"));
        assert!(init["configFile"].as_str().unwrap().ends_with("config.json"));
        assert!(init["host"]["system"].is_string());
    }

    #[test]
    fn shutdown_ends_the_session_and_unknown_methods_are_refused() {
        let answers = session(&[
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"apiVersion":1}}"#,
            r#"{"jsonrpc":"2.0","id":2,"method":"no.such"}"#,
            r#"{"jsonrpc":"2.0","id":3,"method":"jobs.list"}"#,
            r#"{"jsonrpc":"2.0","id":4,"method":"jobs.cancel","params":{"id":"j9"}}"#,
            r#"{"jsonrpc":"2.0","id":5,"method":"jobs.log","params":{}}"#,
        ]);
        // Requests run concurrently; an initialize that lost the race is answered with -32002 – only check those
        // that ran after it
        assert_eq!(by_id(&answers, 1)["result"]["apiVersion"], json!(1));
        let unknown = &by_id(&answers, 2)["error"]["code"];
        assert!(unknown == &json!(-32601) || unknown == &json!(-32002));
        let answers = session(&[r#"{"jsonrpc":"2.0","id":9,"method":"shutdown"}"#]);
        assert_eq!(by_id(&answers, 9)["result"], Value::Null);
    }

    #[test]
    fn wrong_api_versions_are_refused() {
        let answers = session(&[r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"apiVersion":2}}"#]);
        assert_eq!(by_id(&answers, 1)["error"]["code"], json!(-32602));
    }
}
