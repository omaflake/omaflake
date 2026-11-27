// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Build Image page (`build`): one recipe's build – start it (a macOS recipe asks the guest user's password), its
//! place in the queue, progress, outcome and log (the app's `BuildView`, `BuildProgressView`, `BuildOutcome`).

use serde_json::json;

use super::Ctx;
use super::common::*;
use super::text::{Text, tf};
use super::vms::{kind_badge, recipe_name};
use crate::api::types::{BuildResult, JobDto, JobProgress, JobState};
use crate::models::GuestKind;

// The page's model (plan B192: in omaflake-sdk); this file builds it
pub use omaflake_sdk::views::build::*;

/// The credential and look fields of the build form, and the values they have now (the user's, else the defaults)
fn choice_fields(
    ctx: &Ctx,
    v: &BuildValues,
    mac: bool,
    suggested_look: Option<&str>,
) -> (Vec<Field>, String, String, String) {
    let default_credential = ctx.settings.default_credential.clone().unwrap_or_default();
    // macOS too (plan B166): each credential has its own base, the build takes the chosen credential's
    let credential =
        v.credential.clone().filter(|id| ctx.credentials.iter().any(|c| &c.id == id)).unwrap_or(default_credential);
    // An installed look (plan B141): the chosen one while it is here, else the recipe's when it is here (B142), else
    // the default
    let look = v
        .look
        .clone()
        .filter(|id| ctx.looks.looks.iter().any(|l| &l.id == id))
        .or_else(|| suggested_look.map(str::to_string))
        .or_else(|| ctx.settings.default_look.clone())
        .unwrap_or_default();
    let mut c = Field::new("credential", FieldKind::Choice, Text::new("Credential"), json!(credential))
        .param("credential")
        .value_param("credential");
    c.choices = ctx
        .credentials
        .iter()
        .map(|x| Choice::new(json!(x.id), Text::verbatim(format!("{} – {}", x.name, x.user.as_deref().unwrap_or("–")))))
        .collect();
    c.help = Some(Text::new("Who you are in the guest: user, full name, e-mail (Credentials, the key at the top)"));
    if mac {
        c.help = Some(Text::new(
            "macOS images are built on this credential's base: its user and password (one base per credential)",
        ));
    }
    let mut l = Field::new("look", FieldKind::Choice, Text::new("Look"), json!(look))
        .param("look")
        .value_param("look")
        .enabled_if(!ctx.looks.looks.is_empty(), Some(Text::new("Install a look first – the Looks tab")));
    l.choices = ctx.looks.looks.iter().map(|x| Choice::new(json!(x.id), Text::verbatim(&x.title))).collect();
    l.help = Some(Text::new("How the guest looks: colours, terminal, editors (the Looks tab)"));
    // The locale (plan B141): the chosen one while it is there, else the default; without any, this computer's values
    let locale = v
        .locale
        .clone()
        .filter(|id| ctx.locales.iter().any(|l| &l.id == id))
        .or_else(|| ctx.settings.default_locale.clone())
        .unwrap_or_default();
    let mut lo = Field::new("locale", FieldKind::Choice, Text::new("Locale"), json!(locale))
        .param("locale")
        .value_param("locale");
    lo.choices = if ctx.locales.is_empty() {
        vec![Choice::new(json!(""), Text::new("This computer's values"))]
    } else {
        ctx.locales
            .iter()
            .map(|x| Choice::new(json!(x.id), Text::verbatim(format!("{} – {}", x.name, x.keyboards.join(" + ")))))
            .collect()
    };
    lo.help = Some(Text::new("Language, region, time zone and keyboards (Locales, the globe at the top)"));
    (vec![c, l, lo], credential, look, locale)
}

