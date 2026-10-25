// SPDX-License-Identifier: AGPL-3.0-or-later
//! Fine-grained GitHub tokens of private recipe sources (RecipeSources.swift `SourceTokens`) – never in config.json.
//!
//! - **macOS:** the login Keychain (`security-framework`, generic password, service `SourceTokens::SERVICE`, account =
//!   `RecipeSource::id`, label "omaflake recipe source <name>").
//! - **Linux:** the Secret Service over D-Bus (GNOME Keyring, KWallet, KeePassXC …; crate `secret-service`, pure Rust)
//!   with the same service and account as attributes and the same label. When none is reachable – a server, an SSH
//!   session without a desktop – the token goes to `$XDG_CONFIG_HOME/omaflake/tokens.json` (default `~/.config`; the
//!   file 0600, its folder 0700), **unencrypted**; where it was put is returned, so the caller can say so.
//!
//! The system store is a trait (`SecretStore`), so tests run against a fake and a temporary file, never the user's
//! Keychain or keyring.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Error, IoContext, Result};
use crate::file_format::{FileFormat, Step};
use crate::recipe_sources::RecipeSource;

/// Where a token was put
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenPlace {
    /// The login Keychain (macOS)
    Keychain,
    /// The desktop's keyring through the Secret Service (Linux)
    Keyring,
    /// tokens.json – no keyring was reachable
    File(PathBuf),
}

impl TokenPlace {
    /// What to tell the user beside "added": only the file needs a word
    pub fn note(&self) -> Option<String> {
        match self {
            TokenPlace::File(path) => {
                Some(format!("The token is stored unencrypted in {} – no keyring", path.display()))
            }
            _ => None,
        }
    }
}

/// Why a system store could not do something
#[derive(Debug)]
pub enum StoreError {
    /// No store reachable (no session bus, no Secret Service, no answer): the file takes over
    Unavailable,
    /// The store answered with an error (e.g. the unlock prompt was dismissed)
    Failed(Error),
}

/// A system store for secrets – the Keychain or the Secret Service (a fake in tests)
pub trait SecretStore {
    /// Where a token stored here is
    fn place(&self) -> TokenPlace;
    fn get(&self, account: &str) -> std::result::Result<Option<String>, StoreError>;
    /// Whether there is an item – without reading the secret (no prompt)
    fn has(&self, account: &str) -> std::result::Result<bool, StoreError>;
    fn set(&self, account: &str, label: &str, token: &str) -> std::result::Result<(), StoreError>;
    fn delete(&self, account: &str) -> std::result::Result<(), StoreError>;
}

// MARK: The file

static TOKENS_STEPS: [Step; 1] = [|_| Ok(())];

/// tokens.json: `{"$schema": …/tokens/v1.json, "tokens": {"<source id>": "<token>"}}`
#[derive(Debug, Default, Serialize, Deserialize)]
struct TokensFile {
    #[serde(default)]
    tokens: BTreeMap<String, String>,
}

/// The file tokens go to without a keyring
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TokenFile {
    pub path: PathBuf,
}

impl TokenFile {
    /// Schema `tokens` (plan B114); version 1 is the first
    pub const FILE_FORMAT: FileFormat = FileFormat::new("tokens", "tokens.json", 1, &TOKENS_STEPS);

