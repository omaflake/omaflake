// SPDX-License-Identifier: AGPL-3.0-or-later
//! Settings (`settings`): the recipe sources (spec §11.3, plan B107), the general switches, the guest values
//! (spec §11.1) and the data folder – the app's `SettingsView` and `RecipeSourcesSection`. What only one UI has (launch
//! at login, the UI's language) stays in that UI.

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::text::Text;
use crate::types::SourceDto;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPage {
    /// "Settings"
    pub title: Text,
    pub general: SettingsSection,
    pub guest: SettingsSection,
    pub data_folder: DataFolderSection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSection {
    pub title: Text,
    pub fields: Vec<Field>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<Text>,
}

/// `sources` (plan B147): the recipe sources – a tab of the catalog, no longer part of the settings
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcesPage {
    pub title: Text,
    pub sources: SourcesSection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcesSection {
    pub title: Text,
    /// In their order: an earlier source wins when two have the same recipe
    pub rows: Vec<SourceRow>,
    /// A new source: its fields and "Check and Add"
    pub add: SettingsSection,
    pub add_action: Action,
    pub footer: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRow {
    pub id: String,
    pub name: String,
    /// folder, github, url
    pub icon: String,
    /// Official, private (with or without its token)
    pub badges: Vec<Badge>,
    pub subtitle: Text,
    /// Earlier, the on/off switch, remove
    pub actions: Vec<Action>,
    pub source: SourceDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataFolderSection {
    pub title: Text,
    pub location: String,
    pub is_default: bool,
    pub moving: bool,
    /// Default (~/.omaflake), Move …
    pub actions: Vec<Action>,
    pub note: Text,
}

/// The new source's values as typed so far (never a token)
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsParams {
    #[serde(default)]
    pub values: SourceValues,
}

#[derive(Debug, Default, Deserialize)]
pub struct SourceValues {
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub private: bool,
}
