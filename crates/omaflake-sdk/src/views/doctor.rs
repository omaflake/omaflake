// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Doctor (`doctor`, spec §11.2): versions and updates, the checks in three sections with their hints and what can
//! be done about each (the app's `DoctorView`, `RequirementRow`, `RequirementText`, `UpdateRow`); and the page that
//! creates the macOS base (`macBase`, the app's `MacBaseView`).

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::text::Text;
use crate::types::PrerequisiteDto;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorPage {
    /// The page's title ("Doctor")
    pub title: Text,
    /// One line "omaflake" with both versions: the UI's (as it said in `initialize`) and the core's – shown
    /// together with `update`, as one block
    pub versions: Vec<InfoLine>,
    pub update: UpdateLine,
    pub intro: Text,
    /// The checks run now
    pub checking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::time::iso8601::option")]
    pub checked: Option<crate::time::Date>,
    /// "Check Again"
    pub actions: Vec<Action>,
    pub sections: Vec<DoctorSection>,
    /// Items that need attention (the warning sign on the Doctor's button)
    pub issues: usize,
}

/// "Updates": the last `updates.check`, with Download and Check Now
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLine {
    pub label: Text,
    pub text: Text,
    pub tone: Tone,
    pub busy: bool,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSection {
    /// `requirements`, `nixos`, `macos`
    pub id: String,
    pub title: Text,
    pub items: Vec<DoctorItem>,
}

/// One check: status, title, what was found; a sub-line with the hint and the actions (open at first while there is a
/// problem or work runs)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorItem {
    pub item: crate::models::PrerequisiteItem,
    pub status: crate::models::PrerequisiteStatus,
    pub title: Text,
    /// Paths and versions as found (left out where the hint says it all)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Text>,
    /// What to do, while not ok
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<Text>,
    /// It has a sub-line (a hint or actions)
    pub expandable: bool,
    /// The sub-line starts open
    pub expanded: bool,
    /// Its work runs (a download, the base, a Linux builder task)
    pub busy: bool,
    /// The running Linux builder task's title
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<Text>,
    /// A download's bytes so far and in all
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<Download>,
    pub actions: Vec<Action>,
    pub prerequisite: PrerequisiteDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub done: i64,
    pub of: i64,
    /// "12.3 GB / 26.6 GB"
    pub text: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacBasePage {
    /// "Create macOS Base"
    pub title: Text,
    pub intro: Text,
    /// The base as the Doctor sees it: ready (success) or unfinished (warning)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Hint>,
    /// Every base, newest first, with Create Again and Remove (plan B166)
    pub bases: Vec<MacBaseRow>,
    /// "macOS from Apple (IPSW)": the file chosen or Apple's, the download – the IPSW's only place on the page (its
    /// actions set `values.ipsw` with `ui.field`)
    pub ipsw: IpswLine,
    /// The credential the guest user is created from (plan B139)
    pub fields: Vec<Field>,
    pub installing: bool,
    /// Create Base (no password asked: the credential's); "New Credential …" while there is none
    pub actions: Vec<Action>,
    /// The latest base creation's log (its job, as the build page's)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<super::build::LogRef>,
}

/// A base on the macBase page (plan B166)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacBaseRow {
    /// "mac-base-26A434"
    pub name: String,
    /// The macOS build it was installed from
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub build: Option<String>,
    /// The macOS version when it was known at creation (Apple's IPSW)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// The credential's name (its current one, else the one recorded); absent for a base without one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credential: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::time::iso8601::option")]
    pub created: Option<crate::time::Date>,
    /// It knows omaflake's own SSH key (plan B160)
    pub own_key: bool,
    /// Why to look at it: made before omaflake's own key, not installed completely, no credential
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<Hint>,
    /// Create Again … (`macos.installBase {credential, ipsw?, replace}`), Remove … (`macos.removeBase`)
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpswLine {
    pub label: Text,
    pub text: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<Download>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MacBaseParams {
    #[serde(default)]
    pub values: MacBaseValues,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacBaseValues {
    pub credential: Option<String>,
    pub ipsw: Option<String>,
}
