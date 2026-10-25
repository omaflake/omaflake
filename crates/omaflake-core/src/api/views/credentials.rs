// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Credentials page (`credentials`, plan B137): who you are in a guest – named sets of user, full name, e-mail and
//! an optional stored password (for macOS builds). Create, edit, remove them here; an image is built with one of them,
//! the settings name the default.

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::*;
use super::text::{tf, Text};
use super::Ctx;
use crate::api::types::CredentialDto;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialsPage {
    /// "Credentials"
    pub title: Text,
    /// What a credential is, and what an image gets without one (this computer's user and name)
    pub intro: Text,
    pub rows: Vec<CredentialRow>,
    /// "New Credential …" (asks name, user, full name, e-mail) – primary
    pub add: Action,
    /// Where passwords are kept
    pub footer: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialRow {
    pub id: String,
    pub name: String,
    /// "alex · Alex Doe · alex@example.org · 2 images"
    pub subtitle: Text,
    /// default, password stored
    pub badges: Vec<Badge>,
    /// Edit …, Set Password … / Remove Password, Make Default, Remove …
    pub actions: Vec<Action>,
}

fn text_ask(param: &str, label: &str, default: Option<&str>) -> Ask {
    let mut a = Ask::new(param, AskKind::Text, Text::new(label));
    a.default = default.map(str::to_string);
    a
}

/// The asks of a new or an edited credential: name, user, full name, e-mail (prefilled with `c`'s values)
fn asks(ctx: &Ctx, c: Option<&CredentialDto>) -> Vec<Ask> {
    let mut user = text_ask("user", "User in the guests (a–z, 0–9, _ and -)", None);
    user.default = Some(c.and_then(|c| c.user.clone()).unwrap_or_else(|| ctx.host_user.clone()));
    let mut full = text_ask("fullName", "Full name (git, the account)", None);
    full.default = Some(c.and_then(|c| c.full_name.clone()).unwrap_or_else(|| ctx.host_full_name.clone()));
    vec![
        text_ask("name", "Name of the credential (e.g. Work)", c.map(|c| c.name.as_str())),
        user,
        full,
        text_ask("email", "E-mail (git; may stay empty)", c.and_then(|c| c.email.as_deref())),
    ]
}

/// Images built with credential `id`
fn images_with(ctx: &Ctx, id: &str) -> usize {
    ctx.images.iter().filter(|i| i.credential.as_deref() == Some(id)).count()
}

fn row(ctx: &Ctx, c: &CredentialDto) -> CredentialRow {
    let mut parts = vec![Text::verbatim(c.user.clone().unwrap_or_else(|| ctx.host_user.clone()))];
    if let Some(full) = c.full_name.as_deref().filter(|f| !f.is_empty()) {
        parts.push(Text::verbatim(full));
    }
    if let Some(email) = c.email.as_deref() {
        parts.push(Text::verbatim(email));
    }
    parts.push(match images_with(ctx, &c.id) {
        0 => Text::new("no image"),
        1 => Text::new("1 image"),
        n => tf!("%lld images", n),
    });
    let mut badges = Vec::new();
    if c.is_default {
        badges.push(
            Badge::new("default", "ok", Text::new("The default for new images (Settings)"), Tone::Accent)
                .label(Text::new("default")),
        );
    }
    if c.has_password {
        badges.push(Badge::new(
            "password",
            "lock",
            Text::new("A password is stored – macOS images are built without asking for it"),
            Tone::Normal,
        ));
    }
    let id = json!({"id": c.id});
    let mut edit = Action::call("edit", Text::new("Edit …"), None, "credentials.update", id.clone()).primary();
    edit.ask = asks(ctx, Some(c));
    let mut password = Ask::new("password", AskKind::Password, tf!("Password of %@ in macOS guests", &c.name));
    password.prompt = Some(Text::new(
        "The password of the macOS base's user – a macOS build needs it once (sudo in the build VM). It is kept in the keychain, never in a file of omaflake's.",
    ));
    password.repeat = true;
    let mut set_password = Action::call(
        "password",
        Text::new(if c.has_password { "Change Password …" } else { "Set Password …" }),
        Some("lock"),
        "credentials.update",
        id.clone(),
    );
    set_password.ask = vec![password];
    let remove_password = Action::call(
        "removePassword",
        Text::new("Remove Password"),
        Some("lock"),
        "credentials.update",
        json!({"id": c.id, "password": ""}),
    )
    .enabled_if(c.has_password, Some(Text::new("No password stored")))
    .confirm(
        Confirm::new(
            tf!("Remove the password of “%@”?", &c.name),
            Text::new("macOS builds with this credential ask for it again."),
        )
        .button("Remove"),
    );
    let default = Action::call(
        "default",
        Text::new("Make Default"),
        Some("ok"),
        "settings.set",
        json!({"defaultCredential": c.id}),
    )
    .enabled_if(!c.is_default, Some(Text::new("Already the default")));
    let n = images_with(ctx, &c.id);
    let remove_text = if n == 0 {
        Text::new("The credential and its stored password are deleted.")
    } else {
        tf!(
            "The credential and its stored password are deleted. %lld images built with it keep its name, but a new build of them needs another credential.",
            n
        )
    };
    let remove = Action::call("remove", Text::new("Remove …"), Some("trash"), "credentials.remove", id)
        .destructive()
        .confirm(Confirm::new(tf!("Remove the credential “%@”?", &c.name), remove_text).button("Remove").destructive());
    CredentialRow {
        id: c.id.clone(),
        name: c.name.clone(),
        subtitle: Text::joined(parts),
        badges,
        actions: vec![edit, set_password, remove_password, default, remove],
    }
}

pub(crate) fn page(ctx: &Ctx) -> CredentialsPage {
    let mut add =
        Action::call("add", Text::new("New Credential …"), Some("add"), "credentials.add", json!({})).primary();
    add.ask = asks(ctx, None);
    CredentialsPage {
        title: Text::new("Credentials"),
        intro: tf!(
            "Who you are in a guest: user, full name and e-mail – and the password a macOS build needs. Every image is built with one credential; without one it gets this computer's: %@ (%@).",
            &ctx.host_user,
            &ctx.host_full_name
        ),
        rows: ctx.credentials.iter().map(|c| row(ctx, c)).collect(),
        add,
        footer: Text::new(ctx.on_host(
            "Passwords are kept in the login keychain. The default credential for new images is chosen in the Settings; a changed credential makes its images outdated.",
            "Passwords are kept in the desktop's keyring (Secret Service), else unencrypted in ~/.config/omaflake/tokens.json. The default credential for new images is chosen in the Settings; a changed credential makes its images outdated.",
        )),
    }
}
