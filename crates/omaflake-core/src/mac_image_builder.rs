// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of MacImageBuilder.swift.
//!
//! Builds a macOS image from a base and a nix-darwin recipe (spec §6.5, plan M9/M10; formerly
//! `scripts/macos-build-image.sh`):
//!   build the system closure on the Mac → clone the base (new identity) → start headless → install Nix →
//!   nix copy → activate → shut down → `images/<name>.omaflake-image` (read-only) + `current/<line>`
//!
//! The guest user's password is needed once for sudo. It is never written to a file or passed as an argument:
//! it goes to `sudo -S` on stdin, which installs a temporary sudo rule that is removed again at the end.
//! Cancelling (the `&Cancellation` passed to `build`) forces the build VM off at once (`RunnerClient` "kill", else
//! SIGTERM) – its runner is detached and would outlive omaf otherwise.
//!
//! Not 1:1: Swift found the cancellation token as a task-local value; here `build` hands it to every `Shell` call
//! (also inside `BuildVm`). The handler that forces the build VM off is disarmed when the build ends (Swift left it
//! registered on the token). `copy_closure`'s watchdog stops a stalled `nix copy` through a token of its own (so
//! a PID is never signalled after it was reaped), and its error carries the last lines nix printed – Swift's
//! `stderr` was always empty there, as streamed output goes to the log callback.

use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::cancellation::Cancellation;
use crate::error::{codes, IoContext, Result};
use crate::guest_settings::BuildChoice;
use crate::image_store::{Image, ImageInfo, ImageStore};
use crate::ipsw_downloader::KeepAwake;
use crate::json;
use crate::mac_base_installer::MacBaseInstaller;
use crate::mac_vm::{DhcpLeases, MacVmBundle, RunnerClient};
use crate::models::GuestKind;
use crate::paths::{Paths, Settings};
use crate::platform::Platform;
use crate::qemu_runtime::{tail, FileLock};
use crate::recipe_sources::RecipeLibrary;
use crate::shell::{RunOptions, Shell, ShellResult, StreamOptions};
use crate::sockets::{self, Ports};
use crate::spawn::{self, Pid};
use crate::trash::Disposal;
use crate::vm_err;
use crate::vz_backend::VzBackend;

/// A log callback that may borrow (`crate::Log` is `'static`-bounded as a type alias)
type LogFn<'a> = dyn Fn(&str) + Send + Sync + 'a;

#[derive(Debug, Clone)]
pub struct MacImageBuilder {
    pub paths: Paths,
    pub vz: VzBackend,
}

/// What to build (Swift `MacImageBuilder.Request`)
#[derive(Debug, Clone)]
pub struct BuildRequest {
    pub recipe: String,
    pub flake: String,
    /// Guest user of the base (the recipe's `user`)
    pub user: String,
    pub password: String,
    /// None = newest `bases/mac-base-*`
    pub base: Option<PathBuf>,
    /// Keep a failed build folder for inspection instead of deleting it
    pub keep_failed: bool,
    /// The credential and look of the image (plan B137); the macOS guest user stays the base's
    pub choice: BuildChoice,
}

impl BuildRequest {
    /// Swift defaults: the newest base, a failed build is deleted
    pub fn new(recipe: &str, flake: &str, user: &str, password: &str) -> Self {
        BuildRequest {
            recipe: recipe.into(),
            flake: flake.into(),
            user: user.into(),
            password: password.into(),
            base: None,
            keep_failed: false,
            choice: BuildChoice::default(),
        }
    }
}

impl MacImageBuilder {
    pub fn new(paths: Paths, vz: VzBackend) -> Self {
        MacImageBuilder { paths, vz }
    }

    /// Newest base (`bases/mac-base-<os-build>` with the ready marker), None if there is none.
    pub fn newest_base(&self) -> Option<PathBuf> {
        Self::newest_base_in(&self.paths)
    }

    /// Swift `static newestBase(_:)`: names starting with "mac-base-", sorted descending, the first that
    /// `MacBaseInstaller::is_ready`
    pub fn newest_base_in(paths: &Paths) -> Option<PathBuf> {
        let mut names: Vec<String> = fs::read_dir(paths.bases())
            .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
            .unwrap_or_default();
        names.retain(|n| n.starts_with("mac-base-"));
        names.sort();
        names.iter().rev().map(|n| paths.bases().join(n)).find(|dir| MacBaseInstaller::is_ready(dir))
    }

    /// Runs the whole pipeline (keeping the Mac awake); progress goes to `log`, details (guest output) to
    /// `logs/build-<name>.log`. One build per image at a time (`FileLock` `images/.build-<name>.lock`, removed when
    /// done). Cancelled: "Build cancelled".
    pub fn build(&self, r: &BuildRequest, cancel: &Cancellation, log: &LogFn<'_>) -> Result<Image> {
        KeepAwake::during("Building a macOS image", || self.build_awake(r, cancel, log))
    }

