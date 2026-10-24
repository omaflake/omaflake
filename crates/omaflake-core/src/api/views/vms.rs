// SPDX-License-Identifier: AGPL-3.0-or-later
//! The VMs tab (`vms`) and one VM (`vm`): what the app's `InstanceRow` and `InstanceDetail` showed.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::*;
use super::text::{tf, Arg, Text};
use super::Ctx;
use crate::agent_protocol::{AgentState, AgentStatus};
use crate::api::types::Vm;
use crate::models::{GuestKind, NetworkMode, VmStatus};

/// `vms`: the list of VMs
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmsPage {
    #[serde(flatten)]
    pub head: ListHead,
    pub rows: Vec<VmRowView>,
    /// VM files this omaflake cannot read (plan B114)
    pub unreadable: Vec<Hint>,
}

/// One VM: its row and, below it, its details
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmRowView {
    pub name: String,
    pub lamp: Lamp,
    /// The guest kind's symbol
    pub kind: Badge,
    /// What it has switched on, at a glance (off = no badge), the external drive, a newer image
    pub badges: Vec<Badge>,
    /// "dev · 4 CPU · 8 GB · SSH 2201"
    pub subtitle: Text,
    /// The row's buttons, left to right (share folder, start headless, start in window – or terminal, stop)
    pub actions: Vec<Action>,
    pub detail: VmDetail,
    /// The VM as `vms.get` gives it
    pub vm: Vm,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmDetail {
    /// Clipboard, sound, microphone, camera as switches (`vms.set`)
    pub switches: Vec<Action>,
    /// "Changes take effect at the next start." while it runs
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub switches_note: Option<Text>,
    pub info: Vec<InfoLine>,
    /// Export on the left; stop listing, upgrade, reset, remove on the right
    pub actions: Vec<Action>,
}

/// `vm`: one VM (a TUI's detail screen); `params: {name}`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmPage {
    /// The VM's name
    pub title: Text,
    pub row: VmRowView,
}

#[derive(Debug, Default, Deserialize)]
pub struct VmsParams {
    #[serde(default)]
    pub filter: FilterParams,
}

pub(crate) fn page(ctx: &Ctx, p: &VmsParams) -> VmsPage {
    let f = &p.filter;
    let all = &ctx.vms;
    let rows: Vec<VmRowView> = all
        .iter()
        .filter(|v| f.status.is_none_or(|s| s == (v.status == VmStatus::Running)) && f.kind.is_none_or(|k| k == v.kind))
        .map(|v| row(ctx, v))
        .collect();
    let mut head = ListHead::new("Virtual Machines", rows.len(), all.len());
    head.filter = Some(Filter { active: f.is_active(), groups: f.base_groups("Running", "Stopped") });
    head.actions = vec![
        Action::call(
            "import",
            Text::new("Import"),
            Some("import"),
            "ui.step",
            json!({"page": "import", "params": {"expect": "vm"}}),
        )
        .help(Text::new("Import a VM – a copy into the data folder"))
        .ask(Ask::packages("vm", false, "A VM (.omaflake-vm) – it is copied into the data folder", "Import")),
        Action::call(
            "external",
            Text::new("Run from There"),
            Some("external"),
            "ui.step",
            json!({"page": "import", "params": {"expect": "vm", "external": true}}),
        )
        .help(Text::new("Run a VM from where it is – an external disk, a NAS or a folder; nothing is copied"))
        .ask(Ask::packages(
            "vm",
            true,
            "A VM (.omaflake-vm) to run from where it is – nothing is copied",
            "Run from There",
        )),
    ];
    head.empty = if all.is_empty() {
        Some(Hint::new("vm", Text::new("No VMs yet – create one from an image."), Tone::Normal))
    } else if rows.is_empty() {
        Some(filtered_out("vms"))
    } else {
        None
    };
    VmsPage { head, rows, unreadable: unreadable(&ctx.unreadable_vms) }
}

pub fn unreadable(list: &[crate::api::types::Unreadable]) -> Vec<Hint> {
    list.iter()
        .map(|u| Hint::new("warning", Text::verbatim(format!("{}: {}", u.name, u.reason)), Tone::Warning))
        .collect()
}

/// "dev (words)" – a recipe as its rows name it (`AppModel.recipeName`)
pub(crate) fn recipe_name(ctx: &Ctx, id: &str) -> String {
    match ctx.recipes.recipes.iter().find(|r| r.id == id) {
        Some(r) => match &r.distinction {
            Some(d) => format!("{} ({d})", r.display_name),
            None => r.display_name.clone(),
        },
        None => crate::models::RecipeId::name_of_id(id),
    }
}

