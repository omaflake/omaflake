// SPDX-License-Identifier: AGPL-3.0-or-later
//! The look studio (`lookStudio`, plan B150): a look of your own. A look is data, no Nix – its colours, the terminal's
//! sixteen, the desktop's borders and clock, the editors' themes, macOS guests' system look – so every part of it is a
//! field here: colours with the system's colour picker, the rest as choices. It starts from a look here (one of yours
//! is edited, any other copied) or from omaflake's own; what the studio does not show of it (the desktop's split
//! colours, GTK, the pointer) is kept as it is. A preview shows the colours as a window, a terminal and the desktop's
//! borders; what is hard to read is said while you choose. Saved into My Recipes' folder (`looks/my/<name>/`).
//!
//! Everything chosen is a form value of the page: name, title, description, appearance, tags ("dark, pastel"), base
//! (the look it starts from), plain, `color.<name>`, `normal.<0-7>`, `bright.<0-7>`, `border.<0-1>`,
//! `inactive_border`, `clock`, `prompt.<ok|error|directory>`, neovim, emacs (`"<theme>:<variant>"`), `macos.<setting>`.

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::common::*;
use crate::text::Text;

#[derive(Debug, Default, Deserialize)]
pub struct LookStudioParams {
    /// The look it starts from ("my.look.dusk" is edited, any other copied)
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub values: Map<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookStudioPage {
    pub title: Text,
    /// The look of yours being edited ("dusk"); None for a new one or a copy
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    pub intro: Text,
    /// Name, title, description, light or dark, tags, the look it starts from, plain
    pub about: Vec<Field>,
    /// The colours and the rest, a section each (none while it is plain)
    pub sections: Vec<LookSection>,
    /// The look as it is now, for a preview a UI draws itself
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub preview: Option<LookPreview>,
    /// What is hard to read (text on its background and the like)
    pub hints: Vec<Hint>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookSection {
    pub id: String,
    pub title: Text,
    /// What the section's colours colour (help)
    pub help: Text,
    pub fields: Vec<Field>,
}

/// The colours of the look now: "#rrggbb" each
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookPreview {
    pub background: String,
    pub surface: String,
    pub selection: String,
    pub text: String,
    pub subtext: String,
    pub muted: String,
    pub accent: String,
    pub urgent: String,
    /// The terminal's eight, then its bright eight
    pub normal: Vec<String>,
    pub bright: Vec<String>,
    /// The active window's border: one colour or a gradient of two
    pub border: Vec<String>,
    pub inactive_border: String,
    pub clock: String,
}