    /// `$XDG_CONFIG_HOME/omaflake/tokens.json`, `~/.config/…` without it (a relative one is ignored, as the XDG spec says)
    pub fn standard() -> TokenFile {
        Self::under(std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from), &crate::platform::home_dir())
    }

    pub fn under(config_home: Option<PathBuf>, home: &Path) -> TokenFile {
        let config = config_home.filter(|c| c.is_absolute()).unwrap_or_else(|| home.join(".config"));
        TokenFile { path: config.join("omaflake").join("tokens.json") }
    }

    fn load(&self) -> Result<BTreeMap<String, String>> {
        let Ok(data) = std::fs::read(&self.path) else { return Ok(BTreeMap::new()) };
        let (file, _): (TokensFile, bool) = Self::FILE_FORMAT.decode(&data)?;
        Ok(file.tokens)
    }

    fn save(&self, tokens: BTreeMap<String, String>) -> Result<()> {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        Self::FILE_FORMAT.check_writable(&self.path)?;
        let dir = self.path.parent().unwrap_or_else(|| Path::new("."));
        if !dir.exists() {
            std::fs::DirBuilder::new().recursive(true).mode(0o700).create(dir).with_path(dir)?;
        }
        let data = Self::FILE_FORMAT.encode(&TokensFile { tokens })?;
        // Written 0600 from the start (a temporary file next to it, then renamed)
        let tmp = dir.join(format!(".tokens.json.tmp-{}", std::process::id()));
        {
            use std::io::Write;
            use std::os::unix::fs::OpenOptionsExt;
            let _ = std::fs::remove_file(&tmp);
            let mut f =
                std::fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&tmp).with_path(&tmp)?;
            f.write_all(&data).with_path(&tmp)?;
        }
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600)).with_path(&tmp)?;
        std::fs::rename(&tmp, &self.path).with_path(&self.path)
    }

    pub fn get(&self, account: &str) -> Option<String> {
        self.load().ok()?.remove(account).filter(|t| !t.is_empty())
    }

    pub fn has(&self, account: &str) -> bool {
        self.get(account).is_some()
    }

    pub fn set(&self, account: &str, token: &str) -> Result<()> {
        let mut tokens = self.load()?;
        tokens.insert(account.to_string(), token.to_string());
        self.save(tokens)
    }

    /// Removes the account's token; the file goes once it holds none
    pub fn delete(&self, account: &str) -> Result<()> {
        if !self.path.exists() {
            return Ok(());
        }
        let mut tokens = self.load()?;
        if tokens.remove(account).is_none() {
            return Ok(());
        }
        if tokens.is_empty() {
            return std::fs::remove_file(&self.path).with_path(&self.path);
        }
        self.save(tokens)
    }
}

// MARK: Store and file together

/// A system store, with the file as the fallback where there is one (Linux)
pub struct Tokens {
    pub store: Box<dyn SecretStore>,
    pub file: Option<TokenFile>,
}

impl Tokens {
    /// This computer's: the Keychain on macOS; the Secret Service, else tokens.json, on Linux
    pub fn system() -> Tokens {
        #[cfg(target_os = "macos")]
        {
            Tokens { store: Box::new(keychain::Keychain), file: None }
        }
        #[cfg(target_os = "linux")]
        {
            Tokens { store: Box::new(secret_service_store::SecretServiceStore), file: Some(TokenFile::standard()) }
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Tokens { store: Box::new(NoStore), file: Some(TokenFile::standard()) }
        }
    }

    pub fn token(&self, source: &RecipeSource) -> Option<String> {
        self.get_account(&source.id())
    }

    pub fn has(&self, source: &RecipeSource) -> bool {
        self.has_account(&source.id())
    }

    /// Stores the token (None or "" removes it) and says where it went (None: removed). An older one goes from both
    /// places first, so a token is never in the keyring and the file at once.
    pub fn set(&self, token: Option<&str>, source: &RecipeSource) -> Result<Option<TokenPlace>> {
        self.set_account(&source.id(), &format!("omaflake recipe source {}", source.display_name()), token)
    }

    /// Any secret by its account (a source's ID, `credential:<id>` – plan B137)
    pub fn get_account(&self, account: &str) -> Option<String> {
        if let Ok(Some(token)) = self.store.get(account) {
            return Some(token);
        }
        self.file.as_ref().and_then(|f| f.get(account))
    }

    pub fn has_account(&self, account: &str) -> bool {
        matches!(self.store.has(account), Ok(true)) || self.file.as_ref().is_some_and(|f| f.has(account))
    }

    /// Stores a secret under `account` (None or "" removes it), see `set`
    pub fn set_account(&self, account: &str, label: &str, token: Option<&str>) -> Result<Option<TokenPlace>> {
        let account = account.to_string();
        let _ = self.store.delete(&account);
        if let Some(file) = &self.file {
            file.delete(&account)?;
        }
        let Some(token) = token.filter(|t| !t.is_empty()) else { return Ok(None) };
        let label = label.to_string();
        match self.store.set(&account, &label, token) {
            Ok(()) => Ok(Some(self.store.place())),
            Err(StoreError::Failed(e)) => Err(e),
            Err(StoreError::Unavailable) => match &self.file {
                Some(file) => file.set(&account, token).map(|()| Some(TokenPlace::File(file.path.clone()))),
                None => Err(Error::unsupported("Storing a token (no keychain)")),
            },
        }
    }
}

/// No system store (neither macOS nor Linux): every token goes to the file
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
struct NoStore;

