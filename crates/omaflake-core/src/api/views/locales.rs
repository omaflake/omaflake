// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Locales page (`locales`, plan B141) and the form of one (`locale`): where a guest lives – one language with
//! region, one time zone, one to three keyboards. Same shapes as the Credentials page and form, so every UI draws
//! them as it draws those. An image is built with one locale (the default unless the build chooses another).

use serde::Deserialize;
use serde_json::json;

use super::common::*;
use super::credentials::{CredentialDetail, CredentialPage, CredentialRow, CredentialsPage};
use super::text::{tf, Text};
use super::Ctx;
use crate::api::types::{LocaleDto, SettingsAnswer};

fn keyboards_text(l: &LocaleDto) -> String {
    l.keyboards.join(" + ")
}

fn row(l: &LocaleDto) -> CredentialRow {
    let used = l.images.len();
    let usage = match used {
        0 => Text::new("no image"),
        1 => Text::new("1 image"),
        n => tf!("%lld images", n),
    };
    let mut badges = Vec::new();
    if l.is_default {
        badges.push(
            Badge::new(
                "default",
                "ok",
                Text::new("New images get this locale unless the build chooses another"),
                Tone::Accent,
            )
            .label(Text::new("default")),
        );
    }
    let id = json!({"id": l.id});
    let edit = Action::show("edit", Text::new("Edit …"), None, "locale", id.clone()).primary();
    let default = Action::call("default", Text::new("Make Default"), Some("ok"), "locales.setDefault", id.clone())
        .enabled_if(!l.is_default, Some(Text::new("Already the default")));
    let remove = Action::call("remove", Text::new("Remove …"), Some("trash"), "locales.remove", id)
        .destructive()
        .confirm(
            Confirm::new(tf!("Remove the locale “%@”?", &l.name), Text::new("Images built with it keep its values."))
                .button("Remove")
                .destructive(),
        )
        .enabled_if(used == 0, Some(Text::new("Images are built with it – remove them first")));
    let used_by = l.images.join(", ");
    CredentialRow {
        id: l.id.clone(),
        name: l.name.clone(),
        lamp: if used == 0 {
            Lamp::new(LampState::Off, "No image is built with this locale")
        } else {
            Lamp::new(LampState::On, "Images are built with it")
        },
        subtitle: Text::joined(vec![
            Text::verbatim(l.locale.replace(".UTF-8", "")),
            Text::verbatim(keyboards_text(l)),
            Text::verbatim(l.time_zone.replace('_', " ")),
            usage,
        ]),
        badges,
        used_by: l.images.clone(),
        actions: if l.is_default { vec![] } else { vec![default.clone()] },
        detail: CredentialDetail {
            info: vec![
                InfoLine::new("Language and region", Text::verbatim(l.locale.replace(".UTF-8", ""))),
                InfoLine::new("Time zone", Text::verbatim(l.time_zone.replace('_', " "))),
                InfoLine::new("Keyboards", Text::verbatim(keyboards_text(l)))
                    .note(Text::new("the first is the default; Super+Space switches (NixOS)"), Tone::Normal),
                InfoLine::new(
                    "Used by",
                    if used_by.is_empty() { Text::new("no image") } else { Text::verbatim(&used_by) },
                )
                .full(&used_by),
            ],
            actions: vec![edit, default, remove],
        },
    }
}

pub(crate) fn page(ctx: &Ctx) -> CredentialsPage {
    CredentialsPage {
        title: Text::new("Locales"),
        intro: tf!(
            "Where a guest lives: its language and region, its time zone and its keyboards – several if you write in more than one (Super+Space switches on NixOS). Every image is built with one, the default unless the build page chooses another; without any, an image gets this computer's values (keyboard %@).",
            ctx.settings.host_keyboard.clone().unwrap_or_else(|| "–".into())
        ),
        rows: ctx.locales.iter().map(row).collect(),
        add: Action::show("add", Text::new("New Locale …"), Some("add"), "locale", json!({})).primary(),
        footer: Text::new(
            "Locales stay on this computer – they are never in a catalog. A recipe may say which languages it works with; a locale outside gets a warning at the build. This computer's own keyboard is in the Settings.",
        ),
    }
}

/// The form's values (kept by the UI while the user chooses)
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LocaleValues {
    pub name: Option<String>,
    pub locale: Option<String>,
    pub time_zone: Option<String>,
    pub keyboard: Option<String>,
    pub keyboard2: Option<String>,
    pub keyboard3: Option<String>,
}

#[derive(Debug, Default, Deserialize)]
pub struct LocaleParams {
    #[serde(default)]
    pub id: Option<String>,
    #[serde(default)]
    pub values: LocaleValues,
}

