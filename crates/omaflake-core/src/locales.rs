// SPDX-License-Identifier: AGPL-3.0-or-later
//! Locales (plan B141): where a guest lives – one system language with region, one time zone and one or more
//! keyboards (the first is the default; Super+Space switches on NixOS). Created and named like credentials, kept in
//! config.json (`locales`, `defaultLocale`, settings v5), never in a catalog: an image is built with one (the default
//! unless the build chooses another). The host's own keyboard is a setting of its own (`hostKeyboard`): the physical
//! keyboard, preset for new locales. Without any locale an image gets this computer's values.

use serde::{Deserialize, Serialize};

use crate::error::{Result, codes};
use crate::paths::{Paths, Settings};
use crate::vm_err;

/// One locale in config.json
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestLocale {
    /// Stable ID ("l-…"), what images name
    pub id: String,
    /// The user's own name for it ("German", "Work US")
    pub name: String,
    /// The system language and region: "de_DE.UTF-8"
    pub locale: String,
    #[serde(rename = "timeZone")]
    pub time_zone: String,
    /// XKB names ("de", "ru"); the first is the default
    pub keyboards: Vec<String>,
}

/// What a locale's fields are changed to: None leaves a field as it is
#[derive(Debug, Clone, Default)]
pub struct LocaleChange {
    pub name: Option<String>,
    pub locale: Option<String>,
    pub time_zone: Option<String>,
    pub keyboards: Option<Vec<String>>,
}

pub struct Locales {
    paths: Paths,
}

impl Locales {
    /// At most this many keyboards in one guest
    pub const MAX_KEYBOARDS: usize = 3;

    pub fn new(paths: Paths) -> Locales {
        Locales { paths }
    }

