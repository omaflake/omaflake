// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Volumes tab (`volumes`, plan B187; German "Platten") – folders of this computer that VMs share – and its two
//! forms: a new volume (`volume`) and the name a VM sees one under (`volumeMount`). The list has the Credentials
//! page's shape, so every UI draws it as it draws that: each volume with its folder, its VMs as lines of their own
//! (the name each sees it under, "Rename …", "Take Out") and "Add VM" right there, as in the Networks tab.

use serde::Deserialize;
use serde_json::json;

use super::Ctx;
use super::common::*;
use super::credentials::{ActionMenu, CredentialDetail, CredentialPage, CredentialRow, CredentialsPage, DetailItem};
use super::text::{Text, tf};
use crate::api::types::{Vm, VolumeDto};
use crate::models::{GuestKind, VmStatus};
use crate::volumes::Volumes;

fn kind_icon(v: Option<&Vm>) -> &'static str {
    match v.map(|v| v.kind) {
        Some(GuestKind::Macos) => "macos",
        Some(GuestKind::Nixos) => "nixos",
        None => "vm",
    }
}

/// A VM that has the volume, as a line of its detail
fn vm_item(ctx: &Ctx, v: &VolumeDto, vm: &str, mount: &str) -> DetailItem {
    let known = ctx.vms.iter().find(|x| x.name == vm);
    let running = known.is_some_and(|x| x.status == VmStatus::Running);
    let shown = known.and_then(|x| x.volumes.iter().find(|m| m.volume == v.name));
    let named = shown.is_some_and(|m| m.named);
    let subtitle = if named {
        tf!("/Volumes/%@ · named for this VM", mount)
    } else {
        tf!("/Volumes/%@ · the folder's name", mount)
    };
    let ids = json!({"volume": v.name, "vm": vm});
    DetailItem {
        id: vm.to_string(),
        name: vm.to_string(),
        icon: kind_icon(known).into(),
        lamp: Lamp::new(
            if running { LampState::On } else { LampState::Off },
            if running { "Running" } else { "Not running" },
        ),
        subtitle,
        badges: vec![],
        actions: vec![
            Action::show(&format!("rename-{vm}"), Text::new("Rename …"), Some("edit"), "volumeMount", ids.clone())
                .help(tf!("The name %@ sees the volume under", vm)),
            Action::call(&format!("detach-{vm}"), Text::new("Take Out"), Some("minus"), "volumes.detach", ids)
                .help(tf!("Take %@ out of %@ – from its next start", vm, v.name.as_str())),
        ],
    }
}

fn add_menu(ctx: &Ctx, v: &VolumeDto) -> ActionMenu {
    let actions: Vec<Action> = ctx
        .vms
        .iter()
        .filter(|x| !v.vms.iter().any(|m| m.vm == x.name))
        .map(|x| {
            Action::call(
                &format!("attach-{}", x.name),
                Text::verbatim(x.name.clone()),
                Some(kind_icon(Some(x))),
                "volumes.attach",
                json!({"volume": v.name, "vm": x.name}),
            )
            .help(tf!("Give %@ the volume %@ – from its next start", x.name.as_str(), v.name.as_str()))
        })
        .collect();
    ActionMenu {
        label: Text::new("Add VM"),
        icon: Some("add".into()),
        empty: actions
            .is_empty()
            .then(|| Text::new(if ctx.vms.is_empty() { "There is no VM yet" } else { "Every VM has this volume" })),
        actions,
    }
}

fn row(ctx: &Ctx, v: &VolumeDto) -> CredentialRow {
    let id = json!({"name": v.name});
    let subtitle = match v.vms.len() {
        0 => tf!("%@ · no VMs", &v.folder),
        1 => tf!("%@ · 1 VM", &v.folder),
        n => tf!("%@ · %lld VMs", &v.folder, n as i64),
    };
    let info = vec![
        InfoLine::new("Folder", Text::verbatim(v.folder.clone()))
            .note(Text::new(if v.custom { "a folder you chose" } else { "in omaflake's data folder" }), Tone::Normal),
    ];
    let reveal = Action::call(
        "reveal",
        Text::new(ctx.on_host("Show in Finder", "Show Folder")),
        Some("folder"),
        "volumes.reveal",
        id.clone(),
    )
    .then(Then::Reveal);
    let remove =
        Action::call("remove", Text::new("Remove …"), Some("trash"), "volumes.remove", id).destructive().confirm(
            Confirm::new(
                tf!("Remove the volume “%@”?", &v.name),
                Text::new("Its VMs lose it at their next start. Its folder and the files in it stay."),
            )
            .button("Remove")
            .destructive(),
        );
    CredentialRow {
        id: v.name.clone(),
        name: v.name.clone(),
        lamp: Lamp::new(if v.vms.is_empty() { LampState::Off } else { LampState::On }, "VMs have it"),
        subtitle,
        badges: vec![],
        used_by: v.vms.iter().map(|m| m.vm.clone()).collect(),
        actions: vec![],
        detail: CredentialDetail {
            info,
            items: v.vms.iter().map(|m| vm_item(ctx, v, &m.vm, &m.mount)).collect(),
            add: Some(add_menu(ctx, v)),
            actions: vec![reveal, remove],
        },
    }
}

