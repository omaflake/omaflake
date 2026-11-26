// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Networks tab (`networks`, plan B170, spec §11.12) – networks between VMs – and its two forms: a new network
//! (`network`) and adding a VM to one (`networkMember`). The list has the Credentials page's shape, so every UI draws
//! it as it draws that; on top it carries `graph`: the host, every network, every VM and how they are connected –
//! each VM's own NAT to the host, each membership with the VM's address, and what went between which VMs while the
//! switch runs. UIs draw the graph as a picture (app, Linux app) or as an indented tree (`omaf ui`).

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::Ctx;
use super::HostOs;
use super::common::*;
use super::credentials::{CredentialDetail, CredentialPage, CredentialRow, CredentialsPage};
use super::text::{Arg, Text, tf};
use crate::api::types::Vm;
use crate::models::{GuestKind, NetworkMode, VmStatus};
use crate::networks::Network;
use crate::networks::switch::SwitchStatus;

/// A network as the pages see it: its record and what its running switch last wrote
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkInfo {
    pub network: Network,
    pub status: Option<SwitchStatus>,
}

/// The Networks tab: the list (Credentials page shape) and the graph
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworksPage {
    #[serde(flatten)]
    pub list: CredentialsPage,
    pub graph: NetworkGraph,
}

/// Who is connected to what (spec §11.12)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NetworkGraph {
    /// "This Mac" / "This computer"
    pub host: Text,
    pub networks: Vec<GraphNetwork>,
    pub vms: Vec<GraphVm>,
    pub links: Vec<GraphLink>,
    /// Traffic between two VMs on a network (while its switch runs)
    pub flows: Vec<GraphFlow>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphNetwork {
    pub id: String,
    pub label: String,
    /// "10.231.1.0/24"
    pub subnet: String,
    /// Its switch runs
    pub running: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphVm {
    pub id: String,
    pub label: String,
    /// "nixos" / "macos"
    pub kind: String,
    pub running: bool,
}

/// A VM's connection: `to` is a network's id, or "host" for the VM's own NAT
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphLink {
    pub vm: String,
    pub to: String,
    /// "network" (a card on one of omaflake's networks) or "nat" (the VM's own: SSH, the internet)
    pub kind: String,
    /// The VM's address on that network; for "nat": shared (internet) / isolated (no internet)
    pub label: Text,
    /// The card is connected to the running switch / the NAT has the internet
    pub active: bool,
    /// Bytes through the card since the switch started (in + out)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GraphFlow {
    pub network: String,
    pub from: String,
    pub to: String,
    pub bytes: u64,
}

fn running(vm: Option<&Vm>) -> bool {
    vm.is_some_and(|v| v.status == VmStatus::Running)
}

fn bytes(n: u64) -> Arg {
    Arg::bytes(n as i64)
}

/// A member's line in its network's detail: "docker  10.231.1.10 · connected – 12.0 KB in, 3.0 KB out"
fn member_line(info: &NetworkInfo, vm: &str, host: u8, known: Option<&Vm>) -> InfoLine {
    let address = info.network.address(host).to_string();
    let port = info.status.as_ref().and_then(|s| s.ports.iter().find(|p| p.vm.as_deref() == Some(vm)));
    let state = match (port, known) {
        (Some(p), _) => tf!("%@ · connected – %@ in, %@ out", &address, bytes(p.rx_bytes), bytes(p.tx_bytes)),
        (None, None) => tf!("%@ · no such VM any more", &address),
        (None, Some(v)) if v.status == VmStatus::Running => {
            tf!("%@ · not connected – started before it joined: restart it", &address)
        }
        (None, Some(_)) => tf!("%@ · not running", &address),
    };
    let mut line = InfoLine::new("", state);
    line.label = Text::verbatim(vm); // a VM's name, not a text to translate
    if port.is_some() { line.tone(Tone::Success) } else { line }
}

/// A VM as a switch of a network: on while it is a member; a click adds it (`networks.join`) or takes it out
/// (`networks.leave`). A running NixOS VM joins and leaves at once (plan B179), a macOS VM at its next start
pub(crate) fn member_switch(network: &str, v: &Vm, member: bool) -> Action {
    let now = v.status == VmStatus::Running && v.kind == GuestKind::Nixos;
    let help = match (member, now) {
        (true, true) => tf!("In %@ – a click takes it out now", network),
        (true, false) => tf!("In %@ – a click takes it out (from its next start)", network),
        (false, true) => tf!("Not in %@ – a click adds it now", network),
        (false, false) => tf!("Not in %@ – a click adds it (from its next start)", network),
    };
    Action::call(
        &format!("member-{}", v.name),
        Text::verbatim(v.name.clone()),
        Some(if v.kind == GuestKind::Macos { "macos" } else { "nixos" }),
        if member { "networks.leave" } else { "networks.join" },
        json!({"network": network, "vm": v.name}),
    )
    .switch(member)
    .help(help)
}

fn row(ctx: &Ctx, info: &NetworkInfo) -> CredentialRow {
    let n = &info.network;
    let id = json!({"name": n.name});
    let count = n.members.len();
    let subtitle = match count {
        0 => tf!("%@ · no VMs", n.subnet()),
        1 => tf!("%@ · 1 VM", n.subnet()),
        _ => tf!("%@ · %lld VMs", n.subnet(), count as i64),
    };
    let mut info_lines = vec![
        InfoLine::new("Subnet", Text::verbatim(n.subnet())),
        InfoLine::new("Switch", Text::verbatim(n.switch_address().to_string())).note(
            Text::new(if info.status.is_some() {
                "running"
            } else {
                "starts with the first VM of the network, ends a minute after the last"
            }),
            if info.status.is_some() { Tone::Success } else { Tone::Normal },
        ),
    ];
    for (vm, host) in &n.members {
        info_lines.push(member_line(info, vm, *host, ctx.vms.iter().find(|v| &v.name == vm)));
    }
    let docker: Vec<&String> = n
        .members
        .keys()
        .filter(|vm| ctx.vms.iter().any(|v| &&v.name == vm && v.recipe.contains(".nixos.docker")))
        .collect();
    for vm in docker {
        let address = n.member_address(vm).map(|a| a.to_string()).unwrap_or_default();
        info_lines.push(
            InfoLine::new("Docker", Text::verbatim(format!("DOCKER_HOST=tcp://{address}:2375")))
                .note(tf!("in the network's other VMs, for the Docker engine of %@", vm.as_str()), Tone::Normal),
        );
    }
    // One switch per VM (the maintainer, 2026-11-27: a form with one drop-down to add and a row of buttons to take
    // out were hard to read): on = in the network, a click adds or takes out. Members first, then the others
    let mut vms: Vec<&Vm> = ctx.vms.iter().collect();
    vms.sort_by_key(|v| !n.members.contains_key(&v.name));
    let mut actions: Vec<Action> =
        vms.iter().map(|v| member_switch(&n.name, v, n.members.contains_key(&v.name))).collect();
    // A member that is no VM any more can only go
    for vm in n.members.keys().filter(|m| !ctx.vms.iter().any(|v| &&v.name == m)) {
        actions.push(
            Action::call(
                &format!("member-{vm}"),
                Text::verbatim(vm.clone()),
                Some("network-none"),
                "networks.leave",
                json!({"network": n.name, "vm": vm}),
            )
            .switch(true)
            .help(Text::new("No such VM any more – a click takes it out")),
        );
    }
    actions.push(
        Action::call("remove", Text::new("Remove …"), Some("trash"), "networks.remove", id).destructive().confirm(
            Confirm::new(
                tf!("Remove the network “%@”?", &n.name),
                Text::new("Its VMs lose that card at their next start; their own network stays."),
            )
            .button("Remove")
            .destructive(),
        ),
    );
    CredentialRow {
        id: n.name.clone(),
        name: n.name.clone(),
        lamp: Lamp::new(
            if info.status.is_some() { LampState::On } else { LampState::Off },
            "The network's switch runs",
        ),
        subtitle,
        badges: vec![],
        used_by: n.members.keys().cloned().collect(),
        actions: vec![],
        detail: CredentialDetail { info: info_lines, actions },
    }
}

/// The graph: every VM (with its NAT to the host) and every network (with its members)
pub(crate) fn graph(ctx: &Ctx) -> NetworkGraph {
    let host = Text::new(if ctx.host == HostOs::Macos { "This Mac" } else { "This computer" });
    let mut vms: Vec<GraphVm> = ctx
        .vms
        .iter()
        .map(|v| GraphVm {
            id: v.name.clone(),
            label: v.name.clone(),
            kind: v.kind.as_str().into(),
            running: v.status == VmStatus::Running,
        })
        .collect();
    let mut links = vec![];
    for v in &ctx.vms {
        let label = match v.network {
            NetworkMode::Shared => Text::new("NAT · internet"),
            NetworkMode::Isolated => Text::new("NAT · no internet"),
            NetworkMode::None => continue,
        };
        links.push(GraphLink {
            vm: v.name.clone(),
            to: "host".into(),
            kind: "nat".into(),
            label,
            active: v.network == NetworkMode::Shared,
            bytes: None,
        });
    }
    let mut networks = vec![];
    let mut flows = vec![];
    for info in &ctx.networks {
        let n = &info.network;
        networks.push(GraphNetwork {
            id: n.name.clone(),
            label: n.name.clone(),
            subnet: n.subnet(),
            running: info.status.is_some(),
        });
        for (vm, host) in &n.members {
            let known = ctx.vms.iter().find(|v| &v.name == vm);
            if known.is_none() && !vms.iter().any(|g| &g.id == vm) {
                vms.push(GraphVm { id: vm.clone(), label: vm.clone(), kind: "unknown".into(), running: false });
            }
            let port = info.status.as_ref().and_then(|s| s.ports.iter().find(|p| p.vm.as_deref() == Some(vm)));
            links.push(GraphLink {
                vm: vm.clone(),
                to: n.name.clone(),
                kind: "network".into(),
                label: Text::verbatim(n.address(*host).to_string()),
                active: port.is_some() && running(known),
                bytes: port.map(|p| p.rx_bytes + p.tx_bytes),
            });
        }
        for f in info.status.as_ref().map(|s| s.flows.as_slice()).unwrap_or_default() {
            flows.push(GraphFlow { network: n.name.clone(), from: f.from.clone(), to: f.to.clone(), bytes: f.bytes });
        }
    }
    NetworkGraph { host, networks, vms, links, flows }
}

pub(crate) fn page(ctx: &Ctx) -> NetworksPage {
    NetworksPage {
        list: CredentialsPage {
            title: Text::new("Networks"),
            intro: Text::new(
                "Networks between VMs: each VM in a network gets a second network card there and reaches the network's other VMs – NixOS and macOS alike, e.g. a Docker VM for several macOS VMs. Its own network (SSH, the internet) stays as it is. Add a VM by drawing a line from it to a network in the picture, or with its switch under the network; × on a line takes it out. A running NixOS VM changes at once, a macOS VM at its next start.",
            ),
            rows: ctx.networks.iter().map(|n| row(ctx, n)).collect(),
            add: Action::show("add", Text::new("New Network …"), Some("add"), "network", json!({})).primary(),
            footer: Text::new(
                "Networks stay on this computer. omaflake runs each network's switch itself – no administrator rights, nothing reaches the network from outside. Every VM in a network can reach every service the others offer there.",
            ),
        },
        graph: graph(ctx),
    }
}

// MARK: Forms

#[derive(Debug, Clone, Default, Deserialize)]
pub struct NetworkValues {
    pub name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct NetworkParams {
    #[serde(default)]
    pub values: NetworkValues,
}

/// `network`: a new network – its name
pub(crate) fn network_page(ctx: &Ctx, p: &NetworkParams) -> CredentialPage {
    let name = p.values.name.clone().unwrap_or_default();
    let trimmed = name.trim().to_string();
    let mut field = Field::new("name", FieldKind::Text, Text::new("Name"), json!(name))
        .param("name")
        .value_param("name")
        .help(Text::new("Lowercase letters, digits and dashes, e.g. “dev” – the VMs' address range is chosen for it."));
    if trimmed.is_empty() {
        field.error = Some(Text::new("A network needs a name"));
    } else if crate::networks::Networks::validate_name(&trimmed).is_err() {
        field.error = Some(Text::new("1 to 24 lowercase letters, digits and dashes (not at the start or end)"));
    } else if ctx.networks.iter().any(|n| n.network.name == trimmed) {
        field.error = Some(tf!("There is a network \"%@\" already", &trimmed));
    }
    let error = field.error.clone();
    let create =
        Action::call("create", Text::new("Create"), Some("create"), "networks.create", json!({"name": trimmed}))
            .then(Then::Back)
            .enabled_if(error.is_none(), error)
            .primary();
    CredentialPage {
        title: Text::new("New Network"),
        id: None,
        intro: Text::new("A network between VMs. Add VMs to it afterwards; each gets a fixed address in it."),
        fields: vec![field],
        locked: None,
        actions: vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({})), create],
    }
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct MemberValues {
    pub vm: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct MemberParams {
    pub network: String,
    #[serde(default)]
    pub values: MemberValues,
}

/// `networkMember`: which VM joins a network
pub(crate) fn member_page(ctx: &Ctx, p: &MemberParams) -> Option<CredentialPage> {
    let info = ctx.networks.iter().find(|n| n.network.name == p.network)?;
    let candidates: Vec<&Vm> = ctx.vms.iter().filter(|v| !info.network.members.contains_key(&v.name)).collect();
    let vm = p.values.vm.clone().or_else(|| candidates.first().map(|v| v.name.clone())).unwrap_or_default();
    let choices = candidates
        .iter()
        .map(|v| {
            Choice::new(json!(v.name), Text::verbatim(v.name.clone())).icon(if v.kind == GuestKind::Macos {
                "macos"
            } else {
                "nixos"
            })
        })
        .collect();
    let mut field = Field::new("vm", FieldKind::Choice, Text::new("VM"), json!(vm))
        .param("vm")
        .value_param("vm")
        .help(Text::new("It gets a second network card in this network from its next start."));
    field.choices = choices;
    if candidates.is_empty() {
        field.error = Some(Text::new("Every VM is in it"));
    }
    let error = field.error.clone();
    let join =
        Action::call("join", Text::new("Add"), Some("add"), "networks.join", json!({"network": p.network, "vm": vm}))
            .then(Then::Back)
            .enabled_if(error.is_none(), error)
            .primary();
    Some(CredentialPage {
        title: tf!("Add a VM to “%@”", &p.network),
        id: Some(p.network.clone()),
        intro: tf!(
            "A VM in %@ reaches the network's other VMs; a running VM gets the card at its next start.",
            info.network.subnet()
        ),
        fields: vec![field],
        locked: None,
        actions: vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({})), join],
    })
}
