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

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value, json};

use super::Ctx;
use super::common::*;
use super::text::{Text, tf};
use crate::api::types::PartDto;
use crate::composition::{Composition, Issue, PartRef, RecipeSpec, Requirements};
use crate::models::GuestKind;
use crate::my_recipes::MyRecipes;

pub const PAGE: &str = "recipeStudio";

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

/// The modules of the catalog shown at once (plan B148)
pub const PAGE_SIZE: usize = 5;
/// The names suggested while typing
pub const SUGGESTIONS: usize = 6;

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

/// The order modules are offered in: by what they provide, then by title
fn group(p: &PartDto) -> (usize, String) {
    let order = ["desktop", "terminal", "shell", "multiplexer", "editor", "pdf-viewer"];
    let rank = p.provides.iter().filter_map(|c| order.iter().position(|o| o == c)).min().unwrap_or(order.len());
    (rank, p.title.to_lowercase())
}

/// "omaflake/latex ≥ 1.0.0" → "omaflake/latex"
fn bare(label: &str) -> String {
    label.split(' ').next().unwrap_or(label).to_string()
}

fn choice_field(id: &str, label: Text, value: &str, choices: Vec<Choice>) -> Field {
    let mut f = Field::new(id, FieldKind::Choice, label, json!(value)).param(id).value_param(id);
    f.choices = choices;
    f
}

/// Sets the recipe's modules to `list` (`ui.field` on this page)
fn set_modules(id: &str, label: Text, icon: &str, params: &Value, list: Vec<String>) -> Action {
    let mut a = Action::field(id, label, Some(icon), PAGE, params.clone(), "modules", "modules");
    a.params["value"] = json!(list);
    a
}

