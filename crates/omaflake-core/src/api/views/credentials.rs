// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Credentials page (`credentials`, plan B137): who you are in a guest – named sets of user, full name, e-mail and
//! an optional stored password (for macOS builds); an image is built with one of them, the settings name the default.
//! The form of one (`credential`): a new one, or one to edit – only its name while images use it. The same form
//! assigns a credential to a macOS base created before bases kept theirs (`baseCredential`, plan B139).

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
    // Only the first credential starts from this computer's user and name; after that the host has nothing to do
    // with credentials (plan B139)
    let first = c.is_none() && ctx.credentials.is_empty();
    let host = |v: &str| if first { v.to_string() } else { String::new() };
    let user = v.user.clone().unwrap_or_else(|| match c {
        Some(c) => c.user.clone().unwrap_or_default(),
        None => host(&ctx.host_user),
    });
    let full = v.full_name.clone().unwrap_or_else(|| match c {
        Some(c) => c.full_name.clone().unwrap_or_default(),
        None => host(&ctx.host_full_name),
    });
    let email = v.email.clone().unwrap_or_else(|| c.and_then(|c| c.email.clone()).unwrap_or_default());
    let used = c.map_or(0, |c| used_by(ctx, &c.id).len());
    let locked = (used > 0).then(|| {
        Text::new(
            "Images or a macOS base use it, so only its name can change – they carry its user, full name and e-mail, a base also its password. For other values create a new credential.",
        )
    });
    let lock_reason = locked.as_ref().map(|_| Text::new("In use – only the name can change"));
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
        "NixOS: the login user – the automatic login, the home folder /home/<user> and omaf ssh. macOS: the user of a base created with it.",
    )
    .enabled_if(locked.is_none(), lock_reason.clone());
    if user.trim().is_empty() {
        user_field.error = Some(Text::new("A credential needs a user"));
    } else if !valid_user(user.trim()) {
        user_field.error =
            Some(tf!("Invalid user name \"%@\": a–z, 0–9, \"_\" and \"-\", starting with a letter", user.trim()));
    }
    let full_field = text(
        "fullName",
        "Full name",
        &full,
        "NixOS: the account's description and git's user.name. macOS: the account's name in a base created with it.",
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
            "Who you are in a guest. NixOS images take the user, full name and e-mail; a macOS base is created with the user, full name and password. The password – asked once, here – is kept in the keychain: omaflake uses it for macOS bases and builds, and you can copy it whenever a guest asks.",
        ),
        fields: vec![name_field, user_field, full_field, email_field],
        locked,
        actions: vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({})), save],
    })
}

/// The images built with credential `id`, then the macOS bases created from it (plan B139)
fn used_by(ctx: &Ctx, id: &str) -> Vec<String> {
    let images = ctx.images.iter().filter(|i| i.credential.as_deref() == Some(id)).map(|i| i.name.clone());
    let bases = ctx.bases.iter().filter(|b| b.credential.as_deref() == Some(id)).map(|b| b.name.clone());
    images.chain(bases).collect()
}

/// The password ask of a new credential or a changed password: twice, with what it is for
fn password_ask(name: &str) -> Ask {
    let mut password = Ask::new("password", AskKind::Password, tf!("Password of %@", name));
    password.prompt = Some(Text::new(
        "The password of the guest's user, kept in the keychain: omaflake creates macOS bases with it and builds macOS images with it (sudo), so you never type it there; it never goes into an image. Copy it from here whenever a guest asks.",
    ));
    password.repeat = true;
    password
}

