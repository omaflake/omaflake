// SPDX-License-Identifier: AGPL-3.0-or-later
//! Networks between VMs (spec §11.12, plan B170): a network is a virtual Ethernet segment on this computer that VMs of
//! both kinds join with a **second network card** – their own NAT (SSH, the internet, the network mode) stays as it
//! is. omaflake runs the segment itself: one switch process per network (`omaf network-switch`, switch.rs) with the
//! QEMU guests on a stream socket (`-netdev stream`) and the macOS guests on a datagram socket
//! (`VZFileHandleNetworkDeviceAttachment`). No root, no entitlement, no third-party program.
//!
//! **Addresses.** Network n is `10.231.<n>.0/24`; the switch is `.1`, a VM gets a fixed host number (10–254) when it
//! joins. Its card's MAC carries both – `02:6f:6d:<n>:<host>:00` (locally administered, "om") –, so the switch's
//! DHCP server answers from the MAC alone and keeps no leases. No router and no DNS server are handed out: the
//! default route stays on the VM's NAT.
//!
//! **Files.** `<data>/networks.json` (schema `networks` v1) holds the networks and their members – the only place
//! membership is kept (a VM's package does not carry it: a network belongs to this computer). Runtime files live in
//! `<data>/run/net/<name>/`: `qemu.sock`, `vz.sock`, `switch.pid`, `switch.log`, `status.json`.

pub mod packets;
pub mod switch;

use std::collections::BTreeMap;
use std::net::Ipv4Addr;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::error::{IoContext, Result, codes};
use crate::file_format::{FileFormat, Step, write_atomic};
use crate::json::Date;
use crate::paths::Paths;
use crate::spawn::{Pid, live_pid, spawn_detached};
use crate::vm_err;
use packets::Mac;

/// The first two octets of every network's subnet
pub const PREFIX: [u8; 2] = [10, 231];
/// The switch's own host number in each network
pub const SWITCH_HOST: u8 = 1;
/// Host numbers VMs get
pub const FIRST_HOST: u8 = 10;
pub const LAST_HOST: u8 = 254;
/// MAC prefix of every card on omaflake's networks: locally administered, unicast, "om"
pub const MAC_PREFIX: [u8; 3] = [0x02, 0x6f, 0x6d];
/// The most networks a VM joins (QEMU: one fixed PCI slot each, `qemu_command::slot::NETWORKS`)
pub const MAX_PER_VM: usize = 4;

static NETWORKS_STEPS: [Step; 1] = [|_| Ok(())];

