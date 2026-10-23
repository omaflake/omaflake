// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of QMPClient.swift.
//!
//! QEMU Machine Protocol over the instance's Unix socket (line-based JSON).
//! QEMU also sends events between command and response (plan B13) – `execute`
//! skips them and records them in `events`.
//!
//! Not 1:1: Swift kept a raw descriptor (closed in `deinit`, plan B105); here the `UnixStream` closes itself when the
//! client is dropped, also when `connect` fails halfway. A read interrupted by a signal (EINTR) is retried instead of
//! counting as "closed or timed out".

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::error::Result;
use crate::sockets::connect_unix;
use crate::vm_err;

#[derive(Debug)]
pub struct QmpClient {
    stream: UnixStream,
    buffer: Vec<u8>,
    /// Names of the events seen while waiting for answers
    pub events: Vec<String>,
}

impl QmpClient {
    /// Swift's default timeout
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

    /// Connects (`connect_unix`) and switches to command mode (`qmp_capabilities`); "Unexpected QMP greeting" unless
    /// the first message has "QMP".
    pub fn connect(socket: &Path, timeout: Duration) -> Result<QmpClient> {
        let stream = connect_unix(socket, timeout)?;
        let mut client = QmpClient { stream, buffer: Vec::new(), events: Vec::new() };
        let greeting = client.read_message()?;
        if !greeting.contains_key("QMP") {
            return Err(vm_err!("Unexpected QMP greeting"));
        }
        client.execute("qmp_capabilities")?;
        Ok(client)
    }

    /// Sends `{"execute": command}` and returns `return` (an empty object when missing); QMP errors are errors
    /// ("QMP <command>: <desc>").
    pub fn execute(&mut self, command: &str) -> Result<Value> {
        let mut line = serde_json::to_vec(&serde_json::json!({ "execute": command }))?;
        line.push(b'\n');
        self.stream.write_all(&line).map_err(|_| vm_err!("QMP: send failed"))?;
        loop {
            let mut msg = self.read_message()?;
            if let Some(Value::String(event)) = msg.get("event") {
                self.events.push(event.clone());
                continue;
            }
            if let Some(Value::Object(error)) = msg.get("error") {
                let desc = error.get("desc").and_then(Value::as_str).unwrap_or("error");
                return Err(vm_err!("QMP {command}: {desc}"));
            }
            return Ok(msg.remove("return").unwrap_or_else(|| Value::Object(Map::new())));
        }
    }

    /// `query-status` → e.g. "running", "paused", "shutdown" ("unknown" without a status)
    pub fn status(&mut self) -> Result<String> {
        let r = self.execute("query-status")?;
        Ok(r.get("status").and_then(Value::as_str).unwrap_or("unknown").to_string())
    }

    /// The next non-empty line as a JSON object; "QMP: connection closed or timed out" at EOF or timeout
    fn read_message(&mut self) -> Result<Map<String, Value>> {
        loop {
            if let Some(nl) = self.buffer.iter().position(|&b| b == b'\n') {
                let line: Vec<u8> = self.buffer.drain(..=nl).take(nl).collect();
                if line.is_empty() {
                    continue;
                }
                return Self::parse(&line);
            }
            let mut chunk = [0u8; 4096];
            match self.stream.read(&mut chunk) {
                Ok(n) if n > 0 => self.buffer.extend_from_slice(&chunk[..n]),
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                _ => return Err(vm_err!("QMP: connection closed or timed out")),
            }
        }
    }