pub(crate) fn page(ctx: &Ctx) -> CredentialsPage {
    CredentialsPage {
        title: Text::new("Volumes"),
        intro: Text::new(
            "Volumes are folders of this computer that VMs share: a VM has none, one or several, and several VMs can have the same one. A VM sees each at /Volumes/<name> – by default the folder's name, or one you give it for that VM. Add a VM with “Add VM” under the volume; “Take Out” on its line takes it out. A VM gets its volumes at its next start.",
        ),
        rows: ctx.volumes.iter().map(|v| row(ctx, v)).collect(),
        add: Action::show("add", Text::new("New Volume …"), Some("add"), "volume", json!({})).primary(),
        footer: Text::new(
            "Removing a volume never deletes its folder: your files stay. Every VM with a volume can read and change all of it.",
        ),
    }
}

// MARK: Forms

#[derive(Debug, Clone, Default, Deserialize)]
pub struct VolumeValues {
    pub name: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct VolumeParams {
    #[serde(default)]
    pub values: VolumeValues,
}

/// `volume`: a new volume – its name and folder
pub(crate) fn volume_page(ctx: &Ctx, p: &VolumeParams) -> CredentialPage {
    let name = p.values.name.clone().unwrap_or_default();
    let trimmed = name.trim().to_string();
    let path = p.values.path.clone().filter(|s| !s.trim().is_empty());
    let mut field = Field::new("name", FieldKind::Text, Text::new("Name"), json!(name))
        .param("name")
        .value_param("name")
        .help(Text::new("Lowercase letters, digits and dashes, e.g. “projects”."));
    if trimmed.is_empty() {
        field.error = Some(Text::new("A volume needs a name"));
    } else if Volumes::validate_name(&trimmed).is_err() {
        field.error = Some(Text::new("1 to 31 lowercase letters, digits and dashes"));
    } else if ctx.volumes.iter().any(|v| v.name == trimmed) {
        field.error = Some(tf!("There is a volume \"%@\" already", &trimmed));
    }
    let mut folder =
        Field::new("path", FieldKind::Path, Text::new("Folder"), json!(path)).param("path").value_param("path");
    folder.placeholder = Some(Text::verbatim(format!(
        "{}/share/{}",
        ctx.settings.data_folder,
        if trimmed.is_empty() { "<name>" } else { trimmed.as_str() }
    )));
    folder.ask = Some(
        Ask::new("path", AskKind::Folder, Text::new("Choose …"))
            .prompt(Text::new("Folder for the volume"))
            .button("Choose"),
    );
    folder.help = Some(Text::new("Default"));
    let error = field.error.clone();
    let mut params = json!({"name": trimmed});
    if let Some(p) = &path {
        params["path"] = json!(p);
    }
    let create = Action::call("create", Text::new("Create"), Some("create"), "volumes.create", params)
        .then(Then::Back)
        .enabled_if(error.is_none(), error)
        .primary();
    CredentialPage {
        title: Text::new("New Volume"),
        id: None,
        intro: Text::new(
            "A folder VMs share: a new one in omaflake's data folder, or one of yours. Give it to VMs afterwards.",
        ),
        fields: vec![field, folder],
        locked: None,
        actions: vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({})), create],
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MountValues {
    #[serde(rename = "as")]
    pub mount: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MountParams {
    pub volume: String,
    pub vm: String,
    #[serde(default)]
    pub values: MountValues,
}

/// `volumeMount`: the name a VM sees a volume under
pub(crate) fn mount_page(ctx: &Ctx, p: &MountParams) -> Option<CredentialPage> {
    let v = ctx.volumes.iter().find(|v| v.name == p.volume)?;
    let vm = ctx.vms.iter().find(|x| x.name == p.vm)?;
    let current = vm.volumes.iter().find(|m| m.volume == p.volume)?;
    let default = std::path::Path::new(&v.folder)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| Volumes::validate_mount(n).is_ok())
        .unwrap_or_else(|| v.name.clone());
    let value =
        p.values.mount.clone().unwrap_or_else(|| if current.named { current.mount.clone() } else { String::new() });
    let trimmed = value.trim().to_string();
    let shown = if trimmed.is_empty() { default.clone() } else { trimmed.clone() };
    let mut field = Field::new("as", FieldKind::Text, Text::new("Name in the VM"), json!(value))
        .param("as")
        .value_param("as")
        .help(tf!("%@ sees the volume at /Volumes/%@ – empty: the folder's name", &p.vm, &shown));
    field.placeholder = Some(Text::verbatim(&default));
    if !trimmed.is_empty() && Volumes::validate_mount(&trimmed).is_err() {
        field.error = Some(Text::new("1 to 31 characters, no “/”, “,”, “=” or “:”, not starting with “.”"));
    } else if vm.volumes.iter().any(|m| m.volume != p.volume && m.mount.eq_ignore_ascii_case(&shown)) {
        field.error = Some(tf!("%@ has another volume at /Volumes/%@", &p.vm, &shown));
    }
    let error = field.error.clone();
    let save = Action::call(
        "save",
        Text::new("Save"),
        Some("apply"),
        "volumes.rename",
        json!({"volume": p.volume, "vm": p.vm, "as": trimmed}),
    )
    .then(Then::Back)
    .enabled_if(error.is_none(), error)
    .primary();
    Some(CredentialPage {
        title: tf!("“%@” in %@", &p.volume, &p.vm),
        id: Some(p.volume.clone()),
        intro: Text::new("The name the VM sees this volume under – from its next start."),
        fields: vec![field],
        locked: None,
        actions: vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({})), save],
    })
}
