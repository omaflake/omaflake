// SPDX-License-Identifier: AGPL-3.0-or-later
//! Does the Linux builder accept this Mac's builds? (plan B175)
//!
//! A builder whose disk holds a damaged Nix store can stop trusting the Mac: its `/etc/nix/nix.conf` no longer says
//! `trusted-users = root builder`, `nix store info` answers `"trusted": false`, and every input the Mac built itself
//! (unsigned) is refused – "cannot add path … because it lacks a signature by a trusted key". Every NixOS image build
//! fails then. Found with nix-darwin's builder after 0.7.5; how the store got damaged is OPEN (its disk survives
//! restarts, `ephemeral = false`).
//!
//! - **Recognize:** a failed image build with that message notes it (`<data>/builder/refused`); the Doctor's
//!   "Linux builder running" then checks again (`probe`) and stays red while the builder refuses.
//! - **Check:** `probe` builds a tiny Linux derivation whose input was just built on the Mac – unsigned, so the
//!   builder only takes it when it trusts the Mac. It runs through the Nix daemon as the user, for nix-darwin's builder
//!   too (its key in /etc/nix is root's).
//! - **Repair:** a new disk – the builder starts with a fresh store (`repair`). omaflake's own builder as the user;
//!   nix-darwin's through the administrator dialog (`DARWIN_NEW_DISK`).

use std::fs;
use std::path::PathBuf;

use crate::error::{IoContext, Result, codes};
use crate::linux_builder::LinuxBuilder;
use crate::paths::{Paths, Settings};
use crate::prerequisites::Prerequisites;
use crate::shell::{RunOptions, Shell};
use crate::vm_err;

/// What `probe` found
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Trust {
    /// The builder built the probe: it takes the Mac's unsigned paths
    Accepted,
    /// It refused the probe's input – "lacks a signature by a trusted key"
    Refused,
    /// The probe failed for another reason (the builder does not run, no Nix, …): its last lines
    Failed(String),
}

#[derive(Debug, Clone)]
pub struct BuilderHealth {
    pub paths: Paths,
}

impl BuilderHealth {
    /// Nix's message when a store refuses an unsigned path from a client it does not trust
    pub const REFUSED: &'static str = "lacks a signature by a trusted key";

    /// The probe: an aarch64-darwin input built on the Mac (input-addressed, unsigned) and an aarch64-linux derivation
    /// that reads it. The nonce makes both new every time – an input the builder has already is not copied.
    pub const PROBE: &'static str = r#"
      let
        nonce = builtins.getEnv "OMAFLAKE_PROBE_NONCE";
        input = derivation {
          name = "omaflake-builder-probe-input"; system = "aarch64-darwin"; builder = "/bin/sh";
          args = [ "-c" "echo ${nonce} > $out" ];
        };
      in derivation {
        name = "omaflake-builder-probe"; system = "aarch64-linux"; builder = "/bin/sh";
        args = [ "-c" "read x < ${input}; echo $x > $out" ];
      }"#;

    /// As root: nix-darwin's builder gets a new disk – stopped, its `nixos.qcow2` (in the daemon's working directory,
    /// nix-darwin's default /var/lib/linux-builder) removed, started again; it creates a new disk with its size.
    pub const DARWIN_NEW_DISK: &'static str = "p=/Library/LaunchDaemons/org.nixos.linux-builder.plist; \
         d=$(/usr/libexec/PlistBuddy -c 'Print :WorkingDirectory' \"$p\" 2>/dev/null); [ -n \"$d\" ] || d=/var/lib/linux-builder; \
         launchctl bootout system/org.nixos.linux-builder 2>/dev/null || true; \
         rm -f \"$d/nixos.qcow2\"; \
         launchctl bootstrap system \"$p\"; \
         echo 'Linux builder restarted with a new disk'";

    pub fn new(paths: Paths) -> Self {
        BuilderHealth { paths }
    }

    /// `<data>/builder/refused`: a build saw the builder refuse the Mac's paths (its first line: when)
    pub fn marker(&self) -> PathBuf {
        self.paths.root.join("builder").join("refused")
    }

    pub fn is_marked(&self) -> bool {
        self.marker().is_file()
    }

    pub fn mark_refused(&self) -> Result<()> {
        let marker = self.marker();
        if let Some(dir) = marker.parent() {
            fs::create_dir_all(dir).with_path(dir)?;
        }
        let now = crate::json::iso8601::format(&crate::json::now());
        fs::write(&marker, format!("{now}\n")).with_path(&marker)
    }

    pub fn clear(&self) {
        let _ = fs::remove_file(self.marker());
    }

    /// A build's output shows the builder refusing the Mac's paths
    pub fn refused_in(output: &str) -> bool {
        output.contains(Self::REFUSED)
    }

