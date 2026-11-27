// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf volumes` (plan B187): folders of this computer that VMs share – a VM has none, one or several, each at
//! /Volumes/<name> in the guest, the name its own per VM (by default the folder's name).

use omaflake_core::instance_store::InstanceStore;
use omaflake_core::volumes::Volumes;

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
    let env = Env::new();
    let (volumes, store) = stores(&env);
    let path = path.map(crate::util::folder_path);
    let v = volumes.create(name, path.as_deref(), &store.list())?;
    println!("Volume {} created: {}", v.name, v.folder.display());
    Ok(())
}

pub fn remove(name: &str) -> Outcome {
    let env = Env::new();
    let (volumes, store) = stores(&env);
    volumes.remove(&store, name)?;
    println!("Volume {name} removed – out of its VMs from their next start; its folder stays.");
    Ok(())
}

pub fn attach(volume: &str, vm: &str, mount: Option<&str>) -> Outcome {
    let env = Env::new();
    let (volumes, store) = stores(&env);
    let c = volumes.attach(&store, volume, vm, mount)?;
    let name = volumes.mounts(&c).into_iter().find(|m| m.0 == volume).map(|m| m.1).unwrap_or_default();
    println!("{vm} has volume {volume} at /Volumes/{name} (from its next start).");
    Ok(())
}

pub fn detach(volume: &str, vm: &str) -> Outcome {
    let env = Env::new();
    let (volumes, store) = stores(&env);
    volumes.detach(&store, volume, vm)?;
    println!("{vm} loses volume {volume} at its next start; the folder stays.");
    Ok(())
}

pub fn rename(volume: &str, vm: &str, mount: Option<&str>) -> Outcome {
    let env = Env::new();
    let (volumes, store) = stores(&env);
    let c = volumes.rename_mount(&store, volume, vm, mount)?;
    let name = volumes.mounts(&c).into_iter().find(|m| m.0 == volume).map(|m| m.1).unwrap_or_default();
    println!("{vm} sees volume {volume} at /Volumes/{name} (from its next start).");
    Ok(())
}
