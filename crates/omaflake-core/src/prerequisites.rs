// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Prerequisites.swift – work package F5. Done: the types and constants. Skeleton: the checks (`todo!`).
//!
//! What the user's Mac brings (spec §11.2): detected without elevated rights, shared by `omaf doctor` and the app's
//! Requirements page. Nothing here installs anything – `install` is a command the user runs in Terminal.
//!
//! Not 1:1: `host()` read `ProcessInfo.operatingSystemVersion`; Rust: OPEN – `sw_vers -productVersion` on macOS
//! (a Linux host is "missing" until Linux hosts are supported). The IPSW check asks `IpswDownloader::resume_file_for`
//! (`.partial`, see ipsw_downloader.rs) instead of URLSession's `.resume`.
#![allow(unused_variables, dead_code)]

use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::ipsw_downloader::MacOsRelease;
use crate::paths::{Paths, Settings};

/// Swift `Prerequisite.Item`; serialized as Swift's raw value (camelCase: "nixDarwin", "linuxBuilder", …)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrerequisiteItem {
    // what the Mac brings
    Host,
    Nix,
    Flakes,
    NixDarwin,
    LinuxBuilder,
    Qemu,
    Flake,
    SshKey,
    Git,
    // preflight NixOS: builder VM answering, recipes evaluating, space in the Nix store
    BuilderRunning,
    FlakeEval,
    NixSpace,
    // preflight macOS: IPSW from Apple, base, space for installation and images
    Ipsw,
    MacBase,
    MacSpace,
}

impl PrerequisiteItem {
    /// Swift `rawValue` (also the `id`)
    pub fn as_str(self) -> &'static str {
        match self {
            PrerequisiteItem::Host => "host",
            PrerequisiteItem::Nix => "nix",
            PrerequisiteItem::Flakes => "flakes",
            PrerequisiteItem::NixDarwin => "nixDarwin",
            PrerequisiteItem::LinuxBuilder => "linuxBuilder",
            PrerequisiteItem::Qemu => "qemu",
            PrerequisiteItem::Flake => "flake",
            PrerequisiteItem::SshKey => "sshKey",
            PrerequisiteItem::Git => "git",
            PrerequisiteItem::BuilderRunning => "builderRunning",
            PrerequisiteItem::FlakeEval => "flakeEval",
            PrerequisiteItem::NixSpace => "nixSpace",
            PrerequisiteItem::Ipsw => "ipsw",
            PrerequisiteItem::MacBase => "macBase",
            PrerequisiteItem::MacSpace => "macSpace",
        }
    }
}

/// Swift `Prerequisite.Status`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrerequisiteStatus {
    Ok,
    Limited,
    Missing,
}

/// One check's result
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Prerequisite {
    pub item: PrerequisiteItem,
    pub status: PrerequisiteStatus,
    /// What was found (path, version) or what is missing
    pub detail: String,
    /// What to do about it (empty when ok)
    pub hint: String,
    /// Command for Terminal that installs it (official installers only), None if there is none
    pub install: Option<String>,
    /// Space checks: free and needed bytes (for a localized display; `detail` has them in English)
    pub free_bytes: Option<i64>,
    pub needed_bytes: Option<i64>,
    /// macOS base: its guest user, and the recipes' macUser when it differs
    pub base_user: Option<String>,
    pub expected_user: Option<String>,
}

impl Prerequisite {
    pub fn new(
        item: PrerequisiteItem,
        status: PrerequisiteStatus,
        detail: &str,
        hint: &str,
        install: Option<String>,
    ) -> Self {
        Prerequisite {
            item,
            status,
            detail: detail.into(),
            hint: hint.into(),
            install,
            free_bytes: None,
            needed_bytes: None,
            base_user: None,
            expected_user: None,
        }
    }

    pub fn id(&self) -> &'static str {
        self.item.as_str()
    }

    /// The user can enter a path for it (`Settings.nix`, `Settings.qemu`, `Settings.flake`)
    pub fn settable_path(&self) -> bool {
        matches!(self.item, PrerequisiteItem::Nix | PrerequisiteItem::Qemu | PrerequisiteItem::Flake)
    }

    /// Needed for every feature; `limited` items still allow some
    pub fn required(&self) -> bool {
        self.item != PrerequisiteItem::Git
    }
}

#[derive(Debug, Clone)]
pub struct Prerequisites {
    pub paths: Paths,
}

impl Prerequisites {
    // MARK: Official install commands (typed into Terminal by the user's click)

