// SPDX-License-Identifier: AGPL-3.0-or-later
//! The bridge's control socket (`omaf bridge --control <socket>`, plan B162; spec §11.9 "Relay"): what omaf-runner's
//! control socket is for a macOS guest. One JSON object per line in, one per line back – the format of
//! `RunnerClient` (`{"command": …}`):
//!
//! - `status` → `{"state": "running", "pid"}`
//! - `diagnose {check, args?}` → the agent's answer (`0x0601`, cleaned) or `{"error"}` (no diagnosis in this agent,
//!   no answer within 15 s)
//! - `midi` → the host side's MIDI state (`{"endpoints", "in", "out", "agent"}`, strings as omaf-runner's) or
//!   `{"error"}` when the VM runs without MIDI
//!
//! Each connection is served on its own thread; the relay sends one request to the agent at a time.

use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};

use super::midi::MidiHost;
use crate::Log;
use crate::agent_protocol::{AgentProtocol, AgentSender, DiagnoseRelay};

/// The commands the control socket knows
pub const COMMANDS: [&str; 3] = ["status", "diagnose", "midi"];

/// What the control socket answers from
#[derive(Clone)]
pub struct Control {
    pub relay: Arc<DiagnoseRelay>,
    pub sender: AgentSender,
    /// The host's MIDI, when the VM's switch is on and the host has a sequencer
    pub midi: Option<Arc<dyn MidiHost>>,
}

impl Control {
    /// The answer to one request
    pub fn answer(&self, request: &Value) -> Value {
        match request.get("command").and_then(Value::as_str).unwrap_or("") {
            "status" => json!({"state": "running", "pid": std::process::id().to_string()}),
            "diagnose" => {
                let Some(check) = request.get("check").and_then(Value::as_str).filter(|c| !c.is_empty()) else {
                    return json!({"error": "diagnose needs a check"});
                };
                if !self.relay.enabled() {
                    return json!({"error": "the guest's agent has no diagnosis (an image built before it, or no agent yet)"});
                }
                match self.relay.request(&self.sender, check, request.get("args"), AgentProtocol::DIAGNOSE_TIMEOUT) {
                    Ok(answer) => answer,
                    Err(why) => json!({"error": why}),
                }
            }
            "midi" => match &self.midi {
                Some(m) => m.describe(),
                None => json!({"error": "this VM runs without MIDI (omaf set <name> --midi, then restart)"}),
            },
            other => json!({"error": format!("Unknown command {other}; available: {}", COMMANDS.join(", "))}),
        }
    }
}

/// Listens on `path` (a socket left from an earlier run is replaced) and serves `control` on a thread for the life of
/// the process
pub fn serve(path: &Path, control: Control, log: Arc<Log<'static>>) -> Result<(), String> {
    if path.exists() {
        // A leftover socket file of an earlier run (no user content)
        let _ = std::fs::remove_file(path);
    }
    let listener = UnixListener::bind(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let shown: PathBuf = path.to_path_buf();
    std::thread::Builder::new()
        .name("control".into())
        .spawn(move || {
            for stream in listener.incoming() {
                let Ok(stream) = stream else { continue };
                let c = control.clone();
                std::thread::spawn(move || serve_one(stream, &c));
            }
        })
        .map_err(|e| format!("cannot start the control thread: {e}"))?;
    log(&format!("control: {}", shown.display()));
    Ok(())
}

/// One request line (at most 64 KiB, 5 s to arrive), one answer line
fn serve_one(stream: UnixStream, control: &Control) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    let mut line = Vec::new();
    {
        use std::io::Read;
        let mut reader = BufReader::new((&stream).take(64 * 1024));
        if reader.read_until(b'\n', &mut line).is_err() {
            return;
        }
    }
    let request: Value = serde_json::from_slice(&line).unwrap_or(Value::Null);
    let mut out = serde_json::to_vec(&control.answer(&request)).unwrap_or_else(|_| b"{}".to_vec());
    out.push(b'\n');
    let _ = (&stream).write_all(&out);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control() -> (Control, UnixStream) {
        let (guest, host) = UnixStream::pair().unwrap();
        let link = crate::agent_protocol::AgentLink::new(
            host,
            std::env::temp_dir().join("unused-agent.json"),
            "omaflake",
            5,
            Default::default(),
            Arc::new(|_: &str| {}),
        )
        .unwrap();
        (Control { relay: link.diagnoses(), sender: link.sender(), midi: None }, guest)
    }

    #[test]
    fn answers_over_the_socket() {
        let (c, _guest) = control();
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bridge.sock");
        std::fs::write(&path, "left over").unwrap();
        serve(&path, c, Arc::new(|_: &str| {})).unwrap();
        let ask = |request: Value| -> Value {
            let s = crate::sockets::connect_unix(&path, Duration::from_secs(2)).unwrap();
            (&s).write_all(format!("{request}\n").as_bytes()).unwrap();
            let mut line = String::new();
            BufReader::new(&s).read_line(&mut line).unwrap();
            serde_json::from_str(&line).unwrap()
        };
        assert_eq!(ask(json!({"command": "status"}))["state"], "running");
        assert!(ask(json!({"command": "midi"}))["error"].as_str().unwrap().contains("without MIDI"));
        let e = ask(json!({"command": "diagnose", "check": "info"}));
        assert!(e["error"].as_str().unwrap().contains("no diagnosis"), "{e}");
        assert!(ask(json!({"command": "diagnose"}))["error"].as_str().unwrap().contains("needs a check"));
        assert!(ask(json!({"command": "nope"}))["error"].as_str().unwrap().contains("status, diagnose, midi"));
    }
}
