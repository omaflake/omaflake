// SPDX-License-Identifier: AGPL-3.0-or-later
//! The app's own settings (not the core's – api.md §10: each UI keeps its own): stored per user in
//! `$XDG_CONFIG_HOME/omaflake/linux-app.json` (default `~/.config`).

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Prefs {
    /// The tray icon (StatusNotifierItem); with it, closing the window keeps the app running
    pub tray: bool,
}

impl Default for Prefs {
    fn default() -> Prefs {
        Prefs { tray: true }
    }
}

pub fn file() -> PathBuf {
    let config = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|c| !c.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(|| omaflake_core::platform::home_dir().join(".config"));
    config.join("omaflake").join("linux-app.json")
}

impl Prefs {
    pub fn load() -> Prefs {
        std::fs::read(file()).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
    }

    pub fn save(&self) {
        let f = file();
        let result = f
            .parent()
            .map(std::fs::create_dir_all)
            .unwrap_or(Ok(()))
            .and_then(|_| std::fs::write(&f, serde_json::to_vec_pretty(self).unwrap_or_default()));
        if let Err(e) = result {
            eprintln!("omaflake: {}: {e}", f.display());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tray_is_on_by_default_and_unknown_keys_are_ignored() {
        assert!(Prefs::default().tray);
        let p: Prefs = serde_json::from_str(r#"{"tray": false, "later": 1}"#).unwrap();
        assert!(!p.tray);
        let p: Prefs = serde_json::from_str("{}").unwrap();
        assert!(p.tray);
    }
}
