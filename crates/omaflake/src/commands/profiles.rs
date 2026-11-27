// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf profiles` (plan B142): how big a new VM is – CPU cores and RAM –, the default one used by `omaf new` unless
//! `--profile` names another.

use omaflake_core::profiles::{ProfileChange, Profiles};

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

pub fn add(a: &cli::ProfileArgs) -> Outcome {
    let env = Env::new();
    let p = store(&env).add(ProfileChange {
        name: Some(a.name.clone()),
        cpus: Some(a.cpus.unwrap_or(4)),
        memory_gb: Some(a.memory.unwrap_or(8)),
    })?;
    println!("Profile {} added ({} CPU, {} GB RAM).", p.name, p.cpus, p.memory_gb);
    Ok(())
}

pub fn edit(a: &cli::ProfileArgs) -> Outcome {
    let env = Env::new();
    let profiles = store(&env);
    let Some(p) = profiles.find(&a.name) else {
        return Err(usage(&["profiles", "edit"], format!("No profile {} – see omaf profiles", a.name)));
    };
    let p = profiles.update(&p.id, ProfileChange { name: a.rename.clone(), cpus: a.cpus, memory_gb: a.memory })?;
    println!("Profile {} changed ({} CPU, {} GB RAM).", p.name, p.cpus, p.memory_gb);
    Ok(())
}

pub fn remove(reference: &str) -> Outcome {
    let env = Env::new();
    let profiles = store(&env);
    let Some(p) = profiles.find(reference) else {
        return Err(usage(&["profiles", "rm"], format!("No profile {reference} – see omaf profiles")));
    };
    profiles.remove(&p.id)?;
    println!("Profile {} removed.", p.name);
    Ok(())
}

pub fn set_default(reference: &str) -> Outcome {
    let env = Env::new();
    let profiles = store(&env);
    let Some(p) = profiles.find(reference) else {
        return Err(usage(&["profiles", "default"], format!("No profile {reference} – see omaf profiles")));
    };
    profiles.set_default(&p.id)?;
    println!("New VMs get the profile {}.", p.name);
    Ok(())
}
