// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Images tab (`images`) and one image (`image`): every built image (plan B93) and the builds that wait, run or
//! failed on top – the app's `ImageRowView`, `ImageVersionInfo` and `BuildJobRow`.

use std::collections::HashSet;

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::*;
use super::text::{tf, Arg, Text};
use super::vms::{kind_badge, recipe_name, unreadable};
use super::Ctx;
use crate::api::types::{ImageDto, JobDto, JobState};
use crate::models::{GuestKind, RecipeId};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagesPage {
    #[serde(flatten)]
    pub head: ListHead,
    /// Builds that wait, run, failed or were cancelled (finished ones are images now)
    pub jobs: Vec<BuildJobRow>,
    pub rows: Vec<ImageRowView>,
    pub unreadable: Vec<Hint>,
}

/// A build in a list: its state in words, its progress; a click opens the build page (`open`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BuildJobRow {
    pub id: String,
    /// The recipe's name ("dev (velvet)")
    pub name: String,
    pub kind: Badge,
    /// What the row's symbol says: queued (clock), working (spinner), failed, finished, cancelled
    pub state: JobState,
    pub title: Text,
    /// Packages done of announced ("120/400"), while running
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<Text>,
    /// 0…1 while running
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fraction: Option<f64>,
    /// Cancel / remove from the queue, or hide an ended one
    pub actions: Vec<Action>,
    pub open: Action,
    pub job: JobDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageRowView {
    pub name: String,
    /// Green: a VM uses it (plan B100)
    pub lamp: Lamp,
    pub kind: Badge,
    /// "outdated"
    pub badges: Vec<Badge>,
    /// "recipe dev · <created> · 2 VMs"
    pub subtitle: Text,
    /// The VMs using it (the subtitle's tooltip)
    pub used_by: Vec<String>,
    /// Show in the file manager; New VM from it (or the Doctor's warning)
    pub actions: Vec<Action>,
    pub detail: ImageDetail,
    pub image: ImageDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageDetail {
    pub info: Vec<InfoLine>,
    /// Export on the left, remove on the right
    pub actions: Vec<Action>,
}

/// `image`: one image with its size; `params: {name}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImagePage {
    pub row: ImageRowView,
}

#[derive(Debug, Default, Deserialize)]
pub struct ImagesParams {
    #[serde(default)]
    pub filter: FilterParams,
}

pub(crate) fn page(ctx: &Ctx, p: &ImagesParams) -> ImagesPage {
    let f = &p.filter;
    let all = &ctx.images;
    let used: HashSet<&str> = ctx.vms.iter().map(|v| v.image.as_str()).collect();
    let rows: Vec<ImageRowView> = all
        .iter()
        .filter(|i| {
            f.status.is_none_or(|s| s == used.contains(i.name.as_str()))
                && f.kind.is_none_or(|k| k == i.kind)
                && f.recipe.as_ref().is_none_or(|r| *r == i.recipe)
        })
        .map(|i| row(ctx, i))
        .collect();
    let jobs: Vec<BuildJobRow> = ctx
        .builds()
        .filter(|j| {
            j.state != JobState::Finished
                && f.kind.is_none_or(|k| Some(k) == j.guest_kind)
                && f.recipe.as_ref().is_none_or(|r| Some(r) == j.recipe.as_ref())
                && f.status != Some(true)
        })
        .map(|j| job_row(ctx, j))
        .collect();
    let mut recipes: Vec<String> = all.iter().map(|i| i.recipe.clone()).collect::<HashSet<_>>().into_iter().collect();
    recipes.sort_by_key(|r| (RecipeId::name_of_id(r), r.clone()));
    let mut groups = f.base_groups("In use", "Not in use");
    if !recipes.is_empty() {
        let mut options = vec![Choice::new(serde_json::Value::Null, Text::new("All"))];
        options.extend(recipes.iter().map(|r| Choice::new(json!(r), Text::verbatim(recipe_name(ctx, r)))));
        groups.push(FilterGroup {
            id: "recipe".into(),
            label: Text::new("Recipe"),
            kind: FilterKind::Choice,
            value: json!(f.recipe),
            options,
        });
    }
    let mut head = ListHead::new("Images", rows.len(), all.len());
    head.filter = Some(Filter { active: f.is_active(), groups });
    head.actions = vec![Action::call(
        "import",
        Text::new("Import"),
        Some("import"),
        "ui.step",
        json!({"page": "import", "params": {"expect": "image"}}),
    )
    .help(Text::new("Import an image – a copy into the data folder"))
    .ask(Ask::packages("image", false, "An image (.omaflake-image) – it is copied into the data folder", "Import"))];
    head.empty = if all.is_empty() && jobs.is_empty() {
        Some(Hint::new("image", Text::new("No images yet – build one from a recipe."), Tone::Normal))
    } else if rows.is_empty() && jobs.is_empty() {
        Some(filtered_out("images"))
    } else {
        None
    };
    ImagesPage { head, jobs, rows, unreadable: unreadable(&ctx.unreadable_images) }
}

