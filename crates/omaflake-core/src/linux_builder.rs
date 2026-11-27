// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaflake's own Linux builder on a Mac (plan B160): nixpkgs' `darwin.linux-builder`, run as the user – no nix-darwin,
//! nothing in /etc. A LaunchAgent (~/Library/LaunchAgents) runs the builder's `run-builder` with its key and disk in
//! `<data>/builder/`; every build hands it to the Nix daemon with `--builders` (the user is one of Nix's
//! `trusted-users`, which omaflake's Nix install sets). Two stages: the default builder from the binary cache, then
//! omaflake's (nix/hosts/builder-vm.nix: cores, memory, disk), built by the first.
//!
//! A Mac whose Nix already has an aarch64-linux builder (nix-darwin's `nix.linux-builder`, /etc/nix/machines) keeps
//! using that one; omaflake's builder is then not set up.

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{IoContext, Result};
use crate::image_store::ImageStore;
use crate::paths::{Paths, Settings};
use crate::platform::home_dir;
use crate::prerequisites::Prerequisites;
use crate::shell::{RunOptions, Shell, StreamOptions};
use crate::vm_err;
use omaflake_sdk::codes;

/// The two builders the image flake offers (`linuxBuilders.aarch64-darwin.<stage>`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Stage {
    /// nixpkgs' default (1 core, 3 GiB, 20 GiB disk) – from the binary cache, needs no Linux build
    Bootstrap,
    /// with omaflake's settings (nix/hosts/builder-vm.nix) – built by the bootstrap builder
    Omaflake,
}

impl Stage {
    pub fn attribute(self) -> &'static str {
        match self {
            Stage::Bootstrap => "bootstrap",
            Stage::Omaflake => "omaflake",
        }
    }

    /// Jobs the builder runs at once – its cores (nixpkgs' default 1, builder-vm.nix 6)
    pub fn max_jobs(self) -> u32 {
        match self {
            Stage::Bootstrap => 1,
            Stage::Omaflake => 6,
        }
    }
}

#[derive(Debug, Clone)]
pub struct LinuxBuilder {
    pub paths: Paths,
}

impl LinuxBuilder {
    pub const LABEL: &'static str = "app.omaflake.linux-builder";
    /// nix-builder.nix `virtualisation.darwin-builder.hostPort`
    pub const PORT: u16 = 31022;
    /// The builder's public SSH host key, base64 – nixpkgs' publicly known default
    /// (nixos/modules/profiles/keys/ssh_host_ed25519_key.pub, as doc/packages/darwin-builder.section.md lists it)
    pub const HOST_KEY: &'static str = "c3NoLWVkMjU1MTkgQUFBQUMzTnphQzFsWkRJMU5URTVBQUFBSUpCV2N4Yi9CbGFxdDFhdU90RStGOFFVV3JVb3RpQzVxQkorVXVFV2RWQ2Igcm9vdEBuaXhvcwo=";
    /// What the builder can build for (nix-darwin's `nix.linux-builder.supportedFeatures` default)
    pub const FEATURES: &'static str = "kvm,benchmark,big-parallel";

    pub fn new(paths: Paths) -> Self {
        LinuxBuilder { paths }
    }

    /// `<data>/builder`: its key, the link to its `run-builder`, its disk (nixos.qcow2, the working directory)
    pub fn dir(&self) -> PathBuf {
        self.paths.root.join("builder")
    }

    /// The private key the Nix daemon logs in with (root reads it; outside the store)
    pub fn key(&self) -> PathBuf {
        self.dir().join("builder_ed25519")
    }

    /// `$KEYS` of run-builder: only the public key – run-builder adds this folder to the Nix store, where every user
    /// can read it, and the guest authorizes `<KEYS>/builder_ed25519.pub`
    pub fn keys_dir(&self) -> PathBuf {
        self.dir().join("keys")
    }

    /// The out-link of the builder that runs (a GC root, so the store keeps it)
    pub fn current(&self) -> PathBuf {
        self.dir().join("current")
    }

    /// Which stage `current` is (written next to it)
    fn stage_file(&self) -> PathBuf {
        self.dir().join("stage")
    }

