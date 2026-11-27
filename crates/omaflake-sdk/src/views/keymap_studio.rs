// SPDX-License-Identifier: AGPL-3.0-or-later
//! The key binding studio (`keymapStudio`, plan B147, B148): a key binding set of your own. It starts from a set here
//! (a copy) or from the modules' own keys. The modules it covers are chips (one more is added from the platform's
//! modules with keys); each is a card that opens, its keys by the mode of its program as the keys page shows them –
//! caps, what a key does, where it leads afterwards – read only. A key is changed in the key editor (`editor`, a sheet
//! of its own): its keys – recorded by the UI or put together from modifiers and a key, never typed as "Ctrl" –, the
//! mode afterwards, what it overlaps while you choose, the module's own key to go back to. Saved into My Recipes'
//! folder (`keymaps/my/<name>/`); only what differs from the modules' own keys is written. The program's own keys
//! (`own`) are shown, not changed.
//!
//! Everything chosen is a form value of the page: name, title, description, platform, base (the set it starts from),
//! modules (the references it covers), search, per action `key:<module>:<action>` (a list of keys, or "h / Left") and
//! `after:<module>:<action>` (a mode); while the editor is open `edit` (`"<module>:<action>"`), `draft` (its keys),
//! `draftText` (the keys typed, "Ctrl+g / Left" – for a terminal) and `draftAfter`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::common::*;
use crate::text::Text;

#[derive(Debug, Default, Deserialize)]
pub struct KeymapStudioParams {
    /// The set it starts from ("my.keymap.vim" is edited, any other copied)
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub values: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeymapStudioPage {
    pub title: Text,
    /// The set of yours being edited ("vim"); None for a new one or a copy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub intro: Text,
    /// Name, title, description, platform, the set it starts from
    pub about: Vec<Field>,
    /// The platform's modules with keys: those the set covers (`on`, a click takes one out) and those to add
    pub modules: Vec<StudioModuleChip>,
    pub search: Field,
    /// What overlaps or breaks the key rule, across the platform's modules
    pub hints: Vec<Hint>,
    /// A card per module the set covers: its keys by mode
    pub cards: Vec<StudioCard>,
    /// "12 keys differ from the modules' own"
    pub changed: Text,
    /// The key editor, while a key is being changed (`values.edit`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub editor: Option<KeyEditor>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioModuleChip {
    pub reference: String,
    pub title: String,
    pub on: bool,
    /// Adds it to the set, or takes it out (its changes are not saved then)
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioCard {
    pub reference: String,
    pub title: String,
    /// "96 keys"
    pub count: Text,
    /// Keys that differ from the module's own
    pub changed: usize,
    /// Keys that overlap another
    pub conflicts: usize,
    /// Open at first: the first card, or every card the search finds something in
    pub open: bool,
    pub groups: Vec<StudioGroup>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioGroup {
    pub title: Text,
    pub start: bool,
    /// The way there from the start mode, as key caps
    pub way: Vec<Vec<Vec<String>>>,
    /// Open at first: the keys without a mode and the start mode, or every group the search finds something in
    pub open: bool,
    pub rows: Vec<StudioKeyRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioKeyRow {
    pub action: String,
    pub title: Text,
    /// Its keys as caps (each key's chords)
    pub caps: Vec<Vec<Vec<String>>>,
    /// "then Locked"; None: it stays in the mode
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Text>,
    /// It leads back to the mode its program starts in
    pub back_to_start: bool,
    /// Not the module's own key or mode afterwards
    pub changed: bool,
    /// It overlaps another key ("Overlaps New tab")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub does: Option<String>,
    pub own: bool,
    /// Opens the key editor (none for the program's own keys)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub edit: Option<Action>,
    /// Undo what was chosen here (back to where the page started)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset: Option<Action>,
}

/// The key editor (plan B148): one action's keys and the mode afterwards, changed apart from the list
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyEditor {
    pub module: String,
    pub action: String,
    pub title: Text,
    /// "Zellij · Pane"
    pub context: Text,
    /// The way to its mode, as caps
    pub way: Vec<Vec<Vec<String>>>,
    /// The keys chosen so far (`values.draft`), as written ("Ctrl+g") – a UI adds to or replaces them
    pub keys: Vec<String>,
    /// The same as caps
    pub caps: Vec<Vec<Vec<String>>>,
    /// Takes the key at the same place out
    pub remove: Vec<Action>,
    /// The keys as text ("Ctrl+g / Left", `values.draftText`) – for a UI that cannot record or put keys together
    pub text: Field,
    /// The modifiers to put a key together with: `value` as written ("Super"), `label` as on the host's keyboard
    pub modifiers: Vec<KeyChoice>,
    /// The keys to put together with, in groups (letters, digits, function keys, arrows and others)
    pub key_groups: Vec<KeyChoiceGroup>,
    /// Several chords one after the other are allowed (Emacs' Ctrl+x Ctrl+f)
    pub sequences: bool,
    /// The mode afterwards (`values.draftAfter`): its choices stay first, then the start mode, then the others
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Field>,
    /// What the keys chosen overlap, or the key rule they break
    pub conflicts: Vec<Hint>,
    /// The module's own keys
    pub own_caps: Vec<Vec<Vec<String>>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub does: Option<String>,
    /// Back to the module's own keys and mode (and closes)
    pub default: Action,
    pub cancel: Action,
    /// Takes the keys and the mode into the set (and closes)
    pub apply: Action,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyChoice {
    pub value: String,
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct KeyChoiceGroup {
    pub title: Text,
    pub keys: Vec<KeyChoice>,
}
