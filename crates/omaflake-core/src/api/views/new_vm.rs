// SPDX-License-Identifier: AGPL-3.0-or-later
//! The New VM page (`newVm`): a VM from exactly one image (plan B100) – its recipe's defaults, the next free name, the
//! choices a guest kind allows, what is wrong with the values, and the create action (the app's `NewVMView`).

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::common::*;
use super::text::{tf, Text};
use super::vms::{kind_badge, mac_limit, recipe_name};
use super::Ctx;
use crate::api::types::{ImageDto, RecipeDto};
use crate::models::{GuestKind, NetworkMode};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewVmPage {
    /// "New VM"
    pub title: Text,
    pub image: String,
    pub recipe: String,
    pub kind: Badge,
    /// "recipe dev"
    pub subtitle: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub lab: bool,
    /// What the Doctor reports against starting VMs of this kind
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blockers: Option<Blockers>,
    /// In the app's sections: the name and ephemeral, resources and devices, the share folder, start afterwards
    pub sections: Vec<FormSection>,
    /// Said under the form (the macOS limit: created, not started)
    pub warnings: Vec<Text>,
    /// Every value is fine: `create` is enabled
    pub valid: bool,
    /// Cancel, Create
    pub actions: Vec<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FormSection {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<Text>,
    pub fields: Vec<Field>,
}

/// The values a form has now (the UI keeps them while the user edits); missing ones take the defaults
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NewVmValues {
    pub name: Option<String>,
    pub cpus: Option<u32>,
    #[serde(rename = "memoryGB")]
    pub memory_gb: Option<u32>,
    #[serde(rename = "diskGB")]
    pub disk_gb: Option<u32>,
    pub network: Option<NetworkMode>,
    pub share: Option<bool>,
    pub share_path: Option<String>,
    pub ephemeral: Option<bool>,
    pub audio: Option<bool>,
    pub microphone: Option<bool>,
    pub clipboard: Option<bool>,
    pub camera: Option<bool>,
    pub start: Option<bool>,
}

#[derive(Debug, Deserialize)]
pub struct NewVmParams {
    pub image: String,
    #[serde(default)]
    pub values: NewVmValues,
}

/// A recipe's values for a new VM; switches a recipe leaves open: sound and clipboard on, microphone and camera off
pub fn recipe_settings(r: Option<&RecipeDto>, i: &ImageDto) -> Value {
    match r {
        Some(r) => json!({
            "kind": r.kind, "cpus": r.cpus, "memoryGB": r.memory_gb, "diskGB": r.disk_gb, "network": r.network,
            "share": r.share, "ephemeral": r.ephemeral, "user": r.user,
            "audio": r.audio.unwrap_or(true), "microphone": r.microphone.unwrap_or(false),
            "clipboard": r.clipboard.unwrap_or(true), "camera": r.camera.unwrap_or(false), "display": r.display,
        }),
        // A recipe that is gone: the app's RecipeSettings() defaults
        None => json!({
            "kind": i.kind, "cpus": 4, "memoryGB": 8, "diskGB": 64, "network": "shared", "share": false,
            "ephemeral": false, "audio": true, "microphone": false, "clipboard": true, "camera": false,
        }),
    }
}

pub(crate) fn page(ctx: &Ctx, p: &NewVmParams) -> Option<NewVmPage> {
    let image = ctx.images.iter().find(|i| i.name == p.image)?;
    let recipe = ctx.recipes.recipes.iter().find(|r| r.id == image.recipe);
    let v = &p.values;
    let mut s = recipe_settings(recipe, image);
    // Null fields are left out (the server's RecipeSettings skips them)
    if let Some(o) = s.as_object_mut() {
        o.retain(|_, v| !v.is_null());
    }
    let mac = image.kind == GuestKind::Macos;
    macro_rules! take {
        ($field:ident, $key:expr) => {
            if let Some(x) = &v.$field {
                s[$key] = json!(x);
            }
        };
    }
    take!(cpus, "cpus");
    take!(memory_gb, "memoryGB");
    take!(disk_gb, "diskGB");
    take!(network, "network");
    take!(share, "share");
    take!(ephemeral, "ephemeral");
    take!(audio, "audio");
    take!(microphone, "microphone");
    take!(clipboard, "clipboard");
    take!(camera, "camera");
    if mac && s["network"] == json!("isolated") {
        // macOS guests have no isolated mode (plan BM27)
        s["network"] = json!("shared");
    }
    if s["audio"] == json!(false) {
        s["microphone"] = json!(false);
    }
    let get_bool = |k: &str| s[k].as_bool().unwrap_or(false);
    let get_int = |k: &str| s[k].as_i64().unwrap_or(0);

    let suggested = ctx.next_free_name(&image.recipe);
    let name = v.name.clone().unwrap_or_else(|| suggested.clone());
    let name_error = if name.is_empty() {
        Some(Text::new("A name is needed"))
    } else if let Err(e) = ctx.validate_name(&name) {
        // The instance store's rule, keyed where it is the rule (a too long socket path stays the core's text)
        let key = "Invalid name \"%@\": 1–32 characters a–z, 0–9, \"-\"";
        let keyed = tf!(key, &name);
        Some(if keyed.text == e { keyed } else { Text::verbatim(e) })
    } else if ctx.vms.iter().any(|vm| vm.name == name) {
        Some(tf!("“%@” is here already", &name))
    } else {
        None
    };
    let share = get_bool("share");
    let share_path = v.share_path.clone().filter(|p| !p.is_empty());
    let default_share =
        format!("{}/share/{}", ctx.settings.data_folder, if name.is_empty() { "<name>" } else { name.as_str() });
    let start = v.start.unwrap_or(true);

    let mut first = vec![
        Field::new("name", FieldKind::Text, Text::new("Name"), json!(name)).param("name").value_param("name"),
        Field::new(
            "ephemeral",
            FieldKind::Toggle,
            Text::new("Ephemeral (changes are discarded on shutdown)"),
            s["ephemeral"].clone(),
        )
        .param("settings.ephemeral")
        .value_param("ephemeral"),
    ];
    first[0].error = name_error.clone();
    first[0].placeholder = Some(Text::verbatim(&suggested));
    let mut resources = vec![
        Field::new("cpus", FieldKind::Number, tf!("CPU cores: %lld", get_int("cpus")), s["cpus"].clone())
            .param("settings.cpus")
            .value_param("cpus")
            .range(1, 10, 1),
        Field::new("memoryGB", FieldKind::Number, tf!("RAM: %lld GB", get_int("memoryGB")), s["memoryGB"].clone())
            .param("settings.memoryGB")
            .value_param("memoryGB")
            .range(1, 48, 1),
    ];
    if mac {
        // The clone keeps the image's disk (APFS resize in the guest: OPEN, plan BM27)
        resources.push(Field::new("diskGB", FieldKind::Info, Text::new("Disk"), json!(Text::new("as the image"))));
    } else {
        resources.push(
            Field::new("diskGB", FieldKind::Number, tf!("Disk: %lld GB", get_int("diskGB")), s["diskGB"].clone())
                .param("settings.diskGB")
                .value_param("diskGB")
                .range(10, 500, 10),
        );
    }
    resources.push(
        Field::new("audio", FieldKind::Toggle, Text::new("Sound from the VM"), s["audio"].clone())
            .param("settings.audio")
            .value_param("audio"),
    );
    resources.push(
        Field::new("microphone", FieldKind::Toggle, Text::new("Microphone in the VM"), s["microphone"].clone())
            .param("settings.microphone")
            .value_param("microphone")
            .enabled_if(get_bool("audio"), None),
    );
    resources.push(
        Field::new("clipboard", FieldKind::Toggle, Text::new("Shared clipboard"), s["clipboard"].clone())
            .param("settings.clipboard")
            .value_param("clipboard"),
    );
    if !mac {
        resources.push(
            Field::new("camera", FieldKind::Toggle, Text::new("Camera in the VM"), s["camera"].clone())
                .param("settings.camera")
                .value_param("camera"),
        );
    }
    let mut network = Field::new("network", FieldKind::Choice, Text::new("Network"), s["network"].clone())
        .param("settings.network")
        .value_param("network");
    network.choices = NetworkMode::ALL
        .iter()
        .filter(|n| !mac || **n != NetworkMode::Isolated)
        .map(|n| Choice::new(json!(n), Text::verbatim(n.as_str())))
        .collect();
    resources.push(network);

    let mut share_fields = vec![Field::new(
        "share",
        FieldKind::Toggle,
        Text::new(ctx.on_host("Share a Mac folder with the VM", "Share a folder with the VM")),
        json!(share),
    )
    .param("settings.share")
    .value_param("share")];
    if share {
        let mut folder = Field::new("sharePath", FieldKind::Path, Text::new("Folder"), json!(share_path))
            .param("sharePath")
            .value_param("sharePath");
        folder.placeholder = Some(Text::verbatim(default_share));
        folder.ask = Some(
            Ask::new("sharePath", AskKind::Folder, Text::new("Choose …"))
                .prompt(Text::new("Folder to share with the VM"))
                .button("Share"),
        );
        folder.help = Some(Text::new("Default"));
        share_fields.push(folder);
    }
    let start_field = Field::new("start", FieldKind::Toggle, Text::new("Start afterwards"), json!(start))
        .param("start")
        .value_param("start");

    let mut warnings = Vec::new();
    if let (true, true, Some(reason)) = (mac, start, mac_limit(ctx)) {
        warnings.push(tf!("%@ – the VM is created but not started.", reason));
    }
    let blockers = Blockers::of(&ctx.start_blockers(image.kind));
    let valid = name_error.is_none();
    let mut params = json!({"name": name, "image": image.name, "settings": s, "start": start});
    if let (true, Some(path)) = (share, &share_path) {
        params["sharePath"] = json!(path);
    }
    let create = Action::call("create", Text::new("Create"), Some("create"), "vms.create", params)
        .then(Then::Back)
        .enabled_if(valid, name_error)
        .primary();
    Some(NewVmPage {
        title: Text::new("New VM"),
        image: image.name.clone(),
        recipe: image.recipe.clone(),
        kind: kind_badge(image.kind),
        subtitle: tf!("recipe %@", recipe_name(ctx, &image.recipe)),
        description: recipe.and_then(|r| r.description.clone()),
        lab: recipe.is_some_and(|r| r.lab),
        blockers,
        sections: vec![
            FormSection { title: None, fields: first },
            FormSection { title: Some(Text::new("Resources")), fields: resources },
            FormSection { title: Some(Text::new("Share")), fields: share_fields },
            FormSection { title: None, fields: vec![start_field] },
        ],
        warnings,
        valid,
        actions: vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({})), create],
    })
}
