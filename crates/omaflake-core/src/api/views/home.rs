// SPDX-License-Identifier: AGPL-3.0-or-later
//! The main page's frame (`home`): the header's buttons and the tabs with what needs attention there (the app's
//! `MainView.header` and `HomeTabBar`). The tabs come in four groups (plan B147): Run – VMs, images, the activity –,
//! the Workshop where things are built – images from recipes, recipes, key bindings –, the Catalog – recipes, modules,
//! looks and key bindings from the sources, and the sources –, and Profiles – credentials, locales, performance. Which tab is open and whether the activity was seen are the UI's own.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Ctx;
use super::common::*;
use super::text::{Text, tf};
use crate::api::types::JobState;
use crate::models::VmStatus;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HomePage {
    /// "omaflake" – the window's title
    pub title: Text,
    /// Doctor (with its issue count), Settings, the data folder, Quit
    pub actions: Vec<Action>,
    /// Every tab, the groups' in order (each names its `group`)
    pub tabs: Vec<Tab>,
    /// The tab groups: `run`, `workshop`
    pub groups: Vec<TabGroup>,
    /// No VM yet: a UI opening for the first time starts on the recipes tab
    pub first_tab: String,
    /// The help switch (plan B148, `on`: shown): explanations – a page's `intro` and `footer`, a field's `help`, hints
    /// marked `help` – are drawn only while it is on, each in a box of its own. Off by default.
    pub help: Action,
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
    /// The group it is in: `run` or `workshop`
    pub group: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TabGroup {
    pub id: String,
    pub title: Text,
    pub icon: String,
    /// A tab of the group needs attention (its badge, other than a count)
    pub attention: bool,
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
    let tab = |id: &str, label: &str, icon: &str, group: &str, badge: Option<TabBadge>| Tab {
        id: id.into(),
        label: Text::new(label),
        icon: icon.into(),
        badge,
        group: group.into(),
    };
    let warning = |help: &str| TabBadge { kind: "dot".into(), count: None, tone: Tone::Warning, help: Text::new(help) };
    let tabs = vec![
        tab(
            "vms",
            "VMs",
            "vm",
            "run",
            (!running.is_empty()).then(|| TabBadge {
                kind: "count".into(),
                count: Some(running.len()),
                tone: Tone::Success,
                help: Text::new("Running"),
            }),
        ),
        tab("images", "Images", "image", "run", images_badge),
        tab("activity", "Activity", "activity", "run", None),
        tab(
            "recipes",
            "Build Images",
            "build",
            "workshop",
            outdated.then(|| warning("A recipe changed since its image was built")),
        ),
        tab("recipeStudio", "Recipe Studio", "studio", "workshop", None),
        tab("keymapStudio", "Key Binding Studio", "keyboard", "workshop", None),
        tab("lookStudio", "Look Studio", "look", "workshop", None),
        tab("recipeCatalog", "Recipes", "recipe", "catalog", None),
        tab("modules", "Modules", "module", "catalog", None),
        tab(
            "looks",
            "Looks",
            "look",
            "catalog",
            ctx.looks.looks.is_empty().then(|| warning("No look installed – images are built with one")),
        ),
        tab("keymaps", "Key Bindings", "keyboard", "catalog", None),
        tab("sources", "Sources", "source", "catalog", None),
        tab(
            "credentials",
            "Credentials",
            "key",
            "profiles",
            ctx.credentials.is_empty().then(|| warning("No credential yet – images are built with one")),
        ),
        tab("locales", "Locales", "globe", "profiles", None),
        tab("profiles", "Performance", "gauge", "profiles", None),
    ];
    let attention = |g: &str| tabs.iter().any(|t| t.group == g && t.badge.as_ref().is_some_and(|b| b.kind != "count"));
    let group = |id: &str, title: &str, icon: &str| TabGroup {
        id: id.into(),
        title: Text::new(title),
        icon: icon.into(),
        attention: attention(id),
    };
    let groups = vec![
        group("run", "Run", "vm"),
        group("workshop", "Workshop", "workshop"),
        group("catalog", "Catalog", "recipe"),
        group("profiles", "Profiles", "key"),
    ];
    HomePage {
        title: Text::verbatim("omaflake"),
        actions,
        tabs,
        groups,
        first_tab: if ctx.vms.is_empty() { "recipes".into() } else { "vms".into() },
        help: Action::call(
            "help",
            Text::new("Help"),
            Some("help"),
            "settings.set",
            json!({"help": !ctx.settings.help}),
        )
        .switch(ctx.settings.help)
        .help(Text::new(if ctx.settings.help {
            "Hide the explanations"
        } else {
            "Show explanations on every page"
        })),
    }
}