/// The guest kind's symbol with its help
pub fn kind_badge(kind: GuestKind) -> Badge {
    match kind {
        GuestKind::Macos => Badge::new("kind", "macos", Text::new("macOS guest"), Tone::Normal),
        GuestKind::Nixos => Badge::new("kind", "nixos", Text::new("NixOS guest"), Tone::Normal),
    }
}

/// Why a macOS guest cannot start now (`AppModel.macLimitReason`), None if it can
pub(crate) fn mac_limit(ctx: &Ctx) -> Option<Text> {
    let running: Vec<String> = ctx
        .vms
        .iter()
        .filter(|v| v.kind == GuestKind::Macos && v.status == VmStatus::Running)
        .map(|v| v.name.clone())
        .collect();
    (running.len() >= ctx.mac_max).then(|| {
        tf!("macOS allows at most %lld macOS VMs at the same time – running: %@", ctx.mac_max, running.join(", "))
    })
}

pub(crate) fn row(ctx: &Ctx, v: &Vm) -> VmRowView {
    let name = v.name.as_str();
    let running = v.status == VmStatus::Running;
    let busy = v.busy;
    let lamp = if busy {
        Lamp::new(LampState::Working, "Starting or stopping …")
    } else if running {
        Lamp::new(LampState::On, "Running")
    } else {
        Lamp::new(LampState::Off, "Stopped")
    };
    let mut badges = Vec::new();
    if let Some(location) = &v.location {
        badges.push(Badge::new("external", "external", Text::verbatim(location.clone()), Tone::Normal));
    }
    badges.extend(feature_badges(v));
    if v.upgradable {
        badges.push(Badge::new("upgradable", "upgrade", Text::new("Newer image available"), Tone::Warning));
    }
    let mut subtitle = vec![recipe_name(ctx, &v.recipe), format!("{} CPU", v.cpus), format!("{} GB", v.memory_gb)];
    let ssh = match v.kind {
        GuestKind::Macos => v.address.as_ref().map(|a| format!("SSH {a}")),
        GuestKind::Nixos => v.ssh_port.map(|p| format!("SSH {p}")),
    };
    subtitle.extend(ssh);

    let mut actions = Vec::new();
    if v.share {
        actions.push(
            Action::call(
                "showShare",
                Text::new("Show Share Folder"),
                Some("folder"),
                "vms.share",
                json!({"name": name}),
            )
            .then(Then::Reveal),
        );
    }
    let start_blockers = ctx.start_blockers(v.kind);
    if busy {
        // The lamp works; no buttons while an action runs
    } else if running {
        actions.push(
            Action::call(
                "terminal",
                Text::new("Open Terminal with SSH"),
                Some("terminal"),
                "vms.ssh",
                json!({"name": name}),
            )
            .then(Then::Terminal)
            .primary(),
        );
        actions.push(
            Action::call("stop", Text::new("Shut down"), Some("stop"), "vms.stop", json!({"name": name})).destructive(),
        );
    } else if !start_blockers.is_empty() {
        actions.push(Action::doctor_warning(&start_blockers));
    } else {
        let limit = if v.kind == GuestKind::Macos { mac_limit(ctx) } else { None };
        actions.push(
            Action::call(
                "startHeadless",
                Text::new("Start Headless"),
                Some("start-headless"),
                "vms.start",
                json!({"name": name, "display": "none"}),
            )
            .enabled_if(limit.is_none(), limit.clone()),
        );
        actions.push(
            Action::call(
                "startWindow",
                Text::new("Start in Window"),
                Some("start-window"),
                "vms.start",
                json!({"name": name, "display": "auto"}),
            )
            .enabled_if(limit.is_none(), limit)
            .primary(),
        );
    }
    VmRowView {
        name: name.into(),
        lamp,
        kind: kind_badge(v.kind),
        badges,
        subtitle: Text::verbatim(subtitle.join(" · ")),
        actions,
        detail: detail(ctx, v),
        vm: v.clone(),
    }
}

/// Clipboard, sound, microphone (orange: it listens), camera, ephemeral, network other than shared (`FeatureIcons`)
fn feature_badges(v: &Vm) -> Vec<Badge> {
    let mut b = Vec::new();
    if v.clipboard {
        b.push(Badge::new("clipboard", "clipboard", Text::new("Shared clipboard on"), Tone::Normal));
    }
    if v.audio {
        b.push(Badge::new("sound", "sound", Text::new("Sound on"), Tone::Normal));
    }
    if v.microphone {
        b.push(Badge::new("microphone", "microphone", Text::new("Microphone on"), Tone::Warning));
    }
    if v.camera && v.kind == GuestKind::Nixos {
        b.push(Badge::new("camera", "camera", Text::new("Camera on"), Tone::Warning));
    }
    if v.ephemeral {
        b.push(Badge::new("ephemeral", "ephemeral", Text::new("Ephemeral instance"), Tone::Normal));
    }
    match v.network {
        NetworkMode::Isolated => {
            b.push(Badge::new("network", "network-isolated", Text::new("Network isolated"), Tone::Normal))
        }
        NetworkMode::None => b.push(Badge::new("network", "network-none", Text::new("No network"), Tone::Normal)),
        NetworkMode::Shared => {}
    }
    b
}

