// SPDX-License-Identifier: AGPL-3.0-or-later
//! The VMs tab (`vms`) and one VM (`vm`): what the app's `InstanceRow` and `InstanceDetail` showed.

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::text::Text;
use crate::types::Vm;

/// `vms`: the list of VMs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmsPage {
    #[serde(flatten)]
    pub head: ListHead,
    pub rows: Vec<VmRowView>,
    /// VM files this omaflake cannot read (plan B114)
    pub unreadable: Vec<Hint>,
}

/// One VM: its row and, below it, its details
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmRowView {
    pub name: String,
    pub lamp: Lamp,
    /// The guest kind's symbol
    pub kind: Badge,
    /// What it has switched on, at a glance (off = no badge), the external drive, a newer image
    pub badges: Vec<Badge>,
    /// "dev · 4 CPU · 8 GB · SSH 2201"
    pub subtitle: Text,
    /// The row's buttons, left to right (share folder, start headless, start in window – or terminal, stop)
    pub actions: Vec<Action>,
    pub detail: VmDetail,
    /// The VM as `vms.get` gives it
    pub vm: Vm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmDetail {
    /// Clipboard, sound, microphone, camera as switches (`vms.set`)
    pub switches: Vec<Action>,
    /// "Changes take effect at the next start." while it runs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switches_note: Option<Text>,
    /// CPU cores and memory (plan B164): number fields whose `set` is `vms.set {name}` with the value at `cpus` /
    /// `memoryGb`, enabled while the VM is stopped
    pub fields: Vec<Field>,
    pub info: Vec<InfoLine>,
    /// Export on the left; stop listing, upgrade, reset, remove on the right
    pub actions: Vec<Action>,
}

/// `vm`: one VM (a TUI's detail screen); `params: {name}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmPage {
    /// The VM's name
    pub title: Text,
    pub row: VmRowView,
}

#[derive(Debug, Default, Deserialize)]
pub struct VmsParams {
    #[serde(default)]
    pub filter: FilterParams,
}

#[derive(Debug, Deserialize)]
pub struct VmParams {
    pub name: String,
}
