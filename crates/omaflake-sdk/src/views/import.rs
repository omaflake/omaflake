// SPDX-License-Identifier: AGPL-3.0-or-later
//! The import step (`import`, plans B108/B109): what a chosen package is and the one action that brings it in – with
//! the question when something of it is here already (a VM under another name, the same image) or it lies on a
//! network volume (a VM run from where it is). A UI shows no page for it: it runs `action` (asking `confirm`) or shows
//! `reason` (the app's `importPackage` and `chooseExternal`).

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::text::Text;
use crate::types::{Inspection, PackageKindDto};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPage {
    /// "Import" (a UI shows no page for it – a dialog's title)
    pub title: Text,
    pub path: String,
    /// The package's file name
    pub name: String,
    pub kind: PackageKindDto,
    pub valid: bool,
    /// Why it cannot be imported (`"<name>: <reason>"`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    pub inspection: Inspection,
}

#[derive(Debug, Deserialize)]
pub struct ImportParams {
    pub path: String,
    #[serde(default)]
    pub expect: Option<PackageKindDto>,
    /// Run a VM from where it is instead of copying it
    #[serde(default)]
    pub external: bool,
}
