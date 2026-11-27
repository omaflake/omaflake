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

use crate::error::{IoContext, Result};
use crate::linux_builder::LinuxBuilder;
use crate::paths::{Paths, Settings};
use crate::prerequisites::Prerequisites;
use crate::shell::{RunOptions, Shell};
use crate::vm_err;
use omaflake_sdk::codes;

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

    /// The disk size nix/hosts/builder-vm.nix gives the builder (`virtualisation.darwin-builder.diskSize`, GiB)
    pub const DISK_GIB: u64 = 100;
    /// How often a running builder is checked by itself (plan B182): once a day, in the background
    pub const CHECK_EVERY: std::time::Duration = std::time::Duration::from_secs(24 * 3600);

    pub fn new(paths: Paths) -> Self {
        BuilderHealth { paths }
    }

    /// `<data>/builder/checked`: when the last check in the background started
    fn checked_file(&self) -> PathBuf {
        self.paths.root.join("builder").join("checked")
    }

    /// A check is due: none for `CHECK_EVERY` (plan B182)
    pub fn check_due(&self) -> bool {
        let file = self.checked_file();
        fs::metadata(&file)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_none_or(|age| age >= Self::CHECK_EVERY)
    }

    /// Checks the builder in the background when it is due (plan B182): a refusal is noted for the Doctor (`marker`)
    /// before an image build runs into it. Noted as started first, so two Doctor checks do not start two probes.
    pub fn check_in_background(&self) {
        if !self.check_due() {
            return;
        }
        let file = self.checked_file();
        if let Some(dir) = file.parent()
            && fs::create_dir_all(dir).is_ok()
            && fs::write(&file, omaflake_sdk::time::iso8601::format(&omaflake_sdk::time::now())).is_ok()
        {
            let me = self.clone();
            std::thread::spawn(move || {
                let _ = me.probe();
            });
        }
    }

    /// The builder's disk (plan B182): omaflake's own in its folder, nix-darwin's in the daemon's working directory
    /// (its default /var/lib/linux-builder); None for another builder or the default one (its 20 GB are its own)
    pub fn disk(&self) -> Option<PathBuf> {
        let own = LinuxBuilder::new(self.paths.clone());
        if own.is_set_up() && !own.system_has_builder() {
            return (own.stage() == Some(crate::linux_builder::Stage::Omaflake)).then(|| own.disk());
        }
        if !crate::builder_maintenance::BuilderMaintenance::new(self.paths.clone()).system_is_omaflakes() {
            return None;
        }
        let plist = fs::read_to_string("/Library/LaunchDaemons/org.nixos.linux-builder.plist").unwrap_or_default();
        let dir = plist
            .split("<key>WorkingDirectory</key>")
            .nth(1)
            .and_then(|r| r.split("<string>").nth(1))
            .and_then(|r| r.split("</string>").next())
            .unwrap_or("/var/lib/linux-builder");
        Some(PathBuf::from(dir).join("nixos.qcow2"))
    }

    /// A qcow2's virtual size from its header (magic "QFI\xfb", the size at byte 24) – no qemu-img needed
    pub fn qcow2_size(path: &std::path::Path) -> Option<u64> {
        use std::io::Read;
        let mut head = [0u8; 32];
        fs::File::open(path).ok()?.read_exact(&mut head).ok()?;
        (head[0..4] == *b"QFI\xfb").then(|| u64::from_be_bytes(head[24..32].try_into().unwrap_or_default()))
    }

    /// The builder's disk is smaller than its settings (plan B182): (GiB it has, GiB it should have) – it was made
    /// before the setting and is only made when missing
    pub fn disk_too_small(&self) -> Option<(u64, u64)> {
        let have = Self::qcow2_size(&self.disk()?)? >> 30;
        (have + 1 < Self::DISK_GIB).then_some((have, Self::DISK_GIB))
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
        let now = omaflake_sdk::time::iso8601::format(&omaflake_sdk::time::now());
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

    /// The size it checks against is builder-vm.nix's
    #[test]
    fn disk_size_matches_the_builder_settings() {
        let nix =
            std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../nix/hosts/builder-vm.nix")).unwrap();
        assert!(nix.contains(&format!("diskSize = {} * 1024;", BuilderHealth::DISK_GIB)), "{nix}");
    }

    #[test]
    fn qcow2_size_from_the_header() {
        let t = tempfile::tempdir().unwrap();
        let f = t.path().join("d.qcow2");
        let mut head = b"QFI\xfb".to_vec();
        head.extend_from_slice(&[0, 0, 0, 3]);
        head.extend_from_slice(&[0u8; 16]);
        head.extend_from_slice(&(20u64 << 30).to_be_bytes());
        std::fs::write(&f, &head).unwrap();
        assert_eq!(BuilderHealth::qcow2_size(&f), Some(20 << 30));
        std::fs::write(&f, b"not a qcow2 file at all, not at all").unwrap();
        assert_eq!(BuilderHealth::qcow2_size(&f), None);
    }

    #[test]
    fn a_check_is_due_once_a_day() {
        let t = tempfile::tempdir().unwrap();
        let h = BuilderHealth::new(Paths::new(t.path().join("data"), None));
        assert!(h.check_due());
        std::fs::create_dir_all(h.checked_file().parent().unwrap()).unwrap();
        std::fs::write(h.checked_file(), "x").unwrap();
        assert!(!h.check_due());
    }

    #[test]
    fn darwin_new_disk_keeps_to_the_builder_folder() {
        let c = BuilderHealth::DARWIN_NEW_DISK;
        assert!(c.contains("rm -f \"$d/nixos.qcow2\""));
        assert!(c.contains("[ -n \"$d\" ] || d=/var/lib/linux-builder"));
        assert!(c.contains("launchctl bootstrap system \"$p\""));
    }
}
