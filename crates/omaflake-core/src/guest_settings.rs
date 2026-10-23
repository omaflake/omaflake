// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of GuestSettings.swift – work package F3. Done: the types. Skeleton: the functions (`todo!`).
//!
//! The person-specific values of every recipe (spec §11.1): recipes never contain them, omaflake passes them in.
//! Written to `<data folder>/guest/settings.json` and handed to the flake as its `omaflake-guest` input
//! (`--override-input`, lock file untouched). A developer's `local/settings.nix` in the flake overrides single values.
//!
//! Sources: the Mac (user, full name, time zone, locale, keyboard, `~/.ssh/*.pub`), the macOS base (its guest user),
//! and the overrides in the app's settings (`Settings::guest`).
//!
//! Not 1:1 (no Foundation): the Mac's time zone, locale and keyboard come from `TimeZone.current`, `Locale.current`
//! and the HIToolbox preferences in Swift. Rust: OPEN – e.g. the `/etc/localtime` link, `defaults read -g
//! AppleLocale` and `defaults read com.apple.HIToolbox AppleSelectedInputSources` on macOS; `$TZ`/`$LANG`/localectl
//! on Linux.
#![allow(unused_variables, dead_code)]

use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::Result;
use crate::image_store::ImageStore;
use crate::paths::Paths;

/// guest/settings.json. Keys: user, fullName, email, timeZone, locale, keyboard, macUser, authorizedKeys (None left
/// out). Written like Swift's JSONEncoder (`json::to_pretty(…, escape_slashes: true)`), only when the bytes change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestSettings {
    pub user: String,
    #[serde(rename = "fullName")]
    pub full_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(rename = "timeZone")]
    pub time_zone: String,
    pub locale: String,
    pub keyboard: String,
    /// Guest user of macOS VMs – fixed by the base; None = same as `user`
    #[serde(rename = "macUser", default, skip_serializing_if = "Option::is_none")]
    pub mac_user: Option<String>,
    #[serde(rename = "authorizedKeys")]
    pub authorized_keys: Vec<String>,
}

impl GuestSettings {
    /// The Mac's own values, without overrides (shown as defaults in the app): `platform::user_name`,
    /// `platform::full_user_name`, the time zone, `mac_locale`, `mac_keyboard`, `MacBaseInstaller::default_public_keys`
    pub fn mac() -> GuestSettings {
        todo!("port from GuestSettings.swift")
    }

    /// What the flake gets for this Mac now: `mac()` with the overrides of `Settings::guest` (non-empty values; an
    /// empty full name counts) and the newest base's guest user as `mac_user`
    pub fn current(paths: &Paths) -> GuestSettings {
        todo!("port from GuestSettings.swift")
    }

    /// Writes `guest/settings.json` (only when the content changed) and returns the folder.
    pub fn write(paths: &Paths) -> Result<PathBuf> {
        todo!("port from GuestSettings.swift")
    }

    /// "de_DE.UTF-8" from the Mac's language and region (region missing: "US" for English, else the language uppercased)
    pub(crate) fn mac_locale() -> String {
        todo!("port from GuestSettings.swift")
    }

    /// Keyboard layout of the Mac as an XKB/console name: "de", "us", … (the language as a fallback, "us" for English).
    pub(crate) fn mac_keyboard() -> String {
        todo!("port from GuestSettings.swift")
    }

    /// "com.apple.keylayout.German" → "de"; None when unknown (exact name first, then a known name it starts with)
    pub(crate) fn keyboard_from_input_source(id: &str) -> Option<String> {
        todo!("port from GuestSettings.swift")
    }
}

/// Overrides for the guest values, from the app's settings (config.json `guest`). Keys: user, fullName, email,
/// timeZone, locale, keyboard (None left out).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestOverrides {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(rename = "fullName", default, skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(rename = "timeZone", default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard: Option<String>,
}

impl ImageStore {
    /// Arguments for a `nix` call on the image flake: the guest values as its `omaflake-guest` input and the recipes
    /// of the sources as `omaflake-recipes` (RecipeLibrary::stage), without writing the lock file (it would get the
    /// data folder's paths). For a local folder an input is passed only when its flake.nix declares it.
    /// `["--override-input", "omaflake-guest", "path:<dir>", "--override-input", "omaflake-recipes", "path:<dir>",
    /// "--no-write-lock-file"]`, or nothing.
    pub fn guest_arguments(&self, flake: &str) -> Vec<String> {
        todo!("port from GuestSettings.swift")
    }
}
