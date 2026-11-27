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

use std::fs;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use crate::agent_protocol::AgentState;
use crate::backend::{StartOptions, VmBackend};
use crate::error::{Error, IoContext, Result};
use crate::image_store::Image;
use crate::instance_store::InstanceStore;
use crate::mac_vm::{DhcpLeases, MacVmBundle, RunnerClient};
use crate::models::{InstanceConfig, RecipeDefaults};
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
use omaflake_sdk::codes;
use omaflake_sdk::models::{DisplayMode, GuestKind, NetworkMode, StopResult, VmStatus};

#[derive(Debug, Clone)]
pub struct VzBackend {
    pub paths: Paths,
    pub store: InstanceStore,
    /// Signed omaf-runner (scripts/sign-runner.sh)
    pub runner: String,
}

impl VzBackend {
    /// How long the runner's stop request may take when SSH cannot ask the guest (plan B165 round-up)
    pub const REQUEST_WAIT: Duration = Duration::from_secs(15);

    /// Virtualization.framework runs at most two macOS VMs per Mac (plan BM16).
    pub const MAX_RUNNING: usize = 2;

    pub fn new(paths: Paths, store: InstanceStore, runner: &str) -> Self {
        VzBackend { paths, store, runner: runner.into() }
    }

    /// omaf-runner from VM_RUNNER, config.json (`runner`), next to the running executable, or PATH.
    pub fn locate_runner(paths: &Paths) -> Result<String> {
        if let Ok(env) = std::env::var("VM_RUNNER")
            && !env.is_empty()
        {
            return Shell::which(&env);
        }
        if let Some(configured) = Settings::load(paths).runner {
            return Shell::which(&expand_tilde(&configured));
        }
        if let Ok(exe) = std::env::current_exe()
            && let Some(found) =
                Self::runner_candidates(&resolve_symlinks(&exe)).into_iter().find(|c| is_executable_file(c))
        {
            return Ok(found.to_string_lossy().into_owned());
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
            if matches!(profile, "debug" | "release")
                && target.and_then(Path::file_name).is_some_and(|n| n == "target")
                && let Some(repo) = target.and_then(Path::parent)
                && repo.join("Package.swift").exists()
            {
                let other = if profile == "debug" { "release" } else { "debug" };
                for p in [profile, other] {
                    candidates.push(repo.join(".build").join(p).join("omaf-runner"));
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
                vm_err!(
                    codes::NOT_FOUND,
                    "Image missing: {} – rebuild it (omaf build {}) or import it, then omaf upgrade/reset",
                    c.image,
                    c.recipe
                )
            })?;
        Shell::check(
            &self.runner,
            &["clone".into(), path_string(&source), path_string(&ip.vm())],
            &RunOptions::default(),
        )?;
        Self::apply_resources(&self.paths, c)
    }

    /// Cores, memory and disk of a stopped macOS VM into its folder (config.json, disk.img) – the runner reads them at
    /// start (plan B164). The disk only grows: disk.img (raw, sparse) gets the new size; the guest's
    /// `omaflake-grow-disk` (darwin-guest.nix) moves its system container into the new space at the next boot.
    pub(crate) fn apply_resources(paths: &Paths, c: &InstanceConfig) -> Result<()> {
        let vm = MacVmBundle::new(paths.instance(&c.name).vm());
        // Not cloned yet (ephemeral, or never started): the clone gets them when it is made (`clone_image`)
        if !vm.config().exists() {
            return Ok(());
        }
        let mut cfg = vm.load_config()?;
        cfg.cpus = c.cpus;
        cfg.memory_mb = c.memory_gb * 1024;
        let disk = vm.disk();
        if let Ok(have) = fs::metadata(&disk).map(|m| m.len()) {
            let want = u64::from(c.disk_gb) << 30;
            if want > have {
                let f = fs::OpenOptions::new().write(true).open(&disk).with_path(&disk)?;
                f.set_len(want).with_path(&disk)?;
            }
            cfg.disk_gb = cfg.disk_gb.max((want.max(have) >> 30) as u32);
        }
        vm.save(&cfg)
    }

    /// "Network mode \"isolated\" is not available for macOS guests (plan BM27) – use shared or none"
    pub(crate) fn check_network(mode: NetworkMode) -> Result<()> {
        if mode == NetworkMode::Isolated {
            return Err(vm_err!(
                codes::UNSUPPORTED_SYSTEM,
                "Network mode \"isolated\" is not available for macOS guests (plan BM27) – use shared or none"
            ));
        }
        Ok(())
    }

    // MARK: Start, stop, status

    /// The runner's arguments for `start`
    pub(crate) fn runner_arguments(
        &self,
        c: &InstanceConfig,
        options: StartOptions,
        networks: &[crate::networks::Attachment],
    ) -> Result<Vec<String>> {
        let ip = self.paths.instance(&c.name);
        let mut args: Vec<String> = vec![
            "run".into(),
            path_string(&ip.vm()),
            "--socket".into(),
            path_string(&ip.runner_socket()),
            "--pid-file".into(),
            path_string(&ip.runner_pid()),
        ];
        // Recovery is used by hand: always in a window
        if options.display != DisplayMode::None || options.recovery {
            args.push("--window".into());
        }
        if options.recovery {
            args.push("--recovery".into());
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
        // The omaflake agent's port, always (spec §11.9); in macOS guests the agent carries MIDI (plan B161)
        args.extend(["--agent-state".into(), path_string(&ip.agent_state())]);
        if c.midi_on() {
            // The VM's own pair on the Mac is "omaflake <name>" – the bundle's folder is "vm" for every VM
            args.extend(["--midi".into(), "--vm-name".into(), c.name.clone()]);
        }
        // Ports of other VMs on its localhost, over the agent (plan B180)
        for f in crate::networks::Networks::new(self.paths.clone()).forward_targets(c) {
            args.extend(["--forward".into(), f.argument()]);
        }
        // Camera: over the agent from images of 0.7.5 on (plan B169), the port org.omaflake.camera for older ones
        if c.camera_on() {
            args.push("--camera".into());
            if std::env::var("OMAFLAKE_CAMERA_TEST_PATTERN").as_deref() == Ok("1") {
                args.push("--camera-test-pattern".into());
            }
        }
        // Each volume as <name>=<folder> (plan B187): the guest mounts it at /Volumes/<name>
        for (name, folder) in crate::volumes::Volumes::new(self.paths.clone()).prepare_all(c, None)? {
            args.extend(["--volume".into(), format!("{name}={}", path_string(&folder))]);
        }
        // A second card per network (spec §11.12): the runner binds its end and sends to the switch
        for (i, n) in networks.iter().enumerate() {
            args.extend([
                "--lan-mac".into(),
                crate::networks::packets::mac_text(&n.mac),
                "--lan-socket".into(),
                path_string(&ip.network_socket(i)),
                "--lan-switch".into(),
                path_string(&n.vz_socket),
            ]);
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
            if let Ok(reply) = RunnerClient::send("status", &ip.runner_socket(), Duration::from_secs(1))
                && reply.get("state").map(String::as_str) == Some("running")
            {
                return Ok(child);
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
                return Err(vm_err!(
                    codes::PREREQUISITE,
                    "{} lacks the entitlement com.apple.security.virtualization – sign it with scripts/sign-runner.sh",
                    self.runner
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

    /// Starts the guest's helpers (darwin-guest.nix) – over SSH instead of a LaunchAgent, which macOS would announce as
    /// a background item in every clone (BM25): the clipboard helper, and the omaflake agent (MIDI, plan B161, and the
    /// diagnosis, B162). Each starts only once; images without the agent simply lack it. The command ends with a check
    /// that the helpers run, so a start that did not take fails (and is tried again at the next look); the error is
    /// ssh's stderr (or the exit status) – the caller logs it.
    pub fn start_guest_helpers(&self, c: &InstanceConfig, host: &str) -> std::result::Result<(), String> {
        let args = self.ssh_arguments_to(c, host, &[guest_helpers_command(c)], true);
        let Some((exe, rest)) = args.split_first() else { return Err("no ssh".into()) };
        match Shell::run(exe, rest, &RunOptions::default()) {
            Ok(r) if r.status == 0 => Ok(()),
            Ok(r) => {
                let why = r.stderr.trim();
                Err(if why.is_empty() {
                    format!("exit status {} – the helpers did not start (or are not in this image)", r.status)
                } else {
                    format!("exit status {}: {why}", r.status)
                })
            }
            Err(e) => Err(e.message().to_string()),
        }
    }

    /// `start_guest_helpers`, true when they run
    pub fn start_clipboard_agent(&self, c: &InstanceConfig, host: &str) -> bool {
        self.start_guest_helpers(c, host).is_ok()
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
            return Err(vm_err!(codes::EXISTS, "Instance \"{name}\" already exists"));
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
            omaflake_sdk::time::now(),
        );
        config.identity = Some(OmaflakeId::random_words());
        // Ephemeral instances get their clone at every start
        if !config.ephemeral
            && let Err(e) = self.clone_image(&config)
        {
            let _ = Disposal::delete(&[&ip.dir]);
            return Err(e);
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
            return Err(vm_err!(codes::VM_RUNNING, "\"{}\" is already running", c.name));
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
            return Err(vm_err!(
                codes::MAC_LIMIT,
                "macOS allows at most {} macOS VMs at the same time – running: {} (plan BM16)",
                Self::MAX_RUNNING,
                running.join(", ")
            ));
        }
        self.check_entitlement()?;
        if options.recovery && c.ephemeral {
            return Err(vm_err!(
                "\"{}\" starts from its image every time – what is changed in Recovery would be gone at the next \
                 start. Use a VM that keeps its disk",
                c.name
            ));
        }
        if c.ephemeral && ip.vm().exists() {
            self.reset(c)?;
        } else if !ip.vm().exists() {
            self.clone_image(c)?;
        }
        let networks = crate::networks::Networks::new(self.paths.clone()).prepare(&c.name)?;
        let args = self.runner_arguments(c, options, &networks)?;
        self.run_runner(c, &args, Duration::from_secs(30))
    }

    /// Shutdown via SSH (`sudo -n /sbin/shutdown -h now`, allowed by darwin-guest.nix; a stop request from the
    /// runner hangs on the guest's confirmation dialog when a user is logged in, plan BM17) → runner stop request
    /// → after `timeout` the runner forces the VM off → SIGKILL.
    /// A clean shutdown: `sudo shutdown -h now` over SSH (plan BM17). When SSH cannot ask (no address, refused,
    /// sudo without the password), the runner's `stop` (Virtualization.framework's request, the power button) gets
    /// `REQUEST_WAIT`; a guest still running after `timeout` (or that wait) is forced off – `kill`, then SIGKILL. A
    /// forced end is a line in runner.log (plan B165 round-up).
    fn stop(&self, c: &InstanceConfig, timeout: Duration) -> Result<StopResult> {
        let Some(pid) = self.pid(c) else { return Ok(StopResult::AlreadyStopped) };
        let socket = self.paths.instance(&c.name).runner_socket();
        let asked = match self.address(c) {
            Some(host) => {
                let shutdown = ["sudo", "-n", "/sbin/shutdown", "-h", "now"].map(String::from);
                run_ok(&self.ssh_arguments_to(c, &host, &shutdown, true))
            }
            None => false,
        };
        let wait = if asked {
            timeout
        } else {
            let _ = RunnerClient::send("stop", &socket, RunnerClient::DEFAULT_TIMEOUT);
            timeout.min(Self::REQUEST_WAIT)
        };
        if spawn::wait_for_exit(pid, wait) {
            return Ok(StopResult::Powerdown);
        }
        let why = if asked {
            format!("still running {} s after the shutdown over SSH", wait.as_secs())
        } else {
            "the shutdown could not be asked over SSH".to_string()
        };
        let result = if RunnerClient::send("kill", &socket, RunnerClient::DEFAULT_TIMEOUT).is_ok()
            && spawn::wait_for_exit(pid, Duration::from_secs(10))
        {
            StopResult::ForcedOff
        } else {
            spawn::send_signal(pid, libc::SIGKILL);
            spawn::wait_for_exit(pid, Duration::from_secs(5));
            StopResult::Killed
        };
        note_in_runner_log(&self.paths.instance(&c.name).runner_log(), &format!("omaflake: {result} – {why}"));
        Ok(result)
    }

    fn status(&self, c: &InstanceConfig) -> VmStatus {
        if self.pid(c).is_none() { VmStatus::Stopped } else { VmStatus::Running }
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
            if let Some(host) = self.address(c)
                && Ports::ssh_banner_any(&host, 22, sockets::DEFAULT_TIMEOUT).is_some()
            {
                self.apply_host_name(c, &host);
                // Always: the agent carries the diagnosis (plan B162); a start that failed here is retried and logged
                // by the core service's looks (`ensure_clipboard_agents`)
                self.start_clipboard_agent(c, &host);
                return true;
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

/// The shell command that starts a macOS guest's helpers for `c`: the clipboard helper when the clipboard is on, the
/// omaflake agent always where the image has it (it carries MIDI and the diagnosis; older images: nothing to start),
/// and what shows the VM's volumes in the Finder (plan B189, B190).
/// It ends with a check that what it started runs (after a second: the clipboard helper exits at once without its
/// port), so ssh's exit status tells whether the start took.
fn guest_helpers_command(c: &InstanceConfig) -> String {
    const CLIPBOARD: &str = "/run/current-system/sw/bin/omaflake-clipboard-agent";
    const AGENT: &str = "/run/current-system/sw/bin/omaflake-agent-start";
    const SESSION: &str = "/run/current-system/sw/bin/omaflake-volumes-session";
    let mut parts = Vec::new();
    if c.clipboard_on() {
        parts.push(CLIPBOARD.to_string());
    }
    parts.push(format!("{{ [ ! -x {AGENT} ] || {AGENT}; }}"));
    // The VM's volumes in the user's session (plan B189, B190; images from this fix on): FSKit volumes in unchained
    // images, favorites in the Finder's sidebar otherwise – in the background, it waits for the volumes to be mounted;
    // over SSH, since a launch agent may not add favorites under /Volumes and brings a notice at every start (BM25)
    parts.push(format!("{{ [ ! -x {SESSION} ] || nohup {SESSION} >/dev/null 2>&1 </dev/null & }}"));
    parts.push("sleep 1".into());
    let mut checks = Vec::new();
    if c.clipboard_on() {
        // "[.]": this shell's own command line must not hold the helper's name – the helper's start script asks
        // `pgrep -f clipboard-agent-macos.py` whether one runs and would find this shell and not start (found by the
        // maintainer's diagnosis, 0.7.4)
        checks.push("pgrep -f 'clipboard-agent-macos[.]py' >/dev/null".to_string());
    }
    checks.push(format!("{{ [ ! -x {AGENT} ] || pgrep -xq omaflake-agent; }}"));
    parts.push(checks.join(" && "));
    parts.join("; ")
}

/// Appends a line with the time to a VM's runner.log (best effort)
fn note_in_runner_log(file: &Path, line: &str) {
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(file) {
        let _ = writeln!(f, "[{}] {line}", omaflake_sdk::time::local_now("%Y-%m-%d %H:%M:%S"));
    }
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
            omaflake_sdk::time::parse_date("1970-01-01T00:00:00Z").unwrap(),
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
        let info = |recipe: &str, kind, system: &str| {
            ImageInfo::new(recipe, None, kind, system, "", "abc", None, omaflake_sdk::time::now())
        };
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

    /// Plan B165 round-up: a guest that cannot be asked over SSH (no network here) and does not end on the runner's
    /// request is forced off, and runner.log says why
    #[test]
    fn a_guest_that_cannot_be_asked_is_forced_off() {
        let (_t, p) = temp_paths();
        let vz = backend(&p, "/usr/bin/false");
        let c = mac_config("m1", NetworkMode::None);
        let ip = p.instance(&c.name);
        std::fs::create_dir_all(&ip.dir).unwrap();
        std::fs::create_dir_all(&ip.run).unwrap();
        // A process whose path names omaf-runner: a copy of sleep
        let fake = p.root.join("omaf-runner");
        std::fs::create_dir_all(&fake).unwrap();
        std::fs::copy("/bin/sleep", fake.join("sleep")).unwrap();
        // A copied system binary needs a signature of its own on macOS
        if cfg!(target_os = "macos") {
            let signed = std::process::Command::new("/usr/bin/codesign")
                .args(["-f", "-s", "-"])
                .arg(fake.join("sleep"))
                .output()
                .is_ok_and(|o| o.status.success());
            if !signed {
                eprintln!("skipped: codesign could not sign the stand-in runner");
                return;
            }
        }
        let mut child = std::process::Command::new(fake.join("sleep")).arg("30").spawn().unwrap();
        std::fs::write(ip.runner_pid(), format!("{}\n", child.id())).unwrap();
        assert_eq!(vz.status(&c), VmStatus::Running);
        let started = Instant::now();
        let result = vz.stop(&c, Duration::from_secs(60)).unwrap();
        let _ = child.wait();
        // No socket for `kill`: SIGKILL – after the short wait for the request, not the whole timeout
        assert_eq!(result, StopResult::Killed);
        assert!(started.elapsed() < Duration::from_secs(40), "{:?}", started.elapsed());
        let log = std::fs::read_to_string(ip.runner_log()).unwrap();
        assert!(log.contains("omaflake: killed (SIGKILL) – the shutdown could not be asked over SSH"), "{log}");
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
        assert!(
            missing
                .message()
                .starts_with("Image missing: mac-minimal-abc-12345678 – rebuild it (omaf build mac-minimal)")
        );
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

    /// A larger disk (plan B164): disk.img grows (sparse) to the instance's size, config.json says so; never smaller
    #[test]
    fn a_macos_disk_only_grows() {
        let (_t, p) = temp_paths();
        let mut c = mac_config("m2", NetworkMode::Shared);
        // Not cloned yet: nothing to change, no error
        VzBackend::apply_resources(&p, &c).unwrap();
        mac_image(&p, &c.image);
        let dir = p.instance("m2").vm();
        std::fs::create_dir_all(&dir).unwrap();
        for f in ["config.json", "disk.img"] {
            std::fs::copy(p.image_dir(&c.image).join(f), dir.join(f)).unwrap_or_else(|_| {
                std::fs::write(dir.join(f), "").unwrap();
                0
            });
        }
        let disk = dir.join("disk.img");
        std::fs::OpenOptions::new().write(true).open(&disk).unwrap().set_len(80 << 30).unwrap();
        c.disk_gb = 120;
        VzBackend::apply_resources(&p, &c).unwrap();
        assert_eq!(std::fs::metadata(&disk).unwrap().len(), 120 << 30);
        assert_eq!(MacVmBundle::new(&dir).load_config().unwrap().disk_gb, 120);
        c.disk_gb = 40;
        VzBackend::apply_resources(&p, &c).unwrap();
        assert_eq!(std::fs::metadata(&disk).unwrap().len(), 120 << 30, "never smaller");
    }

    #[test]
    fn runner_arguments_follow_the_settings() {
        let (_t, p) = temp_paths();
        let vz = backend(&p, "/usr/bin/false");
        let mut c = mac_config("m1", NetworkMode::None);
        c.microphone = Some(true);
        c.clipboard = Some(false);
        c.set_volumes(vec![crate::models::VolumeMount::new("m1", None)]);
        let ip = p.instance("m1");
        let a = vz.runner_arguments(&c, StartOptions::new(DisplayMode::Window), &[]).unwrap();
        let s = |x: &Path| x.to_string_lossy().into_owned();
        let mut expected = vec!["run".to_string(), s(&ip.vm()), "--socket".into(), s(&ip.runner_socket())];
        expected.extend(["--pid-file".into(), s(&ip.runner_pid()), "--window".into(), "--no-network".into()]);
        expected.extend(["--audio".into(), "--microphone".into(), "--agent-state".into(), s(&ip.agent_state())]);
        expected.extend(["--volume".into(), format!("m1={}", s(&p.share("m1")))]);
        assert_eq!(a, expected);
        assert!(p.share("m1").is_dir());
        let headless =
            vz.runner_arguments(&mac_config("m1", NetworkMode::Shared), StartOptions::new(DisplayMode::None), &[]);
        let headless = headless.unwrap();
        assert!(!headless.contains(&"--window".to_string()) && headless.contains(&"--clipboard".to_string()));
        assert!(!headless.contains(&"--midi".to_string()));
        // Recovery (plan B184): in a window, whatever the display says
        let recovery = StartOptions { recovery: true, ..StartOptions::new(DisplayMode::None) };
        let a = vz.runner_arguments(&mac_config("m1", NetworkMode::Shared), recovery, &[]).unwrap();
        assert!(a.contains(&"--window".to_string()) && a.contains(&"--recovery".to_string()));
        assert!(!headless.contains(&"--recovery".to_string()));
        c.midi = Some(true);
        let a = vz.runner_arguments(&c, StartOptions::new(DisplayMode::None), &[]).unwrap();
        let at = a.iter().position(|x| x == "--agent-state").unwrap();
        assert_eq!(a[at + 2..at + 5], ["--midi", "--vm-name", "m1"]);
        // A card per network, last
        let networks = crate::networks::Networks::new(p.clone());
        networks.create("dev").unwrap();
        networks.join("dev", "m1").unwrap();
        let cards = networks.attachments("m1");
        let a = vz.runner_arguments(&c, StartOptions::new(DisplayMode::None), &cards).unwrap();
        assert_eq!(
            a[a.len() - 6..],
            [
                "--lan-mac".to_string(),
                "02:6f:6d:01:0a:00".into(),
                "--lan-socket".into(),
                s(&ip.network_socket(0)),
                "--lan-switch".into(),
                s(&networks.vz_socket("dev")),
            ]
        );
    }

    #[test]
    fn guest_helpers_start_the_agent_always_and_check_they_run() {
        let agent = "{ [ ! -x /run/current-system/sw/bin/omaflake-agent-start ] || /run/current-system/sw/bin/omaflake-agent-start; }";
        let agent_runs = "{ [ ! -x /run/current-system/sw/bin/omaflake-agent-start ] || pgrep -xq omaflake-agent; }";
        let fav = "/run/current-system/sw/bin/omaflake-volumes-session";
        let agent = format!("{agent}; {{ [ ! -x {fav} ] || nohup {fav} >/dev/null 2>&1 </dev/null & }}");
        let mut c = mac_config("m1", NetworkMode::Shared);
        assert_eq!(
            guest_helpers_command(&c),
            format!(
                "/run/current-system/sw/bin/omaflake-clipboard-agent; {agent}; sleep 1; pgrep -f 'clipboard-agent-macos[.]py' >/dev/null && {agent_runs}"
            )
        );
        c.midi = Some(true);
        c.clipboard = Some(false);
        assert_eq!(guest_helpers_command(&c), format!("{agent}; sleep 1; {agent_runs}"));
        // The command never holds the helper's name as such: its start script's `pgrep -f clipboard-agent-macos.py`
        // would take this very shell for a running helper
        c.clipboard = Some(true);
        assert!(!guest_helpers_command(&c).contains("clipboard-agent-macos.py"));
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
        // The socket may say "running" before the script got to its first line (a busy machine): wait for the line
        let deadline = Instant::now() + Duration::from_secs(5);
        while !std::fs::read_to_string(ip.runner_log()).unwrap_or_default().starts_with("starting\n")
            && Instant::now() < deadline
        {
            std::thread::sleep(Duration::from_millis(20));
        }
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
