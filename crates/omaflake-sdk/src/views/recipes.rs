// SPDX-License-Identifier: AGPL-3.0-or-later
//! The recipe tabs (plan B147): "Build Images" in the workshop (`recipes`) – one row per recipe line here with its
//! versions (plan B111), newer versions in the sources, the suggestion at the first start –, and "Recipes" in the
//! catalog (`recipeCatalog`) – what the remote sources offer (plan B107). Both in the shape of one page (the app's
//! `RecipeRowView`, `RecipeVersionRow`, `OfferedRecipeRow` and `RecipeUpdates`). Lab recipes only in developer mode.

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::text::Text;
use crate::types::{CatalogRecipeDto, RecipeDto, VersionDto};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipesPage {
    #[serde(flatten)]
    pub head: ListHead,
    /// The remote catalogs were looked into this session (else the UI runs the `refresh` action when the tab opens)
    pub catalogs_loaded: bool,
    /// They are being looked into now
    pub loading: bool,
    /// What a UI says while `loading` ("Looking into the recipe sources …", next to a spinner)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loading_text: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<UpdatesSection>,
    /// The first start: omaflake ships no recipes – the official ones are suggested
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<Hint>,
    pub rows: Vec<RecipeRowView>,
    /// The heading over `offered` (while there are offers)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offered_title: Option<Text>,
    /// Not installed, offered by a remote source
    pub offered: Vec<OfferedRow>,
    pub unreadable: Vec<Hint>,
    /// Sources that could not be read
    pub errors: Vec<Hint>,
    /// Lab recipes are left out (developer mode is off)
    pub lab_hidden: bool,
    /// Said under the list while `labHidden`, with the way to Settings
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lab_hint: Option<Hint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeRowView {
    pub id: String,
    pub line: String,
    pub name: String,
    pub lamp: Lamp,
    pub kind: Badge,
    /// Its author when not omaflake, its distinction (plan B110) – after the name
    pub secondary: Vec<String>,
    /// Lab, other architectures
    pub badges: Vec<Badge>,
    pub tags: Vec<String>,
    /// Show in the file manager; build (opens the build page) or the Doctor's warning
    pub actions: Vec<Action>,
    pub detail: RecipeDetail,
    pub recipe: RecipeDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeDetail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub info: Vec<InfoLine>,
    /// What is missing (with "Install Missing Parts"), what is wrong, unfree software (plan B142)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Hint>,
    pub versions: Vec<VersionRow>,
    /// Update (when its source has a newer version) on the left, remove all versions on the right
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionRow {
    pub id: String,
    /// As many words as tell the versions apart; the whole id is `id`
    pub words: String,
    /// "as it is in the folder" / added · newest · images: 2
    pub state: Text,
    pub actions: Vec<Action>,
    pub version: VersionDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferedRow {
    pub id: String,
    pub name: String,
    pub author: String,
    pub lamp: Lamp,
    pub kind: Badge,
    /// Official, lab, other architectures
    pub badges: Vec<Badge>,
    pub tags: Vec<String>,
    pub actions: Vec<Action>,
    pub detail: OfferedDetail,
    pub recipe: CatalogRecipeDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferedDetail {
    pub description: String,
    pub info: Vec<InfoLine>,
    /// "Not reviewed by the omaflake team …" for recipes not from omaflake/recipes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<Text>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatesSection {
    pub title: Text,
    /// "Update All" / "Add All" when there are several
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<Action>,
    pub rows: Vec<UpdateRow>,
    pub note: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRow {
    pub id: String,
    pub name: String,
    pub kind: Badge,
    pub source: String,
    pub action: Action,
}

#[derive(Debug, Default, Deserialize)]
pub struct RecipesParams {
    #[serde(default)]
    pub filter: FilterParams,
}
