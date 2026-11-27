// SPDX-License-Identifier: AGPL-3.0-or-later
//! The activity list (`activity`): what happened in this server – actions done or refused, jobs that ended, VMs that
//! ended or are ready, the core's own messages – worded once here for every UI (the app's `AppModel.message`,
//! `finished(_:)`). Newest first, at most `MAX` entries. Whether the user has seen them is the UI's business: by `id`.

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::text::Text;
use crate::time::Date;
use crate::types::*;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityEntry {
    /// Stable and rising, never reused while the server runs (`clear` included): a UI marks entries seen by
    /// remembering the highest id it has shown – higher ones are unseen (api.md §10.6)
    pub id: u64,
    #[serde(with = "crate::time::iso8601")]
    pub time: Date,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActivityPage {
    #[serde(flatten)]
    pub head: ListHead,
    pub entries: Vec<ActivityEntry>,
    /// The user wants system notifications (Settings): a UI posts an entry's `notification` only then
    pub notifications: bool,
}
