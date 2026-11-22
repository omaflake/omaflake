// SPDX-License-Identifier: AGPL-3.0-or-later
//! Nix in the user's shells (plan B160): omaflake installs Nix with `--no-modify-profile` and puts the lines that load
//! Nix into the user's own ~/.zshrc (and ~/.bash_profile for bash users) – a marked block, never /etc. Only where no
//! shell file loads Nix yet (nix-darwin's /etc/static, the installer's lines, home-manager, own dotfiles); the Doctor
//! shows it and removes it. omaflake itself finds Nix by its paths (`Prerequisites::locate_nix`).

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{IoContext, Result};
use crate::platform::home_dir;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ShellState {
    /// omaflake's block is in the user's shell file
    Added,
    /// Another shell file loads Nix already
    Elsewhere,
    /// No shell file loads Nix
    Missing,
}

#[derive(Debug, Clone)]
pub struct ShellSetup {
    pub home: PathBuf,
    /// System-wide files that may load Nix (/etc/zshrc, …)
    pub system: Vec<PathBuf>,
    /// The user's login shell (`$SHELL`)
    pub shell: String,
}

impl ShellSetup {
    pub const START: &'static str = "# omaflake: Nix";
    pub const END: &'static str = "# omaflake: end";
    /// What the Nix installer writes into /etc/zshrc, here for the user's file
    pub const BODY: &'static str = "if [ -e '/nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh' ]; then\n  . '/nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh'\nfi";

    pub fn current() -> Self {
        ShellSetup {
            home: home_dir(),
            system: ["/etc/zshrc", "/etc/zshenv", "/etc/zprofile", "/etc/bashrc", "/etc/profile", "/etc/bash.bashrc"]
                .map(PathBuf::from)
                .to_vec(),
            shell: std::env::var("SHELL").unwrap_or_default(),
        }
    }

    fn user_files(&self) -> Vec<PathBuf> {
        [".zshrc", ".zshenv", ".zprofile", ".bash_profile", ".bashrc", ".profile"].map(|f| self.home.join(f)).to_vec()
    }

    /// The files omaflake writes its block into: ~/.zshrc, and ~/.bash_profile for a bash user or where it exists
    pub fn targets(&self) -> Vec<PathBuf> {
        let mut t = vec![self.home.join(".zshrc")];
        let bash = self.home.join(".bash_profile");
        if self.shell.ends_with("/bash") || bash.exists() {
            t.push(bash);
        }
        t
    }

    /// A shell file's text loads Nix: the installer's lines, nix-darwin's environment (/etc/static), a Nix profile
    pub fn loads_nix(text: &str) -> bool {
        text.lines().map(str::trim).filter(|l| !l.starts_with('#')).any(|l| {
            l.contains("nix-daemon.sh")
                || l.contains("/etc/static/")
                || l.contains("/nix/var/nix/profiles/default")
                || l.contains(".nix-profile")
                || l.contains("hm-session-vars")
        })
    }

    pub fn state(&self) -> ShellState {
        let read = |p: &Path| fs::read_to_string(p).unwrap_or_default();
        if self.targets().iter().any(|f| read(f).contains(Self::START)) {
            return ShellState::Added;
        }
        let others = self.system.iter().chain(self.user_files().iter()).map(|f| read(f)).collect::<Vec<_>>();
        if others.iter().any(|t| Self::loads_nix(t)) { ShellState::Elsewhere } else { ShellState::Missing }
    }

    fn block() -> String {
        format!("{}\n{}\n{}\n", Self::START, Self::BODY, Self::END)
    }

    /// Adds the block to each target that lacks it (creates ~/.zshrc when there is none); the files written
    pub fn add(&self) -> Result<Vec<PathBuf>> {
        let mut written = Vec::new();
        for file in self.targets() {
            let text = fs::read_to_string(&file).unwrap_or_default();
            if text.contains(Self::START) {
                continue;
            }
            let mut new = text.clone();
            if !new.is_empty() && !new.ends_with('\n') {
                new.push('\n');
            }
            if !new.is_empty() {
                new.push('\n');
            }
            new.push_str(&Self::block());
            fs::write(&file, new).with_path(&file)?;
            written.push(file);
        }
        Ok(written)
    }

    /// Removes omaflake's block (start to end marker, and one blank line before it) from every target
    pub fn remove(&self) -> Result<Vec<PathBuf>> {
        let mut written = Vec::new();
        for file in self.targets() {
            let Ok(text) = fs::read_to_string(&file) else { continue };
            let (Some(start), Some(end)) = (text.find(Self::START), text.find(Self::END)) else { continue };
            if end < start {
                continue;
            }
            let mut from = start;
            if text[..from].ends_with("\n\n") {
                from -= 1;
            }
            let mut to = end + Self::END.len();
            if text[to..].starts_with('\n') {
                to += 1;
            }
            let new = format!("{}{}", &text[..from], &text[to..]);
            fs::write(&file, new).with_path(&file)?;
            written.push(file);
        }
        Ok(written)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn setup(shell: &str) -> (tempfile::TempDir, ShellSetup) {
        let t = tempfile::tempdir().unwrap();
        let s =
            ShellSetup { home: t.path().join("home"), system: vec![t.path().join("etc-zshrc")], shell: shell.into() };
        fs::create_dir_all(&s.home).unwrap();
        (t, s)
    }

    #[test]
    fn adds_once_and_removes_cleanly() {
        let (_t, s) = setup("/bin/zsh");
        let zshrc = s.home.join(".zshrc");
        fs::write(&zshrc, "export EDITOR=vi").unwrap();
        assert_eq!(s.state(), ShellState::Missing);
        assert_eq!(s.add().unwrap(), std::slice::from_ref(&zshrc));
        assert!(s.add().unwrap().is_empty());
        assert_eq!(s.state(), ShellState::Added);
        let text = fs::read_to_string(&zshrc).unwrap();
        assert!(text.starts_with("export EDITOR=vi\n\n# omaflake: Nix\n"));
        assert!(text.ends_with("# omaflake: end\n"));
        s.remove().unwrap();
        assert_eq!(fs::read_to_string(&zshrc).unwrap(), "export EDITOR=vi\n");
    }

    #[test]
    fn creates_zshrc_and_writes_bash_profile_for_bash_users() {
        let (_t, s) = setup("/bin/bash");
        let written = s.add().unwrap();
        assert_eq!(written, [s.home.join(".zshrc"), s.home.join(".bash_profile")]);
        assert_eq!(fs::read_to_string(s.home.join(".zshrc")).unwrap(), ShellSetup::block());
    }

    #[test]
    fn nix_loaded_elsewhere_is_left_alone() {
        let (_t, s) = setup("/bin/zsh");
        fs::write(
            &s.system[0],
            "# Nix\nif [ -e '/nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh' ]; then\nfi\n",
        )
        .unwrap();
        assert_eq!(s.state(), ShellState::Elsewhere);
        assert!(ShellSetup::loads_nix("if test -e /etc/static/zshrc; then . /etc/static/zshrc; fi"));
        assert!(!ShellSetup::loads_nix("# . /nix/var/nix/profiles/default/etc/profile.d/nix-daemon.sh"));
    }
}
