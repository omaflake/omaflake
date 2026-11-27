// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaf macos – macOS bases (spec §6.5 step 1, docs/macos-base.md); port of Sources/omaf/MacOS.swift.

use std::io::Write;
use std::path::Path;

use omaflake_core::MacBaseInstaller;
use omaflake_core::api::service::socket_path;
use omaflake_core::credentials::Credentials;
use omaflake_core::mac_base_installer::{BaseOutcome, BaseRequest};
use omaflake_core::mac_base_unchain::UnchainRequest;
use omaflake_core::{Error, Paths};
use serde_json::{Value, json};

use crate::cli;
use crate::commands::service;
use crate::util::{Env, Outcome, folder_path, usage};

fn last_component(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn install(a: &cli::MacInstall) -> Outcome {
    let env = Env::new();
    let installer = MacBaseInstaller::new(env.paths.clone(), env.backends()?.vz()?.clone());
    // The guest user from a credential, its password from the keychain – nothing is typed (plan B139)
    let login = Credentials::new(env.paths.clone()).base_login(a.credential.as_deref())?;
    let user = login.user.clone();
    let mut request = BaseRequest::new(&user, &login.full_name, None);
    request.ipsw = a.ipsw.as_deref().map(folder_path);
    request.cpus = a.cpus;
    request.memory_gb = a.memory;
    request.disk_gb = a.disk;
    request.window = a.window;
    request.credential = Some((login.credential.id.clone(), login.credential.name.clone()));
    if let Some(name) = &a.replace {
        let Some(old) = MacBaseInstaller::list(&env.paths).into_iter().find(|b| &b.name == name) else {
            return Err(usage(
                &["macos", "install"],
                format!("No macOS base {name} in {}", env.paths.bases().display()),
            ));
        };
        if in_use_by_service(&env.paths, name) {
            return Err(Error::new(format!(
                "{name} is in use – an image build works with it; try again when it is done"
            ))
            .into());
        }
        request.replace = Some(old.path);
    }
    if let Some(base) = installer.existing_base(&request) {
        println!("{} exists already – nothing to do (delete {} to start over).", last_component(&base), base.display());
        return Ok(());
    }
    if !a.no_first_boot {
        request.password = Some(login.password.clone());
    }
    println!("Guest user {user} from the credential {}.", login.credential.name);
    let outcome = installer.install(&request, &|_| {}, &|_, _| {}, &|line: &str| {
        println!("{line}");
        let _ = std::io::stdout().flush();
    })?;
    // The base it replaces goes once the new one is ready (plan B166) – not while the service builds with it
    let outcome = match (&request.replace, outcome) {
        (Some(old), BaseOutcome::Created { dir, ready: true })
            if !in_use_by_service(&env.paths, &last_component(old)) =>
        {
            let dir = installer.supersede(old, &dir, &|line: &str| println!("{line}"))?;
            BaseOutcome::Created { dir, ready: true }
        }
        (Some(old), outcome @ BaseOutcome::Created { ready: true, .. }) => {
            println!(
                "{} is kept – an image build uses it; omaf macos remove {} when that is done",
                last_component(old),
                last_component(old)
            );
            outcome
        }
        (_, outcome) => outcome,
    };
    println!("{}", outcome_text(&outcome, &user));
    Ok(())
}

/// The running service has a job that uses base `name` (`bases.list`'s `inUse`); false without a service
fn in_use_by_service(paths: &Paths, name: &str) -> bool {
    let Some((client, _)) = service::connect(&socket_path(paths)) else { return false };
    client
        .call("bases.list", Value::Null)
        .ok()
        .and_then(|v| v.as_array().cloned())
        .unwrap_or_default()
        .iter()
        .any(|b| b["name"] == json!(name) && b["inUse"] == json!(true))
}

/// `omaf macos bases`: every base, newest first (plan B166)
pub fn bases() -> Outcome {
    let env = Env::new();
    let credentials = Credentials::new(env.paths.clone());
    let list = MacBaseInstaller::list(&env.paths);
    if list.is_empty() {
        println!("No macOS base yet – omaf macos install");
        return Ok(());
    }
    for b in list {
        let credential = b.record.as_ref().map(|r| {
            credentials.get(&r.credential).map(|c| c.name).unwrap_or_else(|| format!("{} (gone)", r.credential_name))
        });
        let mut notes = Vec::new();
        if MacBaseInstaller::is_unchained(&b.path) {
            notes.push("unchained: System Integrity Protection off".to_string());
        }
        if b.newest {
            notes.push("newest of its credential".to_string());
        }
        if !b.ready {
            notes.push("unfinished".into());
        } else if b.record.is_none() {
            notes.push("no credential – omaf macos assign".into());
        } else if !b.record.as_ref().is_some_and(|r| r.own_key) {
            notes.push(format!("without omaflake's own SSH key – omaf macos install --replace {}", b.name));
        }
        println!(
            "{}  macOS {}{}  {}  user {}{}",
            b.name,
            b.os_build.as_deref().unwrap_or("?"),
            b.record.as_ref().and_then(|r| r.version.as_deref()).map(|v| format!(" ({v})")).unwrap_or_default(),
            credential.as_deref().unwrap_or("–"),
            b.user.as_deref().unwrap_or("?"),
            if notes.is_empty() { String::new() } else { format!("  [{}]", notes.join("; ")) }
        );
    }
    Ok(())
}

/// `omaf macos remove <base>`: through the service when it runs (it knows which builds use a base), else here
pub fn remove(a: &cli::MacRemove) -> Outcome {
    let env = Env::new();
    if let Some((client, _)) = service::connect(&socket_path(&env.paths)) {
        client.call("macos.removeBase", json!({"name": a.base})).map_err(Error::from)?;
        println!("Removed {}", a.base);
        return Ok(());
    }
    let Some(base) = MacBaseInstaller::list(&env.paths).into_iter().find(|b| b.name == a.base) else {
        return Err(usage(
            &["macos", "remove"],
            format!("No macOS base {} in {}", a.base, env.paths.bases().display()),
        ));
    };
    omaflake_core::trash::Disposal::delete(&[&base.path])?;
    println!("Removed {} – macOS images and VMs made from it stay", a.base);
    Ok(())
}

/// `omaf macos unchain <base>` (plan B184): the unchained copy of a base – here, never in the service: Recovery and
/// the extension's approval are done by the person at this terminal, in the VM's window
pub fn unchain(a: &cli::MacUnchain) -> Outcome {
    let env = Env::new();
    let Some(base) = MacBaseInstaller::list(&env.paths).into_iter().find(|b| b.name == a.base) else {
        return Err(usage(
            &["macos", "unchain"],
            format!("No macOS base {} in {}", a.base, env.paths.bases().display()),
        ));
    };
    let installer = MacBaseInstaller::new(env.paths.clone(), env.backends()?.vz()?.clone());
    // Before any password is looked up: what cannot be unchained says so
    let login = if base.ready && base.record.is_some() && !MacBaseInstaller::is_unchained(&base.path) {
        Some(Credentials::new(env.paths.clone()).login_of_base(&base.path)?)
    } else {
        None
    };
    let tools = if a.no_tools { None } else { MacBaseInstaller::unchained_tools() };
    if !a.no_tools && tools.is_none() {
        println!(
            "This omaflake has no omaflake Tools with the camera extension – the base gets System Integrity Protection off only."
        );
    }
    let request = UnchainRequest {
        source: base.path.clone(),
        password: login.map(|l| l.password).unwrap_or_default(),
        tools,
        // This Mac's keyboard for the base (it has the US one): the password for the extension's approval is typed
        keyboard: env.settings().host_keyboard,
    };
    let step = |n: u32, title: &str| println!("[{n}/{}] {title}", omaflake_core::mac_base_unchain::STEPS);
    let outcome = installer.unchain(
        &request,
        &|line: &str| {
            println!("{line}");
            let _ = std::io::stdout().flush();
        },
        &step,
    )?;
    let (BaseOutcome::Created { dir, .. } | BaseOutcome::Existing(dir)) = outcome;
    println!(
        "Unchained macOS images: omaf build <recipe> --unchained (from {}). Their VMs run with System Integrity \
         Protection off.",
        last_component(&dir)
    );
    Ok(())
}

/// `omaf macos assign <base> <credential>`: the credential of a base from before bases kept theirs (plan B139)
pub fn assign(a: &cli::MacAssign) -> Outcome {
    let env = Env::new();
    let Some(base) = MacBaseInstaller::list(&env.paths).into_iter().find(|b| b.name == a.base) else {
        return Err(usage(
            &["macos", "assign"],
            format!("No macOS base {} in {}", a.base, env.paths.bases().display()),
        ));
    };
    let record = Credentials::new(env.paths.clone()).assign_to_base(&base.path, &a.credential)?;
    println!(
        "{} uses the credential {} – macOS images are built with its password now (a wrong one shows at the next build).",
        a.base, record.credential_name
    );
    Ok(())
}

/// What `omaf macos install` says at the end
pub fn outcome_text(outcome: &BaseOutcome, user: &str) -> String {
    match outcome {
        BaseOutcome::Existing(base) => format!("Delete {} to start over.", base.display()),
        BaseOutcome::Created { dir, ready: true } => format!(
            "Base {} is ready. Set macUser in nix/local/settings.nix to {user} if it differs\nfrom your NixOS guest \
             user, then build an image: omaf build macos/minimal",
            last_component(dir)
        ),
        BaseOutcome::Created { dir, ready: false } => format!(
            "Base {} cloned without the first boot (omaf-runner run … --user … to provision it).",
            last_component(dir)
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn outcomes() {
        let dir = PathBuf::from("/data/bases/mac-base-26A434");
        assert_eq!(
            outcome_text(&BaseOutcome::Created { dir: dir.clone(), ready: true }, "guest"),
            "Base mac-base-26A434 is ready. Set macUser in nix/local/settings.nix to guest if it differs\nfrom your \
             NixOS guest user, then build an image: omaf build macos/minimal"
        );
        assert_eq!(
            outcome_text(&BaseOutcome::Created { dir: dir.clone(), ready: false }, "guest"),
            "Base mac-base-26A434 cloned without the first boot (omaf-runner run … --user … to provision it)."
        );
        assert_eq!(
            outcome_text(&BaseOutcome::Existing(dir), "guest"),
            "Delete /data/bases/mac-base-26A434 to start over."
        );
    }
}