    fn build_awake(&self, r: &BuildRequest, cancel: &Cancellation, log: &LogFn<'_>) -> Result<Image> {
        let paths = &self.paths;
        let images = ImageStore::new(paths.clone());
        let base = r
            .base
            .clone()
            .or_else(|| self.newest_base())
            .filter(|b| MacVmBundle::new(b).config().exists())
            .ok_or_else(|| {
                vm_err!(
                    codes::PREREQUISITE,
                    "No base found (expected {}/mac-base-<os-build>) – create one with: omaf macos install",
                    paths.bases().display()
                )
            })?;
        self.vz.check_entitlement()?;
        if let Some(base_user) = MacBaseInstaller::user_of(&base, &paths.logs()) {
            if base_user != r.user {
                return Err(vm_err!(
                    codes::PREREQUISITE,
                    "{} has the guest user {base_user}, but the recipe expects {} (macUser in your recipes' local \
                         settings) – make them match",
                    file_name(&base),
                    r.user
                ));
            }
        }
        let flake_ref = ImageStore::flake_ref(&r.flake);
        let cancelled = || vm_err!(codes::CANCELLED, "Build cancelled");

        // 1. System closure on the Mac – fails early, before a VM is started
        log(&format!("Building darwinConfigurations.{} on the Mac", r.recipe));
        let mut args: Vec<String> = vec![
            "build".into(),
            format!("{flake_ref}#darwinConfigurations.\"{}\".system", r.recipe),
            "--no-link".into(),
            "--print-out-paths".into(),
            "-L".into(),
        ];
        args.extend(images.guest_arguments_for(&r.flake, &r.choice));
        let built = Shell::stream("nix", &args, &StreamOptions { cancel: Some(cancel), ..Default::default() }, log)
            .map_err(|e| if cancel.is_cancelled() { cancelled() } else { e })?;
        if cancel.is_cancelled() {
            return Err(cancelled());
        }
        let system = built.stdout.split('\n').rfind(|l| !l.is_empty()).unwrap_or("").to_string();
        if built.status != 0 || !system.starts_with("/nix/store/") {
            return Err(vm_err!("nix build of darwinConfigurations.{} failed ({}) – see log", r.recipe, built.status));
        }
        let name = ImageStore::image_name(&r.recipe, &system);
        let image = paths.image_dir(&name);
        if image.exists() {
            log(&format!("Image exists already: {}", image.display()));
            images.set_current(&name)?;
            return images.image(&name);
        }

        // 2. Clone the base, start it headless
        let work = work_folder(paths, &name);
        // One build per image at a time (app and CLI): the lock lives next to the folder and ends with this build
        let _lock = FileLock::new(
            &paths.images().join(format!(".build-{name}.lock")),
            vm_err!(codes::IN_USE, "A build of {name} is already running (app or CLI) – wait for it or cancel it"),
            true,
        )?;
        if work.exists() {
            // Not locked, so no build is using it: left over from a crash or kept with --keep-failed
            log(&format!("Deleting the leftover build folder {}", file_name(&work)));
            Disposal::delete(&[&work])?;
        }
        fs::create_dir_all(paths.logs()).with_path(&paths.logs())?;
        let log_file = paths.logs().join(format!("build-{name}.log"));
        log(&format!("Cloning {} (log: {})", file_name(&base), log_file.display()));
        let with_cancel = RunOptions { cancel: Some(cancel), ..Default::default() };
        Shell::check(&self.vz.runner, &["clone".into(), path_string(&base), path_string(&work)], &with_cancel)
            .map_err(|e| if cancel.is_cancelled() { cancelled() } else { e })?;
        // The socket lives in a short run folder: `.build-<name>/runner.sock` with a recipe ID of twelve words is longer
        // than a Unix socket path may be (103 bytes)
        let run = build_run_folder(paths, &name);
        fs::create_dir_all(&run).with_path(&run)?;
        let socket = run.join("runner.sock");
        let pid_file = work.join("runner.pid");
        let run_args = ["run".into(), path_string(&work), "--socket".into(), path_string(&socket)];
        let run_args: Vec<String> = run_args.into_iter().chain(["--pid-file".into(), path_string(&pid_file)]).collect();
        let child = spawn::spawn_detached(&self.vz.runner, &run_args, &log_file, &[])?;
        // Cancelling forces the build VM off at once (its runner is detached and would outlive omaf otherwise)
        let armed = Arc::new(AtomicBool::new(true));
        {
            let (armed, socket) = (armed.clone(), socket.clone());
            cancel.on_cancel(move || {
                if armed.load(Ordering::SeqCst)
                    && RunnerClient::send("kill", &socket, RunnerClient::DEFAULT_TIMEOUT).is_err()
                {
                    spawn::send_signal(child, libc::SIGTERM);
                }
            });
        }
        // Disarmed when this build ends, however it ends
        struct Disarm(Arc<AtomicBool>);
        impl Drop for Disarm {
            fn drop(&mut self) {
                self.0.store(false, Ordering::SeqCst);
            }
        }
        let _disarm = Disarm(armed);

        let mut vm = BuildVm::new(child, &work, socket.clone(), &r.user, log_file.clone(), cancel.clone());
        if let Err(e) = self.provision(&mut vm, r, &system, cancel, log) {
            vm.force_off();
            let was_cancelled = cancel.is_cancelled();
            if r.keep_failed && !was_cancelled {
                log(&format!("Kept for inspection: {}", work.display()));
            } else if work.exists() {
                let _ = Disposal::delete(&[&work]);
                log(&if was_cancelled {
                    "Build cancelled – its folder was deleted".to_string()
                } else {
                    format!("Failed build deleted (log: {}; --keep-failed keeps it for inspection)", log_file.display())
                });
            }
            let _ = Disposal::delete(&[&run]);
            return Err(if was_cancelled { cancelled() } else { e });
        }

        // 7. Store as image: without the runner's runtime files, read-only, current/<recipe>
        let _ = Disposal::delete(&[&run, &pid_file]);
        let settings = Settings::load(paths);
        let library = RecipeLibrary::new(paths.clone());
        let recipe_source = library.available(&settings).into_iter().find(|a| a.id == r.recipe).map(|a| a.source);
        let mut info = ImageInfo::new(
            &r.recipe,
            recipe_source,
            GuestKind::Macos,
            Platform::host_system(GuestKind::Macos).unwrap_or(""),
            &r.flake,
            &images.recipe_revision(&r.recipe, &r.flake),
            Some(system.clone()),
            json::now(),
        );
        info.set_choice(&r.choice);
        ImageStore::write_info(&info, &work)?;
        fs::rename(&work, &image).with_path(&image)?;
        Shell::check(
            &crate::platform::host_tool("/bin/chmod"),
            &["-R", "a-w", &path_string(&image)],
            &RunOptions::default(),
        )?;
        images.set_current(&name)?;
        library.keep(&r.recipe, &settings);
        library.prune_versions();
        log(&format!("Image ready: {}", image.display()));
        images.image(&name)
    }

