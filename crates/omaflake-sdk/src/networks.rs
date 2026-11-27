// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaflake's networks between VMs (spec §11.12) as the API shows them: a network, its switch's status.

use std::collections::BTreeMap;
use std::net::Ipv4Addr;

use serde::{Deserialize, Serialize};

use crate::time::Date;

pub type Mac = [u8; 6];

/// The first two octets of every network's subnet
pub const PREFIX: [u8; 2] = [10, 231];

/// The switch's own host number in each network
pub const SWITCH_HOST: u8 = 1;

/// MAC prefix of every card on omaflake's networks: locally administered, unicast, "om"
pub const MAC_PREFIX: [u8; 3] = [0x02, 0x6f, 0x6d];

/// One network
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Network {
    /// Its name ("dev"): lowercase letters, digits and dashes – it is part of paths and of what the guest sees
    pub name: String,
    /// The third octet of its subnet `10.231.<number>.0/24` (1–254)
    pub number: u8,
    #[serde(with = "crate::time::iso8601")]
    pub created: Date,
    /// VM name → its host number
    #[serde(default)]
    pub members: BTreeMap<String, u8>,
}

impl Network {
    pub fn address(&self, host: u8) -> Ipv4Addr {
        Ipv4Addr::new(PREFIX[0], PREFIX[1], self.number, host)
    }

    pub fn switch_address(&self) -> Ipv4Addr {
        self.address(SWITCH_HOST)
    }

    /// "10.231.3.0/24"
    pub fn subnet(&self) -> String {
        format!("{}/24", self.address(0))
    }

    pub fn mac(&self, host: u8) -> Mac {
        mac_for(self.number, host)
    }

    pub fn switch_mac(&self) -> Mac {
        self.mac(SWITCH_HOST)
    }

    /// The address of a member
    pub fn member_address(&self, vm: &str) -> Option<Ipv4Addr> {
        self.members.get(vm).map(|h| self.address(*h))
    }
}

pub fn mac_for(number: u8, host: u8) -> Mac {
    [MAC_PREFIX[0], MAC_PREFIX[1], MAC_PREFIX[2], number, host, 0]
}

/// One port's numbers in status.json
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct PortStatus {
    /// "qemu" or "vz"
    pub kind: String,
    /// The card's MAC, once it sent a frame
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac: Option<String>,
    /// The member behind that MAC
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vm: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// From the VM into the switch
    #[serde(rename = "rxFrames")]
    pub rx_frames: u64,
    #[serde(rename = "rxBytes")]
    pub rx_bytes: u64,
    /// From the switch to the VM
    #[serde(rename = "txFrames")]
    pub tx_frames: u64,
    #[serde(rename = "txBytes")]
    pub tx_bytes: u64,
    #[serde(rename = "dropped")]
    pub dropped: u64,
}

/// Bytes from one VM to another (unicast frames only)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flow {
    pub from: String,
    pub to: String,
    pub bytes: u64,
}

/// status.json of a running switch
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SwitchStatus {
    pub network: String,
    pub pid: i32,
    #[serde(with = "crate::time::iso8601")]
    pub started: Date,
    #[serde(with = "crate::time::iso8601")]
    pub updated: Date,
    pub ports: Vec<PortStatus>,
    #[serde(default)]
    pub flows: Vec<Flow>,
}
