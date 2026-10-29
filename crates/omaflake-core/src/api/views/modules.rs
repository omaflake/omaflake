// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Modules tab (`modules`, plan B142): the modules and key binding sets here – installed from a source or in a
//! local folder – with the recipes made of them, and what the sources' catalogs offer besides. The same shape as the
//! Looks tab (`LooksPage`, rows without colours), so every UI draws it as it draws that.

use serde_json::json;

use super::common::*;
use super::looks::{LookDetail, LookRow, LooksPage};
use super::recipes::RecipesParams;
use super::text::{tf, Text};
use super::Ctx;
use crate::api::types::PartDto;
use crate::models::GuestKind;

fn what(p: &PartDto) -> Text {
    match (p.kind.as_str(), p.platform) {
        ("keymap", _) => Text::new("key bindings"),
        (_, Some(GuestKind::Macos)) => Text::new("macOS module"),
        _ => Text::new("NixOS module"),
    }
}

fn badges(p: &PartDto) -> Vec<Badge> {
    let mut out = Vec::new();
    if p.official {
        out.push(Badge::new("official", "official", Text::new("By omaflake, from its catalog"), Tone::Accent));
    }
    if p.update {
        out.push(
            Badge::new("update", "update", Text::new("Its source has a newer version"), Tone::Warning)
                .label(Text::new("update")),
        );
    }
    if p.unfree {
        out.push(
            Badge::new("unfree", "license", Text::new("Not free software"), Tone::Normal).label(Text::new("unfree")),
        );
    }
    out
}

fn info(p: &PartDto) -> Vec<InfoLine> {
    let mut info = vec![InfoLine::new("About", Text::verbatim(&p.description))];
    if let Some(why) = &p.broken {
        info.insert(0, InfoLine::new("Cannot be read", Text::verbatim(why)));
    }
    if !p.provides.is_empty() {
        info.push(InfoLine::new("Provides", Text::verbatim(p.provides.join(", "))));
    }
    if !p.needs.is_empty() {
        info.push(InfoLine::new("Needs", Text::verbatim(p.needs.join(", "))));
    }
    if !p.enhances.is_empty() {
        info.push(InfoLine::new("Works with", Text::verbatim(p.enhances.join(", "))));
    }
    if !p.keys.is_empty() {
        // The keys themselves as caps in the keys window (`keys_action`), not as text here (plan B148)
        info.push(InfoLine::new("Keys", tf!("%lld keys", p.keys.len())));
    }
    info.push(InfoLine::new("Source", Text::verbatim(&p.source_name)));
    info
}

