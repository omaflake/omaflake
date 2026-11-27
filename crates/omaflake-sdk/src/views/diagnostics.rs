// SPDX-License-Identifier: AGPL-3.0-or-later
//! A running VM's diagnosis (`diagnostics`, plan B162, docs/api.md): one row per check with its status, what was found,
//! a hint while it failed and what can be done (confirm a sound, add omaflake's SSH key), the run's log below –
//! the Doctor's style. The checks themselves are `crate::diagnostics`; the server keeps the newest run per VM
//! (`DiagnosisRecord`) and runs one when the page is subscribed and the VM has none since it started.

use serde::{Deserialize, Serialize};

use super::build::LogRef;
use super::common::*;
use crate::diagnostics::{CHECKS, CheckResult, CheckStatus};
use crate::text::Text;
use crate::time::Date;

/// The newest diagnosis of a VM in its current run (the server's state)
#[derive(Debug, Clone, PartialEq)]
pub struct DiagnosisRecord {
    /// The VM's run it belongs to (`diagnostics::run_id`)
    pub run: Option<String>,
    /// Its job (`diagnostics`)
    pub job: String,
    /// Every check: pending, running or done (confirmed ones as pass or fail)
    pub checks: Vec<CheckResult>,
    pub file: String,
    pub finished: Option<Date>,
}

impl DiagnosisRecord {
    pub fn new(run: Option<String>, file: String) -> DiagnosisRecord {
        DiagnosisRecord {
            run,
            job: String::new(),
            checks: CHECKS.iter().map(|id| CheckResult::new(id, CheckStatus::Pending, "")).collect(),
            file,
            finished: None,
        }
    }

    /// Replaces the check of the same id
    pub fn set(&mut self, r: &CheckResult) {
        if let Some(x) = self.checks.iter_mut().find(|x| x.id == r.id) {
            *x = r.clone();
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsPage {
    /// "Diagnosis"
    pub title: Text,
    /// `"<VM> · <recipe>"`
    pub subtitle: Text,
    pub state: DiagnosisState,
    /// When the newest run ended
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::time::iso8601::option")]
    pub checked: Option<Date>,
    pub intro: Text,
    pub checks: Vec<DiagnosticsCheck>,
    /// Run (Again), Copy Log, Show Log File
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<LogRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DiagnosisState {
    Idle,
    Running,
    Finished,
    Failed,
}

/// One check's row
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsCheck {
    pub id: String,
    pub title: Text,
    pub icon: String,
    pub status: CheckStatus,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Text>,
    /// What to do, while it failed or waits for the user
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<Text>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Deserialize)]
pub struct DiagnosticsParams {
    pub name: String,
}
