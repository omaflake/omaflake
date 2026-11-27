// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Volumes tab (`volumes`, plan B187; German "Platten") – folders of this computer that VMs share – and its two
//! forms: a new volume (`volume`) and the name a VM sees one under (`volumeMount`). The list has the Credentials
//! page's shape, so every UI draws it as it draws that: each volume with its folder, its VMs as lines of their own
//! (the name each sees it under, "Rename …", "Take Out") and "Add VM" right there, as in the Networks tab.

use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize)]
pub struct VolumeValues {
    pub name: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct VolumeParams {
    #[serde(default)]
    pub values: VolumeValues,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MountValues {
    #[serde(rename = "as")]
    pub mount: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MountParams {
    pub volume: String,
    pub vm: String,
    #[serde(default)]
    pub values: MountValues,
}
