// SPDX-License-Identifier: AGPL-3.0-or-later
//! The New VM page (`newVm`): a VM from exactly one image (plan B100) – its recipe's defaults, the next free name, the
//! choices a guest kind allows, what is wrong with the values, and the create action (the app's `NewVMView`).

use serde_json::{Value, json};

use super::Ctx;
use super::common::*;
use super::text::{Text, tf};
use super::vms::{kind_badge, mac_limit, recipe_name};
use crate::api::types::{ImageDto, RecipeDto};
use crate::models::{GuestKind, NetworkMode};

// The page's model (plan B192: in omaflake-sdk); this file builds it
pub use omaflake_sdk::views::new_vm::*;

/// A recipe's values for a new VM; switches a recipe leaves open: sound and clipboard on, microphone, camera and MIDI
/// off
pub fn recipe_settings(r: Option<&RecipeDto>, i: &ImageDto) -> Value {
    match r {
        Some(r) => json!({
            "kind": r.kind, "cpus": r.cpus, "memoryGB": r.memory_gb, "diskGB": r.disk_gb, "network": r.network,
            "share": r.share, "ephemeral": r.ephemeral, "user": r.user,
            "audio": r.audio.unwrap_or(true), "microphone": r.microphone.unwrap_or(false),
            "clipboard": r.clipboard.unwrap_or(true), "camera": r.camera.unwrap_or(false),
            "midi": r.midi.unwrap_or(false), "display": r.display,
        }),
        // A recipe that is gone: the app's RecipeSettings() defaults
        None => json!({
            "kind": i.kind, "cpus": 4, "memoryGB": 8, "diskGB": 64, "network": "shared", "share": false,
            "ephemeral": false, "audio": true, "microphone": false, "clipboard": true, "camera": false, "midi": false,
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
    take!(midi, "midi");
    if !mac {
        take!(displays, "displays");
    }
    // The profile (plan B142): the chosen one, else the default; with one its CPU cores and RAM are the VM's
    let profile_id = v
        .profile
        .clone()
        .or_else(|| ctx.profiles.iter().find(|p| p.is_default).map(|p| p.id.clone()))
        .unwrap_or_default();
    let profile = ctx.profiles.iter().find(|p| p.id == profile_id);
    if let Some(p) = profile {
        s["cpus"] = json!(p.cpus);
        s["memoryGB"] = json!(p.memory_gb);
    }
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
    let share_path = v.share_path.clone().filter(|p| !p.is_empty());
    // Its volume (plan B187): none, a new one (by default when the recipe shares a folder) or one there is
    let volume = v.volume.clone().unwrap_or_else(|| if get_bool("share") { "+".into() } else { String::new() });
    let own = crate::volumes::Volumes::own_name(&name);
    let volume_name = v.volume_name.clone().unwrap_or_else(|| own.clone());
    let volume_name = volume_name.trim().to_string();
    let mount_as = v.mount_as.clone().unwrap_or_default().trim().to_string();
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
    let from_profile = profile.map(|p| tf!("From the profile %@ – choose custom values to change it", &p.name));
    let mut profile_field = Field::new("profile", FieldKind::Choice, Text::new("Profile"), json!(profile_id))
        .value_param("profile")
        .help(Text::new("CPU cores and RAM (Profiles, at the top)"));
    profile_field.choices = ctx
        .profiles
        .iter()
        .map(|p| Choice::new(json!(p.id), Text::verbatim(format!("{} – {} CPU · {} GB", p.name, p.cpus, p.memory_gb))))
        .chain(std::iter::once(Choice::new(json!(""), Text::new("Custom values"))))
        .collect();
    let mut resources = vec![
        Field::new("cpus", FieldKind::Number, tf!("CPU cores: %lld", get_int("cpus")), s["cpus"].clone())
            .param("settings.cpus")
            .value_param("cpus")
            .range(1, 10, 1)
            .enabled_if(profile.is_none(), from_profile.clone()),
        Field::new("memoryGB", FieldKind::Number, tf!("RAM: %lld GB", get_int("memoryGB")), s["memoryGB"].clone())
            .param("settings.memoryGB")
            .value_param("memoryGB")
            .range(1, 48, 1)
            .enabled_if(profile.is_none(), from_profile),
    ];
    if !ctx.profiles.is_empty() {
        resources.insert(0, profile_field);
    }
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
        // More monitors, each in a window of its own (plan B171); macOS VMs have one (Virtualization.framework)
        let displays = s["displays"].as_i64().unwrap_or(1);
        resources.push(
            Field::new("displays", FieldKind::Number, tf!("Displays: %lld", displays), json!(displays))
                .param("settings.displays")
                .value_param("displays")
                .range(1, crate::models::InstanceConfig::MAX_DISPLAYS as i64, 1)
                .help(Text::new("Each display in a window of its own")),
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
    // macOS too (plan B169): through OBS in the VM until omaflake's own camera extension
    let camera = Field::new("camera", FieldKind::Toggle, Text::new("Camera in the VM"), s["camera"].clone())
        .param("settings.camera")
        .value_param("camera");
    resources.push(if mac { camera.help(Text::new(super::vms::CAMERA_MACOS)) } else { camera });
    // MIDI (plan B161): every guest kind
    resources.push(
        Field::new("midi", FieldKind::Toggle, Text::new("MIDI devices in the VM"), s["midi"].clone())
            .param("settings.midi")
            .value_param("midi"),
    );
    let mut network = Field::new("network", FieldKind::Choice, Text::new("Network"), s["network"].clone())
        .param("settings.network")
        .value_param("network");
    network.choices = NetworkMode::ALL
        .iter()
        .filter(|n| !mac || **n != NetworkMode::Isolated)
        .map(|n| Choice::new(json!(n), Text::verbatim(n.as_str())))
        .collect();
    resources.push(network);

    let (share_fields, volume_error, volumes_param) = volume_fields(ctx, &volume, &volume_name, share_path, &mount_as);
    let start_field = Field::new("start", FieldKind::Toggle, Text::new("Start afterwards"), json!(start))
        .param("start")
        .value_param("start");

    let mut warnings = Vec::new();
    // Less than the recipe needs: a warning, the VM can still be created (plan B141)
    if let Some(req) = recipe.and_then(|r| r.parts.as_ref()).map(|p| p.requirements).filter(|r| r.cpus > 0) {
        let short = get_int("cpus") < req.cpus as i64
            || get_int("memoryGB") < req.memory_gb as i64
            || (!mac && get_int("diskGB") < req.disk_gb as i64);
        if short {
            warnings.push(tf!(
                "Its recipe needs at least %lld CPU cores, %lld GB RAM and a %lld GB disk – this VM gets less and may be slow or run out of space.",
                req.cpus,
                req.memory_gb,
                req.disk_gb
            ));
        }
    }
    if let (true, true, Some(reason)) = (mac, start, mac_limit(ctx)) {
        warnings.push(tf!("%@ – the VM is created but not started.", reason));
    }
    let blockers = blockers_of(&ctx.start_blockers(image.kind), ctx.host);
    let valid = name_error.is_none() && volume_error.is_none();
    let mut settings = s.clone();
    settings["share"] = json!(!volume.is_empty());
    let params =
        json!({"name": name, "image": image.name, "settings": settings, "start": start, "volumes": volumes_param});
    let create = Action::call("create", Text::new("Create"), Some("create"), "vms.create", params)
        .then(Then::Back)
        .enabled_if(valid, name_error.or(volume_error))
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
            FormSection { title: Some(Text::new("Volume")), fields: share_fields },
            FormSection { title: None, fields: vec![start_field] },
        ],
        warnings,
        valid,
        actions: vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({})), create],
    })
}

