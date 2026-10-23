// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaf macos – macOS bases (spec §6.5 step 1, docs/macos-base.md); port of Sources/omaf/MacOS.swift.

use std::io::Write;
use std::path::Path;

use omaflake_core::mac_base_installer::{BaseOutcome, BaseRequest};
use omaflake_core::platform::{full_user_name, user_name};
use omaflake_core::MacBaseInstaller;

use crate::cli;
use crate::util::{folder_path, read_password, Env, Outcome};

fn last_component(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

pub fn install(a: &cli::MacInstall) -> Outcome {
    let env = Env::new();
    let installer = MacBaseInstaller::new(env.paths.clone(), env.backends()?.vz()?.clone());
    let user = a.user.clone().unwrap_or_else(user_name);
    let mut request = BaseRequest::new(&user, &a.full_name.clone().unwrap_or_else(full_user_name), None);
    request.ipsw = a.ipsw.as_deref().map(folder_path);
    request.cpus = a.cpus;
    request.memory_gb = a.memory;
    request.disk_gb = a.disk;
    request.window = a.window;
    if let Some(base) = installer.existing_base(&request) {
        println!("{} exists already – nothing to do (delete {} to start over).", last_component(&base), base.display());
        return Ok(());
    }
    // Asked up front, so the long download/installation runs unattended
    if !a.no_first_boot {
        request.password =
            Some(read_password(&format!("Password for the new guest user {user}: "), a.password_stdin, true)?);
    }
    let outcome = installer.install(&request, &|_| {}, &|_, _| {}, &|line: &str| {
        println!("{line}");
        let _ = std::io::stdout().flush();
    })?;
    println!("{}", outcome_text(&outcome, &user));
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
