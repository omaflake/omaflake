// SPDX-License-Identifier: AGPL-3.0-or-later
//! The diagnosis of a VM (`omaf diagnose`, plan B162): its checks and their results.

use serde::{Deserialize, Serialize};

/// The checks, in the order they run
pub const CHECKS: [&str; 10] =
    ["agent", "ssh", "clipboard", "share", "network", "sound", "microphone", "camera", "midi", "disk"];

/// A check's title (a key of the views' texts)
pub fn title(id: &str) -> &'static str {
    match id {
        "agent" => "Agent",
        "ssh" => "SSH",
        "clipboard" => "Clipboard",
        "share" => "Volumes",
        "network" => "Network",
        "sound" => "Sound",
        "microphone" => "Microphone",
        "camera" => "Camera",
        "midi" => "MIDI",
        "disk" => "Disk",
        _ => "?",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pending,
    Running,
    Pass,
    Fail,
    /// Only the user can tell (did you hear it?)
    Confirm,
    Skipped,
}

impl CheckStatus {
    /// The log's mark
    pub fn mark(self) -> &'static str {
        match self {
            CheckStatus::Pass => "✓",
            CheckStatus::Fail => "✗",
            CheckStatus::Confirm => "?",
            CheckStatus::Skipped => "–",
            CheckStatus::Pending | CheckStatus::Running => "·",
        }
    }

    pub fn is_done(self) -> bool {
        !matches!(self, CheckStatus::Pending | CheckStatus::Running)
    }
}

/// One check's outcome (the job's `result.checks[]`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub status: CheckStatus,
    pub detail: String,
}

impl CheckResult {
    pub fn new(id: &str, status: CheckStatus, detail: impl Into<String>) -> CheckResult {
        CheckResult { id: id.into(), status, detail: detail.into() }
    }

    /// `✓ Clipboard: both ways`
    pub fn line(&self) -> String {
        if self.detail.is_empty() {
            format!("{} {}", self.status.mark(), title(&self.id))
        } else {
            format!("{} {}: {}", self.status.mark(), title(&self.id), self.detail)
        }
    }
}
