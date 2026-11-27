// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Networks tab (`networks`, plan B170, spec §11.12) – networks between VMs – and its two forms: a new network
//! (`network`) and adding a VM to one (`networkMember`). The list has the Credentials page's shape, so every UI draws
//! it as it draws that; on top it carries `graph`: the host, every network, every VM and how they are connected –
//! each VM's own NAT to the host, each membership with the VM's address, and what went between which VMs while the
//! switch runs. UIs draw the graph as a picture (app, Linux app) or as an indented tree (`omaf ui`).

use serde::{Deserialize, Serialize};

use super::credentials::CredentialsPage;
use crate::networks::{Network, SwitchStatus};
use crate::text::Text;

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

#[derive(Debug, Clone, Default, Deserialize)]
pub struct NetworkValues {
    pub name: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct NetworkParams {
    #[serde(default)]
    pub values: NetworkValues,
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