    /// "QMP: not a JSON object" for anything but an object
    pub(crate) fn parse(line: &[u8]) -> Result<Map<String, Value>> {
        match serde_json::from_slice::<Value>(line)? {
            Value::Object(obj) => Ok(obj),
            _ => Err(vm_err!("QMP: not a JSON object")),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{BufRead, BufReader};
    use std::os::unix::net::UnixListener;
    use std::thread::JoinHandle;

    #[test]
    fn parses_event_and_return() {
        // Order as observed in the boot probe (plan B13)
        let ev =
            QmpClient::parse(br#"{"timestamp": {"seconds": 1, "microseconds": 2}, "event": "POWERDOWN"}"#).unwrap();
        assert_eq!(ev["event"], "POWERDOWN");
        let ret = QmpClient::parse(br#"{"return": {}}"#).unwrap();
        assert!(ret.contains_key("return"));
        assert_eq!(QmpClient::parse(b"[1]").unwrap_err().message(), "QMP: not a JSON object");
    }

    /// A fake QEMU: sends `greeting`, then answers each command line with the replies `answer` gives; returns the
    /// commands it received.
    fn fake_qemu(
        dir: &Path,
        greeting: &'static str,
        answer: fn(&str) -> Vec<String>,
    ) -> (std::path::PathBuf, JoinHandle<Vec<String>>) {
        let socket = dir.join("qmp.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let t = std::thread::spawn(move || {
            let (mut conn, _) = listener.accept().unwrap();
            conn.write_all(greeting.as_bytes()).unwrap();
            let mut seen = vec![];
            let mut reader = BufReader::new(conn.try_clone().unwrap());
            let mut line = String::new();
            while reader.read_line(&mut line).unwrap_or(0) > 0 {
                let cmd: Value = serde_json::from_str(line.trim()).unwrap();
                let name = cmd["execute"].as_str().unwrap().to_string();
                for reply in answer(&name) {
                    conn.write_all(reply.as_bytes()).unwrap();
                }
                seen.push(name);
                line.clear();
            }
            seen
        });
        (socket, t)
    }

    const GREETING: &str = "{\"QMP\": {\"version\": {}, \"capabilities\": []}}\r\n";

    #[test]
    fn talks_to_a_fake_qemu() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, t) = fake_qemu(dir.path(), GREETING, |cmd| match cmd {
            "qmp_capabilities" => vec!["{\"return\": {}}\r\n".into()],
            // An event arrives between command and answer, split over two writes, with an empty line before
            "query-status" => vec![
                "\n{\"timestamp\": {\"seconds\": 1, \"microseconds\": 2}, \"ev".into(),
                "ent\": \"RESUME\"}\r\n{\"return\": {\"status\": \"running\", \"running\": true}}\r\n".into(),
            ],
            "system_powerdown" => vec!["{\"return\": {}}\n".into()],
            "nothing" => vec!["{\"id\": 1}\n".into()],
            _ => vec![
                "{\"error\": {\"class\": \"CommandNotFound\", \"desc\": \"The command bad has not been found\"}}\n"
                    .into(),
            ],
        });
        let mut qmp = QmpClient::connect(&socket, QmpClient::DEFAULT_TIMEOUT).unwrap();
        assert_eq!(qmp.status().unwrap(), "running");
        assert_eq!(qmp.events, ["RESUME"]);
        assert_eq!(qmp.execute("system_powerdown").unwrap(), serde_json::json!({}));
        assert_eq!(qmp.execute("nothing").unwrap(), serde_json::json!({}));
        assert_eq!(qmp.execute("bad").unwrap_err().message(), "QMP bad: The command bad has not been found");
        drop(qmp);
        assert_eq!(t.join().unwrap(), ["qmp_capabilities", "query-status", "system_powerdown", "nothing", "bad"]);
    }

    #[test]
    fn refuses_another_greeting_and_times_out() {
        let dir = tempfile::tempdir().unwrap();
        let (socket, t) = fake_qemu(dir.path(), "{\"hello\": 1}\n", |_| vec![]);
        assert_eq!(
            QmpClient::connect(&socket, QmpClient::DEFAULT_TIMEOUT).unwrap_err().message(),
            "Unexpected QMP greeting"
        );
        t.join().unwrap();

        // A server that never answers the capabilities
        let dir = tempfile::tempdir().unwrap();
        let (socket, t) = fake_qemu(dir.path(), GREETING, |_| vec![]);
        assert_eq!(
            QmpClient::connect(&socket, Duration::from_millis(200)).unwrap_err().message(),
            "QMP: connection closed or timed out"
        );
        t.join().unwrap();
    }
}
