// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Credentials page (`credentials`, plan B137): who you are in a guest – named sets of user, full name, e-mail and
//! an optional stored password (for macOS builds); an image is built with one of them, the settings name the default.
//! The form of one (`credential`): a new one, or one to edit – only its name while images use it.

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
    /// On while images use it
    pub lamp: Lamp,
    /// "alex · 2 images"
    pub subtitle: Text,
    /// default, no password
    pub badges: Vec<Badge>,
    /// The images built with it
    pub used_by: Vec<String>,
    /// The quick action: Copy Password
    pub actions: Vec<Action>,
    /// Opened: its values with what each is for, the images using it; Edit …, Set/Change Password …, Make Default,
    /// Remove … (only while no image uses it)
    pub detail: CredentialDetail,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialDetail {
    pub info: Vec<InfoLine>,
    pub actions: Vec<Action>,
}

/// The form of one credential (`credential`): new (no `id`) or to edit
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialPage {
    /// "New Credential" / "Edit Credential"
    pub title: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// What a credential is for
    pub intro: Text,
    /// name, user, full name, e-mail – each with what it is for in NixOS and macOS guests
    pub fields: Vec<Field>,
    /// Why only the name can change (images use it)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locked: Option<Text>,
    /// Cancel, Save / Create
    pub actions: Vec<Action>,
}

/// The form's values (the UI keeps them while the user types); missing ones are the credential's (or the defaults)
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CredentialValues {
    pub name: Option<String>,
    pub user: Option<String>,
    pub full_name: Option<String>,
    pub email: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct CredentialParams {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub values: CredentialValues,
}

/// A user name the guests take: a–z first (or "_"), then a–z, 0–9, "_" and "-", at most 32
pub fn valid_user(u: &str) -> bool {
    u.len() <= 32
        && u.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c == '_')
        && u.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '_' || c == '-')
}

pub(crate) fn credential_page(ctx: &Ctx, p: &CredentialParams) -> Option<CredentialPage> {
    let c = match &p.id {
        Some(id) => Some(ctx.credentials.iter().find(|c| &c.id == id)?),
        None => None,
    };
    let v = &p.values;
    let name = v.name.clone().unwrap_or_else(|| c.map(|c| c.name.clone()).unwrap_or_default());
    let user =
        v.user.clone().unwrap_or_else(|| c.and_then(|c| c.user.clone()).unwrap_or_else(|| ctx.host_user.clone()));
    let full = v
        .full_name
        .clone()
        .unwrap_or_else(|| c.and_then(|c| c.full_name.clone()).unwrap_or_else(|| ctx.host_full_name.clone()));
    let email = v.email.clone().unwrap_or_else(|| c.and_then(|c| c.email.clone()).unwrap_or_default());
    let used = c.map_or(0, |c| images_with(ctx, &c.id));
    let locked = (used > 0).then(|| {
        tf!(
            "%lld images are built with it, so only its name can change – they carry its user, full name and e-mail. For other values create a new credential (or remove those images first).",
            used
        )
    });
    let lock_reason = locked.as_ref().map(|_| Text::new("Used by images – only the name can change"));
    let text = |id: &str, label: &str, value: &str, help: &str| {
        Field::new(id, FieldKind::Text, Text::new(label), json!(value)).param(id).value_param(id).help(Text::new(help))
    };
    let mut name_field =
        text("name", "Name", &name, "Only for you – to tell your credentials apart in the lists and when building.");
    let taken =
        ctx.credentials.iter().any(|x| Some(&x.id) != p.id.as_ref() && x.name.eq_ignore_ascii_case(name.trim()));
    if name.trim().is_empty() {
        name_field.error = Some(Text::new("A credential needs a name"));
    } else if taken {
        name_field.error = Some(tf!("There is a credential \"%@\" already", name.trim()));
    }
    let mut user_field = text(
        "user",
        "User",
        &user,
        "NixOS: the login user – the automatic login, the home folder /home/<user> and omaf ssh. macOS guests keep the base's user.",
    )
    .enabled_if(locked.is_none(), lock_reason.clone());
    if !valid_user(user.trim()) {
        user_field.error =
            Some(tf!("Invalid user name \"%@\": a–z, 0–9, \"_\" and \"-\", starting with a letter", user.trim()));
    }
    let full_field = text(
        "fullName",
        "Full name",
        &full,
        "NixOS: the account's description and git's user.name. Not used in macOS guests.",
    )
    .enabled_if(locked.is_none(), lock_reason.clone());
    let mut email_field =
        text("email", "E-mail", &email, "NixOS: git's user.email; may stay empty. Not used in macOS guests.")
            .enabled_if(locked.is_none(), lock_reason);
    email_field.placeholder = Some(Text::new("none"));
    let valid = name_field.error.is_none() && (locked.is_some() || user_field.error.is_none());
    let error = name_field.error.clone().or_else(|| user_field.error.clone());
    // While images use it only the name is sent: the core refuses the rest then too
    let mut params = json!({"name": name.trim()});
    if locked.is_none() {
        params["user"] = json!(user.trim());
        params["fullName"] = json!(full.trim());
        params["email"] = json!(email.trim());
    }
    let save = match &p.id {
        Some(id) => {
            params["id"] = json!(id);
            Action::call("save", Text::new("Save"), Some("apply"), "credentials.update", params)
        }
        None => {
            let mut create = Action::call("create", Text::new("Create"), Some("create"), "credentials.add", params);
            // Every credential has a password: asked when it is created
            create.ask = vec![password_ask(name.trim())];
            create
        }
    }
    .then(Then::Back)
    .enabled_if(valid, error)
    .primary();
    Some(CredentialPage {
        title: Text::new(if p.id.is_some() { "Edit Credential" } else { "New Credential" }),
        id: p.id.clone(),
        intro: Text::new(
            "Who you are in a guest. NixOS guests take the user, full name and e-mail; macOS guests keep the base's user. The password – asked when you create it – is kept in the keychain for macOS builds and to copy whenever a guest asks.",
        ),
        fields: vec![name_field, user_field, full_field, email_field],
        locked,
        actions: vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({})), save],
    })
}

