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
//! `after:<module>:<action>` (a mode); while the editor is open `edit` ("<module>:<action>"), `draft` (its keys),
//! `draftText` (the keys typed, "Ctrl+g / Left" – for a terminal) and `draftAfter`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::common::*;
use super::keys::{caps, key_caps, mode_title, ways};
use super::text::{tf, Text};
use super::Ctx;
use crate::composition::{parse_key, Binding, Composition, Issue, PartRef, RecipeSpec};
use crate::keymaps::{KeyEntry, Keymap, KeymapStore, Keys};
use crate::models::GuestKind;
use crate::modules::{Module, ModuleStore};
use crate::my_recipes::MyRecipes;

pub const PAGE: &str = "keymapStudio";

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

fn keys_text(keys: &[String]) -> String {
    keys.join(" / ")
}

/// "h / Left" → ["h", "Left"]; None when one is not a key
fn parse_keys(text: &str) -> Option<Vec<String>> {
    let keys: Vec<String> = text.split('/').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
    (!keys.is_empty() && keys.iter().all(|k| parse_key(k).is_some())).then_some(keys)
}

/// A value of keys: a list (the key editor's) or a text ("h / Left"); Err when one is not a key
fn keys_of(v: &Value) -> Result<Vec<String>, ()> {
    match v {
        Value::Array(list) => {
            let keys: Vec<String> = list.iter().filter_map(Value::as_str).map(str::to_string).collect();
            if !keys.is_empty() && keys.iter().all(|k| parse_key(k).is_some()) {
                Ok(keys)
            } else {
                Err(())
            }
        }
        Value::String(t) => parse_keys(t).ok_or(()),
        _ => Err(()),
    }
}

/// Sets a value of this page (`ui.field`), and more at once (`also`: value param → value, null clears)
fn set_values(
    id: &str,
    label: Text,
    icon: Option<&str>,
    page_params: &Value,
    first: (&str, Value),
    also: &[(&str, Value)],
) -> Action {
    let mut a = Action::field(id, label, icon, PAGE, page_params.clone(), first.0, first.0);
    a.params["value"] = first.1;
    if !also.is_empty() {
        a.params["also"] = Value::Array(also.iter().map(|(k, v)| json!({"valueParam": k, "value": v})).collect());
    }
    a
}

/// The editor's own values, cleared when it closes
const DRAFTS: [&str; 4] = ["edit", "draft", "draftText", "draftAfter"];

/// The keys a key is put together from: `value` as written, `label` as the cap shows it on this host
fn key_groups(host: super::HostOs) -> Vec<KeyChoiceGroup> {
    let choice = |v: &str| KeyChoice { value: v.to_string(), label: caps(v, host)[0][0].clone() };
    let group = |title: &str, keys: Vec<String>| KeyChoiceGroup {
        title: Text::new(title),
        keys: keys.iter().map(|k| choice(k)).collect(),
    };
    vec![
        group("Letters", ('a'..='z').map(String::from).collect()),
        group("Digits", ('0'..='9').map(String::from).collect()),
        group(
            "Arrows and others",
            [
                "Left",
                "Right",
                "Up",
                "Down",
                "Return",
                "Escape",
                "Tab",
                "Space",
                "Backspace",
                "PageUp",
                "PageDown",
                "Plus",
            ]
            .iter()
            .map(|s| s.to_string())
            .collect(),
        ),
        group("Function keys", (1..=12).map(|i| format!("F{i}")).collect()),
        group(
            "Signs",
            [",", ".", "/", ";", "'", "[", "]", "-", "=", "`", "\\"].iter().map(|s| s.to_string()).collect(),
        ),
    ]
}