pub(crate) fn job_row(ctx: &Ctx, j: &JobDto) -> BuildJobRow {
    let running = matches!(j.state, JobState::Running);
    let title = match j.state {
        JobState::Queued => Text::new("Waiting to build"),
        JobState::Checking => Text::new("Checking …"),
        JobState::Running => match &j.progress {
            Some(p) => tf!("%lld/%lld %@", p.step, p.total, Text::new(&p.title)),
            None => Text::verbatim(&j.title),
        },
        JobState::Failed => Text::new("Build failed – show log"),
        JobState::Cancelled => Text::new("Build cancelled"),
        JobState::Finished => Text::new("Built"),
    };
    let progress = j.progress.as_ref().filter(|_| running);
    let actions = if j.state.is_active() {
        let help = if j.state == JobState::Queued { "Remove from the queue" } else { "Cancel build" };
        vec![Action::call("cancel", Text::new(help), Some("cancel"), "jobs.cancel", json!({"id": j.id}))]
    } else {
        vec![Action::call("dismiss", Text::new("Hide"), Some("hide"), "jobs.dismiss", json!({"id": j.id}))]
    };
    let recipe = j.recipe.clone().unwrap_or_default();
    BuildJobRow {
        id: j.id.clone(),
        name: j.recipe.as_deref().map(|r| recipe_name(ctx, r)).unwrap_or_else(|| j.title.clone()),
        kind: kind_badge(j.guest_kind.unwrap_or(GuestKind::Nixos)),
        state: j.state,
        title,
        count: progress.filter(|p| p.of > 0).map(|p| Text::verbatim(format!("{}/{}", p.done.min(p.of), p.of))),
        fraction: progress.map(|p| p.fraction),
        actions,
        open: Action::show("open", Text::new("Show the build and its log"), None, "build", json!({"recipe": recipe})),
        job: j.clone(),
    }
}

/// The recipe evaluates to something else than when the image was built (None: not known) – `AppModel.isOutdated`
pub(crate) fn is_outdated(ctx: &Ctx, i: &ImageDto) -> Option<bool> {
    if let Some(o) = i.outdated {
        return Some(o);
    }
    if !i.current {
        return None;
    }
    ctx.recipes.recipes.iter().find(|r| r.image.as_deref() == Some(i.name.as_str())).map(|r| r.outdated)
}

pub(crate) fn row(ctx: &Ctx, i: &ImageDto) -> ImageRowView {
    let used_by = i.instances.clone();
    let outdated = is_outdated(ctx, i);
    let mut badges = Vec::new();
    if outdated == Some(true) {
        badges.push(
            Badge::new(
                "outdated",
                "outdated",
                Text::new("Built from an earlier state of the recipe – a new build would differ"),
                Tone::Warning,
            )
            .label(Text::new("outdated")),
        );
    }
    let vms = match used_by.len() {
        0 => Text::new("no VM"),
        1 => Text::new("1 VM"),
        n => tf!("%lld VMs", n),
    };
    let subtitle = Text::joined(vec![
        tf!("recipe %@", recipe_name(ctx, &i.recipe)),
        Text::fmt("%@", vec![Arg::date(i.created)]),
        vms,
    ]);
    let blockers = ctx.start_blockers(i.kind);
    let new_vm = if blockers.is_empty() {
        Action::show("newVm", Text::new("New VM from this image"), Some("new-vm"), "newVm", json!({"image": i.name}))
    } else {
        Action::doctor_warning(&blockers)
    };
    let actions = vec![
        Action::call("reveal", Text::new("Show in Finder"), Some("folder"), "ui.reveal", json!({"path": i.path})),
        new_vm,
    ];
    ImageRowView {
        name: i.name.clone(),
        lamp: if used_by.is_empty() {
            Lamp::new(LampState::Off, "No VM uses this image")
        } else {
            Lamp::new(LampState::On, "Used by VMs")
        },
        kind: kind_badge(i.kind),
        badges,
        subtitle,
        detail: detail(ctx, i, outdated),
        used_by,
        actions,
        image: i.clone(),
    }
}

