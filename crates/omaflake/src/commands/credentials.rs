// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf credentials` (plan B137, B139): who you are in a guest – named sets of user, full name, e-mail and a password,
//! kept in the keychain for macOS bases and builds. An image is built with one of them (`omaf build --credential`), a
//! macOS base is created with one; the default is the settings'. Only the first one starts from this computer's user.

use omaflake_core::Paths;
use omaflake_core::credentials::{Credential, CredentialChange, Credentials};
use omaflake_core::guest_settings::BuildChoice;
use omaflake_core::platform::{full_user_name, user_name};

use crate::cli;
use crate::util::{Env, Failure, Outcome, pad, read_password, usage};

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
        println!("(no credentials yet – omaf credentials add <name>: images and macOS bases need one)");
    }
    Ok(())
}

fn change(
    a: &cli::CredentialArgs,
    name: Option<String>,
    adding: bool,
    first: bool,
) -> Result<CredentialChange, Failure> {
    // Every credential has a password (plan B137): asked when it is added (stdin with --password), on edit only with
    // --password
    let prompt = "Password of the guest's user (kept in the keychain/keyring; omaflake uses it by itself): ";
    let password = if adding {
        Some(read_password(prompt, a.password, !a.password)?)
    } else if a.password {
        Some(read_password(prompt, !std::io::IsTerminal::is_terminal(&std::io::stdin()), true)?)
    } else {
        None
    };
    Ok(CredentialChange {
        name,
        // Only the first credential starts from this computer's user and name (plan B139)
        user: a.user.clone().or_else(|| (adding && first).then(user_name)),
        full_name: a.full_name.clone().or_else(|| (adding && first).then(full_user_name)),
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
    let credentials = Credentials::new(env.paths.clone());
    if a.user.is_none() && !credentials.list().is_empty() {
        return Err(usage(&["credentials", "add"], "--user: the guest user of this credential"));
    }
    let first = credentials.list().is_empty();
    let (c, place) = credentials.add(change(a, Some(a.name.clone()), true, first)?)?;
    said(place);
    println!("Credential {} added ({}).", c.name, c.user.as_deref().unwrap_or("–"));
    Ok(())
}

pub fn edit(a: &cli::CredentialArgs) -> Outcome {
    let env = Env::new();
    let credentials = Credentials::new(env.paths.clone());
    let c = credential(&credentials, &a.name, "edit")?;
    let change = change(a, a.rename.clone(), false, false)?;
    // Images and macOS bases carry its values: while used, only the name can change (plan B137, B139)
    omaflake_core::credentials::Usage::of(&env.paths, &c.id).check_change(&c, &change)?;
    let (c, place) = credentials.update(&c.id, change)?;
    said(place);
    println!("Credential {} changed – images built with it are now outdated: build them again to take it.", c.name);
    Ok(())
}

pub fn remove(reference: &str) -> Outcome {
    let env = Env::new();
    let credentials = Credentials::new(env.paths.clone());
    let c = credential(&credentials, reference, "rm")?;
    omaflake_core::credentials::Usage::of(&env.paths, &c.id).check_remove()?;
    credentials.remove(&c.id)?;
    println!("Credential {} removed.", c.name);
    Ok(())
}

pub fn set_default(reference: &str) -> Outcome {
    let env = Env::new();
    let credentials = Credentials::new(env.paths.clone());
    let c = credential(&credentials, reference, "default")?;
    credentials.set_default(&c.id)?;
    println!("New images and macOS bases get the credential {}.", c.name);
    Ok(())
}

pub fn copy_password(reference: &str) -> Outcome {
    let env = Env::new();
    let credentials = Credentials::new(env.paths.clone());
    let c = credential(&credentials, reference, "copy-password")?;
    let Some(password) = credentials.password(&c.id) else {
        return Err(usage(&["credentials", "copy-password"], format!("No password stored for {}", c.name)));
    };
    if !omaflake_core::credentials::confirm_owner(&format!("copy the password of “{}”", c.name))? {
        return Err(usage(&["credentials", "copy-password"], "Not copied – not confirmed"));
    }
    omaflake_core::credentials::copy_to_clipboard(&password)?;
    println!("Password of {} copied.", c.name);
    Ok(())
}

/// `--credential` and `--look` of `omaf build`: None = the defaults (a credential is needed – plan B139); a look is an
/// installed one (plan B141)
pub fn build_choice(
    paths: &Paths,
    credential_ref: Option<&str>,
    look: Option<&str>,
    locale: Option<&str>,
) -> Result<BuildChoice, Failure> {
    let defaults = BuildChoice::defaults(paths);
    let credential = match credential_ref.filter(|r| !r.is_empty()) {
        None => {
            Some(defaults.credential.ok_or_else(|| {
                usage(&["build"], "No credential yet – create one first: omaf credentials add <name>")
            })?)
        }
        Some(r) => Some(
            credential(&Credentials::new(paths.clone()), r, "list")
                .map_err(|_| usage(&["build"], format!("--credential: no credential {r} – see omaf credentials")))?,
        ),
    };
    let look = match look.filter(|l| !l.is_empty()) {
        None => defaults.look,
        Some(l) => Some(
            omaflake_core::looks::LookStore::new(paths.clone())
                .find(&omaflake_core::Settings::load(paths), l)
                .ok_or_else(|| usage(&["build"], format!("--look {l}: not installed – see omaf looks")))?
                .id,
        ),
    };
    let locale = match locale.filter(|l| !l.is_empty()) {
        None => defaults.locale,
        Some(l) => Some(
            omaflake_core::locales::Locales::new(paths.clone())
                .find(l)
                .ok_or_else(|| usage(&["build"], format!("--locale {l}: no such locale – see omaf locales")))?,
        ),
    };
    Ok(BuildChoice { credential, look, locale })
}
