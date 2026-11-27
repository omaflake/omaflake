// SPDX-License-Identifier: AGPL-3.0-or-later
//! A running VM's diagnosis (`diagnostics`, plan B162, docs/api.md): one row per check with its status, what was found,
//! a hint while it failed and what can be done (confirm a sound, add omaflake's SSH key), the run's log below –
//! the Doctor's style. The checks themselves are `crate::diagnostics`; the server keeps the newest run per VM
//! (`DiagnosisRecord`) and runs one when the page is subscribed and the VM has none since it started.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Ctx;
use super::build::{LogRef, log_ref};
use super::common::*;
use super::text::{Text, tf};
use crate::api::types::JobState;
use crate::diagnostics::{CHECKS, CheckResult, CheckStatus, OWN_DETAILS, title};
use crate::json::Date;
use crate::models::{GuestKind, VmStatus};

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
    /// "<VM> · <recipe>"
    pub subtitle: Text,
    pub state: DiagnosisState,
    /// When the newest run ended
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::json::iso8601::option")]
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

fn icon(id: &str) -> &'static str {
    match id {
        "agent" => "vm",
        "ssh" => "terminal",
        "clipboard" => "clipboard",
        "share" => "folder",
        "network" => "globe",
        "sound" => "sound",
        "microphone" => "microphone",
        "camera" => "camera",
        "midi" => "midi",
        _ => "gauge",
    }
}

/// What to do about a check that failed (or waits for the user)
fn hint(id: &str, status: CheckStatus, kind: GuestKind) -> Option<Text> {
    let mac = kind == GuestKind::Macos;
    let text = match (id, status) {
        ("agent", CheckStatus::Fail) if mac => {
            "macOS guests start the omaflake agent over SSH – check SSH. Images built before the diagnosis lack it: rebuild the image, then upgrade the VM."
        }
        ("agent", CheckStatus::Fail) => {
            "The omaflake agent starts with the guest. Images built before the diagnosis lack it: rebuild the image, then upgrade the VM."
        }
        ("ssh", CheckStatus::Fail) if mac => {
            "omaflake's key is not in the guest's authorized_keys (images before 0.7.4) – Add omaflake's Key … appends it with your own key; then the clipboard helper and the agent start."
        }
        ("ssh", CheckStatus::Fail) => "SSH reaches the guest through its forwarded port – is the guest still starting?",
        ("clipboard", CheckStatus::Fail) if mac => {
            "macOS guests: the clipboard helper is started over SSH – check SSH first."
        }
        ("clipboard", CheckStatus::Fail) => "The agent carries the clipboard – check the agent first.",
        ("share", CheckStatus::Fail) => {
            "A guest mounts each volume at /Volumes/<name> – images before 0.8.1 cannot: rebuild the image, then upgrade the VM."
        }
        ("network", CheckStatus::Fail) => "Check the VM's network setting and this computer's connection.",
        ("sound", CheckStatus::Confirm) => "A short tone played in the guest – did you hear it?",
        ("sound", CheckStatus::Fail) => "The guest found no output device, or playing failed – the detail says why.",
        ("microphone", CheckStatus::Confirm) => {
            "Hardly anything came in – say something while it records, then Run Again."
        }
        ("microphone", CheckStatus::Fail) => {
            "The guest found no input device, or recording failed – the detail says why."
        }
        ("camera", CheckStatus::Fail) => {
            "The camera reaches NixOS guests as a loopback device – the detail says what is missing."
        }
        ("midi", CheckStatus::Fail) => "A device may have failed in the guest – the detail names it.",
        ("disk", CheckStatus::Fail) => {
            "Little space left: remove files in the guest, or create the VM again with a bigger disk."
        }
        _ => return None,
    };
    Some(Text::new(text))
}

/// A detail as a text: omaflake's own words translated, what a guest or tool said as it is
fn detail(d: &str) -> Option<Text> {
    if d.is_empty() {
        None
    } else if OWN_DETAILS.contains(&d) {
        Some(Text::new(d))
    } else {
        Some(Text::verbatim(d))
    }
}

/// The terminal line that appends omaflake's public key to a macOS guest's authorized_keys – with the user's own key
/// (no BatchMode: their passphrase or agent works)
pub(crate) fn add_key_line(ctx: &Ctx, user: &str, address: &str) -> String {
    let key = crate::ssh_identity::SshIdentity::new(&ctx.paths).public_key_file();
    let key = crate::ssh_command::SshCommand::shell_line(&[key.to_string_lossy().into_owned()]);
    format!("ssh -p 22 {user}@{address} 'mkdir -p ~/.ssh && cat >> ~/.ssh/authorized_keys' < {key}")
}