    /// Nix's macOS release tarball in <data>/cache, checked against the SHA-256 that nixos.org publishes next to it.
    pub(crate) fn nix_tarball(&self, version: &str, cancel: &Cancellation, log: &LogFn<'_>) -> Result<PathBuf> {
        let name = format!("nix-{version}-aarch64-darwin.tar.xz");
        let base = format!("https://releases.nixos.org/nix/nix-{version}/{name}");
        self.cached_download(&base, &name, cancel, log)
    }

    /// `<base>` into `cache/<name>` unless the file there has the SHA-256 published at `<base>.sha256`
    fn cached_download(&self, base: &str, name: &str, cancel: &Cancellation, log: &LogFn<'_>) -> Result<PathBuf> {
        let cache = self.paths.cache();
        let file = cache.join(name);
        fs::create_dir_all(&cache).with_path(&cache)?;
        let opts = RunOptions { cancel: Some(cancel), ..Default::default() };
        let expected = Shell::check("/usr/bin/curl", &["-fsSL", &format!("{base}.sha256")], &opts)?.trim().to_string();
        let sum = || {
            Shell::check("/usr/bin/shasum", &["-a", "256", &path_string(&file)], &opts)
                .ok()
                .and_then(|out| out.split(' ').next().map(str::to_string))
        };
        if sum().as_deref() != Some(expected.as_str()) {
            log(&format!("Downloading {name}"));
            Shell::check("/usr/bin/curl", &["-fsSL", "-o", &path_string(&file), base], &opts)?;
            if sum().as_deref() != Some(expected.as_str()) {
                let _ = Disposal::delete(&[&file]);
                return Err(vm_err!("{name}: checksum does not match nixos.org's"));
            }
        }
        Ok(file)
    }

    /// `nix copy` into the build VM with a watchdog: a copy of ~9 GiB over ssh-ng once stood still after 632 MB with
    /// both sides idle (plan B75). No new path for `stall` (Swift default 300 s) → stop and copy again; paths already
    /// in the guest are skipped, later attempts use the older ssh:// protocol.
    pub(crate) fn copy_closure(
        &self,
        system: &str,
        stores: &[String],
        env: &[(&str, &str)],
        stall: Duration,
        cancel: &Cancellation,
        log: &LogFn<'_>,
    ) -> Result<()> {
        self.copy_closure_with("nix", system, stores, env, stall, cancel, log)
    }