    pub fn disk(&self) -> PathBuf {
        self.dir().join("nixos.qcow2")
    }

    pub fn vm_log(&self) -> PathBuf {
        self.paths.logs().join("linux-builder-vm.log")
    }

    pub fn plist(&self) -> PathBuf {
        home_dir().join("Library/LaunchAgents").join(format!("{}.plist", Self::LABEL))
    }

    /// Set up: key, a builder and the LaunchAgent are there
    pub fn is_set_up(&self) -> bool {
        self.key().is_file() && self.current().join("bin/run-builder").exists() && self.plist().is_file()
    }

    pub fn stage(&self) -> Option<Stage> {
        match fs::read_to_string(self.stage_file()).ok()?.trim() {
            "bootstrap" => Some(Stage::Bootstrap),
            "omaflake" => Some(Stage::Omaflake),
            _ => None,
        }
    }

    /// `ssh-ng://builder@localhost:31022 aarch64-linux <key> <jobs> 1 <features> - <host key>` – a line of Nix's
    /// `builders` (Nix takes the port in the URL: checked with Nix 2.34)
    pub fn machine(&self, stage: Stage) -> String {
        format!(
            "ssh-ng://builder@localhost:{} aarch64-linux {} {} 1 {} - {}",
            Self::PORT,
            self.key().display(),
            stage.max_jobs(),
            Self::FEATURES,
            Self::HOST_KEY
        )
    }

    /// For every `nix` call that may build Linux: this builder, when it is set up and Nix has no Linux builder of its
    /// own. `--builders-use-substitutes`: the builder fetches from the binary cache itself.
    pub fn arguments(&self) -> Vec<String> {
        if !cfg!(target_os = "macos") || !self.is_set_up() || self.system_has_builder() {
            return Vec::new();
        }
        let stage = self.stage().unwrap_or(Stage::Bootstrap);
        vec!["--builders".into(), self.machine(stage), "--builders-use-substitutes".into()]
    }

    /// Nix itself has an aarch64-linux builder (nix-darwin's, or one of /etc/nix/machines)
    pub fn system_has_builder(&self) -> bool {
        let settings = Settings::load(&self.paths);
        let Some(nix) = Prerequisites::locate_nix(&settings) else { return false };
        Prerequisites::system_linux_builder(&Prerequisites::nix_config(&nix)).is_some()
    }

    /// This Mac can run VMs: `kern.hv_support` is 1 (0 in a Mac VM without nested virtualization)
    pub fn has_hypervisor() -> bool {
        if !cfg!(target_os = "macos") {
            return true;
        }
        Shell::check("/usr/sbin/sysctl", &["-n", "kern.hv_support"], &RunOptions::default())
            .map(|v| v.trim() == "1")
            .unwrap_or(true)
    }

    /// Creates the builder key (no passphrase: the daemon logs in unattended) and puts its public half into `$KEYS`
    pub fn ensure_key(&self) -> Result<()> {
        let dir = self.dir();
        fs::create_dir_all(self.keys_dir()).with_path(&self.keys_dir())?;
        let key = self.key();
        if !key.is_file() || !key.with_extension("pub").is_file() {
            for f in [key.clone(), key.with_extension("pub")] {
                if f.exists() {
                    fs::remove_file(&f).with_path(&f)?;
                }
            }
            let path = key.to_string_lossy().into_owned();
            Shell::check(
                "/usr/bin/ssh-keygen",
                &["-q", "-t", "ed25519", "-N", "", "-C", "builder@localhost", "-f", &path],
                &RunOptions { cwd: Some(&dir), ..Default::default() },
            )?;
        }
        let public = key.with_extension("pub");
        let target = self.keys_dir().join("builder_ed25519.pub");
        fs::copy(&public, &target).with_path(&target)?;
        Ok(())
    }

