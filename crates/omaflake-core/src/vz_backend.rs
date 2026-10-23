// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of VZBackend.swift.
//!
//! macOS guests on Virtualization.framework (spec §6.4, plan M10): every instance is an APFS clone of the
//! image folder in `instances/<name>.omaflake-vm/vm/`, run by its own detached `omaf-runner` process.
//!
//! Network: `shared` = NAT (address from /var/db/dhcpd_leases, SSH on port 22), `none` = no network card.
//! `isolated` is not available: Virtualization.framework's NAT has no restricted mode, and a vmnet host-mode
//! network (`VZVmnetNetworkDeviceAttachment`, macOS 26) needs the restricted vmnet entitlement (plan BM27).
//! Ephemeral: a fresh clone at every start; the clone of the previous run is deleted (BM27).
//!
//! Not 1:1: `locate_runner` looked next to `Bundle.main.executableURL`; here it is `std::env::current_exe()`
//! (resolved), with the same two candidates (`../Helpers/omaf-runner.app/Contents/MacOS/omaf-runner`, `omaf-runner`)
//! plus the development layout: `omaf` from this repository's `target/{debug,release}` finds the Swift package's
//! `.build/{debug,release}/omaf-runner`. `start` is split into checks and `run_runner` (spawn and wait) so the latter
//! can be tested with a stand-in runner.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::agent_protocol::AgentState;
use crate::backend::{StartOptions, VmBackend};
use crate::error::{codes, Error, Result};
use crate::image_store::Image;
use crate::instance_store::InstanceStore;
use crate::json;
use crate::mac_vm::{DhcpLeases, MacVmBundle, RunnerClient};
use crate::models::{DisplayMode, GuestKind, InstanceConfig, NetworkMode, RecipeDefaults, StopResult, VmStatus};
use crate::omaflake_id::OmaflakeId;
use crate::paths::{Paths, Settings};
use crate::platform::{expand_tilde, is_executable_file, resolve_symlinks, standardize};
use crate::qemu_runtime::tail;
use crate::shell::{RunOptions, Shell};
use crate::sockets::{self, Ports};
use crate::spawn::{self, Pid};
use crate::ssh_command::SshCommand;
use crate::trash::Disposal;
use crate::vm_err;

#[derive(Debug, Clone)]
pub struct VzBackend {
    pub paths: Paths,
    pub store: InstanceStore,
    /// Signed omaf-runner (scripts/sign-runner.sh)
    pub runner: String,
}

impl VzBackend {
    /// Virtualization.framework runs at most two macOS VMs per Mac (plan BM16).
    pub const MAX_RUNNING: usize = 2;

    pub fn new(paths: Paths, store: InstanceStore, runner: &str) -> Self {
        VzBackend { paths, store, runner: runner.into() }
    }

    /// omaf-runner from VM_RUNNER, config.json (`runner`), next to the running executable, or PATH.
    pub fn locate_runner(paths: &Paths) -> Result<String> {
        if let Ok(env) = std::env::var("VM_RUNNER") {
            if !env.is_empty() {
                return Shell::which(&env);
            }
        }
        if let Some(configured) = Settings::load(paths).runner {
            return Shell::which(&expand_tilde(&configured));
        }
        if let Ok(exe) = std::env::current_exe() {
            if let Some(found) =
                Self::runner_candidates(&resolve_symlinks(&exe)).into_iter().find(|c| is_executable_file(c))
            {
                return Ok(found.to_string_lossy().into_owned());
            }
        }
        Shell::which("omaf-runner")
    }