    pub fn list(&self) -> Vec<GuestLocale> {
        Settings::load(&self.paths).locales.unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Option<GuestLocale> {
        self.list().into_iter().find(|l| l.id == id)
    }

    /// By ID, else by name (any case)
    pub fn find(&self, id_or_name: &str) -> Option<GuestLocale> {
        let all = self.list();
        all.iter()
            .find(|l| l.id == id_or_name)
            .or_else(|| all.iter().find(|l| l.name.eq_ignore_ascii_case(id_or_name)))
            .cloned()
    }

    /// The default: the settings' `defaultLocale`, else the first – None only without any
    pub fn default_locale(&self) -> Option<GuestLocale> {
        let s = Settings::load(&self.paths);
        let all = s.locales.unwrap_or_default();
        s.default_locale.and_then(|id| all.iter().find(|l| l.id == id).cloned()).or_else(|| all.into_iter().next())
    }

    fn check(l: &GuestLocale) -> Result<()> {
        if l.name.trim().is_empty() {
            return Err(vm_err!("A locale needs a name"));
        }
        if l.locale.trim().is_empty() || l.time_zone.trim().is_empty() {
            return Err(vm_err!("A locale needs a language and a time zone"));
        }
        if l.keyboards.is_empty() || l.keyboards.len() > Self::MAX_KEYBOARDS {
            return Err(vm_err!("A locale has one to three keyboards"));
        }
        let mut seen = Vec::new();
        for k in &l.keyboards {
            if seen.contains(k) {
                return Err(vm_err!("The keyboard {} is there twice", k));
            }
            seen.push(k.clone());
        }
        Ok(())
    }

    fn apply(l: &mut GuestLocale, c: &LocaleChange) {
        if let Some(v) = &c.name {
            l.name = v.trim().to_string();
        }
        if let Some(v) = &c.locale {
            l.locale = v.trim().to_string();
        }
        if let Some(v) = &c.time_zone {
            l.time_zone = v.trim().to_string();
        }
        if let Some(v) = &c.keyboards {
            l.keyboards = v.iter().map(|k| k.trim().to_string()).filter(|k| !k.is_empty()).collect();
        }
    }

    /// A new locale (the first one becomes the default)
    pub fn add(&self, change: LocaleChange) -> Result<GuestLocale> {
        let mut s = Settings::load(&self.paths);
        let mut all = s.locales.clone().unwrap_or_default();
        let mut l = GuestLocale { id: random_id(), ..Default::default() };
        Self::apply(&mut l, &change);
        Self::check(&l)?;
        if all.iter().any(|x| x.name.eq_ignore_ascii_case(&l.name)) {
            return Err(vm_err!(codes::EXISTS, "There is a locale \"{}\" already", l.name));
        }
        all.push(l.clone());
        if s.default_locale.as_ref().is_none_or(|id| !all.iter().any(|x| &x.id == id)) {
            s.default_locale = Some(l.id.clone());
        }
        s.locales = Some(all);
        s.save(&self.paths)?;
        Ok(l)
    }

    pub fn update(&self, id: &str, change: LocaleChange) -> Result<GuestLocale> {
        let mut s = Settings::load(&self.paths);
        let mut all = s.locales.clone().unwrap_or_default();
        let Some(i) = all.iter().position(|l| l.id == id) else {
            return Err(vm_err!(codes::NOT_FOUND, "No locale \"{}\"", id));
        };
        let mut l = all[i].clone();
        Self::apply(&mut l, &change);
        Self::check(&l)?;
        if all.iter().any(|x| x.id != id && x.name.eq_ignore_ascii_case(&l.name)) {
            return Err(vm_err!(codes::EXISTS, "There is a locale \"{}\" already", l.name));
        }
        all[i] = l.clone();
        s.locales = Some(all);
        s.save(&self.paths)?;
        Ok(l)
    }

    /// Removes a locale; when it was the default, another one becomes it
    pub fn remove(&self, id: &str) -> Result<GuestLocale> {
        let mut s = Settings::load(&self.paths);
        let mut all = s.locales.clone().unwrap_or_default();
        let Some(i) = all.iter().position(|l| l.id == id) else {
            return Err(vm_err!(codes::NOT_FOUND, "No locale \"{}\"", id));
        };
        let l = all.remove(i);
        if s.default_locale.as_deref() == Some(id) {
            s.default_locale = all.first().map(|x| x.id.clone());
        }
        s.locales = (!all.is_empty()).then_some(all);
        s.save(&self.paths)?;
        Ok(l)
    }

    pub fn set_default(&self, id: &str) -> Result<()> {
        let mut s = Settings::load(&self.paths);
        if !s.locales.as_ref().is_some_and(|all| all.iter().any(|l| l.id == id)) {
            return Err(vm_err!(codes::NOT_FOUND, "No locale \"{}\"", id));
        }
        s.default_locale = Some(id.to_string());
        s.save(&self.paths)
    }

    /// The images built with locale `id`
    pub fn images_with(&self, id: &str) -> Vec<String> {
        crate::image_store::ImageStore::new(self.paths.clone())
            .list()
            .into_iter()
            .filter(|i| i.info.locale.as_deref() == Some(id))
            .map(|i| i.name)
            .collect()
    }

    /// Whether `change` may be made: images carry the locale's values – while used, only the name can change
    pub fn check_change(&self, id: &str, change: &LocaleChange) -> Result<()> {
        let Some(current) = self.get(id) else { return Ok(()) };
        let changes = change.locale.as_ref().is_some_and(|v| v.trim() != current.locale)
            || change.time_zone.as_ref().is_some_and(|v| v.trim() != current.time_zone)
            || change.keyboards.as_ref().is_some_and(|v| {
                v.iter().map(|k| k.trim()).filter(|k| !k.is_empty()).collect::<Vec<_>>()
                    != current.keyboards.iter().map(String::as_str).collect::<Vec<_>>()
            });
        if changes && !self.images_with(id).is_empty() {
            return Err(vm_err!(codes::IN_USE, "Images are built with this locale – only its name can change"));
        }
        Ok(())
    }
}

/// A random locale ID: "l-" and 12 hex digits
pub fn random_id() -> String {
    format!("l-{}", &crate::credentials::random_id()[2..])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;

    fn change(name: &str, keyboards: &[&str]) -> LocaleChange {
        LocaleChange {
            name: Some(name.into()),
            locale: Some("de_DE.UTF-8".into()),
            time_zone: Some("Europe/Berlin".into()),
            keyboards: Some(keyboards.iter().map(|k| k.to_string()).collect()),
        }
    }

    #[test]
    fn add_update_remove_and_the_default() {
        let (_t, p) = temp_paths();
        let l = Locales::new(p.clone());
        assert!(l.default_locale().is_none());
        let de = l.add(change("German", &["de", "ru"])).unwrap();
        assert!(de.id.starts_with("l-") && de.id.len() == 14);
        assert_eq!(l.default_locale().map(|x| x.id), Some(de.id.clone()), "the first is the default");
        assert!(l.add(change("german", &["de"])).is_err(), "names are unique, any case");
        assert!(l.add(change("Four", &["de", "us", "fr", "ru"])).is_err(), "at most three keyboards");
        assert!(l.add(change("Twice", &["de", "de"])).is_err());
        assert!(l.add(LocaleChange { name: Some("Empty".into()), ..Default::default() }).is_err());
        let us = l.add(LocaleChange { locale: Some("en_US.UTF-8".into()), ..change("US", &["us"]) }).unwrap();
        let de2 = l.update(&de.id, LocaleChange { name: Some("Deutsch".into()), ..Default::default() }).unwrap();
        assert_eq!((de2.name.as_str(), de2.keyboards.len()), ("Deutsch", 2));
        assert_eq!(l.find("deutsch").map(|x| x.id), Some(de.id.clone()));
        l.set_default(&us.id).unwrap();
        assert!(l.set_default("l-none").is_err());
        l.remove(&us.id).unwrap();
        assert_eq!(l.default_locale().map(|x| x.id), Some(de.id), "another becomes the default");
    }
}
