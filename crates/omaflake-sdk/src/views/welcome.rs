// SPDX-License-Identifier: AGPL-3.0-or-later
//! `welcome`: the first start's few questions (plan B144) – what to run, the recipes and looks to start with, who you
//! are in a guest, developer mode – and one button that does it all (`setup.welcome`): your credential, then the jobs
//! for what you chose (Set Up for NixOS, the suggested recipes and looks, the macOS base). The page has a credential
//! form's shape, so every interface draws it as one; the app opens it at its first start, the Doctor offers it.

use serde::Deserialize;

/// The form's values (kept by the UI while the user answers)
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WelcomeValues {
    pub guests: Option<String>,
    pub recommended: Option<bool>,
    pub user: Option<String>,
    pub full_name: Option<String>,
    pub lab: Option<bool>,
}

#[derive(Debug, Default, Deserialize)]
pub struct WelcomeParams {
    #[serde(default)]
    pub values: WelcomeValues,
}