    /// `nix build --out-link <dir>/<stage> <flake>#linuxBuilders.aarch64-darwin.<stage>` (the Omaflake stage with the
    /// bootstrap builder: it is a Linux build) – the store path of its run-builder
    pub fn build(&self, stage: Stage, on_line: &(dyn Fn(&str) + Send + Sync)) -> Result<PathBuf> {
        let settings = Settings::load(&self.paths);
        let flake = settings.flake_path()?;
        let nix = Prerequisites::locate_nix(&settings).ok_or_else(|| vm_err!(codes::PREREQUISITE, "nix not found"))?;
        let link = self.dir().join(stage.attribute());
        let target = format!("{}#linuxBuilders.aarch64-darwin.{}", ImageStore::flake_ref(&flake), stage.attribute());
        let link_s = link.to_string_lossy().into_owned();
        let mut args: Vec<String> = vec![
            "build".into(),
            "--out-link".into(),
            link_s,
            "--print-out-paths".into(),
            "--no-write-lock-file".into(),
            "--extra-experimental-features".into(),
            "nix-command flakes".into(),
            target,
        ];
        if stage == Stage::Omaflake && !self.system_has_builder() {
            args.extend(["--builders".into(), self.machine(Stage::Bootstrap), "--builders-use-substitutes".into()]);
        }
        let r = Shell::stream(&nix, &args, &StreamOptions::default(), on_line)?;
        let out = r.stdout.trim().to_string();
        if r.status != 0 || !Path::new(&out).join("bin/run-builder").exists() {
            return Err(vm_err!("Building the Linux builder failed – see the lines above"));
        }
        Ok(link)
    }

    /// Makes `link` (an out-link of `build`) the builder that runs: `current` → its store path, the stage noted
    pub fn select(&self, stage: Stage, link: &Path) -> Result<()> {
        let target = fs::read_link(link).with_path(link)?;
        let current = self.current();
        if current.symlink_metadata().is_ok() {
            fs::remove_file(&current).with_path(&current)?;
        }
        // A plain symlink into the store: the stage's own out-link (a GC root) keeps the path alive
        std::os::unix::fs::symlink(&target, &current).with_path(&current)?;
        fs::write(self.stage_file(), stage.attribute()).with_path(&self.stage_file())?;
        Ok(())
    }