fn row(p: &PartDto) -> LookRow {
    let used = p.recipes.len();
    let usage = match used {
        0 => Text::new("no recipe"),
        1 => Text::new("1 recipe"),
        n => tf!("%lld recipes", n),
    };
    let mut actions = Vec::new();
    if p.update {
        actions.push(
            Action::call(
                "update",
                Text::new("Update"),
                Some("download"),
                "modules.install",
                json!({"id": p.id, "source": p.source}),
            )
            .help(Text::new("Installs the newer version; images made with it are outdated then")),
        );
    }
    let mut remove =
        Action::call("remove", Text::new("Remove …"), Some("trash"), "modules.remove", json!({"id": p.id}))
            .destructive()
            .confirm(
                Confirm::new(
                    tf!("Remove %@?", &p.title),
                    if used == 0 {
                        Text::new("Install it again from its source any time.")
                    } else {
                        tf!("Recipes made of it cannot be built until it is back: %@", p.recipes.join(", "))
                    },
                )
                .button("Remove")
                .destructive(),
            )
            .enabled_if(p.installed, Some(Text::new("A local folder's – remove it there")));
    if used > 0 {
        remove = remove.help(tf!("Used by %@", p.recipes.join(", ")));
    }
    // A module's keys in the keys window, as caps by mode (plan B148)
    if p.kind == "module" && !p.keys.is_empty() {
        let platform = if p.platform == Some(GuestKind::Macos) { "macos" } else { "nixos" };
        actions.insert(
            0,
            Action::show(
                "keys",
                Text::new("Keys …"),
                Some("keyboard"),
                "keys",
                json!({"module": p.reference, "platform": platform}),
            ),
        );
    }
    // A key binding set opens in the key binding studio: one of yours is edited, any other copied (plan B147)
    if p.kind == "keymap" {
        let mine = p.id.starts_with(&format!("{}.keymap.", crate::my_recipes::MyRecipes::AUTHOR));
        actions.insert(
            0,
            Action::show(
                "studio",
                Text::new(if mine { "Edit …" } else { "Copy to My Key Bindings …" }),
                Some("keyboard"),
                "keymapStudio",
                json!({"id": p.id}),
            ),
        );
    }
    actions.push(remove);
    // One that cannot be read can only be updated or removed
    if p.broken.is_some() {
        actions.retain(|a| a.id == "update" || a.id == "remove");
    }
    let mut lines = info(p);
    lines.push(InfoLine::new(
        "Used by",
        if used == 0 { Text::new("no recipe") } else { Text::verbatim(p.recipes.join(", ")) },
    ));
    LookRow {
        id: p.id.clone(),
        name: p.title.clone(),
        lamp: if p.broken.is_some() {
            broken_lamp()
        } else if used == 0 {
            Lamp::new(LampState::Off, "No recipe here is made of it")
        } else {
            Lamp::new(LampState::On, "Recipes here are made of it")
        },
        subtitle: Text::joined(vec![Text::verbatim(&p.reference), Text::verbatim(&p.version), what(p), usage]),
        badges: badges(p),
        swatches: None,
        // The quick buttons: its keys, open in the key binding studio, update
        actions: actions.iter().filter(|a| ["keys", "studio", "update"].contains(&a.id.as_str())).cloned().collect(),
        detail: LookDetail { info: lines, actions },
    }
}

fn offered_row(p: &PartDto) -> LookRow {
    let install = Action::call(
        "install",
        Text::new("Install"),
        Some("download"),
        "modules.install",
        json!({"id": p.id, "source": p.source}),
    )
    .primary();
    LookRow {
        id: p.id.clone(),
        name: p.title.clone(),
        lamp: Lamp::new(LampState::Off, "Not installed"),
        subtitle: Text::joined(vec![
            Text::verbatim(&p.reference),
            Text::verbatim(&p.version),
            what(p),
            Text::verbatim(&p.source_name),
        ]),
        badges: badges(p),
        swatches: None,
        actions: vec![install.clone()],
        detail: LookDetail { info: info(p), actions: vec![install] },
    }
}

/// Modules first (NixOS, then macOS), then key binding sets; each by title
fn order(p: &PartDto) -> (u8, String) {
    let group = match (p.kind.as_str(), p.platform) {
        ("keymap", _) => 2,
        (_, Some(GuestKind::Macos)) => 1,
        _ => 0,
    };
    (group, p.title.to_lowercase())
}

/// The catalog's modules (plan B147: key binding sets have a tab of their own, `keymaps_page`)
pub(crate) fn page(ctx: &Ctx, p: &RecipesParams) -> LooksPage {
    tab(ctx, "module", &p.filter)
}

/// The catalog's key binding sets: here and offered, each opening in the key binding studio
pub(crate) fn keymaps_page(ctx: &Ctx, p: &RecipesParams) -> LooksPage {
    tab(ctx, "keymap", &p.filter)
}

