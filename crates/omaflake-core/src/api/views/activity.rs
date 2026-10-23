// SPDX-License-Identifier: AGPL-3.0-or-later
//! The activity list (`activity`): what happened in this server – actions done or refused, jobs that ended, VMs that
//! ended or are ready, the core's own messages – worded once here for every UI (the app's `AppModel.message`,
//! `finished(_:)`). Newest first, at most `MAX` entries. Whether the user has seen them is the UI's business.

use std::collections::VecDeque;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::common::*;
use super::text::{tf, Text};
use crate::api::protocol::RpcError;
use crate::api::types::*;
use crate::models::RecipeId;

/// Entries kept (the app kept 30; a TUI's list may be longer)
pub const MAX: usize = 100;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    /// Rising: a UI remembers the newest it has shown (new = unseen)
    pub id: u64,
    #[serde(with = "crate::json::iso8601")]
    pub time: DateTime<Utc>,
    pub level: Level,
    pub text: Text,
    /// What a system notification says, when one belongs to it (VM ready or ended, image built or failed) – the UI
    /// posts it if notifications are on (Settings) and its window is not in front
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notification: Option<Notification>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Notification {
    pub title: Text,
    pub body: Text,
}

#[derive(Debug, Default)]
pub(crate) struct ActivityLog {
    next: u64,
    entries: VecDeque<ActivityEntry>,
}