    /// Where a runner may sit relative to the running executable `exe` (resolved), in this order:
    /// - in omaflake.app: its own helper bundle, so macOS does not take a VM window for the app (plan B65):
    ///   `Contents/MacOS/omaf` → `Contents/Helpers/omaf-runner.app/Contents/MacOS/omaf-runner` (scripts/bundle-app.sh)
    /// - next to omaf (CLI builds): a plain sibling
    /// - development: omaf from `<repo>/target/{debug,release}` (also `…/deps` for tests) uses the Swift package's
    ///   `<repo>/.build/{debug,release}/omaf-runner`, the same profile first
    pub(crate) fn runner_candidates(exe: &Path) -> Vec<PathBuf> {
        let Some(dir) = exe.parent() else { return Vec::new() };
        let mut candidates = vec![
            standardize(&dir.join("../Helpers/omaf-runner.app/Contents/MacOS/omaf-runner")),
            dir.join("omaf-runner"),
        ];
        let profile_dir = if dir.file_name().is_some_and(|n| n == "deps") { dir.parent() } else { Some(dir) };
        if let Some(profile_dir) = profile_dir {
            let profile = profile_dir.file_name().and_then(|n| n.to_str()).unwrap_or("");
            let target = profile_dir.parent();
            if matches!(profile, "debug" | "release") && target.and_then(Path::file_name).is_some_and(|n| n == "target")
            {
                if let Some(repo) = target.and_then(Path::parent) {
                    if repo.join("Package.swift").exists() {
                        let other = if profile == "debug" { "release" } else { "debug" };
                        for p in [profile, other] {
                            candidates.push(repo.join(".build").join(p).join("omaf-runner"));
                        }
                    }
                }
            }
        }
        candidates
    }

    // MARK: Create, reset, remove

    /// `omaf-runner clone` (APFS clone, new machine identifier and MAC) of the image inside the package or the one in
    /// the data folder, then CPU/RAM from the instance settings. The disk keeps the image's size: growing a macOS
    /// system disk needs an APFS resize in the guest (OPEN).
    pub(crate) fn clone_image(&self, c: &InstanceConfig) -> Result<()> {
        let ip = self.paths.instance(&c.name);
        // The image inside the package (an exported or external VM may carry it), else the one in the data folder
        let source = [ip.embedded_image(), self.paths.image_dir(&c.image)]
            .into_iter()
            .find(|dir| MacVmBundle::new(dir).config().exists())
            .ok_or_else(|| {
                Error::with_code(
                    codes::NOT_FOUND,
                    format!(
                        "Image missing: {} – rebuild it (omaf build {}) or import it, then omaf upgrade/reset",
                        c.image, c.recipe
                    ),
                )
            })?;
        Shell::check(
            &self.runner,
            &["clone".into(), path_string(&source), path_string(&ip.vm())],
            &RunOptions::default(),
        )?;
        let vm = MacVmBundle::new(ip.vm());
        let mut cfg = vm.load_config()?;
        cfg.cpus = c.cpus;
        cfg.memory_mb = c.memory_gb * 1024;
        vm.save(&cfg)
    }

    /// "Network mode \"isolated\" is not available for macOS guests (plan BM27) – use shared or none"
    pub(crate) fn check_network(mode: NetworkMode) -> Result<()> {
        if mode == NetworkMode::Isolated {
            return Err(Error::with_code(
                codes::UNSUPPORTED_SYSTEM,
                "Network mode \"isolated\" is not available for macOS guests (plan BM27) – use shared or none",
            ));
        }
        Ok(())
    }

    // MARK: Start, stop, status

    /// The runner's arguments for `start`
    pub(crate) fn runner_arguments(&self, c: &InstanceConfig, options: StartOptions) -> Result<Vec<String>> {
        let ip = self.paths.instance(&c.name);
        let mut args: Vec<String> = vec![
            "run".into(),
            path_string(&ip.vm()),
            "--socket".into(),
            path_string(&ip.runner_socket()),
            "--pid-file".into(),
            path_string(&ip.runner_pid()),
        ];
        if options.display != DisplayMode::None {
            args.push("--window".into());
        }
        if c.network == NetworkMode::None {
            args.push("--no-network".into());
        }
        if c.sound_on() {
            args.push("--audio".into());
        }
        if c.microphone_on() {
            args.push("--microphone".into());
        }
        if c.clipboard_on() {
            args.push("--clipboard".into());
        }
        // The omaflake agent's port, always (spec §11.9); the guest side for macOS starts with plan B106's follow-up
        args.extend(["--agent-state".into(), path_string(&ip.agent_state())]);
        // Camera: the Mac side and the port are ready; the guest's camera extension comes after M12 (plan B49)
        if c.camera_on() {
            args.push("--camera".into());
            if std::env::var("OMAFLAKE_CAMERA_TEST_PATTERN").as_deref() == Ok("1") {
                args.push("--camera-test-pattern".into());
            }
        }
        if c.share {
            args.extend(["--share".into(), path_string(&self.paths.prepare_share(c, None)?)]);
        }
        Ok(args)
    }

