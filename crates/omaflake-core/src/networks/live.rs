// SPDX-License-Identifier: AGPL-3.0-or-later
//! Joining and leaving a network while a NixOS VM runs (plan B179). QEMU starts every VM with four `pcie-root-port`s
//! on the network slots (qemu_command.rs); a card joined later is plugged into a free one over QMP (`netdev_add` of a
//! stream to the switch, `device_add` of the virtio-net card), a card that leaves is unplugged (`device_del`, then
//! `netdev_del` once the guest let it go). NetworkManager in the guest brings a new card up with DHCP by itself.
//! `<instance>/run/lan.json` keeps which network has which port while the VM runs: QEMU writes nothing that says it.
//! macOS VMs cannot: Virtualization.framework takes no device while a VM runs – their cards change at the next start.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::json;

use super::packets::mac_text;
use super::{Attachment, MAX_PER_VM};
use crate::error::{IoContext, Result, codes};
use crate::paths::Paths;
use crate::qmp_client::QmpClient;
use crate::vm_err;

/// The ports of a running VM: network → slot index (0–3)
pub type Ports = BTreeMap<String, usize>;

pub struct LiveCards {
    file: PathBuf,
    qmp: PathBuf,
}

impl LiveCards {
    pub fn new(paths: &Paths, vm: &str) -> Self {
        let ip = paths.instance(vm);
        LiveCards { file: ip.run.join("lan.json"), qmp: ip.qmp_socket() }
    }

    pub fn ports(&self) -> Ports {
        std::fs::read(&self.file).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    fn save(&self, ports: &Ports) -> Result<()> {
        if let Some(dir) = self.file.parent() {
            std::fs::create_dir_all(dir).with_path(dir)?;
        }
        std::fs::write(&self.file, serde_json::to_vec(ports)?).with_path(&self.file)
    }

    /// At start: the cards QEMU gets, in their order (`lan<i>` on `lanport<i>`)
    pub fn started(&self, cards: &[Attachment]) -> Result<()> {
        self.save(&cards.iter().take(MAX_PER_VM).enumerate().map(|(i, c)| (c.network.clone(), i)).collect())
    }

    /// The first slot no network has
    pub fn free_slot(ports: &Ports) -> Option<usize> {
        (0..MAX_PER_VM).find(|i| !ports.values().any(|p| p == i))
    }

    /// Plugs `card` into the running VM (its switch must run)
    pub fn attach(&self, card: &Attachment) -> Result<()> {
        let mut ports = self.ports();
        if ports.contains_key(&card.network) {
            return Ok(());
        }
        let i = Self::free_slot(&ports)
            .ok_or_else(|| vm_err!(codes::UNSUPPORTED_SYSTEM, "A VM is in at most {} networks", MAX_PER_VM))?;
        let mut q = QmpClient::connect(&self.qmp, QmpClient::DEFAULT_TIMEOUT)?;
        q.execute_with("netdev_add", Some(Self::netdev(i, &card.qemu_socket)))?;
        if let Err(e) = q.execute_with("device_add", Some(Self::device(i, card))) {
            let _ = q.execute_with("netdev_del", Some(json!({"id": format!("lan{i}")})));
            return Err(e);
        }
        ports.insert(card.network.clone(), i);
        self.save(&ports)
    }

    /// Unplugs the card of `network` from the running VM: the guest lets the card go (up to 10 s), then its netdev
    pub fn detach(&self, network: &str) -> Result<()> {
        let mut ports = self.ports();
        let Some(i) = ports.get(network).copied() else { return Ok(()) };
        let mut q = QmpClient::connect(&self.qmp, QmpClient::DEFAULT_TIMEOUT)?;
        q.execute_with("device_del", Some(json!({"id": format!("lancard{i}")})))?;
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            // The netdev goes once the card is gone (QEMU: "… is in use" until then)
            match q.execute_with("netdev_del", Some(json!({"id": format!("lan{i}")}))) {
                Ok(_) => break,
                Err(e) if Instant::now() >= deadline => {
                    return Err(vm_err!("The VM did not let its card in \"{}\" go: {}", network, e.message()));
                }
                Err(_) => std::thread::sleep(Duration::from_millis(300)),
            }
        }
        ports.remove(network);
        self.save(&ports)
    }

