// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Profiles page (`profiles`, plan B142) and the form of one (`profile`): how big a new VM is – CPU cores and RAM.
//! Same shapes as the Credentials page and form, so every UI draws them as it draws those. A new VM is created with
//! the default profile unless the New VM page chooses another (or custom values).

use serde::Deserialize;
use serde_json::json;

use super::Ctx;
use super::common::*;
use super::credentials::{CredentialDetail, CredentialPage, CredentialRow, CredentialsPage};
use super::text::{Text, tf};
use crate::api::types::ProfileDto;
use crate::profiles::Profiles;

fn size(p: &ProfileDto) -> Text {
    tf!("%lld CPU · %lld GB RAM", p.cpus, p.memory_gb)
}

fn row(p: &ProfileDto) -> CredentialRow {
    let mut badges = Vec::new();
    if p.is_default {
        badges.push(
            Badge::new("default", "ok", Text::new("New VMs get this profile unless another is chosen"), Tone::Accent)
                .label(Text::new("default")),
        );
    }
    let id = json!({"id": p.id});
    let edit = Action::show("edit", Text::new("Edit …"), None, "profile", id.clone()).primary();
    let default = Action::call("default", Text::new("Make Default"), Some("ok"), "profiles.setDefault", id.clone())
        .enabled_if(!p.is_default, Some(Text::new("Already the default")));
    let remove =
        Action::call("remove", Text::new("Remove …"), Some("trash"), "profiles.remove", id).destructive().confirm(
            Confirm::new(tf!("Remove the profile “%@”?", &p.name), Text::new("VMs created with it keep their size."))
                .button("Remove")
                .destructive(),
        );
    CredentialRow {
        id: p.id.clone(),
        name: p.name.clone(),
        lamp: Lamp::new(if p.is_default { LampState::On } else { LampState::Off }, "The default profile"),
        subtitle: size(p),
        badges,
        used_by: vec![],
        actions: if p.is_default { vec![] } else { vec![default.clone()] },
        detail: CredentialDetail {
            info: vec![
                InfoLine::new("CPU cores", Text::verbatim(p.cpus.to_string())),
                InfoLine::new("RAM", tf!("%lld GB", p.memory_gb)),
            ],
            actions: vec![edit, default, remove],
        },
    }
}

pub(crate) fn page(ctx: &Ctx) -> CredentialsPage {
    CredentialsPage {
        title: Text::new("Performance"),
        intro: Text::new(
            "How big a new VM is: its CPU cores and RAM. A new VM gets the default profile unless you choose another; without any, it gets what its recipe asks for. A recipe says what it needs at least – a smaller profile gets a warning.",
        ),
        rows: ctx.profiles.iter().map(row).collect(),
        add: Action::show("add", Text::new("New Profile …"), Some("add"), "profile", json!({})).primary(),
        footer: Text::new(
            "Profiles stay on this computer – they are never in a catalog. A VM's disk is its own: the recipe's size, changed on the New VM page.",
        ),
    }
}

/// The form's values (kept by the UI while the user chooses)
#[derive(Debug, Clone, Default, Deserialize)]
pub struct ProfileValues {
    pub name: Option<String>,
    pub cpus: Option<u32>,
    #[serde(rename = "memoryGB")]
    pub memory_gb: Option<u32>,
}

#[derive(Debug, Default, Deserialize)]
pub struct ProfileParams {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub values: ProfileValues,
}

/// `profile`: a new profile (4 CPU, 8 GB to start from) or one to edit
pub(crate) fn profile_page(ctx: &Ctx, p: &ProfileParams) -> Option<CredentialPage> {
    let current = match &p.id {
        Some(id) => Some(ctx.profiles.iter().find(|x| &x.id == id)?),
        None => None,
    };
    let v = &p.values;
    let name = v.name.clone().unwrap_or_else(|| current.map(|c| c.name.clone()).unwrap_or_default());
    let cpus = v.cpus.unwrap_or_else(|| current.map_or(4, |c| c.cpus));
    let memory = v.memory_gb.unwrap_or_else(|| current.map_or(8, |c| c.memory_gb));
    let mut name_field = Field::new("name", FieldKind::Text, Text::new("Name"), json!(name))
        .param("name")
        .value_param("name")
        .help(Text::new("Only for you – to tell your profiles apart when creating a VM."));
    let taken = ctx.profiles.iter().any(|x| Some(&x.id) != p.id.as_ref() && x.name.eq_ignore_ascii_case(name.trim()));
    if name.trim().is_empty() {
        name_field.error = Some(Text::new("A profile needs a name"));
    } else if taken {
        name_field.error = Some(tf!("There is a profile \"%@\" already", name.trim()));
    }
    let fields = vec![
        name_field.clone(),
        Field::new("cpus", FieldKind::Number, tf!("CPU cores: %lld", cpus), json!(cpus))
            .param("cpus")
            .value_param("cpus")
            .range(1, Profiles::MAX_CPUS as i64, 1),
        Field::new("memoryGB", FieldKind::Number, tf!("RAM: %lld GB", memory), json!(memory))
            .param("memoryGB")
            .value_param("memoryGB")
            .range(1, Profiles::MAX_MEMORY_GB as i64, 1),
    ];
    let error = name_field.error.clone();
    let mut params = json!({"name": name.trim(), "cpus": cpus, "memoryGB": memory});
    let save = match &p.id {
        Some(id) => {
            params["id"] = json!(id);
            Action::call("save", Text::new("Save"), Some("apply"), "profiles.update", params)
        }
        None => Action::call("create", Text::new("Create"), Some("create"), "profiles.add", params),
    }
    .then(Then::Back)
    .enabled_if(error.is_none(), error)
    .primary();
    Some(CredentialPage {
        title: Text::new(if p.id.is_some() { "Edit Profile" } else { "New Profile" }),
        id: p.id.clone(),
        intro: Text::new("CPU cores and RAM for new VMs. VMs created with it keep their size when it changes."),
        fields,
        locked: None,
        actions: vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({})), save],
    })
}