#[cfg(not(any(target_os = "macos", target_os = "linux")))]
impl SecretStore for NoStore {
    fn place(&self) -> TokenPlace {
        TokenPlace::Keyring
    }
    fn get(&self, _: &str) -> std::result::Result<Option<String>, StoreError> {
        Err(StoreError::Unavailable)
    }
    fn has(&self, _: &str) -> std::result::Result<bool, StoreError> {
        Err(StoreError::Unavailable)
    }
    fn set(&self, _: &str, _: &str, _: &str) -> std::result::Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
    fn delete(&self, _: &str) -> std::result::Result<(), StoreError> {
        Err(StoreError::Unavailable)
    }
}

/// The tokens of this computer's store – what the CLI and the core API use
pub enum SourceTokens {}

impl SourceTokens {
    pub const SERVICE: &'static str = "app.omaflake.recipe-source";

    pub fn token(source: &RecipeSource) -> Option<String> {
        Tokens::system().token(source)
    }

    /// A token is stored for this source. Reads only the item's attributes, not the secret: no Keychain prompt, also
    /// for an item another program (an older omaflake) wrote.
    pub fn has(source: &RecipeSource) -> bool {
        Tokens::system().has(source)
    }

    /// Stores the token (None or "" removes it); where it went (`TokenPlace::note` says when it is a file)
    pub fn set(token: Option<&str>, source: &RecipeSource) -> Result<Option<TokenPlace>> {
        Tokens::system().set(token, source)
    }
}

// MARK: macOS

#[cfg(target_os = "macos")]
mod keychain {
    use super::{SecretStore, SourceTokens, StoreError, TokenPlace};
    use crate::vm_err;

    pub struct Keychain;

    impl SecretStore for Keychain {
        fn place(&self) -> TokenPlace {
            TokenPlace::Keychain
        }

        fn get(&self, account: &str) -> Result<Option<String>, StoreError> {
            Ok(security_framework::passwords::get_generic_password(SourceTokens::SERVICE, account)
                .ok()
                .and_then(|data| String::from_utf8(data).ok()))
        }

        fn has(&self, account: &str) -> Result<bool, StoreError> {
            use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
            Ok(ItemSearchOptions::new()
                .class(ItemClass::generic_password())
                .service(SourceTokens::SERVICE)
                .account(account)
                .load_attributes(true)
                .limit(Limit::Max(1))
                .search()
                .is_ok_and(|found| !found.is_empty()))
        }

        fn set(&self, account: &str, label: &str, token: &str) -> Result<(), StoreError> {
            use security_framework::passwords::{set_generic_password_options, PasswordOptions};
            let mut options = PasswordOptions::new_generic_password(SourceTokens::SERVICE, account);
            options.set_label(label);
            set_generic_password_options(token.as_bytes(), options)
                .map_err(|e| StoreError::Failed(vm_err!("Cannot store the token in the Keychain ({})", e.code())))
        }

        fn delete(&self, account: &str) -> Result<(), StoreError> {
            let _ = security_framework::passwords::delete_generic_password(SourceTokens::SERVICE, account);
            Ok(())
        }
    }
}

// MARK: Linux

#[cfg(target_os = "linux")]
mod secret_service_store {
    use std::collections::HashMap;
    use std::sync::mpsc;
    use std::time::Duration;

    use secret_service::blocking::SecretService;
    use secret_service::{EncryptionType, Error as SsError};

    use super::{SecretStore, SourceTokens, StoreError, TokenPlace};
    use crate::vm_err;

    /// The Secret Service of the session bus. Every call runs on its own thread with a time limit: the blocking API
    /// must not run inside an async runtime (the Linux app's), and a keyring that never answers is treated as none.
    pub struct SecretServiceStore;

    /// Reading: a few seconds; storing may wait for the user to unlock the keyring
    const READ: Duration = Duration::from_secs(5);
    const WRITE: Duration = Duration::from_secs(60);

    fn attributes(account: &str) -> HashMap<&str, &str> {
        HashMap::from([("service", SourceTokens::SERVICE), ("account", account)])
    }

    /// A session bus can be there at all: an address, or the systemd user bus socket
    fn session_bus() -> bool {
        std::env::var_os("DBUS_SESSION_BUS_ADDRESS").is_some_and(|a| !a.is_empty())
            || std::env::var_os("XDG_RUNTIME_DIR")
                .is_some_and(|d| !d.is_empty() && std::path::Path::new(&d).join("bus").exists())
    }

