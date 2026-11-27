// SPDX-License-Identifier: AGPL-3.0-or-later
//! The recipe studio (`recipeStudio`, plan B147; before it the recipe editor of B141/B142): a recipe of your own put
//! together like a kit. The catalog – the modules here and those your sources offer – is searched (with suggestions
//! while typing) and narrowed by tags and capabilities as chips, five modules a page, each opening its details (plan
//! B148); a module is added to the recipe or taken out of it with one click. The recipe says
//! what is wrong while you build it – a module that needs another (with the one to add), two that do not go together,
//! overlapping keys – and shows its keys before it is saved. Look, key binding sets, what a VM needs at least and how
//! it runs are fields as before. Saved into My Recipes (`my_recipes::MyRecipes`). Opened empty (the workshop's tab,
//! "New Recipe …"), on one of yours ("Edit …") or on any other ("Copy to My Recipes …").
//!
//! Everything chosen is a form value of the page (`values`), so every UI keeps it as it keeps a form's: `modules` is
//! the list of references, set as a whole by the add and remove buttons (`ui.field` with the new list).

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::common::*;
use crate::text::Text;

#[derive(Debug, Default, Deserialize)]
pub struct StudioParams {
    /// The recipe it starts from: one of yours is edited, any other copied
    #[serde(default)]
    pub id: Option<String>,
    /// The form's values: name, platform, description, modules (references), search, tags (a list), page (from 0),
    /// look, keymaps (references, a later one wins), cpus, memoryGB, diskGB, display, network, share, ephemeral
    #[serde(default)]
    pub values: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeStudioPage {
    pub title: Text,
    /// The recipe of yours being edited (its base); None for a new one or a copy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub intro: Text,
    /// Name, platform, description
    pub about: Vec<Field>,
    pub catalog: StudioCatalog,
    pub recipe: StudioRecipe,
    /// The look
    pub dress: Vec<Field>,
    /// The key binding sets, as many as wanted, in order (plan B153)
    pub keymaps: StudioKeymaps,
    /// What a VM needs at least and how it runs
    pub vm: Vec<Field>,
    /// Section titles: about, dress, vm
    pub titles: StudioTitles,
    /// Cancel, Delete … (one of yours), Save
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioTitles {
    pub about: Text,
    pub dress: Text,
    pub vm: Text,
}

/// The recipe's key binding sets (plan B153): a list like its modules – one added from a menu, each taken out or moved;
/// a later one wins for the same action
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioKeymaps {
    pub title: Text,
    pub help: Text,
    pub rows: Vec<StudioKeymap>,
    /// The sets not in the recipe, each an action that adds it at the end
    pub add: Vec<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Text>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioKeymap {
    /// "omaflake/default"
    pub reference: String,
    pub title: String,
    /// "1 of 2 – wins over the ones above"
    pub subtitle: Text,
    pub remove: Action,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub up: Option<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub down: Option<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioCatalog {
    pub title: Text,
    /// "24 modules", "3 of 24 modules"
    pub count: Text,
    pub search: Field,
    /// While something is typed: the names that match best (a UI shows them under the field; Enter takes the first)
    pub suggest: Vec<StudioSuggestion>,
    /// Tags and capabilities as chips, the most common first; several on: modules with all of them
    pub tags: Vec<StudioTag>,
    /// One page of the modules that match – those in the recipe too, marked `inside`
    pub rows: Vec<StudioModule>,
    pub pages: StudioPages,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Text>,
    /// Clears the search and the chips
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset: Option<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioSuggestion {
    pub reference: String,
    pub title: String,
    /// What it provides ("terminal")
    pub subtitle: Text,
    /// In the recipe already
    pub inside: bool,
    /// Add it (a module in the recipe has none)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioTag {
    pub id: String,
    pub label: Text,
    /// Modules of the platform with it
    pub count: usize,
    pub on: bool,
    /// Turns it on or off (and goes back to the first page)
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioPages {
    /// From 0
    pub index: usize,
    pub count: usize,
    /// "6–10 of 23"
    pub text: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioRecipe {
    pub title: Text,
    pub rows: Vec<StudioModule>,
    /// What is wrong, with the module to add where one is missing
    pub hints: Vec<Hint>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Text>,
    /// Its keys before it is saved (the keys page on a draft)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keys: Option<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioModule {
    /// "omaflake/zellij"
    pub reference: String,
    pub title: String,
    pub description: String,
    /// "multiplexer · needs terminal · 96 keys · omaflake"
    pub subtitle: Text,
    pub badges: Vec<Badge>,
    pub tags: Vec<String>,
    /// In the recipe (the catalog marks it with a check instead of Add)
    pub inside: bool,
    /// What a UI shows when it is opened: what it is, provides, needs, works with, keys, version, source
    pub details: Vec<InfoLine>,
    /// Opened: its keys in the keys window (a module here with keys)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub detail_actions: Vec<Action>,
    /// Add to the recipe, or take out of it
    pub action: Action,
}
