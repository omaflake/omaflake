// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf networks` (plan B170, spec §11.12): networks between VMs – each member gets a second card on a switch omaflake
//! runs; its own NAT stays. `omaf network-switch` is that switch (hidden, started before a member starts).

use omaflake_core::instance_store::InstanceStore;
use omaflake_core::networks::switch::{Switch, SwitchStatus};
use omaflake_core::networks::{Network, Networks, packets::mac_text};
use omaflake_core::paths::Paths;

use crate::cli;
use crate::util::{Env, Outcome, pad};

fn store(env: &Env) -> Networks {
    Networks::new(env.paths.clone())
}

pub fn list() -> Outcome {
    let env = Env::new();
    let networks = store(&env);
    let all = networks.list()?;
    for n in &all {
        let running = if networks.switch_pid(&n.name).is_some() { "running" } else { "idle" };
        let members: Vec<&str> = n.members.keys().map(String::as_str).collect();
        println!(
            "{}{}{}{}",
            pad(&n.name, 16),
            pad(&n.subnet(), 18),
            pad(running, 9),
            if members.is_empty() { "–".to_string() } else { members.join(", ") }
        );
    }
    if all.is_empty() {
        println!("(no networks yet – omaf networks create <name>, then omaf networks join <name> <vm>)");
    }
    Ok(())
}

pub fn create(name: &str) -> Outcome {
    let env = Env::new();
    let n = store(&env).create(name)?;
    println!("Network {} created: {} (the switch is {}).", n.name, n.subnet(), n.switch_address());
    Ok(())
}

pub fn remove(name: &str) -> Outcome {
    let env = Env::new();
    store(&env).remove(name)?;
    println!("Network {name} removed – its members lose that card at their next start.");
    Ok(())
}

pub fn join(network: &str, vm: &str) -> Outcome {
    let env = Env::new();
    if !InstanceStore::new(env.paths.clone(), "qemu-img").exists(vm) {
        return Err(omaflake_core::Error::new(format!("No VM \"{vm}\"")).into());
    }
    let address = store(&env).join(network, vm)?;
    println!("{vm} joins network {network} as {address} (a second card from its next start).");
    Ok(())
}

pub fn leave(network: &str, vm: &str) -> Outcome {
    let env = Env::new();
    store(&env).leave(network, vm)?;
    println!("{vm} leaves network {network} (from its next start).");
    Ok(())
}

pub fn show(name: &str) -> Outcome {
    let env = Env::new();
    let networks = store(&env);
    let n = networks.get(name)?;
    let status = networks.status(name);
    print!("{}", show_text(&n, status.as_ref()));
    Ok(())
}

/// `omaf networks show`'s text
pub fn show_text(n: &Network, status: Option<&SwitchStatus>) -> String {
    let mut out = format!("{}  {}  switch {}", n.name, n.subnet(), n.switch_address());
    match status {
        Some(s) => out += &format!(" – running (pid {})\n", s.pid),
        None => out += " – not running (it starts with the first member)\n",
    }
    for (vm, host) in &n.members {
        let port = status.and_then(|s| s.ports.iter().find(|p| p.vm.as_deref() == Some(vm.as_str())));
        let line = match port {
            Some(p) => format!("connected ({}), in {} B, out {} B", p.kind, p.rx_bytes, p.tx_bytes),
            None => "not connected".into(),
        };
        out += &format!(
            "  {}{}{}{}\n",
            pad(vm, 16),
            pad(&n.address(*host).to_string(), 15),
            pad(&mac_text(&n.mac(*host)), 19),
            line
        );
    }
    if n.members.is_empty() {
        out += "  (no members – omaf networks join <network> <vm>)\n";
    }
    for f in status.map(|s| s.flows.as_slice()).unwrap_or_default() {
        out += &format!("  {} → {}: {} B\n", f.from, f.to, f.bytes);
    }
    out
}

/// `omaf network-switch <network> --data <folder>`: the switch process
pub fn switch(a: &cli::NetworkSwitch) -> Outcome {
    Switch::serve(Paths::new(&a.data, None), &a.network)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_lists_members_and_traffic() {
        let dir = tempfile::tempdir().unwrap();
        let networks = Networks::new(Paths::new(dir.path(), None));
        networks.create("dev").unwrap();
        networks.join("dev", "docker").unwrap();
        networks.join("dev", "mac1").unwrap();
        let n = networks.get("dev").unwrap();
        let idle = show_text(&n, None);
        assert!(idle.starts_with("dev  10.231.1.0/24  switch 10.231.1.1 – not running"), "{idle}");
        assert!(idle.contains("docker          10.231.1.10    02:6f:6d:01:0a:00  not connected"), "{idle}");
        let status: SwitchStatus = serde_json::from_value(serde_json::json!({
            "network": "dev", "pid": 42, "started": "2026-11-25T10:00:00Z", "updated": "2026-11-25T10:00:02Z",
            "ports": [{"kind": "qemu", "mac": "02:6f:6d:01:0a:00", "vm": "docker", "address": "10.231.1.10",
                       "rxFrames": 3, "rxBytes": 900, "txFrames": 2, "txBytes": 600, "dropped": 0}],
            "flows": [{"from": "mac1", "to": "docker", "bytes": 600}]
        }))
        .unwrap();
        let running = show_text(&n, Some(&status));
        assert!(running.contains("running (pid 42)"), "{running}");
        assert!(running.contains("connected (qemu), in 900 B, out 600 B"), "{running}");
        assert!(running.contains("mac1            10.231.1.11    02:6f:6d:01:0b:00  not connected"), "{running}");
        assert!(running.contains("mac1 → docker: 600 B"), "{running}");
    }
}