pub(crate) fn page(ctx: &Ctx, p: &DiagnosticsParams) -> Option<DiagnosticsPage> {
    let v = ctx.vms.iter().find(|v| v.name == p.name)?;
    let name = v.name.as_str();
    let running = v.status == VmStatus::Running;
    let record = ctx.diagnostics.get(name).filter(|_| running);
    let job = record.and_then(|r| ctx.jobs.iter().find(|j| j.id == r.job));
    let active = job.is_some_and(|j| j.state.is_active());
    let state = match (record, job) {
        (None, _) => DiagnosisState::Idle,
        (Some(_), _) if active => DiagnosisState::Running,
        (Some(_), Some(j)) if j.state == JobState::Finished => DiagnosisState::Finished,
        (Some(_), Some(_)) => DiagnosisState::Failed,
        // The job left the list (dismissed): what was found is still there
        (Some(r), None) if r.finished.is_some() => DiagnosisState::Finished,
        (Some(_), None) => DiagnosisState::Failed,
    };
    let checks = CHECKS
        .iter()
        .map(|id| {
            let found = record.and_then(|r| r.checks.iter().find(|c| c.id == *id));
            let status = found.map(|c| c.status).unwrap_or(CheckStatus::Pending);
            let mut actions = Vec::new();
            if status == CheckStatus::Confirm && matches!(*id, "sound" | "microphone") {
                let confirm = |ok: bool, label: &str, icon: &str| {
                    Action::call(
                        if ok { "heard" } else { "notHeard" },
                        Text::new(label),
                        Some(icon),
                        "diagnostics.confirm",
                        json!({"name": name, "check": id, "ok": ok}),
                    )
                };
                actions.push(confirm(true, "Heard It", "ok"));
                actions.push(confirm(false, "Not Heard", "cancel"));
            }
            if *id == "ssh"
                && status == CheckStatus::Fail
                && v.kind == GuestKind::Macos
                && let Some(address) = &v.address
            {
                actions.push(
                    Action::call(
                        "addKey",
                        Text::new("Add omaflake's Key …"),
                        Some("key"),
                        "ui.terminal",
                        json!({"line": add_key_line(ctx, &v.user, address)}),
                    )
                    .help(Text::new("Appends omaflake's public key to the guest's authorized_keys – ssh asks for your own key's passphrase or the guest's password")),
                );
            }
            DiagnosticsCheck {
                id: id.to_string(),
                title: Text::new(title(id)),
                icon: icon(id).into(),
                status,
                detail: found.and_then(|c| detail(&c.detail)),
                hint: hint(id, status, v.kind),
                actions,
            }
        })
        .collect();
    let file = record.map(|r| r.file.clone());
    let run = if active {
        Action::call("run", Text::new("Running …"), Some("refresh"), "diagnostics.run", json!({"name": name}))
            .busy(true)
            .disabled(Text::new("The diagnosis runs"))
    } else {
        let label = if record.is_some() { "Run Again" } else { "Run" };
        Action::call("run", Text::new(label), Some("refresh"), "diagnostics.run", json!({"name": name}))
            .enabled_if(running, Some(Text::new("Start the VM first")))
            .primary()
    };
    let no_log = Some(Text::new("No diagnosis yet"));
    let actions = vec![
        run,
        Action::call("copy", Text::new("Copy Log"), Some("copy"), "diagnostics.copy", json!({"name": name}))
            .help(Text::new("Puts the diagnosis and its log on this computer's clipboard"))
            .enabled_if(file.is_some(), no_log.clone()),
        Action::call("reveal", Text::new("Show Log File"), Some("folder"), "ui.reveal", json!({"path": file}))
            .enabled_if(file.is_some(), no_log),
    ];
    Some(DiagnosticsPage {
        title: Text::new("Diagnosis"),
        subtitle: Text::verbatim(format!("{name} · {}", super::vms::recipe_name(ctx, &v.recipe))),
        state,
        checked: record.and_then(|r| r.finished),
        intro: if running {
            Text::new(
                "Checks what this VM's features do – the guest's half through its omaflake agent, this computer's half here. A switch that is off skips its check; sound and the microphone ask you.",
            )
        } else {
            tf!("Start “%@” first – the diagnosis checks a running VM.", name)
        },
        checks,
        actions,
        log: job.map(log_ref),
        file,
    })
}
