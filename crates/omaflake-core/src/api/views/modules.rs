// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Modules tab (`modules`, plan B142): the modules and key binding sets here – installed from a source or in a
//! local folder – with the recipes made of them, and what the sources' catalogs offer besides. The same shape as the
//! Looks tab (`LooksPage`, rows without colours), so every UI draws it as it draws that.

use serde_json::json;

use super::common::*;
use super::looks::{LookDetail, LookRow, LooksPage};
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
        let keys: Vec<String> = p.keys.iter().map(|(a, k)| format!("{a}: {k}")).collect();
        info.push(InfoLine::new("Keys", Text::verbatim(keys.join(" · "))).full(keys.join("\n")));
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
    actions.push(remove);
    let mut lines = info(p);
    lines.push(InfoLine::new(
        "Used by",
        if used == 0 { Text::new("no recipe") } else { Text::verbatim(p.recipes.join(", ")) },
    ));
    LookRow {
        id: p.id.clone(),
        name: p.title.clone(),
        lamp: if used == 0 {
            Lamp::new(LampState::Off, "No recipe here is made of it")
        } else {
            Lamp::new(LampState::On, "Recipes here are made of it")
        },
        subtitle: Text::joined(vec![Text::verbatim(&p.reference), Text::verbatim(&p.version), what(p), usage]),
        badges: badges(p),
        swatches: None,
        actions: if p.update { vec![actions[0].clone()] } else { vec![] },
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

pub(crate) fn page(ctx: &Ctx) -> LooksPage {
    let a = &ctx.modules;
    let mut here: Vec<&PartDto> = a.parts.iter().collect();
    here.sort_by_key(|p| order(p));
    let mut offered: Vec<&PartDto> = a.offered.iter().collect();
    offered.sort_by_key(|p| order(p));
    let rows: Vec<LookRow> = here.into_iter().map(row).collect();
    let offered: Vec<LookRow> = offered.into_iter().map(offered_row).collect();
    let mut head = ListHead::new("Modules", rows.len(), rows.len());
    head.actions =
        vec![Action::call("refresh", Text::new("Refresh"), Some("refresh"), "modules.list", json!({"fetch": true}))
            .help(Text::new("Look into the sources for modules and key bindings"))
            .busy(ctx.fetching)];
    let installing = ctx.active_job(crate::api::types::JobKind::ModuleInstall).is_some();
    let shown = offered.iter().map(|r| (r.actions.as_slice(), r.badges.iter().any(|b| b.id == "official")));
    head.actions.extend(install_all("modules.installAll", shown, installing));
    head.empty = (rows.is_empty() && offered.is_empty()).then(|| {
        Hint::new(
            "module",
            Text::new("No module yet – installing a recipe brings the modules it is made of."),
            Tone::Normal,
        )
    });
    LooksPage {
        head,
        catalogs_loaded: a.fetched,
        loading: ctx.fetching,
        loading_text: ctx.fetching.then(|| Text::new("Looking into the sources …")),
        offered_title: (!offered.is_empty()).then(|| Text::new("In your sources")),
        rows,
        offered,
        unreadable: a
            .unreadable
            .iter()
            .map(|(id, e)| Hint::new("warning", tf!("%@ cannot be read: %@", id, e), Tone::Warning))
            .collect(),
        footer: Text::new(
            "A module is a piece of a recipe – a desktop, a terminal, an editor, LaTeX – written in Nix; key bindings give its actions their keys. Recipes name them, and installing a recipe brings them along. Modules are code: install them only from sources you trust.",
        ),
    }
}
