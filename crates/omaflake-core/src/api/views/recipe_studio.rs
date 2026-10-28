// SPDX-License-Identifier: AGPL-3.0-or-later
//! The recipe studio (`recipeStudio`, plan B147; before it the recipe editor of B141/B142): a recipe of your own put
//! together like a kit. The catalog – the modules here and those your sources offer – is searched and filtered by
//! capability, tag and author; a module is added to the recipe or taken out of it with one click. The recipe says
//! what is wrong while you build it – a module that needs another (with the one to add), two that do not go together,
//! overlapping keys – and shows its keys before it is saved. Look, key binding sets, what a VM needs at least and how
//! it runs are fields as before. Saved into My Recipes (`my_recipes::MyRecipes`). Opened empty (the workshop's tab,
//! "New Recipe …"), on one of yours ("Edit …") or on any other ("Copy to My Recipes …").
//!
//! Everything chosen is a form value of the page (`values`), so every UI keeps it as it keeps a form's: `modules` is
//! the list of references, set as a whole by the add and remove buttons (`ui.field` with the new list).

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::common::*;
use super::text::{tf, Text};
use super::Ctx;
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
    /// The form's values: name, platform, description, modules (references), search, capability, tag, author, look,
    /// keymap1, keymap2, cpus, memoryGB, diskGB, display, network, share, ephemeral
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
    /// Look and key binding sets
    pub dress: Vec<Field>,
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StudioCatalog {
    pub title: Text,
    /// "12 of 24 modules"
    pub count: Text,
    pub search: Field,
    /// Capability, tag, author (choices; "" = any)
    pub filters: Vec<Field>,
    /// The modules that match and are not in the recipe yet
    pub rows: Vec<StudioModule>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Text>,
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
    let keymap = |i: usize| {
        text(&format!("keymap{}", i + 1))
            .unwrap_or_else(|| parts.and_then(|x| x.keymaps.get(i)).map(|k| bare(k)).unwrap_or_default())
    };
    let (keymap1, keymap2) = (keymap(0), keymap(1));
    let req = parts.map(|x| x.requirements).filter(|r| r.cpus > 0).unwrap_or(Requirements {
        cpus: 4,
        memory_gb: 8,
        disk_gb: 40,
    });
    let cpus = num("cpus", req.cpus);
    let memory = num("memoryGB", req.memory_gb);
    let disk = num("diskGB", req.disk_gb);
    let display = text("display").unwrap_or_else(|| {
        from.and_then(|r| r.display).map(|d| d.as_str().to_string()).unwrap_or_else(|| {
            if platform == GuestKind::Macos {
                "window".into()
            } else {
                "gpu".into()
            }
        })
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

    // The catalog: search and filters
    let search = text("search").unwrap_or_default().trim().to_lowercase();
    let capability = text("capability").unwrap_or_default();
    let tag = text("tag").unwrap_or_default();
    let author = text("author").unwrap_or_default();
    let any = || Choice::new(json!(""), Text::new("any"));
    let distinct =
        |f: &dyn Fn(&PartDto) -> Vec<String>| -> BTreeSet<String> { modules.iter().flat_map(|m| f(m)).collect() };
    let filter = |id: &str, label: &str, value: &str, values: BTreeSet<String>| {
        let mut choices = vec![any()];
        choices.extend(values.into_iter().map(|x| Choice::new(json!(x), Text::verbatim(x.clone()))));
        choice_field(id, Text::new(label), value, choices)
    };
    let filters = vec![
        filter("capability", "Capability", &capability, distinct(&|m| m.provides.clone())),
        filter("tag", "Tag", &tag, distinct(&|m| m.tags.clone())),
        filter("author", "Author", &author, distinct(&|m| vec![m.reference.split('/').next().unwrap_or("").into()])),
    ];
    let matches = |m: &PartDto| {
        let hay =
            format!("{} {} {} {} {}", m.title, m.description, m.reference, m.tags.join(" "), m.provides.join(" "))
                .to_lowercase();
        search.split_whitespace().all(|w| hay.contains(w))
            && (capability.is_empty() || m.provides.contains(&capability))
            && (tag.is_empty() || m.tags.contains(&tag))
            && (author.is_empty() || m.reference.starts_with(&format!("{author}/")))
    };
    let here = |m: &PartDto| ctx.modules.parts.iter().any(|x| x.id == m.id);
    let row = |m: &PartDto, inside: bool| {
        let mut sub = Vec::new();
        if !m.provides.is_empty() {
            sub.push(Text::verbatim(m.provides.join(", ")));
        }
        if !m.needs.is_empty() {
            sub.push(tf!("needs %@", m.needs.join(", ")));
        }
        match m.keys.len() {
            0 => {}
            1 => sub.push(Text::new("1 key")),
            n => sub.push(tf!("%lld keys", n)),
        }
        sub.push(Text::verbatim(&m.source_name));
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
        let action = if inside {
            let list: Vec<String> = chosen.iter().filter(|r| **r != m.reference).cloned().collect();
            set_modules("remove", Text::new("Remove"), "minus", &page_params, list)
        } else {
            let mut list = chosen.clone();
            list.push(m.reference.clone());
            set_modules("add", Text::new("Add"), "add", &page_params, list).primary()
        };
        StudioModule {
            reference: m.reference.clone(),
            title: m.title.clone(),
            description: m.description.clone(),
            subtitle: Text::joined(sub),
            badges,
            tags: m.tags.clone(),
            action,
        }
    };
    let offered: Vec<StudioModule> =
        modules.iter().filter(|m| !chosen.contains(&m.reference) && matches(m)).map(|m| row(m, false)).collect();
    let outside = modules.len() - chosen.len();
    let catalog = StudioCatalog {
        title: Text::new("Catalog"),
        count: tf!("%lld of %lld modules", offered.len(), outside),
        search: {
            let mut f =
                Field::new("search", FieldKind::Text, Text::new("Search"), json!(text("search").unwrap_or_default()))
                    .value_param("search");
            f.placeholder = Some(Text::new("Search modules: name, what it is, tags"));
            f
        },
        filters,
        empty: offered.is_empty().then(|| {
            Text::new(if outside == 0 {
                "Every module is in the recipe"
            } else {
                "No module matches – change the search"
            })
        }),
        rows: offered,
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
        keymaps: [&keymap1, &keymap2].into_iter().filter(|k| !k.is_empty()).map(|k| PartRef::new(k)).collect(),
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
        let text = match i {
            Issue::KeysOverlap { first, second } => {
                tf!("Keys overlap: %@ and %@ (%@)", &first.title, &second.title, first.key_text())
            }
            Issue::KeyRule { binding } => {
                tf!("%@ (%@): without a mode a key starts with Super or Ctrl", &binding.title, binding.key_text())
            }
            _ => continue,
        };
        hints.push(Hint::new("warning", text, Tone::Warning));
    }
    if inside.iter().any(|m| !here(m)) {
        hints.push(Hint::new(
            "info",
            Text::new("Modules from the catalog are installed with the recipe when it is saved; their keys show then."),
            Tone::Normal,
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
    let named = |kind: &str| -> Vec<Choice> {
        let mut out = vec![Choice::new(json!(""), Text::new("none"))];
        let mut seen = Vec::new();
        for p in ctx.modules.parts.iter().chain(ctx.modules.offered.iter()).filter(|p| p.kind == kind) {
            if !seen.contains(&p.reference) {
                seen.push(p.reference.clone());
                out.push(Choice::new(json!(p.reference), Text::verbatim(format!("{} – {}", p.title, p.reference))));
            }
        }
        out
    };
    let mut look_choices = vec![Choice::new(json!(""), Text::new("none"))];
    for l in &ctx.looks.looks {
        look_choices.push(Choice::new(json!(l.reference), Text::verbatim(&l.title)));
    }
    for l in &ctx.looks.offered {
        if !ctx.looks.looks.iter().any(|x| x.reference == l.reference) {
            look_choices.push(Choice::new(json!(l.reference), Text::verbatim(&l.title)));
        }
    }
    let mut dress = vec![choice_field("look", Text::new("Look"), &look, look_choices)
        .help(Text::new("Suggested – the build may choose another"))];
    for (i, value) in [(1usize, &keymap1), (2usize, &keymap2)] {
        let id = format!("keymap{i}");
        dress.push(choice_field(&id, tf!("Key bindings %lld", i), value, named("keymap")).help(Text::new(if i == 1 {
            "Without any, every module has its own default keys"
        } else {
            "Wins over the first for the same action"
        })));
    }

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
        vm: vm_fields,
        titles: StudioTitles {
            about: Text::new("Recipe"),
            dress: Text::new("Look and keys"),
            vm: Text::new("The VM"),
        },
        actions,
    })
}
