// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of QMPClient.swift – work package F2. Done: the type. Skeleton: the functions (`todo!`).
//!
//! QEMU Machine Protocol over the instance's Unix socket (line-based JSON).
//! QEMU also sends events between command and response (plan B13) – `execute`
//! skips them and records them in `events`.
#![allow(unused_variables, dead_code)]

use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::error::Result;

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
        todo!("port from QMPClient.swift")
    }

    /// Sends `{"execute": command}` and returns `return` (an empty object when missing); QMP errors are errors
    /// ("QMP <command>: <desc>").
    pub fn execute(&mut self, command: &str) -> Result<Value> {
        todo!("port from QMPClient.swift")
    }

    /// `query-status` → e.g. "running", "paused", "shutdown" ("unknown" without a status)
    pub fn status(&mut self) -> Result<String> {
        todo!("port from QMPClient.swift")
    }

    /// The next non-empty line as a JSON object; "QMP: connection closed or timed out" at EOF or timeout
    fn read_message(&mut self) -> Result<Map<String, Value>> {
        todo!("port from QMPClient.swift")
    }

    /// "QMP: not a JSON object" for anything but an object
    pub(crate) fn parse(line: &[u8]) -> Result<Map<String, Value>> {
        todo!("port from QMPClient.swift")
    }
}
