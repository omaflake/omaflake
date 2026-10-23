// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of BuilderMaintenance.swift – work package F5. Done: constants, `shell_quote`, `nix_installer_url`.
//! Skeleton: the rest (`todo!`).
//!
//! Admin tasks around the Linux builder: apply the image flake's nix-darwin host (`#linux-builder`) and clean up
//! the builder's Nix store. Both need root – the app asks through macOS' administrator dialog (`AdminShell`), the CLI
//! through sudo. After Developer ID signing a privileged helper replaces the dialog (spec §11.5).
#![allow(unused_variables, dead_code)]

use std::path::{Path, PathBuf};

use crate::error::Result;
use crate::paths::Paths;

#[derive(Debug, Clone)]
pub struct BuilderMaintenance {
    pub paths: Paths,
}

impl BuilderMaintenance {
    /// Written by nix/hosts/linux-builder.nix: the running nix-darwin system comes from omaflake's flake
    pub const MARKER: &'static str = "/etc/omaflake/linux-builder";

    /// Nix with the official installer of the NixOS community (github.com/NixOS/nix-installer): creates the /nix
    /// volume, the build users and the daemon; `--no-confirm` because the app has asked already. Uninstall:
    /// `/nix/nix-installer uninstall`. Pinned: one release, its macOS binary checked against the release's SHA-256
    /// before it runs (plan B66). `OMAFLAKE_NIX_INSTALLER_URL` points to a mirror of the same file.
    pub const NIX_INSTALLER_VERSION: &'static str = "2.35.3";
    pub const NIX_INSTALLER_SHA256: &'static str = "c4991562a71a7ef2dc52d9760545c8541db4a468bbcc91dcafd57175638bf17e";

    /// Unused store paths in the builder (everything can be rebuilt or downloaded again), with free space before/after.
    /// `ssh linux-builder` works for root only (the key in /etc/nix is root's).
    pub const CLEAN_UP_COMMAND: &'static str =
        "ssh linux-builder 'df -h / | tail -1; nix-store --gc 2>&1 | tail -1; df -h / | tail -1'";

    pub fn new(paths: Paths) -> Self {
        BuilderMaintenance { paths }
    }

    /// Is the current nix-darwin system omaflake's `#linux-builder`? If not, applying replaces the user's own
    /// nix-darwin configuration – the app says so before it asks.
    pub fn system_is_omaflakes(&self) -> bool {
        Path::new(Self::MARKER).exists()
    }

    /// logs/linux-builder.log
    pub fn log(&self) -> PathBuf {
        self.paths.logs().join("linux-builder.log")
    }

    /// Step 1 of `darwin-rebuild switch`, as the user: builds the image flake's builder host and returns the system's
    /// store path. Root cannot do this – macOS' privacy protection keeps it out of ~/Documents etc. (plan B40), and it
    /// would write a root-owned flake.lock into the checkout. Swift default `host`: "linux-builder".
    /// `nix build --no-link --print-out-paths --no-write-lock-file --extra-experimental-features "nix-command flakes"
    /// path:<flake>#darwinConfigurations.<host>.system`; failure: "Building the Linux builder host failed:\n<last 1500
    /// characters of stderr>" (the stream's stderr is empty – Swift quirk kept, OPEN: show the streamed lines instead).
    pub fn build_system(&self, host: &str, on_line: &(dyn Fn(&str) + Send + Sync)) -> Result<String> {
        todo!("port from BuilderMaintenance.swift")
    }

    /// A nix-darwin system in the store (the only thing root gets to activate)
    pub(crate) fn is_system(path: &str) -> bool {
        todo!("port from BuilderMaintenance.swift")
    }

    /// Step 2 of `darwin-rebuild switch`, as root: set the system profile and activate (as darwin-rebuild does).
    pub fn activate_command(system: &str) -> String {
        todo!("port from BuilderMaintenance.swift")
    }

    // MARK: Setup by click (spec §11.5)

    /// `OMAFLAKE_NIX_INSTALLER_URL`, else the pinned release's aarch64-darwin binary on GitHub
    pub fn nix_installer_url() -> String {
        std::env::var("OMAFLAKE_NIX_INSTALLER_URL").unwrap_or_else(|_| {
            format!(
                "https://github.com/NixOS/nix-installer/releases/download/{}/nix-installer-aarch64-darwin",
                Self::NIX_INSTALLER_VERSION
            )
        })
    }

    /// Downloads the installer with curl into a temporary folder, checks its SHA-256 with `shasum -a 256 -c -`, runs
    /// `install --no-confirm` (Swift `installNixCommand`)
    pub fn install_nix_command() -> String {
        todo!("port from BuilderMaintenance.swift")
    }

    /// First activation of nix-darwin: it refuses to take over /etc files it did not write (the Nix installer's
    /// nix.conf and shell rc files) – move them aside as docs/linux-builder.md says (rename, never delete), then
    /// set the system profile and activate.
    pub fn first_activation_command(system: &str) -> String {
        todo!("port from BuilderMaintenance.swift")
    }

    /// Wraps a command for a root shell: Nix tools on PATH, root's HOME, output appended to `log` (created by the
    /// user first, so it stays theirs); a header line "=== <ISO 8601 time> <command>"; `set -o pipefail`.
    pub fn root_script(&self, command: &str) -> Result<String> {
        todo!("port from BuilderMaintenance.swift")
    }
}

/// Runs a shell command as root after macOS' administrator dialog (AppleScript `do shell script … with administrator
/// privileges`). The command travels as an argument, never spliced into AppleScript source.
pub enum AdminShell {}

impl AdminShell {
    /// Blocks until the command ends; returns its output (`\r` turned into `\n`). The user cancelling the dialog
    /// (osascript "(-128)") is an error with code `codes::CANCELLED` (Swift `AdminShell.Cancelled`).
    /// macOS only; elsewhere `Error::unsupported`.
    pub fn run(command: &str, prompt: &str) -> Result<String> {
        todo!("port from BuilderMaintenance.swift")
    }
}

/// Single-quotes a word for /bin/sh.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shell_quoting() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }
}
