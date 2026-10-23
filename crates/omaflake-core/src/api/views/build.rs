// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Build Image page (`build`): one recipe's build – start it (a macOS recipe asks the guest user's password), its
//! place in the queue, progress, outcome and log (the app's `BuildView`, `BuildProgressView`, `BuildOutcome`).

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::*;
use super::text::{tf, Text};
use super::vms::{kind_badge, recipe_name};
use super::Ctx;
use crate::api::types::{BuildResult, JobDto, JobProgress, JobState};
use crate::models::GuestKind;

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
    /// Build / Add to Queue (macOS: asks the password) – or Cancel / Remove from Queue while it waits or runs
    pub actions: Vec<Action>,
    /// What the Doctor reports against building (while no build runs)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blockers: Option<Blockers>,
    /// macOS without a base: the way to create it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_base: Option<Hint>,
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
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::json::iso8601::option")]
    pub started: Option<chrono::DateTime<chrono::Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::json::iso8601::option")]
    pub last_output: Option<chrono::DateTime<chrono::Utc>>,
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
}

pub fn progress(j: &JobDto) -> Progress {
    let p = j.progress.clone().unwrap_or(JobProgress {
        step: 0,
        total: 0,
        title: String::new(),
        done: 0,
        of: 0,
        fraction: 0.0,
    });
    Progress {
        step: Text::verbatim(format!("{}/{}", p.step, p.total)),
        title: if p.title.is_empty() { Text::verbatim("") } else { Text::new(&p.title) },
        count: (p.of > 0).then(|| Text::verbatim(format!("{}/{}", p.done.min(p.of), p.of))),
        fraction: p.fraction,
        started: j.started,
        last_output: j.last_output,
        quiet_warning: 120,
        quiet_help: Text::new(
            "Some steps print nothing for minutes (writing the disk image, copying); Cancel stops the build",
        ),
    }
}

fn state_of(j: &JobDto) -> BuildState {
    match j.state {
        JobState::Queued => BuildState::Queued,
        JobState::Checking => BuildState::Checking,
        JobState::Running => BuildState::Running,
        JobState::Finished => BuildState::Finished,
        JobState::Failed => BuildState::Failed,
        JobState::Cancelled => BuildState::Cancelled,
    }
}

pub(crate) fn page(ctx: &Ctx, p: &BuildParams) -> BuildPage {
    let r = ctx.recipes.recipes.iter().find(|r| r.id == p.recipe);
    let kind = r.map(|r| r.kind).unwrap_or(GuestKind::Nixos);
    let mac = kind == GuestKind::Macos;
    let job = ctx.latest_build(&p.recipe).cloned();
    let active = job.as_ref().is_some_and(|j| j.state.is_active());
    let blockers = if r.is_some() { ctx.build_blockers(kind) } else { vec![] };
    let has_base = ctx.doctor.as_ref().is_none_or(|d| d.has_mac_base);
    let mut badges = Vec::new();
    if r.is_some_and(|r| r.lab) {
        badges.push(Badge::new("lab", "lab", Text::new("Development recipe (developer mode)"), Tone::Warning));
    }

    let mut actions = Vec::new();
    match &job {
        Some(j) if active => {
            let label = if j.state == JobState::Queued { "Remove from Queue" } else { "Cancel" };
            actions.push(Action::call("cancel", Text::new(label), Some("cancel"), "jobs.cancel", json!({"id": j.id})));
        }
        _ => {
            let queue = ctx.builds().any(|b| b.state.is_active() && b.guest_kind == Some(kind));
            let mut build = Action::call(
                "build",
                Text::new(if queue { "Add to Queue" } else { "Build" }),
                Some("build"),
                "images.build",
                json!({"recipe": p.recipe}),
            );
            if mac {
                // sudo in the build VM, once; kept by the core only until the build uses it
                let user = ctx
                    .doctor
                    .as_ref()
                    .and_then(|d| d.mac_base_user.clone())
                    .or_else(|| r.and_then(|r| r.user.clone()))
                    .unwrap_or_else(|| ctx.host_user.clone());
                build = build.ask(Ask::new("password", AskKind::Password, tf!("Password of %@", user)));
            }
            build = build
                .enabled_if(r.is_some(), None)
                .enabled_if(r.is_none_or(|r| r.runs_here), r.map(|r| super::recipes::not_here(ctx, &r.systems)))
                .enabled_if(blockers.is_empty(), Some(tf!("Open the Doctor first: %@", blocker_titles(&blockers))))
                .enabled_if(
                    !mac || has_base,
                    Some(Text::new(
                        "No macOS base yet – it is needed for macOS images and is created from Apple's macOS once.",
                    )),
                )
                .primary();
            actions.push(build);
        }
    }

    let (status, progress_view, outcome) = match &job {
        Some(j) if j.state == JobState::Queued => (Some(Text::new("Waiting in the queue")), None, None),
        Some(j) if active => (None, Some(progress(j)), None),
        Some(j) => (None, None, outcome(ctx, j)),
        None => (None, None, None),
    };
    let log = job.as_ref().map(log_ref);
    BuildPage {
        title: Text::new("Build Image"),
        recipe: p.recipe.clone(),
        name: if p.recipe.is_empty() { "–".into() } else { recipe_name(ctx, &p.recipe) },
        kind: kind_badge(kind),
        badges,
        description: r.and_then(|r| r.description.clone()),
        state: job.as_ref().map(state_of).unwrap_or(BuildState::Idle),
        actions,
        blockers: if active { None } else { Blockers::of(&blockers) },
        mac_base: (mac && !has_base && !active).then(|| {
            Hint::new(
                "macos",
                Text::new("No macOS base yet – it is needed for macOS images and is created from Apple's macOS once."),
                Tone::Normal,
            )
            .action(Action::show(
                "macBase",
                Text::new("Create macOS Base …"),
                Some("macos"),
                "macBase",
                json!({}),
            ))
        }),
        status,
        progress: progress_view,
        outcome,
        newest: if job.is_none() { r.and_then(|r| r.image.as_ref()).map(|i| tf!("newest image: %@", i)) } else { None },
        log,
        job,
    }
}