pub(crate) fn page(ctx: &Ctx, p: &KeymapStudioParams) -> KeymapStudioPage {
    let lang = ctx.language.as_deref();
    let v = &p.values;
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let settings = crate::paths::Settings::load(&ctx.paths);
    let sets = KeymapStore::new(ctx.paths.clone()).list(&settings);
    let from = p.id.as_ref().and_then(|id| sets.iter().find(|k| &k.id == id));
    let mine = from.filter(|k| k.author == MyRecipes::AUTHOR).map(|k| k.name.clone());
    let page_params = match &p.id {
        Some(id) => json!({"id": id}),
        None => json!({}),
    };

    let name = text("name").unwrap_or_else(|| from.map(|k| k.name.clone()).unwrap_or_default());
    let title = text("title").unwrap_or_else(|| from.map(|k| k.title.clone()).unwrap_or_default());
    let description = text("description").unwrap_or_else(|| from.map(|k| k.description.clone()).unwrap_or_default());
    let platform = text("platform").and_then(|k| GuestKind::parse(&k)).unwrap_or(GuestKind::Nixos);
    let base_ref = text("base").unwrap_or_else(|| from.map(Keymap::reference).unwrap_or_default());
    let base = sets.iter().find(|k| k.reference() == base_ref);

    // The platform's modules with keys; those the set covers: as chosen, else those the base set names, else all
    let store = ModuleStore::new(ctx.paths.clone());
    let mut modules: Vec<Module> =
        store.list(&settings).into_iter().filter(|m| m.platform == platform && !m.actions.is_empty()).collect();
    modules.sort_by_key(|m| (!m.desktop, m.title.to_lowercase()));
    let covered: Vec<String> = match v.get("modules").and_then(Value::as_array) {
        Some(list) => list.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => {
            let named: Vec<String> = modules
                .iter()
                .map(Module::reference)
                .filter(|r| base.is_some_and(|b| b.keys.contains_key(r)))
                .collect();
            if named.is_empty() {
                modules.iter().map(Module::reference).collect()
            } else {
                named
            }
        }
    };
    let covered: Vec<String> = modules.iter().map(Module::reference).filter(|r| covered.contains(r)).collect();

    // The key being changed: its draft keys and mode afterwards count while the editor is open
    let edit = text("edit").and_then(|e| {
        let (m, a) = e.rsplit_once(':')?;
        let module = modules.iter().find(|x| x.reference() == m && covered.contains(&x.reference()))?;
        module.actions.get(a).filter(|x| !x.own)?;
        Some((m.to_string(), a.to_string()))
    });
    let mut draft_error = None;
    let draft_keys: Option<Vec<String>> = edit.as_ref().and_then(|_| {
        if let Some(t) = text("draftText") {
            match parse_keys(&t) {
                Some(k) => return Some(k),
                None => draft_error = Some(Text::new("Keys like Ctrl+g, Super+Return, Space f f – several with /")),
            }
        }
        v.get("draft").and_then(|d| keys_of(d).ok())
    });
    let draft_after = edit.as_ref().and_then(|_| text("draftAfter"));

    // The set as chosen: the base set's entries, then the page's values – only what differs from the modules' keys
    let mut keys: Keys = BTreeMap::new();
    let mut errors: BTreeMap<String, Text> = BTreeMap::new();
    for m in modules.iter().filter(|m| covered.contains(&m.reference())) {
        let r = m.reference();
        for (action, a) in m.actions.iter().filter(|(_, a)| !a.own) {
            let from_base = base.and_then(|b| b.keys.get(&r)).and_then(|x| x.get(action));
            let mut key = from_base.filter(|e| !e.key.is_empty()).map(|e| e.key.clone()).unwrap_or(a.key.clone());
            let mut after = from_base.and_then(|e| e.after.clone()).or(a.after.clone());
            let id = format!("key:{r}:{action}");
            if let Some(value) = v.get(&id).filter(|x| !x.is_null()) {
                match keys_of(value) {
                    Ok(k) => key = k,
                    Err(()) => {
                        errors.insert(id, Text::new("Keys like Ctrl+g, Super+Return, Space f f – several with /"));
                    }
                }
            }
            if let Some(t) = text(&format!("after:{r}:{action}")) {
                after = Some(t);
            }
            if edit.as_ref() == Some(&(r.clone(), action.clone())) {
                if let Some(k) = &draft_keys {
                    key = k.clone();
                }
                if let Some(t) = &draft_after {
                    after = Some(t.clone());
                }
            }
            let entry = KeyEntry {
                key: if key != a.key { key } else { vec![] },
                after: if after != a.after { after } else { None },
            };
            if !entry.key.is_empty() || entry.after.is_some() {
                keys.entry(r.clone()).or_default().insert(action.clone(), entry);
            }
        }
    }
    let changed_count: usize = keys.values().map(|a| a.len()).sum();

    // What it gives, put together with every module of the platform: the bindings and what overlaps
    let draft = Keymap {
        id: format!("{}.keymap.{}", MyRecipes::AUTHOR, if name.is_empty() { "new" } else { &name }),
        author: MyRecipes::AUTHOR.into(),
        name: name.clone(),
        version: "1.0.0".into(),
        title: title.clone(),
        description: description.clone(),
        descriptions: BTreeMap::new(),
        keys: keys.clone(),
        source: String::new(),
        hash: None,
        folder: Default::default(),
    };
    let spec = RecipeSpec {
        platform,
        modules: modules.iter().map(|m| PartRef::new(&m.reference())).collect(),
        look: None,
        keymaps: vec![PartRef::new(&draft.reference())],
        requirements: Default::default(),
        locales: Default::default(),
        data: Value::Null,
    };
    let composition = Composition::of(&spec, &modules, std::slice::from_ref(&draft), &store.capabilities(&settings));
    let mut conflicts: BTreeMap<(String, String), Text> = BTreeMap::new();
    let mut rule: BTreeMap<(String, String), Text> = BTreeMap::new();
    let mut hints = Vec::new();
    for i in &composition.issues {
        match i {
            Issue::KeysOverlap { first, second } => {
                for (a, b) in [(first, second), (second, first)] {
                    conflicts.insert((a.module.clone(), a.action.clone()), tf!("Overlaps %@", &b.title));
                }
                hints.push(
                    Hint::new("warning", tf!("Keys overlap: %@ and %@", &first.title, &second.title), Tone::Warning)
                        .keys(key_caps(&first.keys, ctx.host)),
                );
            }
            Issue::KeyRule { binding } => {
                let t = tf!("%@: without a mode a key starts with Super or Ctrl", &binding.title);
                rule.insert((binding.module.clone(), binding.action.clone()), t.clone());
                hints.push(Hint::new("warning", t, Tone::Warning).keys(key_caps(&binding.keys, ctx.host)));
            }
            _ => {}
        }
    }

    // A card per module the set covers: its keys by mode, as the keys page groups them
    let search = text("search").unwrap_or_default().trim().to_lowercase();
    let closing: Vec<(&str, Value)> = DRAFTS[1..].iter().map(|k| (*k, Value::Null)).collect();
    let mut cards = Vec::new();
    for (ci, m) in modules.iter().filter(|m| covered.contains(&m.reference())).enumerate() {
        let r = m.reference();
        let bindings: Vec<&Binding> = composition.bindings.iter().filter(|b| b.module == r).collect();
        let start = m.modes.iter().find(|(_, x)| x.start).map(|(id, _)| id.clone());
        let found = ways(&bindings, start.as_deref());
        let row = |b: &Binding, here: Option<&str>| -> Option<StudioKeyRow> {
            let a = m.actions.get(&b.action)?;
            let title = a.title_in(lang).to_string();
            let hay =
                format!("{} {} {} {}", title, b.key_text(), b.does.clone().unwrap_or_default(), m.title).to_lowercase();
            if !search.is_empty() && !search.split_whitespace().all(|w| hay.contains(w)) {
                return None;
            }
            let id = format!("key:{}:{}", b.module, b.action);
            let after_id = format!("after:{}:{}", b.module, b.action);
            let after = b.after.as_deref().filter(|x| Some(*x) != here);
            let edit = (!a.own).then(|| {
                let mut also = closing.clone();
                also.retain(|(k, _)| *k != "edit");
                set_values(
                    "edit",
                    Text::new("Change …"),
                    Some("edit"),
                    &page_params,
                    ("edit", json!(format!("{}:{}", b.module, b.action))),
                    &also,
                )
            });
            let touched = v.contains_key(&id) || v.contains_key(&after_id);
            let reset = touched.then(|| {
                set_values(
                    "reset",
                    Text::new("Undo"),
                    Some("reset"),
                    &page_params,
                    (&id, Value::Null),
                    &[(&after_id, Value::Null)],
                )
            });
            Some(StudioKeyRow {
                action: b.action.clone(),
                title: Text::verbatim(&title),
                caps: key_caps(&b.keys, ctx.host),
                after: after.map(|x| tf!("then %@", mode_title(&m.modes, x, lang))),
                back_to_start: after.is_some() && after == start.as_deref(),
                changed: !a.own && (b.keys != a.key || b.after != a.after),
                conflict: conflicts
                    .get(&(b.module.clone(), b.action.clone()))
                    .or_else(|| rule.get(&(b.module.clone(), b.action.clone())))
                    .cloned(),
                does: b.does.clone(),
                own: a.own,
                edit,
                reset,
            })
        };
        let mut groups = Vec::new();
        let modeless: Vec<StudioKeyRow> =
            bindings.iter().filter(|b| b.modes.is_empty()).filter_map(|b| row(b, None)).collect();
        if !modeless.is_empty() {
            let title = if m.modes.is_empty() { Text::new("In the program") } else { Text::new("In every mode") };
            groups.push(StudioGroup { title, start: false, way: vec![], open: true, rows: modeless });
        }
        let mut order: Vec<&String> = m.modes.keys().collect();
        order.sort_by_key(|id| {
            (found.get(*id).map(|w| w.len()).unwrap_or(usize::MAX), mode_title(&m.modes, id, lang).text.to_lowercase())
        });
        for id in order {
            let rows: Vec<StudioKeyRow> =
                bindings.iter().filter(|b| b.modes.contains(id)).filter_map(|b| row(b, Some(id))).collect();
            if rows.is_empty() {
                continue;
            }
            let is_start = Some(id) == start.as_ref();
            groups.push(StudioGroup {
                title: mode_title(&m.modes, id, lang),
                start: is_start,
                way: found.get(id).map(|w| w.iter().map(|(k, _)| caps(k, ctx.host)).collect()).unwrap_or_default(),
                open: is_start || !search.is_empty(),
                rows,
            });
        }
        if groups.is_empty() && !search.is_empty() {
            continue;
        }
        let all: Vec<&Binding> = bindings.iter().copied().filter(|b| !b.own).collect();
        cards.push(StudioCard {
            reference: r.clone(),
            title: m.title.clone(),
            count: if m.actions.len() == 1 { Text::new("1 key") } else { tf!("%lld keys", m.actions.len()) },
            changed: keys.get(&r).map(|a| a.len()).unwrap_or(0),
            conflicts: all.iter().filter(|b| conflicts.contains_key(&(b.module.clone(), b.action.clone()))).count(),
            open: ci == 0 || !search.is_empty(),
            groups,
        });
    }

    // The chips: the modules it covers, then those to add
    let chips: Vec<StudioModuleChip> = modules
        .iter()
        .map(|m| {
            let r = m.reference();
            let on = covered.contains(&r);
            let list: Vec<String> = if on {
                covered.iter().filter(|x| **x != r).cloned().collect()
            } else {
                modules.iter().map(Module::reference).filter(|x| covered.contains(x) || *x == r).collect()
            };
            let label = if on { tf!("Take %@ out of the set", &m.title) } else { tf!("Add %@", &m.title) };
            StudioModuleChip {
                reference: r,
                title: m.title.clone(),
                on,
                action: set_values(
                    if on { "remove" } else { "add" },
                    label,
                    Some(if on { "minus" } else { "add" }),
                    &page_params,
                    ("modules", json!(list)),
                    &[],
                ),
            }
        })
        .collect();

    // The key editor
    let editor = edit.as_ref().and_then(|(mr, ar)| {
        let m = modules.iter().find(|x| &x.reference() == mr)?;
        let a = m.actions.get(ar)?;
        let b = composition.bindings.iter().find(|b| &b.module == mr && &b.action == ar)?;
        let start = m.modes.iter().find(|(_, x)| x.start).map(|(id, _)| id.clone());
        let bindings: Vec<&Binding> = composition.bindings.iter().filter(|x| &x.module == mr).collect();
        let found = ways(&bindings, start.as_deref());
        let here = (b.modes.len() == 1).then(|| b.modes[0].clone());
        let current = b.keys.clone();
        let key_id = format!("key:{mr}:{ar}");
        let after_id = format!("after:{mr}:{ar}");
        let mut text_field = Field::new(
            "draftText",
            FieldKind::Text,
            Text::new("Keys"),
            json!(text("draftText").unwrap_or_else(|| keys_text(&current))),
        )
        .value_param("draftText");
        text_field.placeholder = Some(Text::new("Ctrl+g / Left"));
        text_field.error = draft_error.clone();
        let after = here.as_ref().filter(|_| !m.modes.is_empty()).map(|here| {
            let value = b.after.clone().unwrap_or_else(|| here.clone());
            let mut f = Field::new("draftAfter", FieldKind::Choice, Text::new("Afterwards"), json!(value))
                .value_param("draftAfter");
            let mut order: Vec<&String> = m.modes.keys().collect();
            order.sort_by_key(|id| {
                (*id != here, Some(*id) != start.as_ref(), mode_title(&m.modes, id, lang).text.to_lowercase())
            });
            f.choices = order
                .into_iter()
                .map(|mode| {
                    let label = if mode == here {
                        tf!("stays in %@", mode_title(&m.modes, mode, lang))
                    } else if Some(mode) == start.as_ref() {
                        tf!("back to %@", mode_title(&m.modes, mode, lang))
                    } else {
                        tf!("to %@", mode_title(&m.modes, mode, lang))
                    };
                    Choice::new(json!(mode), label)
                })
                .collect();
            f
        });
        let mut problems = Vec::new();
        if let Some(t) = conflicts.get(&(mr.clone(), ar.clone())) {
            problems.push(Hint::new("warning", t.clone(), Tone::Warning).keys(key_caps(&current, ctx.host)));
        }
        if let Some(t) = rule.get(&(mr.clone(), ar.clone())) {
            problems.push(Hint::new("warning", t.clone(), Tone::Warning));
        }
        let close = |extra: Vec<(&str, Value)>| -> Vec<(String, Value)> {
            let mut out: Vec<(String, Value)> = extra.into_iter().map(|(k, v)| (k.to_string(), v)).collect();
            out.extend(DRAFTS.iter().map(|k| (k.to_string(), Value::Null)));
            out
        };
        let action_of = |id: &str, label: Text, icon: Option<&str>, values: Vec<(String, Value)>| {
            let (first, rest) = values.split_first().expect("a value");
            let rest: Vec<(&str, Value)> = rest.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
            set_values(id, label, icon, &page_params, (first.0.as_str(), first.1.clone()), &rest)
        };
        let mut apply_values = vec![(key_id.as_str(), json!(current))];
        if after.is_some() {
            apply_values.push((after_id.as_str(), json!(b.after.clone().or(here.clone()))));
        }
        let apply = action_of("apply", Text::new("Apply"), Some("apply"), close(apply_values))
            .enabled_if(!current.is_empty() && draft_error.is_none(), draft_error.clone())
            .primary();
        let default = action_of(
            "default",
            Text::new("The module's own"),
            Some("reset"),
            close(vec![(key_id.as_str(), json!(a.key)), (after_id.as_str(), json!(a.after.clone().or(here.clone())))]),
        );
        let cancel = action_of("cancel", Text::new("Cancel"), None, close(vec![]));
        let remove = (0..current.len())
            .map(|i| {
                let rest: Vec<&String> = current.iter().enumerate().filter(|(j, _)| *j != i).map(|(_, k)| k).collect();
                set_values(
                    "removeKey",
                    Text::new("Remove this key"),
                    Some("minus"),
                    &page_params,
                    ("draft", json!(rest)),
                    &[("draftText", Value::Null)],
                )
            })
            .collect();
        let way = here
            .as_ref()
            .and_then(|h| found.get(h))
            .map(|w| w.iter().map(|(k, _)| caps(k, ctx.host)).collect())
            .unwrap_or_default();
        let mode = here.as_ref().map(|h| mode_title(&m.modes, h, lang));
        Some(KeyEditor {
            module: mr.clone(),
            action: ar.clone(),
            title: Text::verbatim(a.title_in(lang)),
            context: match mode {
                Some(t) => Text::joined(vec![Text::verbatim(&m.title), t]),
                None => Text::verbatim(&m.title),
            },
            way,
            caps: key_caps(&current, ctx.host),
            keys: current,
            remove,
            text: text_field,
            modifiers: ["Ctrl", "Alt", "Shift", "Super"]
                .iter()
                .map(|x| KeyChoice { value: x.to_string(), label: caps(x, ctx.host)[0][0].clone() })
                .collect(),
            key_groups: key_groups(ctx.host),
            sequences: true,
            after,
            conflicts: problems,
            own_caps: key_caps(&a.key, ctx.host),
            does: b.does.clone(),
            default,
            cancel,
            apply,
        })
    });

    // About
    let taken = sets.iter().any(|k| k.author == MyRecipes::AUTHOR && k.name == name && mine.as_deref() != Some(&name));
    let mut name_field = Field::new("name", FieldKind::Text, Text::new("Name"), json!(name))
        .value_param("name")
        .help(Text::new("lowercase letters, digits and - – recipes name the set my/<name>"));
    if name.is_empty() || !crate::recipe_sources::RecipeRef::valid_name(&name) {
        name_field.error = Some(Text::new("A name is lowercase letters, digits and -"));
    } else if taken {
        name_field.error = Some(tf!("There are key bindings \"%@\" of yours already", &name));
    }
    let mut platform_field = Field::new("platform", FieldKind::Choice, Text::new("Platform"), json!(platform.as_str()))
        .value_param("platform");
    platform_field.choices = vec![
        Choice::new(json!("nixos"), Text::verbatim("NixOS")),
        Choice::new(json!("macos"), Text::verbatim("macOS")),
    ];
    platform_field.help = Some(Text::new("Whose modules are shown – a set may name modules of both"));
    let mut base_field = Field::new("base", FieldKind::Choice, Text::new("Starts from"), json!(base_ref))
        .value_param("base")
        .help(Text::new("The set whose keys it starts with; without one, every module's own keys"));
    base_field.choices = vec![Choice::new(json!(""), Text::new("the modules' own keys"))];
    for k in &sets {
        base_field
            .choices
            .push(Choice::new(json!(k.reference()), Text::verbatim(format!("{} – {}", k.title, k.reference()))));
    }
    let about = vec![
        name_field.clone(),
        Field::new("title", FieldKind::Text, Text::new("Title"), json!(title)).value_param("title"),
        Field::new("description", FieldKind::Text, Text::new("Description"), json!(description))
            .value_param("description"),
        platform_field,
        base_field,
    ];
    let mut search_field =
        Field::new("search", FieldKind::Text, Text::new("Search"), json!(text("search").unwrap_or_default()))
            .value_param("search");
    search_field.placeholder = Some(Text::new("Search keys: what it does, its key"));

    let mut params = json!({"name": name, "title": title, "description": description, "keys": keys});
    if let Some(n) = &mine {
        params["previous"] = json!(n);
    }
    let error = name_field.error.clone().or_else(|| errors.values().next().cloned());
    let save = Action::call("save", Text::new("Save"), Some("apply"), "keymaps.save", params)
        .then(Then::Back)
        .enabled_if(error.is_none(), error)
        .primary();
    let mut actions = vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({}))];
    if let Some(n) = &mine {
        actions.push(
            Action::call("delete", Text::new("Delete …"), Some("trash"), "keymaps.delete", json!({"name": n}))
                .destructive()
                .then(Then::Back)
                .confirm(
                    Confirm::new(
                        tf!("Delete the key bindings “%@”?", n),
                        Text::new("Recipes that name them get each module's own keys."),
                    )
                    .button("Delete")
                    .destructive(),
                ),
        );
    }
    actions.push(save);
    KeymapStudioPage {
        title: Text::new(if mine.is_some() {
            "Edit Key Bindings"
        } else if from.is_some() {
            "Copy Key Bindings"
        } else {
            "Key Binding Studio"
        }),
        id: mine,
        intro: Text::new(
            "Key bindings of your own: open a module, then “Change …” at a key – record it or put it together, and choose where it leads afterwards. What overlaps is said while you choose. Only what differs from the modules' own keys is saved, in My Recipes' folder; recipes name the set my/<name>.",
        ),
        about,
        modules: chips,
        search: search_field,
        hints,
        cards,
        changed: tf!("%lld keys differ from the modules' own", changed_count),
        editor,
        actions,
    }
}
