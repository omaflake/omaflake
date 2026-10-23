// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of MacVM.swift – work package F4. Done: the types, paths and the config.json format. Skeleton: the rest
//! (`todo!`). `clonefile` is macOS-only (`Error::unsupported` elsewhere).
#![allow(unused_variables, dead_code)]

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::Result;
use crate::file_format::{FileFormat, Step};

/// The VM window's size (Swift `MacVMConfig.Display`; default 1920×1200 at 80 ppi)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacDisplay {
    pub width: u32,
    pub height: u32,
    pub ppi: u32,
}

impl Default for MacDisplay {
    fn default() -> Self {
        MacDisplay { width: 1920, height: 1200, ppi: 80 }
    }
}

/// The folder's config.json, schema `macos-vm` (plan B114). 0 → 1: `version` (always 1) gave way to `$schema`
fn macvm_v0_to_v1(o: &mut Map<String, Value>) -> Result<()> {
    o.remove("version");
    Ok(())
}

static MACVM_STEPS: [Step; 1] = [macvm_v0_to_v1];

/// Settings of a macOS VM (config.json in the VM folder, spec §5). Keys: cpus, memoryMB, diskGB, display,
/// macAddress, osBuild (None left out). Hardware model, machine identifier and auxiliary storage are separate files
/// next to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacVmConfig {
    pub cpus: u32,
    #[serde(rename = "memoryMB")]
    pub memory_mb: u32,
    #[serde(rename = "diskGB")]
    pub disk_gb: u32,
    pub display: MacDisplay,
    /// fixed MAC per VM (for the NAT IP from /var/db/dhcpd_leases)
    #[serde(rename = "macAddress")]
    pub mac_address: String,
    /// Build of the installed IPSW, e.g. "27A5321"
    #[serde(rename = "osBuild", default, skip_serializing_if = "Option::is_none")]
    pub os_build: Option<String>,
}

impl MacVmConfig {
    /// config.json of a macOS VM (see `macvm_v0_to_v1`)
    pub const FILE_FORMAT: FileFormat = FileFormat::new("macos-vm", "config.json of a macOS VM", 0, &MACVM_STEPS);

    pub fn new(cpus: u32, memory_mb: u32, disk_gb: u32, mac_address: &str, os_build: Option<String>) -> Self {
        MacVmConfig {
            cpus,
            memory_mb,
            disk_gb,
            display: MacDisplay::default(),
            mac_address: mac_address.into(),
            os_build,
        }
    }
}

/// Folder of a macOS VM: base, image and instance share the same layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacVmBundle {
    pub dir: PathBuf,
}

impl MacVmBundle {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        MacVmBundle { dir: dir.into() }
    }

    pub fn config(&self) -> PathBuf {
        self.dir.join("config.json")
    }
    /// VZMacHardwareModel.dataRepresentation
    pub fn hardware_model(&self) -> PathBuf {
        self.dir.join("hardware-model")
    }
    /// VZMacMachineIdentifier.dataRepresentation (new per instance)
    pub fn machine_id(&self) -> PathBuf {
        self.dir.join("machine-id")
    }
    /// VZMacAuxiliaryStorage
    pub fn aux_storage(&self) -> PathBuf {
        self.dir.join("aux.img")
    }
    /// System disk, RAW (VZDiskImageStorageDeviceAttachment only supports RAW, plan BM9)
    pub fn disk(&self) -> PathBuf {
        self.dir.join("disk.img")
    }

    /// The files that make up a VM (runtime files like sockets and logs are not part of it).
    pub fn files(&self) -> [PathBuf; 5] {
        [self.config(), self.hardware_model(), self.machine_id(), self.aux_storage(), self.disk()]
    }

    /// config.json; "Not a macOS VM (config.json missing): <dir>". Bases and images are read-only: those are brought
    /// up to date at every read instead of written back.
    pub fn load_config(&self) -> Result<MacVmConfig> {
        todo!("port from MacVM.swift")
    }

    pub fn save(&self, c: &MacVmConfig) -> Result<()> {
        todo!("port from MacVM.swift")
    }

    /// APFS clone of the VM files into a new folder (no extra space until blocks change), writable (aux.img 0600, the
    /// rest 0644), with a new MAC address so that clone and source get different NAT addresses. The machine
    /// identifier needs Virtualization.framework – `omaf-runner clone` replaces it. Swift `clone(to:)`.
    pub fn clone_to(&self, target: &Path) -> Result<MacVmBundle> {
        todo!("port from MacVM.swift")
    }

    /// Creates an empty, sparse RAW disk (size a multiple of 512).
    pub fn create_disk(&self, gigabytes: u64) -> Result<()> {
        todo!("port from MacVM.swift")
    }
}

/// MAC addresses for VM network cards.
pub enum MacAddress {}

impl MacAddress {
    /// Random unicast address with the "locally administered" bit set (like VZMACAddress.randomLocallyAdministered),
    /// lowercase "xx:xx:xx:xx:xx:xx".
    pub fn random_locally_administered() -> String {
        todo!("port from MacVM.swift")
    }
}

/// IP addresses from the macOS DHCP server (vmnet/NAT), /var/db/dhcpd_leases.
pub enum DhcpLeases {}

impl DhcpLeases {
    pub const FILE: &'static str = "/var/db/dhcpd_leases";

    /// MAC as written in the lease file: lowercase, leading zeros per octet removed
    /// ("e6:25:86:0f:90:4d" → "e6:25:86:f:90:4d").
    pub fn normalize(mac: &str) -> String {
        todo!("port from MacVM.swift")
    }

    /// Most recent IP for the MAC (entries are newest first; with several, the largest `lease` wins). Swift
    /// `ip(for:in:)`.
    pub fn ip_in(mac: &str, text: &str) -> Option<String> {
        todo!("port from MacVM.swift")
    }

    /// From `FILE`. Swift `ip(for:)`.
    pub fn ip(mac: &str) -> Option<String> {
        todo!("port from MacVM.swift")
    }
}

/// Controls a running omaf-runner via its Unix socket (one JSON line out, one back).
pub enum RunnerClient {}

impl RunnerClient {
    pub const COMMANDS: [&'static str; 4] = ["status", "stop", "kill", "show"];
    /// Swift's default timeout
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

    /// Sends `{"command": <command>}` + newline, reads one line back as a string map; "Invalid response from
    /// omaf-runner: <text>" otherwise.
    pub fn send(command: &str, socket: &Path, timeout: Duration) -> Result<HashMap<String, String>> {
        todo!("port from MacVM.swift")
    }
}
