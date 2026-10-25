// SPDX-License-Identifier: AGPL-3.0-or-later
//! Credentials (plan B137): named sets of who you are in a guest – user, full name, e-mail and, optionally, the
//! password a macOS build needs (sudo in the build VM). An image is built from a recipe with one credential and one
//! look; the settings only name the default ones.
//!
//! The values live in config.json (`credentials`, `defaultCredential`, settings v3); a password never does – it goes
//! to this computer's secret store like a private source's token (`source_tokens::Tokens`: the Keychain on macOS,
//! the Secret Service on Linux, else the 0600 file), under the account `credential:<id>`.

use serde::{Deserialize, Serialize};

use crate::error::{codes, Result};
use crate::paths::{Paths, Settings};
use crate::source_tokens::{TokenPlace, Tokens};
use crate::vm_err;

/// One credential in config.json. None fields are this computer's values (user name, full name) or none (e-mail).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Credential {
    /// Stable ID ("c-…"), what images and the secret store name
    pub id: String,
    /// The user's own name for it ("Work", "Private")
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(rename = "fullName", default, skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// What a credential's fields are changed to: None leaves a field as it is, Some("") clears it
#[derive(Debug, Clone, Default)]
pub struct CredentialChange {
    pub name: Option<String>,
    pub user: Option<String>,
    pub full_name: Option<String>,
    pub email: Option<String>,
    /// The password: Some("") removes it
    pub password: Option<String>,
}

/// The credentials of a data folder's settings, with their passwords in the secret store
pub struct Credentials {
    paths: Paths,
    tokens: Tokens,
}

impl Credentials {
    pub fn new(paths: Paths) -> Credentials {
        Credentials { paths, tokens: Tokens::system() }
    }

    /// With another secret store (tests)
    pub fn with_tokens(paths: Paths, tokens: Tokens) -> Credentials {
        Credentials { paths, tokens }
    }

    fn account(id: &str) -> String {
        format!("credential:{id}")
    }

    pub fn list(&self) -> Vec<Credential> {
        Settings::load(&self.paths).credentials.unwrap_or_default()
    }

    pub fn get(&self, id: &str) -> Option<Credential> {
        self.list().into_iter().find(|c| c.id == id)
    }

    /// By ID, else by name (case-insensitive) – what the CLI takes
    pub fn find(&self, id_or_name: &str) -> Option<Credential> {
        let all = self.list();
        all.iter()
            .find(|c| c.id == id_or_name)
            .or_else(|| all.iter().find(|c| c.name.eq_ignore_ascii_case(id_or_name)))
            .cloned()
    }

    /// The default credential (the settings' `defaultCredential`); None: this computer's values
    pub fn default_credential(&self) -> Option<Credential> {
        let s = Settings::load(&self.paths);
        let id = s.default_credential?;
        s.credentials.unwrap_or_default().into_iter().find(|c| c.id == id)
    }

    pub fn has_password(&self, id: &str) -> bool {
        self.tokens.has_account(&Self::account(id))
    }

    pub fn password(&self, id: &str) -> Option<String> {
        self.tokens.get_account(&Self::account(id))
    }

    /// A new credential (the first one becomes the default) and where its password went, if it has one
    pub fn add(&self, mut change: CredentialChange) -> Result<(Credential, Option<TokenPlace>)> {
        let name = change.name.take().map(|n| n.trim().to_string()).unwrap_or_default();
        if name.is_empty() {
            return Err(vm_err!("A credential needs a name"));
        }
        let mut s = Settings::load(&self.paths);
        let mut all = s.credentials.clone().unwrap_or_default();
        if all.iter().any(|c| c.name.eq_ignore_ascii_case(&name)) {
            return Err(vm_err!(codes::EXISTS, "There is a credential \"{name}\" already"));
        }
        let mut c = Credential { id: Self::new_id(&all), name, ..Default::default() };
        Self::apply(&mut c, &change)?;
        all.push(c.clone());
        if s.default_credential.is_none() && all.len() == 1 {
            s.default_credential = Some(c.id.clone());
        }
        s.credentials = Some(all);
        s.save(&self.paths)?;
        let place = match change.password.as_deref() {
            Some(p) if !p.is_empty() => self.tokens.set_account(&Self::account(&c.id), &Self::label(&c), Some(p))?,
            _ => None,
        };
        Ok((c, place))
    }

    /// Changes a credential; a new password replaces the stored one, "" removes it
    pub fn update(&self, id: &str, change: CredentialChange) -> Result<(Credential, Option<TokenPlace>)> {
        let mut s = Settings::load(&self.paths);
        let mut all = s.credentials.clone().unwrap_or_default();
        let Some(i) = all.iter().position(|c| c.id == id) else {
            return Err(vm_err!(codes::NOT_FOUND, "No credential \"{id}\""));
        };
        if let Some(name) = change.name.as_deref().map(str::trim) {
            if name.is_empty() {
                return Err(vm_err!("A credential needs a name"));
            }
            if all.iter().any(|c| c.id != id && c.name.eq_ignore_ascii_case(name)) {
                return Err(vm_err!(codes::EXISTS, "There is a credential \"{name}\" already"));
            }
            all[i].name = name.to_string();
        }
        Self::apply(&mut all[i], &change)?;
        let c = all[i].clone();
        s.credentials = Some(all);
        s.save(&self.paths)?;
        let place = match change.password.as_deref() {
            Some(p) => self.tokens.set_account(&Self::account(id), &Self::label(&c), Some(p))?,
            None => None,
        };
        Ok((c, place))
    }

    /// Removes a credential and its password; images built with it keep its name in their image.json
    pub fn remove(&self, id: &str) -> Result<Credential> {
        let mut s = Settings::load(&self.paths);
        let mut all = s.credentials.clone().unwrap_or_default();
        let Some(i) = all.iter().position(|c| c.id == id) else {
            return Err(vm_err!(codes::NOT_FOUND, "No credential \"{id}\""));
        };
        let c = all.remove(i);
        if s.default_credential.as_deref() == Some(id) {
            s.default_credential = None;
        }
        s.credentials = (!all.is_empty()).then_some(all);
        s.save(&self.paths)?;
        let _ = self.tokens.set_account(&Self::account(id), &Self::label(&c), None);
        Ok(c)
    }

    /// Makes `id` the default (None: this computer's values)
    pub fn set_default(&self, id: Option<&str>) -> Result<()> {
        let mut s = Settings::load(&self.paths);
        if let Some(id) = id {
            if !s.credentials.as_ref().is_some_and(|all| all.iter().any(|c| c.id == id)) {
                return Err(vm_err!(codes::NOT_FOUND, "No credential \"{id}\""));
            }
        }
        s.default_credential = id.map(str::to_string);
        s.save(&self.paths)
    }

    fn apply(c: &mut Credential, change: &CredentialChange) -> Result<()> {
        let clean = |v: &str| (!v.trim().is_empty()).then(|| v.trim().to_string());
        if let Some(user) = &change.user {
            let user = clean(user);
            if let Some(u) = &user {
                let ok = u.len() <= 32
                    && u.chars().next().is_some_and(|ch| ch.is_ascii_lowercase() || ch == '_')
                    && u.chars().all(|ch| ch.is_ascii_lowercase() || ch.is_ascii_digit() || ch == '_' || ch == '-');
                if !ok {
                    return Err(vm_err!(
                        "Invalid user name \"{u}\": a–z, 0–9, \"_\" and \"-\", starting with a letter"
                    ));
                }
            }
            c.user = user;
        }
        if let Some(full) = &change.full_name {
            c.full_name = clean(full);
        }
        if let Some(email) = &change.email {
            c.email = clean(email);
        }
        Ok(())
    }

    fn label(c: &Credential) -> String {
        format!("omaflake credential {}", c.name)
    }

    /// "c-<12 hex digits>", random: the secret store's account `credential:<id>` is this computer's, shared by every
    /// data folder – a counter would let two folders (or a test's) reach each other's passwords
    fn new_id(all: &[Credential]) -> String {
        loop {
            let id = random_id();
            if all.iter().all(|c| c.id != id) {
                return id;
            }
        }
    }
}

/// A random credential ID: "c-" and 12 hex digits (time, process and a counter, hashed)
pub fn random_id() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};
    use std::sync::atomic::{AtomicU64, Ordering};
    static COUNT: AtomicU64 = AtomicU64::new(0);
    let mut h = RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_nanos()));
    h.write_u32(std::process::id());
    h.write_u64(COUNT.fetch_add(1, Ordering::Relaxed));
    format!("c-{:012x}", h.finish() & 0xffff_ffff_ffff)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source_tokens::{SecretStore, StoreError, TokenFile};
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Clone, Default)]
    struct Fake(Arc<Mutex<HashMap<String, String>>>);

    impl SecretStore for Fake {
        fn place(&self) -> TokenPlace {
            TokenPlace::Keychain
        }
        fn get(&self, account: &str) -> std::result::Result<Option<String>, StoreError> {
            Ok(self.0.lock().unwrap().get(account).cloned())
        }
        fn has(&self, account: &str) -> std::result::Result<bool, StoreError> {
            Ok(self.0.lock().unwrap().contains_key(account))
        }
        fn set(&self, account: &str, _: &str, token: &str) -> std::result::Result<(), StoreError> {
            self.0.lock().unwrap().insert(account.into(), token.into());
            Ok(())
        }
        fn delete(&self, account: &str) -> std::result::Result<(), StoreError> {
            self.0.lock().unwrap().remove(account);
            Ok(())
        }
    }

    fn setup() -> (tempfile::TempDir, Credentials, Fake) {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().to_path_buf(), None);
        let fake = Fake::default();
        let tokens =
            Tokens { store: Box::new(fake.clone()), file: Some(TokenFile { path: t.path().join("tokens.json") }) };
        (t, Credentials::with_tokens(paths, tokens), fake)
    }

    fn change(name: &str, user: &str, password: Option<&str>) -> CredentialChange {
        CredentialChange {
            name: Some(name.into()),
            user: Some(user.into()),
            password: password.map(str::to_string),
            ..Default::default()
        }
    }

    #[test]
    fn add_update_remove_with_the_password_in_the_store() {
        let (_t, c, fake) = setup();
        let (work, place) = c.add(change("Work", "alex", Some("secret"))).unwrap();
        let w1 = work.id.clone();
        assert!(w1.starts_with("c-") && w1.len() == 14 && work.user.as_deref() == Some("alex"));
        assert_eq!(place, Some(TokenPlace::Keychain));
        assert_eq!(c.password(&w1).as_deref(), Some("secret"));
        assert!(!std::fs::read_to_string(&c.paths.config_file).unwrap().contains("secret"));
        // The first one is the default
        assert_eq!(c.default_credential().map(|d| d.id), Some(w1.clone()));
        let (private, _) = c.add(change("Private", "al", None)).unwrap();
        let p2 = private.id.clone();
        assert!(p2 != w1 && !c.has_password(&p2));
        assert!(c.add(change("work", "x", None)).is_err()); // names are unique, any case
        assert!(c.add(change("Bad", "Alex!", None)).is_err());
        // A change keeps what it does not name; "" clears
        let (w, _) =
            c.update(&w1, CredentialChange { email: Some("a@example.org".into()), ..Default::default() }).unwrap();
        assert_eq!((w.user.as_deref(), w.email.as_deref()), (Some("alex"), Some("a@example.org")));
        c.update(&w1, CredentialChange { password: Some(String::new()), ..Default::default() }).unwrap();
        assert!(!c.has_password(&w1));
        c.update(&w1, CredentialChange { password: Some("new".into()), ..Default::default() }).unwrap();
        assert_eq!(c.password(&w1).as_deref(), Some("new"));
        assert_eq!(c.find("private").map(|p| p.id), Some(p2.clone()));
        // Removing the default leaves this computer's values; the password goes too
        c.remove(&w1).unwrap();
        assert!(c.default_credential().is_none() && fake.0.lock().unwrap().is_empty());
        c.set_default(Some(&p2)).unwrap();
        assert!(c.set_default(Some("c-9")).is_err());
        assert!(c.add(change("Work", "alex", None)).is_ok()); // the name is free again
    }
}