    /// `copy_closure` with another `nix` (tests)
    #[allow(clippy::too_many_arguments)]
    fn copy_closure_with(
        &self,
        nix: &str,
        system: &str,
        stores: &[String],
        env: &[(&str, &str)],
        stall: Duration,
        cancel: &Cancellation,
        log: &LogFn<'_>,
    ) -> Result<()> {
        struct Watch {
            last: Instant,
            copied: usize,
            stalled: bool,
            /// The last lines nix printed (for the error)
            output: String,
        }
        // Swift checked every 30 s; a shorter stall (tests) is checked as often as it lasts
        let tick = stall.min(Duration::from_secs(30));
        for (attempt, store) in stores.iter().enumerate() {
            cancel.check()?;
            let watch = Mutex::new(Watch { last: Instant::now(), copied: 0, stalled: false, output: String::new() });
            // A token per attempt: the watchdog stops this copy without cancelling the build
            let stop = Cancellation::new();
            {
                let stop = stop.clone();
                cancel.on_cancel(move || stop.cancel());
            }
            let done = AtomicBool::new(false);
            let r = std::thread::scope(|s| {
                s.spawn(|| {
                    let mut next = Instant::now() + tick;
                    while !done.load(Ordering::SeqCst) {
                        std::thread::sleep(Duration::from_millis(50).min(tick));
                        if Instant::now() < next {
                            continue;
                        }
                        next = Instant::now() + tick;
                        let mut w = watch.lock().unwrap_or_else(|e| e.into_inner());
                        if !w.stalled && w.last.elapsed() > stall {
                            w.stalled = true;
                            stop.cancel();
                        }
                    }
                });
                let on_line = |line: &str| {
                    let n = {
                        let mut w = watch.lock().unwrap_or_else(|e| e.into_inner());
                        w.last = Instant::now();
                        w.output.push_str(line);
                        w.output.push('\n');
                        if w.output.len() > 4000 {
                            let cut = w.output.len() - 2000;
                            let cut = (cut..w.output.len()).find(|i| w.output.is_char_boundary(*i)).unwrap_or(0);
                            w.output.drain(..cut);
                        }
                        if line.starts_with("copying path") {
                            w.copied += 1;
                            Some(w.copied)
                        } else {
                            None
                        }
                    };
                    if let Some(n) = n.filter(|n| n % 100 == 0) {
                        log(&format!("… {n} paths copied"));
                    }
                };
                let args = ["copy", "-v", "--no-check-sigs", "--to", store, system];
                let r = Shell::stream(
                    nix,
                    &args,
                    &StreamOptions { environment: env, cancel: Some(&stop), ..Default::default() },
                    &on_line,
                );
                done.store(true, Ordering::SeqCst);
                r
            })?;
            if r.status == 0 {
                return Ok(());
            }
            cancel.check()?;
            let w = watch.into_inner().unwrap_or_else(|e| e.into_inner());
            if !w.stalled {
                let start = w.output.len().saturating_sub(800);
                let start = (start..=w.output.len()).find(|i| w.output.is_char_boundary(*i)).unwrap_or(0);
                return Err(vm_err!("Copying into the build VM failed ({}):\n{}", r.status, &w.output[start..]));
            }
            if attempt + 1 < stores.len() {
                log(&format!(
                    "No progress for {} minutes – copying again (attempt {} of {})",
                    stall.as_secs() / 60,
                    attempt + 2,
                    stores.len()
                ));
            }
        }
        Err(vm_err!("Copying into the build VM stood still {} times", stores.len()))
    }

