// SPDX-License-Identifier: AGPL-3.0-or-later
//! The recipe editor (`recipeEditor`, plan B141, B142): a recipe of your own put together from building blocks – a
//! platform, one switch per module (those here and those your sources offer), a look, up to two key binding sets, what a
//! VM needs at least and how it runs – with what is wrong said while you choose. Saved into My Recipes
//! (`my_recipes::MyRecipes`). Opened empty ("New Recipe …"), on one of yours ("Edit …") or on any other ("Copy to My
//! Recipes …"). The shape of the credential form (`CredentialPage`), so every UI draws it as it draws that.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde_json::{json, Map, Value};

use super::common::*;
use super::credentials::CredentialPage;
use super::text::{tf, Text};
use super::Ctx;
use crate::api::types::PartDto;
use crate::composition::{Composition, Issue, PartRef, RecipeSpec, Requirements};
use crate::models::GuestKind;
use crate::my_recipes::MyRecipes;

#[derive(Debug, Default, Deserialize)]
pub struct EditorParams {
    /// The recipe it starts from: one of yours is edited, any other copied
    #[serde(default)]
    pub id: Option<String>,
    /// The form's values: name, platform, description, look, keymap1, keymap2, cpus, memoryGB, diskGB, display, network,
    /// share, ephemeral, and "module:<author>/<name>" → on/off
    #[serde(default)]
    pub values: Map<String, Value>,
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

pub(crate) fn page(ctx: &Ctx, p: &EditorParams) -> Option<CredentialPage> {
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

    let name = text("name").unwrap_or_else(|| from.and_then(|r| r.name.clone()).unwrap_or_default());
    let platform = text("platform")
        .and_then(|k| GuestKind::parse(&k))
        .or_else(|| from.map(|r| r.kind))
        .unwrap_or(GuestKind::Nixos);
    let description =
        text("description").unwrap_or_else(|| from.and_then(|r| r.description.clone()).unwrap_or_default());
    let start_modules: Vec<String> =
        parts.map(|x| x.modules.iter().map(|m| bare(&m.reference)).collect()).unwrap_or_default();
    let all: Vec<&PartDto> = ctx.modules.parts.iter().chain(ctx.modules.offered.iter()).collect();
    let mut modules: Vec<&PartDto> = all
        .iter()
        .copied()
        .filter(|m| m.kind == "module" && m.platform == Some(platform))
        .fold(Vec::new(), |mut acc: Vec<&PartDto>, m| {
            if !acc.iter().any(|x| x.reference == m.reference) {
                acc.push(m);
            }
            acc
        });
    modules.sort_by_key(|m| group(m));
    let chosen: Vec<String> = modules
        .iter()
        .filter(|m| flag(&format!("module:{}", m.reference), start_modules.contains(&m.reference)))
        .map(|m| m.reference.clone())
        .collect();
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

    // The name: a new one, or the one being edited; a copy keeps the name (it is "my/…")
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
    let mut platform_field = Field::new("platform", FieldKind::Choice, Text::new("Platform"), json!(platform.as_str()))
        .param("platform")
        .value_param("platform")
        .help(Text::new("What the guest is: NixOS, or macOS on a macOS base"));
    platform_field.choices = vec![
        Choice::new(json!("nixos"), Text::verbatim("NixOS")),
        Choice::new(json!("macos"), Text::verbatim("macOS")),
    ];
    let mut fields = vec![
        name_field.clone(),
        platform_field,
        Field::new("description", FieldKind::Text, Text::new("Description"), json!(description))
            .param("description")
            .value_param("description"),
    ];
    // One switch per module
    for m in &modules {
        let key = format!("module:{}", m.reference);
        let mut label = m.title.clone();
        if !m.provides.is_empty() {
            label = format!("{label} ({})", m.provides.join(", "));
        }
        let mut f = Field::new(&key, FieldKind::Toggle, Text::verbatim(label), json!(chosen.contains(&m.reference)))
            .value_param(&key);
        f.help = Some(if m.installed || ctx.modules.parts.iter().any(|x| x.id == m.id) {
            Text::verbatim(&m.description)
        } else {
            tf!("%@ – from %@, installed with the recipe", &m.description, &m.source_name)
        });
        fields.push(f);
    }
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
    let mut look_field = Field::new("look", FieldKind::Choice, Text::new("Look"), json!(look))
        .param("look")
        .value_param("look")
        .help(Text::new("Suggested – the build may choose another"));
    look_field.choices = vec![Choice::new(json!(""), Text::new("none"))];
    for l in &ctx.looks.looks {
        look_field.choices.push(Choice::new(json!(l.reference), Text::verbatim(&l.title)));
    }
    for l in &ctx.looks.offered {
        if !ctx.looks.looks.iter().any(|x| x.reference == l.reference) {
            look_field.choices.push(Choice::new(json!(l.reference), Text::verbatim(&l.title)));
        }
    }
    fields.push(look_field);
    for (i, value) in [(1usize, &keymap1), (2usize, &keymap2)] {
        let id = format!("keymap{i}");
        let mut f = Field::new(&id, FieldKind::Choice, tf!("Key bindings %lld", i), json!(value))
            .param(&id)
            .value_param(&id)
            .help(Text::new(if i == 1 {
                "Without any, every module has its own default keys"
            } else {
                "Wins over the first for the same action"
            }));
        f.choices = named("keymap");
        fields.push(f);
    }
    // What a VM needs at least and how it runs
    fields.push(
        Field::new("cpus", FieldKind::Number, tf!("At least %lld CPU cores", cpus), json!(cpus))
            .param("cpus")
            .value_param("cpus")
            .range(1, 16, 1),
    );
    fields.push(
        Field::new("memoryGB", FieldKind::Number, tf!("At least %lld GB RAM", memory), json!(memory))
            .param("memoryGB")
            .value_param("memoryGB")
            .range(1, 64, 1),
    );
    fields.push(
        Field::new("diskGB", FieldKind::Number, tf!("At least a %lld GB disk", disk), json!(disk))
            .param("diskGB")
            .value_param("diskGB")
            .range(10, 500, 10),
    );
    let mut display_field = Field::new("display", FieldKind::Choice, Text::new("Display"), json!(display))
        .param("display")
        .value_param("display");
    display_field.choices = ["gpu", "window", "none"]
        .iter()
        .filter(|d| platform == GuestKind::Nixos || **d != "gpu")
        .map(|d| Choice::new(json!(d), Text::verbatim(*d)))
        .collect();
    fields.push(display_field);
    let mut network_field = Field::new("network", FieldKind::Choice, Text::new("Network"), json!(network))
        .param("network")
        .value_param("network");
    network_field.choices = ["shared", "isolated", "none"]
        .iter()
        .filter(|n| platform == GuestKind::Nixos || **n != "isolated")
        .map(|n| Choice::new(json!(n), Text::verbatim(*n)))
        .collect();
    fields.push(network_field);
    fields.push(
        Field::new("share", FieldKind::Toggle, Text::new("Share a folder"), json!(share))
            .param("share")
            .value_param("share"),
    );
    fields.push(
        Field::new(
            "ephemeral",
            FieldKind::Toggle,
            Text::new("Ephemeral (changes are discarded on shutdown)"),
            json!(ephemeral),
        )
        .param("ephemeral")
        .value_param("ephemeral"),
    );

    // What is wrong, said while choosing: the modules here put together (missing ones are installed with it)
    let spec = RecipeSpec {
        platform,
        modules: chosen.iter().map(|r| PartRef::new(r)).collect(),
        look: (!look.is_empty()).then(|| PartRef::new(&look)),
        keymaps: [&keymap1, &keymap2].into_iter().filter(|k| !k.is_empty()).map(|k| PartRef::new(k)).collect(),
        requirements: Requirements { cpus, memory_gb: memory, disk_gb: disk },
        locales: Default::default(),
        data: Value::Null,
    };
    let settings = crate::paths::Settings::load(&ctx.paths);
    let store = crate::modules::ModuleStore::new(ctx.paths.clone());
    let composition = Composition::of(
        &spec,
        &store.list(&settings),
        &crate::keymaps::KeymapStore::new(ctx.paths.clone()).list(&settings),
        &store.capabilities(&settings),
    );
    let check: Vec<Text> = composition
        .issues
        .iter()
        .filter_map(|i| match i {
            Issue::Needs { module, capability } => Some(tf!("%@ needs a %@", module, capability)),
            Issue::Incompatible { capability, modules } => {
                Some(tf!("%@ do not go together: each is the %@", modules.join(" + "), capability))
            }
            Issue::KeysOverlap { first, second } => {
                Some(tf!("Keys overlap: %@ and %@ (%@)", &first.title, &second.title, &first.key))
            }
            Issue::KeyRule { binding } => {
                Some(tf!("%@ (%@): without a mode a key starts with Super or Ctrl", &binding.title, &binding.key))
            }
            _ => None,
        })
        .collect();
    let mut check_field = Field::new(
        "check",
        FieldKind::Info,
        Text::new("Check"),
        json!(if check.is_empty() { Text::new("Nothing to warn about") } else { Text::joined(check) }),
    );
    check_field.help = Some(Text::new("Warnings only – the recipe can be saved and built"));
    fields.push(check_field);

    let mut vm = BTreeMap::new();
    vm.insert("display".to_string(), json!(display));
    vm.insert("network".to_string(), json!(network));
    vm.insert("share".to_string(), json!(share));
    vm.insert("ephemeral".to_string(), json!(ephemeral));
    let mut params = json!({
        "name": name, "platform": platform.as_str(), "description": description, "modules": chosen,
        "look": look, "keymaps": spec.keymaps.iter().map(|k| k.id.clone()).collect::<Vec<_>>(),
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
    Some(CredentialPage {
        title: Text::new(if own.is_some() {
            "Edit Recipe"
        } else if from.is_some() {
            "Copy to My Recipes"
        } else {
            "New Recipe"
        }),
        id: own,
        intro: Text::new(
            "A recipe of your own: what is in it (modules), how it looks, which keys it uses, what a VM from it needs. It is saved in My Recipes – a folder in the data folder that is one of your sources – and built like any other recipe.",
        ),
        fields,
        locked: None,
        actions,
    })
}