impl ActivityLog {
    pub fn push(&mut self, level: Level, text: Text, notification: Option<Notification>) {
        self.next += 1;
        self.entries.push_front(ActivityEntry { id: self.next, time: crate::json::now(), level, text, notification });
        self.entries.truncate(MAX);
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn list(&self) -> Vec<ActivityEntry> {
        self.entries.iter().cloned().collect()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    #[serde(flatten)]
    pub head: ListHead,
    pub entries: Vec<ActivityEntry>,
}

pub fn page(entries: &[ActivityEntry]) -> ActivityPage {
    let mut head = ListHead::new("Activity", entries.len(), entries.len());
    head.actions = vec![Action::call("clear", Text::new("Clear History"), Some("trash"), "activity.clear", json!({}))
        .enabled_if(!entries.is_empty(), None)];
    if entries.is_empty() {
        head.empty = Some(Hint::new("activity", Text::new("Nothing happened yet in this session."), Tone::Normal));
    }
    ActivityPage { head, entries: entries.to_vec() }
}

fn notification(title: &str, body: Text) -> Option<Notification> {
    Some(Notification { title: Text::new(title), body })
}

/// The recipe's short name ("dev") – `RecipeText.name`
fn name_of(id: &str) -> String {
    RecipeId::name_of_id(id)
}

/// What a job that ended says (`AppModel.finished`) – the core sends no `message` for it (api.md §6)
pub fn job_ended(job: &JobDto) -> Option<(Level, Text, Option<Notification>)> {
    let name = job.recipe.as_deref().map(name_of).unwrap_or_else(|| job.title.clone());
    let error = job.error.clone().unwrap_or_default();
    let result = |_: ()| job.result.clone().unwrap_or(Value::Null);
    use JobKind as K;
    use JobState as S;
    Some(match (job.kind, job.state) {
        (K::Build, S::Finished) => (
            Level::Info,
            tf!("Image %@ ready", &name),
            notification("Image ready", tf!("%@ can be used for new VMs.", &name)),
        ),
        (K::Build, S::Failed) => (
            Level::Error,
            tf!("Build %@: %@", &name, &error),
            notification("Build failed", tf!("%@ – the log is in the Images tab.", &name)),
        ),
        (K::Build, S::Cancelled) => (Level::Info, tf!("Build of %@ cancelled", &name), None),
        (K::IpswDownload, S::Finished) => {
            let version = result(()).get("version").and_then(Value::as_str).map(str::to_string);
            (
                Level::Info,
                version.map(|v| tf!("macOS %@ downloaded", v)).unwrap_or_else(|| Text::new("macOS downloaded")),
                None,
            )
        }
        (K::IpswDownload, S::Cancelled) => (Level::Info, Text::new("Download paused – it continues from there"), None),
        (K::MacosInstall, S::Failed) => (Level::Error, tf!("macOS base: %@", &error), None),
        (K::DataMove, S::Finished) => {
            let folder = result(()).get("dataFolder").and_then(Value::as_str).unwrap_or(&job.title).to_string();
            (Level::Info, tf!("Data folder: %@", folder), None)
        }
        (K::BuilderInstallNix | K::BuilderSetup | K::BuilderApply | K::BuilderCleanUp, S::Failed) => {
            let first = match &job.log_file {
                Some(f) => tf!("Failed – see %@", f),
                None => Text::new("Failed"),
            };
            let tail: String = error.chars().rev().take(400).collect::<Vec<_>>().into_iter().rev().collect();
            (Level::Error, tf!("%@\n%@", first, tail), None)
        }
        (K::BuilderInstallNix, S::Finished) => (Level::Info, Text::new("Nix installed"), None),
        (K::BuilderSetup | K::BuilderApply, S::Finished) => {
            (Level::Info, Text::new("Linux builder settings applied"), None)
        }
        (K::BuilderCleanUp, S::Finished) => {
            let message = result(()).get("message").and_then(Value::as_str).map(str::to_string);
            (Level::Info, message.map(Text::verbatim).unwrap_or_else(|| Text::new("Linux builder cleaned up")), None)
        }
        (K::RecipeInstall, S::Finished) => {
            let written: Vec<String> =
                serde_json::from_value::<InstallResult>(result(())).map(|r| r.installed).unwrap_or_default();
            let id = job.recipe.clone().unwrap_or_else(|| job.title.clone());
            if written.is_empty() {
                (Level::Info, tf!("%@ is up to date", RecipeId::qualified(&id)), None)
            } else {
                let names: Vec<String> = written.iter().map(|w| RecipeId::qualified(w)).collect();
                (Level::Info, tf!("Installed %@", names.join(", ")), None)
            }
        }
        (K::Import, S::Finished) => {
            let r: ImportResult = serde_json::from_value(result(())).ok()?;
            let text = match r.kind {
                PackageKindDto::Vm => tf!("VM %@ imported", r.names.join("")),
                PackageKindDto::Image => tf!("Image %@ imported", r.names.join("")),
                PackageKindDto::Recipe if r.names.is_empty() => {
                    Text::new("These recipes are here already, unchanged – nothing to import")
                }
                PackageKindDto::Recipe => {
                    let names: Vec<String> = r.names.iter().map(|n| RecipeId::qualified(n)).collect();
                    tf!("Recipes imported: %@", names.join(", "))
                }
            };
            (Level::Info, text, None)
        }
        (K::Export, S::Finished) => {
            let path = result(()).get("path").and_then(Value::as_str).unwrap_or_default().to_string();
            let shown = job.title.strip_prefix("Export ").unwrap_or(&job.title).to_string();
            (Level::Info, tf!("%@ exported to %@", shown, path), None)
        }
        (_, S::Failed) => (Level::Error, Text::verbatim(format!("{}: {error}", job.title)), None),
        _ => return None,
    })
}

/// A VM that ended on its own (shut down inside, crashed, stopped with the CLI)
pub fn vm_ended(name: &str) -> (Level, Text, Option<Notification>) {
    (Level::Info, tf!("%@ has ended", name), notification("VM ended", tf!("%@ is no longer running.", name)))
}

pub fn vm_ready(name: &str, via: Via) -> (Level, Text, Option<Notification>) {
    let body = match via {
        Via::Ssh => tf!("%@ is running and answers SSH.", name),
        Via::Agent => tf!("%@ is running; its omaflake agent reported in.", name),
    };
    (Level::Info, tf!("%@ is ready", name), notification("VM ready", body))
}

/// What an action that a user does says when it is done or refused (the app's `perform` and its callers). Methods
/// whose outcome a page shows by itself (lists, settings, the source form, update checks) say nothing.
pub fn call_done(method: &str, params: &Value, outcome: &Result<Value, RpcError>) -> Option<(Level, Text)> {
    let name = params.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
    let ok = match outcome {
        Ok(v) => v,
        Err(e) => {
            let text = match method {
                m if m.starts_with("vms.") && !name.is_empty() => Text::verbatim(format!("{name}: {}", e.message)),
                "images.build" => {
                    let recipe = params.get("recipe").and_then(Value::as_str).unwrap_or_default();
                    tf!("Build %@: %@", name_of(recipe), &e.message)
                }
                "macos.installBase" => tf!("macOS base: %@", &e.message),
                "packages.import" | "packages.export" => {
                    let what = params
                        .get("path")
                        .and_then(Value::as_str)
                        .map(|p| p.rsplit('/').next().unwrap_or(p).to_string());
                    Text::verbatim(format!("{}: {}", what.unwrap_or(name), e.message))
                }
                m if ACTIONS.contains(&m) => Text::verbatim(e.message.clone()),
                _ => return None,
            };
            return Some((Level::Error, text));
        }
    };
    let text = match method {
        "vms.stop" => {
            let result = ok.get("result").and_then(Value::as_str).unwrap_or_default();
            Text::verbatim(format!("{name}: {result}"))
        }
        "vms.reset" => tf!("%@ reset", &name),
        "vms.remove" => tf!("%@ deleted", &name),
        "vms.upgrade" => tf!("%@ now uses %@", &name, ok.get("image").and_then(Value::as_str).unwrap_or_default()),
        "vms.create" => match ok.get("sshPort").and_then(Value::as_i64) {
            Some(port) if ok.get("kind") == Some(&json!("nixos")) => tf!("%@ created (SSH port %lld)", &name, port),
            _ => tf!("%@ created", &name),
        },
        "vms.set" if ok.get("takesEffectAtNextStart") == Some(&json!(true)) => {
            tf!("%@: takes effect at the next start", &name)
        }
        "vms.forget" => tf!("%@ is no longer listed – its package stays where it is", &name),
        "vms.useExternal" => {
            let vm = ok.get("name").and_then(Value::as_str).unwrap_or_default();
            let path = params.get("path").and_then(Value::as_str).unwrap_or_default();
            let parent = path.rsplit_once('/').map(|(p, _)| p).unwrap_or(path);
            tf!("%@ runs from %@", vm, parent)
        }
        "images.remove" => tf!("Image %@ deleted", &name),
        "recipes.remove" => {
            let id = params.get("id").and_then(Value::as_str).unwrap_or_default();
            tf!("%@ removed", RecipeId::qualified(id))
        }
        "recipes.removeLine" => {
            let line = params.get("line").and_then(Value::as_str).unwrap_or_default();
            let shown = name_of(line.split('@').next().unwrap_or(line));
            let count = |k: &str| ok.get(k).and_then(Value::as_array).map(Vec::len).unwrap_or(0);
            match count("kept") {
                0 => tf!("%@: %lld versions removed", shown, count("removed")),
                kept => tf!("%@: %lld versions removed, %lld kept", shown, count("removed"), kept),
            }
        }
        _ => return None,
    };
    Some((Level::Info, text))
}

/// The methods a page's actions send whose refusal goes into the activity list (besides `vms.*`)
const ACTIONS: [&str; 17] = [
    "images.remove",
    "recipes.install",
    "recipes.installSuggested",
    "recipes.installUpdates",
    "recipes.remove",
    "recipes.removeLine",
    "sources.remove",
    "sources.setEnabled",
    "sources.move",
    "dataFolder.move",
    "doctor.setPath",
    "macos.downloadIpsw",
    "builder.installNix",
    "builder.setup",
    "builder.apply",
    "builder.cleanUp",
    "jobs.cancel",
];

/// Whether `call_done` may say something about this method (the server clones the params only then)
pub fn is_action(method: &str) -> bool {
    method.starts_with("vms.")
        && !matches!(
            method,
            "vms.list"
                | "vms.get"
                | "vms.ssh"
                | "vms.agent"
                | "vms.log"
                | "vms.wait"
                | "vms.share"
                | "vms.nextName"
                | "vms.limits"
        )
        || matches!(method, "images.build" | "macos.installBase" | "packages.import" | "packages.export")
        || ACTIONS.contains(&method)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calls_are_worded_as_the_app_did() {
        let p = json!({"name": "research"});
        assert_eq!(call_done("vms.remove", &p, &Ok(Value::Null)).unwrap().1.text, "research deleted");
        let created = json!({"name": "research", "kind": "nixos", "sshPort": 2201});
        assert_eq!(call_done("vms.create", &p, &Ok(created)).unwrap().1.text, "research created (SSH port 2201)");
        let e = RpcError::server(Some(crate::error::codes::VM_RUNNING), "running – stop it first");
        let (level, t) = call_done("vms.reset", &p, &Err(e.clone())).unwrap();
        assert_eq!(
            (level, t.text.as_str(), t.key.is_none()),
            (Level::Error, "research: running – stop it first", true)
        );
        assert!(call_done("vms.list", &Value::Null, &Ok(json!({}))).is_none());
        assert!(call_done("settings.set", &json!({}), &Err(e)).is_none());
        let removed = json!({"removed": ["a", "b"], "kept": ["c"]});
        let t = call_done("recipes.removeLine", &json!({"line": "omaflake.nixos.dev@src"}), &Ok(removed)).unwrap().1;
        assert_eq!(t.text, "dev: 2 versions removed, 1 kept");
        assert_eq!(t.key.as_deref(), Some("%@: %lld versions removed, %lld kept"));
        assert!(
            is_action("vms.start")
                && !is_action("vms.list")
                && is_action("builder.apply")
                && !is_action("settings.set")
        );
    }

    #[test]
    fn the_log_keeps_the_newest() {
        let mut log = ActivityLog::default();
        for i in 0..(MAX + 5) {
            log.push(Level::Info, Text::verbatim(i.to_string()), None);
        }
        let list = log.list();
        assert_eq!(list.len(), MAX);
        assert_eq!(list[0].text.text, (MAX + 4).to_string());
        assert!(list[0].id > list[1].id);
        log.clear();
        assert!(log.list().is_empty());
    }
}