fn tab(ctx: &Ctx, kind: &str, f: &FilterParams) -> LooksPage {
    let a = &ctx.modules;
    let mut here: Vec<&PartDto> = a.parts.iter().filter(|p| p.kind == kind).collect();
    here.sort_by_key(|p| order(p));
    let mut offered: Vec<&PartDto> = a.offered.iter().filter(|p| p.kind == kind).collect();
    offered.sort_by_key(|p| order(p));
    // Those here, then those offered – searched, narrowed by chips, a page at a time (plan B148)
    let all = here.len() + offered.len();
    let items: Vec<Browsable<(bool, &PartDto)>> = here
        .into_iter()
        .map(|p| (true, p))
        .chain(offered.into_iter().map(|p| (false, p)))
        .map(|(is_here, p)| Browsable {
            row: (is_here, p),
            haystack: format!(
                "{} {} {} {} {}",
                p.title,
                p.description,
                p.reference,
                p.tags.join(" "),
                p.provides.join(" ")
            ),
            tags: p
                .tags
                .iter()
                .chain(p.provides.iter())
                .cloned()
                .collect::<std::collections::BTreeSet<_>>()
                .into_iter()
                .collect(),
        })
        .collect();
    let keymaps = kind == "keymap";
    let page_name = if keymaps { "keymaps" } else { "modules" };
    let (shown, matched, browse) = browse(
        page_name,
        f,
        items,
        if keymaps { "Search key bindings" } else { "Search modules: name, what it is, tags" },
    );
    let rows: Vec<LookRow> = shown.iter().filter(|(h, _)| *h).map(|(_, p)| row(p)).collect();
    let offered: Vec<LookRow> = shown.iter().filter(|(h, _)| !*h).map(|(_, p)| offered_row(p)).collect();
    let mut head = ListHead::new(if keymaps { "Key Bindings" } else { "Modules" }, matched, all);
    head.browse = (all > 0).then_some(browse);
    head.actions =
        vec![Action::call("refresh", Text::new("Refresh"), Some("refresh"), "modules.list", json!({"fetch": true}))
            .help(Text::new("Look into the sources for modules and key bindings"))
            .busy(ctx.fetching)];
    let installing = ctx.active_job(crate::api::types::JobKind::ModuleInstall).is_some();
    let shown = offered.iter().map(|r| (r.actions.as_slice(), r.badges.iter().any(|b| b.id == "official")));
    head.actions.extend(install_all("modules.installAll", shown, installing));
    let updates = a
        .parts
        .iter()
        .filter(|p| p.kind == kind && p.update)
        .map(|p| json!({"id": p.id, "source": p.source}))
        .collect();
    head.actions.extend(update_all("modules.installAll", updates, installing));
    if keymaps {
        head.actions.push(
            Action::show("new", Text::new("New Key Bindings …"), Some("add"), "keymapStudio", json!({}))
                .help(Text::new("Key bindings of your own, in the key binding studio")),
        );
    }
    head.empty = if all > 0 && rows.is_empty() && offered.is_empty() {
        Some(filtered_out(page_name))
    } else {
        (rows.is_empty() && offered.is_empty()).then(|| {
        Hint::new(
            if keymaps { "keyboard" } else { "module" },
            Text::new(if keymaps {
                "No key bindings yet – installing a recipe brings the ones it names; the key binding studio makes your own."
            } else {
                "No module yet – installing a recipe brings the modules it is made of."
            }),
            Tone::Normal,
        )
        })
    };
    // Only this tab's: a key binding set's ids have ".keymap."
    let unreadable = unreadable_hints(
        a.unreadable.iter().filter(|(id, _)| id.contains(".keymap.") == keymaps),
        a.fetched,
        &head.actions[0],
    );
    LooksPage {
        head,
        catalogs_loaded: a.fetched,
        loading: ctx.fetching,
        loading_text: ctx.fetching.then(|| Text::new("Looking into the sources …")),
        offered_title: (!offered.is_empty()).then(|| Text::new("In your sources")),
        rows,
        offered,
        unreadable,
        footer: Text::new(if keymaps {
            "Key bindings give the modules' actions their keys – a recipe names one or two sets, a later one wins. Each module works without any: it has its own keys."
        } else {
            "A module is a piece of a recipe – a desktop, a terminal, an editor, LaTeX – written in Nix; key bindings give its actions their keys. Recipes name them, and installing a recipe brings them along. Modules are code: install them only from sources you trust."
        }),
    }
}
