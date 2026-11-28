// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf profiles` (plan B142): how big a new VM is – CPU cores and RAM –, the default one used by `omaf new` unless
//! `--profile` names another.

use omaflake_core::profiles::{Profile, Profiles};
use serde_json::json;

use super::service;
use crate::cli;
use crate::util::{Env, Outcome, pad, usage};

fn store(env: &Env) -> Profiles {
    Profiles::new(env.paths.clone())
}

pub fn list() -> Outcome {
    let env = Env::new();
    let profiles = store(&env);
    let default = profiles.default_profile().map(|p| p.id);
    let all = profiles.list();
    for p in &all {
        println!(
            "{} {}{} CPU · {} GB RAM",
            if default.as_deref() == Some(p.id.as_str()) { "*" } else { " " },
            pad(&p.name, 16),
            p.cpus,
            p.memory_gb
        );
    }
    if all.is_empty() {
        println!(
            "(no profiles yet – new VMs get what their recipe asks for: omaf profiles add <name> --cpus 4 --memory 8)"
        );
    }
    Ok(())
}

/// The profile `id` as it is stored now
fn stored(id: &str) -> omaflake_core::Result<Profile> {
    store(&Env::new())
        .list()
        .into_iter()
        .find(|p| p.id == id)
        .ok_or_else(|| omaflake_core::Error::new(format!("No profile {id} – see omaf profiles")))
}

/// `profiles.add` / `profiles.update`: only what is given
fn change(
    method: &str,
    id: Option<&str>,
    name: Option<&str>,
    cpus: Option<u32>,
    memory: Option<u32>,
) -> omaflake_core::Result<Profile> {
    let mut params = json!({});
    let given = [("id", id.map(|v| json!(v))), ("name", name.map(|v| json!(v)))];
    let sizes = [("cpus", cpus.map(|v| json!(v))), ("memoryGB", memory.map(|v| json!(v)))];
    for (key, value) in given.into_iter().chain(sizes) {
        if let Some(v) = value {
            params[key] = v;
        }
    }
    let answer = service::rpc(method, params)?;
    stored(answer["id"].as_str().unwrap_or_default())
}

pub fn add(a: &cli::ProfileArgs) -> Outcome {
    let p = change("profiles.add", None, Some(&a.name), Some(a.cpus.unwrap_or(4)), Some(a.memory.unwrap_or(8)))?;
    println!("Profile {} added ({} CPU, {} GB RAM).", p.name, p.cpus, p.memory_gb);
    Ok(())
}

pub fn edit(a: &cli::ProfileArgs) -> Outcome {
    let Some(p) = store(&Env::new()).find(&a.name) else {
        return Err(usage(&["profiles", "edit"], format!("No profile {} – see omaf profiles", a.name)));
    };
    let p = change("profiles.update", Some(&p.id), a.rename.as_deref(), a.cpus, a.memory)?;
    println!("Profile {} changed ({} CPU, {} GB RAM).", p.name, p.cpus, p.memory_gb);
    Ok(())
}

pub fn remove(reference: &str) -> Outcome {
    let Some(p) = store(&Env::new()).find(reference) else {
        return Err(usage(&["profiles", "rm"], format!("No profile {reference} – see omaf profiles")));
    };
    service::rpc("profiles.remove", json!({"id": p.id}))?;
    println!("Profile {} removed.", p.name);
    Ok(())
}

pub fn set_default(reference: &str) -> Outcome {
    let Some(p) = store(&Env::new()).find(reference) else {
        return Err(usage(&["profiles", "default"], format!("No profile {reference} – see omaf profiles")));
    };
    service::rpc("profiles.setDefault", json!({"id": p.id}))?;
    println!("New VMs get the profile {}.", p.name);
    Ok(())
}
