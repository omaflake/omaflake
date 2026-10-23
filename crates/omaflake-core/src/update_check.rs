// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of UpdateCheck.swift – work package F3. Done: the feed type. Skeleton: the functions (`todo!`).
//!
//! Is there a newer omaflake? Reads a small JSON feed (spec §11.6, plan B68):
//!   {"version": "0.2.0", "url": "https://…/omaflake-0.2.0.zip", "sha256": "…", "notes": "…", "page": "https://…"}
//! The feed's address comes from the app's Info.plist (`OMUpdateFeedURL`, set with the first release) or
//! `OMAFLAKE_UPDATE_FEED`. This is the only network access the app makes on its own, and it can be switched off.
//! Installing the update itself comes with signed releases (M12).
//!
//! Not 1:1: Swift read `OMUpdateFeedURL` from the running bundle. The Rust core has no bundle – the app passes the
//! Info.plist value in (`feed_url(Some(…))`); the CLI passes None. `newer` is blocking (Swift: async).
#![allow(unused_variables, dead_code)]

use serde::{Deserialize, Serialize};

use crate::error::Result;

/// The feed (Swift `UpdateFeed`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateFeed {
    pub version: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

/// Swift `enum UpdateCheck` (namespace)
pub enum UpdateCheck {}

impl UpdateCheck {
    /// `OMAFLAKE_UPDATE_FEED`, else `info_plist_feed` (the app's `OMUpdateFeedURL`); None when empty or not a URL
    pub fn feed_url(info_plist_feed: Option<&str>) -> Option<String> {
        todo!("port from UpdateCheck.swift")
    }

    /// The feed's release when it is newer than `current`, None when up to date. GET with a 15 s timeout, no cache,
    /// `User-Agent: omaflake/<current>`; a status other than 200 is "Update feed answered <status>". `feed` may be a
    /// file:// URL (tests).
    pub fn newer(current: &str, feed: &str) -> Result<Option<UpdateFeed>> {
        todo!("port from UpdateCheck.swift")
    }

    /// "0.10.0" > "0.9.1"; a missing part counts as 0; anything after "-" (pre-release) is ignored
    pub fn is_newer(a: &str, than: &str) -> bool {
        todo!("port from UpdateCheck.swift")
    }
}
