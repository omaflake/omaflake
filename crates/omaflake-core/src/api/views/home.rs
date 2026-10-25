// SPDX-License-Identifier: AGPL-3.0-or-later
//! The main page's frame (`home`): the header's buttons and the tabs with what needs attention there (the app's
//! `MainView.header` and `HomeTabBar`). Which tab is open and whether the activity was seen are the UI's own.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::*;
use super::text::{tf, Text};
use super::Ctx;
use crate::api::types::JobState;
use crate::models::VmStatus;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomePage {
    /// "omaflake" – the window's title
    pub title: Text,
    /// Doctor (with its issue count), Settings, the data folder, Quit
    pub actions: Vec<Action>,
    pub tabs: Vec<Tab>,
    /// No VM yet: a UI opening for the first time starts on the recipes tab
    pub first_tab: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Tab {
    /// `vms`, `images`, `recipes`, `activity` – also the page it shows
    pub id: String,
    pub label: Text,
    pub icon: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<TabBadge>,
}

/// What a tab shows next to its name: a count (running VMs), a spinner (a build runs), a symbol (a build failed) or a
/// dot (a recipe changed since its image)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabBadge {
    /// `count`, `working`, `failed`, `dot`
    pub kind: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<usize>,
    pub tone: Tone,
    pub help: Text,
}

pub(crate) fn page(ctx: &Ctx) -> HomePage {
    let running: Vec<&str> =
        ctx.vms.iter().filter(|v| v.status == VmStatus::Running).map(|v| v.name.as_str()).collect();
    let mut quit = Action::call("quit", Text::new("Quit omaflake"), Some("quit"), "ui.quit", json!({})).short("Quit");
    if !running.is_empty() {
        quit = quit.confirm(
            Confirm::new(
                Text::new("Quit omaflake?"),
                tf!(
                    "These VMs keep running without the app: %@. You can stop them later with omaf or when you open omaflake again.",
                    running.join(", ")
                ),
            )
            .button("Quit"),
        );
    }
    let issues = ctx.doctor.as_ref().map(|d| d.issues).unwrap_or(0);
    let mut doctor =
        Action::show("doctor", Text::new("Doctor – requirements and preflight"), Some("doctor"), "doctor", json!({}))
            .short("Doctor");
    doctor.badge = (issues > 0).then_some(issues);
    let actions = vec![
        doctor,
        Action::show("credentials", Text::new("Credentials"), Some("key"), "credentials", json!({})),
        Action::show("settings", Text::new("Settings"), Some("settings"), "settings", json!({})),
        Action::call(
            "dataFolder",
            Text::new(ctx.on_host("Show Data Folder in Finder", "Show Data Folder")),
            Some("folder"),
            "ui.open",
            json!({"path": ctx.settings.data_folder}),
        )
        .short("Data Folder"),
        quit,
    ];
    let builds: Vec<_> = ctx.builds().collect();
    let images_badge = if builds.iter().any(|b| b.state.is_active()) {
        Some(TabBadge {
            kind: "working".into(),
            count: None,
            tone: Tone::Accent,
            help: Text::new("A build is running"),
        })
    } else if builds.iter().any(|b| b.state == JobState::Failed) {
        Some(TabBadge { kind: "failed".into(), count: None, tone: Tone::Danger, help: Text::new("A build failed") })
    } else {
        None
    };
    let outdated = super::recipes::recipe_rows(&ctx.recipes.recipes).iter().any(|r| r.image.is_some() && r.outdated);
    let tabs = vec![
        Tab {
            id: "vms".into(),
            label: Text::new("VMs"),
            icon: "vm".into(),
            badge: (!running.is_empty()).then(|| TabBadge {
                kind: "count".into(),
                count: Some(running.len()),
                tone: Tone::Success,
                help: Text::new("Running"),
            }),
        },
        Tab { id: "images".into(), label: Text::new("Images"), icon: "image".into(), badge: images_badge },
        Tab {
            id: "recipes".into(),
            label: Text::new("Recipes"),
            icon: "recipe".into(),
            badge: outdated.then(|| TabBadge {
                kind: "dot".into(),
                count: None,
                tone: Tone::Warning,
                help: Text::new("A recipe changed since its image was built"),
            }),
        },
        Tab { id: "activity".into(), label: Text::new("Activity"), icon: "activity".into(), badge: None },
    ];
    HomePage {
        title: Text::verbatim("omaflake"),
        actions,
        tabs,
        first_tab: if ctx.vms.is_empty() { "recipes".into() } else { "vms".into() },
    }
}
