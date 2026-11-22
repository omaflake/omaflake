// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaflake's own SSH key (plan B160, reverses B16): `<data>/ssh/id_ed25519` without a passphrase – created by
//! omaflake, no Terminal. It goes into every guest next to the user's ~/.ssh keys, and every `ssh` omaflake runs
//! offers it first (`-i`), so image builds and macOS bases need neither a key in ~/.ssh nor an ssh-agent. The user's
//! keys still work as before (ssh tries them after this one).

use std::fs;
use std::path::PathBuf;

use crate::error::{IoContext, Result};
use crate::paths::Paths;
use crate::shell::{RunOptions, Shell};

#[derive(Debug, Clone)]
pub struct SshIdentity {
    pub dir: PathBuf,
}

impl SshIdentity {
    pub fn new(paths: &Paths) -> Self {
        SshIdentity { dir: paths.root.join("ssh") }
    }

    /// The data folder's (VM_HOME, else the one config.json names)
    pub fn standard() -> Self {
        Self::new(&Paths::standard())
    }

    pub fn key(&self) -> PathBuf {
        self.dir.join("id_ed25519")
    }

    pub fn public_key_file(&self) -> PathBuf {
        self.dir.join("id_ed25519.pub")
    }

    pub fn exists(&self) -> bool {
        self.key().is_file() && self.public_key_file().is_file()
    }

    /// Its public key, trimmed
    pub fn public_key(&self) -> Option<String> {
        let text = fs::read_to_string(self.public_key_file()).ok()?;
        let key = text.trim();
        (key.starts_with("ssh-") || key.starts_with("ecdsa-")).then(|| key.to_string())
    }

    /// Creates the key once (ed25519, no passphrase, comment "omaflake"); the folder only for its user
    pub fn ensure(&self) -> Result<()> {
        if self.exists() {
            return Ok(());
        }
        fs::create_dir_all(&self.dir).with_path(&self.dir)?;
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&self.dir, fs::Permissions::from_mode(0o700)).with_path(&self.dir)?;
        }
        for f in [self.key(), self.public_key_file()] {
            if f.exists() {
                fs::remove_file(&f).with_path(&f)?;
            }
        }
        let key = self.key().to_string_lossy().into_owned();
        Shell::check(
            "/usr/bin/ssh-keygen",
            &["-q", "-t", "ed25519", "-N", "", "-C", "omaflake", "-f", &key],
            &RunOptions::default(),
        )?;
        Ok(())
    }

    /// `["-i", <key>]` for every `ssh` omaflake runs – nothing while there is no key
    pub fn arguments(&self) -> Vec<String> {
        if self.key().is_file() { vec!["-i".into(), self.key().to_string_lossy().into_owned()] } else { Vec::new() }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creates_once_and_offers_itself_to_ssh() {
        let t = tempfile::tempdir().unwrap();
        let id = SshIdentity { dir: t.path().join("ssh") };
        assert!(id.arguments().is_empty());
        assert_eq!(id.public_key(), None);
        id.ensure().unwrap();
        let first = id.public_key().unwrap();
        assert!(first.starts_with("ssh-ed25519 ") && first.ends_with(" omaflake"));
        id.ensure().unwrap();
        assert_eq!(id.public_key().unwrap(), first);
        assert_eq!(id.arguments(), ["-i".to_string(), id.key().to_string_lossy().into_owned()]);
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(fs::metadata(&id.dir).unwrap().permissions().mode() & 0o777, 0o700);
    }
}
