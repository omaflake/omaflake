// SPDX-License-Identifier: AGPL-3.0-or-later
//! `welcome`: the first start's few questions (plan B144) – what to run, the recipes and looks to start with, who you
//! are in a guest, developer mode – and one button that does it all (`setup.welcome`): your credential, then the jobs
//! for what you chose (Set Up for NixOS, the suggested recipes and looks, the macOS base). The page has a credential
//! form's shape, so every interface draws it as one; the app opens it at its first start, the Doctor offers it.

use serde_json::json;

use super::common::*;
use super::credentials::{CredentialPage, valid_user};
use super::text::{Text, tf};
use super::{Ctx, HostOs};

// The page's model (plan B192: in omaflake-sdk); this file builds it
pub use omaflake_sdk::views::welcome::*;

pub(crate) fn page(ctx: &Ctx, p: &WelcomeParams) -> CredentialPage {
    let v = &p.values;
    let mac = ctx.host == HostOs::Macos;
    // On a Linux computer NixOS guests are all there is (plan B124)
    let guests = if mac { v.guests.clone().unwrap_or_else(|| "nixos".into()) } else { "nixos".into() };
    let recommended = v.recommended.unwrap_or(true);
    let lab = v.lab.unwrap_or(ctx.settings.developer_mode);
    // Who you are – asked only while there is no credential yet; the first starts from this computer's user
    let first = ctx.credentials.is_empty();
    let user = v.user.clone().unwrap_or_else(|| if first { ctx.host_user.clone() } else { String::new() });
    let full = v.full_name.clone().unwrap_or_else(|| if first { ctx.host_full_name.clone() } else { String::new() });

    let mut fields = Vec::new();
    if mac {
        let mut f = Field::new("guests", FieldKind::Choice, Text::new("What do you want to run?"), json!(guests))
            .param("guests")
            .value_param("guests")
            .help(Text::new(
                "NixOS VMs need Nix and a small Linux VM that builds their images; macOS VMs need a macOS base – about 30 minutes and 30 GB once.",
            ));
        f.choices = vec![
            Choice::new(json!("nixos"), Text::new("NixOS VMs")),
            Choice::new(json!("macos"), Text::new("macOS VMs")),
            Choice::new(json!("both"), Text::new("NixOS and macOS VMs")),
        ];
        fields.push(f);
    }
    fields.push(
        Field::new("recommended", FieldKind::Toggle, Text::new("Start with omaflake's recipes and looks"), json!(recommended))
            .param("recommended")
            .value_param("recommended")
            .help(Text::new(
                "Installs the recipes the official catalog suggests for what you run, and omaflake's looks. Without it you pick them yourself in Recipes and Looks.",
            )),
    );
    let mut error = None;
    if first {
        let mut user_field = Field::new("user", FieldKind::Text, Text::new("User"), json!(user))
            .param("user")
            .value_param("user")
            .help(Text::new("Your user in the guests: the login, the home folder, omaf ssh."));
        if user.trim().is_empty() {
            user_field.error = Some(Text::new("A credential needs a user"));
        } else if !valid_user(user.trim()) {
            user_field.error =
                Some(tf!("Invalid user name \"%@\": a–z, 0–9, \"_\" and \"-\", starting with a letter", user.trim()));
        }
        error = user_field.error.clone();
        fields.push(user_field);
        fields.push(
            Field::new("fullName", FieldKind::Text, Text::new("Full name"), json!(full))
                .param("fullName")
                .value_param("fullName")
                .help(Text::new("Your name in the guests (the account, git)")),
        );
    }
    fields.push(
        Field::new("lab", FieldKind::Toggle, Text::new("Developer mode"), json!(lab))
            .param("lab")
            .value_param("lab")
            .help(Text::new(
                "Shows the lab recipes and the tools for making your own – you can switch it later in Settings",
            )),
    );

    let mut params = json!({"guests": guests, "recommended": recommended, "lab": lab});
    if first {
        params["user"] = json!(user.trim());
        params["fullName"] = json!(full.trim());
    }
    let mut set_up = Action::call("setUp", Text::new("Set Up"), Some("install"), "setup.welcome", params);
    if first {
        // Every credential has a password: asked here, kept in the keychain
        let mut password = Ask::new("password", AskKind::Password, Text::new("Your password in the guests"));
        password.prompt = Some(Text::new(
            "The password of your user in the guests, kept in the keychain: omaflake creates macOS bases and builds macOS images with it, so you never type it there. Copy it from Credentials whenever a guest asks.",
        ));
        password.repeat = true;
        set_up.ask = vec![password];
    }
    let set_up = set_up.then(Then::Back).enabled_if(error.is_none(), error).primary();
    CredentialPage {
        title: Text::new("Welcome to omaflake"),
        id: None,
        intro: Text::new(
            "A few questions, then omaflake sets up what you need – it shows each step in the Doctor. Everything here can be changed later.",
        ),
        fields,
        locked: None,
        actions: vec![Action::call("later", Text::new("Later"), None, "ui.back", json!({})), set_up],
    }
}