fn detail(ctx: &Ctx, v: &Vm) -> VmDetail {
    let name = v.name.as_str();
    let mac = v.kind == GuestKind::Macos;
    let running = v.status == VmStatus::Running;
    let set = |id: &str, label: &str, icon: &str, on: bool, params: serde_json::Value| {
        Action::call(id, Text::new(label), Some(icon), "vms.set", params).switch(on).enabled_if(!v.busy, None)
    };
    let mut microphone = json!({"name": name, "microphone": !v.microphone});
    if !v.microphone {
        // The microphone needs sound
        microphone["audio"] = json!(true);
    }
    let switches = vec![
        set("clipboard", "Clipboard", "clipboard", v.clipboard, json!({"name": name, "clipboard": !v.clipboard}))
            .help(Text::new("Shared Clipboard")),
        set("sound", "Sound", "sound", v.audio, json!({"name": name, "audio": !v.audio})),
        set("microphone", "Microphone", "microphone", v.microphone, microphone),
        set("camera", "Camera", "camera", v.camera && !mac, json!({"name": name, "camera": !v.camera}))
            .enabled_if(!mac, Some(Text::new("The camera for macOS guests comes with a signed release"))),
    ];

    let n = ctx.recipes.id_lengths.vm;
    let mut info = Vec::new();
    let mut image = InfoLine::new("Image", Text::verbatim(&v.image)).full(&v.image);
    if v.upgradable {
        image = image.note(Text::new("newer image available"), Tone::Warning);
    }
    info.push(image);
    info.push(InfoLine::new("omaflake ID", Text::verbatim(short_id(&v.omaflake_id, n))).full(&v.omaflake_id));
    let recipe_label = ctx
        .recipes
        .recipes
        .iter()
        .find(|r| r.id == v.recipe)
        .map(|r| r.label.clone())
        .unwrap_or_else(|| crate::models::RecipeId::qualified(&v.recipe));
    info.push(InfoLine::new("Recipe", Text::verbatim(recipe_label)).full(&v.recipe));
    info.push(InfoLine::new(
        "Resources",
        Text::verbatim(format!("{} CPU · {} GB RAM · {} GB", v.cpus, v.memory_gb, v.disk_gb)),
    ));
    // Linux hosts (plan B124): KVM, or TCG – emulated, many times slower – when /dev/kvm cannot be used; a Mac's HVF is
    // not shown
    match v.accelerator.as_deref() {
        Some("kvm") => info.push(InfoLine::new("Accelerator", Text::verbatim("KVM"))),
        // No usable /dev/kvm, or forced with OMAFLAKE_ACCEL=tcg – the Doctor tells which (the page does not look at the
        // host: the same model on every machine)
        Some("tcg") => info.push(
            InfoLine::new("Accelerator", Text::verbatim("TCG"))
                .note(Text::new("emulated, many times slower – without KVM (see the Doctor)"), Tone::Warning),
        ),
        _ => {}
    }
    info.push(InfoLine::new("Network", Text::verbatim(v.network.as_str())));
    let ssh = match ctx.ssh.get(name) {
        Some(line) => Text::verbatim(line.clone()),
        None if mac => Text::new("address while running"),
        None => Text::verbatim("…"),
    };
    let full = ctx.ssh.get(name).cloned();
    let mut ssh_line = InfoLine::new("SSH", ssh);
    ssh_line.full = full;
    info.push(ssh_line);
    if let (true, Some(dir)) = (v.share, &v.share_path) {
        info.push(InfoLine::new("Share folder", Text::verbatim(dir)).full(dir));
    }
    if v.ephemeral {
        info.push(InfoLine::new("Ephemeral", Text::new("changes are discarded on power-off")));
    }
    info.push(InfoLine::new("Created", Text::fmt("%@", vec![Arg::date(v.created)])));
    if let Some(location) = &v.location {
        info.push(InfoLine::new("Runs from", Text::verbatim(location)).full(location));
    }
    if let Some(agent) = v.agent.as_ref().and_then(|a| serde_json::from_value::<AgentState>(a.clone()).ok()) {
        if let Some(hello) = &agent.hello {
            let line = if agent.agent_is_older() {
                InfoLine::new("Agent", Text::verbatim(&hello.agent))
                    .note(Text::new("older than this omaflake – rebuild the image"), Tone::Warning)
            } else {
                let os = hello.os_name.clone().or_else(|| hello.os.clone()).unwrap_or_default();
                InfoLine::new("Agent", Text::verbatim(format!("{} · {os}", hello.agent)))
            };
            info.push(line);
            if let Some(status) = &agent.status {
                let line = guest_line(status);
                let full = line.text.clone();
                info.push(InfoLine::new("Guest", line).full(full));
            }
        }
    }

    let stopped_reason = if running { Some(Text::new("Shut the VM down first")) } else { None };
    let free = !running && !v.busy;
    let mut actions = vec![Action::call(
        "export",
        Text::new("Export …"),
        Some("export"),
        "packages.export",
        json!({"kind": "vm", "name": name}),
    )
    .ask(Ask::export_folder(tf!(
        "Where to put “%@” – an external disk, a NAS or a folder. It goes along with its user, SSH keys and data: keep it for yourself.",
        name
    )))];
    if v.location.is_some() {
        actions.push(Action::call(
            "forget",
            Text::new("Stop Listing"),
            Some("hide"),
            "vms.forget",
            json!({"name": name}),
        ));
    }
    if let (true, Some(latest)) = (v.upgradable, &v.latest_image) {
        actions.push(
            Action::call(
                "upgrade",
                Text::new("Upgrade to New Image …"),
                Some("upgrade"),
                "vms.upgrade",
                json!({"name": name, "image": latest}),
            )
            .confirm(Confirm::new(
                tf!("Upgrade “%@” to %@?", name, latest),
                Text::new("The current state is deleted and cannot be restored; settings are kept."),
            )),
        );
    }
    actions.push(
        Action::call("reset", Text::new("Reset …"), Some("reset"), "vms.reset", json!({"name": name})).confirm(
            Confirm::new(
                tf!("Reset “%@”?", name),
                Text::new("The current state of the VM is deleted and cannot be restored."),
            ),
        ),
    );
    let remove_text = match &v.location {
        Some(location) => tf!(
            "The VM runs from %@ – it is deleted there and cannot be restored. Its share folder is kept if it holds files. To keep it there, choose Stop Listing instead.",
            location
        ),
        None => Text::new("The VM and its disk are deleted and cannot be restored. Its share folder is kept if it holds files."),
    };
    actions.push(
        Action::call("remove", Text::new("Remove …"), Some("trash"), "vms.remove", json!({"name": name}))
            .destructive()
            .confirm(Confirm::new(tf!("Remove “%@”?", name), remove_text).button("Remove").destructive()),
    );
    let actions = actions.into_iter().map(|a| a.enabled_if(free, stopped_reason.clone())).collect();
    VmDetail {
        switches,
        switches_note: running.then(|| Text::new("Changes take effect at the next start.")),
        info,
        actions,
    }
}

