// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Recipes tab (`recipes`): one row per recipe line with its versions (plan B111), what the remote sources offer
//! (plan B107), newer versions there, the suggestion at the first start – the app's `RecipeRowView`,
//! `RecipeVersionRow`, `OfferedRecipeRow` and `RecipeUpdates`. Lab recipes only in developer mode.

use std::collections::{BTreeMap, BTreeSet};

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::*;
use super::text::{tf, Arg, Text};
use super::vms::{kind_badge, unreadable};
use super::Ctx;
use crate::api::types::{CatalogRecipeDto, RecipeDto, RecipePartsDto, VersionDto};
use crate::composition::Issue;
use crate::models::RecipeId;
use crate::recipe_sources::RecipeSourceKind;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipesPage {
    #[serde(flatten)]
    pub head: ListHead,
    /// The remote catalogs were looked into this session (else the UI runs the `refresh` action when the tab opens)
    pub catalogs_loaded: bool,
    /// They are being looked into now
    pub loading: bool,
    /// What a UI says while `loading` ("Looking into the recipe sources …", next to a spinner)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loading_text: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub updates: Option<UpdatesSection>,
    /// The first start: omaflake ships no recipes – the official ones are suggested
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggestion: Option<Hint>,
    pub rows: Vec<RecipeRowView>,
    /// The heading over `offered` (while there are offers)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offered_title: Option<Text>,
    /// Not installed, offered by a remote source
    pub offered: Vec<OfferedRow>,
    pub unreadable: Vec<Hint>,
    /// Sources that could not be read
    pub errors: Vec<Hint>,
    /// Lab recipes are left out (developer mode is off)
    pub lab_hidden: bool,
    /// Said under the list while `labHidden`, with the way to Settings
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lab_hint: Option<Hint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeRowView {
    pub id: String,
    pub line: String,
    pub name: String,
    pub lamp: Lamp,
    pub kind: Badge,
    /// Its author when not omaflake, its distinction (plan B110) – after the name
    pub secondary: Vec<String>,
    /// Lab, other architectures
    pub badges: Vec<Badge>,
    pub tags: Vec<String>,
    /// Show in the file manager; build (opens the build page) or the Doctor's warning
    pub actions: Vec<Action>,
    pub detail: RecipeDetail,
    pub recipe: RecipeDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeDetail {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub info: Vec<InfoLine>,
    /// What is missing (with "Install Missing Parts"), what is wrong, unfree software (plan B142)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub notes: Vec<Hint>,
    pub versions: Vec<VersionRow>,
    /// Update (when its source has a newer version) on the left, remove all versions on the right
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionRow {
    pub id: String,
    /// As many words as tell the versions apart; the whole id is `id`
    pub words: String,
    /// "as it is in the folder" / added · newest · images: 2
    pub state: Text,
    pub actions: Vec<Action>,
    pub version: VersionDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferedRow {
    pub id: String,
    pub name: String,
    pub author: String,
    pub lamp: Lamp,
    pub kind: Badge,
    /// Official, lab, other architectures
    pub badges: Vec<Badge>,
    pub tags: Vec<String>,
    pub actions: Vec<Action>,
    pub detail: OfferedDetail,
    pub recipe: CatalogRecipeDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct OfferedDetail {
    pub description: String,
    pub info: Vec<InfoLine>,
    /// "Not reviewed by the omaflake team …" for recipes not from omaflake/recipes
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub warning: Option<Text>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdatesSection {
    pub title: Text,
    /// "Update All" / "Add All" when there are several
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub all: Option<Action>,
    pub rows: Vec<UpdateRow>,
    pub note: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateRow {
    pub id: String,
    pub name: String,
    pub kind: Badge,
    pub source: String,
    pub action: Action,
}

#[derive(Debug, Default, Deserialize)]
pub struct RecipesParams {
    #[serde(default)]
    pub filter: FilterParams,
}

/// One row per line: its newest version names it; built ones first, each kind by name (`AppModel.recipeRows`)
pub fn recipe_rows(recipes: &[RecipeDto]) -> Vec<&RecipeDto> {
    let mut lines: BTreeMap<&str, Vec<&RecipeDto>> = BTreeMap::new();
    for r in recipes {
        lines.entry(r.line.as_str()).or_default().push(r);
    }
    let mut rows: Vec<&RecipeDto> = lines
        .values()
        .filter_map(|versions| {
            versions.iter().find(|r| r.versions.first().is_some_and(|v| v.id == r.id)).or(versions.first()).copied()
        })
        .collect();
    rows.sort_by(|a, b| {
        (a.image.is_none(), a.kind, &a.display_name, &a.id).cmp(&(b.image.is_none(), b.kind, &b.display_name, &b.id))
    });
    rows
}

pub(crate) fn page(ctx: &Ctx, p: &RecipesParams) -> RecipesPage {
    let f = &p.filter;
    let all = recipe_rows(&ctx.recipes.recipes);
    let tag_ok = |tags: &[String]| f.tag.as_ref().is_none_or(|t| tags.contains(t));
    let rows: Vec<RecipeRowView> = all
        .iter()
        .filter(|r| {
            (f.other_systems || r.runs_here)
                && f.status.is_none_or(|s| s == r.image.is_some())
                && f.kind.is_none_or(|k| k == r.kind)
                && tag_ok(&r.tags)
        })
        .map(|r| row(ctx, r))
        .collect();
    let offered_all: Vec<&CatalogRecipeDto> =
        ctx.available.as_ref().map(|a| a.offered.iter().collect()).unwrap_or_default();
    let offered: Vec<OfferedRow> = offered_all
        .iter()
        .filter(|r| {
            (f.other_systems || r.runs_here)
                && f.status != Some(true)
                && f.kind.is_none_or(|k| k == r.kind)
                && tag_ok(&r.tags)
        })
        .map(|r| offered_row(ctx, r))
        .collect();
    let tags: BTreeSet<&String> = all.iter().flat_map(|r| r.tags.iter()).collect();
    let mut groups = f.base_groups("With image", "Without image");
    if !tags.is_empty() {
        let mut options = vec![Choice::new(serde_json::Value::Null, Text::new("All"))];
        options.extend(tags.iter().map(|t| Choice::new(json!(t), Text::verbatim(*t))));
        groups.push(FilterGroup {
            id: "tag".into(),
            label: Text::new("Tag"),
            kind: FilterKind::Choice,
            value: json!(f.tag),
            options,
        });
    }
    groups.push(FilterGroup {
        id: "otherSystems".into(),
        label: Text::new("Other Architectures"),
        kind: FilterKind::Toggle,
        value: json!(f.other_systems),
        options: vec![],
    });
    let mut head = ListHead::new("Recipes", rows.len(), all.len());
    head.filter = Some(Filter { active: f.is_active(), groups });
    head.actions = vec![
        Action::call("refresh", Text::new("Refresh"), Some("refresh"), "recipes.available", json!({}))
            .help(Text::new("Look for new recipes in the sources"))
            .busy(ctx.fetching)
            .enabled_if(!ctx.fetching, None),
        Action::call(
            "import",
            Text::new("Import"),
            Some("import"),
            "ui.step",
            json!({"page": "import", "params": {"expect": "recipe"}}),
        )
        .help(Text::new("Import recipes – a copy into your recipes (sources to use as they are: Settings)"))
        .ask(Ask::packages(
            "recipe",
            false,
            "Recipes (.omaflake-recipe) or a folder of recipes – copied into your recipes",
            "Import",
        )),
        Action::show("newRecipe", Text::new("New Recipe …"), Some("add"), "recipeStudio", json!({}))
            .help(Text::new("Put a recipe together from modules, a look and key bindings")),
    ];
    // Recipes being downloaded are left out (their Install is greyed out)
    let shown = offered.iter().map(|r| (r.actions.as_slice(), r.badges.iter().any(|b| b.id == "official")));
    head.actions.extend(install_all("recipes.installAll", shown, false));
    head.empty = if all.is_empty() && offered.is_empty() {
        Some(Hint::new(
            "recipe",
            Text::new(if ctx.fetching {
                "Looking into the recipe sources …"
            } else {
                "No recipes – add a recipe source in Settings."
            }),
            Tone::Normal,
        ))
    } else if rows.is_empty() && offered.is_empty() {
        Some(filtered_out("recipes"))
    } else {
        None
    };
    let suggestion = ctx.available.as_ref().filter(|a| all.is_empty() && !a.suggested.is_empty()).map(|_| {
        Hint::new("official", Text::new("Start with the official recipes?"), Tone::Accent).action(
            Action::call(
                "installSuggested",
                Text::new("Install suggested"),
                Some("install"),
                "recipes.installSuggested",
                json!({}),
            )
            .primary(),
        )
    });
    let errors = ctx
        .available
        .as_ref()
        .map(|a| {
            a.errors
                .iter()
                .map(|(id, e)| {
                    Hint::new("warning", Text::verbatim(format!("{}: {e}", ctx.source_name(id))), Tone::Warning)
                })
                .collect()
        })
        .unwrap_or_default();
    let unreadable = unreadable(&ctx.recipes.unreadable)
        .into_iter()
        .zip(&ctx.recipes.unreadable)
        .map(|(mut h, u)| {
            h.text = Text::verbatim(format!("{}: {}", RecipeId::qualified(&u.name), u.reason));
            h
        })
        .collect();
    let lab_hidden = ctx.recipes.lab_hidden;
    RecipesPage {
        head,
        catalogs_loaded: ctx.available.is_some(),
        loading: ctx.fetching,
        loading_text: ctx.fetching.then(|| Text::new("Looking into the recipe sources …")),
        updates: updates(ctx),
        suggestion,
        rows,
        offered_title: (!offered.is_empty()).then(|| Text::new("More recipes in your sources")),
        offered,
        unreadable,
        errors,
        lab_hidden,
        lab_hint: lab_hidden.then(|| {
            Hint::new("lab", Text::new("Lab recipes are hidden – Developer Mode in Settings shows them."), Tone::Normal)
                .action(Action::show("settings", Text::new("Settings"), Some("settings"), "settings", json!({})))
        }),
    }
}

fn arch_badge(ctx: &Ctx, systems: &[String]) -> Badge {
    let arch: Vec<&str> = systems.iter().map(|s| s.split('-').next().unwrap_or(s)).collect();
    let key = ctx.on_host("Not for this Mac – built for %@", "Not for this computer – built for %@");
    Badge::new("arch", "arch", tf!(key, systems.join(", ")), Tone::Warning).label(Text::verbatim(arch.join(", ")))
}

/// "Not for this Mac – it is built for …": why a recipe for another architecture cannot be built here
pub(crate) fn not_here(ctx: &Ctx, systems: &[String]) -> Text {
    let key = ctx.on_host("Not for this Mac – it is built for %@", "Not for this computer – it is built for %@");
    tf!(key, systems.join(", "))
}

fn lab_badge() -> Badge {
    Badge::new("lab", "lab", Text::new("Development recipe (developer mode)"), Tone::Warning)
}

pub(crate) fn row(ctx: &Ctx, r: &RecipeDto) -> RecipeRowView {
    let building = ctx.active_build(&r.id).is_some();
    let missing = r.parts.as_ref().is_some_and(|p| p.issues.iter().any(Issue::blocks));
    let lamp = if building {
        Lamp::new(LampState::Working, "being built – see Images")
    } else if missing {
        Lamp::new(LampState::Attention, "Parts it is made of are not here – install them")
    } else if r.image.is_none() {
        Lamp::new(LampState::Off, "not built yet")
    } else if r.outdated {
        Lamp::new(LampState::Attention, "Recipe changed – build a new version")
    } else {
        Lamp::new(LampState::On, "Built – its image is in Images")
    };
    let mut secondary = Vec::new();
    if let Some(a) = r.author.as_ref().filter(|a| !a.is_empty() && *a != "omaflake") {
        secondary.push(a.clone());
    }
    secondary.extend(r.distinction.clone());
    let mut badges = Vec::new();
    if r.lab {
        badges.push(lab_badge());
    }
    if !r.runs_here {
        badges.push(arch_badge(ctx, &r.systems));
    }
    let reveal_label = ctx.on_host("Show in Finder", "Show in File Manager");
    let reveal = match &r.folder {
        Some(folder) => {
            Action::call("reveal", Text::new(reveal_label), Some("folder"), "ui.reveal", json!({"path": folder}))
        }
        None => Action::call("reveal", Text::new(reveal_label), Some("folder"), "ui.reveal", json!({}))
            .disabled(Text::new(reveal_label)),
    };
    // Always the build button: greyed out for another architecture, and with the warning sign while the Doctor reports
    // what building this kind needs (Nix for NixOS, a base for macOS) – the recipe itself stays installed
    let blockers = ctx.build_blockers(r.kind);
    let label = if r.image.is_none() || !r.runs_here { "Build image" } else { "Build a new version of this image" };
    let build = Action::show("build", Text::new(label), Some("build"), "build", json!({"recipe": r.id})).primary();
    let build = if !r.runs_here {
        build.disabled(not_here(ctx, &r.systems))
    } else if !blockers.is_empty() {
        build.blocked_by(&blockers)
    } else {
        build.enabled_if(!building, Some(Text::new("being built – see Images")))
    };
    RecipeRowView {
        id: r.id.clone(),
        line: r.line.clone(),
        name: r.display_name.clone(),
        lamp,
        kind: kind_badge(r.kind),
        secondary,
        badges,
        tags: r.tags.clone(),
        actions: super::keys::action(ctx, &r.id, json!({"recipe": r.id})).into_iter().chain([reveal, build]).collect(),
        detail: detail(ctx, r),
        recipe: r.clone(),
    }
}

/// What a recipe is made of, said for people (plan B142): what is missing – with the action that installs it –, what
/// is wrong (warnings: the recipe stays buildable), unfree software
pub(crate) fn notes(ctx: &Ctx, recipe: &str, p: &RecipePartsDto) -> Vec<Hint> {
    let mut out = Vec::new();
    let missing: Vec<String> = p
        .issues
        .iter()
        .filter_map(|i| match i {
            Issue::MissingModule { reference, have } | Issue::MissingKeymap { reference, have } => Some(match have {
                Some(v) => format!("{reference} ({v})"),
                None => reference.clone(),
            }),
            _ => None,
        })
        .collect();
    let look_missing = p.look.as_ref().filter(|_| p.look_id.is_none());
    let install = || {
        Action::call(
            "installParts",
            Text::new("Install Missing Parts"),
            Some("download"),
            "recipes.installParts",
            json!({"id": recipe}),
        )
        .enabled_if(!ctx.installing(recipe), Some(Text::new("Being installed")))
        .primary()
    };
    if !missing.is_empty() {
        out.push(
            Hint::new(
                "warning",
                tf!("Not here yet: %@ – the recipe is made of them", missing.join(", ")),
                Tone::Warning,
            )
            .action(install()),
        );
    }
    if let Some(look) = look_missing {
        let hint = Hint::new(
            "look",
            tf!("Its look %@ is not installed – the build uses the one you choose", look),
            Tone::Normal,
        );
        out.push(if missing.is_empty() { hint.action(install()) } else { hint });
    }
    for i in &p.issues {
        let text = match i {
            Issue::Needs { module, capability } => {
                tf!("%@ needs a %@ – no module of the recipe provides one", module, capability)
            }
            Issue::Incompatible { capability, modules } => {
                tf!("%@ do not go together: each is the %@", modules.join(" + "), capability)
            }
            Issue::KeysOverlap { first, second } => tf!(
                "Keys overlap: %@ (%@, %@) and %@ (%@, %@)",
                &first.title,
                &first.module,
                first.key_text(),
                &second.title,
                &second.module,
                second.key_text()
            ),
            Issue::KeyRule { binding } => tf!(
                "%@ (%@, %@): without a mode a key starts with Super or Ctrl",
                &binding.title,
                &binding.module,
                binding.key_text()
            ),
            _ => continue,
        };
        out.push(Hint::new("warning", text, Tone::Warning));
    }
    if !p.unfree.is_empty() {
        out.push(Hint::new("license", tf!("Not free software: %@", p.unfree.join(", ")), Tone::Normal));
    }
    out
}

/// The switches a new VM starts with, in words (`RecipeRowView.defaults`)
fn defaults_text(r: &RecipeDto) -> Text {
    let mut on = Vec::new();
    if r.clipboard.unwrap_or(true) {
        on.push(Text::new("Clipboard"));
    }
    if r.audio.unwrap_or(true) {
        on.push(Text::new("Sound"));
    }
    if r.microphone.unwrap_or(false) {
        on.push(Text::new("Microphone"));
    }
    if r.camera.unwrap_or(false) && r.kind == crate::models::GuestKind::Nixos {
        on.push(Text::new("Camera"));
    }
    if r.share {
        on.push(Text::new("Share folder"));
    }
    if r.ephemeral {
        on.push(Text::new("Ephemeral"));
    }
    if on.is_empty() {
        Text::verbatim("–")
    } else {
        Text::joined(on)
    }
}

fn detail(ctx: &Ctx, r: &RecipeDto) -> RecipeDetail {
    let images: Vec<_> = ctx.images.iter().filter(|i| i.line == r.line).collect();
    let newest = images.iter().map(|i| i.created).max();
    let mut info = vec![
        InfoLine::new(
            "Resources",
            Text::verbatim(format!("{} CPU · {} GB RAM · {} GB", r.cpus, r.memory_gb, r.disk_gb)),
        ),
        InfoLine::new("Architectures", Text::verbatim(r.systems.join(", "))),
        InfoLine::new("Network", Text::verbatim(r.network.as_str())),
        InfoLine::new("New VMs", defaults_text(r)),
        InfoLine::new(
            "Images",
            match newest {
                None => Text::new("not built yet"),
                Some(d) => Text::fmt("%lld built, the newest %@", vec![Arg::from(images.len()), Arg::date(d)]),
            },
        ),
    ];
    match r.source.as_deref() {
        Some("import") => info.push(InfoLine::new("Source", Text::new("imported (a package or a folder)"))),
        Some(origin) => {
            let s = ctx.sources.iter().find(|s| s.id == origin);
            let name = Text::verbatim(s.map(|s| s.display_name.clone()).unwrap_or_else(|| origin.to_string()));
            let value = if s.is_some_and(|s| s.kind == RecipeSourceKind::Folder) {
                Text::joined(vec![name, Text::new("used as it is in the folder")])
            } else {
                name
            };
            info.push(
                InfoLine::new("Source", value)
                    .full(s.map(|s| s.location.clone()).unwrap_or_else(|| origin.to_string())),
            );
        }
        None => {}
    }
    if let Some(l) = &r.license {
        info.push(InfoLine::new("License", Text::verbatim(l)));
    }
    // What it is made of (plan B142)
    let mut notes_here = Vec::new();
    if let Some(p) = &r.parts {
        let modules: Vec<String> = p
            .modules
            .iter()
            .map(|m| match (&m.title, &m.version) {
                (Some(t), Some(v)) => format!("{t} {v}"),
                _ => m.reference.clone(),
            })
            .collect();
        info.push(
            InfoLine::new("Modules", Text::verbatim(modules.join(", ")))
                .full(p.modules.iter().map(|m| m.reference.clone()).collect::<Vec<_>>().join(", ")),
        );
        if let Some(look) = &p.look {
            info.push(InfoLine::new("Look", Text::verbatim(look)));
        }
        if !p.keymaps.is_empty() {
            info.push(InfoLine::new("Key bindings", Text::verbatim(p.keymaps.join(", "))));
        }
        if p.requirements.cpus > 0 {
            info.push(InfoLine::new(
                "Needs at least",
                Text::verbatim(format!(
                    "{} CPU · {} GB RAM · {} GB",
                    p.requirements.cpus, p.requirements.memory_gb, p.requirements.disk_gb
                )),
            ));
        }
        notes_here = notes(ctx, &r.id, p);
    }
    let n = ctx.recipes.id_lengths.recipe;
    let versions = r.versions.iter().map(|v| version_row(v, n)).collect();
    let mut actions = Vec::new();
    if let Some(u) = ctx.available.as_ref().and_then(|a| a.updates.iter().find(|u| u.id == r.id)) {
        actions.push(
            Action::call(
                "update",
                Text::new("Update"),
                Some("install"),
                "recipes.install",
                json!({"id": u.recipe.id, "source": u.recipe.source}),
            )
            .enabled_if(!ctx.installing(&u.recipe.id), None),
        );
    }
    // A recipe of your own is edited, any other copied into My Recipes (plan B142)
    let mine = r
        .folder
        .as_deref()
        .and_then(|f| crate::my_recipes::MyRecipes::new(ctx.paths.clone()).base_of(std::path::Path::new(f)))
        .is_some();
    if r.parts.is_some() {
        actions.push(Action::show(
            if mine { "edit" } else { "copy" },
            Text::new(if mine { "Edit …" } else { "Copy to My Recipes …" }),
            Some(if mine { "edit" } else { "copy" }),
            "recipeStudio",
            json!({"id": r.id}),
        ));
    }
    actions.push(
        Action::call("removeAll", Text::new("Remove All Versions …"), Some("trash"), "recipes.removeLine", json!({"line": r.line}))
            .destructive()
            .confirm(
                Confirm::new(
                    tf!("Remove all versions of “%@”?", &r.display_name),
                    Text::new("Versions images are built from stay while those images are there; a recipe in a local folder stays in its folder. Images and VMs stay."),
                )
                .button("Remove All")
                .destructive(),
            )
            .enabled_if(r.versions.iter().any(|v| !v.live && v.images.is_empty()), None),
    );
    RecipeDetail { description: r.description.clone(), info, notes: notes_here, versions, actions }
}

fn version_row(v: &VersionDto, words: usize) -> VersionRow {
    let mut parts = Vec::new();
    if v.live {
        parts.push(Text::new("as it is in the folder"));
    } else if let Some(added) = v.added {
        parts.push(Text::fmt("%@", vec![Arg::date(added)]));
    }
    if v.newest {
        parts.push(Text::new("newest"));
    }
    if !v.images.is_empty() {
        parts.push(tf!("images: %lld", v.images.len()));
    }
    let remove = Action::call("remove", Text::new("Remove this version …"), Some("trash"), "recipes.remove", json!({"id": v.id}))
        .destructive()
        .confirm(
            Confirm::new(
                tf!("Remove the recipe version “%@”?", &v.label),
                Text::new("Its files are deleted; images built from it and their VMs stay. You can install it again from its source."),
            )
            .button("Remove")
            .destructive(),
        )
        .enabled_if(!v.live, Some(Text::new("Lives in a local folder – change or remove it there")))
        .enabled_if(v.images.is_empty(), Some(Text::new("Kept while images built from it are there")));
    VersionRow {
        id: v.id.clone(),
        words: crate::omaflake_id::OmaflakeId::short(RecipeId::words(&v.id).unwrap_or("?"), words),
        state: if parts.is_empty() { Text::verbatim("") } else { Text::joined(parts) },
        actions: vec![
            Action::call(
                "export",
                Text::new("Export …"),
                Some("export"),
                "packages.export",
                json!({"kind": "recipe", "name": v.id}),
            )
            .ask(Ask::export_folder(Text::new(
                "Where to put the recipe (with the recipes it needs) – to keep or to share",
            ))),
            remove,
        ],
        version: v.clone(),
    }
}

fn offered_row(ctx: &Ctx, r: &CatalogRecipeDto) -> OfferedRow {
    let source = ctx.sources.iter().find(|s| s.id == r.source);
    let source_name = ctx.source_name(&r.source);
    // Official: by omaflake *and* from omaflake/recipes – an author name alone is not enough
    let official = source.is_some_and(|s| s.official) && r.author == "omaflake";
    let installing = ctx.installing(&r.id);
    let mut badges = Vec::new();
    if official {
        badges.push(Badge::new("official", "official", Text::new("Official recipe"), Tone::Accent));
    }
    if r.lab {
        badges.push(lab_badge());
    }
    if !r.runs_here {
        badges.push(arch_badge(ctx, &r.systems));
    }
    let size: u64 = r.files.iter().map(|f| f.size).sum();
    let mut info = vec![
        InfoLine::new("ID", Text::verbatim(RecipeId::qualified(&r.id))),
        InfoLine::new(
            "Source",
            if official {
                Text::joined(vec![Text::verbatim(&source_name), Text::new("official")])
            } else {
                Text::verbatim(&source_name)
            },
        ),
        InfoLine::new("Architectures", Text::verbatim(r.systems.join(", "))),
    ];
    if let Some(l) = &r.license {
        info.push(InfoLine::new("License", Text::verbatim(l)));
    }
    if !r.modules.is_empty() {
        info.push(InfoLine::new("Modules", Text::verbatim(r.modules.join(", "))));
    }
    info.push(InfoLine::new(
        "Files",
        Text::fmt("%lld files, %@", vec![Arg::from(r.files.len()), Arg::bytes(size as i64)]),
    ));
    if let Some(h) = r.homepage.as_ref().filter(|h| !h.is_empty()) {
        info.push(InfoLine::new("Homepage", Text::verbatim(h)).full(h));
    }
    OfferedRow {
        id: r.id.clone(),
        name: r.name.clone(),
        author: r.author.clone(),
        lamp: if installing {
            Lamp::new(LampState::Working, "Downloading …")
        } else {
            Lamp::new(LampState::Off, "Not installed")
        },
        kind: kind_badge(r.kind),
        badges,
        tags: r.tags.clone(),
        actions: vec![Action::call(
            "install",
            tf!("Install from %@", &source_name),
            Some("install"),
            "recipes.install",
            json!({"id": r.id, "source": r.source}),
        )
        .enabled_if(!installing, None)
        .busy(installing)
        .primary()],
        detail: OfferedDetail {
            description: r.description.clone(),
            info,
            warning: (!official).then(|| {
                Text::new(ctx.on_host(
                    "Not reviewed by the omaflake team – its Nix code runs on this Mac when you build it.",
                    "Not reviewed by the omaflake team – its Nix code runs on this computer when you build it.",
                ))
            }),
        },
        recipe: r.clone(),
    }
}

fn updates(ctx: &Ctx) -> Option<UpdatesSection> {
    let updates = &ctx.available.as_ref()?.updates;
    if updates.is_empty() {
        return None;
    }
    let keep = ctx.settings.keep_recipe_versions;
    let rows = updates
        .iter()
        .map(|u| UpdateRow {
            id: u.id.clone(),
            name: RecipeId::qualified(&u.id),
            kind: kind_badge(u.recipe.kind),
            source: ctx.source_name(&u.recipe.source),
            action: Action::call(
                "update",
                Text::new(if keep { "Add Version" } else { "Update" }),
                Some("install"),
                "recipes.install",
                json!({"id": u.recipe.id, "source": u.recipe.source}),
            )
            .enabled_if(!ctx.installing(&u.recipe.id), None)
            .primary(),
        })
        .collect();
    Some(UpdatesSection {
        title: Text::new("Newer recipe versions in your sources"),
        all: (updates.len() > 1).then(|| {
            Action::call(
                "updateAll",
                Text::new(if keep { "Add All" } else { "Update All" }),
                Some("install"),
                "recipes.installUpdates",
                json!({}),
            )
        }),
        rows,
        note: Text::new(if keep {
            "The new version is added; the older ones stay (Settings: keep older recipe versions)."
        } else {
            "The new version takes over; the older one goes unless an image is built from it."
        }),
    })
}
