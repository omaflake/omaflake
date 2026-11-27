// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Images tab (`images`) and one image (`image`): every built image (plan B93) and the builds that wait, run or
//! failed on top – the app's `ImageRowView`, `ImageVersionInfo` and `BuildJobRow`.

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::text::Text;
use crate::types::{ImageDto, JobDto, JobState};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagesPage {
    #[serde(flatten)]
    pub head: ListHead,
    /// Builds that wait, run, failed or were cancelled (finished ones are images now)
    pub jobs: Vec<BuildJobRow>,
    pub rows: Vec<ImageRowView>,
    pub unreadable: Vec<Hint>,
    /// The macOS bases (on a Mac only, plan B137): what macOS images are built from
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bases: Option<BasesSection>,
}

/// "macOS Bases": one row per base, a hint and the way to create one when there is none
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BasesSection {
    pub title: Text,
    pub rows: Vec<BaseRow>,
    /// No base yet: what it is for, with "Create macOS Base …"
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Hint>,
    /// The base being created (its IPSW downloaded, then installed): state, progress; a click opens its page
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub job: Option<BuildJobRow>,
    /// "Create macOS Base …" (another one, from a newer macOS); clients show it only without `empty` (whose action
    /// it is then)
    pub create: Action,
    pub footer: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BaseRow {
    pub name: String,
    pub lamp: Lamp,
    /// "macOS 26A434 · user alex · 2026-10-17 · 42 GB"
    pub subtitle: Text,
    /// newest (used for new images), incomplete
    pub badges: Vec<Badge>,
    /// Show in Finder, Remove …
    pub actions: Vec<Action>,
}

/// A build in a list: its state in words, its progress; a click opens the build page (`open`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildJobRow {
    pub id: String,
    /// The recipe's name ("dev (velvet)")
    pub name: String,
    pub kind: Badge,
    /// What the row's symbol says: queued (clock), working (spinner), failed, finished, cancelled
    pub state: JobState,
    pub title: Text,
    /// Packages done of announced ("120/400"), while running
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<Text>,
    /// 0…1 while running
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fraction: Option<f64>,
    /// Cancel / remove from the queue, or hide an ended one
    pub actions: Vec<Action>,
    /// The build page (the row's primary action: a click on the row)
    pub open: Action,
    pub job: JobDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageRowView {
    pub name: String,
    /// Green: a VM uses it (plan B100)
    pub lamp: Lamp,
    pub kind: Badge,
    /// "outdated"
    pub badges: Vec<Badge>,
    /// `"recipe dev · <created> · 2 VMs"`
    pub subtitle: Text,
    /// The VMs using it (the subtitle's tooltip)
    pub used_by: Vec<String>,
    /// Show in the file manager; New VM from it (or the Doctor's warning)
    pub actions: Vec<Action>,
    pub detail: ImageDetail,
    pub image: ImageDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageDetail {
    pub info: Vec<InfoLine>,
    /// Export on the left, remove on the right
    pub actions: Vec<Action>,
}

/// `image`: one image with its size; `params: {name}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePage {
    /// The image's name
    pub title: Text,
    pub row: ImageRowView,
}

#[derive(Debug, Default, Deserialize)]
pub struct ImagesParams {
    #[serde(default)]
    pub filter: FilterParams,
}

#[derive(Debug, Deserialize)]
pub struct ImageParams {
    pub name: String,
}
