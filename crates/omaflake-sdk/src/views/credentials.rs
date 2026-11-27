// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Credentials page (`credentials`, plan B137): who you are in a guest – named sets of user, full name, e-mail and
//! an optional stored password (for macOS builds); an image is built with one of them, the settings name the default.
//! The form of one (`credential`): a new one, or one to edit – only its name while images use it. The same form
//! assigns a credential to a macOS base created before bases kept theirs (`baseCredential`, plan B139).

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::text::Text;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsPage {
    /// "Credentials"
    pub title: Text,
    /// What a credential is, and what an image gets without one (this computer's user and name)
    pub intro: Text,
    pub rows: Vec<CredentialRow>,
    /// "New Credential …" (asks name, user, full name, e-mail) – primary
    pub add: Action,
    /// Where passwords are kept
    pub footer: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialRow {
    pub id: String,
    pub name: String,
    /// On while images use it
    pub lamp: Lamp,
    /// "alex · 2 images"
    pub subtitle: Text,
    /// default, no password
    pub badges: Vec<Badge>,
    /// The images built with it
    pub used_by: Vec<String>,
    /// The quick action: Copy Password
    pub actions: Vec<Action>,
    /// Opened: its values with what each is for, the images using it; Edit …, Set/Change Password …, Make Default,
    /// Remove … (only while no image uses it)
    pub detail: CredentialDetail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialDetail {
    pub info: Vec<InfoLine>,
    /// What belongs to it, one line each with its own actions (a network's VMs, plan B185); before `add`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub items: Vec<DetailItem>,
    /// A button that opens a list to choose one from, and does that one's action (a network's "Add VM", plan B185)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub add: Option<ActionMenu>,
    pub actions: Vec<Action>,
}

impl CredentialDetail {
    pub fn new(info: Vec<InfoLine>, actions: Vec<Action>) -> CredentialDetail {
        CredentialDetail { info, items: vec![], add: None, actions }
    }
}

/// One thing that belongs to a row, as a line in its detail: lamp, symbol, name, what is known, its actions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetailItem {
    pub id: String,
    pub name: String,
    pub icon: String,
    pub lamp: Lamp,
    /// "10.231.1.10 · connected – 12 KB in, 3 KB out"
    pub subtitle: Text,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub badges: Vec<Badge>,
    /// Drawn as symbol buttons at the line's end
    pub actions: Vec<Action>,
}

/// A button with a list: a click opens the list, a choice does its action. Empty: `empty` says why there is nothing
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ActionMenu {
    pub label: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub actions: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Text>,
}

/// The form of one credential (`credential`): new (no `id`) or to edit
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialPage {
    /// "New Credential" / "Edit Credential"
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// What a credential is for
    pub intro: Text,
    /// name, user, full name, e-mail – each with what it is for in NixOS and macOS guests
    pub fields: Vec<Field>,
    /// Why only the name can change (images use it)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<Text>,
    /// Cancel, Save / Create
    pub actions: Vec<Action>,
}

/// The form's values (the UI keeps them while the user types); missing ones are the credential's (or the defaults)
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialValues {
    pub name: Option<String>,
    pub user: Option<String>,
    pub full_name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CredentialParams {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub values: CredentialValues,
}

#[derive(Debug, Default, Deserialize)]
pub struct BaseCredentialParams {
    pub name: String,
    #[serde(default)]
    pub values: BaseCredentialValues,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BaseCredentialValues {
    pub credential: Option<String>,
}