    /// Spawns the runner detached and waits until it reports the VM running (`timeout`; Swift: 30 s); if it exits
    /// first, its log's last lines are the error
    pub(crate) fn run_runner(&self, c: &InstanceConfig, args: &[String], timeout: Duration) -> Result<Pid> {
        let ip = self.paths.instance(&c.name);
        // The run folder is this Mac's (plan B108): a package brought from elsewhere has none yet
        std::fs::create_dir_all(&ip.run).map_err(|e| Error::from(e).context(format!("{}: ", ip.run.display())))?;
        let child = spawn::spawn_detached(&self.runner, args, &ip.runner_log(), &[])?;

        // Ready as soon as the runner reports the VM running; if it exits first, show its log
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if spawn::reap_if_exited(child) {
                return Err(vm_err!("omaf-runner exited during startup:\n{}", tail(&ip.runner_log(), 5)));
            }
            if let Ok(reply) = RunnerClient::send("status", &ip.runner_socket(), Duration::from_secs(1)) {
                if reply.get("state").map(String::as_str) == Some("running") {
                    return Ok(child);
                }
            }
            std::thread::sleep(Duration::from_millis(300));
        }
        Err(vm_err!("omaf-runner does not report the VM as running ({})", ip.runner_log().display()))
    }

    /// The runner's PID from its pid file, if alive and really omaf-runner
    pub fn pid(&self, c: &InstanceConfig) -> Option<Pid> {
        spawn::live_pid(&self.paths.instance(&c.name).runner_pid(), "omaf-runner")
    }

    /// Without the entitlement, Virtualization.framework refuses to start – say so before spawning
    /// (`codesign -d --entitlements - --xml <runner>` must mention com.apple.security.virtualization).
    pub fn check_entitlement(&self) -> Result<()> {
        #[cfg(target_os = "macos")]
        {
            let r = Shell::run(
                "/usr/bin/codesign",
                &["-d", "--entitlements", "-", "--xml", &self.runner],
                &RunOptions::default(),
            )?;
            if !r.stdout.contains("com.apple.security.virtualization") {
                return Err(Error::with_code(
                    codes::PREREQUISITE,
                    format!(
                        "{} lacks the entitlement com.apple.security.virtualization – sign it with scripts/sign-runner.sh",
                        self.runner
                    ),
                ));
            }
            Ok(())
        }
        #[cfg(not(target_os = "macos"))]
        {
            Err(Error::unsupported("Virtualization.framework"))
        }
    }

    // MARK: SSH

    /// NAT address of the running guest (from the DHCP leases, by the MAC address of its clone).
    pub fn address(&self, c: &InstanceConfig) -> Option<String> {
        if c.network == NetworkMode::None {
            return None;
        }
        let cfg = MacVmBundle::new(self.paths.instance(&c.name).vm()).load_config().ok()?;
        DhcpLeases::ip(&cfg.mac_address)
    }

    /// Sets HostName, LocalHostName and ComputerName to the instance name. The image carries its recipe's name
    /// (nix-darwin sets it only on activation), so every clone would otherwise share it. Needs the sudo rule from
    /// darwin-guest.nix; with an older image it fails and the guest keeps the recipe's name.
    pub fn apply_host_name(&self, c: &InstanceConfig, host: &str) -> bool {
        let command = ["HostName", "LocalHostName", "ComputerName"]
            .map(|key| format!("sudo -n /usr/sbin/scutil --set {key} {}", c.name))
            .join(" && ");
        run_ok(&self.ssh_arguments_to(c, host, &[command], true))
    }

    /// Starts the guest's clipboard agent (darwin-guest.nix) – over SSH instead of a LaunchAgent, which macOS would
    /// announce as a background item in every clone (BM25). Images without it simply have no shared clipboard.
    pub fn start_clipboard_agent(&self, c: &InstanceConfig, host: &str) -> bool {
        run_ok(&self.ssh_arguments_to(c, host, &["/run/current-system/sw/bin/omaflake-clipboard-agent".into()], true))
    }

    /// `SshCommand::arguments` to `host` port 22; `batch` adds `-o BatchMode=yes -o ConnectTimeout=10` after ssh
    pub(crate) fn ssh_arguments_to(
        &self,
        c: &InstanceConfig,
        host: &str,
        command: &[String],
        batch: bool,
    ) -> Vec<String> {
        let mut args = SshCommand::arguments(c, &self.paths, host, Some(22), command);
        if batch {
            args.splice(1..1, ["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"].map(String::from));
        }
        args
    }
}