pub fn progress(j: &JobDto) -> Progress {
    let p = j.progress.clone().unwrap_or(JobProgress {
        step: 0,
        total: 0,
        title: String::new(),
        done: 0,
        of: 0,
        fraction: 0.0,
    });
    Progress {
        step: Text::verbatim(format!("{}/{}", p.step, p.total)),
        title: if p.title.is_empty() { Text::verbatim("") } else { Text::new(&p.title) },
        count: (p.of > 0).then(|| Text::verbatim(format!("{}/{}", p.done.min(p.of), p.of))),
        fraction: p.fraction,
        started: j.started,
        last_output: j.last_output,
        quiet_warning: 120,
        quiet_help: Text::new(
            "Some steps print nothing for minutes (writing the disk image, copying); Cancel stops the build",
        ),
    }
}

fn state_of(j: &JobDto) -> BuildState {
    match j.state {
        JobState::Queued => BuildState::Queued,
        JobState::Checking => BuildState::Checking,
        JobState::Running => BuildState::Running,
        JobState::Finished => BuildState::Finished,
        JobState::Failed => BuildState::Failed,
        JobState::Cancelled => BuildState::Cancelled,
    }
}

pub(crate) fn page(ctx: &Ctx, p: &BuildParams) -> BuildPage {
    let r = ctx.recipes.recipes.iter().find(|r| r.id == p.recipe);
    let kind = r.map(|r| r.kind).unwrap_or(GuestKind::Nixos);
    let mac = kind == GuestKind::Macos;
    let job = ctx.latest_build(&p.recipe).cloned();
    let active = job.as_ref().is_some_and(|j| j.state.is_active());
    let blockers = if r.is_some() { ctx.build_blockers(kind) } else { vec![] };
    let has_base = ctx.doctor.as_ref().is_none_or(|d| d.has_mac_base);
    let mut badges = Vec::new();
    if r.is_some_and(|r| r.lab) {
        badges.push(Badge::new("lab", "lab", Text::new("Development recipe (developer mode)"), Tone::Warning));
    }

    let parts = r.and_then(|r| r.parts.as_ref());
    let (mut fields, credential, look, locale) =
        choice_fields(ctx, &p.values, mac, parts.and_then(|x| x.look_id.as_deref()));
    // macOS images are built on the newest base of the chosen credential (plan B166) – else on a base without a
    // credential yet, which asks to assign one (plan B139)
    let base = if mac {
        ctx.bases
            .iter()
            .find(|b| b.ready && b.newest && !b.unchained && b.credential.as_deref() == Some(credential.as_str()))
            .or_else(|| ctx.bases.iter().find(|b| b.ready && b.newest && !b.unchained && b.credential.is_none()))
    } else {
        None
    };
    let credential_name = ctx
        .credentials
        .iter()
        .find(|c| c.id == credential)
        .map(|c| c.name.clone())
        .unwrap_or_else(|| credential.clone());
    // The build VM in a window (macOS only): to watch where it waits
    let window = p.values.window.unwrap_or(false);
    if mac {
        let mut w = Field::new("window", FieldKind::Toggle, Text::new("Show the build VM"), json!(window))
            .param("window")
            .value_param("window");
        w.help = Some(Text::new("Shows the build VM in a window – to see where a build waits; off: headless"));
        fields.push(w);
    }
    // Unchained (macOS only, plan B184): offered once the chosen credential has an unchained base
    let unchained_base =
        mac && ctx.bases.iter().any(|b| b.unchained && b.ready && b.credential.as_deref() == Some(credential.as_str()));
    let unchained = unchained_base && p.values.unchained.unwrap_or(false);
    if unchained_base {
        let mut u = Field::new("unchained", FieldKind::Toggle, Text::new("Unchained"), json!(unchained))
            .param("unchained")
            .value_param("unchained");
        u.help = Some(Text::new(
            "Builds from the credential's unchained base: the image and its VMs run with macOS's System Integrity Protection off and have omaflake's camera. Never a recipe's current image – make a VM from it with “New VM from this image”",
        ));
        fields.push(u);
    }
    let missing = parts.is_some_and(|x| x.issues.iter().any(crate::composition::Issue::blocks));
    let mut notes = parts.map(|x| super::recipes::notes(ctx, &p.recipe, x)).unwrap_or_default();
    // A locale the recipe is not made for: a warning, the build stays possible (plan B141)
    if let (Some(x), Some(l)) = (parts, ctx.locales.iter().find(|l| l.id == locale)) {
        let text = match x.locales.outside(&l.locale) {
            Some(crate::composition::Outside::Languages(langs)) => {
                Some(tf!("This recipe is made for %@ – not for the locale %@", langs.join(", "), &l.name))
            }
            Some(crate::composition::Outside::RightToLeft) => {
                Some(tf!("This recipe is not made for right-to-left languages like the locale %@", &l.name))
            }
            None => None,
        };
        if let Some(text) = text {
            notes.push(Hint::new("warning", text, Tone::Warning));
        }
    }
    let no_credential = ctx.credentials.is_empty();
    let base_without = base.filter(|b| b.credential.is_none());
    let mut actions = Vec::new();
    match &job {
        Some(j) if active => {
            let label = if j.state == JobState::Queued { "Remove from Queue" } else { "Cancel" };
            actions.push(Action::call("cancel", Text::new(label), Some("cancel"), "jobs.cancel", json!({"id": j.id})));
        }
        _ => {
            let queue = ctx.builds().any(|b| b.state.is_active() && b.guest_kind == Some(kind));
            let mut build = Action::call(
                "build",
                Text::new(if queue { "Add to Queue" } else { "Build" }),
                Some("build"),
                "images.build",
                json!({"recipe": p.recipe, "credential": credential, "look": look, "locale": locale}),
            );
            if mac {
                build.params["window"] = json!(window);
            }
            if unchained {
                build.params["unchained"] = json!(true);
            }
            // No password is asked: NixOS needs none, macOS takes the base credential's (plan B139)
            build = build
                .enabled_if(!no_credential, Some(Text::new("Create a credential first – the key at the top")))
                .enabled_if(!ctx.looks.looks.is_empty(), Some(Text::new("Install a look first – the Looks tab")))
                .enabled_if(base_without.is_none(), Some(Text::new("Assign a credential to the macOS base first")))
                .enabled_if(r.is_some(), None)
                .enabled_if(r.is_none_or(|r| r.runs_here), r.map(|r| super::recipes::not_here(ctx, &r.systems)))
                .enabled_if(blockers.is_empty(), Some(tf!("Open the Doctor first: %@", blocker_titles(&blockers))))
                .enabled_if(!missing, Some(Text::new("Install what the recipe is made of first")))
                .enabled_if(
                    !mac || has_base,
                    Some(Text::new(
                        "No macOS base yet – it is needed for macOS images and is created from Apple's macOS once.",
                    )),
                )
                .enabled_if(
                    !mac || !has_base || base.is_some(),
                    Some(tf!("No macOS base for %@ yet", &credential_name)),
                )
                .primary();
            actions.push(build);
        }
    }

    let (status, progress_view, outcome) = match &job {
        Some(j) if j.state == JobState::Queued => (Some(Text::new("Waiting in the queue")), None, None),
        Some(j) if active => (None, Some(progress(j)), None),
        Some(j) => (None, None, outcome(ctx, j)),
        None => (None, None, None),
    };
    let log = job.as_ref().map(log_ref);
    BuildPage {
        title: Text::new("Build Image"),
        recipe: p.recipe.clone(),
        name: if p.recipe.is_empty() { "–".into() } else { recipe_name(ctx, &p.recipe) },
        kind: kind_badge(kind),
        badges,
        description: r.and_then(|r| r.description.clone()),
        state: job.as_ref().map(state_of).unwrap_or(BuildState::Idle),
        actions,
        fields: if active { vec![] } else { fields },
        blockers: if active { None } else { blockers_of(&blockers, ctx.host) },
        mac_base: if !mac || active {
            None
        } else if !has_base {
            Some(
                Hint::new(
                    "macos",
                    Text::new(
                        "No macOS base yet – it is needed for macOS images and is created from Apple's macOS once.",
                    ),
                    Tone::Normal,
                )
                .action(create_base_for(&credential, &credential_name)),
            )
        } else if base.is_none() {
            // Other credentials have bases, this one has none (plan B166)
            Some(
                Hint::new(
                    "macos",
                    tf!("No macOS base for %@ yet – each credential has its own base with its user and password; it is created from Apple's macOS once.", &credential_name),
                    Tone::Normal,
                )
                .action(create_base_for(&credential, &credential_name)),
            )
        } else {
            base_without.map(|b| {
                Hint::new(
                    "macos",
                    tf!("%@ has no credential yet – assign the one its user was created with; then images are built without asking for a password.", &b.name),
                    Tone::Warning,
                )
                .action(Action::show(
                    "assign",
                    Text::new("Assign Credential …"),
                    Some("key"),
                    "baseCredential",
                    json!({"name": b.name}),
                ))
            })
        },
        notes: if active { vec![] } else { notes },
        status,
        progress: progress_view,
        outcome,
        newest: if job.is_none() { r.and_then(|r| r.image.as_ref()).map(|i| tf!("newest image: %@", i)) } else { None },
        log,
        job,
    }
}