/// A job's log as a page shows it: where to read it, the note on hidden lines once it ended, the whole file
pub fn log_ref(j: &JobDto) -> LogRef {
    LogRef {
        job: j.id.clone(),
        from: j.log_lines.saturating_sub(crate::api::jobs::MAX_LINES),
        lines: j.log_lines,
        note: (!j.state.is_active() && j.hidden_lines > 0).then(|| {
            tf!(
                "(%lld routine lines of the disk image tool hidden – full log: %@)",
                j.hidden_lines,
                j.log_file.clone().unwrap_or_default()
            )
        }),
        file: j.log_file.clone(),
        full: j.log_file.as_ref().map(|f| {
            Action::call("fullLog", Text::new("Full Log …"), Some("log"), "ui.open", json!({"path": f}))
                .help(Text::new("Every line, also the routine ones left out above"))
        }),
    }
}

fn outcome(ctx: &Ctx, j: &JobDto) -> Option<Outcome> {
    Some(match j.state {
        JobState::Finished => {
            let image = j.result.clone().and_then(|r| serde_json::from_value::<BuildResult>(r).ok()).map(|r| r.image);
            let known = image.as_ref().and_then(|name| ctx.images.iter().find(|i| &i.name == name));
            Outcome {
                state: BuildState::Finished,
                icon: "finished".into(),
                tone: Tone::Accent,
                title: Text::new("Image ready"),
                detail: image
                    .as_ref()
                    .map(|name| Text::verbatim(known.map(|i| i.label.clone()).unwrap_or_else(|| name.clone()))),
                action: known.map(|i| {
                    Action::show("newVm", Text::new("New VM …"), Some("new-vm"), "newVm", json!({"image": i.name}))
                        .primary()
                }),
            }
        }
        JobState::Failed => Outcome {
            state: BuildState::Failed,
            icon: "failed".into(),
            tone: Tone::Danger,
            title: Text::new("Build failed"),
            detail: j.error.as_ref().map(Text::verbatim),
            action: None,
        },
        JobState::Cancelled => Outcome {
            state: BuildState::Cancelled,
            icon: "cancelled".into(),
            tone: Tone::Normal,
            title: Text::new("Build cancelled"),
            detail: None,
            action: None,
        },
        _ => return None,
    })
}
