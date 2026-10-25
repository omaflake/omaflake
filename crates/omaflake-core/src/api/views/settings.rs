// SPDX-License-Identifier: AGPL-3.0-or-later
//! Settings (`settings`): the recipe sources (spec §11.3, plan B107), the general switches, the guest values
//! (spec §11.1) and the data folder – the app's `SettingsView` and `RecipeSourcesSection`. What only one UI has (launch
//! at login, the UI's language) stays in that UI.

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

use super::common::*;
use super::text::{tf, Arg, Text};
use super::Ctx;
use crate::api::types::{GuestValues, Look, SettingsAnswer, SourceDto};
use crate::recipe_sources::RecipeSourceKind;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsPage {
    /// "Settings"
    pub title: Text,
    pub sources: SourcesSection,
    pub general: SettingsSection,
    pub guest: SettingsSection,
    pub data_folder: DataFolderSection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsSection {
    pub title: Text,
    pub fields: Vec<Field>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub footer: Option<Text>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourcesSection {
    pub title: Text,
    /// In their order: an earlier source wins when two have the same recipe
    pub rows: Vec<SourceRow>,
    /// A new source: its fields and "Check and Add"
    pub add: SettingsSection,
    pub add_action: Action,
    pub footer: Text,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceRow {
    pub id: String,
    pub name: String,
    /// folder, github, url
    pub icon: String,
    /// Official, private (with or without its token)
    pub badges: Vec<Badge>,
    pub subtitle: Text,
    /// Earlier, the on/off switch, remove
    pub actions: Vec<Action>,
    pub source: SourceDto,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataFolderSection {
    pub title: Text,
    pub location: String,
    pub is_default: bool,
    pub moving: bool,
    /// Default (~/.omaflake), Move …
    pub actions: Vec<Action>,
    pub note: Text,
}

/// The new source's values as typed so far (never a token)
#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsParams {
    #[serde(default)]
    pub values: SourceValues,
}

#[derive(Debug, Default, Deserialize)]
pub struct SourceValues {
    #[serde(default)]
    pub input: String,
    #[serde(default)]
    pub private: bool,
}

/// The input names a GitHub repository ("owner/repo", a github.com URL) – only those can be private
pub fn looks_like_github(input: &str) -> bool {
    let t = input.trim();
    if t.contains("github.com/") {
        return true;
    }
    if t.starts_with('/') || t.starts_with('~') || t.contains("://") {
        return false;
    }
    t.split('/').filter(|s| !s.is_empty()).count() == 2
}

fn source_row(ctx: &Ctx, s: &SourceDto, first: bool) -> SourceRow {
    let mut badges = Vec::new();
    if s.official {
        badges.push(Badge::new("official", "official", Text::new("Official recipes"), Tone::Accent));
    }
    if s.private {
        badges.push(if s.has_token {
            let help =
                ctx.on_host("Private – token in the Keychain", "Private – token stored (keyring or tokens.json)");
            Badge::new("private", "lock", Text::new(help), Tone::Normal)
        } else {
            let help = ctx.on_host("Private – no token in the Keychain", "Private – no token stored");
            Badge::new("private", "lock-warning", Text::new(help), Tone::Warning)
        });
    }
    let (icon, subtitle) = match s.kind {
        RecipeSourceKind::Folder => ("folder", "used as it is – changes count at once"),
        RecipeSourceKind::Github => ("github", "GitHub"),
        RecipeSourceKind::Url => ("url", "HTTPS"),
    };
    SourceRow {
        id: s.id.clone(),
        name: s.display_name.clone(),
        icon: icon.into(),
        badges,
        subtitle: Text::new(subtitle),
        actions: vec![
            Action::call("up", Text::new("Earlier"), Some("up"), "sources.move", json!({"id": s.id, "up": true}))
                .help(Text::new("Earlier: wins when two sources have the same recipe"))
                .enabled_if(!first, None),
            Action::call(
                "enabled",
                Text::new("Enabled"),
                None,
                "sources.setEnabled",
                json!({"id": s.id, "enabled": !s.enabled}),
            )
            .switch(s.enabled),
            Action::call(
                "remove",
                Text::new("Remove this source"),
                Some("trash"),
                "sources.remove",
                json!({"id": s.id}),
            )
            .destructive()
            .confirm(
                Confirm::new(
                    tf!("Remove the source “%@”?", &s.display_name),
                    Text::new(ctx.on_host(
                        "Recipes installed from it stay until you remove them; its token goes from the Keychain.",
                        "Recipes installed from it stay until you remove them; its token goes from the keyring or tokens.json.",
                    )),
                )
                .button("Remove")
                .destructive(),
            ),
        ],
        source: s.clone(),
    }
}

/// A guest value from a list: "Like this Mac – <value>" (null) or one of the choices
fn guest_choice(
    ctx: &Ctx,
    id: &str,
    label: &str,
    value: &Option<String>,
    mac: &Option<String>,
    options: Vec<Choice>,
    base: &Value,
) -> Field {
    let mut f = Field::new(id, FieldKind::Choice, Text::new(label), json!(value))
        .settings(&format!("guest.{id}"), base.clone());
    let like = ctx.on_host("Like this Mac – %@", "Like this computer – %@");
    f.choices = vec![Choice::new(Value::Null, tf!(like, mac.clone().unwrap_or_default()))];
    f.choices.extend(options);
    f
}

/// The guests' look (plan B135): "Each recipe's own" (null) or one of the looks – there is no computer's look
fn look_choice(g: &GuestValues, looks: &[Look], base: &Value) -> Field {
    let mut f =
        Field::new("look", FieldKind::Choice, Text::new("Look"), json!(g.look)).settings("guest.look", base.clone());
    f.choices = vec![Choice::new(Value::Null, Text::new("Each recipe's own"))];
    f.choices.extend(looks.iter().map(|l| Choice::new(json!(l.name), Text::verbatim(l.title.clone()))));
    f
}

pub(crate) fn page(ctx: &Ctx, answer: &SettingsAnswer, p: &SettingsParams) -> SettingsPage {
    let s = &answer.settings;
    let rows = ctx.sources.iter().enumerate().map(|(i, s)| source_row(ctx, s, i == 0)).collect();
    let github = looks_like_github(&p.values.input);
    let private = github && p.values.private;
    let mut add_fields = vec![{
        let mut f = Field::new(
            "input",
            FieldKind::Text,
            Text::new("GitHub repository, HTTPS URL or folder"),
            json!(p.values.input),
        )
        .param("input")
        .value_param("input");
        f.ask = Some(
            Ask::new("input", AskKind::Folder, Text::new("Folder …"))
                .prompt(Text::new("Folder with recipes (recipes/<author>/<kind>/<name>/)"))
                .button("Use"),
        );
        f
    }];
    if github {
        add_fields.push(
            Field::new("private", FieldKind::Toggle, Text::new("Private repository"), json!(p.values.private))
                .param("private")
                .value_param("private"),
        );
    }
    let mut add = Action::call(
        "add",
        Text::new("Check and Add"),
        Some("add"),
        "sources.add",
        json!({"input": p.values.input, "private": private}),
    )
    .enabled_if(!p.values.input.trim().is_empty(), None)
    .primary();
    if private {
        let mut token = Ask::new(
            "token",
            AskKind::Password,
            Text::new("Fine-grained token (Contents: Read-only on this repository)"),
        );
        token.prompt = Some(Text::new(ctx.on_host(
            "The token is checked against the repository and kept in your Keychain – never in a file.",
            "The token is checked against the repository and kept in your keyring – without one (a server, an SSH session) unencrypted in a file only you can read.",
        )));
        add = add.ask(token);
    }

    let flag = |id: &str, label: &str, help: &str, value: bool| {
        Field::new(id, FieldKind::Toggle, Text::new(label), json!(value)).help(Text::new(help)).settings(id, json!({}))
    };
    let general = SettingsSection {
        title: Text::new("General"),
        fields: vec![
            flag("notifications", "Notifications", ctx.on_host(
                "Only while omaflake is not in front: VM ready, image built or failed, VM ended. macOS asks once whether omaflake may send them.",
                "Only while omaflake is not in front: VM ready, image built or failed, VM ended.",
            ), s.notifications),
            flag("checkForUpdates", "Check for updates", "Once a day – the only network access omaflake makes on its own.", s.check_for_updates),
            flag("keepRecipeVersions", "Keep older recipe versions", "When a newer version of a recipe comes (an update, an import, a change in a local folder after a build). Off: only the newest stays – and every version an image is built from, as long as the image is there.", s.keep_recipe_versions),
            flag("developerMode", "Developer Mode", "Shows the lab recipes – omaflake's development recipes with test app and debug tools, and recipes others mark with lab = true.", s.developer_mode),
        ],
        footer: None,
    };

    let g = &s.guest;
    let d = &answer.guest_defaults;
    // settings.set replaces the whole guest object: each field sends the others as they are
    let base = json!({"guest": g});
    let text = |id: &str, label: &str, value: &Option<String>, placeholder: &Option<String>| {
        let mut f = Field::new(id, FieldKind::Text, Text::new(label), json!(value))
            .settings(&format!("guest.{id}"), base.clone());
        f.placeholder = Some(Text::verbatim(placeholder.clone().unwrap_or_default()));
        f
    };
    let c = &answer.choices;
    let mut keyboards: Vec<Choice> = c
        .keyboards
        .iter()
        .map(|k| Choice::new(json!(k.code), Text::verbatim(format!("{} ({})", k.code, k.locale))))
        .collect();
    keyboards.sort_by(|a, b| a.label.text.cmp(&b.label.text));
    let guest = SettingsSection {
        title: Text::new("Guest user in the VMs"),
        fields: vec![
            text("user", "User", &g.user, &d.user),
            text("fullName", "Full name", &g.full_name, &d.full_name),
            text("email", "E-mail (git)", &g.email, &None),
            guest_choice(ctx, "locale", "Language and region", &g.locale, &d.locale, choices(&c.locales, |l| l.replace(".UTF-8", "")), &base),
            guest_choice(ctx, "keyboard", "Keyboard", &g.keyboard, &d.keyboard, keyboards, &base),
            guest_choice(ctx, "timeZone", "Time zone", &g.time_zone, &d.time_zone, choices(&c.time_zones, |z| z.replace('_', " ")), &base),
            look_choice(g, &c.looks, &base),
        ],
        footer: Some(Text::new(ctx.on_host(
            "Empty fields and \"Like this Mac\" use your Mac's values. The lists offer only what both NixOS and macOS VMs support. Your SSH keys from ~/.ssh are added automatically. macOS VMs always use the user of the macOS base. The look colours a NixOS VM's whole desktop and a macOS VM's terminal and editors; a changed look comes with the next build of an image. A developer flake's local/settings.nix overrides these values.",
            "Empty fields and \"Like this computer\" use this computer's values. The lists offer only what both NixOS and macOS VMs support. Your SSH keys from ~/.ssh are added automatically. The look colours a NixOS VM's whole desktop and a macOS VM's terminal and editors; a changed look comes with the next build of an image. A developer flake's local/settings.nix overrides these values.",
        ))),
    };

    let moving = ctx.active_job(crate::api::types::JobKind::DataMove).is_some();
    let running = ctx.vms.iter().any(|v| v.status == crate::models::VmStatus::Running);
    let stop_first = Some(Text::new("Stop all VMs before moving the data folder."));
    let move_text = Text::new(
        "Images, instances and bases are moved there (renamed, not copied). The settings stay in ~/.omaflake.",
    );
    let data_folder = DataFolderSection {
        title: Text::new("Data Folder"),
        location: s.data_folder.clone(),
        is_default: answer.is_default_data_folder,
        moving,
        actions: vec![
            Action::call("default", Text::new("Default (~/.omaflake)"), None, "dataFolder.move", json!({"target": ctx.default_data_folder}))
                .confirm(Confirm::new(tf!("Move the data folder to %@?", &ctx.default_data_folder), move_text.clone()))
                .busy(moving)
                .enabled_if(!moving && !answer.is_default_data_folder, None)
                .enabled_if(!running, stop_first.clone()),
            Action::call("move", Text::new("Move …"), Some("folder"), "dataFolder.move", json!({}))
                .ask(
                    Ask::new("target", AskKind::Folder, Text::new("Move …"))
                        .prompt(Text::new("Empty folder for omaflake's images, instances and bases"))
                        .button("Use as Data Folder"),
                )
                .confirm(Confirm::new(Text::fmt("Move the data folder to %@?", vec![Arg::param("target")]), move_text))
                .enabled_if(!moving, None)
                .enabled_if(!running, stop_first),
        ],
        note: Text::new(ctx.on_host(
            "By default omaflake keeps everything in ~/.omaflake and needs no access to Documents or Desktop. A folder there asks for access once.",
            "By default omaflake keeps everything in ~/.omaflake.",
        )),
    };
    SettingsPage {
        title: Text::new("Settings"),
        sources: SourcesSection {
            title: Text::new("Recipe Sources"),
            rows,
            add: SettingsSection { title: Text::new("Recipe Sources"), fields: add_fields, footer: None },
            add_action: add,
            footer: Text::new(ctx.on_host(
                "omaflake ships no recipes; it reads them from these sources. Recipes from other sources than omaflake/recipes are not reviewed by the omaflake team – their Nix code runs on this Mac when you build them.",
                "omaflake ships no recipes; it reads them from these sources. Recipes from other sources than omaflake/recipes are not reviewed by the omaflake team – their Nix code runs on this computer when you build them.",
            )),
        },
        general,
        guest,
        data_folder,
    }
}

/// Choices with plain names (UIs may name them in their language: Foundation's locale names in the app)
fn choices(values: &[String], name: impl Fn(&str) -> String) -> Vec<Choice> {
    let mut c: Vec<Choice> = values.iter().map(|v| Choice::new(json!(v), Text::verbatim(name(v)))).collect();
    c.sort_by_key(|c| c.label.text.to_lowercase());
    c
}
