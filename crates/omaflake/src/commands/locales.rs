// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf locales` (plan B141): where a guest lives – language and region, time zone, one to three keyboards. Created
//! like credentials, kept in config.json; an image is built with one (`omaf build --locale`), the default otherwise.

use omaflake_core::guest_settings::GuestSettings;
use omaflake_core::locales::{GuestLocale, LocaleChange, Locales};
use omaflake_core::{GuestChoices, Settings};

use serde_json::json;

use super::service;
use crate::cli;
use crate::util::{Env, Outcome, pad, usage};

fn keyboards(text: &str) -> Result<Vec<String>, crate::util::Failure> {
    let list: Vec<String> = text.split(',').map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
    for k in &list {
        if !GuestChoices::KEYBOARDS.iter().any(|(c, _)| c == k) {
            let codes: Vec<&str> = GuestChoices::KEYBOARDS.iter().map(|(c, _)| *c).collect();
            return Err(usage(&["locales"], format!("--keyboards: {k} is none of {}", codes.join(", "))));
        }
    }
    Ok(list)
}

pub fn list() -> Outcome {
    let env = Env::new();
    let locales = Locales::new(env.paths.clone());
    let default = locales.default_locale().map(|l| l.id);
    let all = locales.list();
    for l in &all {
        let used = locales.images_with(&l.id).len();
        println!(
            "{} {}{}{}{}{}",
            if default.as_deref() == Some(l.id.as_str()) { "*" } else { " " },
            pad(&l.name, 16),
            pad(&l.locale.replace(".UTF-8", ""), 8),
            pad(&l.keyboards.join("+"), 12),
            pad(&l.time_zone, 20),
            if used == 0 { String::new() } else { format!("{used} image(s)") }
        );
    }
    if all.is_empty() {
        let host = Settings::load(&env.paths).host_keyboard.unwrap_or_else(|| GuestSettings::mac().keyboard);
        println!("(no locales yet – images get this computer's values, keyboard {host}: omaf locales add <name>)");
    }
    Ok(())
}

/// The locale `id` as it is stored now
fn stored(id: &str) -> omaflake_core::Result<GuestLocale> {
    Locales::new(Env::new().paths)
        .list()
        .into_iter()
        .find(|l| l.id == id)
        .ok_or_else(|| omaflake_core::Error::new(format!("No locale {id} – see omaf locales")))
}

/// `locales.add` / `locales.update`: only what is given
fn change(method: &str, id: Option<&str>, c: LocaleChange) -> omaflake_core::Result<GuestLocale> {
    let mut params = json!({});
    let texts = [("id", id.map(str::to_string)), ("name", c.name), ("locale", c.locale), ("timeZone", c.time_zone)];
    for (key, value) in texts {
        if let Some(v) = value {
            params[key] = json!(v);
        }
    }
    if let Some(k) = c.keyboards {
        params["keyboards"] = json!(k);
    }
    let answer = service::rpc(method, params)?;
    stored(answer["id"].as_str().unwrap_or_default())
}

pub fn add(a: &cli::LocaleArgs) -> Outcome {
    let env = Env::new();
    let mac = GuestSettings::mac();
    let host = Settings::load(&env.paths).host_keyboard.unwrap_or(mac.keyboard);
    let l = change(
        "locales.add",
        None,
        LocaleChange {
            name: Some(a.name.clone()),
            locale: Some(a.locale.clone().unwrap_or(mac.locale)),
            time_zone: Some(a.time_zone.clone().unwrap_or(mac.time_zone)),
            keyboards: Some(match &a.keyboards {
                Some(k) => keyboards(k)?,
                None => vec![host],
            }),
        },
    )?;
    println!("Locale {} added ({}, {}, {}).", l.name, l.locale, l.time_zone, l.keyboards.join("+"));
    Ok(())
}

pub fn edit(a: &cli::LocaleArgs) -> Outcome {
    let Some(l) = Locales::new(Env::new().paths).find(&a.name) else {
        return Err(usage(&["locales", "edit"], format!("No locale {} – see omaf locales", a.name)));
    };
    // The service checks the change: images are built with a locale's values
    let l = change(
        "locales.update",
        Some(&l.id),
        LocaleChange {
            name: a.rename.clone(),
            locale: a.locale.clone(),
            time_zone: a.time_zone.clone(),
            keyboards: a.keyboards.as_deref().map(keyboards).transpose()?,
        },
    )?;
    println!("Locale {} changed ({}, {}, {}).", l.name, l.locale, l.time_zone, l.keyboards.join("+"));
    Ok(())
}

pub fn remove(reference: &str) -> Outcome {
    let locales = Locales::new(Env::new().paths);
    let Some(l) = locales.find(reference) else {
        return Err(usage(&["locales", "rm"], format!("No locale {reference} – see omaf locales")));
    };
    let used = locales.images_with(&l.id).len();
    if used > 0 {
        return Err(usage(
            &["locales", "rm"],
            format!("Images are built with {} ({used}) – remove them first", l.name),
        ));
    }
    service::rpc("locales.remove", json!({"id": l.id}))?;
    println!("Locale {} removed.", l.name);
    Ok(())
}

pub fn set_default(reference: &str) -> Outcome {
    let Some(l) = Locales::new(Env::new().paths).find(reference) else {
        return Err(usage(&["locales", "default"], format!("No locale {reference} – see omaf locales")));
    };
    service::rpc("locales.setDefault", json!({"id": l.id}))?;
    println!("New images get the locale {}.", l.name);
    Ok(())
}