    fn mapped(e: SsError) -> StoreError {
        match e {
            SsError::Prompt => StoreError::Failed(vm_err!("The keyring was not unlocked – the token was not stored")),
            SsError::Crypto(_) | SsError::Zvariant(_) => StoreError::Failed(vm_err!("Keyring: {e}")),
            // No service, no session, no collection, locked without a way to ask: as if there were no keyring
            _ => StoreError::Unavailable,
        }
    }

    fn run<T: Send + 'static>(
        limit: Duration,
        f: impl FnOnce(&SecretService) -> Result<T, SsError> + Send + 'static,
    ) -> Result<T, StoreError> {
        if !session_bus() {
            return Err(StoreError::Unavailable);
        }
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let result = SecretService::connect(EncryptionType::Dh).and_then(|ss| f(&ss));
            let _ = tx.send(result);
        });
        match rx.recv_timeout(limit) {
            Ok(result) => result.map_err(mapped),
            Err(_) => Err(StoreError::Unavailable),
        }
    }

    impl SecretStore for SecretServiceStore {
        fn place(&self) -> TokenPlace {
            TokenPlace::Keyring
        }

        fn get(&self, account: &str) -> Result<Option<String>, StoreError> {
            let account = account.to_string();
            run(READ, move |ss| {
                let found = ss.search_items(attributes(&account))?;
                let Some(item) = found.unlocked.first().or(found.locked.first()) else { return Ok(None) };
                if item.is_locked()? {
                    item.unlock()?;
                }
                Ok(String::from_utf8(item.get_secret()?).ok())
            })
        }

        fn has(&self, account: &str) -> Result<bool, StoreError> {
            let account = account.to_string();
            run(READ, move |ss| {
                let found = ss.search_items(attributes(&account))?;
                Ok(!found.unlocked.is_empty() || !found.locked.is_empty())
            })
        }

        fn set(&self, account: &str, label: &str, token: &str) -> Result<(), StoreError> {
            let (account, label, token) = (account.to_string(), label.to_string(), token.to_string());
            run(WRITE, move |ss| {
                let collection = ss.get_default_collection().or_else(|_| ss.get_any_collection())?;
                if collection.is_locked()? {
                    collection.unlock()?;
                }
                collection.create_item(&label, attributes(&account), token.as_bytes(), true, "text/plain")?;
                Ok(())
            })
        }

        fn delete(&self, account: &str) -> Result<(), StoreError> {
            let account = account.to_string();
            run(READ, move |ss| {
                let found = ss.search_items(attributes(&account))?;
                for item in found.unlocked.iter().chain(found.locked.iter()) {
                    item.delete()?;
                }
                Ok(())
            })
        }
    }
}

