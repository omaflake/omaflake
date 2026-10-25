// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaf macos – macOS bases (spec §6.5 step 1, docs/macos-base.md); port of Sources/omaf/MacOS.swift.

use std::io::Write;
use std::path::Path;

use omaflake_core::credentials::Credentials;
use omaflake_core::mac_base_installer::{BaseOutcome, BaseRecord, BaseRequest};
use omaflake_core::MacBaseInstaller;

use crate::cli;
use crate::util::{folder_path, usage, Env, Outcome};

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
    if let BaseOutcome::Created { dir, ready: true } = &outcome {
        BaseRecord {
            credential: login.credential.id.clone(),
            credential_name: login.credential.name.clone(),
            user: user.clone(),
        }
        .write(dir)?;
    }
    println!("{}", outcome_text(&outcome, &user));
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