    /// Builds `PROBE` (a few seconds) and updates the marker: set when refused, removed when accepted
    pub fn probe(&self) -> Trust {
        let trust = self.run_probe();
        match &trust {
            Trust::Accepted => self.clear(),
            Trust::Refused => {
                let _ = self.mark_refused();
            }
            Trust::Failed(_) => {}
        }
        trust
    }

    fn run_probe(&self) -> Trust {
        let settings = Settings::load(&self.paths);
        let Some(nix) = Prerequisites::locate_nix(&settings) else {
            return Trust::Failed("nix not found".into());
        };
        let nonce = format!(
            "{}-{}",
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0),
            std::process::id()
        );
        let mut args: Vec<String> = [
            "build",
            "--no-link",
            "--impure",
            "--extra-experimental-features",
            "nix-command",
            "--option",
            "connect-timeout",
            "15",
        ]
        .map(String::from)
        .to_vec();
        args.extend(LinuxBuilder::new(self.paths.clone()).arguments());
        args.extend(["--expr".to_string(), Self::PROBE.to_string()]);
        let env = [("OMAFLAKE_PROBE_NONCE", nonce.as_str())];
        match Shell::run(&nix, &args, &RunOptions { environment: &env, ..Default::default() }) {
            Ok(r) => Self::judge(r.status, &format!("{}{}", r.stdout, r.stderr)),
            Err(e) => Trust::Failed(e.message().to_string()),
        }
    }

    /// The probe's exit status and output → `Trust`
    pub(crate) fn judge(status: i32, output: &str) -> Trust {
        if status == 0 {
            Trust::Accepted
        } else if Self::refused_in(output) {
            Trust::Refused
        } else {
            let lines: Vec<&str> = output.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
            Trust::Failed(lines[lines.len().saturating_sub(4)..].join("\n"))
        }
    }

    /// Gives the builder a new disk and checks it again. omaflake's own builder as the user; nix-darwin's runs
    /// `DARWIN_NEW_DISK` through `admin` (the administrator dialog, or sudo in the CLI). `wait` waits until the
    /// builder answers.
    pub fn repair(
        &self,
        log: &(dyn Fn(&str) + Send + Sync),
        admin: &dyn Fn(&str) -> Result<String>,
        wait: &dyn Fn() -> Result<()>,
    ) -> Result<String> {
        let own = LinuxBuilder::new(self.paths.clone());
        if own.is_set_up() && !own.system_has_builder() {
            log("Stopping omaflake's Linux builder and removing its disk …");
            own.stop();
            own.reset_disk()?;
            log("Starting it with a new disk …");
            own.start()?;
        } else if own.system_has_builder() {
            log("Giving nix-darwin's Linux builder a new disk …");
            admin(Self::DARWIN_NEW_DISK)?;
        } else {
            return Err(vm_err!(codes::PREREQUISITE, "Set up the Linux builder first (Linux builder → Set Up …)"));
        }
        wait()?;
        log("Checking whether the builder accepts this Mac's builds …");
        match self.probe() {
            Trust::Accepted => Ok("The Linux builder accepts this Mac's builds again".to_string()),
            Trust::Refused => Err(vm_err!(
                "The Linux builder still refuses this Mac's builds after a new disk – the cause is elsewhere (OPEN, plan B175)"
            )),
            Trust::Failed(why) => Err(vm_err!("Checking the Linux builder failed:\n{}", why)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn judge_tells_refusal_from_other_failures() {
        assert_eq!(BuilderHealth::judge(0, "/nix/store/x-omaflake-builder-probe\n"), Trust::Accepted);
        let refused = "copying 1 paths...\nerror: cannot add path '/nix/store/a-omaflake-builder-probe-input' because it lacks a signature by a trusted key\n";
        assert_eq!(BuilderHealth::judge(1, refused), Trust::Refused);
        let other = "a\nb\nc\nd\nerror: Cannot build '/nix/store/x.drv'.\n";
        assert_eq!(
            BuilderHealth::judge(1, other),
            Trust::Failed("b\nc\nd\nerror: Cannot build '/nix/store/x.drv'.".into())
        );
    }

    #[test]
    fn marker_is_set_and_cleared() {
        let t = tempfile::tempdir().unwrap();
        let h = BuilderHealth::new(Paths::new(t.path().join("data"), None));
        assert!(!h.is_marked());
        h.mark_refused().unwrap();
        assert!(h.is_marked());
        h.clear();
        assert!(!h.is_marked());
    }

    #[test]
    fn darwin_new_disk_keeps_to_the_builder_folder() {
        let c = BuilderHealth::DARWIN_NEW_DISK;
        assert!(c.contains("rm -f \"$d/nixos.qcow2\""));
        assert!(c.contains("[ -n \"$d\" ] || d=/var/lib/linux-builder"));
        assert!(c.contains("launchctl bootstrap system \"$p\""));
    }
}
