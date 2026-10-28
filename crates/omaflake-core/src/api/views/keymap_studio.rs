// SPDX-License-Identifier: AGPL-3.0-or-later
//! The key binding studio (`keymapStudio`, plan B147): a key binding set of your own. It starts from a set here (a copy)
//! or from the modules' own keys; one module at a time, by the mode of its program as the keys page shows them, each
//! action has its keys and – where its program has modes – the mode it leads to afterwards (Zellij: does `n` in pane
//! mode lock again, or stay in pane mode). What overlaps is said while you choose, across every module of the
//! platform. Saved into My Recipes' folder (`keymaps/my/<name>/`); only what differs from the modules' own keys is
//! written. The program's own keys (`own`) are shown, not changed.
//!
//! Everything chosen is a form value of the page: name, title, description, platform, base (the set it starts from),
//! module, search, and per action `key:<module>:<action>` ("h / Left") and `after:<module>:<action>` (a mode).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::common::*;
use super::keys::{caps, mode_title, ways};
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
    /// The module whose keys are shown, and the search
    pub module: Field,
    pub search: Field,
    /// What overlaps or breaks the key rule, across the platform's modules
    pub hints: Vec<Hint>,
    /// The module's keys by mode
    pub groups: Vec<StudioGroup>,
    /// "12 keys changed"
    pub changed: Text,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioGroup {
    pub title: Text,
    pub start: bool,
    /// The way there from the start mode, as key caps
    pub way: Vec<Vec<Vec<String>>>,
    pub rows: Vec<StudioKeyRow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioKeyRow {
    pub action: String,
    pub title: Text,
    /// Its keys ("h / Left"); disabled for the program's own keys
    pub key: Field,
    /// The mode afterwards – for an action in one mode of a program with modes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<Field>,
    /// Not the module's own key or mode afterwards
    pub changed: bool,
    /// Undo the key and mode chosen here (back to where the page started)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset: Option<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub does: Option<String>,
    /// It overlaps another key ("Overlaps New tab (n)")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<Text>,
    pub own: bool,
}

fn keys_text(keys: &[String]) -> String {
    keys.join(" / ")
}

/// "h / Left" → ["h", "Left"]; None when one is not a key
fn parse_keys(text: &str) -> Option<Vec<String>> {
    let keys: Vec<String> = text.split('/').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
    (!keys.is_empty() && keys.iter().all(|k| parse_key(k).is_some())).then_some(keys)
}

/// Sets the values `key:` and `after:` of an action (`ui.field` on this page)
fn set_value(id: &str, label: Text, page_params: &Value, value_param: &str, value: Value) -> Action {
    let mut a = Action::field(id, label, Some("reset"), PAGE, page_params.clone(), value_param, value_param);
    a.params["value"] = value;
    a
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

    // The platform's modules with keys
    let store = ModuleStore::new(ctx.paths.clone());
    let mut modules: Vec<Module> =
        store.list(&settings).into_iter().filter(|m| m.platform == platform && !m.actions.is_empty()).collect();
    modules.sort_by_key(|m| (!m.desktop, m.title.to_lowercase()));
    let module_ref = text("module")
        .filter(|r| modules.iter().any(|m| &m.reference() == r))
        .or_else(|| modules.first().map(Module::reference))
        .unwrap_or_default();

    // The set as chosen: the base set's entries, then the page's values – only what differs from the modules' keys
    let mut keys: Keys = BTreeMap::new();
    let mut errors: BTreeMap<String, Text> = BTreeMap::new();
    for m in &modules {
        let r = m.reference();
        for (action, a) in m.actions.iter().filter(|(_, a)| !a.own) {
            let from_base = base.and_then(|b| b.keys.get(&r)).and_then(|x| x.get(action));
            let mut key = from_base.filter(|e| !e.key.is_empty()).map(|e| e.key.clone()).unwrap_or(a.key.clone());
            let mut after = from_base.and_then(|e| e.after.clone()).or(a.after.clone());
            let id = format!("key:{r}:{action}");
            if let Some(t) = text(&id) {
                match parse_keys(&t) {
                    Some(k) => key = k,
                    None => {
                        errors.insert(id, Text::new("Keys like Ctrl+g, Super+Return, Space f f – several with /"));
                    }
                }
            }
            if let Some(t) = text(&format!("after:{r}:{action}")) {
                after = Some(t);
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
    let mut hints = Vec::new();
    for i in &composition.issues {
        match i {
            Issue::KeysOverlap { first, second } => {
                for (a, b) in [(first, second), (second, first)] {
                    conflicts.insert((a.module.clone(), a.action.clone()), tf!("Overlaps %@", &b.title));
                }
                hints.push(
                    Hint::new("warning", tf!("Keys overlap: %@ and %@", &first.title, &second.title), Tone::Warning)
                        .keys(super::keys::key_caps(&first.keys, ctx.host)),
                );
            }
            Issue::KeyRule { binding } => hints.push(
                Hint::new(
                    "warning",
                    tf!("%@: without a mode a key starts with Super or Ctrl", &binding.title),
                    Tone::Warning,
                )
                .keys(super::keys::key_caps(&binding.keys, ctx.host)),
            ),
            _ => {}
        }
    }

    // The module's keys by mode, as the keys page groups them
    let module = modules.iter().find(|m| m.reference() == module_ref);
    let bindings: Vec<&Binding> = composition.bindings.iter().filter(|b| b.module == module_ref).collect();
    let search = text("search").unwrap_or_default().trim().to_lowercase();
    let mut groups = Vec::new();
    if let Some(m) = module {
        let start = m.modes.iter().find(|(_, x)| x.start).map(|(id, _)| id.clone());
        let found = ways(&bindings, start.as_deref());
        let row = |b: &Binding| -> Option<StudioKeyRow> {
            let a = m.actions.get(&b.action)?;
            let title = a.title_in(lang).to_string();
            let hay = format!("{} {} {}", title, b.key_text(), b.does.clone().unwrap_or_default()).to_lowercase();
            if !search.is_empty() && !search.split_whitespace().all(|w| hay.contains(w)) {
                return None;
            }
            let id = format!("key:{}:{}", b.module, b.action);
            let mut key =
                Field::new(&id, FieldKind::Text, Text::verbatim(&title), json!(keys_text(&b.keys))).value_param(&id);
            if let Some(t) = text(&id) {
                key.value = json!(t);
            }
            key.error = errors.get(&id).cloned();
            if a.own {
                key.enabled = false;
                key.reason = Some(Text::new("The program's own key – key bindings do not change it"));
            }
            // The mode afterwards: for an action in one mode, choosing among the module's modes
            let after = (!a.own && b.modes.len() == 1 && !m.modes.is_empty()).then(|| {
                let here = &b.modes[0];
                let id = format!("after:{}:{}", b.module, b.action);
                let value = b.after.clone().unwrap_or_else(|| here.clone());
                let mut f = Field::new(&id, FieldKind::Choice, Text::new("then"), json!(value)).value_param(&id);
                f.choices = m
                    .modes
                    .keys()
                    .map(|mode| {
                        let label = if mode == here {
                            tf!("stays in %@", mode_title(&m.modes, mode, lang))
                        } else {
                            mode_title(&m.modes, mode, lang)
                        };
                        Choice::new(json!(mode), label)
                    })
                    .collect();
                f
            });
            let changed = !a.own && (b.keys != a.key || b.after != a.after);
            // Undo what was chosen here: both values back to where the page started (the base set, or the module's)
            let after_id = format!("after:{}:{}", b.module, b.action);
            let touched = v.contains_key(&id) || v.contains_key(&after_id);
            let reset = touched.then(|| {
                let mut r = set_value("reset", Text::new("Undo"), &page_params, &id, Value::Null);
                r.params["also"] = json!([{"valueParam": after_id, "value": null}]);
                r
            });
            Some(StudioKeyRow {
                action: b.action.clone(),
                title: Text::verbatim(&title),
                key,
                after,
                changed,
                reset,
                does: b.does.clone(),
                conflict: conflicts.get(&(b.module.clone(), b.action.clone())).cloned(),
                own: a.own,
            })
        };
        let modeless: Vec<StudioKeyRow> =
            bindings.iter().filter(|b| b.modes.is_empty()).filter_map(|b| row(b)).collect();
        if !modeless.is_empty() {
            let title = if m.modes.is_empty() { Text::new("In the program") } else { Text::new("In every mode") };
            groups.push(StudioGroup { title, start: false, way: vec![], rows: modeless });
        }
        let mut order: Vec<&String> = m.modes.keys().collect();
        order.sort_by_key(|id| {
            (found.get(*id).map(|w| w.len()).unwrap_or(usize::MAX), mode_title(&m.modes, id, lang).text.to_lowercase())
        });
        for id in order {
            let rows: Vec<StudioKeyRow> =
                bindings.iter().filter(|b| b.modes.contains(id)).filter_map(|b| row(b)).collect();
            if rows.is_empty() {
                continue;
            }
            groups.push(StudioGroup {
                title: mode_title(&m.modes, id, lang),
                start: Some(id) == start.as_ref(),
                way: found.get(id).map(|w| w.iter().map(|(k, _)| caps(k, ctx.host)).collect()).unwrap_or_default(),
                rows,
            });
        }
    }

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
    let mut module_field =
        Field::new("module", FieldKind::Choice, Text::new("Module"), json!(module_ref)).value_param("module");
    module_field.choices = modules
        .iter()
        .map(|m| {
            let n = keys.get(&m.reference()).map(|a| a.len()).unwrap_or(0);
            let label = match (m.actions.len(), n) {
                (1, 0) => tf!("%@ – 1 key", &m.title),
                (all, 0) => tf!("%@ – %lld keys", &m.title, all),
                (all, n) => tf!("%@ – %lld keys, %lld changed", &m.title, all, n),
            };
            Choice::new(json!(m.reference()), label)
        })
        .collect();
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
            "Key bindings of your own: change a key, or where a key leads afterwards – one module at a time, by the mode of its program. What overlaps is said while you choose. Only what differs from the modules' own keys is saved, in My Recipes' folder; recipes name the set my/<name>.",
        ),
        about,
        module: module_field,
        search: search_field,
        hints,
        groups,
        changed: tf!("%lld keys differ from the modules' own", changed_count),
        actions,
    }
}