/// `locale`: a new locale (preset with this computer's values) or one to edit – only its name while images use it
pub(crate) fn locale_page(ctx: &Ctx, answer: &SettingsAnswer, p: &LocaleParams) -> Option<CredentialPage> {
    let l = match &p.id {
        Some(id) => Some(ctx.locales.iter().find(|l| &l.id == id)?),
        None => None,
    };
    let d = &answer.guest_defaults;
    let v = &p.values;
    let host_keyboard =
        ctx.settings.host_keyboard.clone().or_else(|| d.keyboard.clone()).unwrap_or_else(|| "us".into());
    let name = v.name.clone().unwrap_or_else(|| l.map(|l| l.name.clone()).unwrap_or_default());
    let locale = v
        .locale
        .clone()
        .unwrap_or_else(|| l.map(|l| l.locale.clone()).or_else(|| d.locale.clone()).unwrap_or_default());
    let time_zone = v
        .time_zone
        .clone()
        .unwrap_or_else(|| l.map(|l| l.time_zone.clone()).or_else(|| d.time_zone.clone()).unwrap_or_default());
    let current = |i: usize| l.and_then(|l| l.keyboards.get(i).cloned());
    let keyboard = v.keyboard.clone().unwrap_or_else(|| current(0).unwrap_or(host_keyboard));
    let keyboard2 = v.keyboard2.clone().unwrap_or_else(|| current(1).unwrap_or_default());
    let keyboard3 = v.keyboard3.clone().unwrap_or_else(|| current(2).unwrap_or_default());
    let used = l.map_or(0, |l| l.images.len());
    let locked = (used > 0).then(|| {
        Text::new("Images are built with it, so only its name can change – they carry its language, time zone and keyboards. For other values create a new locale.")
    });
    let lock_reason = locked.as_ref().map(|_| Text::new("In use – only the name can change"));
    let c = &answer.choices;
    let choice = |id: &str, label: &str, value: &str, options: Vec<Choice>, help: &str| {
        let mut f = Field::new(id, FieldKind::Choice, Text::new(label), json!(value))
            .param(id)
            .value_param(id)
            .help(Text::new(help))
            .enabled_if(locked.is_none(), lock_reason.clone());
        f.choices = options;
        f
    };
    let mut keyboards: Vec<Choice> = c
        .keyboards
        .iter()
        .map(|k| Choice::new(json!(k.code), Text::verbatim(format!("{} ({})", k.code, k.locale))))
        .collect();
    keyboards.sort_by(|a, b| a.label.text.cmp(&b.label.text));
    let mut more = vec![Choice::new(json!(""), Text::new("none"))];
    more.extend(keyboards.clone());
    let mut name_field = Field::new("name", FieldKind::Text, Text::new("Name"), json!(name))
        .param("name")
        .value_param("name")
        .help(Text::new("Only for you – to tell your locales apart when building."));
    let taken = ctx.locales.iter().any(|x| Some(&x.id) != p.id.as_ref() && x.name.eq_ignore_ascii_case(name.trim()));
    if name.trim().is_empty() {
        name_field.error = Some(Text::new("A locale needs a name"));
    } else if taken {
        name_field.error = Some(tf!("There is a locale \"%@\" already", name.trim()));
    }
    let chosen: Vec<String> =
        [&keyboard, &keyboard2, &keyboard3].into_iter().filter(|k| !k.is_empty()).cloned().collect();
    let mut keyboard3_field = choice("keyboard3", "Third keyboard", &keyboard3, more.clone(), "Optional");
    let twice = chosen.iter().enumerate().any(|(i, k)| chosen[..i].contains(k));
    if twice {
        keyboard3_field.error = Some(Text::new("A keyboard is chosen twice"));
    }
    let fields = vec![
        name_field.clone(),
        choice(
            "locale",
            "Language and region",
            &locale,
            super::settings::choices(&c.locales, |l| l.replace(".UTF-8", "")),
            "The guest's system language, dates and numbers – one per locale.",
        ),
        choice(
            "timeZone",
            "Time zone",
            &time_zone,
            super::settings::choices(&c.time_zones, |z| z.replace('_', " ")),
            "The guest's clock.",
        ),
        choice("keyboard", "Keyboard", &keyboard, keyboards, "The guest's first keyboard – the default."),
        choice(
            "keyboard2",
            "Second keyboard",
            &keyboard2,
            more,
            "Optional – Super+Space switches (NixOS); macOS shows it in the menu bar.",
        ),
        keyboard3_field.clone(),
    ];
    let error = name_field.error.clone().or(keyboard3_field.error.clone());
    let mut params = json!({"name": name.trim()});
    if locked.is_none() {
        params["locale"] = json!(locale);
        params["timeZone"] = json!(time_zone);
        params["keyboards"] = json!(chosen);
    }
    let save = match &p.id {
        Some(id) => {
            params["id"] = json!(id);
            Action::call("save", Text::new("Save"), Some("apply"), "locales.update", params)
        }
        None => Action::call("create", Text::new("Create"), Some("create"), "locales.add", params),
    }
    .then(Then::Back)
    .enabled_if(error.is_none(), error)
    .primary();
    Some(CredentialPage {
        title: Text::new(if p.id.is_some() { "Edit Locale" } else { "New Locale" }),
        id: p.id.clone(),
        intro: Text::new(
            "Where a guest lives: one language and region, one time zone, one to three keyboards. Images are built with it; changing it later needs a new build.",
        ),
        fields,
        locked,
        actions: vec![Action::call("cancel", Text::new("Cancel"), None, "ui.back", json!({})), save],
    })
}