    pub(crate) fn netdev(i: usize, socket: &Path) -> serde_json::Value {
        json!({
            "type": "stream",
            "id": format!("lan{i}"),
            "server": false,
            "addr": {"type": "unix", "path": socket.to_string_lossy()},
            "reconnect-ms": 1000
        })
    }

    pub(crate) fn device(i: usize, card: &Attachment) -> serde_json::Value {
        json!({
            "driver": "virtio-net-pci",
            "netdev": format!("lan{i}"),
            "mac": mac_text(&card.mac),
            "bus": format!("lanport{i}"),
            "id": format!("lancard{i}")
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn card(network: &str) -> Attachment {
        Attachment {
            network: network.into(),
            mac: super::super::mac_for(1, 10),
            address: std::net::Ipv4Addr::new(10, 231, 1, 10),
            qemu_socket: format!("/data/run/net/{network}/qemu.sock").into(),
            vz_socket: format!("/data/run/net/{network}/vz.sock").into(),
        }
    }

    #[test]
    fn ports_at_start_and_free_slots() {
        let t = tempfile::tempdir().unwrap();
        let p = Paths::new(t.path().join("data"), None);
        let live = LiveCards::new(&p, "dev");
        assert!(live.ports().is_empty());
        live.started(&[card("a"), card("b")]).unwrap();
        let ports = live.ports();
        assert_eq!(ports, BTreeMap::from([("a".to_string(), 0), ("b".to_string(), 1)]));
        assert_eq!(LiveCards::free_slot(&ports), Some(2));
        let full: Ports = (0..4).map(|i| (format!("n{i}"), i)).collect();
        assert_eq!(LiveCards::free_slot(&full), None);
        let gap = BTreeMap::from([("a".to_string(), 0), ("c".to_string(), 2)]);
        assert_eq!(LiveCards::free_slot(&gap), Some(1));
    }

    /// Against a real QEMU (`OMAF_TEST_QEMU=<qemu-system-aarch64>`, TCG, no guest): a card plugged into a free port
    /// while the machine exists, then unplugged – the commands as `attach`/`detach` send them
    #[test]
    fn plug_and_unplug_in_a_real_qemu() {
        let Some(qemu) = std::env::var("OMAF_TEST_QEMU").ok().filter(|q| !q.is_empty()) else { return };
        let t = tempfile::tempdir().unwrap();
        let p = Paths::new(t.path().join("d"), None);
        let live = LiveCards::new(&p, "dev");
        std::fs::create_dir_all(p.instance("dev").run).unwrap();
        let qmp = p.instance("dev").qmp_socket();
        let mut args =
            vec!["-machine", "virt", "-accel", "tcg", "-cpu", "max", "-nodefaults", "-display", "none", "-S"]
                .into_iter()
                .map(String::from)
                .collect::<Vec<_>>();
        for (i, at) in ["0xa", "0xb", "0xc", "0xd"].iter().enumerate() {
            args.extend(["-device".into(), format!("pcie-root-port,id=lanport{i},chassis={},addr={at}", 10 + i)]);
        }
        args.extend(["-qmp".into(), format!("unix:{},server=on,wait=off", qmp.display())]);
        let mut child = std::process::Command::new(qemu).args(&args).spawn().unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while !qmp.exists() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        live.started(&[card("a")]).unwrap();
        live.attach(&card("b")).unwrap();
        assert_eq!(live.ports().get("b"), Some(&1), "the first free port");
        live.attach(&card("b")).unwrap();
        assert_eq!(live.ports().len(), 2, "plugged in once");
        live.detach("b").unwrap();
        assert!(!live.ports().contains_key("b"));
        let _ = child.kill();
        let _ = child.wait();
    }

    /// The QMP arguments as QEMU 11.1.2 took them (netdev_add of a stream, device_add on a root port)
    #[test]
    fn qmp_arguments() {
        let c = card("dev");
        assert_eq!(
            LiveCards::netdev(2, &c.qemu_socket),
            json!({"type": "stream", "id": "lan2", "server": false,
                   "addr": {"type": "unix", "path": "/data/run/net/dev/qemu.sock"}, "reconnect-ms": 1000})
        );
        assert_eq!(
            LiveCards::device(2, &c),
            json!({"driver": "virtio-net-pci", "netdev": "lan2", "mac": "02:6f:6d:01:0a:00", "bus": "lanport2", "id": "lancard2"})
        );
    }
}