impl VmBackend for VzBackend {
    fn kind(&self) -> GuestKind {
        GuestKind::Macos
    }

    /// Ephemeral instances get their clone at every start; a failed clone deletes the new package again
    fn create(&self, name: &str, recipe: &str, image: &Image, settings: &RecipeDefaults) -> Result<InstanceConfig> {
        self.store.validate(name)?;
        if self.store.exists(name) {
            return Err(Error::with_code(codes::EXISTS, format!("Instance \"{name}\" already exists")));
        }
        if image.kind() != GuestKind::Macos {
            return Err(vm_err!("{} is not a macOS image", image.name));
        }
        InstanceStore::check_runs_here(image)?;
        Self::check_network(settings.network)?;
        let ip = self.paths.instance(name);
        for dir in [&ip.dir, &ip.run] {
            std::fs::create_dir_all(dir).map_err(|e| Error::from(e).context(format!("{}: ", dir.display())))?;
        }
        // The SSH port of a macOS guest is 22 on its own NAT address; no port forwarding
        let mut config = InstanceConfig::new(
            name,
            if image.recipe().is_empty() { recipe } else { image.recipe() },
            &image.name,
            &path_string(&image.path),
            &image.info.system,
            settings,
            22,
            json::now(),
        );
        config.identity = Some(OmaflakeId::random_words());
        // Ephemeral instances get their clone at every start
        if !config.ephemeral {
            if let Err(e) = self.clone_image(&config) {
                let _ = Disposal::delete(&[&ip.dir]);
                return Err(e);
            }
        }
        self.store.save(&config)?;
        Ok(config)
    }

    /// `omaf-runner run <vm> --socket … --pid-file … [--window] [--no-network] [--audio] [--microphone] [--clipboard]
    /// --agent-state <file> [--camera [--camera-test-pattern]] [--share <dir>]`, detached; ready when the runner's
    /// status says "running" (30 s). At most `MAX_RUNNING` macOS VMs.
    fn start(&self, c: &InstanceConfig, options: StartOptions) -> Result<Pid> {
        let ip = self.paths.instance(&c.name);
        if self.pid(c).is_some() {
            return Err(Error::with_code(codes::VM_RUNNING, format!("\"{}\" is already running", c.name)));
        }
        InstanceStore::check_config_runs_here(c)?;
        Self::check_network(c.network)?;
        let running: Vec<String> = self
            .store
            .list()
            .into_iter()
            .filter(|o| o.guest_kind() == GuestKind::Macos && o.name != c.name && self.pid(o).is_some())
            .map(|o| o.name)
            .collect();
        if running.len() >= Self::MAX_RUNNING {
            return Err(Error::with_code(
                codes::MAC_LIMIT,
                format!(
                    "macOS allows at most {} macOS VMs at the same time – running: {} (plan BM16)",
                    Self::MAX_RUNNING,
                    running.join(", ")
                ),
            ));
        }
        self.check_entitlement()?;
        if c.ephemeral && ip.vm().exists() {
            self.reset(c)?;
        } else if !ip.vm().exists() {
            self.clone_image(c)?;
        }
        let args = self.runner_arguments(c, options)?;
        self.run_runner(c, &args, Duration::from_secs(30))
    }

    /// Shutdown via SSH (`sudo -n /sbin/shutdown -h now`, allowed by darwin-guest.nix; a stop request from the
    /// runner hangs on the guest's confirmation dialog when a user is logged in, plan BM17) → runner stop request
    /// → after `timeout` the runner forces the VM off → SIGKILL.
    fn stop(&self, c: &InstanceConfig, timeout: Duration) -> Result<StopResult> {
        let Some(pid) = self.pid(c) else { return Ok(StopResult::AlreadyStopped) };
        let socket = self.paths.instance(&c.name).runner_socket();
        if let Some(host) = self.address(c) {
            let shutdown = ["sudo", "-n", "/sbin/shutdown", "-h", "now"].map(String::from);
            let _ = run_ok(&self.ssh_arguments_to(c, &host, &shutdown, true));
        } else {
            let _ = RunnerClient::send("stop", &socket, RunnerClient::DEFAULT_TIMEOUT);
        }
        if spawn::wait_for_exit(pid, timeout) {
            return Ok(StopResult::Powerdown);
        }
        if RunnerClient::send("kill", &socket, RunnerClient::DEFAULT_TIMEOUT).is_ok()
            && spawn::wait_for_exit(pid, Duration::from_secs(10))
        {
            return Ok(StopResult::ForcedOff);
        }
        spawn::send_signal(pid, libc::SIGKILL);
        spawn::wait_for_exit(pid, Duration::from_secs(5));
        Ok(StopResult::Killed)
    }

