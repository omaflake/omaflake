// SPDX-License-Identifier: AGPL-3.0-or-later
//! Credentials (plan B137): named sets of who you are in a guest – user, full name, e-mail and, optionally, the
//! password a macOS build needs (sudo in the build VM). An image is built from a recipe with one credential and one
//! look; the settings only name the default ones.
//!
//! The values live in config.json (`credentials`, `defaultCredential`, settings v3); a password never does – it goes
//! to this computer's secret store like a private source's token (`source_tokens::Tokens`: the Keychain on macOS,
//! the Secret Service on Linux, else the 0600 file), under the account `credential:<id>`.

use serde::{Deserialize, Serialize};

use crate::error::{Result, codes};
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

    /// The default credential: the settings' `defaultCredential`, else the first one – there is a default whenever
    /// there is a credential (plan B139); None only without any
    pub fn default_credential(&self) -> Option<Credential> {
        let s = Settings::load(&self.paths);
        let all = s.credentials.unwrap_or_default();
        s.default_credential.and_then(|id| all.iter().find(|c| c.id == id).cloned()).or_else(|| all.into_iter().next())
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
        // Every credential has a password (the maintainer's decision, plan B137)
        if change.password.as_deref().is_none_or(str::is_empty) {
            return Err(vm_err!("A credential needs a password"));
        }
        let mut c = Credential { id: Self::new_id(&all), name, ..Default::default() };
        Self::apply(&mut c, &change)?;
        // The host has nothing to do with credentials (plan B139): each names its own user
        if c.user.is_none() {
            return Err(vm_err!("A credential needs a user"));
        }
        all.push(c.clone());
        if s.default_credential.as_ref().is_none_or(|id| !all.iter().any(|c| &c.id == id)) {
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
            Some("") => return Err(vm_err!("A credential's password cannot be removed – only changed")),
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
            // Another one becomes the default
            s.default_credential = all.first().map(|c| c.id.clone());
        }
        s.credentials = (!all.is_empty()).then_some(all);
        s.save(&self.paths)?;
        let _ = self.tokens.set_account(&Self::account(id), &Self::label(&c), None);
        Ok(c)
    }

    /// Makes `id` the default – there is always one while there are credentials (plan B139)
    pub fn set_default(&self, id: &str) -> Result<()> {
        let mut s = Settings::load(&self.paths);
        if !s.credentials.as_ref().is_some_and(|all| all.iter().any(|c| c.id == id)) {
            return Err(vm_err!(codes::NOT_FOUND, "No credential \"{id}\""));
        }
        s.default_credential = Some(id.to_string());
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

/// Asks the person at this computer to prove it is them before a password leaves the secret store (plan B137): Touch
/// ID or the login password (LocalAuthentication, `deviceOwnerAuthentication`) on macOS. On Linux the keyring's own
/// unlock is the guard (OPEN: a polkit check). `Ok(false)`: cancelled or failed.
pub fn confirm_owner(reason: &str) -> Result<bool> {
    #[cfg(target_os = "macos")]
    {
        use objc2_foundation::{NSError, NSString};
        use objc2_local_authentication::{LAContext, LAPolicy};
        let (tx, rx) = std::sync::mpsc::channel::<bool>();
        let reply = block2::RcBlock::new(move |ok: objc2::runtime::Bool, _error: *mut NSError| {
            let _ = tx.send(ok.as_bool());
        });
        // SAFETY: the reply only sends on a channel (sendable); the context lives until the answer came
        let context = unsafe { LAContext::new() };
        unsafe {
            context.evaluatePolicy_localizedReason_reply(
                LAPolicy::DeviceOwnerAuthentication,
                &NSString::from_str(reason),
                &reply,
            )
        };
        let ok = rx.recv_timeout(std::time::Duration::from_secs(300)).unwrap_or(false);
        drop(context);
        Ok(ok)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = reason;
        Ok(true)
    }
}

/// Puts `text` on this computer's clipboard: pbcopy on macOS; wl-copy, else xclip on Linux
pub fn copy_to_clipboard(text: &str) -> Result<()> {
    use std::io::Write;
    use std::process::{Command, Stdio};
    let tools: &[(&str, &[&str])] = if cfg!(target_os = "macos") {
        &[("/usr/bin/pbcopy", &[])]
    } else {
        &[("wl-copy", &[]), ("xclip", &["-selection", "clipboard"])]
    };
    for (tool, args) in tools {
        let Ok(mut child) =
            Command::new(tool).args(*args).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn()
        else {
            continue;
        };
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(text.as_bytes())?;
        }
        if child.wait()?.success() {
            return Ok(());
        }
    }
    Err(vm_err!("No clipboard tool (pbcopy, wl-copy, xclip) to copy with"))
}

/// A guest user's account as omaflake creates or logs into it (plan B139): from a credential, with its password
#[derive(Debug, Clone)]
pub struct Login {
    pub credential: Credential,
    pub user: String,
    pub full_name: String,
    pub password: String,
}

/// What uses a credential (plan B137, B139): images built with it, macOS bases created from it
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Usage {
    pub images: Vec<String>,
    pub bases: Vec<String>,
}