fn row(ctx: &Ctx, c: &CredentialDto) -> CredentialRow {
    let used_by = used_by(ctx, &c.id);
    let images = ctx.images.iter().filter(|i| i.credential.as_deref() == Some(c.id.as_str())).count();
    let bases = ctx.bases.iter().filter(|b| b.credential.as_deref() == Some(c.id.as_str())).count();
    let user = c.user.clone().unwrap_or_else(|| "–".into());
    let mut parts = vec![
        Text::verbatim(&user),
        match images {
            0 => Text::new("no image"),
            1 => Text::new("1 image"),
            n => tf!("%lld images", n),
        },
    ];
    match bases {
        0 => {}
        1 => parts.push(Text::new("1 macOS base")),
        n => parts.push(tf!("%lld macOS bases", n)),
    }
    let subtitle = Text::joined(parts);
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
    // A base's account keeps the password it was created with
    let set_password = set_password.enabled_if(
        bases == 0,
        Some(Text::new("A macOS base's user has this password – it stays as long as the base is there")),
    );
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
        .enabled_if(bases == 0, Some(Text::new("A macOS base was created from it – remove the base first")))
        .enabled_if(images == 0, Some(Text::new("Images are built with it – remove them first")));
    let used = used_by.join(", ");
    let none = Text::new("none");
    let info = vec![
        InfoLine::new("User", Text::verbatim(&user))
            .note(Text::new("NixOS: login, home folder, SSH · macOS: the user of its bases"), Tone::Normal),
        InfoLine::new("Full name", c.full_name.clone().map_or(none.clone(), Text::verbatim))
            .note(Text::new("NixOS: the account, git"), Tone::Normal),
        InfoLine::new("E-mail", c.email.clone().map_or(none.clone(), Text::verbatim))
            .note(Text::new("NixOS: git"), Tone::Normal),
        InfoLine::new("Password", Text::new(if c.has_password { "stored in the keychain" } else { "none – set one" }))
            .note(Text::new("macOS bases and builds; copy it whenever a guest asks"), Tone::Normal),
        InfoLine::new("Used by", if used.is_empty() { Text::new("no image") } else { Text::verbatim(&used) })
            .full(&used),
    ];
    CredentialRow {
        id: c.id.clone(),
        name: c.name.clone(),
        lamp: if used_by.is_empty() {
            Lamp::new(LampState::Off, "No image or macOS base uses this credential")
        } else {
            Lamp::new(LampState::On, "Images or a macOS base use it")
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
        intro: Text::new(
            "Who you are in a guest. NixOS images take the user (login, home folder, SSH), the full name and the e-mail (the account, git); a macOS base is created with the user, full name and password, and its images are built with them. The password is asked once, when you create the credential – after that omaflake uses it by itself. Every image and every macOS base is made with a credential; the default one unless you choose another.",
        ),
        rows: ctx.credentials.iter().map(|c| row(ctx, c)).collect(),
        add,
        footer: Text::new(ctx.on_host(
            "Passwords are kept in the login keychain. The default credential for new images is chosen in the Settings; a changed credential makes its images outdated.",
            "Passwords are kept in the desktop's keyring (Secret Service), else unencrypted in ~/.config/omaflake/tokens.json. The default credential for new images is chosen in the Settings; a changed credential makes its images outdated.",
        )),
    }
}

/// A choice of the credentials ("Work – alex"), for what needs a password: a macOS base (plan B139)
pub(crate) fn credential_choice(ctx: &Ctx, value: &str, help: &str) -> Field {
    let mut f = Field::new("credential", FieldKind::Choice, Text::new("Credential"), json!(value))
        .param("credential")
        .value_param("credential");
    f.choices = ctx
        .credentials
        .iter()
        .map(|x| Choice::new(json!(x.id), Text::verbatim(format!("{} – {}", x.name, x.user.as_deref().unwrap_or("–")))))
        .collect();
    f.help = Some(Text::new(help));
    f
}

/// "New Credential …" where a credential is needed and there is none
pub(crate) fn new_credential() -> Action {
    Action::show("newCredential", Text::new("New Credential …"), Some("add"), "credential", json!({}))
}

#[derive(Debug, Default, Deserialize)]
pub struct BaseCredentialParams {
    pub name: String,
    #[serde(default)]
    pub values: BaseCredentialValues,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct BaseCredentialValues {
    pub credential: Option<String>,
}

/// `baseCredential`: the credential of a macOS base created before bases kept theirs (plan B139) – the one with its
/// user and password; macOS images are built with it, without asking for a password
pub(crate) fn base_credential_page(ctx: &Ctx, p: &BaseCredentialParams) -> Option<CredentialPage> {
    let b = ctx.bases.iter().find(|b| b.name == p.name)?;
    let user_of = |c: &CredentialDto| c.user.clone().unwrap_or_default();
    let chosen = p
        .values
        .credential
        .clone()
        .filter(|id| ctx.credentials.iter().any(|c| &c.id == id))
        .or_else(|| b.credential.clone())
        .or_else(|| ctx.credentials.iter().find(|c| Some(user_of(c)) == b.user).map(|c| c.id.clone()))
        .or_else(|| ctx.settings.default_credential.clone())
        .or_else(|| ctx.credentials.first().map(|c| c.id.clone()))
        .unwrap_or_default();
    let mut field = credential_choice(
        ctx,
        &chosen,
        "The credential with the base's user and its password – the same user and password it was created with.",
    );
    let c = ctx.credentials.iter().find(|c| c.id == chosen);
    if let (Some(c), Some(base_user)) = (c, &b.user) {
        if &user_of(c) != base_user {
            field.error = Some(tf!("The base's user is %@ – this credential has %@", base_user, user_of(c)));
        }
    }
    let error =
        if ctx.credentials.is_empty() { Some(Text::new("Create a credential first")) } else { field.error.clone() };
    let save = Action::call(
        "save",
        Text::new("Use Credential"),
        Some("apply"),
        "bases.assign",
        json!({"name": b.name, "credential": chosen}),
    )
    .then(Then::Back)
    .enabled_if(error.is_none(), error)
    .primary();
    let mut actions = vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({}))];
    if ctx.credentials.is_empty() {
        actions.push(new_credential());
    }
    actions.push(save);
    Some(CredentialPage {
        title: tf!("Credential of %@", &b.name),
        id: None,
        intro: tf!(
            "%@ was created before bases kept their credential. Choose the one its user %@ was created with – same user, same password. macOS images are then built with it, without asking for a password; a wrong password shows at the next build.",
            &b.name,
            b.user.clone().unwrap_or_else(|| "?".into())
        ),
        fields: vec![field],
        locked: None,
        actions,
    })
}