    fn status(&self, c: &InstanceConfig) -> VmStatus {
        if self.pid(c).is_none() {
            VmStatus::Stopped
        } else {
            VmStatus::Running
        }
    }

    fn reset(&self, c: &InstanceConfig) -> Result<()> {
        let ip = self.paths.instance(&c.name);
        Disposal::delete(&[ip.vm(), ip.known_hosts()])?;
        self.clone_image(c)
    }

    fn upgrade(&self, c: &InstanceConfig, image: &Image) -> Result<()> {
        if c.image == image.name {
            return Err(vm_err!("\"{}\" already uses {}", c.name, image.name));
        }
        if image.kind() != GuestKind::Macos {
            return Err(vm_err!("{} is not a macOS image", image.name));
        }
        InstanceStore::check_runs_here(image)?;
        let ip = self.paths.instance(&c.name);
        Disposal::delete(&[ip.vm(), ip.known_hosts()])?;
        let mut config = c.clone();
        config.image = image.name.clone();
        config.image_path = path_string(&image.path);
        self.store.replace_embedded_image(&c.name, image)?;
        self.clone_image(&config)?;
        self.store.save(&config)
    }

    fn remove(&self, c: &InstanceConfig) -> Result<()> {
        self.store.remove(&c.name)
    }

    /// Once SSH answers, the guest is named after the instance (plan BM28) and gets its clipboard agent (B48).
    fn wait_for_ssh(&self, c: &InstanceConfig, timeout: Duration) -> bool {
        let deadline = Instant::now() + timeout;
        while Instant::now() < deadline {
            if let Some(host) = self.address(c) {
                if Ports::ssh_banner(&host, 22, sockets::DEFAULT_TIMEOUT).is_some() {
                    self.apply_host_name(c, &host);
                    if c.clipboard_on() {
                        self.start_clipboard_agent(c, &host);
                    }
                    return true;
                }
            }
            if self.pid(c).is_none() {
                return false;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        false
    }

    /// What the guest's omaflake agent last reported (agent.json, written by omaf-runner) – None while stopped
    fn agent_state(&self, c: &InstanceConfig) -> Option<AgentState> {
        if self.status(c) != VmStatus::Running {
            return None;
        }
        AgentState::load(&self.paths.instance(&c.name).agent_state())
    }

    /// Without a lease yet, ssh fails with a clear "could not resolve" instead of a wrong host
    /// (`no-address-yet.<name>.invalid`)
    fn ssh_arguments(&self, c: &InstanceConfig, command: &[String]) -> Vec<String> {
        let host = self.address(c).unwrap_or_else(|| format!("no-address-yet.{}.invalid", c.name));
        self.ssh_arguments_to(c, &host, command, false)
    }
}

fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

/// Runs `args` (program first); true when it exited with 0
fn run_ok(args: &[String]) -> bool {
    let Some((exe, rest)) = args.split_first() else { return false };
    Shell::run(exe, rest, &RunOptions::default()).is_ok_and(|r| r.status == 0)
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::image_store::ImageInfo;
    use crate::mac_vm::MacVmConfig;
    use crate::paths::tests::temp_paths;
    use std::io::{BufRead, Write};
    use std::os::unix::fs::PermissionsExt;
    use std::os::unix::net::UnixListener;

    /// Swift test helper `macConfig(_:network:)`
    pub(crate) fn mac_config(name: &str, network: NetworkMode) -> InstanceConfig {
        let s = RecipeDefaults::new(GuestKind::Macos, None, 4, 8, 80, network, false, false, Some("test".into()));
        InstanceConfig::new(
            name,
            "mac-minimal",
            "mac-minimal-abc-12345678",
            "/img",
            "aarch64-darwin",
            &s,
            22,
            json::parse_date("1970-01-01T00:00:00Z").unwrap(),
        )
    }

    /// A stand-in for omaf-runner: `clone <src> <dst>` copies config.json, `run` does what `run` says
    pub(crate) fn fake_runner(dir: &Path, run: &str) -> String {
        std::fs::create_dir_all(dir).unwrap();
        let file = dir.join("omaf-runner");
        let script = format!(
            "#!/bin/sh\ncase \"$1\" in\nclone) [ -e \"$3\" ] && exit 1; mkdir -p \"$3\" && cp \"$2/config.json\" \"$3/\" ;;\nrun) {run} ;;\n*) exit 2 ;;\nesac\n"
        );
        std::fs::write(&file, script).unwrap();
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o755)).unwrap();
        file.to_string_lossy().into_owned()
    }