/// Load, free memory and disk, addresses, users – as the agent reported them (plan B96)
fn guest_line(s: &AgentStatus) -> Text {
    let mut parts = Vec::new();
    if let Some(load) = s.load.as_ref().and_then(|l| l.first()) {
        parts.push(tf!("load %@", format!("{load:.2}")));
    }
    if let Some(m) = &s.memory {
        parts.push(Text::fmt("%@ of %@ RAM free", vec![Arg::bytes(m.available as i64), Arg::bytes(m.total as i64)]));
    }
    if let Some(d) = &s.disk {
        parts.push(Text::fmt("%@ of %@ disk free", vec![Arg::bytes(d.free as i64), Arg::bytes(d.total as i64)]));
    }
    if let Some(a) = &s.addresses {
        let v4: Vec<&str> = a.iter().filter(|a| a.family == "ipv4").map(|a| a.address.as_str()).collect();
        if !v4.is_empty() {
            parts.push(Text::verbatim(v4.join(", ")));
        }
    }
    if let Some(u) = s.users.as_ref().filter(|u| !u.is_empty()) {
        parts.push(Text::verbatim(u.join(", ")));
    }
    if parts.is_empty() {
        return Text::verbatim("");
    }
    Text::joined(parts)
}

/// The first `count` words of an omaflake ID ("vm:research:amber-lake-quiet", 1 → "vm:research:amber"; `IDText.short`)
pub fn short_id(id: &str, count: usize) -> String {
    match id.rfind([':', '@']) {
        Some(cut) => format!("{}{}", &id[..=cut], crate::omaflake_id::OmaflakeId::short(&id[cut + 1..], count)),
        None => id.to_string(),
    }
}

#[derive(Debug, Deserialize)]
pub struct VmParams {
    pub name: String,
}

pub(crate) fn vm_page(ctx: &Ctx, p: &VmParams) -> Option<VmPage> {
    ctx.vms.iter().find(|v| v.name == p.name).map(|v| VmPage { title: Text::verbatim(&v.name), row: row(ctx, v) })
}
