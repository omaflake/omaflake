// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of UpdateCheck.swift – work package F3.
//!
//! Is there a newer omaflake? Reads a small JSON feed (spec §11.6, plan B68):
//!   {"version": "0.2.0", "url": "https://…/omaflake-0.2.0.zip", "sha256": "…", "notes": "…", "page": "https://…"}
//! The feed's address comes from the app's Info.plist (`OMUpdateFeedURL`, set with the first release) or
//! `OMAFLAKE_UPDATE_FEED`. This is the only network access the app makes on its own, and it can be switched off.
//! Installing the update itself comes with signed releases (M12).
//!
//! Not 1:1: Swift read `OMUpdateFeedURL` from the running bundle. The Rust core has no bundle – the app passes the
//! Info.plist value in (`feed_url(Some(…))`); the CLI passes None. `newer` is blocking (Swift: async).
use serde::{Deserialize, Serialize};

use std::time::Duration;

use crate::error::{codes, Result};
use crate::vm_err;

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
    /// `OMAFLAKE_UPDATE_FEED`, else `info_plist_feed` (the app's `OMUpdateFeedURL`); None when empty or not a URL.
    /// As in Swift, a set but empty `OMAFLAKE_UPDATE_FEED` means no feed (it does not fall back to the Info.plist).
    pub fn feed_url(info_plist_feed: Option<&str>) -> Option<String> {
        let raw = match std::env::var("OMAFLAKE_UPDATE_FEED") {
            Ok(v) => v,
            Err(_) => info_plist_feed?.to_string(),
        };
        let raw = raw.trim();
        let valid = !raw.is_empty() && !raw.chars().any(|c| c.is_whitespace() || c.is_control());
        valid.then(|| raw.to_string())
    }

    /// The feed's release when it is newer than `current`, None when up to date. GET with a 15 s timeout, no cache,
    /// `User-Agent: omaflake/<current>`; a status other than 200 is "Update feed answered <status>". `feed` may be a
    /// file:// URL (tests).
    pub fn newer(current: &str, feed: &str) -> Result<Option<UpdateFeed>> {
        let data = match feed.strip_prefix("file://") {
            // Swift's URLSession reads a file URL without an HTTP status
            Some(path) => {
                std::fs::read(path).map_err(|e| vm_err!(codes::NOT_FOUND, "{path}: {}", crate::error::os_text(&e)))?
            }
            None => {
                let agent: ureq::Agent = ureq::Agent::config_builder()
                    .timeout_global(Some(Duration::from_secs(15)))
                    .http_status_as_error(false)
                    .build()
                    .into();
                let network = |e: ureq::Error| vm_err!(codes::NETWORK, "Update feed: {e}");
                let mut response =
                    agent.get(feed).header("User-Agent", &format!("omaflake/{current}")).call().map_err(network)?;
                let status = response.status().as_u16();
                if status != 200 {
                    return Err(vm_err!(codes::NETWORK, "Update feed answered {status}"));
                }
                response.body_mut().read_to_vec().map_err(network)?
            }
        };
        let release: UpdateFeed = serde_json::from_slice(&data).map_err(|e| vm_err!("Update feed: {e}"))?;
        Ok(Self::is_newer(&release.version, current).then_some(release))
    }

    /// "0.10.0" > "0.9.1"; a missing part counts as 0; anything after "-" (pre-release) is ignored
    pub fn is_newer(a: &str, than: &str) -> bool {
        fn parts(v: &str) -> Vec<i64> {
            match v.split('-').find(|s| !s.is_empty()) {
                Some(main) => main.split('.').filter(|s| !s.is_empty()).map(|p| p.parse().unwrap_or(0)).collect(),
                None => Vec::new(),
            }
        }
        let (x, y) = (parts(a), parts(than));
        for i in 0..x.len().max(y.len()) {
            let l = x.get(i).copied().unwrap_or(0);
            let r = y.get(i).copied().unwrap_or(0);
            if l != r {
                return l > r;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;

    #[test]
    fn version_comparison() {
        assert!(UpdateCheck::is_newer("0.10.0", "0.9.1"));
        assert!(UpdateCheck::is_newer("0.2", "0.1.9"));
        assert!(!UpdateCheck::is_newer("0.2.0", "0.2"));
        assert!(!UpdateCheck::is_newer("0.2.0-beta", "0.2.0"));
    }

    #[test]
    fn feed_from_a_file() {
        let (_t, paths) = temp_paths();
        let feed = paths.root.join("feed.json");
        std::fs::write(
            &feed,
            r#"{"version":"9.0.0","url":"https://example.invalid/o.zip","sha256":null,"notes":"n","page":null}"#,
        )
        .unwrap();
        let url = format!("file://{}", feed.display());
        let release = UpdateCheck::newer("0.1", &url).unwrap();
        assert_eq!(release.map(|r| r.version).as_deref(), Some("9.0.0"));
        assert_eq!(UpdateCheck::newer("9.0.0", &url).unwrap(), None);
    }

    #[test]
    fn feed_url_from_the_info_plist() {
        if std::env::var_os("OMAFLAKE_UPDATE_FEED").is_some() {
            return; // the environment decides
        }
        assert_eq!(
            UpdateCheck::feed_url(Some("https://omaflake.dev/feed.json")).as_deref(),
            Some("https://omaflake.dev/feed.json")
        );
        assert_eq!(UpdateCheck::feed_url(Some("")), None);
        assert_eq!(UpdateCheck::feed_url(None), None);
    }
}
