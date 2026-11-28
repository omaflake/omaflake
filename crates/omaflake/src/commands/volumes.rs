// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf volumes` (plan B187): folders of this computer that VMs share – a VM has none, one or several, each at
//! /Volumes/<name> in the guest, the name its own per VM (by default the folder's name).

use omaflake_core::instance_store::InstanceStore;
use omaflake_core::volumes::Volumes;

use serde_json::json;

use super::service;
use crate::util::{Env, Outcome, pad};

fn stores(env: &Env) -> (Volumes, InstanceStore) {
    (Volumes::new(env.paths.clone()), InstanceStore::new(env.paths.clone(), "qemu-img"))
}

pub fn list() -> Outcome {
    let env = Env::new();
    let (volumes, store) = stores(&env);
    let all = volumes.dtos(&store.list());
    for v in &all {
        let vms: Vec<String> = v.vms.iter().map(|m| format!("{} (/Volumes/{})", m.vm, m.mount)).collect();
        println!("{}{}  {}", pad(&v.name, 16), v.folder, if vms.is_empty() { "–".to_string() } else { vms.join(", ") });
    }
    if all.is_empty() {
        println!("(no volumes yet – omaf volumes create <name>, then omaf volumes attach <name> <vm>)");
    }
    Ok(())
}

pub fn create(name: &str, path: Option<&str>) -> Outcome {
    let mut params = json!({"name": name});
    if let Some(p) = path {
        params["path"] = json!(crate::util::folder_path(p));
    }
    let v = service::rpc("volumes.create", params)?;
    println!("Volume {} created: {}", v["name"].as_str().unwrap_or(name), v["folder"].as_str().unwrap_or("?"));
    Ok(())
}

pub fn remove(name: &str) -> Outcome {
    let env = Env::new();
    let (volumes, store) = stores(&env);
    let folder = volumes.list(&store.list()).into_iter().find(|v| v.name == name).map(|v| v.folder);
    service::rpc("volumes.remove", json!({"name": name}))?;
    // A default folder holding only what macOS leaves there goes with it; the user's files always stay
    match folder.filter(|f| f.exists()) {
        Some(f) => {
            println!("Volume {name} removed – out of its VMs from their next start; its folder {} stays.", f.display())
        }
        None => println!("Volume {name} removed – out of its VMs from their next start; its empty folder is gone."),
    }
    Ok(())
}

/// The name `vm` sees `volume` under (/Volumes/<name>), as its file says now
fn mount_name(vm: &str, volume: &str) -> String {
    let env = Env::new();
    let (volumes, store) = stores(&env);
    let mounts = store.load(vm).map(|c| volumes.mounts(&c)).unwrap_or_default();
    mounts.into_iter().find(|m| m.0 == volume).map(|m| m.1).unwrap_or_default()
}

fn of_vm(volume: &str, vm: &str, mount: Option<&str>) -> serde_json::Value {
    let mut params = json!({"volume": volume, "vm": vm});
    if let Some(m) = mount {
        params["as"] = json!(m);
    }
    params
}

pub fn attach(volume: &str, vm: &str, mount: Option<&str>) -> Outcome {
    service::rpc("volumes.attach", of_vm(volume, vm, mount))?;
    println!("{vm} has volume {volume} at /Volumes/{} (from its next start).", mount_name(vm, volume));
    Ok(())
}

pub fn detach(volume: &str, vm: &str) -> Outcome {
    service::rpc("volumes.detach", of_vm(volume, vm, None))?;
    println!("{vm} loses volume {volume} at its next start; the folder stays.");
    Ok(())
}

pub fn rename(volume: &str, vm: &str, mount: Option<&str>) -> Outcome {
    service::rpc("volumes.rename", of_vm(volume, vm, mount))?;
    println!("{vm} sees volume {volume} at /Volumes/{} (from its next start).", mount_name(vm, volume));
    Ok(())
}
