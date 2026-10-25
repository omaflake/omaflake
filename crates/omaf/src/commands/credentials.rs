// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf credentials` (plan B137): who you are in a guest – named sets of user, full name, e-mail and a stored password
//! for macOS builds. An image is built with one of them (`omaf build --credential`), the default is the settings'.

use omaflake_core::credentials::{Credential, CredentialChange, Credentials};
use omaflake_core::guest_settings::BuildChoice;
use omaflake_core::platform::{full_user_name, user_name};
use omaflake_core::{GuestChoices, Paths};

use crate::cli;
use crate::util::{pad, read_password, usage, Env, Failure, Outcome};

fn credential(credentials: &Credentials, reference: &str, command: &'static str) -> Result<Credential, Failure> {
    credentials
        .find(reference)
        .ok_or_else(|| usage(&["credentials", command], format!("No credential {reference} – see omaf credentials")))
}

pub fn list() -> Outcome {
    let env = Env::new();
    let credentials = Credentials::new(env.paths.clone());
    let default = credentials.default_credential().map(|c| c.id);
    let all = credentials.list();
    let mark = |yes: bool| if yes { "*" } else { " " };
    println!("{} {}{}{}", mark(default.is_none()), pad("computer", 16), pad(&user_name(), 14), full_user_name());
    for c in &all {
        let mut notes = Vec::new();
        if let Some(e) = &c.email {
            notes.push(e.clone());
        }
        if credentials.has_password(&c.id) {
            notes.push("password stored".into());
        }
        println!(
            "{} {}{}{}{}",
            mark(default.as_deref() == Some(c.id.as_str())),
            pad(&c.name, 16),
            pad(c.user.as_deref().unwrap_or("–"), 14),
            c.full_name.as_deref().unwrap_or("–"),
            if notes.is_empty() { String::new() } else { format!("  ({})", notes.join(", ")) }
        );
    }
    if all.is_empty() {
        println!("(no credentials yet – omaf credentials add <name> --user <user> --full-name <name>)");
    }
    Ok(())
}

fn change(a: &cli::CredentialArgs, name: Option<String>, adding: bool) -> Result<CredentialChange, Failure> {
    let password = if a.password {
        Some(read_password("Password of the macOS base's user (stored in the keychain/keyring): ", false, true)?)
    } else if a.no_password && !adding {
        Some(String::new())
    } else {
        None
    };
    Ok(CredentialChange {
        name,
        user: a.user.clone().or_else(|| adding.then(user_name)),
        full_name: a.full_name.clone().or_else(|| adding.then(full_user_name)),
        email: a.email.clone(),
        password,
    })
}

fn said(place: Option<omaflake_core::source_tokens::TokenPlace>) {
    if let Some(omaflake_core::source_tokens::TokenPlace::File(path)) = place {
        eprintln!("The password is stored unencrypted in {} – no keyring", path.display());
    }
}

pub fn add(a: &cli::CredentialArgs) -> Outcome {
    let env = Env::new();
    let (c, place) = Credentials::new(env.paths.clone()).add(change(a, Some(a.name.clone()), true)?)?;
    said(place);
    println!("Credential {} added ({}).", c.name, c.user.as_deref().unwrap_or("–"));
    Ok(())
}

pub fn edit(a: &cli::CredentialArgs) -> Outcome {
    let env = Env::new();
    let credentials = Credentials::new(env.paths.clone());
    let c = credential(&credentials, &a.name, "edit")?;
    let (c, place) = credentials.update(&c.id, change(a, a.rename.clone(), false)?)?;
    said(place);
    println!("Credential {} changed – images built with it are now outdated: build them again to take it.", c.name);
    Ok(())
}

pub fn remove(reference: &str) -> Outcome {
    let env = Env::new();
    let credentials = Credentials::new(env.paths.clone());
    let c = credential(&credentials, reference, "rm")?;
    credentials.remove(&c.id)?;
    println!("Credential {} removed.", c.name);
    Ok(())
}

pub fn set_default(reference: &str) -> Outcome {
    let env = Env::new();
    let credentials = Credentials::new(env.paths.clone());
    if reference == "computer" || reference.is_empty() {
        credentials.set_default(None)?;
        println!("New images get this computer's user and name.");
        return Ok(());
    }
    let c = credential(&credentials, reference, "default")?;
    credentials.set_default(Some(&c.id))?;
    println!("New images get the credential {}.", c.name);
    Ok(())
}

/// `--credential` and `--look` of `omaf build`: None = the defaults; "computer" = this computer's values; "recipe" =
/// the recipe's own look
pub fn build_choice(paths: &Paths, credential_ref: Option<&str>, look: Option<&str>) -> Result<BuildChoice, Failure> {
    let defaults = BuildChoice::defaults(paths);
    let credential = match credential_ref {
        None => defaults.credential,
        Some("computer") | Some("") => None,
        Some(r) => Some(
            credential(&Credentials::new(paths.clone()), r, "list")
                .map_err(|_| usage(&["build"], format!("--credential: no credential {r} – see omaf credentials")))?,
        ),
    };
    let look = match look {
        None => defaults.look,
        Some("recipe") | Some("") => None,
        Some(l) if GuestChoices::LOOKS.iter().any(|(n, _)| *n == l) => Some(l.to_string()),
        Some(l) => {
            let names: Vec<&str> = GuestChoices::LOOKS.iter().map(|(n, _)| *n).collect();
            return Err(usage(&["build"], format!("--look {l}: {} or recipe", names.join(", "))));
        }
    };
    Ok(BuildChoice { credential, look })
}