    /// nix-darwin with omaflake's Linux builder host, in two stages (`omaf builder setup`; the app's "Set Up …")
    pub const INSTALL_NIX_DARWIN: &'static str = "omaf builder setup";
    /// omaflake's patched QEMU from the flake (spec §11.2)
    pub const INSTALL_QEMU: &'static str = "nix profile install github:omaflake/omaflake#qemu";
    pub const INSTALL_SSH_KEY: &'static str = "ssh-keygen -t ed25519";
    pub const INSTALL_GIT: &'static str = "xcode-select --install";

    /// Space the Nix store should have for building NixOS images
    pub const NIX_SPACE_NEEDED: i64 = 20_000_000_000;
    /// Space a macOS installation and one image need besides the IPSW
    pub const MAC_SPACE_NEEDED: i64 = 30_000_000_000;

    pub fn new(paths: Paths) -> Self {
        Prerequisites { paths }
    }

    /// The official installer of the NixOS community (also what the app runs through the administrator dialog):
    /// `sudo sh -c '<BuilderMaintenance::install_nix_command()>'`
    pub fn install_nix() -> String {
        todo!("port from Prerequisites.swift")
    }

    /// All checks, in the order shown: host, nix, flakes, nix-darwin, Linux builder, QEMU, flake, SSH key, git.
    pub fn check(&self) -> Vec<Prerequisite> {
        todo!("port from Prerequisites.swift")
    }

    // MARK: Checks

    pub(crate) fn host(&self) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn nix_item(&self, nix: Option<&str>) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn flakes(&self, nix: Option<&str>, config: &HashMap<String, String>) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn nix_darwin(&self) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn linux_builder(&self, nix: Option<&str>, config: &HashMap<String, String>) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn qemu(&self) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn flake(&self, settings: &Settings, nix: Option<&str>) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn ssh_key(&self) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn git(&self) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    // MARK: Preflight (before building images)

    pub fn nixos_preflight(&self) -> Vec<Prerequisite> {
        todo!("port from Prerequisites.swift")
    }

    /// `latest`: what Apple offers this Mac (`MacOsRelease::latest`), None when unknown (offline).
    pub fn macos_preflight(&self, latest: Option<&MacOsRelease>) -> Vec<Prerequisite> {
        todo!("port from Prerequisites.swift")
    }

    pub fn builder_running(&self) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn flake_eval(&self, settings: &Settings) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn ipsw(&self, latest: Option<&MacOsRelease>) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn mac_base(&self) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn space(&self, item: PrerequisiteItem, at: &Path, needed: i64) -> Prerequisite {
        todo!("port from Prerequisites.swift")
    }

    /// Host and port of the Linux builder from the SSH configuration nix-darwin writes (`Host linux-builder`);
    /// its key is root's, so only the SSH banner is checked. `configs` None = /etc/ssh/ssh_config.d/* and
    /// /etc/ssh/ssh_config.
    pub(crate) fn builder_ssh_target(configs: Option<&[String]>) -> Option<(String, u16)> {
        todo!("port from Prerequisites.swift")
    }

    // MARK: Detection helpers

    /// `nix`: `Settings.nix` (must be executable), then PATH and the usual Nix locations (`Shell::which`).
    pub fn locate_nix(settings: &Settings) -> Option<String> {
        todo!("port from Prerequisites.swift")
    }

    /// git that does not trigger the Command Line Tools install dialog: /usr/bin/git is only a shim that asks
    /// to install them when they are missing, so it counts only when `xcode-select -p` finds developer tools.
    pub fn locate_git() -> Option<String> {
        todo!("port from Prerequisites.swift")
    }

    /// `nix config show` as key → value
    pub(crate) fn nix_config(nix: &str) -> HashMap<String, String> {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn parse_config(text: &str) -> HashMap<String, String> {
        todo!("port from Prerequisites.swift")
    }

    pub(crate) fn flakes_enabled(config: &HashMap<String, String>) -> bool {
        todo!("port from Prerequisites.swift")
    }

    /// `builders` is a list separated by ";" or newlines, or "@<file>" with one machine per line (`read` reads it;
    /// Swift default: the file's contents)
    pub(crate) fn builder_lines(builders: &str, read: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
        todo!("port from Prerequisites.swift")
    }

    /// Machine line: `<uri> <systems, comma-separated> …` with aarch64-linux among the systems
    pub(crate) fn is_linux_builder(line: &str) -> bool {
        todo!("port from Prerequisites.swift")
    }
}