    /// Steps 3–6 inside the running build VM.
    fn provision(
        &self,
        vm: &mut BuildVm,
        r: &BuildRequest,
        system: &str,
        cancel: &Cancellation,
        log: &LogFn<'_>,
    ) -> Result<()> {
        log("Waiting for the VM");
        let mac = MacVmBundle::new(&vm.dir).load_config()?.mac_address;
        let deadline = Instant::now() + Duration::from_secs(600);
        let ip = loop {
            cancel.check()?;
            if vm.exited() {
                return Err(vm_err!("omaf-runner exited:\n{}", tail(&vm.log_file, 3)));
            }
            let ip = DhcpLeases::ip(&mac);
            if Instant::now() >= deadline {
                // What was missing – the address, the open port or SSH's greeting (plan B61)
                let port = match &ip {
                    Some(a) if Ports::is_open(a, 22, sockets::DEFAULT_TIMEOUT) => "port 22 open but no SSH greeting",
                    Some(_) => "port 22 closed",
                    None => "",
                };
                let what = match &ip {
                    Some(a) => format!("address {a}, {port}"),
                    None => format!("no DHCP lease for {mac}"),
                };
                return Err(vm_err!("No SSH after 10 minutes ({what})"));
            }
            match ip {
                Some(a) if Ports::ssh_banner(&a, 22, sockets::DEFAULT_TIMEOUT).is_some() => break a,
                _ => std::thread::sleep(Duration::from_secs(5)),
            }
        };
        vm.ip = Some(ip.clone());
        log(&format!("VM at {ip}"));
        let mut logged_in = false;
        for _ in 0..30 {
            cancel.check()?;
            logged_in = vm.ssh("true", None).is_ok_and(|res| res.status == 0);
            if logged_in {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        if !logged_in {
            return Err(vm_err!(
                codes::AUTH,
                "SSH login as {} with your key failed (is your public key in the base? docs/macos-base.md)",
                r.user
            ));
        }

        // 3. Temporary sudo without a password (removed in step 6)
        let rule = "/etc/sudoers.d/zz-omaflake-build";
        let password = format!("{}\n", r.password);
        let sudo = vm.ssh(
            &format!("sudo -S -p '' sh -c 'echo \"{} ALL=(ALL) NOPASSWD: ALL\" > {rule} && chmod 440 {rule}'", r.user),
            Some(password.as_bytes()),
        )?;
        if sudo.status != 0 {
            return Err(vm_err!(codes::AUTH, "sudo in the guest failed (wrong password?)"));
        }

        // 4. Nix (same version as on the Mac), the guest user trusted for nix copy
        if vm.ssh("test -x /nix/var/nix/profiles/default/bin/nix", None)?.status != 0 {
            let version_line =
                Shell::check("nix", &["--version"], &RunOptions { cancel: Some(cancel), ..Default::default() })?;
            let version = version_line.split(' ').next_back().map(|v| v.trim().to_string()).unwrap_or_default();
            log(&format!("Installing Nix {version} in the guest (a few minutes)"));
            // The Mac brings the release tarball (cached, checksum from nixos.org): a fresh macOS guest downloads its
            // aerial wallpapers and assets meanwhile, and Nix's own download crawled (plan B76).
            // No channel: images use flakes; fetching nixpkgs-unstable as a channel took minutes (plan B74)
            let tarball = self.nix_tarball(&version, cancel, log)?;
            let name = format!("nix-{version}-aarch64-darwin");
            let data = fs::read(&tarball).with_path(&tarball)?;
            let put = vm.ssh(&format!("cat > /tmp/{name}.tar.xz"), Some(&data))?;
            if put.status != 0 {
                return Err(vm_err!("Copying the Nix tarball into the guest failed"));
            }
            vm.check(
                &format!("cd /tmp && tar -xJf {name}.tar.xz && cd {name} && ./install --daemon --yes --no-channel-add"),
                "Nix installation",
            )?;
        }
        vm.check(
            &format!(
                "grep -q '^trusted-users' /etc/nix/nix.conf || echo 'trusted-users = root {}' | sudo tee -a /etc/nix/nix.conf \
                 >/dev/null; sudo launchctl kickstart -k system/org.nixos.nix-daemon",
                r.user
            ),
            "nix-daemon setup",
        )?;
        let store = format!("ssh-ng://{}@{ip}?remote-program=/nix/var/nix/profiles/default/bin/nix-daemon", r.user);
        let ssh_opts = BuildVm::SSH_OPTIONS.join(" ");
        let env = [("NIX_SSHOPTS", ssh_opts.as_str())];
        // The restarted daemon needs a moment; paths built on the Mac are unsigned and need a trusted user
        let mut trusted = false;
        for _ in 0..30 {
            cancel.check()?;
            let info = Shell::run(
                "nix",
                &["store", "info", "--json", "--store", &store],
                &RunOptions { environment: &env, cancel: Some(cancel), ..Default::default() },
            );
            trusted = info.is_ok_and(|i| i.stdout.contains("\"trusted\":true"));
            if trusted {
                break;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        if !trusted {
            return Err(vm_err!("The guest's nix-daemon does not trust {}", r.user));
        }

        // 5. Copy and activate
        log("Copying the closure");
        let legacy = format!("ssh://{}@{ip}?remote-program=/nix/var/nix/profiles/default/bin/nix-store", r.user);
        self.copy_closure(system, &[store, legacy.clone(), legacy], &env, Duration::from_secs(300), cancel, log)?;
        log("Activating");
        // nix-darwin takes over these files; the originals stay next to them (never delete)
        vm.check(
            "for f in /etc/nix/nix.conf /etc/bashrc /etc/zshrc; do [ -L \"$f\" ] || [ ! -e \"$f\" ] || sudo mv \"$f\" \"$f.before-nix-darwin\"; done",
            "moving /etc files aside",
        )?;
        vm.check(
            &format!(
                "sudo /nix/var/nix/profiles/default/bin/nix-env -p /nix/var/nix/profiles/system --set '{system}' && sudo '{system}/activate'"
            ),
            "Activation",
        )?;

        // 6. Remove the sudo rule, shut down
        vm.check(&format!("sudo rm -f {rule}"), "removing the temporary sudo rule")?;
        let _ = vm.ssh("sudo -n /sbin/shutdown -h now", None);
        log("Shutting down");
        let off = Instant::now() + Duration::from_secs(120);
        while !vm.exited() {
            cancel.check()?;
            if Instant::now() >= off {
                return Err(vm_err!("The build VM did not shut down"));
            }
            std::thread::sleep(Duration::from_secs(2));
        }
        Ok(())
    }
}

/// The headless build VM: its runner process (our child) and SSH into it. Swift `MacImageBuilder.BuildVM`.
#[derive(Debug)]
pub(crate) struct BuildVm {
    pub child: Pid,
    /// The build VM's folder (`images/.build-<name>`)
    pub dir: PathBuf,
    pub socket: PathBuf,
    pub user: String,
    pub log_file: PathBuf,
    pub ip: Option<String>,
    /// The build's token: `ssh` runs stop when it is cancelled
    cancel: Cancellation,
    done: bool,
}

impl BuildVm {
    /// ssh options for the build VM (also used by `MacBaseInstaller`)
    pub(crate) const SSH_OPTIONS: [&'static str; 10] = [
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "UserKnownHostsFile=/dev/null",
        "-o",
        "StrictHostKeyChecking=no",
        "-o",
        "LogLevel=ERROR",
    ];

    pub(crate) fn new(
        child: Pid,
        dir: &Path,
        socket: PathBuf,
        user: &str,
        log_file: PathBuf,
        cancel: Cancellation,
    ) -> Self {
        BuildVm { child, dir: dir.to_path_buf(), socket, user: user.into(), log_file, ip: None, cancel, done: false }
    }

    /// True once the runner has exited (reaps it, so no zombie is left).
    pub(crate) fn exited(&mut self) -> bool {
        if !self.done {
            self.done = spawn::reap_if_exited(self.child);
        }
        self.done
    }

    /// Runner "kill", else SIGTERM; after 15 s SIGKILL
    pub(crate) fn force_off(&mut self) {
        if self.exited() {
            return;
        }
        if RunnerClient::send("kill", &self.socket, RunnerClient::DEFAULT_TIMEOUT).is_err() {
            spawn::send_signal(self.child, libc::SIGTERM);
        }
        let deadline = Instant::now() + Duration::from_secs(15);
        while !self.exited() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(300));
        }
        if !self.exited() {
            spawn::send_signal(self.child, libc::SIGKILL);
            // reaped as soon as it is gone
            let deadline = Instant::now() + Duration::from_secs(5);
            while !self.exited() && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(50));
            }
        }
    }

