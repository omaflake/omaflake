// SPDX-License-Identifier: AGPL-3.0-or-later
//! The page models (docs/api.md §10): what each page of omaflake shows – rows, details, actions, fields, texts. The
//! core builds them and serves them (`views.get`, `views.subscribe`); every interface only draws them.

use serde::Serialize;
use serde_json::{Value, json};

// The texts every page shows
pub use crate::text::{Arg, Text};

pub mod activity;
pub mod build;
pub mod common;
pub mod credentials;
pub mod diagnostics;
pub mod doctor;
pub mod home;
pub mod images;
pub mod import;
pub mod keymap_studio;
pub mod keys;
pub mod locales;
pub mod look_studio;
pub mod looks;
pub mod networks;
pub mod new_vm;
pub mod profiles;
pub mod recipe_studio;
pub mod recipes;
pub mod settings;
pub mod vms;
pub mod volumes;
pub mod welcome;

/// The computer the core runs on (`initialize`'s `host`): texts that name the Mac, Finder or the Keychain have a
/// host-neutral variant for the others
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Macos,
    Linux,
}

impl HostOs {
    pub fn this() -> HostOs {
        if cfg!(target_os = "macos") { HostOs::Macos } else { HostOs::Linux }
    }
}

/// A page and its params, as `views.get` and `views.subscribe` take them
#[derive(Debug, Clone, PartialEq, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRef {
    pub page: String,
    #[serde(default = "empty_object")]
    pub params: Value,
}

impl PageRef {
    /// The key a subscription is known by
    pub fn key(&self) -> String {
        format!("{} {}", self.page, self.params)
    }
}

fn empty_object() -> Value {
    json!({})
}
