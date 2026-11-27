// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Keys page (`keys`, plan B147): every key of a recipe – or of the recipe an image or a VM was built from –
//! grouped by module and, within a module, by the mode of its program. A mode shows the way there from the mode its
//! program starts in (Zellij: Ctrl+g › p › Pane); a key says the mode it leads to ("then Locked"), what it does in
//! the program's own words and which key binding set gave it. The UIs show it in a window or page of its own and
//! filter it by what is typed.

use serde::{Deserialize, Serialize};

use super::common::*;
use crate::text::Text;

/// Whose keys: a recipe by id, an image or a VM by name (through the recipe it was built from), a recipe being put
/// together in the recipe studio (`draft`), or one module here by its reference ("omaflake/zellij", with `platform`)
#[derive(Debug, Default, Deserialize)]
pub struct KeysParams {
    pub recipe: Option<String>,
    pub image: Option<String>,
    pub vm: Option<String>,
    pub draft: Option<Draft>,
    pub module: Option<String>,
    pub platform: Option<String>,
}

/// A recipe not saved yet: its platform, modules and key binding sets (`"<author>/<name>"`)
#[derive(Debug, Default, Deserialize)]
pub struct Draft {
    #[serde(default)]
    pub name: String,
    pub platform: String,
    #[serde(default)]
    pub modules: Vec<String>,
    #[serde(default)]
    pub keymaps: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeysPage {
    pub title: Text,
    /// The recipe ("neovim-minimal"), and the image or VM when asked for one
    pub subtitle: Text,
    pub intro: Text,
    pub notes: Vec<Hint>,
    pub modules: Vec<KeyModule>,
    /// The key binding sets in the order the recipe names them
    pub keymaps: Vec<String>,
    /// What the search field says while empty
    pub search: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyModule {
    /// "omaflake/zellij"
    pub reference: String,
    pub title: String,
    pub groups: Vec<KeyGroup>,
}

/// The keys of one mode of a module's program, or its keys without a mode
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyGroup {
    /// The mode's id; None: keys without a mode
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    pub title: Text,
    /// Its program starts in it
    pub start: bool,
    /// The way there from the start mode: each step's keys and the mode it reaches; empty for the start mode and for
    /// keys without a mode
    pub way: Vec<KeyStep>,
    pub rows: Vec<KeyRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyStep {
    pub caps: Vec<Vec<String>>,
    pub mode: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyRow {
    pub action: String,
    /// The keys as written ("Ctrl+g", "Space f f") – for the search
    pub keys: Vec<String>,
    /// Each key as key caps: its chords, each a list of caps ("Cmd", "Shift", "Enter")
    pub caps: Vec<Vec<Vec<String>>>,
    pub title: Text,
    /// "then Locked"; None: it stays in the mode
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Text>,
    /// It leads back to the mode its program starts in
    pub back_to_start: bool,
    /// Only switches the mode (it does nothing else)
    pub switches: bool,
    /// What it does in the program's own words (Zellij's `NewPane "Down"`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub does: Option<String>,
    /// The key binding set that gave the key or the mode afterwards; None: the module's default
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
    /// The program's own key, shown for reference
    pub own: bool,
}