    /// One command in the guest (remote shell line); stdout/stderr are appended to the build log.
    pub(crate) fn ssh(&self, command: &str, input: Option<&[u8]>) -> Result<ShellResult> {
        let target = format!("{}@{}", self.user, self.ip.as_deref().unwrap_or(""));
        let args: Vec<&str> = Self::SSH_OPTIONS.iter().copied().chain([target.as_str(), command]).collect();
        let r = Shell::run(
            &crate::platform::host_tool("/usr/bin/ssh"),
            &args,
            &RunOptions { input, cancel: Some(&self.cancel), ..Default::default() },
        )?;
        self.append_log(&format!("{}{}", r.stdout, r.stderr));
        Ok(r)
    }

    /// "<what> failed – see <log>" unless the command succeeds
    pub(crate) fn check(&self, command: &str, what: &str) -> Result<()> {
        if self.ssh(command, None)?.status != 0 {
            return Err(vm_err!("{what} failed – see {}", self.log_file.display()));
        }
        Ok(())
    }

    fn append_log(&self, text: &str) {
        if text.is_empty() {
            return;
        }
        // Like Swift's FileHandle(forWritingTo:): only an existing log (the runner's spawn created it)
        if let Ok(mut f) = OpenOptions::new().append(true).open(&self.log_file) {
            let _ = f.write_all(text.as_bytes());
        }
    }
}

/// The build VM's runtime folder (its runner socket): `run/build-<16 hex digits of the name's SHA-256>` – short, so the
/// socket path stays within the 103 bytes a Unix socket allows
fn build_run_folder(paths: &Paths, name: &str) -> PathBuf {
    use sha2::{Digest, Sha256};
    let hash = hex::encode(Sha256::digest(name.as_bytes()));
    paths.run().join(format!("build-{}", &hash[..16]))
}

/// The build VM's folder while it is built: `images/.build-<name>`
pub(crate) fn work_folder(paths: &Paths, name: &str) -> PathBuf {
    paths.images().join(format!(".build-{name}"))
}