    fn backend(p: &Paths, runner: &str) -> VzBackend {
        VzBackend::new(p.clone(), InstanceStore::new(p.clone(), "/usr/bin/false"), runner)
    }

    fn mac_image(p: &Paths, name: &str) -> PathBuf {
        let dir = p.image_dir(name);
        std::fs::create_dir_all(&dir).unwrap();
        MacVmBundle::new(&dir)
            .save(&MacVmConfig::new(2, 4096, 80, "02:00:00:00:00:01", Some("26A434".into())))
            .unwrap();
        dir
    }

    #[test]
    fn isolated_network_is_rejected() {
        let (_t, p) = temp_paths();
        let vz = backend(&p, "/usr/bin/false");
        let info =
            |recipe: &str, kind, system: &str| ImageInfo::new(recipe, None, kind, system, "", "abc", None, json::now());
        let image = Image {
            name: "mac-minimal-abc-12345678".into(),
            info: info("omaflake.macos.minimal", GuestKind::Macos, "aarch64-darwin"),
            path: p.images(),
        };
        let mut s = RecipeDefaults::new(GuestKind::Macos, None, 4, 8, 80, NetworkMode::Isolated, false, false, None);
        assert!(vz.create("m1", "mac-minimal", &image, &s).is_err());
        assert!(!p.instance("m1").dir.exists());
        s.network = NetworkMode::Shared;
        // a NixOS image cannot become a macOS instance
        let qcow = Image {
            name: "dev-abc-12345678".into(),
            info: info("omaflake.nixos.dev", GuestKind::Nixos, "aarch64-linux"),
            path: p.images(),
        };
        assert!(vz.create("m2", "dev", &qcow, &s).is_err());
    }

    #[test]
    fn isolated_network_is_not_available() {
        let e = VzBackend::check_network(NetworkMode::Isolated).unwrap_err();
        assert!(e.message().starts_with("Network mode \"isolated\" is not available for macOS guests"));
        assert!(
            VzBackend::check_network(NetworkMode::Shared).is_ok()
                && VzBackend::check_network(NetworkMode::None).is_ok()
        );
    }

    #[test]
    fn stopped_without_runner_process() {
        let (_t, p) = temp_paths();
        let vz = backend(&p, "/usr/bin/false");
        let c = mac_config("m1", NetworkMode::Shared);
        std::fs::create_dir_all(&p.instance(&c.name).dir).unwrap();
        std::fs::create_dir_all(&p.instance(&c.name).run).unwrap();
        assert_eq!(vz.status(&c), VmStatus::Stopped);
        // a live PID of another program (this test process) does not count as the runner
        std::fs::write(p.instance(&c.name).runner_pid(), format!("{}\n", std::process::id())).unwrap();
        assert_eq!(vz.status(&c), VmStatus::Stopped);
        assert_eq!(vz.stop(&c, Duration::from_secs(1)).unwrap(), StopResult::AlreadyStopped);
        assert_eq!(vz.agent_state(&c), None);
    }

    #[test]
    fn ssh_goes_to_nat_address_port_22() {
        let (_t, p) = temp_paths();
        let c = mac_config("m1", NetworkMode::Shared);
        let args = SshCommand::arguments(&c, &p, "192.168.64.5", Some(22), &["uptime".into()]);
        assert_eq!(args[0], crate::platform::host_tool("/usr/bin/ssh"));
        assert_eq!(args[1..3], ["-p", "22"]);
        assert_eq!(args[args.len() - 2..], ["test@192.168.64.5", "uptime"]);
        // NixOS default unchanged: port forwarding on 127.0.0.1
        let n = SshCommand::arguments(&crate::models::tests::config("t1"), &p, SshCommand::DEFAULT_HOST, None, &[]);
        assert!(n[1..3] == ["-p", "2230"] && n.last().unwrap().ends_with("@127.0.0.1"));

        let vz = backend(&p, "/usr/bin/false");
        let batch = vz.ssh_arguments_to(&c, "192.168.64.5", &["true".into()], true);
        assert_eq!(
            batch[..7],
            [
                crate::platform::host_tool("/usr/bin/ssh").as_str(),
                "-o",
                "BatchMode=yes",
                "-o",
                "ConnectTimeout=10",
                "-p",
                "22"
            ]
        );
        // no lease yet (no clone, so no MAC address): a host that cannot resolve
        let plain = vz.ssh_arguments(&c, &["uptime".into()]);
        assert_eq!(plain[1..3], ["-p", "22"]);
        assert_eq!(plain[plain.len() - 2..], ["test@no-address-yet.m1.invalid", "uptime"]);
        assert_eq!(vz.address(&mac_config("m1", NetworkMode::None)), None);
    }

