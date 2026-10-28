// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Keys page (`keys`, plan B147): every key of a recipe – or of the recipe an image or a VM was built from –
//! grouped by module and, within a module, by the mode of its program. A mode shows the way there from the mode its
//! program starts in (Zellij: Ctrl+g › p › Pane); a key says the mode it leads to ("then Locked"), what it does in
//! the program's own words and which key binding set gave it. The UIs show it in a window or page of its own and
//! filter it by what is typed.

use std::collections::{BTreeMap, VecDeque};

use serde::{Deserialize, Serialize};

use super::common::*;
use super::text::{tf, Text};
use super::{Ctx, HostOs};
use crate::api::types::{RecipeDto, RecipePartsDto};
use crate::composition::Binding;
use crate::modules::ModuleMode;

/// Whose keys: a recipe by id, an image or a VM by name (through the recipe it was built from)
#[derive(Debug, Default, Deserialize)]
pub struct KeysParams {
    pub recipe: Option<String>,
    pub image: Option<String>,
    pub vm: Option<String>,
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
    /// Each key as key caps: its chords, each a list of caps ("⌘", "⇧", "↩")
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

/// One key as caps: chords separated by spaces, keys of a chord by "+"; the Mac's names on a Mac (Super is ⌘ there)
pub fn caps(key: &str, host: HostOs) -> Vec<Vec<String>> {
    let mac = host == HostOs::Macos;
    key.split_whitespace()
        .map(|chord| {
            chord
                .split('+')
                .map(|k| {
                    match k {
                        "Super" if mac => "⌘",
                        "Super" => "Super",
                        "Ctrl" if mac => "⌃",
                        "Alt" if mac => "⌥",
                        "Shift" => "⇧",
                        "Return" => "↩",
                        "Escape" => "Esc",
                        "Tab" => "⇥",
                        "Backspace" => "⌫",
                        "Space" => "␣",
                        "Left" => "←",
                        "Right" => "→",
                        "Up" => "↑",
                        "Down" => "↓",
                        "PageUp" => "⇞",
                        "PageDown" => "⇟",
                        "Plus" => "+",
                        "Digit" => "1–9",
                        "Arrow" => "←↑→↓",
                        other => other,
                    }
                    .to_string()
                })
                .collect()
        })
        .collect()
}

/// The keyboard button of a recipe's, an image's or a VM's row (`params`: `{recipe}`, `{image}` or `{vm}`), when its
/// recipe has keys
pub(crate) fn action(ctx: &Ctx, recipe: &str, params: serde_json::Value) -> Option<Action> {
    let r = ctx.recipes.recipes.iter().find(|r| r.id == recipe)?;
    r.parts.as_ref().filter(|p| !p.bindings.is_empty())?;
    Some(Action::show("keys", Text::new("Keys …"), Some("keyboard"), "keys", params))
}

fn recipe_of<'a>(ctx: &'a Ctx, p: &KeysParams) -> (Option<&'a RecipeDto>, Text, Vec<Hint>) {
    let by_id = |id: &str| ctx.recipes.recipes.iter().find(|r| r.id == id);
    let built = Hint::new(
        "info",
        Text::new("As its recipe has them now – a module or key binding set updated since the build may differ."),
        Tone::Normal,
    );
    if let Some(name) = &p.vm {
        let recipe = ctx.vms.iter().find(|v| &v.name == name).map(|v| v.recipe.clone());
        let r = recipe.as_deref().and_then(by_id);
        return (r, tf!("VM %@", name), vec![built]);
    }
    if let Some(name) = &p.image {
        let recipe = ctx.images.iter().find(|i| &i.name == name).map(|i| i.recipe.clone());
        let r = recipe.as_deref().and_then(by_id);
        return (r, tf!("Image %@", name), vec![built]);
    }
    let r = p.recipe.as_deref().and_then(by_id);
    (r, Text::verbatim(r.map(|r| r.display_name.clone()).unwrap_or_default()), vec![])
}

pub(crate) fn page(ctx: &Ctx, p: &KeysParams) -> KeysPage {
    let (recipe, whose, mut notes) = recipe_of(ctx, p);
    let name = recipe.map(|r| r.display_name.clone()).unwrap_or_default();
    let parts = recipe.and_then(|r| r.parts.as_ref());
    let modules = match parts {
        Some(parts) => modules(ctx, parts),
        None => {
            notes = vec![Hint::new(
                "warning",
                Text::new("Its recipe is not here – its keys are not known."),
                Tone::Warning,
            )];
            vec![]
        }
    };
    KeysPage {
        title: if name.is_empty() { Text::new("Keys") } else { tf!("Keys of %@", name) },
        subtitle: whose,
        intro: Text::new(
            "By module and by the mode its program is in. A mode shows the keys that lead there; “then …” is the mode a key leads to.",
        ),
        notes,
        modules,
        keymaps: parts.map(|p| p.keymaps.clone()).unwrap_or_default(),
        search: Text::new("Search keys"),
    }
}

