// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Looks tab (`looks`, plan B141): the looks here – installed from a source or in a local folder – with the
//! default one, their colours and usage, and what the sources' catalogs offer besides. Nothing is built in: images are
//! built with the default look unless the build page chooses another.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::*;
use super::text::{tf, Text};
use super::Ctx;
use crate::api::types::{LookDto, OfferedLookDto};
use crate::looks::Swatches;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LooksPage {
    #[serde(flatten)]
    pub head: ListHead,
    /// The catalogs were looked into this session (else the UI runs the head's `refresh` when the tab opens)
    pub catalogs_loaded: bool,
    /// They are being looked into now
    pub loading: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loading_text: Option<Text>,
    pub rows: Vec<LookRow>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub offered_title: Option<Text>,
    /// Not here, offered by a source
    pub offered: Vec<LookRow>,
    pub unreadable: Vec<Hint>,
    pub footer: Text,
}

/// One look: its name, a strip of its colours, what it is (author, appearance, usage); opened: its description and
/// actions
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookRow {
    pub id: String,
    pub name: String,
    /// On while images are built with it
    pub lamp: Lamp,
    /// "omaflake/catppuccin · dark · 2 images"
    pub subtitle: Text,
    /// default, official, update, local folder
    pub badges: Vec<Badge>,
    /// Its colours as "#RRGGBB" for a strip: background, surface, text, accent, urgent, then the terminal's red,
    /// green, yellow, blue, magenta and cyan; empty for a plain look
    pub swatches: Vec<String>,
    /// The quick action: Install (offered), Make Default (here)
    pub actions: Vec<Action>,
    pub detail: LookDetail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LookDetail {
    pub info: Vec<InfoLine>,
    pub actions: Vec<Action>,
}

fn strip(s: &Option<Swatches>) -> Vec<String> {
    let Some(s) = s else { return vec![] };
    let mut out: Vec<String> = [&s.background, &s.surface, &s.text, &s.accent, &s.urgent]
        .into_iter()
        .filter(|c| !c.is_empty())
        .cloned()
        .collect();
    out.extend(s.terminal.iter().skip(1).take(6).cloned());
    out
}

fn appearance(a: &Option<String>, plain: bool) -> Text {
    match (a.as_deref(), plain) {
        (_, true) => Text::new("each program's own colours"),
        (Some("light"), _) => Text::new("light"),
        _ => Text::new("dark"),
    }
}

fn row(l: &LookDto) -> LookRow {
    let used = l.images.len();
    let usage = match used {
        0 => Text::new("no image"),
        1 => Text::new("1 image"),
        n => tf!("%lld images", n),
    };
    let mut badges = Vec::new();
    if l.is_default {
        badges.push(
            Badge::new(
                "default",
                "ok",
                Text::new("New images get this look unless the build chooses another"),
                Tone::Accent,
            )
            .label(Text::new("default")),
        );
    }
    if l.official {
        badges.push(Badge::new("official", "official", Text::new("By omaflake, from its catalog"), Tone::Accent));
    }
    if l.update {
        badges.push(
            Badge::new("update", "update", Text::new("Its source has a newer version"), Tone::Warning)
                .label(Text::new("update")),
        );
    }
    if !l.installed {
        badges.push(Badge::new(
            "folder",
            "folder",
            tf!("From the folder %@, used as it is there", &l.source_name),
            Tone::Normal,
        ));
    }
    let id = json!({"id": l.id});
    let make_default = Action::call("default", Text::new("Make Default"), Some("ok"), "looks.setDefault", id.clone())
        .enabled_if(!l.is_default, Some(Text::new("Already the default")));
    let mut actions = vec![make_default.clone()];
    if l.update {
        actions.push(
            Action::call(
                "update",
                Text::new("Update"),
                Some("download"),
                "looks.install",
                json!({"id": l.id, "source": l.source}),
            )
            .help(Text::new("Installs the newer version; images built with this look are outdated then")),
        );
    }
    actions.push(
        Action::call("remove", Text::new("Remove …"), Some("trash"), "looks.remove", id)
            .destructive()
            .confirm(
                Confirm::new(
                    tf!("Remove the look “%@”?", &l.title),
                    Text::new("Install it again from its source any time."),
                )
                .button("Remove")
                .destructive(),
            )
            .enabled_if(l.installed, Some(Text::new("A local folder's look – remove it there")))
            .enabled_if(used == 0, Some(Text::new("Images are built with it – remove them first"))),
    );
    let used_by = l.images.join(", ");
    LookRow {
        id: l.id.clone(),
        name: l.title.clone(),
        lamp: if used == 0 {
            Lamp::new(LampState::Off, "No image is built with this look")
        } else {
            Lamp::new(LampState::On, "Images are built with it")
        },
        subtitle: Text::joined(vec![Text::verbatim(&l.reference), appearance(&l.appearance, l.plain), usage]),
        badges,
        swatches: strip(&l.swatches),
        actions: if l.is_default { vec![] } else { vec![make_default] },
        detail: LookDetail {
            info: vec![
                InfoLine::new("About", Text::verbatim(&l.description)),
                InfoLine::new("Source", Text::verbatim(&l.source_name)),
                InfoLine::new(
                    "Used by",
                    if used_by.is_empty() { Text::new("no image") } else { Text::verbatim(&used_by) },
                )
                .full(&used_by),
            ],
            actions,
        },
    }
}

fn offered_row(l: &OfferedLookDto) -> LookRow {
    let mut badges = Vec::new();
    if l.official {
        badges.push(Badge::new("official", "official", Text::new("By omaflake, from its catalog"), Tone::Accent));
    }
    let install = Action::call(
        "install",
        Text::new("Install"),
        Some("download"),
        "looks.install",
        json!({"id": l.id, "source": l.source}),
    )
    .primary();
    LookRow {
        id: l.id.clone(),
        name: l.title.clone(),
        lamp: Lamp::new(LampState::Off, "Not installed"),
        subtitle: Text::joined(vec![
            Text::verbatim(&l.reference),
            appearance(&l.appearance, l.plain),
            Text::verbatim(&l.source_name),
        ]),
        badges,
        swatches: strip(&l.swatches),
        actions: vec![install.clone()],
        detail: LookDetail {
            info: vec![
                InfoLine::new("About", Text::verbatim(&l.description)),
                InfoLine::new("Source", Text::verbatim(&l.source_name)),
            ],
            actions: vec![install],
        },
    }
}

pub(crate) fn page(ctx: &Ctx) -> LooksPage {
    let a = &ctx.looks;
    let rows: Vec<LookRow> = a.looks.iter().map(row).collect();
    let offered: Vec<LookRow> = a.offered.iter().map(offered_row).collect();
    let mut head = ListHead::new("Looks", rows.len(), rows.len());
    head.actions =
        vec![Action::call("refresh", Text::new("Refresh"), Some("refresh"), "looks.list", json!({"fetch": true}))
            .help(Text::new("Look into the sources for looks"))
            .busy(ctx.fetching)];
    head.empty = (rows.is_empty() && offered.is_empty()).then(|| {
        Hint::new(
            "look",
            Text::new("No look yet – images are built with one. Install omaflake's own and Catppuccin to start."),
            Tone::Warning,
        )
        .action(
            Action::call(
                "installLooks",
                Text::new("Install Recommended"),
                Some("download"),
                "looks.installRecommended",
                json!({}),
            )
            .primary(),
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
            "A look is how a guest looks – colours, the terminal's palette, the editors' theme, the desktop's borders. It is data, not code: omaflake's image flake turns it into each program's configuration. New images get the default look unless the build page chooses another; a changed look makes its images outdated.",
        ),
    }
}