    #[test]
    fn instance_paths_for_mac_guests() {
        let (_t, p) = temp_paths();
        let ip = p.instance("m1");
        assert!(ip.vm().ends_with("vm") && ip.runner_socket().ends_with("runner.sock"));
        // runtime files live in run/<name>/, outside the package (plan B108)
        assert_eq!(ip.runner_socket().parent().unwrap(), p.run().join("m1"));
        assert_eq!(ip.config().parent().unwrap().file_name().unwrap(), "m1.omaflake-vm");
    }

    #[test]
    fn the_runner_socket_limits_instance_names() {
        // the longer runner socket name limits instance names
        let store = InstanceStore::new(Paths::new(format!("/{}", "x".repeat(60)), None), "");
        assert!(store.validate(&"a".repeat(28)).is_err());
    }

    #[test]
    fn runner_is_found_in_the_app_next_to_omaf_and_in_the_dev_layout() {
        let app = VzBackend::runner_candidates(Path::new("/A/omaflake.app/Contents/MacOS/omaf"));
        assert_eq!(app[0], Path::new("/A/omaflake.app/Contents/Helpers/omaf-runner.app/Contents/MacOS/omaf-runner"));
        assert_eq!(app[1], Path::new("/A/omaflake.app/Contents/MacOS/omaf-runner"));
        assert_eq!(app.len(), 2);

        let (_t, p) = temp_paths();
        let repo = p.root.join("repo");
        std::fs::create_dir_all(repo.join("target/release/deps")).unwrap();
        // not a checkout of this repository: no Swift package, no development candidates
        assert_eq!(VzBackend::runner_candidates(&repo.join("target/release/omaf")).len(), 2);
        std::fs::write(repo.join("Package.swift"), "").unwrap();
        let release = VzBackend::runner_candidates(&repo.join("target/release/omaf"));
        assert_eq!(release[2..], [repo.join(".build/release/omaf-runner"), repo.join(".build/debug/omaf-runner")]);
        let tests = VzBackend::runner_candidates(&repo.join("target/debug/deps/omaflake_core-1234"));
        assert_eq!(tests[2..], [repo.join(".build/debug/omaf-runner"), repo.join(".build/release/omaf-runner")]);
    }

    #[test]
    fn runner_from_the_settings() {
        if std::env::var("VM_RUNNER").is_ok_and(|v| !v.is_empty()) {
            return;
        }
        let (_t, p) = temp_paths();
        let runner = fake_runner(&p.root.join("bin"), "exit 0");
        Settings { runner: Some(runner.clone()), ..Default::default() }.save(&p).unwrap();
        assert_eq!(VzBackend::locate_runner(&p).unwrap(), runner);
    }

    #[test]
    fn clone_and_reset_take_the_image_and_the_instance_settings() {
        let (_t, p) = temp_paths();
        let vz = backend(&p, &fake_runner(&p.root.join("bin"), "exit 0"));
        let c = mac_config("m1", NetworkMode::Shared);
        let missing = vz.clone_image(&c).unwrap_err();
        assert_eq!(missing.code(), Some(codes::NOT_FOUND));
        assert!(missing
            .message()
            .starts_with("Image missing: mac-minimal-abc-12345678 – rebuild it (omaf build mac-minimal)"));
        mac_image(&p, &c.image);
        vz.clone_image(&c).unwrap();
        let vm = MacVmBundle::new(p.instance("m1").vm()).load_config().unwrap();
        assert_eq!((vm.cpus, vm.memory_mb, vm.disk_gb), (4, 8192, 80));
        // reset: the old clone and the known host keys go, a fresh clone comes
        std::fs::write(p.instance("m1").known_hosts(), "x").unwrap();
        std::fs::write(p.instance("m1").vm().join("disk.img"), "used").unwrap();
        vz.reset(&c).unwrap();
        assert!(!p.instance("m1").known_hosts().exists() && !p.instance("m1").vm().join("disk.img").exists());
        assert!(p.instance("m1").vm().join("config.json").exists());
    }

