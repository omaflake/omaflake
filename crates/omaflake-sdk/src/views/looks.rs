// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Looks tab (`looks`, plan B141): the looks here – installed from a source or in a local folder – with the
//! default one, their colours and usage, and what the sources' catalogs offer besides. Nothing is built in: images are
//! built with the default look unless the build page chooses another.

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::text::Text;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LooksPage {
    #[serde(flatten)]
    pub head: ListHead,
    /// The catalogs were looked into this session (else the UI runs the head's `refresh` when the tab opens)
    pub catalogs_loaded: bool,
    /// They are being looked into now
    pub loading: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loading_text: Option<Text>,
    pub rows: Vec<LookRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offered_title: Option<Text>,
    /// Not here, offered by a source
    pub offered: Vec<LookRow>,
    pub unreadable: Vec<Hint>,
    pub footer: Text,
}

/// One look: its name, a strip of its colours, what it is (author, appearance, usage); opened: its description and
/// actions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookRow {
    pub id: String,
    pub name: String,
    /// On while images are built with it
    pub lamp: Lamp,
    /// "omaflake/catppuccin · dark · 2 images"
    pub subtitle: Text,
    /// default, official, update, local folder
    pub badges: Vec<Badge>,
    /// Its colours as "#RRGGBB" for a strip: background, surface, text, accent, urgent, then the terminal's red,
    /// green, yellow, blue, magenta and cyan; empty for a plain look; none on a module's row (the modules page)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swatches: Option<Vec<String>>,
    /// The quick action: Install (offered), Make Default (here)
    pub actions: Vec<Action>,
    pub detail: LookDetail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookDetail {
    pub info: Vec<InfoLine>,
    pub actions: Vec<Action>,
}
