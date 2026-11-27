// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Profiles page (`profiles`, plan B142) and the form of one (`profile`): how big a new VM is – CPU cores and RAM.
//! Same shapes as the Credentials page and form, so every UI draws them as it draws those. A new VM is created with
//! the default profile unless the New VM page chooses another (or custom values).

use serde::Deserialize;

/// The form's values (kept by the UI while the user chooses)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProfileValues {
    pub name: Option<String>,
    pub cpus: Option<u32>,
    #[serde(rename = "memoryGB")]
    pub memory_gb: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ProfileParams {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub values: ProfileValues,
}