/// The New VM page's volume (plan B187): none, a new one – its name and folder – or one there is; and the name in the
/// VM. Returns the fields, what keeps it from being created, and `vms.create`'s `volumes`.
fn volume_fields(
    ctx: &Ctx,
    volume: &str,
    volume_name: &str,
    share_path: Option<String>,
    mount_as: &str,
) -> (Vec<Field>, Option<Text>, serde_json::Value) {
    let mut choices = vec![
        Choice::new(json!(""), Text::new("No volume")).icon("minus"),
        Choice::new(json!("+"), Text::new("A new volume")).icon("add"),
    ];
    choices
        .extend(ctx.volumes.iter().map(|v| Choice::new(json!(v.name), Text::verbatim(v.name.clone())).icon("folder")));
    let mut which = Field::new("volume", FieldKind::Choice, Text::new("Volume"), json!(volume))
        .value_param("volume")
        .help(Text::new(
            "A folder of this computer the VM shares – with other VMs too, if they have it. More in the Volumes tab.",
        ));
    which.choices = choices;
    let mut fields = vec![which];
    let mut error = None;
    let folder: Option<String> = if volume == "+" {
        let mut n = Field::new("volumeName", FieldKind::Text, Text::new("Name"), json!(volume_name))
            .value_param("volumeName")
            .help(Text::new("Lowercase letters, digits and dashes – the volume's name in omaflake"));
        if volume_name.is_empty() || crate::volumes::Volumes::validate_name(volume_name).is_err() {
            n.error = Some(Text::new("1 to 31 lowercase letters, digits and dashes"));
        } else if ctx.volumes.iter().any(|v| v.name == volume_name) {
            n.error = Some(tf!("There is a volume \"%@\" already", volume_name));
        }
        error = n.error.clone();
        fields.push(n);
        let default = format!(
            "{}/share/{}",
            ctx.settings.data_folder,
            if volume_name.is_empty() { "<name>" } else { volume_name }
        );
        let mut f =
            Field::new("sharePath", FieldKind::Path, Text::new("Folder"), json!(share_path)).value_param("sharePath");
        f.placeholder = Some(Text::verbatim(&default));
        f.ask = Some(
            Ask::new("sharePath", AskKind::Folder, Text::new("Choose …"))
                .prompt(Text::new("Folder for the volume"))
                .button("Choose"),
        );
        f.help = Some(Text::new("Default"));
        fields.push(f);
        Some(share_path.clone().unwrap_or(default))
    } else {
        ctx.volumes.iter().find(|v| v.name == volume).map(|v| v.folder.clone())
    };
    let Some(folder) = folder else { return (fields, error, json!([])) };
    let default_mount = std::path::Path::new(&folder)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .filter(|n| crate::volumes::Volumes::validate_mount(n).is_ok())
        .unwrap_or_else(|| if volume == "+" { volume_name.to_string() } else { volume.to_string() });
    let mut m = Field::new("mountAs", FieldKind::Text, Text::new("Name in the VM"), json!(mount_as))
        .value_param("mountAs")
        .help(tf!(
            "The VM sees it at /Volumes/%@",
            if mount_as.is_empty() { default_mount.as_str() } else { mount_as }
        ));
    m.placeholder = Some(Text::verbatim(&default_mount));
    if !mount_as.is_empty() && crate::volumes::Volumes::validate_mount(mount_as).is_err() {
        m.error = Some(Text::new("1 to 31 characters, no “/”, “,”, “=” or “:”, not starting with “.”"));
        error = error.or(m.error.clone());
    }
    fields.push(m);
    let mut entry = json!({"volume": if volume == "+" { volume_name } else { volume }});
    if !mount_as.is_empty() {
        entry["as"] = json!(mount_as);
    }
    if volume == "+" {
        entry["create"] = json!(true);
        if let Some(p) = &share_path {
            entry["path"] = json!(p);
        }
    }
    (fields, error, json!([entry]))
}
