// SPDX-License-Identifier: AGPL-3.0-or-later
//! The New VM page (`newVm`): a VM from exactly one image (plan B100) – its recipe's defaults, the next free name, the
//! choices a guest kind allows, what is wrong with the values, and the create action (the app's `NewVMView`).

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::models::NetworkMode;
use crate::text::Text;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewVmPage {
    /// "New VM"
    pub title: Text,
    pub image: String,
    pub recipe: String,
    pub kind: Badge,
    /// "recipe dev"
    pub subtitle: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub lab: bool,
    /// What the Doctor reports against starting VMs of this kind
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blockers: Option<Blockers>,
    /// In the app's sections: the name and ephemeral, resources and devices, the share folder, start afterwards
    pub sections: Vec<FormSection>,
    /// Said under the form (the macOS limit: created, not started)
    pub warnings: Vec<Text>,
    /// Every value is fine: `create` is enabled
    pub valid: bool,
    /// Cancel, Create
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormSection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
    pub fields: Vec<Field>,
}

/// The values a form has now (the UI keeps them while the user edits); missing ones take the defaults
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewVmValues {
    pub name: Option<String>,
    /// A profile's id; "" = custom values (plan B142)
    pub profile: Option<String>,
    pub cpus: Option<u32>,
    #[serde(rename = "memoryGB")]
    pub memory_gb: Option<u32>,
    #[serde(rename = "diskGB")]
    pub disk_gb: Option<u32>,
    pub network: Option<NetworkMode>,
    pub share: Option<bool>,
    pub share_path: Option<String>,
    /// Plan B187: "" none, "+" a new volume, else an existing volume's name
    pub volume: Option<String>,
    /// A new volume's name
    pub volume_name: Option<String>,
    /// The name in the VM (`/Volumes/<name>`); empty: the folder's name
    pub mount_as: Option<String>,
    pub ephemeral: Option<bool>,
    pub audio: Option<bool>,
    pub microphone: Option<bool>,
    pub clipboard: Option<bool>,
    pub camera: Option<bool>,
    pub midi: Option<bool>,
    /// NixOS guests (plan B171)
    pub displays: Option<u32>,
    pub start: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct NewVmParams {
    pub image: String,
    #[serde(default)]
    pub values: NewVmValues,
}