impl Usage {
    pub fn of(paths: &Paths, id: &str) -> Usage {
        let images = crate::image_store::ImageStore::new(paths.clone())
            .list()
            .into_iter()
            .filter(|i| i.info.credential.as_deref() == Some(id))
            .map(|i| i.name)
            .collect();
        let bases = crate::mac_base_installer::MacBaseInstaller::list(paths)
            .into_iter()
            .filter(|b| b.record.as_ref().is_some_and(|r| r.credential == id))
            .map(|b| b.name)
            .collect();
        Usage { images, bases }
    }

    pub fn is_empty(&self) -> bool {
        self.images.is_empty() && self.bases.is_empty()
    }

    /// Whether `change` of `current` may be made: images carry its user, full name and e-mail, a macOS base's
    /// account has its user and password – while used, only the name can change
    pub fn check_change(&self, current: &Credential, change: &CredentialChange) -> Result<()> {
        let clean = |v: &Option<String>| v.as_deref().map(str::trim).filter(|v| !v.is_empty()).map(str::to_string);
        let changes = |new: &Option<String>, old: &Option<String>| new.is_some() && clean(new) != clean(old);
        let values = changes(&change.user, &current.user)
            || changes(&change.full_name, &current.full_name)
            || changes(&change.email, &current.email);
        if values && !self.bases.is_empty() {
            return Err(vm_err!(
                codes::IN_USE,
                "A macOS base was created from this credential – only its name can change"
            ));
        }
        if values && !self.images.is_empty() {
            return Err(vm_err!(codes::IN_USE, "Images are built with this credential – only its name can change"));
        }
        if change.password.as_deref().is_some_and(|p| !p.is_empty()) && !self.bases.is_empty() {
            return Err(vm_err!(
                codes::IN_USE,
                "A macOS base's user has this password – it stays as long as the base is there"
            ));
        }
        Ok(())
    }

    /// Whether the credential may be removed
    pub fn check_remove(&self) -> Result<()> {
        if !self.bases.is_empty() {
            return Err(vm_err!(codes::IN_USE, "A macOS base was created from this credential – remove it first"));
        }
        if !self.images.is_empty() {
            return Err(vm_err!(codes::IN_USE, "Images are built with this credential – remove them first"));
        }
        Ok(())
    }
}

impl Credentials {
    /// The account of a credential: its user and full name (else this computer's) and its stored password
    pub fn login(&self, c: &Credential) -> Result<Login> {
        let password = self.password(&c.id).filter(|p| !p.is_empty()).ok_or_else(|| {
            vm_err!(
                codes::PREREQUISITE,
                "The credential \"{}\" has no password yet – set one under Credentials",
                c.name
            )
        })?;
        Ok(Login {
            user: c.user.clone().filter(|u| !u.is_empty()).unwrap_or_else(crate::platform::user_name),
            full_name: c.full_name.clone().filter(|n| !n.is_empty()).unwrap_or_else(crate::platform::full_user_name),
            credential: c.clone(),
            password,
        })
    }

    /// The account a new macOS base gets (plan B139): credential `reference` (ID or name), else the default one
    pub fn base_login(&self, reference: Option<&str>) -> Result<Login> {
        let c = match reference.filter(|r| !r.is_empty()) {
            Some(r) => self.find(r).ok_or_else(|| vm_err!(codes::NOT_FOUND, "No credential \"{r}\""))?,
            None => self.default_credential().ok_or_else(|| {
                vm_err!(
                    codes::PREREQUISITE,
                    "No credential yet – create one first (Credentials, or omaf credentials add)"
                )
            })?,
        };
        self.login(&c)
    }

    /// The account macOS image builds log into on base `dir`: its record's credential with its password – so no
    /// build asks for one (plan B139)
    pub fn login_of_base(&self, dir: &std::path::Path) -> Result<Login> {
        let name = dir.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let record = crate::mac_base_installer::BaseRecord::of(dir).ok_or_else(|| {
            vm_err!(
                codes::PREREQUISITE,
                "{} has no credential yet – assign the one its user was created with (Images → macOS Bases, or omaf macos assign {} <credential>)",
                name,
                name
            )
        })?;
        let c = self.get(&record.credential).ok_or_else(|| {
            vm_err!(
                codes::NOT_FOUND,
                "The credential of {} (\"{}\") is gone – assign another one",
                name,
                record.credential_name
            )
        })?;
        let mut login = self.login(&c)?;
        // The account is the base's: its user, whatever the credential says now
        login.user = record.user;
        Ok(login)
    }