/// The JSON object of tokens.json, for the schema test
#[cfg(test)]
pub(crate) fn sample_file() -> Vec<u8> {
    let mut tokens = BTreeMap::new();
    tokens.insert("github:o/r".to_string(), "t".to_string());
    TokenFile::FILE_FORMAT.encode(&TokensFile { tokens }).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::Value;
    use std::cell::RefCell;
    use std::collections::HashMap;
    use std::os::unix::fs::PermissionsExt;

    /// A keyring in memory; `reachable` false = no Secret Service
    struct Fake {
        reachable: bool,
        items: RefCell<HashMap<String, (String, String)>>,
    }

    impl Fake {
        fn new(reachable: bool) -> Fake {
            Fake { reachable, items: RefCell::new(HashMap::new()) }
        }
        fn up(&self) -> std::result::Result<(), StoreError> {
            if self.reachable {
                Ok(())
            } else {
                Err(StoreError::Unavailable)
            }
        }
    }

    impl SecretStore for std::rc::Rc<Fake> {
        fn place(&self) -> TokenPlace {
            TokenPlace::Keyring
        }
        fn get(&self, account: &str) -> std::result::Result<Option<String>, StoreError> {
            self.up()?;
            Ok(self.items.borrow().get(account).map(|i| i.1.clone()))
        }
        fn has(&self, account: &str) -> std::result::Result<bool, StoreError> {
            self.up()?;
            Ok(self.items.borrow().contains_key(account))
        }
        fn set(&self, account: &str, label: &str, token: &str) -> std::result::Result<(), StoreError> {
            self.up()?;
            self.items.borrow_mut().insert(account.into(), (label.into(), token.into()));
            Ok(())
        }
        fn delete(&self, account: &str) -> std::result::Result<(), StoreError> {
            self.up()?;
            self.items.borrow_mut().remove(account);
            Ok(())
        }
    }

    fn source() -> RecipeSource {
        RecipeSource::parse("https://github.com/someone/private-recipes", true).unwrap()
    }

    fn tokens(fake: &std::rc::Rc<Fake>, dir: &Path) -> Tokens {
        Tokens { store: Box::new(fake.clone()), file: Some(TokenFile::under(Some(dir.to_path_buf()), dir)) }
    }

    #[test]
    fn the_keyring_first() {
        let dir = tempfile::tempdir().unwrap();
        let fake = std::rc::Rc::new(Fake::new(true));
        let t = tokens(&fake, dir.path());
        let s = source();
        assert!(!t.has(&s) && t.token(&s).is_none());
        assert_eq!(t.set(Some("ghp_x"), &s).unwrap(), Some(TokenPlace::Keyring));
        assert!(t.has(&s));
        assert_eq!(t.token(&s).as_deref(), Some("ghp_x"));
        let items = fake.items.borrow();
        let (label, _) = &items[&s.id()];
        assert_eq!(label, &format!("omaflake recipe source {}", s.display_name()));
        drop(items);
        assert!(!dir.path().join("omaflake/tokens.json").exists());
        assert_eq!(t.set(None, &s).unwrap(), None);
        assert!(!t.has(&s) && fake.items.borrow().is_empty());
    }

    #[test]
    fn without_a_keyring_a_private_file() {
        let dir = tempfile::tempdir().unwrap();
        let fake = std::rc::Rc::new(Fake::new(false));
        let t = tokens(&fake, dir.path());
        let s = source();
        let file = dir.path().join("omaflake/tokens.json");
        let place = t.set(Some("ghp_y"), &s).unwrap().unwrap();
        assert_eq!(place, TokenPlace::File(file.clone()));
        assert!(place.note().unwrap().contains("unencrypted"), "{:?}", place.note());
        assert_eq!(std::fs::metadata(&file).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(std::fs::metadata(file.parent().unwrap()).unwrap().permissions().mode() & 0o777, 0o700);
        assert!(t.has(&s));
        assert_eq!(t.token(&s).as_deref(), Some("ghp_y"));
        let written: Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(written["$schema"], "https://omaflake.dev/schema/tokens/v1.json");
        assert_eq!(written["tokens"][s.id()], "ghp_y");
        // another source's token stays when one is removed; the file goes with the last
        let other = RecipeSource::parse("https://github.com/someone/more", true).unwrap();
        t.set(Some("ghp_z"), &other).unwrap();
        t.set(None, &s).unwrap();
        assert!(!t.has(&s) && t.has(&other));
        t.set(None, &other).unwrap();
        assert!(!file.exists());
    }

    /// A token moves out of the file once the keyring is there (stored again)
    #[test]
    fn never_in_both() {
        let dir = tempfile::tempdir().unwrap();
        let s = source();
        let off = std::rc::Rc::new(Fake::new(false));
        tokens(&off, dir.path()).set(Some("old"), &s).unwrap();
        let on = std::rc::Rc::new(Fake::new(true));
        let t = tokens(&on, dir.path());
        assert_eq!(t.token(&s).as_deref(), Some("old"));
        assert_eq!(t.set(Some("new"), &s).unwrap(), Some(TokenPlace::Keyring));
        assert!(!dir.path().join("omaflake/tokens.json").exists());
        assert_eq!(t.token(&s).as_deref(), Some("new"));
    }

    #[test]
    fn the_files_place() {
        let home = Path::new("/home/u");
        assert_eq!(TokenFile::under(None, home).path, Path::new("/home/u/.config/omaflake/tokens.json"));
        assert_eq!(TokenFile::under(Some("rel".into()), home).path, Path::new("/home/u/.config/omaflake/tokens.json"));
        assert_eq!(TokenFile::under(Some("/x".into()), home).path, Path::new("/x/omaflake/tokens.json"));
    }

    /// A newer tokens.json (a newer omaflake's) is not overwritten
    #[test]
    fn a_newer_file_stays() {
        let dir = tempfile::tempdir().unwrap();
        let file = TokenFile::under(Some(dir.path().to_path_buf()), dir.path());
        std::fs::create_dir_all(file.path.parent().unwrap()).unwrap();
        std::fs::write(&file.path, br#"{"$schema":"https://omaflake.dev/schema/tokens/v9.json","tokens":{}}"#).unwrap();
        assert!(file.set("a", "b").is_err());
    }
}