fn detail(ctx: &Ctx, i: &ImageDto, outdated: Option<bool>) -> ImageDetail {
    let recipe_label = ctx
        .recipes
        .recipes
        .iter()
        .find(|r| r.id == i.recipe)
        .map(|r| r.label.clone())
        .unwrap_or_else(|| RecipeId::qualified(&i.recipe));
    let revision = if i.revision == "nogit" {
        Text::new("no git revision")
    } else if let Some(rev) = i.revision.strip_suffix("-dirty") {
        tf!("%@ %@", rev, Text::new("(with uncommitted changes)"))
    } else {
        Text::verbatim(&i.revision)
    };
    let status = match outdated {
        Some(true) => InfoLine::new("Status", Text::new("built from an older state of the recipe")).tone(Tone::Warning),
        Some(false) => InfoLine::new("Status", Text::new("matches the recipe")),
        None => InfoLine::new("Status", Text::verbatim("–")),
    };
    let used = i.instances.join(", ");
    let mut info = vec![
        InfoLine::new("omaflake ID", Text::verbatim(&i.label)).full(&i.omaflake_id),
        InfoLine::new("Recipe", Text::verbatim(recipe_label)).full(&i.recipe),
        InfoLine::new("Built", Text::fmt("%@", vec![Arg::date(i.created)])),
        InfoLine::new("Recipe state", revision),
        status,
        InfoLine::new("Used by", if used.is_empty() { Text::new("no VM") } else { Text::verbatim(&used) }).full(&used),
        InfoLine::new(
            "Size",
            match i.size_bytes {
                Some(b) => Text::fmt("%@", vec![Arg::bytes(b)]),
                None => Text::new("unknown"),
            },
        ),
    ];
    if let Some(source) = &i.source {
        info.push(InfoLine::new("Source", Text::verbatim(source)).full(source));
    }
    if let Some(store) = &i.store_path {
        let label = if i.kind == GuestKind::Macos { "System" } else { "Disk image" };
        let last = store.rsplit('/').next().unwrap_or(store);
        info.push(InfoLine::new(label, Text::verbatim(last)).full(store));
    }
    let actions = vec![
        Action::call("export", Text::new("Export …"), Some("export"), "packages.export", json!({"kind": "image", "name": i.name}))
            .ask(Ask::export_folder(Text::new(
                "Where to put the image – an external disk, a NAS or a folder. It carries the user it was built for: share recipes instead.",
            ))),
        Action::call("remove", Text::new("Remove …"), Some("trash"), "images.remove", json!({"name": i.name}))
            .destructive()
            .confirm(
                Confirm::new(
                    tf!("Remove the image “%@”?", &i.name),
                    Text::new("It is deleted and cannot be restored. VMs that use it must be removed first."),
                )
                .button("Remove")
                .destructive(),
            )
            .enabled_if(i.instances.is_empty(), Some(Text::new("Used by VMs – remove or upgrade them first"))),
    ];
    ImageDetail { info, actions }
}

#[derive(Debug, Deserialize)]
pub struct ImageParams {
    pub name: String,
}

pub(crate) fn image_page(ctx: &Ctx, p: &ImageParams) -> Option<ImagePage> {
    ctx.images.iter().find(|i| i.name == p.name).map(|i| ImagePage { row: row(ctx, i) })
}