    /// Assigns credential `reference` to base `dir` – for a base created before bases had one (plan B139). Its user
    /// must be the base's; whether the password is right shows the next build.
    pub fn assign_to_base(
        &self,
        dir: &std::path::Path,
        reference: &str,
    ) -> Result<crate::mac_base_installer::BaseRecord> {
        use crate::mac_base_installer::{BaseRecord, MacBaseInstaller};
        let c = self.find(reference).ok_or_else(|| vm_err!(codes::NOT_FOUND, "No credential \"{reference}\""))?;
        let login = self.login(&c)?;
        if !MacBaseInstaller::is_ready(dir) {
            return Err(vm_err!(codes::PREREQUISITE, "{} is not installed completely", dir.display()));
        }
        if let Some(user) = MacBaseInstaller::user_of(dir, &self.paths.logs())
            && user != login.user
        {
            return Err(vm_err!("The base's user is {} – the credential \"{}\" has {}", user, c.name, login.user));
        }
        let record = BaseRecord { credential: c.id.clone(), credential_name: c.name.clone(), user: login.user };
        record.write(dir)?;
        Ok(record)
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
        assert!(c.add(change("Private", "al", None)).is_err()); // every credential has a password
        let (private, _) = c.add(change("Private", "al", Some("pw"))).unwrap();
        let p2 = private.id.clone();
        assert!(p2 != w1 && c.has_password(&p2));
        assert!(c.add(change("work", "x", Some("pw"))).is_err()); // names are unique, any case
        assert!(c.add(change("Bad", "Alex!", Some("pw"))).is_err());
        // A change keeps what it does not name; "" clears
        let (w, _) =
            c.update(&w1, CredentialChange { email: Some("a@example.org".into()), ..Default::default() }).unwrap();
        assert_eq!((w.user.as_deref(), w.email.as_deref()), (Some("alex"), Some("a@example.org")));
        // A password can change, not go
        assert!(c.update(&w1, CredentialChange { password: Some(String::new()), ..Default::default() }).is_err());
        assert!(c.has_password(&w1));
        c.update(&w1, CredentialChange { password: Some("new".into()), ..Default::default() }).unwrap();
        assert_eq!(c.password(&w1).as_deref(), Some("new"));
        assert_eq!(c.find("private").map(|p| p.id), Some(p2.clone()));
        // Removing the default makes another one the default; the password goes too
        c.remove(&w1).unwrap();
        assert_eq!(c.default_credential().map(|d| d.id), Some(p2.clone()));
        assert_eq!(fake.0.lock().unwrap().len(), 1);
        c.set_default(&p2).unwrap();
        assert!(c.set_default("c-9").is_err());
        assert!(c.add(change("Work", "alex", Some("pw"))).is_ok()); // the name is free again
        // Every credential names its own user – the host has nothing to do with them (plan B139)
        assert!(
            c.add(CredentialChange { name: Some("NoUser".into()), password: Some("pw".into()), ..Default::default() })
                .is_err()
        );
        c.remove(&p2).unwrap();
        let rest = c.list();
        assert_eq!(c.default_credential().map(|d| d.id), rest.first().map(|r| r.id.clone()));
    }

    #[test]
    fn a_base_keeps_its_credential_and_builds_log_in_with_it() {
        use crate::mac_base_installer::{BaseRecord, MacBaseInstaller};
        use std::os::unix::fs::PermissionsExt;
        let (_t, c, _fake) = setup();
        // No credential: a base cannot be created
        assert!(c.base_login(None).is_err());
        let (work, _) = c.add(change("Work", "alex", Some("secret"))).unwrap();
        let (other, _) = c.add(change("Other", "sam", Some("pw"))).unwrap();
        // The default, or the one named
        let login = c.base_login(None).unwrap();
        assert_eq!((login.user.as_str(), login.password.as_str()), ("alex", "secret"));
        assert_eq!(c.base_login(Some("other")).unwrap().credential.id, other.id);
        // A finished base from before: read-only, its user in the ready marker, no record
        let dir = c.paths.bases().join("mac-base-1");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join(MacBaseInstaller::READY_MARKER), "alex\n").unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        let e = c.login_of_base(&dir).unwrap_err();
        assert!(e.to_string().contains("has no credential yet"), "{e}");
        // Another user than the base's is refused; the right one is written, the folder stays read-only
        assert!(c.assign_to_base(&dir, "Other").is_err());
        let record = c.assign_to_base(&dir, &work.id).unwrap();
        assert_eq!(BaseRecord::of(&dir), Some(record));
        assert_eq!(std::fs::metadata(&dir).unwrap().permissions().mode() & 0o777, 0o555);
        let login = c.login_of_base(&dir).unwrap();
        assert_eq!((login.credential.id.as_str(), login.password.as_str()), (work.id.as_str(), "secret"));
        // Used by the base: user and password stay, removing is refused; the name may change
        let usage = Usage::of(&c.paths, &work.id);
        assert_eq!(usage.bases, vec!["mac-base-1".to_string()]);
        let current = c.get(&work.id).unwrap();
        assert!(
            usage.check_change(&current, &CredentialChange { user: Some("bob".into()), ..Default::default() }).is_err()
        );
        assert!(
            usage
                .check_change(&current, &CredentialChange { password: Some("x".into()), ..Default::default() })
                .is_err()
        );
        assert!(
            usage.check_change(&current, &CredentialChange { name: Some("Job".into()), ..Default::default() }).is_ok()
        );
        assert!(usage.check_remove().is_err());
        assert!(Usage::of(&c.paths, &other.id).check_remove().is_ok());
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
    }
}
