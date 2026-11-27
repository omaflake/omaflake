// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Build Image page (`build`): one recipe's build – start it (a macOS recipe asks the guest user's password), its
//! place in the queue, progress, outcome and log (the app's `BuildView`, `BuildProgressView`, `BuildOutcome`).

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::text::Text;
use crate::types::JobDto;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildPage {
    /// "Build Image"
    pub title: Text,
    pub recipe: String,
    pub name: String,
    pub kind: Badge,
    /// Lab, the description (as a tooltip in the app)
    pub badges: Vec<Badge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    /// `idle` (no build in this session) or the latest build's state
    pub state: BuildState,
    /// Build / Add to Queue (macOS: asks the password unless the credential has one) – or Cancel / Remove from Queue
    /// while it waits or runs
    pub actions: Vec<Action>,
    /// What the image is built with besides the recipe (plan B137): the credential and the look, preset with the
    /// settings' defaults; their values go into the build action's params (`valueParam`)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub fields: Vec<Field>,
    /// What the Doctor reports against building (while no build runs)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blockers: Option<Blockers>,
    /// macOS without a base: the way to create it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_base: Option<Hint>,
    /// What the recipe is made of and what is wrong with it (plan B142): missing parts (with "Install Missing Parts …"
    /// – they block the build), warnings (a need no module provides, incompatible modules, keys that overlap or break
    /// the key rule, a locale the recipe is not made for), unfree software
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Hint>,
    /// "Waiting in the queue"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<Progress>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outcome: Option<Outcome>,
    /// "newest image: …" when no build of this session is shown
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub newest: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<LogRef>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<JobDto>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum BuildState {
    Idle,
    Queued,
    Checking,
    Running,
    Finished,
    Failed,
    Cancelled,
}

/// Step "2/4", what it does, packages done of announced, a bar over all steps; the UI counts the running time from
/// `started` and the quiet time from `lastOutput` (orange after `quietWarning` seconds, with `quietHelp`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Progress {
    pub step: Text,
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<Text>,
    pub fraction: f64,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::time::iso8601::option")]
    pub started: Option<crate::time::Date>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::time::iso8601::option")]
    pub last_output: Option<crate::time::Date>,
    pub quiet_warning: u32,
    pub quiet_help: Text,
}

/// How the build ended, said above its log
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub state: BuildState,
    pub icon: String,
    pub tone: Tone,
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Text>,
    /// "New VM …" from the image just built
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
}

/// Where the log is: `jobs.log {id: job, from}` reads what it has so far (from the oldest line kept), the `job.log`
/// events (topic `jobs`) bring the new ones; the whole log in `file`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LogRef {
    /// The job's id (a build, the macOS base's creation)
    pub job: String,
    /// The first line `jobs.log` still has (older ones than the newest 5000 are gone)
    pub from: usize,
    /// Lines so far (`jobs.log`'s `next`)
    pub lines: usize,
    /// "(n routine lines … hidden – full log: …)" once it ended
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub file: Option<String>,
    /// "Full Log …": the log file in the system's viewer
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full: Option<Action>,
}

#[derive(Debug, Deserialize)]
pub struct BuildParams {
    pub recipe: String,
    /// The form's values: `credential` ("" = this computer) and `look` ("" = the recipe's own)
    #[serde(default)]
    pub values: BuildValues,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BuildValues {
    pub credential: Option<String>,
    pub look: Option<String>,
    pub locale: Option<String>,
    /// macOS: the build VM in a window
    pub window: Option<bool>,
    /// macOS: from the credential's unchained base (plan B184)
    pub unchained: Option<bool>,
}