    /// The LaunchAgent: runs `current/bin/run-builder` in `<data>/builder` with `KEYS`, keeps it alive, logs to
    /// logs/linux-builder-vm.log
    pub fn plist_text(&self) -> String {
        let esc = |p: &Path| xml_escape(&p.to_string_lossy());
        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key><string>{label}</string>
  <key>ProgramArguments</key><array><string>{program}</string></array>
  <key>WorkingDirectory</key><string>{dir}</string>
  <key>EnvironmentVariables</key><dict><key>KEYS</key><string>{keys}</string></dict>
  <key>RunAtLoad</key><true/>
  <key>KeepAlive</key><true/>
  <key>ProcessType</key><string>Background</string>
  <key>StandardOutPath</key><string>{log}</string>
  <key>StandardErrorPath</key><string>{log}</string>
</dict>
</plist>
"#,
            label = Self::LABEL,
            program = esc(&self.current().join("bin/run-builder")),
            dir = esc(&self.dir()),
            keys = esc(&self.keys_dir()),
            log = esc(&self.vm_log()),
        )
    }

    /// Writes the LaunchAgent and (re)starts it
    pub fn start(&self) -> Result<()> {
        let plist = self.plist();
        if let Some(parent) = plist.parent() {
            fs::create_dir_all(parent).with_path(parent)?;
        }
        fs::create_dir_all(self.paths.logs()).with_path(&self.paths.logs())?;
        fs::write(&plist, self.plist_text()).with_path(&plist)?;
        self.stop();
        let plist_s = plist.to_string_lossy().into_owned();
        Shell::check("/bin/launchctl", &["bootstrap", &Self::domain(), &plist_s], &RunOptions::default())
            .map_err(|e| vm_err!("Starting the Linux builder failed: {}", e.message()))?;
        Ok(())
    }

    /// Stops the LaunchAgent (quietly when it does not run)
    pub fn stop(&self) {
        let service = format!("{}/{}", Self::domain(), Self::LABEL);
        let _ = Shell::run("/bin/launchctl", &["bootout", &service], &RunOptions::default());
    }

    /// Stops the builder and removes the LaunchAgent; key, builders and disk stay in `<data>/builder`
    pub fn remove(&self) -> Result<()> {
        self.stop();
        let plist = self.plist();
        if plist.exists() {
            fs::remove_file(&plist).with_path(&plist)?;
        }
        Ok(())
    }

    /// Removes the builder's disk: a new builder creates its own with its size (the disk is only created when missing)
    pub fn reset_disk(&self) -> Result<()> {
        let disk = self.disk();
        if disk.exists() {
            fs::remove_file(&disk).with_path(&disk)?;
        }
        Ok(())
    }

    /// `ssh builder@localhost` with the builder key – as the user (for Clean Up)
    pub fn ssh_arguments(&self, command: &str) -> Vec<String> {
        vec![
            "-p".into(),
            Self::PORT.to_string(),
            "-i".into(),
            self.key().to_string_lossy().into_owned(),
            "-o".into(),
            "IdentitiesOnly=yes".into(),
            "-o".into(),
            "BatchMode=yes".into(),
            "-o".into(),
            "StrictHostKeyChecking=no".into(),
            "-o".into(),
            "UserKnownHostsFile=/dev/null".into(),
            "-o".into(),
            "LogLevel=ERROR".into(),
            "builder@localhost".into(),
            command.into(),
        ]
    }

    /// What omaflake's builder needs before it starts: a hypervisor, Nix, and the user among Nix's trusted-users
    pub fn preconditions(&self) -> Result<()> {
        if !Self::has_hypervisor() {
            return Err(vm_err!(
                codes::PREREQUISITE,
                "This Mac cannot run VMs (no hypervisor – probably itself a VM without nested virtualization). The Linux builder needs a Mac that can."
            ));
        }
        let settings = Settings::load(&self.paths);
        let nix = Prerequisites::locate_nix(&settings).ok_or_else(|| vm_err!(codes::PREREQUISITE, "nix not found"))?;
        let config = Prerequisites::nix_config(&nix);
        if !Prerequisites::is_trusted(&config, &crate::platform::user_name(), &Prerequisites::user_groups()) {
            return Err(vm_err!(
                codes::PREREQUISITE,
                "Nix does not let you hand it a builder: you are not one of its trusted-users. Allow it in the Doctor (Linux builder → Allow …)."
            ));
        }
        Ok(())
    }

    /// Waits until the builder answers – up to `minutes` (its first start downloads and boots it), a line every 30 s;
    /// an error when it does not (plan B160: it gave up silently and the next build failed)
    pub fn wait(&self, minutes: u64, log: &(dyn Fn(&str) + Send + Sync), cancelled: &dyn Fn() -> bool) -> Result<()> {
        log("Waiting for the Linux builder …");
        let pre = Prerequisites::new(self.paths.clone());
        let start = std::time::Instant::now();
        let mut said = 0;
        loop {
            if pre.builder_running().status == omaflake_sdk::models::PrerequisiteStatus::Ok {
                log("The Linux builder answers");
                return Ok(());
            }
            if cancelled() {
                return Err(crate::error::Error::cancelled());
            }
            let waited = start.elapsed().as_secs();
            if waited >= minutes * 60 {
                return Err(vm_err!(
                    codes::PREREQUISITE,
                    "The Linux builder did not answer within {} minutes. Its log: {}",
                    minutes,
                    self.vm_log().display()
                ));
            }
            if waited / 30 > said {
                said = waited / 30;
                log(&format!("Still waiting for the Linux builder ({} s) …", waited));
            }
            std::thread::sleep(std::time::Duration::from_secs(2));
        }
    }

    /// Sets the builder up, as the user: key → the default builder from the binary cache → LaunchAgent → wait →
    /// omaflake's builder (`apply`). A builder of Nix's own is used as it is.
    pub fn set_up(&self, log: &(dyn Fn(&str) + Send + Sync), cancelled: &dyn Fn() -> bool) -> Result<()> {
        if self.system_has_builder() {
            log("Nix has a Linux builder already – omaflake uses that one");
            return Ok(());
        }
        self.preconditions()?;
        log("Creating the builder's key …");
        self.ensure_key()?;
        if self.stage() == Some(Stage::Omaflake) && self.current().join("bin/run-builder").exists() {
            log("Starting the Linux builder …");
            self.start()?;
            return self.wait(10, log, cancelled);
        }
        log("Downloading the Linux builder (stage 1 of 2, about 430 MB from the binary cache) …");
        let link = self.build(Stage::Bootstrap, log)?;
        self.select(Stage::Bootstrap, &link)?;
        log("Starting the Linux builder (stage 1 of 2) – its first start takes a few minutes …");
        self.start()?;
        self.wait(15, log, cancelled)?;
        self.apply(log, cancelled)
    }

    /// Builds omaflake's builder (nix/hosts/builder-vm.nix) with the running one and switches to it with a new disk
    pub fn apply(&self, log: &(dyn Fn(&str) + Send + Sync), cancelled: &dyn Fn() -> bool) -> Result<()> {
        self.preconditions()?;
        log("Building omaflake's Linux builder (stage 2 of 2) …");
        let link = self.build(Stage::Omaflake, log)?;
        log("Switching to omaflake's Linux builder (a new disk with its size) …");
        self.stop();
        self.reset_disk()?;
        self.select(Stage::Omaflake, &link)?;
        self.start()?;
        self.wait(15, log, cancelled)
    }

    fn domain() -> String {
        // SAFETY: getuid has no preconditions and cannot fail
        format!("gui/{}", unsafe { libc::getuid() })
    }
}