fn path_string(p: &Path) -> String {
    p.to_string_lossy().into_owned()
}

fn file_name(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::instance_store::InstanceStore;
    use crate::mac_vm::MacVmConfig;
    use crate::paths::tests::temp_paths;
    use std::os::unix::fs::PermissionsExt;

    fn builder(p: &Paths) -> MacImageBuilder {
        let vz = VzBackend::new(p.clone(), InstanceStore::new(p.clone(), "/usr/bin/false"), "/usr/bin/false");
        MacImageBuilder::new(p.clone(), vz)
    }

    fn script(dir: &Path, name: &str, body: &str) -> String {
        fs::create_dir_all(dir).unwrap();
        let file = dir.join(name);
        fs::write(&file, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(&file, fs::Permissions::from_mode(0o755)).unwrap();
        path_string(&file)
    }

    fn collect() -> (Arc<Mutex<Vec<String>>>, impl Fn(&str) + Send + Sync) {
        let lines = Arc::new(Mutex::new(Vec::new()));
        let l = lines.clone();
        (lines, move |s: &str| l.lock().unwrap().push(s.to_string()))
    }

    // VZBackendTests.newestBaseSkipsFreshAndIncompleteFolders
    #[test]
    fn newest_base_skips_fresh_and_incomplete_folders() {
        let (_t, p) = temp_paths();
        let b = builder(&p);
        assert_eq!(b.newest_base(), None);
        let save = |dir: &Path, mac: &str| {
            fs::create_dir_all(dir).unwrap();
            MacVmBundle::new(dir).save(&MacVmConfig::new(4, 8192, 80, mac, None)).unwrap();
        };
        for name in ["mac-base-26A434", "mac-fresh-27B100", "mac-base-27A200"] {
            let dir = p.bases().join(name);
            save(&dir, "02:00:00:00:00:01");
            fs::write(dir.join(MacBaseInstaller::READY_MARKER), "").unwrap();
        }
        // a newer base without the ready marker (first boot unfinished) does not count
        save(&p.bases().join("mac-base-27Z999"), "02:00:00:00:00:02");
        // newer name without config.json (e.g. an interrupted clone) does not count
        fs::create_dir_all(p.bases().join("mac-base-28A1")).unwrap();
        assert_eq!(b.newest_base().unwrap().file_name().unwrap(), "mac-base-27A200");
        assert_eq!(MacImageBuilder::newest_base_in(&p), b.newest_base());
    }

    #[test]
    fn a_build_without_a_base_says_how_to_make_one() {
        let (_t, p) = temp_paths();
        let r = BuildRequest::new("omaflake.macos.minimal", "/f", "u", "pw");
        let e = builder(&p).build(&r, &Cancellation::new(), &|_| {}).unwrap_err();
        assert_eq!(e.code(), Some(codes::PREREQUISITE));
        assert!(e.message().starts_with("No base found (expected ") && e.message().ends_with("omaf macos install"));
    }

    #[test]
    fn copy_reports_every_hundred_paths() {
        let (_t, p) = temp_paths();
        let nix = script(
            &p.root.join("bin"),
            "nix",
            "i=0; while [ $i -lt 250 ]; do echo \"copying path '/nix/store/$i'\" >&2; i=$((i+1)); done",
        );
        let (lines, log) = collect();
        let stores = ["ssh-ng://u@h".to_string()];
        builder(&p)
            .copy_closure_with(&nix, "/nix/store/x", &stores, &[], Duration::from_secs(300), &Cancellation::new(), &log)
            .unwrap();
        assert_eq!(*lines.lock().unwrap(), vec!["… 100 paths copied", "… 200 paths copied"]);
    }

    #[test]
    fn a_stalled_copy_is_stopped_and_tried_again() {
        let (_t, p) = temp_paths();
        let nix = script(&p.root.join("bin"), "nix", "echo \"copying path '/nix/store/a'\" >&2; exec sleep 20");
        let (lines, log) = collect();
        let stores = ["ssh-ng://u@h".to_string(), "ssh://u@h".to_string()];
        let t0 = Instant::now();
        let e = builder(&p)
            .copy_closure_with(
                &nix,
                "/nix/store/x",
                &stores,
                &[],
                Duration::from_millis(500),
                &Cancellation::new(),
                &log,
            )
            .unwrap_err();
        assert_eq!(e.message(), "Copying into the build VM stood still 2 times");
        assert!(t0.elapsed() < Duration::from_secs(10));
        assert_eq!(*lines.lock().unwrap(), vec!["No progress for 0 minutes – copying again (attempt 2 of 2)"]);
    }

    #[test]
    fn a_failed_copy_shows_what_nix_said() {
        let (_t, p) = temp_paths();
        let nix = script(&p.root.join("bin"), "nix", "echo 'error: cannot connect to ssh-ng://u@h' >&2; exit 1");
        let stores = ["ssh-ng://u@h".to_string(), "ssh://u@h".to_string()];
        let e = builder(&p)
            .copy_closure_with(
                &nix,
                "/nix/store/x",
                &stores,
                &[],
                Duration::from_secs(300),
                &Cancellation::new(),
                &|_| {},
            )
            .unwrap_err();
        assert_eq!(e.message(), "Copying into the build VM failed (1):\nerror: cannot connect to ssh-ng://u@h\n");
    }

    #[test]
    fn a_cancelled_copy_stops_at_once() {
        let (_t, p) = temp_paths();
        let nix = script(&p.root.join("bin"), "nix", "exec sleep 20");
        let cancel = Cancellation::new();
        let c = cancel.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(300));
            c.cancel();
        });
        let t0 = Instant::now();
        let stores = ["ssh-ng://u@h".to_string()];
        let e = builder(&p)
            .copy_closure_with(&nix, "/nix/store/x", &stores, &[], Duration::from_secs(300), &cancel, &|_| {})
            .unwrap_err();
        assert!(e.is_cancelled() && t0.elapsed() < Duration::from_secs(5));
    }

    // curl and shasum of macOS (the Nix installer tarball for macOS guests)
    #[cfg(target_os = "macos")]
    #[test]
    fn downloads_are_cached_and_checked() {
        let (_t, p) = temp_paths();
        let src = p.root.join("src");
        fs::create_dir_all(&src).unwrap();
        fs::write(src.join("nix.tar.xz"), "tarball").unwrap();
        // shasum -a 256 of "tarball"
        let sum = Shell::check(
            "/usr/bin/shasum",
            &["-a", "256", &path_string(&src.join("nix.tar.xz"))],
            &RunOptions::default(),
        )
        .unwrap();
        fs::write(src.join("nix.tar.xz.sha256"), format!("{}\n", sum.split(' ').next().unwrap())).unwrap();
        let base = format!("file://{}", path_string(&src.join("nix.tar.xz")));
        let (lines, log) = collect();
        let b = builder(&p);
        let file = b.cached_download(&base, "nix-x.tar.xz", &Cancellation::new(), &log).unwrap();
        assert_eq!(file, p.cache().join("nix-x.tar.xz"));
        assert_eq!(fs::read_to_string(&file).unwrap(), "tarball");
        // cached: not downloaded again
        b.cached_download(&base, "nix-x.tar.xz", &Cancellation::new(), &log).unwrap();
        assert_eq!(*lines.lock().unwrap(), vec!["Downloading nix-x.tar.xz"]);
        // a file that does not match the published sum is deleted
        fs::write(src.join("nix.tar.xz.sha256"), "0000\n").unwrap();
        let e = b.cached_download(&base, "nix-x.tar.xz", &Cancellation::new(), &|_| {}).unwrap_err();
        assert_eq!(e.message(), "nix-x.tar.xz: checksum does not match nixos.org's");
        assert!(!file.exists());
    }

    #[test]
    fn the_build_vm_is_forced_off_and_reaped() {
        let (_t, p) = temp_paths();
        fs::create_dir_all(p.logs()).unwrap();
        let log_file = p.logs().join("build-x.log");
        let child = spawn::spawn_detached("sleep", &["30"], &log_file, &[]).unwrap();
        let mut vm = BuildVm::new(child, &p.root, p.root.join("none.sock"), "u", log_file.clone(), Cancellation::new());
        assert!(!vm.exited());
        vm.force_off();
        assert!(vm.exited());
        // ssh without an address fails; what ssh said goes to the build log
        let e = vm.check("true", "Nothing").unwrap_err();
        assert_eq!(e.message(), format!("Nothing failed – see {}", log_file.display()));
        assert!(!fs::read_to_string(&log_file).unwrap().is_empty());
    }

    #[test]
    fn work_folder_is_hidden_in_images() {
        let (_t, p) = temp_paths();
        assert_eq!(work_folder(&p, "x"), p.images().join(".build-x"));
    }

    #[test]
    fn the_build_socket_fits_a_unix_socket_path() {
        // A twelve-word recipe ID in the default data folder of a long user name
        let p = Paths::new(PathBuf::from("/Users/a-rather-long-user-name/.omaflake"), None);
        let name =
            "omaflake.macos.minimal@bring-action-column-pattern-before-long-proof-similar-warfare-napkin-mom-benefit";
        let socket = build_run_folder(&p, name).join("runner.sock");
        assert!(socket.as_os_str().len() <= 103, "{}", socket.display());
        assert_ne!(build_run_folder(&p, name), build_run_folder(&p, "omaflake.macos.minimal@other"));
    }
}