    #[test]
    fn runner_arguments_follow_the_settings() {
        let (_t, p) = temp_paths();
        let vz = backend(&p, "/usr/bin/false");
        let mut c = mac_config("m1", NetworkMode::None);
        c.microphone = Some(true);
        c.clipboard = Some(false);
        c.share = true;
        let ip = p.instance("m1");
        let a = vz.runner_arguments(&c, StartOptions::new(DisplayMode::Window)).unwrap();
        let s = |x: &Path| x.to_string_lossy().into_owned();
        let mut expected = vec!["run".to_string(), s(&ip.vm()), "--socket".into(), s(&ip.runner_socket())];
        expected.extend(["--pid-file".into(), s(&ip.runner_pid()), "--window".into(), "--no-network".into()]);
        expected.extend(["--audio".into(), "--microphone".into(), "--agent-state".into(), s(&ip.agent_state())]);
        expected.extend(["--share".into(), s(&p.share("m1"))]);
        assert_eq!(a, expected);
        assert!(p.share("m1").is_dir());
        let headless =
            vz.runner_arguments(&mac_config("m1", NetworkMode::Shared), StartOptions::new(DisplayMode::None));
        let headless = headless.unwrap();
        assert!(!headless.contains(&"--window".to_string()) && headless.contains(&"--clipboard".to_string()));
    }

    #[test]
    fn the_runner_is_ready_when_its_socket_says_running() {
        let (_t, p) = temp_paths();
        let vz = backend(&p, &fake_runner(&p.root.join("bin"), "echo starting; exec sleep 30"));
        let c = mac_config("m1", NetworkMode::Shared);
        let ip = p.instance("m1");
        std::fs::create_dir_all(&ip.run).unwrap();
        let listener = UnixListener::bind(ip.runner_socket()).unwrap();
        std::thread::spawn(move || {
            for (n, s) in listener.incoming().enumerate() {
                let Ok(mut s) = s else { return };
                let mut line = String::new();
                let _ = std::io::BufReader::new(s.try_clone().unwrap()).read_line(&mut line);
                let state = if n == 0 { "starting" } else { "running" };
                let _ = s.write_all(format!("{{\"state\":\"{state}\"}}\n").as_bytes());
            }
        });
        let pid = vz.run_runner(&c, &["run".into()], Duration::from_secs(10)).unwrap();
        assert!(spawn::is_alive(pid));
        spawn::send_signal(pid, libc::SIGKILL);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !spawn::reap_if_exited(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        assert!(std::fs::read_to_string(ip.runner_log()).unwrap().starts_with("starting\n"));
    }

    #[test]
    fn a_runner_that_exits_during_startup_shows_its_log() {
        let (_t, p) = temp_paths();
        let vz = backend(&p, &fake_runner(&p.root.join("bin"), "echo 'Error: no entitlement'; exit 1"));
        let c = mac_config("m1", NetworkMode::Shared);
        let e = vz.run_runner(&c, &["run".into()], Duration::from_secs(10)).unwrap_err();
        assert!(e.message().starts_with("omaf-runner exited during startup:\n"));
        assert!(e.message().contains("Error: no entitlement"));
        // without an answer on its socket the wait ends with the log's place
        let vz = backend(&p, &fake_runner(&p.root.join("bin2"), "exec sleep 5"));
        let e = vz.run_runner(&c, &["run".into()], Duration::from_millis(700)).unwrap_err();
        assert!(e.message().starts_with("omaf-runner does not report the VM as running ("));
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn an_unsigned_runner_lacks_the_entitlement() {
        let (_t, p) = temp_paths();
        let vz = backend(&p, &fake_runner(&p.root.join("bin"), "exit 0"));
        let e = vz.check_entitlement().unwrap_err();
        assert!(e.message().ends_with(
            "lacks the entitlement com.apple.security.virtualization – sign it with scripts/sign-runner.sh"
        ));
    }
}