fn xml_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn builder() -> (tempfile::TempDir, LinuxBuilder) {
        let t = tempfile::tempdir().unwrap();
        let paths = Paths::new(t.path().join("data with space"), None);
        (t, LinuxBuilder::new(paths))
    }

    #[test]
    fn machine_line_has_port_key_jobs_and_host_key() {
        let (_t, b) = builder();
        let line = b.machine(Stage::Omaflake);
        let fields: Vec<&str> = line.split(' ').collect();
        assert_eq!(fields[0], "ssh-ng://builder@localhost:31022");
        assert_eq!(fields[1], "aarch64-linux");
        assert!(line.contains(" 6 1 kvm,benchmark,big-parallel - c3NoLWVk"));
        assert_eq!(b.machine(Stage::Bootstrap).split(" 1 1 ").count(), 2);
    }

    #[test]
    fn plist_runs_the_current_builder_with_its_keys() {
        let (_t, b) = builder();
        let p = b.plist_text();
        assert!(p.contains("<string>app.omaflake.linux-builder</string>"));
        assert!(p.contains(&format!("<string>{}</string>", b.current().join("bin/run-builder").display())));
        assert!(p.contains(&format!("<key>KEYS</key><string>{}</string>", b.keys_dir().display())));
        assert!(p.contains("<key>KeepAlive</key><true/>"));
        assert_eq!(xml_escape("a&b<c>\"d"), "a&amp;b&lt;c&gt;&quot;d");
    }

    #[test]
    fn select_points_current_at_the_stage_and_notes_it() {
        let (_t, b) = builder();
        fs::create_dir_all(b.dir()).unwrap();
        let store = b.paths.root.join("store-path");
        fs::create_dir_all(store.join("bin")).unwrap();
        let link = b.dir().join("omaflake");
        std::os::unix::fs::symlink(&store, &link).unwrap();
        b.select(Stage::Omaflake, &link).unwrap();
        assert_eq!(fs::read_link(b.current()).unwrap(), store);
        assert_eq!(b.stage(), Some(Stage::Omaflake));
        // not set up without key and LaunchAgent
        assert!(!b.is_set_up());
    }

    #[test]
    fn key_is_created_once_and_only_its_public_half_goes_to_keys() {
        let (_t, b) = builder();
        b.ensure_key().unwrap();
        let first = fs::read_to_string(b.key()).unwrap();
        b.ensure_key().unwrap();
        assert_eq!(fs::read_to_string(b.key()).unwrap(), first);
        let keys: Vec<_> = fs::read_dir(b.keys_dir()).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(keys, ["builder_ed25519.pub"]);
    }
}