/// The modules in the recipe's order, those with keys on the whole desktop first
fn modules(ctx: &Ctx, parts: &RecipePartsDto) -> Vec<KeyModule> {
    let lang = ctx.language.as_deref();
    let mut out: Vec<(bool, KeyModule)> = Vec::new();
    for m in &parts.modules {
        let reference = m.reference.split_whitespace().next().unwrap_or_default().to_string();
        let bindings: Vec<&Binding> = parts.bindings.iter().filter(|b| b.module == reference).collect();
        if bindings.is_empty() {
            continue;
        }
        let desktop = bindings.iter().any(|b| b.context == "desktop");
        out.push((
            desktop,
            KeyModule {
                title: m.title.clone().unwrap_or_else(|| reference.clone()),
                groups: groups(ctx, &bindings, &m.modes, lang),
                reference,
            },
        ));
    }
    out.sort_by_key(|(desktop, _)| !desktop);
    out.into_iter().map(|(_, m)| m).collect()
}

fn mode_title(modes: &BTreeMap<String, ModuleMode>, id: &str, lang: Option<&str>) -> Text {
    Text::verbatim(modes.get(id).map(|m| m.title_in(lang)).unwrap_or(id))
}

/// The keys without a mode (the desktop's first), then the start mode, then the modes in the order they are reached
fn groups(ctx: &Ctx, bindings: &[&Binding], modes: &BTreeMap<String, ModuleMode>, lang: Option<&str>) -> Vec<KeyGroup> {
    let start = modes.iter().find(|(_, m)| m.start).map(|(id, _)| id.clone());
    let row = |b: &Binding, here: Option<&str>| {
        let title = match (lang, &b.title_de) {
            (Some("de"), Some(t)) => t.clone(),
            _ => b.title.clone(),
        };
        let after = b.after.as_deref().filter(|a| Some(*a) != here);
        KeyRow {
            action: b.action.clone(),
            keys: b.keys.clone(),
            caps: b.keys.iter().map(|k| caps(k, ctx.host)).collect(),
            title: Text::verbatim(title),
            after: after.map(|a| tf!("then %@", mode_title(modes, a, lang))),
            back_to_start: after.is_some() && after == start.as_deref(),
            switches: after.is_some() && b.does.is_none(),
            does: b.does.clone(),
            from: b.from.clone(),
            own: b.own,
        }
    };
    // Doing something first, then the keys that only change the mode; each part by title
    let sorted = |mut rows: Vec<KeyRow>| {
        rows.sort_by(|a, b| (a.switches, &a.title.text).cmp(&(b.switches, &b.title.text)));
        rows
    };
    let mut out = Vec::new();
    for desktop in [true, false] {
        let rows: Vec<KeyRow> = bindings
            .iter()
            .filter(|b| b.modes.is_empty() && (b.context == "desktop") == desktop)
            .map(|b| row(b, None))
            .collect();
        if rows.is_empty() {
            continue;
        }
        let title = match (desktop, modes.is_empty()) {
            (true, _) => Text::new("Anywhere on the desktop"),
            (false, true) => Text::new("In the program"),
            (false, false) => Text::new("In every mode"),
        };
        out.push(KeyGroup { mode: None, title, start: false, way: vec![], rows: sorted(rows) });
    }
    let ways = ways(bindings, start.as_deref());
    let mut order: Vec<&String> = modes.keys().collect();
    // Nearer modes first; as near: by name
    order.sort_by_key(|id| {
        (ways.get(*id).map(|w| w.len()).unwrap_or(usize::MAX), mode_title(modes, id, lang).text.to_lowercase())
    });
    for id in order {
        let rows: Vec<KeyRow> =
            bindings.iter().filter(|b| b.modes.contains(id)).map(|b| row(b, Some(id.as_str()))).collect();
        if rows.is_empty() {
            continue;
        }
        let way = ways
            .get(id)
            .map(|w| {
                w.iter()
                    .map(|(key, to)| KeyStep { caps: caps(key, ctx.host), mode: mode_title(modes, to, lang) })
                    .collect()
            })
            .unwrap_or_default();
        out.push(KeyGroup {
            mode: Some(id.clone()),
            title: mode_title(modes, id, lang),
            start: Some(id) == start.as_ref(),
            way,
            rows: sorted(rows),
        });
    }
    out
}

/// The shortest way from the start mode to each mode: (key, mode reached) per step. Keys that only change the mode
/// are preferred, then the first key of an action.
fn ways(bindings: &[&Binding], start: Option<&str>) -> BTreeMap<String, Vec<(String, String)>> {
    let mut found: BTreeMap<String, Vec<(String, String)>> = BTreeMap::new();
    let Some(start) = start else { return found };
    let mut edges: Vec<&Binding> =
        bindings.iter().copied().filter(|b| b.after.is_some() && !b.keys.is_empty()).collect();
    edges.sort_by_key(|b| (b.does.is_some(), b.action.clone()));
    found.insert(start.to_string(), vec![]);
    let mut queue = VecDeque::from([start.to_string()]);
    while let Some(mode) = queue.pop_front() {
        let way = found[&mode].clone();
        for b in edges.iter().filter(|b| b.modes.contains(&mode)) {
            let to = b.after.clone().unwrap_or_default();
            if found.contains_key(&to) {
                continue;
            }
            let mut next = way.clone();
            next.push((b.keys[0].clone(), to.clone()));
            found.insert(to.clone(), next);
            queue.push_back(to);
        }
    }
    found
}
