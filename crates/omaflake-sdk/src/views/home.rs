// SPDX-License-Identifier: AGPL-3.0-or-later
//! The main page's frame (`home`): the header's buttons and the tabs with what needs attention there (the app's
//! `MainView.header` and `HomeTabBar`). The tabs come in four groups (plan B147): Run – VMs, images, the activity –,
//! the Workshop where things are built – images from recipes, recipes, key bindings –, the Catalog – recipes, modules,
//! looks and key bindings from the sources, and the sources –, and Profiles – credentials, locales, performance. Which tab is open and whether the activity was seen are the UI's own.

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::text::Text;

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