pub(crate) fn page(ctx: &Ctx, p: &StudioParams) -> Option<RecipeStudioPage> {
    let my = MyRecipes::new(ctx.paths.clone());
    let from = match &p.id {
        Some(id) => Some(ctx.recipes.recipes.iter().find(|r| &r.id == id)?),
        None => None,
    };
    let own = from.and_then(|r| r.folder.as_ref()).and_then(|f| my.base_of(std::path::Path::new(f)));
    let parts = from.and_then(|r| r.parts.as_ref());
    let v = &p.values;
    let text = |k: &str| v.get(k).and_then(Value::as_str).map(str::to_string);
    let num = |k: &str, fallback: u32| v.get(k).and_then(Value::as_u64).map(|n| n as u32).unwrap_or(fallback);
    let flag = |k: &str, fallback: bool| v.get(k).and_then(Value::as_bool).unwrap_or(fallback);
    // The page's own params (without its values): what its `ui.field` actions name
    let page_params = match &p.id {
        Some(id) => json!({"id": id}),
        None => json!({}),
    };

    let name = text("name").unwrap_or_else(|| from.and_then(|r| r.name.clone()).unwrap_or_default());
    let platform = text("platform")
        .and_then(|k| GuestKind::parse(&k))
        .or_else(|| from.map(|r| r.kind))
        .unwrap_or(GuestKind::Nixos);
    let description =
        text("description").unwrap_or_else(|| from.and_then(|r| r.description.clone()).unwrap_or_default());

    // The catalog: every module of the platform, here or offered (once each)
    let mut modules: Vec<&PartDto> = Vec::new();
    for m in ctx.modules.parts.iter().chain(ctx.modules.offered.iter()) {
        if m.kind == "module" && m.platform == Some(platform) && !modules.iter().any(|x| x.reference == m.reference) {
            modules.push(m);
        }
    }
    modules.sort_by_key(|m| group(m));
    let by_ref: BTreeMap<&str, &PartDto> = modules.iter().map(|m| (m.reference.as_str(), *m)).collect();
    let chosen: Vec<String> = match v.get("modules").and_then(Value::as_array) {
        Some(list) => list.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => parts.map(|x| x.modules.iter().map(|m| bare(&m.reference)).collect()).unwrap_or_default(),
    };
    let chosen: Vec<String> = chosen.into_iter().filter(|r| by_ref.contains_key(r.as_str())).collect();

    let look = text("look").unwrap_or_else(|| parts.and_then(|x| x.look.as_deref()).map(bare).unwrap_or_default());
    let keymap_refs: Vec<String> = match v.get("keymaps").and_then(Value::as_array) {
        Some(list) => list.iter().filter_map(Value::as_str).map(str::to_string).collect(),
        None => parts.map(|x| x.keymaps.iter().map(|k| bare(k)).collect()).unwrap_or_default(),
    };
    let req = parts.map(|x| x.requirements).filter(|r| r.cpus > 0).unwrap_or(Requirements {
        cpus: 4,
        memory_gb: 8,
        disk_gb: 40,
    });
    let cpus = num("cpus", req.cpus);
    let memory = num("memoryGB", req.memory_gb);
    let disk = num("diskGB", req.disk_gb);
    let display = text("display").unwrap_or_else(|| {
        from.and_then(|r| r.display)
            .map(|d| d.as_str().to_string())
            .unwrap_or_else(|| if platform == GuestKind::Macos { "window".into() } else { "gpu".into() })
    });
    let network =
        text("network").unwrap_or_else(|| from.map(|r| r.network.as_str().to_string()).unwrap_or("shared".into()));
    let share = flag("share", from.is_none_or(|r| r.share));
    let ephemeral = flag("ephemeral", from.is_some_and(|r| r.ephemeral));

    // About: the name (a new one, or the one being edited; a copy keeps the name – it is "my/…"), platform, description
    let editing = own.as_ref().map(|base| base.split('.').nth(2).unwrap_or_default().to_string());
    let taken = ctx.recipes.recipes.iter().any(|r| {
        r.base == format!("{}.{}.{}", MyRecipes::AUTHOR, platform.as_str(), name)
            && editing.as_deref() != Some(name.as_str())
    });
    let mut name_field = Field::new("name", FieldKind::Text, Text::new("Name"), json!(name))
        .param("name")
        .value_param("name")
        .help(Text::new("lowercase letters, digits and - – it is the guest's host name too"));
    if name.is_empty() || !crate::recipe_sources::RecipeRef::valid_name(&name) {
        name_field.error = Some(Text::new("A recipe's name is lowercase letters, digits and -"));
    } else if taken {
        name_field.error = Some(tf!("There is a recipe \"%@\" of yours already", &name));
    }
    let platform_field = choice_field(
        "platform",
        Text::new("Platform"),
        platform.as_str(),
        vec![
            Choice::new(json!("nixos"), Text::verbatim("NixOS")),
            Choice::new(json!("macos"), Text::verbatim("macOS")),
        ],
    )
    .help(Text::new("What the guest is: NixOS, or macOS on a macOS base"));
    let about = vec![
        name_field.clone(),
        platform_field,
        Field::new("description", FieldKind::Text, Text::new("Description"), json!(description))
            .param("description")
            .value_param("description"),
    ];

    // The catalog: the search, the chips (tags and capabilities), one page of what matches (plan B148)
    let query = text("search").unwrap_or_default();
    let search = query.trim().to_lowercase();
    let on: Vec<String> = v
        .get("tags")
        .and_then(Value::as_array)
        .map(|l| l.iter().filter_map(Value::as_str).map(str::to_string).collect())
        .unwrap_or_default();
    let labels_of = |m: &PartDto| -> BTreeSet<String> { m.tags.iter().chain(m.provides.iter()).cloned().collect() };
    let matches = |m: &PartDto| {
        let hay =
            format!("{} {} {} {} {}", m.title, m.description, m.reference, m.tags.join(" "), m.provides.join(" "))
                .to_lowercase();
        let labels = labels_of(m);
        search.split_whitespace().all(|w| hay.contains(w)) && on.iter().all(|t| labels.contains(t))
    };
    // Best first while searching: the title starts with it, then contains it, then the rest; else the kit's order
    let rank = |m: &PartDto| {
        let title = m.title.to_lowercase();
        if search.is_empty() || title.starts_with(&search) {
            0
        } else if title.contains(&search) {
            1
        } else {
            2
        }
    };
    let mut found: Vec<&PartDto> = modules.iter().copied().filter(|m| matches(m)).collect();
    found.sort_by_key(|m| rank(m));
    let here = |m: &PartDto| ctx.modules.parts.iter().any(|x| x.id == m.id);
    let add_list = |m: &PartDto| {
        let mut list = chosen.clone();
        list.push(m.reference.clone());
        list
    };
    let row = |m: &PartDto, inside: bool| {
        let mut sub = Vec::new();
        if !m.provides.is_empty() {
            sub.push(Text::verbatim(m.provides.join(", ")));
        }
        if !m.needs.is_empty() {
            sub.push(tf!("needs %@", m.needs.join(", ")));
        }
        let mut badges = Vec::new();
        if m.official {
            badges.push(Badge::new("official", "official", Text::new("Official – from omaflake"), Tone::Accent));
        }
        if !here(m) {
            badges.push(
                Badge::new("offered", "download", Text::new("Not here yet – installed with the recipe"), Tone::Normal)
                    .label(Text::new("from the catalog")),
            );
        }
        if m.unfree {
            badges.push(Badge::new("unfree", "unfree", Text::new("Not free software"), Tone::Warning));
        }
        let mut details = Vec::new();
        if !m.description.is_empty() {
            details.push(InfoLine::new("About", Text::verbatim(&m.description)));
        }
        if !m.provides.is_empty() {
            details.push(InfoLine::new("Provides", Text::verbatim(m.provides.join(", "))));
        }
        if !m.needs.is_empty() {
            details.push(InfoLine::new("Needs", Text::verbatim(m.needs.join(", "))));
        }
        if !m.enhances.is_empty() {
            details.push(InfoLine::new("Works with", Text::verbatim(m.enhances.join(", "))));
        }
        if !m.keys.is_empty() {
            details.push(InfoLine::new("Keys", tf!("%lld keys", m.keys.len())));
        }
        if !m.tags.is_empty() {
            details.push(InfoLine::new("Tags", Text::verbatim(m.tags.join(", "))));
        }
        details.push(InfoLine::new("Version", Text::verbatim(&m.version)));
        details.push(InfoLine::new("Source", Text::verbatim(&m.source_name)));
        let mut detail_actions = Vec::new();
        if here(m) && !m.keys.is_empty() {
            detail_actions.push(Action::show(
                "keys",
                Text::new("Keys …"),
                Some("keyboard"),
                "keys",
                json!({"module": m.reference, "platform": platform.as_str()}),
            ));
        }
        let action = if inside {
            let list: Vec<String> = chosen.iter().filter(|r| **r != m.reference).cloned().collect();
            set_modules("remove", Text::new("Remove"), "minus", &page_params, list)
        } else {
            set_modules("add", Text::new("Add"), "add", &page_params, add_list(m)).primary()
        };
        StudioModule {
            reference: m.reference.clone(),
            title: m.title.clone(),
            description: m.description.clone(),
            subtitle: Text::joined(sub),
            badges,
            tags: m.tags.clone(),
            inside,
            details,
            detail_actions,
            action,
        }
    };
    let pages = found.len().div_ceil(PAGE_SIZE).max(1);
    // The page belongs to the search it was turned with (`pageSearch`): another search starts on the first; a page
    // that is gone (fewer found) is the last one
    let same_search = v.get("pageSearch").and_then(Value::as_str).unwrap_or_default() == query;
    let index =
        v.get("page").and_then(Value::as_u64).filter(|_| same_search).map(|n| n as usize).unwrap_or(0).min(pages - 1);
    let go = |id: &str, label: &str, icon: &str, to: usize| {
        let mut a = Action::field(id, Text::new(label), Some(icon), PAGE, page_params.clone(), "page", "page");
        a.params["value"] = json!(to);
        a.params["also"] = json!([{"valueParam": "pageSearch", "value": query}]);
        a
    };
    let shown: Vec<StudioModule> =
        found.iter().skip(index * PAGE_SIZE).take(PAGE_SIZE).map(|m| row(m, chosen.contains(&m.reference))).collect();
    let first = index * PAGE_SIZE + 1;
    let pages_info = StudioPages {
        index,
        count: pages,
        text: if found.is_empty() {
            Text::verbatim("")
        } else {
            tf!("%lld–%lld of %lld", first, first + shown.len() - 1, found.len())
        },
        previous: (index > 0).then(|| go("previous", "Previous", "chevron-left", index - 1)),
        next: (index + 1 < pages).then(|| go("next", "Next", "chevron-right", index + 1)),
    };
    let suggest: Vec<StudioSuggestion> = if search.is_empty() {
        vec![]
    } else {
        found
            .iter()
            .take(SUGGESTIONS)
            .map(|m| {
                let inside = chosen.contains(&m.reference);
                StudioSuggestion {
                    reference: m.reference.clone(),
                    title: m.title.clone(),
                    subtitle: Text::verbatim(m.provides.join(", ")),
                    inside,
                    action: (!inside).then(|| {
                        let mut a = set_modules("add", tf!("Add %@", &m.title), "add", &page_params, add_list(m));
                        // Added: the search is done
                        a.params["also"] =
                            json!([{"valueParam": "search", "value": null}, {"valueParam": "page", "value": null}]);
                        a
                    }),
                }
            })
            .collect()
    };
    // The chips: every tag and capability of the platform's modules, the most common first
    let mut counts: BTreeMap<String, usize> = BTreeMap::new();
    for m in &modules {
        for l in labels_of(m) {
            *counts.entry(l).or_default() += 1;
        }
    }
    let mut chips: Vec<(String, usize)> = counts.into_iter().collect();
    chips.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let tags = chips
        .into_iter()
        .map(|(id, count)| {
            let is_on = on.contains(&id);
            let list: Vec<String> = if is_on {
                on.iter().filter(|t| **t != id).cloned().collect()
            } else {
                [on.clone(), vec![id.clone()]].concat()
            };
            let mut a = Action::field("tag", Text::verbatim(&id), None, PAGE, page_params.clone(), "tags", "tags");
            a.params["value"] = json!(list);
            a.params["also"] = json!([{"valueParam": "page", "value": null}]);
            StudioTag { label: Text::verbatim(&id), id, count, on: is_on, action: a.switch(is_on) }
        })
        .collect();
    let filtered = !search.is_empty() || !on.is_empty();
    let catalog = StudioCatalog {
        title: Text::new("Find modules"),
        count: if filtered {
            tf!("%lld of %lld modules", found.len(), modules.len())
        } else {
            tf!("%lld modules", modules.len())
        },
        search: {
            let mut f = Field::new("search", FieldKind::Text, Text::new("Search"), json!(query)).value_param("search");
            f.placeholder = Some(Text::new("Search modules: name, what it is, tags"));
            f
        },
        suggest,
        tags,
        rows: shown,
        pages: pages_info,
        empty: found.is_empty().then(|| Text::new("No module matches – change the search or the tags")),
        reset: filtered.then(|| {
            let mut a = Action::field(
                "reset",
                Text::new("Show all"),
                Some("refresh"),
                PAGE,
                page_params.clone(),
                "search",
                "search",
            );
            a.params["value"] = Value::Null;
            a.params["also"] = json!([{"valueParam": "tags", "value": null}, {"valueParam": "page", "value": null}]);
            a
        }),
    };

    // The recipe: its modules in the catalog's order, what is wrong while it is built
    let inside: Vec<&PartDto> = modules.iter().copied().filter(|m| chosen.contains(&m.reference)).collect();
    let mut hints = Vec::new();
    for m in &inside {
        for need in &m.needs {
            if inside.iter().any(|x| x.provides.contains(need)) {
                continue;
            }
            let mut h = Hint::new("warning", tf!("%@ needs a %@", &m.title, need), Tone::Warning);
            // The one to add: here before offered, official first
            let mut candidates: Vec<&&PartDto> = modules.iter().filter(|x| x.provides.contains(need)).collect();
            candidates.sort_by_key(|x| (!here(x), !x.official, x.title.to_lowercase()));
            if let Some(c) = candidates.first() {
                let mut list = chosen.clone();
                list.push(c.reference.clone());
                h = h.action(set_modules("addNeeded", tf!("Add %@", &c.title), "add", &page_params, list).primary());
            }
            hints.push(h);
        }
    }
    let settings = crate::paths::Settings::load(&ctx.paths);
    let store = crate::modules::ModuleStore::new(ctx.paths.clone());
    let capabilities = store.capabilities(&settings);
    for (cap, c) in &capabilities {
        let both: Vec<&str> = inside.iter().filter(|m| m.provides.contains(cap)).map(|m| m.title.as_str()).collect();
        if c.exclusive && both.len() > 1 {
            hints.push(Hint::new(
                "warning",
                tf!("%@ do not go together: each is the %@", both.join(" + "), cap),
                Tone::Warning,
            ));
        }
    }
    let spec = RecipeSpec {
        platform,
        modules: chosen.iter().map(|r| PartRef::new(r)).collect(),
        look: (!look.is_empty()).then(|| PartRef::new(&look)),
        keymaps: keymap_refs.iter().map(|k| PartRef::new(k)).collect(),
        requirements: Requirements { cpus, memory_gb: memory, disk_gb: disk },
        locales: Default::default(),
        data: Value::Null,
    };
    let composition = Composition::of(
        &spec,
        &store.list(&settings),
        &crate::keymaps::KeymapStore::new(ctx.paths.clone()).list(&settings),
        &capabilities,
    );
    for i in &composition.issues {
        let (text, keys) = match i {
            Issue::KeysOverlap { first, second } => {
                (tf!("Keys overlap: %@ and %@", &first.title, &second.title), &first.keys)
            }
            Issue::KeyRule { binding } => {
                (tf!("%@: without a mode a key starts with Super or Ctrl", &binding.title), &binding.keys)
            }
            _ => continue,
        };
        hints.push(Hint::new("warning", text, Tone::Warning).keys(super::keys::key_caps(keys, ctx.host)));
    }
    if inside.iter().any(|m| !here(m)) {
        hints.push(Hint::help(
            "info",
            Text::new("Modules from the catalog are installed with the recipe when it is saved; their keys show then."),
        ));
    }
    let keymaps: Vec<String> = spec.keymaps.iter().map(|k| k.id.clone()).collect();
    let keys = (!composition.bindings.is_empty()).then(|| {
        Action::show(
            "keys",
            Text::new("Keys …"),
            Some("keyboard"),
            "keys",
            json!({"draft": {"platform": platform.as_str(), "modules": chosen, "keymaps": keymaps, "name": name}}),
        )
    });
    let recipe = StudioRecipe {
        title: tf!("Your recipe – %lld modules", inside.len()),
        rows: inside.iter().map(|m| row(m, true)).collect(),
        hints,
        empty: inside
            .is_empty()
            .then(|| Text::new("Add modules from the catalog – a desktop, a terminal, an editor …")),
        keys,
    };

    // Look and key bindings: those here and those offered
    // Every key binding set here or offered, once each: (reference, title)
    let mut sets: Vec<(String, String)> = Vec::new();
    for p in ctx.modules.parts.iter().chain(ctx.modules.offered.iter()).filter(|p| p.kind == "keymap") {
        if !sets.iter().any(|(r, _)| *r == p.reference) {
            sets.push((p.reference.clone(), p.title.clone()));
        }
    }
    let mut look_choices = vec![Choice::new(json!(""), Text::new("none"))];
    for l in &ctx.looks.looks {
        look_choices.push(Choice::new(json!(l.reference), Text::verbatim(&l.title)));
    }
    for l in &ctx.looks.offered {
        if !ctx.looks.looks.iter().any(|x| x.reference == l.reference) {
            look_choices.push(Choice::new(json!(l.reference), Text::verbatim(&l.title)));
        }
    }
    let dress = vec![
        choice_field("look", Text::new("Look"), &look, look_choices)
            .help(Text::new("Suggested – the build may choose another")),
    ];
    // The key binding sets: each action sets the whole list (`ui.field` on `values.keymaps`)
    let set_keymaps = |id: &str, label: Text, icon: &str, list: Vec<String>| {
        let mut a = Action::field(id, label, Some(icon), PAGE, page_params.clone(), "keymaps", "keymaps");
        a.params["value"] = json!(list);
        a
    };
    let n = keymap_refs.len();
    let keymaps_section = StudioKeymaps {
        title: Text::new("Key bindings"),
        help: Text::new(
            "Key binding sets give the modules' actions their keys; a later one wins for the same action. Without any, every module has its own keys.",
        ),
        rows: keymap_refs
            .iter()
            .enumerate()
            .map(|(i, r)| {
                let title = sets.iter().find(|(x, _)| x == r).map(|(_, t)| t.clone()).unwrap_or_else(|| r.clone());
                let moved = |to: usize| {
                    let mut l = keymap_refs.clone();
                    l.swap(i, to);
                    l
                };
                StudioKeymap {
                    reference: r.clone(),
                    title,
                    subtitle: if n == 1 {
                        Text::verbatim(r)
                    } else if i + 1 == n {
                        Text::joined(vec![Text::verbatim(r), Text::new("wins over the ones above")])
                    } else {
                        Text::verbatim(r)
                    },
                    remove: set_keymaps(
                        "remove",
                        Text::new("Remove"),
                        "minus",
                        keymap_refs.iter().filter(|x| *x != r).cloned().collect(),
                    ),
                    up: (i > 0).then(|| set_keymaps("up", Text::new("Earlier"), "up", moved(i - 1))),
                    down: (i + 1 < n).then(|| set_keymaps("down", Text::new("Later"), "down", moved(i + 1))),
                }
            })
            .collect(),
        add: sets
            .iter()
            .filter(|(r, _)| !keymap_refs.contains(r))
            .map(|(r, t)| {
                let mut l = keymap_refs.clone();
                l.push(r.clone());
                set_keymaps("add", Text::verbatim(format!("{t} – {r}")), "add", l)
            })
            .collect(),
        empty: keymap_refs.is_empty().then(|| Text::new("None – every module has its own keys")),
    };

    // What a VM needs at least and how it runs
    let number = |id: &str, label: Text, value: u32, (min, max, step): (i64, i64, i64)| {
        Field::new(id, FieldKind::Number, label, json!(value)).param(id).value_param(id).range(min, max, step)
    };
    let mut vm_fields = vec![
        number("cpus", tf!("At least %lld CPU cores", cpus), cpus, (1, 16, 1)),
        number("memoryGB", tf!("At least %lld GB RAM", memory), memory, (1, 64, 1)),
        number("diskGB", tf!("At least a %lld GB disk", disk), disk, (10, 500, 10)),
    ];
    let options = |list: &[&'static str]| list.iter().map(|d| Choice::new(json!(d), Text::verbatim(*d))).collect();
    let displays: Vec<&str> =
        ["gpu", "window", "none"].into_iter().filter(|d| platform == GuestKind::Nixos || *d != "gpu").collect();
    let networks: Vec<&str> = ["shared", "isolated", "none"]
        .into_iter()
        .filter(|n| platform == GuestKind::Nixos || *n != "isolated")
        .collect();
    vm_fields.push(choice_field("display", Text::new("Display"), &display, options(&displays)));
    vm_fields.push(choice_field("network", Text::new("Network"), &network, options(&networks)));
    vm_fields.push(
        Field::new("share", FieldKind::Toggle, Text::new("Share a folder"), json!(share))
            .param("share")
            .value_param("share"),
    );
    vm_fields.push(
        Field::new(
            "ephemeral",
            FieldKind::Toggle,
            Text::new("Ephemeral (changes are discarded on shutdown)"),
            json!(ephemeral),
        )
        .param("ephemeral")
        .value_param("ephemeral"),
    );

    let mut vm = BTreeMap::new();
    vm.insert("display".to_string(), json!(display));
    vm.insert("network".to_string(), json!(network));
    vm.insert("share".to_string(), json!(share));
    vm.insert("ephemeral".to_string(), json!(ephemeral));
    let mut params = json!({
        "name": name, "platform": platform.as_str(), "description": description, "modules": chosen,
        "look": look, "keymaps": keymaps,
        "requirements": {"cpus": cpus, "memory_gb": memory, "disk_gb": disk}, "vm": vm,
    });
    if let Some(base) = &own {
        params["previous"] = json!(base);
    }
    let error = name_field.error.clone().or_else(|| chosen.is_empty().then(|| Text::new("Choose at least one module")));
    let save = Action::call("save", Text::new("Save"), Some("apply"), "recipes.save", params)
        .then(Then::Back)
        .enabled_if(error.is_none(), error)
        .primary();
    let mut actions = vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({}))];
    if let Some(base) = &own {
        actions.push(
            Action::call("delete", Text::new("Delete …"), Some("trash"), "recipes.delete", json!({"base": base}))
                .destructive()
                .then(Then::Back)
                .confirm(
                    Confirm::new(tf!("Delete the recipe “%@”?", &name), Text::new("Images built from it stay."))
                        .button("Delete")
                        .destructive(),
                ),
        );
    }
    actions.push(save);
    Some(RecipeStudioPage {
        title: Text::new(if own.is_some() {
            "Edit Recipe"
        } else if from.is_some() {
            "Copy to My Recipes"
        } else {
            "Recipe Studio"
        }),
        id: own,
        intro: Text::new(
            "Put a recipe of your own together: find modules in the catalog and add them – what is missing or does not fit is said while you build. Saved in My Recipes, a folder in the data folder that is one of your sources, and built like any other recipe.",
        ),
        about,
        catalog,
        recipe,
        dress,
        keymaps: keymaps_section,
        vm: vm_fields,
        titles: StudioTitles { about: Text::new("Recipe"), dress: Text::new("Look"), vm: Text::new("The VM") },
        actions,
    })
}