/// One network
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Network {
    /// Its name ("dev"): lowercase letters, digits and dashes – it is part of paths and of what the guest sees
    pub name: String,
    /// The third octet of its subnet `10.231.<number>.0/24` (1–254)
    pub number: u8,
    #[serde(with = "crate::json::iso8601")]
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

/// (network number, host number) of a MAC on one of omaflake's networks
pub fn parse_member_mac(mac: &Mac) -> Option<(u8, u8)> {
    (mac[0..3] == MAC_PREFIX && mac[5] == 0).then_some((mac[3], mac[4]))
}

/// A VM's card on a network, as a backend attaches it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Attachment {
    pub network: String,
    pub mac: Mac,
    pub address: Ipv4Addr,
    /// QEMU connects here (`-netdev stream`)
    pub qemu_socket: PathBuf,
    /// omaf-runner sends its datagrams here (`VZFileHandleNetworkDeviceAttachment`)
    pub vz_socket: PathBuf,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct NetworksFile {
    #[serde(default)]
    networks: Vec<Network>,
}

/// The networks of a data folder
#[derive(Debug, Clone)]
pub struct Networks {
    paths: Paths,
}

impl Networks {
    /// Schema `networks`; version 1 is the first
    pub const FILE_FORMAT: FileFormat = FileFormat::new("networks", "networks.json", 1, &NETWORKS_STEPS);

    pub fn new(paths: Paths) -> Networks {
        Networks { paths }
    }

    pub fn file(&self) -> PathBuf {
        self.paths.root.join("networks.json")
    }

    /// `<data>/run/net/<name>`
    pub fn run_dir(&self, name: &str) -> PathBuf {
        self.paths.run().join("net").join(name)
    }

    pub fn qemu_socket(&self, name: &str) -> PathBuf {
        self.run_dir(name).join("qemu.sock")
    }

    pub fn vz_socket(&self, name: &str) -> PathBuf {
        self.run_dir(name).join("vz.sock")
    }

    pub fn pid_file(&self, name: &str) -> PathBuf {
        self.run_dir(name).join("switch.pid")
    }

    pub fn log_file(&self, name: &str) -> PathBuf {
        self.run_dir(name).join("switch.log")
    }

    pub fn status_file(&self, name: &str) -> PathBuf {
        self.run_dir(name).join("status.json")
    }

    // MARK: The file

    pub fn list(&self) -> Result<Vec<Network>> {
        let Ok(data) = std::fs::read(self.file()) else { return Ok(vec![]) };
        let (file, _): (NetworksFile, bool) = Self::FILE_FORMAT.decode(&data)?;
        Ok(file.networks)
    }

    fn save(&self, networks: Vec<Network>) -> Result<()> {
        let file = self.file();
        Self::FILE_FORMAT.check_writable(&file)?;
        std::fs::create_dir_all(&self.paths.root).with_path(&self.paths.root)?;
        write_atomic(&file, &Self::FILE_FORMAT.encode(&NetworksFile { networks })?)
    }

    pub fn get(&self, name: &str) -> Result<Network> {
        self.list()?
            .into_iter()
            .find(|n| n.name == name)
            .ok_or_else(|| vm_err!(codes::NOT_FOUND, "No network \"{}\"", name))
    }

    /// The networks a VM is in, by name
    pub fn of_vm(&self, vm: &str) -> Vec<Network> {
        self.list().unwrap_or_default().into_iter().filter(|n| n.members.contains_key(vm)).collect()
    }

    /// The cards a VM gets when it starts, in the order of its networks' names
    pub fn attachments(&self, vm: &str) -> Vec<Attachment> {
        self.of_vm(vm)
            .into_iter()
            .take(MAX_PER_VM)
            .map(|n| {
                let host = n.members[vm];
                Attachment {
                    mac: n.mac(host),
                    address: n.address(host),
                    qemu_socket: self.qemu_socket(&n.name),
                    vz_socket: self.vz_socket(&n.name),
                    network: n.name,
                }
            })
            .collect()
    }

    pub fn validate_name(name: &str) -> Result<()> {
        let ok = !name.is_empty()
            && name.len() <= 24
            && name.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
            && !name.starts_with('-')
            && !name.ends_with('-');
        if !ok {
            return Err(vm_err!(
                "A network's name has 1 to 24 lowercase letters, digits and dashes (not at the start or end): \"{}\"",
                name
            ));
        }
        Ok(())
    }

    pub fn create(&self, name: &str) -> Result<Network> {
        Self::validate_name(name)?;
        let mut all = self.list()?;
        if all.iter().any(|n| n.name == name) {
            return Err(vm_err!(codes::EXISTS, "Network \"{}\" exists already", name));
        }
        let Some(number) = (1..=254u8).find(|k| all.iter().all(|n| n.number != *k)) else {
            return Err(vm_err!("No free network number (at most 254 networks)"));
        };
        let network = Network { name: name.into(), number, created: crate::json::now(), members: BTreeMap::new() };
        all.push(network.clone());
        all.sort_by(|a, b| a.name.cmp(&b.name));
        self.save(all)?;
        Ok(network)
    }

    /// Removes a network; its members only lose that card (at their next start). A running switch is stopped.
    pub fn remove(&self, name: &str) -> Result<()> {
        let mut all = self.list()?;
        let before = all.len();
        all.retain(|n| n.name != name);
        if all.len() == before {
            return Err(vm_err!(codes::NOT_FOUND, "No network \"{}\"", name));
        }
        self.save(all)?;
        self.stop_switch(name);
        Ok(())
    }

    /// Adds a VM to a network (its card appears at its next start); the address stays the VM's while it is a member
    pub fn join(&self, name: &str, vm: &str) -> Result<Ipv4Addr> {
        let mut all = self.list()?;
        let count = all.iter().filter(|n| n.members.contains_key(vm) && n.name != name).count();
        let Some(network) = all.iter_mut().find(|n| n.name == name) else {
            return Err(vm_err!(codes::NOT_FOUND, "No network \"{}\"", name));
        };
        if let Some(address) = network.member_address(vm) {
            return Ok(address);
        }
        if count >= MAX_PER_VM {
            return Err(vm_err!("A VM is in at most {} networks", MAX_PER_VM));
        }
        let Some(host) = (FIRST_HOST..=LAST_HOST).find(|h| network.members.values().all(|m| m != h)) else {
            return Err(vm_err!("Network \"{}\" is full", name));
        };
        network.members.insert(vm.into(), host);
        let address = network.address(host);
        self.save(all)?;
        Ok(address)
    }

    pub fn leave(&self, name: &str, vm: &str) -> Result<()> {
        let mut all = self.list()?;
        let Some(network) = all.iter_mut().find(|n| n.name == name) else {
            return Err(vm_err!(codes::NOT_FOUND, "No network \"{}\"", name));
        };
        if network.members.remove(vm).is_none() {
            return Err(vm_err!(codes::NOT_FOUND, "\"{}\" is not in network \"{}\"", vm, name));
        }
        self.save(all)
    }

    /// Sets a VM's networks to exactly `names` (keeping its address in those it stays in)
    pub fn set_vm(&self, vm: &str, names: &[String]) -> Result<()> {
        if names.len() > MAX_PER_VM {
            return Err(vm_err!("A VM is in at most {} networks", MAX_PER_VM));
        }
        let all = self.list()?;
        for name in names {
            if !all.iter().any(|n| &n.name == name) {
                return Err(vm_err!(codes::NOT_FOUND, "No network \"{}\"", name));
            }
        }
        for n in all.iter().filter(|n| n.members.contains_key(vm) && !names.contains(&n.name)) {
            self.leave(&n.name, vm)?;
        }
        for name in names {
            self.join(name, vm)?;
        }
        Ok(())
    }

    /// A VM is gone: out of every network (`InstanceStore::remove`)
    pub fn forget_vm(&self, vm: &str) -> Result<()> {
        let mut all = self.list()?;
        let mut changed = false;
        for n in &mut all {
            changed |= n.members.remove(vm).is_some();
        }
        if changed { self.save(all) } else { Ok(()) }
    }

    // MARK: The switch

    /// The running switch of a network
    pub fn switch_pid(&self, name: &str) -> Option<Pid> {
        live_pid(&self.pid_file(name), "omaf")
    }

    /// Starts the switches of a VM's networks that are not running (before the VM starts). `omaf` is the program
    /// that serves `network-switch` (this omaflake's own CLI).
    pub fn ensure_switches(&self, vm: &str, omaf: &str) -> Result<()> {
        for n in self.of_vm(vm).into_iter().take(MAX_PER_VM) {
            self.ensure_switch(&n.name, omaf)?;
        }
        Ok(())
    }

    /// Before a VM starts: its networks' switches running, and the cards it gets (none without networks – then
    /// nothing is started and no `omaf` is needed)
    pub fn prepare(&self, vm: &str) -> Result<Vec<Attachment>> {
        let cards = self.attachments(vm);
        if cards.is_empty() {
            return Ok(cards);
        }
        let omaf = crate::cli_link::CliLink::running_omaf()
            .map(|p| path_text(&p))
            .or_else(|| crate::shell::Shell::which("omaf").ok())
            .ok_or_else(|| vm_err!(codes::PREREQUISITE, "omaf not found – it runs the networks' switches"))?;
        self.ensure_switches(vm, &omaf)?;
        Ok(cards)
    }

    pub fn ensure_switch(&self, name: &str, omaf: &str) -> Result<()> {
        if self.switch_pid(name).is_some() && self.qemu_socket(name).exists() {
            return Ok(());
        }
        let dir = self.run_dir(name);
        std::fs::create_dir_all(&dir).with_path(&dir)?;
        let args = vec!["network-switch".to_string(), name.to_string(), "--data".into(), path_text(&self.paths.root)];
        let pid = spawn_detached(omaf, &args, &self.log_file(name), &[])?;
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            if self.qemu_socket(name).exists() && self.vz_socket(name).exists() {
                return Ok(());
            }
            if crate::spawn::reap_if_exited(pid) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let log = std::fs::read_to_string(self.log_file(name)).unwrap_or_default();
        let last: Vec<&str> = log.lines().rev().take(3).collect();
        Err(vm_err!(
            "The switch of network \"{}\" did not start: {}",
            name,
            last.into_iter().rev().collect::<Vec<_>>().join(" / ")
        ))
    }

    pub fn stop_switch(&self, name: &str) {
        if let Some(pid) = self.switch_pid(name) {
            crate::spawn::send_signal(pid, libc::SIGTERM);
            let _ = crate::spawn::wait_for_exit(pid, Duration::from_secs(2));
        }
    }

    /// What the running switch last wrote (None: not running)
    pub fn status(&self, name: &str) -> Option<switch::SwitchStatus> {
        self.switch_pid(name)?;
        let data = std::fs::read(self.status_file(name)).ok()?;
        serde_json::from_slice(&data).ok()
    }
}

/// `<path>` as a string for a command line
pub(crate) fn path_text(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn networks() -> (tempfile::TempDir, Networks) {
        let dir = tempfile::tempdir().unwrap();
        let n = Networks::new(Paths::new(dir.path(), None));
        (dir, n)
    }

    #[test]
    fn names() {
        for ok in ["dev", "a", "team-2", "x123456789012345678901234"[..24].to_string().as_str()] {
            assert!(Networks::validate_name(ok).is_ok(), "{ok}");
        }
        for bad in ["", "Dev", "a b", "-a", "a-", "a/b", "ä", "x123456789012345678901234"] {
            assert!(Networks::validate_name(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn create_join_leave_remove() {
        let (_d, n) = networks();
        assert!(n.list().unwrap().is_empty());
        let dev = n.create("dev").unwrap();
        assert_eq!((dev.number, dev.subnet()), (1, "10.231.1.0/24".to_string()));
        assert_eq!(n.create("ops").unwrap().number, 2);
        assert!(n.create("dev").is_err());
        assert_eq!(n.join("dev", "docker").unwrap(), Ipv4Addr::new(10, 231, 1, 10));
        assert_eq!(n.join("dev", "mac1").unwrap(), Ipv4Addr::new(10, 231, 1, 11));
        assert_eq!(n.join("dev", "docker").unwrap(), Ipv4Addr::new(10, 231, 1, 10), "joining again keeps it");
        n.leave("dev", "docker").unwrap();
        assert!(n.leave("dev", "docker").is_err());
        assert_eq!(n.join("dev", "mac2").unwrap(), Ipv4Addr::new(10, 231, 1, 10), "a free number is reused");
        n.remove("ops").unwrap();
        assert!(n.remove("ops").is_err());
        assert_eq!(n.create("lab").unwrap().number, 2, "a free network number is reused");
        let text = std::fs::read_to_string(n.file()).unwrap();
        assert!(text.contains("https://omaflake.dev/schema/networks/v1.json"), "{text}");
    }

    #[test]
    fn a_vms_cards() {
        let (_d, n) = networks();
        n.create("dev").unwrap();
        n.create("ops").unwrap();
        n.set_vm("vm1", &["ops".into(), "dev".into()]).unwrap();
        let a = n.attachments("vm1");
        assert_eq!(a.len(), 2);
        assert_eq!(a[0].network, "dev");
        assert_eq!(packets::mac_text(&a[0].mac), "02:6f:6d:01:0a:00");
        assert_eq!(a[1].address, Ipv4Addr::new(10, 231, 2, 10));
        assert!(a[0].qemu_socket.ends_with("run/net/dev/qemu.sock"));
        assert!(a[1].vz_socket.ends_with("run/net/ops/vz.sock"));
        assert_eq!(parse_member_mac(&a[1].mac), Some((2, 10)));
        assert_eq!(parse_member_mac(&[0x02, 0x6f, 0x6d, 2, 10, 1]), None);
        n.set_vm("vm1", &["ops".into()]).unwrap();
        assert_eq!(n.of_vm("vm1").len(), 1);
        assert!(n.set_vm("vm1", &["nope".into()]).is_err());
        assert_eq!(n.get("ops").unwrap().member_address("vm1"), Some(Ipv4Addr::new(10, 231, 2, 10)));
        n.forget_vm("vm1").unwrap();
        assert!(n.of_vm("vm1").is_empty());
    }

    #[test]
    fn at_most_four_networks_per_vm() {
        let (_d, n) = networks();
        for i in 0..5 {
            n.create(&format!("n{i}")).unwrap();
        }
        for i in 0..4 {
            n.join(&format!("n{i}"), "vm").unwrap();
        }
        assert!(n.join("n4", "vm").is_err());
        assert!(n.set_vm("vm", &(0..5).map(|i| format!("n{i}")).collect::<Vec<_>>()).is_err());
    }
}
