// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Doctor (`doctor`, spec §11.2): versions and updates, the checks in three sections with their hints and what can
//! be done about each (the app's `DoctorView`, `RequirementRow`, `RequirementText`, `UpdateRow`); and the page that
//! creates the macOS base (`macBase`, the app's `MacBaseView`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::common::*;
use super::text::{tf, Arg, Text};
use super::Ctx;
use crate::api::types::{CliLinkDto, JobDto, JobKind, PrerequisiteDto, UpdateState};
use crate::prerequisites::{PrerequisiteItem as I, PrerequisiteStatus as S};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorPage {
    /// The page's title ("Doctor")
    pub title: Text,
    /// "omaflake" (the UI's own version) and "Core" (this server's)
    pub versions: Vec<InfoLine>,
    pub update: UpdateLine,
    pub intro: Text,
    /// The checks run now
    pub checking: bool,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::json::iso8601::option")]
    pub checked: Option<chrono::DateTime<chrono::Utc>>,
    /// "Check Again"
    pub actions: Vec<Action>,
    pub sections: Vec<DoctorSection>,
    /// Items that need attention (the warning sign on the Doctor's button)
    pub issues: usize,
}

/// "Updates": the last `updates.check`, with Download and Check Now
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateLine {
    pub label: Text,
    pub text: Text,
    pub tone: Tone,
    pub busy: bool,
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorSection {
    /// `requirements`, `nixos`, `macos`
    pub id: String,
    pub title: Text,
    pub items: Vec<DoctorItem>,
}

/// One check: status, title, what was found; a sub-line with the hint and the actions (open at first while there is a
/// problem or work runs)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorItem {
    pub item: crate::prerequisites::PrerequisiteItem,
    pub status: crate::prerequisites::PrerequisiteStatus,
    pub title: Text,
    /// Paths and versions as found (left out where the hint says it all)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<Text>,
    /// What to do, while not ok
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub hint: Option<Text>,
    /// It has a sub-line (a hint or actions)
    pub expandable: bool,
    /// The sub-line starts open
    pub expanded: bool,
    /// Its work runs (a download, the base, a Linux builder task)
    pub busy: bool,
    /// The running Linux builder task's title
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub task: Option<Text>,
    /// A download's bytes so far and in all
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<Download>,
    pub actions: Vec<Action>,
    pub prerequisite: PrerequisiteDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Download {
    pub done: i64,
    pub of: i64,
    /// "12.3 GB / 26.6 GB"
    pub text: Text,
}

/// The hint for a check; the base/recipe user mismatch names both users (`RequirementText.hint`)
pub fn hint(p: &PrerequisiteDto) -> Text {
    if let (I::MacBase, Some(base), Some(expected)) = (p.item, &p.base_user, &p.expected_user) {
        return tf!(
            "The base's user %@ differs from macUser %@ in your image flake's settings – set macUser to %@ in nix/local/settings.nix, or create the base again with %@.",
            base,
            expected,
            base,
            expected
        );
    }
    let limited = p.status == S::Limited;
    Text::new(match p.item {
        I::Host if limited => "macOS 27 is needed for automatic macOS bases.",
        I::Host => "omaflake needs a Mac with Apple silicon.",
        I::Nix => "Install Nix, or enter the path to the nix binary.",
        I::Flakes => {
            "Add “nix-command flakes” to experimental-features (nix-darwin: nix.settings.experimental-features)."
        }
        I::NixDarwin => "Set up nix-darwin – it provides the Linux builder for NixOS images.",
        I::LinuxBuilder => "Enable nix.linux-builder in your nix-darwin configuration (see the guide).",
        I::Qemu if limited => {
            "Works headless and in 2D – without 3D and without automatic window resolution; omaflake's QEMU adds both."
        }
        I::Qemu => "Needed for NixOS guests: install omaflake's QEMU, or enter a path.",
        I::Flake => "Choose the folder with your recipes (flake.nix).",
        I::SshKey => "Create a key – used for SSH into VMs, image builds and macOS bases.",
        I::Git => "Optional: without it, image names show “nogit” instead of the flake's revision.",
        I::Cli if limited => {
            "Another omaf comes first on your PATH – install this omaflake's, or remove the other one."
        }
        I::Cli => "Optional: install the omaf command to use omaflake in a terminal.",
        I::BuilderRunning => {
            "The Linux builder VM does not answer – it is started by nix-darwin (launchd: org.nixos.linux-builder)."
        }
        I::FlakeEval => "Your recipes do not evaluate – see the message.",
        I::NixSpace | I::MacSpace => "Not enough free space – free some up first.",
        I::Ipsw if limited => "Partly downloaded – continues where it stopped.",
        I::Ipsw => "Download macOS from Apple, or choose an IPSW file you already have.",
        I::MacBase if limited => "Unfinished – Create Base continues it; use the same password as before.",
        I::MacBase => "Create it once from the IPSW – about 5 minutes, no manual steps.",
    })
}

/// Paths and versions as found; space in the UI's units (`RequirementText.detail`)
fn detail(p: &PrerequisiteDto) -> Text {
    match (p.free_bytes, p.needed_bytes) {
        (Some(free), Some(needed)) => Text::fmt("%@ free, about %@ needed", vec![Arg::bytes(free), Arg::bytes(needed)]),
        _ => Text::verbatim(&p.detail),
    }
}

/// The IPSW download: on its own, or while the base's creation downloads
pub(crate) fn download_job(ctx: &Ctx) -> Option<&JobDto> {
    ctx.active_job(JobKind::IpswDownload)
        .or_else(|| ctx.active_job(JobKind::MacosInstall).filter(|j| j.progress.as_ref().is_some_and(|p| p.of > 0)))
}

pub fn download(j: &JobDto) -> Download {
    let (done, of) = j.progress.as_ref().map(|p| (p.done, p.of)).unwrap_or((0, 0));
    Download {
        done,
        of,
        text: if of > 0 { Text::fmt("%@ / %@", vec![Arg::bytes(done), Arg::bytes(of)]) } else { Text::verbatim("…") },
    }
}

fn builder_task(ctx: &Ctx) -> Option<&JobDto> {
    ctx.jobs.iter().rev().find(|j| {
        matches!(
            j.kind,
            JobKind::BuilderInstallNix | JobKind::BuilderSetup | JobKind::BuilderApply | JobKind::BuilderCleanUp
        ) && j.state.is_active()
    })
}

/// The `omaf` command's hint: a file in the way, or the link's folder missing from the PATH; else as `hint`
fn cli_hint(p: &PrerequisiteDto, cli: Option<&CliLinkDto>) -> Text {
    match cli {
        Some(c) if c.blocked && p.status != S::Ok => {
            tf!("%@ is there and is not omaflake's – remove or rename it, then install.", &c.link)
        }
        Some(c) if c.installed && !c.on_path && p.status != S::Ok => {
            let dir = c.link.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
            tf!("Add %@ to your PATH in your shell's profile (export PATH=\"%@:$PATH\").", &dir, &dir)
        }
        _ => hint(p),
    }
}

/// Install … and Uninstall … of the `omaf` command (plan B122)
fn cli_actions(ctx: &Ctx, p: &PrerequisiteDto, cli: Option<&CliLinkDto>) -> Vec<Action> {
    let Some(c) = cli else { return vec![] };
    let running = ctx.active_job(JobKind::CliInstall);
    let busy_reason = running.map(|j| Text::verbatim(&j.title));
    let mut actions = Vec::new();
    if let Some(target) = c.target.as_ref().filter(|_| p.status != S::Ok) {
        let text = if c.admin {
            tf!("omaflake creates the link %@ to its omaf (%@), so omaf works in Terminal. A link that is there already is replaced. macOS asks for an administrator.", &c.link, target)
        } else {
            tf!("omaflake creates the link %@ to its omaf (%@), so omaf works in a terminal. A link that is there already is replaced.", &c.link, target)
        };
        let mut a = Action::call("installCli", Text::new("Install …"), Some("install"), "doctor.installCli", json!({}))
            .confirm(Confirm::new(Text::new("Install the omaf command?"), text).button("Install"))
            .busy(running.is_some())
            .enabled_if(running.is_none(), busy_reason.clone())
            .enabled_if(!c.blocked, Some(Text::verbatim(&c.link)));
        a.help = p.install.as_ref().map(Text::verbatim);
        actions.push(a);
    }
    if c.installed {
        let text = if c.admin {
            tf!(
                "omaflake removes the link %@; omaf itself stays in omaflake. macOS asks for an administrator.",
                &c.link
            )
        } else {
            tf!("omaflake removes the link %@; omaf itself stays in omaflake.", &c.link)
        };
        actions.push(
            Action::call("uninstallCli", Text::new("Uninstall …"), Some("trash"), "doctor.uninstallCli", json!({}))
                .confirm(Confirm::new(Text::new("Uninstall the omaf command?"), text).button("Uninstall").destructive())
                .enabled_if(running.is_none(), busy_reason),
        );
    }
    actions
}

fn item(ctx: &Ctx, p: &PrerequisiteDto) -> DoctorItem {
    let report = ctx.doctor.as_ref();
    let task = builder_task(ctx);
    let downloading = download_job(ctx);
    let installing = ctx.active_job(JobKind::MacosInstall).is_some();
    let builder_actions = p.item == I::LinuxBuilder && p.status == S::Ok;
    let cli = report.and_then(|d| d.cli.as_ref()).filter(|_| p.item == I::Cli);
    let cli_installed = cli.is_some_and(|c| c.installed);
    let busy = (p.item == I::Ipsw && downloading.is_some())
        || (p.item == I::MacBase && installing)
        || (p.item == I::LinuxBuilder && task.is_some())
        || (p.item == I::Cli && ctx.active_job(JobKind::CliInstall).is_some());
    let has_subline = p.status != S::Ok || p.settable_path || busy || builder_actions || cli_installed;
    let task_busy = task.map(|t| Text::verbatim(&t.title));
    let no_task = |a: Action| a.enabled_if(task.is_none(), task_busy.clone());

    let mut actions = Vec::new();
    if p.item == I::Cli {
        actions.extend(cli_actions(ctx, p, cli));
    } else if p.item == I::Nix && p.status == S::Missing {
        let mut a = no_task(
            Action::call("installNix", Text::new("Install …"), Some("install"), "builder.installNix", json!({})).confirm(
                Confirm::new(
                    Text::new("Install Nix?"),
                    Text::new("omaflake installs Nix with the official installer of the NixOS community (nix-installer). It creates the /nix volume, Nix's build users and its background service, and needs the internet. Uninstall later with /nix/nix-installer uninstall."),
                )
                .button("Install"),
            ),
        );
        a.help = p.install.as_ref().map(Text::verbatim);
        actions.push(a);
    } else if p.item == I::NixDarwin && p.status == S::Missing {
        let nix_missing =
            report.is_some_and(|d| d.prerequisites.iter().any(|x| x.item == I::Nix && x.status == S::Missing));
        actions.push(
            no_task(
                Action::call("setUp", Text::new("Set Up …"), Some("install"), "builder.setup", json!({})).confirm(
                    Confirm::new(
                        Text::new("Set up nix-darwin and the Linux builder?"),
                        Text::new("omaflake sets up nix-darwin with its Linux builder host (nix/hosts/linux-builder.nix) – needed to build NixOS images. nix-darwin then manages /etc/nix/nix.conf; the files there that the Nix installer wrote are kept as *.before-nix-darwin. First the standard builder, then omaflake's settings; macOS asks for an administrator twice. This takes several minutes."),
                    )
                    .button("Set Up"),
                ),
            )
            .enabled_if(!nix_missing, Some(Text::new("Nix"))),
        );
    } else if let Some(command) = &p.install {
        actions.push(
            Action::call("install", Text::new("Install …"), Some("install"), "ui.terminal", json!({"line": command}))
                .help(Text::verbatim(command)),
        );
    }
    if builder_actions {
        actions.push(no_task(
            Action::call("cleanUp", Text::new("Clean Up …"), Some("clean"), "builder.cleanUp", json!({}))
                .help(Text::new("Removes unused data from the Linux builder's disk (asks for an administrator)")),
        ));
        let replaces = !report.is_some_and(|d| d.builder_system_is_omaflakes);
        let text = if replaces {
            "This Mac's nix-darwin system does not come from omaflake's image flake yet. Applying replaces your current nix-darwin configuration with omaflake's Linux builder host (nix/hosts/linux-builder.nix and your local/darwin.nix). Only continue if you set up nix-darwin for omaflake."
        } else {
            "Runs darwin-rebuild with omaflake's Linux builder host. This takes a few minutes; macOS asks for an administrator."
        };
        let params = if replaces { json!({"force": true}) } else { json!({}) };
        actions.push(no_task(
            Action::call("apply", Text::new("Apply Settings …"), Some("apply"), "builder.apply", params)
                .help(Text::new("darwin-rebuild switch with omaflake's Linux builder host (asks for an administrator)"))
                .confirm(
                    Confirm::new(Text::new("Apply Linux builder settings?"), Text::new(text)).button(if replaces {
                        "Replace and Apply"
                    } else {
                        "Apply"
                    }),
                ),
        ));
    }
    if matches!(p.item, I::LinuxBuilder | I::BuilderRunning) {
        actions.push(ctx.guide("linux-builder", "linux-builder", "Linux Builder Guide"));
    }
    if p.settable_path {
        let (kind, prompt) = match p.item {
            I::Flake => (AskKind::Folder, "Folder with the flake.nix of your recipes"),
            I::Nix => (AskKind::File, "The nix binary"),
            _ => (AskKind::File, "The qemu-system-aarch64 binary"),
        };
        let item = serde_json::to_value(p.item).unwrap_or_default();
        actions.push(
            Action::call("setPath", Text::new("Set Path …"), Some("folder"), "doctor.setPath", json!({"item": item}))
                .ask(Ask::new("path", kind, Text::new("Set Path …")).prompt(Text::new(prompt))),
        );
    }
    if p.item == I::Ipsw {
        match downloading {
            Some(j) => actions.push(Action::call(
                "pause",
                Text::new("Pause"),
                Some("pause"),
                "jobs.cancel",
                json!({"id": j.id}),
            )),
            None => actions.push(
                Action::call(
                    "download",
                    Text::new(if p.status == S::Limited { "Continue Download" } else { "Download" }),
                    Some("download"),
                    "macos.downloadIpsw",
                    json!({}),
                )
                .enabled_if(report.is_some_and(|d| d.latest_release.is_some()), None),
            ),
        }
    }
    if p.item == I::MacBase {
        actions.push(
            Action::show(
                "macBase",
                Text::new(if installing { "Show …" } else { "Create …" }),
                Some("macos"),
                "macBase",
                json!({}),
            )
            .busy(installing),
        );
    }
    let show_detail = p.status != S::Missing || matches!(p.item, I::FlakeEval | I::NixSpace | I::MacSpace | I::Ipsw);
    DoctorItem {
        item: p.item,
        status: p.status,
        title: Text::new(&p.title),
        detail: show_detail.then(|| detail(p)),
        hint: (p.status != S::Ok).then(|| if p.item == I::Cli { cli_hint(p, cli) } else { hint(p) }),
        expandable: has_subline,
        expanded: has_subline && (p.status != S::Ok || busy),
        busy,
        task: if p.item == I::LinuxBuilder { task.map(|t| Text::verbatim(&t.title)) } else { None },
        download: if p.item == I::Ipsw { downloading.map(download) } else { None },
        actions,
        prerequisite: p.clone(),
    }
}

pub(crate) fn page(ctx: &Ctx) -> DoctorPage {
    let report = ctx.doctor.as_ref();
    let section = |id: &str, title: &str, list: Option<&Vec<PrerequisiteDto>>| DoctorSection {
        id: id.into(),
        title: Text::new(title),
        items: list.map(|l| l.iter().map(|p| item(ctx, p)).collect()).unwrap_or_default(),
    };
    DoctorPage {
        title: Text::new("Doctor"),
        versions: vec![InfoLine::new("Core", Text::verbatim(&ctx.omaf_version))],
        update: update_line(ctx),
        intro: Text::new(ctx.on_host(
            "omaflake uses what your Mac already has. Set up missing parts here or enter their path.",
            "omaflake uses what this computer already has. Set up missing parts here or enter their path.",
        )),
        checking: ctx.checking,
        checked: report.map(|d| d.checked),
        actions: vec![Action::call("check", Text::new("Check Again"), Some("refresh"), "doctor.check", json!({}))
            .busy(ctx.checking)
            .enabled_if(!ctx.checking, None)],
        sections: vec![
            section("requirements", "Requirements", report.map(|d| &d.prerequisites)),
            section("nixos", "NixOS – before building images", report.map(|d| &d.nixos_preflight)),
            section("macos", "macOS – before building images", report.map(|d| &d.macos_preflight)),
        ],
        issues: report.map(|d| d.issues).unwrap_or(0),
    }
}

fn update_line(ctx: &Ctx) -> UpdateLine {
    let u = &ctx.update;
    let check = Action::call("checkNow", Text::new("Check Now"), Some("refresh"), "updates.check", json!({}));
    let (text, tone, actions) = if u.checking {
        (Text::verbatim("…"), Tone::Normal, vec![])
    } else if let Some(error) = &u.error {
        (tf!("Check failed: %@", error), Tone::Warning, vec![check])
    } else {
        match u.state {
            None | Some(UpdateState::NoFeed) => {
                (Text::new("No update feed yet (comes with the first release)"), Tone::Normal, vec![])
            }
            Some(UpdateState::Off) => (Text::new("Check switched off (Settings)"), Tone::Normal, vec![]),
            Some(UpdateState::UpToDate) => (
                Text::fmt("Up to date – checked %@", vec![Arg::date(u.checked.unwrap_or_else(crate::json::now))]),
                Tone::Normal,
                vec![check],
            ),
            Some(UpdateState::Available) => {
                let r = u.release.as_ref();
                let version = r.map(|r| r.version.clone()).unwrap_or_default();
                let url = r.map(|r| r.page.clone().unwrap_or_else(|| r.url.clone())).unwrap_or_default();
                (
                    tf!("%@ available", version),
                    Tone::Accent,
                    vec![
                        Action::call(
                            "download",
                            Text::new("Download …"),
                            Some("download"),
                            "ui.open",
                            json!({"url": url}),
                        ),
                        check,
                    ],
                )
            }
        }
    };
    UpdateLine { label: Text::new("Updates"), text, tone, busy: u.checking, actions }
}

// MARK: macOS base

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacBasePage {
    /// "Create macOS Base"
    pub title: Text,
    pub intro: Text,
    /// The base as the Doctor sees it: ready (success) or unfinished (warning)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base: Option<Hint>,
    /// "macOS from Apple (IPSW)": the file chosen or Apple's, the download – the IPSW's only place on the page (its
    /// actions set `values.ipsw` with `ui.field`)
    pub ipsw: IpswLine,
    /// The guest user and its full name
    pub fields: Vec<Field>,
    pub installing: bool,
    /// Create Base (asks the password twice)
    pub actions: Vec<Action>,
    /// The latest base creation's log (its job, as the build page's)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log: Option<super::build::LogRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct IpswLine {
    pub label: Text,
    pub text: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub download: Option<Download>,
    pub actions: Vec<Action>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MacBaseParams {
    #[serde(default)]
    pub values: MacBaseValues,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MacBaseValues {
    pub user: Option<String>,
    pub full_name: Option<String>,
    pub ipsw: Option<String>,
}

pub(crate) fn mac_base_page(ctx: &Ctx, p: &MacBaseParams) -> MacBasePage {
    let report = ctx.doctor.as_ref();
    let find = |item: I| report.and_then(|d| d.macos_preflight.iter().find(|x| x.item == item));
    let base_item = find(I::MacBase);
    let ipsw_item = find(I::Ipsw);
    let installing = ctx.active_job(JobKind::MacosInstall).is_some();
    let downloading = download_job(ctx);
    // The macOS recipes' guest user, so base and images agree from the start
    let default_user = ctx
        .recipes
        .recipes
        .iter()
        .find(|r| r.kind == crate::models::GuestKind::Macos)
        .and_then(|r| r.user.clone())
        .unwrap_or_else(|| ctx.host_user.clone());
    let user = p.values.user.clone().unwrap_or(default_user);
    let full_name = p.values.full_name.clone().unwrap_or_else(|| ctx.host_full_name.clone());
    let ipsw = p.values.ipsw.clone().filter(|s| !s.is_empty());
    let base = base_item.and_then(|b| match b.status {
        S::Ok => Some(Hint::new("ok", Text::verbatim(&b.detail), Tone::Success)),
        S::Limited => Some(Hint::new("warning", hint(b), Tone::Warning)),
        S::Missing => None,
    });
    let mut ipsw_actions = Vec::new();
    match downloading {
        Some(j) => ipsw_actions.push(Action::call(
            "pause",
            Text::new("Pause"),
            Some("pause"),
            "jobs.cancel",
            json!({"id": j.id}),
        )),
        None => {
            if ipsw.is_some() {
                let mut apple =
                    Action::field("useApple", Text::new("Use Apple's"), None, "macBase", json!({}), "ipsw", "ipsw");
                apple.params["value"] = Value::Null;
                ipsw_actions.push(apple);
            }
            ipsw_actions.push(
                Action::field(
                    "chooseIpsw",
                    Text::new("Choose IPSW …"),
                    Some("folder"),
                    "macBase",
                    json!({}),
                    "ipsw",
                    "ipsw",
                )
                .ask({
                    let mut a = Ask::new("value", AskKind::File, Text::new("Choose IPSW …"))
                        .prompt(Text::new("A macOS restore image (.ipsw) from Apple"));
                    a.accept = Some("ipsw".into());
                    a
                }),
            );
        }
    }
    let ipsw_text = match (&ipsw, ipsw_item) {
        (Some(path), _) => Text::verbatim(path.rsplit('/').next().unwrap_or(path)),
        (None, Some(i)) => Text::verbatim(&i.detail),
        (None, None) => Text::verbatim("…"),
    };
    let fields = vec![
        Field::new("user", FieldKind::Text, Text::new("Guest user"), json!(user))
            .param("user")
            .value_param("user")
            .enabled_if(!installing, None),
        Field::new("fullName", FieldKind::Text, Text::new("Full name"), json!(full_name))
            .param("fullName")
            .value_param("fullName")
            .enabled_if(!installing, None),
    ];
    let mut params = json!({"user": user, "fullName": full_name});
    if let Some(i) = &ipsw {
        params["ipsw"] = json!(i);
    }
    let mut password = Ask::new("password", AskKind::Password, Text::new("Password"));
    password.repeat = true;
    let create = Action::call("create", Text::new("Create Base"), Some("create"), "macos.installBase", params)
        .ask(password)
        .enabled_if(!installing, Some(Text::new("running …")))
        .enabled_if(downloading.is_none(), None)
        .enabled_if(!user.is_empty(), Some(Text::new("Guest user")))
        .enabled_if(base_item.is_none_or(|b| b.status != S::Ok), base_item.map(|b| Text::verbatim(&b.detail)))
        .primary();
    MacBasePage {
        title: Text::new("Create macOS Base"),
        intro: Text::new("Creates your macOS base from the IPSW: your guest user, automatic login, SSH with your keys from ~/.ssh. No manual steps; a missing IPSW is downloaded first."),
        base,
        ipsw: IpswLine {
            label: Text::new("macOS from Apple (IPSW)"),
            text: ipsw_text,
            download: downloading.map(download),
            actions: ipsw_actions,
        },
        fields,
        installing,
        actions: vec![create],
        log: ctx.jobs.iter().rev().find(|j| j.kind == JobKind::MacosInstall).map(super::build::log_ref),
    }
}