/// Images built with credential `id`
fn images_with(ctx: &Ctx, id: &str) -> usize {
    ctx.images.iter().filter(|i| i.credential.as_deref() == Some(id)).count()
}

/// The password ask of a new credential or a changed password: twice, with what it is for
fn password_ask(name: &str) -> Ask {
    let mut password = Ask::new("password", AskKind::Password, tf!("Password of %@", name));
    password.prompt = Some(Text::new(
        "The password of the guest's user – a macOS build needs it once (sudo in the build VM, the base's user); it never goes into an image. Kept in the keychain: copy it from here whenever a guest asks.",
    ));
    password.repeat = true;
    password
}

fn row(ctx: &Ctx, c: &CredentialDto) -> CredentialRow {
    let used_by: Vec<String> =
        ctx.images.iter().filter(|i| i.credential.as_deref() == Some(c.id.as_str())).map(|i| i.name.clone()).collect();
    let user = c.user.clone().unwrap_or_else(|| ctx.host_user.clone());
    let subtitle = Text::joined(vec![
        Text::verbatim(&user),
        match used_by.len() {
            0 => Text::new("no image"),
            1 => Text::new("1 image"),
            n => tf!("%lld images", n),
        },
    ]);
    let mut badges = Vec::new();
    if c.is_default {
        badges.push(
            Badge::new("default", "ok", Text::new("The default for new images (Settings)"), Tone::Accent)
                .label(Text::new("default")),
        );
    }
    if !c.has_password {
        badges.push(Badge::new(
            "no-password",
            "lock-warning",
            Text::new("No password stored – set one (Set Password …)"),
            Tone::Warning,
        ));
    }
    let id = json!({"id": c.id});
    // The quick action: the password to the clipboard – always, after Touch ID or the login password
    let copy = Action::call(
        "copyPassword",
        Text::new("Copy Password"),
        Some("clipboard"),
        "credentials.copyPassword",
        id.clone(),
    )
    .help(Text::new(ctx.on_host(
        "Copies the password to the clipboard after Touch ID or your login password; cleared after 90 s",
        "Copies the password to the clipboard; cleared after 90 s",
    )))
    .enabled_if(c.has_password, Some(Text::new("No password stored")));
    let edit = Action::show("edit", Text::new("Edit …"), None, "credential", id.clone()).primary();
    let mut set_password = Action::call(
        "password",
        Text::new(if c.has_password { "Change Password …" } else { "Set Password …" }),
        Some("lock"),
        "credentials.update",
        id.clone(),
    );
    set_password.ask = vec![password_ask(&c.name)];
    let default = Action::call(
        "default",
        Text::new("Make Default"),
        Some("ok"),
        "settings.set",
        json!({"defaultCredential": c.id}),
    )
    .enabled_if(!c.is_default, Some(Text::new("Already the default")));
    let remove = Action::call("remove", Text::new("Remove …"), Some("trash"), "credentials.remove", id)
        .destructive()
        .confirm(
            Confirm::new(
                tf!("Remove the credential “%@”?", &c.name),
                Text::new("The credential and its stored password are deleted."),
            )
            .button("Remove")
            .destructive(),
        )
        .enabled_if(used_by.is_empty(), Some(Text::new("Images are built with it – remove them first")));
    let used = used_by.join(", ");
    let none = Text::new("none");
    let info = vec![
        InfoLine::new("User", Text::verbatim(&user))
            .note(Text::new("NixOS: login, home folder, SSH · macOS: the base's user"), Tone::Normal),
        InfoLine::new("Full name", c.full_name.clone().map_or(none.clone(), Text::verbatim))
            .note(Text::new("NixOS: the account, git"), Tone::Normal),
        InfoLine::new("E-mail", c.email.clone().map_or(none.clone(), Text::verbatim))
            .note(Text::new("NixOS: git"), Tone::Normal),
        InfoLine::new("Password", Text::new(if c.has_password { "stored in the keychain" } else { "none – set one" }))
            .note(Text::new("macOS builds (sudo); copy it whenever a guest asks"), Tone::Normal),
        InfoLine::new("Used by", if used.is_empty() { Text::new("no image") } else { Text::verbatim(&used) })
            .full(&used),
    ];
    CredentialRow {
        id: c.id.clone(),
        name: c.name.clone(),
        lamp: if used_by.is_empty() {
            Lamp::new(LampState::Off, "No image uses this credential")
        } else {
            Lamp::new(LampState::On, "Images are built with it")
        },
        subtitle,
        badges,
        used_by,
        actions: vec![copy],
        detail: CredentialDetail { info, actions: vec![edit, set_password, default, remove] },
    }
}

pub(crate) fn page(ctx: &Ctx) -> CredentialsPage {
    let add = Action::show("add", Text::new("New Credential …"), Some("add"), "credential", json!({})).primary();
    CredentialsPage {
        title: Text::new("Credentials"),
        intro: tf!(
            "Who you are in a guest. NixOS guests take the user (login, home folder, SSH), the full name and the e-mail (the account, git); macOS guests keep the base's user and use only the stored password, for building. Every image is built with one credential – without one it gets this computer's: %@ (%@).",
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