/// "Create Base for <credential> …": the macBase page with that credential chosen (plan B166)
fn create_base_for(credential: &str, name: &str) -> Action {
    Action::show(
        "macBase",
        tf!("Create Base for %@ …", name),
        Some("macos"),
        "macBase",
        json!({"values": {"credential": credential}}),
    )
}

/// A job's log as a page shows it: where to read it, the note on hidden lines once it ended, the whole file
pub fn log_ref(j: &JobDto) -> LogRef {
    LogRef {
        job: j.id.clone(),
        from: j.log_lines.saturating_sub(crate::api::jobs::MAX_LINES),
        lines: j.log_lines,
        note: (!j.state.is_active() && j.hidden_lines > 0).then(|| {
            tf!(
                "(%lld routine lines of the disk image tool hidden – full log: %@)",
                j.hidden_lines,
                j.log_file.clone().unwrap_or_default()
            )
        }),
        file: j.log_file.clone(),
        full: j.log_file.as_ref().map(|f| {
            Action::call("fullLog", Text::new("Full Log …"), Some("log"), "ui.open", json!({"path": f}))
                .help(Text::new("Every line, also the routine ones left out above"))
        }),
    }
}

fn outcome(ctx: &Ctx, j: &JobDto) -> Option<Outcome> {
    Some(match j.state {
        JobState::Finished => {
            let image = j.result.clone().and_then(|r| serde_json::from_value::<BuildResult>(r).ok()).map(|r| r.image);
            let known = image.as_ref().and_then(|name| ctx.images.iter().find(|i| &i.name == name));
            Outcome {
                state: BuildState::Finished,
                icon: "finished".into(),
                tone: Tone::Accent,
                title: Text::new("Image ready"),
                detail: image
                    .as_ref()
                    .map(|name| Text::verbatim(known.map(|i| i.label.clone()).unwrap_or_else(|| name.clone()))),
                action: known.map(|i| {
                    Action::show("newVm", Text::new("New VM …"), Some("new-vm"), "newVm", json!({"image": i.name}))
                        .primary()
                }),
            }
        }
        JobState::Failed => Outcome {
            state: BuildState::Failed,
            icon: "failed".into(),
            tone: Tone::Danger,
            title: Text::new("Build failed"),
            detail: j.error_text.clone().or_else(|| j.error.as_ref().map(Text::verbatim)),
            action: None,
        },
        JobState::Cancelled => Outcome {
            state: BuildState::Cancelled,
            icon: "cancelled".into(),
            tone: Tone::Normal,
            title: Text::new("Build cancelled"),
            detail: None,
            action: None,
        },
        _ => return None,
    })
}
