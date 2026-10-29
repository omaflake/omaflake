// SPDX-License-Identifier: AGPL-3.0-or-later
//! Instances, images, settings, doctor and builder – port of Sources/omaf/Omaf.swift (the recipe list and build are in
//! recipes.rs).

use std::os::unix::fs::MetadataExt;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::time::{Duration, Instant};

use chrono::Utc;
use omaflake_core::builder_maintenance::BuilderMaintenance;
use omaflake_core::cli_link::CliLink;
use omaflake_core::guest_choices::GuestChoices;
use omaflake_core::platform::is_executable_file;
use omaflake_core::qemu_command::Accel;
use omaflake_core::{
    AgentState, DataFolder, DisplayMode, Error, GlBackend, GuestKind, GuestSettings, InstanceConfig, InstanceLock,
    Lengths, MacOsRelease, NetworkMode, OmaflakeId, Paths, Prerequisite, PrerequisiteStatus, Prerequisites, RecipeId,
    RecipeLibrary, StartOptions, VmBackend, VmStatus, json,
};

use crate::cli;
use crate::util::{
    Env, Failure, Outcome, choose_recipe, folder_path, gib, local_date, pad, resolve_image, sudo, usage,
};

fn running(name: &str) -> Error {
    Error::new(format!("\"{name}\" is running – first run: omaf stop {name}"))
}

// MARK: - Images

pub fn images_remove(name: &str) -> Outcome {
    let env = Env::new();
    let image = resolve_image(name, &env)?;
    env.images().remove(&image, &env.backends()?.store)?;
    println!("{image} deleted.");
    Ok(())
}

pub fn images_list() -> Outcome {
    let env = Env::new();
    let backends = env.backends()?;
    let instances = backends.store.list();
    let images = env.images();
    let lengths = Lengths::current(&env.paths);
    for image in images.list() {
        let current =
            if images.current(image.recipe()).is_some_and(|c| c.name == image.name) { "  (current)" } else { "" };
        let users: Vec<&str> = instances.iter().filter(|c| c.image == image.name).map(|c| c.name.as_str()).collect();
        let used = if users.is_empty() { "–".to_string() } else { users.join(", ") };
        println!("{}{current}", image.name);
        println!("    ID:        {}", OmaflakeId::image(&image, lengths.image));
        println!(
            "    Recipe:    {}   revision {}, {}",
            RecipeId::qualified(image.recipe()),
            image.info.revision,
            local_date(&image.info.created, "%Y-%m-%d %H:%M")
        );
        println!("    System:    {}", image.info.system);
        let look =
            image.info.look.as_deref().map_or("unknown (built before looks)".to_string(), GuestChoices::look_name);
        println!("    Look:      {look}");
        let credential = match &image.info.credential {
            None => "this computer's user and name".to_string(),
            Some(id) => omaflake_core::credentials::Credentials::new(env.paths.clone())
                .get(id)
                .map(|c| c.name)
                .or_else(|| image.info.credential_name.clone().map(|n| format!("{n} (removed)")))
                .unwrap_or_else(|| id.clone()),
        };
        println!("    Credential: {credential}");
        println!("    Instances: {used}");
    }
    // Images this omaflake cannot read (too old, too new, damaged, plan B114)
    for u in images.unreadable() {
        println!("⚠ {}: {}", u.name, u.reason);
    }
    Ok(())
}

// MARK: - Instances

pub fn new(a: &cli::New) -> Outcome {
    let Some(input) = a.recipe.as_ref().or(a.flavor.as_ref()) else {
        return Err(usage(&["new"], "Missing expected argument '--recipe <recipe>'"));
    };
    let env = Env::new();
    let backends = env.backends()?;
    let defaults = env.images().recipe_defaults();
    let chooser = |ids: &[String]| choose_recipe(&env, ids);
    // A name: the newest version of its line that has an image, else its newest (plan B111)
    let images = env.images();
    let has_image = |id: &str| images.current(id).is_some();
    let recipe = RecipeId::resolve_newest_in(input, &defaults, &env.recipe_places(), &has_image, Some(&chooser))?;
    let mut settings = defaults[&recipe].clone();
    let backend = backends.backend(settings.kind)?;
    let Some(image) = env.images().current(&recipe) else {
        return Err(Error::new(format!("No image for \"{recipe}\" yet – first run: omaf build {recipe}")).into());
    };
    if let Some(v) = cli::inverted(a.ephemeral, a.no_ephemeral) {
        settings.ephemeral = v;
    }
    if let Some(v) = cli::inverted(a.audio, a.no_audio) {
        settings.audio = Some(v);
    }
    if let Some(v) = cli::inverted(a.microphone, a.no_microphone) {
        settings.microphone = Some(v);
    }
    if let Some(v) = cli::inverted(a.clipboard, a.no_clipboard) {
        settings.clipboard = Some(v);
    }
    if let Some(v) = cli::inverted(a.camera, a.no_camera) {
        settings.camera = Some(v);
    }
    if settings.kind == GuestKind::Macos && settings.camera == Some(true) {
        eprintln!("Note: macOS VMs get the camera port, but the guest's camera extension comes after M12 (plan B49).");
    }
    if let Some(v) = cli::inverted(a.share, a.no_share) {
        settings.share = v;
    }
    // A profile's CPU cores and RAM (plan B142): the one named, else the default; --cpus and --memory win
    let profiles = omaflake_core::profiles::Profiles::new(env.paths.clone());
    let profile = match &a.profile {
        Some(p) => {
            Some(profiles.find(p).ok_or_else(|| usage(&["new"], format!("No profile {p} – see omaf profiles")))?)
        }
        None => profiles.default_profile(),
    };
    let needs = (settings.cpus, settings.memory_gb);
    if let Some(p) = &profile {
        settings.cpus = p.cpus;
        settings.memory_gb = p.memory_gb;
    }
    if let Some(cpus) = a.cpus {
        settings.cpus = cpus;
    }
    if let Some(user) = &a.user {
        settings.user = Some(user.clone());
    }
    if let Some(memory) = a.memory {
        settings.memory_gb = memory;
    }
    if let Some(disk) = a.disk {
        settings.disk_gb = disk;
    }
    if settings.cpus < needs.0 || settings.memory_gb < needs.1 {
        eprintln!(
            "Note: the recipe needs at least {} CPU cores and {} GB RAM – this VM gets {} and {} GB.",
            needs.0, needs.1, settings.cpus, settings.memory_gb
        );
    }
    if let Some(network) = &a.network {
        let Some(mode) = NetworkMode::ALL.into_iter().find(|m| m.as_str() == network) else {
            return Err(Error::new(format!("Unknown network mode \"{network}\"")).into());
        };
        settings.network = mode;
    }
    let mut share_path = None;
    if let Some(dir) = &a.share_dir {
        let path = folder_path(dir);
        if !path.is_dir() {
            return Err(Error::new(format!("Share folder missing: {}", path.display())).into());
        }
        share_path = Some(path.to_string_lossy().into_owned());
        settings.share = true;
    }
    let mut c = backend.create(&a.name, &recipe, &image, &settings)?;
    if let Some(path) = share_path {
        c.share_path = Some(path);
        backends.store.save(&c)?;
    }
    println!("{}", created_line(&c));
    Ok(())
}

/// "Instance <name> created (image <image>, SSH port <port>[, ephemeral])."
pub fn created_line(c: &InstanceConfig) -> String {
    let ssh = if c.guest_kind() == GuestKind::Macos {
        "SSH on its NAT address".to_string()
    } else {
        format!("SSH port {}", c.ssh_port)
    };
    format!("Instance {} created (image {}, {ssh}{}).", c.name, c.image, if c.ephemeral { ", ephemeral" } else { "" })
}

/// What a start shows: without --window/--headless the recipe's `display` decides (default: headless); a window gets
/// 3D (gpu) unless it is a macOS guest, --no-gl, the recipe asks for a plain window or QEMU cannot do it (`gl`).
pub fn display_for(
    window: bool,
    headless: bool,
    no_gl: bool,
    c: &InstanceConfig,
    gl: impl FnOnce() -> omaflake_core::Result<bool>,
) -> omaflake_core::Result<DisplayMode> {
    let wanted = if headless {
        DisplayMode::None
    } else if window {
        DisplayMode::Gpu
    } else {
        c.display.unwrap_or(DisplayMode::None)
    };
    Ok(if wanted == DisplayMode::None {
        DisplayMode::None
    } else if c.guest_kind() == GuestKind::Macos || no_gl || wanted == DisplayMode::Window {
        DisplayMode::Window
    } else if gl()? {
        DisplayMode::Gpu
    } else {
        DisplayMode::Window
    })
}

pub fn start(a: &cli::Start) -> Outcome {
    let env = Env::new();
    let backends = env.backends()?;
    let c = backends.store.load(&a.name)?;
    let backend = backends.backend_for(&c)?;
    let gl = match a.gl.as_str() {
        "es" => GlBackend::Es,
        "on" => GlBackend::On,
        _ => return Err(Error::new("--gl must be es or on").into()),
    };
    let lock = InstanceLock::new(&env.paths.instance(&a.name))?;
    let display = display_for(a.window, a.headless, a.no_gl, &c, || Ok(backends.qemu()?.runtime.capabilities().gl))?;
    let pid = backend.start(&c, StartOptions { display, gl })?;
    if a.window && c.guest_kind() == GuestKind::Nixos {
        let shown = if display == DisplayMode::Gpu {
            format!("3D (virtio-gpu-gl, gl={})", gl.as_str())
        } else {
            "no 3D".to_string()
        };
        println!("Display: {shown}");
    }
    println!("{}", running_line(&c, pid));
    if c.guest_kind() == GuestKind::Nixos {
        // Linux hosts without a usable /dev/kvm (plan B124)
        if let Some(line) =
            backends.qemu().ok().and_then(|q| q.runtime.accelerator(&c.name)).and_then(|a| accel_line(a, kvm_usable()))
        {
            println!("{line}");
        }
    }
    drop(lock);
    if a.wait {
        wait_for_ssh(&c, backend, 300.0)?;
    }
    Ok(())
}

/// What `omaf start` says about QEMU's accelerator: only TCG is worth a line (plan B124) – forced (`kvm_usable`: KVM
/// was there, OMAFLAKE_ACCEL=tcg) or for want of /dev/kvm
pub fn accel_line(accel: Accel, kvm_usable: bool) -> Option<&'static str> {
    (accel == Accel::Tcg).then_some(if kvm_usable {
        "Accelerator: TCG – emulated, many times slower (OMAFLAKE_ACCEL=tcg); SSH may take minutes"
    } else {
        "Accelerator: TCG – emulated, many times slower (no usable /dev/kvm – see omaf doctor); SSH may take minutes"
    })
}

/// Whether this host's /dev/kvm can be used (a TCG start then was forced)
fn kvm_usable() -> bool {
    omaflake_core::qemu_host::KvmState::detect() == omaflake_core::qemu_host::KvmState::Usable
}

/// "<name> is running (PID <pid>[, SSH …])."
pub fn running_line(c: &InstanceConfig, pid: i32) -> String {
    let ssh = if c.network == NetworkMode::None {
        String::new()
    } else if c.guest_kind() == GuestKind::Macos {
        ", SSH once it has its NAT address (omaf wait)".to_string()
    } else {
        format!(", SSH 127.0.0.1:{}", c.ssh_port)
    };
    format!("{} is running (PID {pid}{ssh}).", c.name)
}

pub fn stop(a: &cli::Stop) -> Outcome {
    let env = Env::new();
    let backends = env.backends()?;
    let c = backends.store.load(&a.name)?;
    let backend = backends.backend_for(&c)?;
    let lock = InstanceLock::new(&env.paths.instance(&a.name))?;
    let result = backend.stop(&c, Duration::from_secs_f64(a.timeout.max(0.0)))?;
    drop(lock);
    println!("{}: {}.", a.name, result.description());
    Ok(())
}

/// Reset, upgrade and rm: the instance must be stopped; `action` runs under its lock
fn while_stopped<T>(
    name: &str,
    check_first: impl FnOnce(&Env, &InstanceConfig) -> omaflake_core::Result<T>,
    action: impl FnOnce(&dyn VmBackend, &InstanceConfig, T) -> omaflake_core::Result<String>,
) -> Outcome {
    let env = Env::new();
    let backends = env.backends()?;
    let c = backends.store.load(name)?;
    let backend = backends.backend_for(&c)?;
    let checked = check_first(&env, &c)?;
    let lock = InstanceLock::new(&env.paths.instance(name))?;
    if backend.status(&c) != VmStatus::Stopped {
        return Err(running(name).into());
    }
    let done = action(backend, &c, checked)?;
    drop(lock);
    println!("{done}");
    Ok(())
}

pub fn reset(name: &str) -> Outcome {
    while_stopped(name, |_, _| Ok(()), |b, c, _| b.reset(c).map(|_| format!("{name} reset.")))
}

pub fn upgrade(name: &str) -> Outcome {
    while_stopped(
        name,
        |env, c| {
            env.images().current(&c.recipe).ok_or_else(|| {
                Error::new(format!("No image for \"{}\" – first run: omaf build {}", c.recipe, c.recipe))
            })
        },
        |b, c, image| b.upgrade(c, &image).map(|_| format!("{name} now uses {}.", image.name)),
    )
}

pub fn remove(name: &str) -> Outcome {
    while_stopped(name, |_, _| Ok(()), |b, c, _| b.remove(c).map(|_| format!("{name} deleted.")))
}

/// Header of `omaf list`
pub fn list_header() -> String {
    pad("NAME", 16)
        + &pad("STATUS", 8)
        + &pad("KIND", 7)
        + &pad("RECIPE", 13)
        + &pad("SSH", 16)
        + &pad("EPHEMERAL", 10)
        + &pad("DISK", 10)
        + "IMAGE"
}

/// One row of `omaf list`
pub fn list_row(c: &InstanceConfig, status: Option<VmStatus>, ssh: &str, used: &str, image_words: usize) -> String {
    let image = RecipeId::base(&c.image).to_string()
        + &RecipeId::words(&c.image).map(|w| format!("@{}", OmaflakeId::short(w, image_words))).unwrap_or_default();
    pad(&c.name, 16)
        + &pad(status.map(|s| s.as_str()).unwrap_or("?"), 8)
        + &pad(c.guest_kind().as_str(), 7)
        + &pad(&RecipeId::name_of_id(&c.recipe), 13)
        + &pad(ssh, 16)
        + &pad(if c.ephemeral { "yes" } else { "no" }, 10)
        + &pad(used, 10)
        + &image
}

pub fn list(a: &cli::List) -> Outcome {
    let env = Env::new();
    let backends = env.backends()?;
    let all = backends.store.list();
    // VMs this omaflake cannot read (too old, too new, damaged, plan B114)
    for u in backends.store.unreadable() {
        println!("⚠ {}: {}", u.name, u.reason);
    }
    if all.is_empty() {
        println!("No instances. Create one with: omaf new <name> --recipe <recipe>");
        return Ok(());
    }
    let lengths = Lengths::current(&env.paths);
    println!("{}", list_header());
    for c in &all {
        let mut used = gib(allocated_size(&env.paths.instance(&c.name).disk()));
        let backend = backends.backend_for(c).ok();
        let status = backend.map(|b| b.status(c));
        let mut ssh = c.ssh_port.to_string();
        if c.guest_kind() == GuestKind::Macos {
            // NAT address while running. The disk is an APFS clone of the image; its own share (the changed blocks)
            // cannot be read cheaply, the allocated size would count the shared blocks too.
            ssh = if status == Some(VmStatus::Running) {
                backends.vz().ok().and_then(|vz| vz.address(c)).unwrap_or_else(|| "–".into())
            } else {
                "–".into()
            };
            used = "clone".into();
        }
        println!("{}", list_row(c, status, &ssh, &used, lengths.image));
        if backends.store.is_external(&c.name) {
            println!("    runs from {}", env.paths.instance(&c.name).dir.display());
        }
        if a.long {
            println!("    {}  from {}", OmaflakeId::vm(c, lengths.vm), OmaflakeId::recipe(&c.recipe, lengths.recipe));
        }
    }
    Ok(())
}

/// Blocks the file takes on disk (Swift `totalFileAllocatedSize`); 0 when it is not there
fn allocated_size(file: &Path) -> u64 {
    std::fs::metadata(file).map(|m| m.blocks() * 512).unwrap_or(0)
}

pub fn ssh(a: &cli::Ssh) -> Outcome {
    let env = Env::new();
    let backends = env.backends()?;
    let c = backends.store.load(&a.name)?;
    if c.network == NetworkMode::None {
        return Err(Error::new(format!("\"{}\" has no network – serial console only (omaf log)", a.name)).into());
    }
    let command: Vec<String> = a.command.iter().skip_while(|s| *s == "--").cloned().collect();
    let args = backends.backend_for(&c)?.ssh_arguments(&c, &command);
    let Some((first, rest)) = args.split_first() else { return Err(Error::new("Could not start ssh").into()) };
    let e = std::process::Command::new(first).arg0("ssh").args(rest).exec();
    Err(Error::new(format!("Could not start ssh: {}", omaflake_core::error::os_text(&e))).into())
}

pub fn wait(a: &cli::Wait) -> Outcome {
    let env = Env::new();
    let backends = env.backends()?;
    let c = backends.store.load(&a.name)?;
    wait_for_ssh(&c, backends.backend_for(&c)?, a.timeout)?;
    Ok(())
}

/// Swift `Wait.wait`: until sshd sends its banner, or the timeout
pub fn wait_for_ssh(c: &InstanceConfig, backend: &dyn VmBackend, timeout: f64) -> omaflake_core::Result<()> {
    if c.network == NetworkMode::None {
        return Err(Error::new(format!("\"{}\" has no network – cannot check SSH", c.name)));
    }
    let t0 = Instant::now();
    if !backend.wait_for_ssh(c, Duration::from_secs_f64(timeout.max(0.0))) {
        return Err(Error::new(if backend.status(c) == VmStatus::Stopped {
            format!("\"{}\" is not running", c.name)
        } else {
            format!("SSH not ready after {} s", timeout as i64)
        }));
    }
    println!("{}: SSH ready after {} s.", c.name, t0.elapsed().as_secs());
    Ok(())
}

pub fn agent(a: &cli::Agent) -> Outcome {
    let env = Env::new();
    let backends = env.backends()?;
    let c = backends.store.load(&a.name)?;
    let backend = backends.backend_for(&c)?;
    if backend.status(&c) != VmStatus::Running {
        return Err(Error::new(format!("\"{}\" is not running", a.name)).into());
    }
    let t0 = Instant::now();
    let mut state = backend.agent_state(&c);
    while !state.as_ref().is_some_and(AgentState::is_ready) && t0.elapsed().as_secs_f64() < a.wait {
        std::thread::sleep(Duration::from_millis(500));
        state = backend.agent_state(&c);
    }
    let Some(state) = state else {
        return Err(Error::new(if c.guest_kind() == GuestKind::Macos {
            "macOS guests have no omaflake agent yet (spec §11.9)"
        } else {
            "No word from the agent (yet) – an image built before the agent has none; omaf wait checks SSH"
        })
        .into());
    };
    if a.json {
        println!("{}", String::from_utf8_lossy(&json::to_pretty(&state, true)?));
        return Ok(());
    }
    for line in agent_lines(&state, Utc::now()) {
        println!("{line}");
    }
    Ok(())
}

/// What `omaf agent` prints for a state, `now` for "Reported: … s ago"
pub fn agent_lines(state: &AgentState, now: chrono::DateTime<Utc>) -> Vec<String> {
    let mut out = Vec::new();
    if let Some(problem) = &state.problem {
        out.push(format!("Problem:  {problem}"));
    }
    if let Some(h) = &state.hello {
        // Swift `formatted(date: .omitted, time: .standard)` – locale-dependent there; 24-hour time here
        let at = state.hello_at.map(|d| format!(" at {}", local_date(&d, "%H:%M:%S"))).unwrap_or_default();
        let protocol = state.protocol_version.map(|v| v.to_string()).unwrap_or_else(|| "none".into());
        let older = if state.agent_is_older() { " (older than this omaflake – rebuild the image)" } else { "" };
        out.push(format!("Agent:    {}, protocol {protocol}{at}{older}", h.agent));
        let guest: Vec<String> =
            [h.os_name.clone(), h.kernel.as_ref().map(|k| format!("kernel {k}")), h.hostname.clone()]
                .into_iter()
                .flatten()
                .collect();
        out.push(format!("Guest:    {}", guest.join(", ")));
    }
    if let Some(s) = &state.status {
        let gb = |n: u64| format!("{:.1} GB", n as f64 / 1_073_741_824.0);
        if let Some(up) = s.uptime {
            out.push(format!("Uptime:   {} s", up as i64));
        }
        if let Some(l) = &s.load {
            out.push(format!("Load:     {}", l.iter().map(|x| format!("{x:.2}")).collect::<Vec<_>>().join(" ")));
        }
        if let Some(m) = &s.memory {
            out.push(format!("Memory:   {} of {} available", gb(m.available), gb(m.total)));
        }
        if let Some(d) = &s.disk {
            out.push(format!("Disk:     {} of {} free ({})", gb(d.free), gb(d.total), d.path));
        }
        for a in s.addresses.iter().flatten() {
            out.push(format!("Address:  {} ({})", a.address, a.interface));
        }
        if let Some(u) = s.users.as_ref().filter(|u| !u.is_empty()) {
            out.push(format!("Users:    {}", u.join(", ")));
        }
        if let Some(at) = state.status_at {
            out.push(format!("Reported: {} s ago", (now - at).num_seconds()));
        }
    }
    if state.goodbye_at.is_some() {
        out.push("The agent said goodbye (the guest is shutting down).".into());
    }
    out
}

pub fn log(a: &cli::Log) -> Outcome {
    let ip = Env::new().paths.instance(&a.name);
    let file = if a.qemu { ip.qemu_log() } else { ip.serial_log() };
    if !file.exists() {
        return Err(Error::new(format!("No log: {}", file.display())).into());
    }
    let mut args = vec!["-n".to_string(), a.lines.to_string()];
    if a.follow {
        args.push("-f".into());
    }
    args.push(file.to_string_lossy().into_owned());
    let _ =
        std::process::Command::new(omaflake_core::platform::host_tool("/usr/bin/tail")).arg0("tail").args(&args).exec();
    Err(Error::new("Could not start tail").into())
}

pub fn set(a: &cli::Set) -> Outcome {
    let env = Env::new();
    let store = env.backends()?.store;
    let lock = InstanceLock::new(&env.paths.instance(&a.name))?;
    let mut c = store.load(&a.name)?;
    if let Some(v) = cli::inverted(a.audio, a.no_audio) {
        c.audio = Some(v);
    }
    if let Some(v) = cli::inverted(a.microphone, a.no_microphone) {
        c.microphone = Some(v);
    }
    if let Some(v) = cli::inverted(a.clipboard, a.no_clipboard) {
        c.clipboard = Some(v);
    }
    if let Some(v) = cli::inverted(a.camera, a.no_camera) {
        c.camera = Some(v);
    }
    store.save(&c)?;
    drop(lock);
    println!("{}", set_line(&c));
    Ok(())
}

/// "<name>: sound on, microphone off, clipboard on, camera off (from the next start)"
pub fn set_line(c: &InstanceConfig) -> String {
    let on = |b: bool| if b { "on" } else { "off" };
    format!(
        "{}: sound {}, microphone {}, clipboard {}, camera {} (from the next start)",
        c.name,
        on(c.sound_on()),
        on(c.microphone_on()),
        on(c.clipboard_on()),
        on(c.camera_on())
    )
}

// MARK: - Settings, doctor, builder

pub fn config(a: &cli::Config) -> Outcome {
    let mut env = Env::new();
    if let Some(home) = &a.home {
        let target = if home == "default" { Paths::default_home() } else { folder_path(home) };
        let settings_file = Paths::default_home().join("config.json");
        let moved = DataFolder::move_to(&env.paths, &target, &settings_file, &|line: &str| println!("{line}"))?;
        env = Env::with_paths(moved);
    }
    let mut s = env.settings();
    if let Some(flake) = &a.flake {
        // Flake URLs (path:, github:, git+file:, …) as given; folders as absolute paths
        s.flake =
            Some(if flake.contains(':') { flake.clone() } else { folder_path(flake).to_string_lossy().into_owned() });
    }
    if let Some(qemu) = &a.qemu {
        s.qemu = Some(qemu.clone());
    }
    if let Some(runner) = &a.runner {
        s.runner = Some(folder_path(runner).to_string_lossy().into_owned());
    }
    if let Some(nix) = &a.nix {
        s.nix = Some(folder_path(nix).to_string_lossy().into_owned());
    }
    let guest_change = a.host_keyboard.is_some();
    if let Some(k) = &a.host_keyboard {
        if !k.is_empty() && !GuestChoices::KEYBOARDS.iter().any(|(c, _)| c == k) {
            let codes: Vec<&str> = GuestChoices::KEYBOARDS.iter().map(|(c, _)| *c).collect();
            return Err(usage(&["config"], format!("--host-keyboard: one of {}", codes.join(", "))));
        }
        s.host_keyboard = (!k.is_empty()).then(|| k.clone());
    }
    if let Some(keep) = &a.keep_recipe_versions {
        if keep != "on" && keep != "off" {
            return Err(usage(&["config"], "--keep-recipe-versions: on or off"));
        }
        s.keep_recipe_versions = Some(keep == "on");
    }
    let changed = a.flake.is_some()
        || a.qemu.is_some()
        || a.runner.is_some()
        || a.nix.is_some()
        || guest_change
        || a.keep_recipe_versions.is_some();
    if changed {
        s.save(&env.paths)?;
    }
    if a.keep_recipe_versions.as_deref() == Some("off") {
        let _ = RecipeLibrary::new(env.paths.clone()).prune_versions();
    }
    for line in config_lines(&s, &env.paths, &GuestSettings::current(&env.paths)) {
        println!("{line}");
    }
    Ok(())
}

/// What `omaf config` shows
pub fn config_lines(s: &omaflake_core::Settings, paths: &Paths, g: &GuestSettings) -> Vec<String> {
    vec![
        format!("flake:  {}", s.flake.as_deref().unwrap_or("–")),
        format!("qemu:   {}", s.qemu.as_deref().unwrap_or("– (PATH)")),
        format!("runner: {}", s.runner.as_deref().unwrap_or("– (next to omaf, then PATH)")),
        format!("nix:    {}", s.nix.as_deref().unwrap_or("– (PATH)")),
        format!("home:   {}", paths.root.display()),
        format!("config: {}", paths.config_file.display()),
        format!(
            "keyboard: {} (this computer's{})",
            s.host_keyboard.clone().unwrap_or_else(|| GuestSettings::mac().keyboard),
            if s.host_keyboard.is_some() { "" } else { ", as the system says" }
        ),
        format!("ssh keys: {} (into every guest)", g.authorized_keys.len()),
        format!(
            "default locale: {}",
            omaflake_core::locales::Locales::new(paths.clone()).default_locale().map_or_else(
                || "– (none yet: images get this computer's values; omaf locales add)".to_string(),
                |l| format!("{} ({}, {}, {})", l.name, l.locale, l.time_zone, l.keyboards.join("+"))
            )
        ),
        format!(
            "default credential: {}",
            omaflake_core::credentials::Credentials::new(paths.clone())
                .default_credential()
                .map_or_else(|| "– (none yet: omaf credentials add)".to_string(), |c| c.name)
        ),
        format!(
            "default look: {}",
            omaflake_core::looks::LookStore::new(paths.clone())
                .default_look(s)
                .map_or_else(|| "– (none installed: omaf looks install recommended)".to_string(), |l| l.title)
        ),
        format!(
            "recipe versions: {}",
            if s.keep_recipe_versions.unwrap_or(false) {
                "older ones are kept"
            } else {
                "the newest and those images use"
            }
        ),
    ]
}

pub fn doctor(a: &cli::Doctor) -> Outcome {
    if a.install_cli || a.uninstall_cli {
        return cli_link(a.install_cli);
    }
    let env = Env::new();
    let pre = Prerequisites::new(env.paths.clone());
    let latest = env.backends().ok().and_then(|b| b.vz().ok().and_then(|vz| MacOsRelease::latest(&vz.runner)));
    let groups = [
        ("Requirements", pre.check()),
        ("NixOS preflight", pre.nixos_preflight()),
        ("macOS preflight", pre.macos_preflight(latest.as_ref())),
    ];
    for (title, items) in &groups {
        // A Linux host has no macOS preflight (plan B124)
        if items.is_empty() {
            continue;
        }
        println!("{title}");
        for p in items {
            for line in doctor_lines(p) {
                println!("{line}");
            }
        }
    }
    if groups[0].1.iter().any(|p| p.required() && p.status == PrerequisiteStatus::Missing) {
        return Err(Failure::Exit(1));
    }
    Ok(())
}

/// One item of `omaf doctor`: mark, name, detail; then hint and install command (continued lines indented)
pub fn doctor_lines(p: &Prerequisite) -> Vec<String> {
    let mark = match p.status {
        PrerequisiteStatus::Ok => "✓",
        PrerequisiteStatus::Limited => "~",
        PrerequisiteStatus::Missing => "✗",
    };
    let mut out = vec![format!("  {mark} {}{}", pad(p.item.as_str(), 15), p.detail)];
    if !p.hint.is_empty() {
        out.push(format!("      {}", p.hint));
    }
    if let Some(cmd) = &p.install {
        out.push(format!("      {}", cmd.replace("\\\n", "\\\n      ")));
    }
    out
}

/// `omaf doctor --install-cli` / `--uninstall-cli`: the link to this omaf (cli_link.rs); root through sudo on macOS
fn cli_link(install: bool) -> Outcome {
    let link = CliLink::current();
    let as_root = |script: &str| -> omaflake_core::Result<()> {
        println!("{} needs an administrator (sudo) …", link.link.display());
        sudo(script).map_err(|_| Error::new("sudo failed – nothing was changed"))
    };
    let done = if install { link.install(&as_root)? } else { link.uninstall(&as_root)? };
    println!("{done}");
    Ok(())
}

fn stderr_line(line: &str) {
    eprintln!("{line}");
}

pub fn builder_setup() -> Outcome {
    let m = BuilderMaintenance::new(Env::new().paths);
    if is_executable_file(Path::new("/run/current-system/sw/bin/darwin-rebuild")) {
        return Err(Error::new("nix-darwin is already set up – use: omaf builder apply").into());
    }
    println!("Stage 1 of 2: nix-darwin with the standard builder …");
    let first = m.build_system("linux-builder-bootstrap", &stderr_line)?;
    sudo(&m.root_script(&BuilderMaintenance::first_activation_command(&first))?)?;
    println!("Waiting for the Linux builder …");
    let pre = Prerequisites::new(m.paths.clone());
    for _ in 0..60 {
        if pre.builder_running().status == PrerequisiteStatus::Ok {
            break;
        }
        std::thread::sleep(Duration::from_secs(2));
    }
    println!("Stage 2 of 2: omaflake's builder settings …");
    let second = m.build_system("linux-builder", &stderr_line)?;
    sudo(&m.root_script(&BuilderMaintenance::activate_command(&second))?)?;
    println!("nix-darwin and the Linux builder are set up.");
    Ok(())
}

pub fn builder_apply(force: bool) -> Outcome {
    let m = BuilderMaintenance::new(Env::new().paths);
    if !(m.system_is_omaflakes() || force) {
        return Err(Error::new(format!(
            "This Mac's nix-darwin system is not omaflake's builder host ({} is missing). Applying replaces your \
             nix-darwin configuration – run with --force if that is what you want.",
            BuilderMaintenance::MARKER
        ))
        .into());
    }
    println!("Building the builder host …");
    let system = m.build_system("linux-builder", &stderr_line)?;
    sudo(&m.root_script(&BuilderMaintenance::activate_command(&system))?)
}

pub fn builder_clean_up() -> Outcome {
    let m = BuilderMaintenance::new(Env::new().paths);
    sudo(&m.root_script(BuilderMaintenance::CLEAN_UP_COMMAND)?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use omaflake_core::agent_protocol::{AgentAddress, AgentDisk, AgentMemory, ProtocolRange};
    use omaflake_core::{AgentHello, AgentStatus, PrerequisiteItem, RecipeDefaults};

    fn config(name: &str, kind: GuestKind) -> InstanceConfig {
        let mut d = RecipeDefaults::new(
            kind,
            Some(vec!["aarch64-linux".into()]),
            2,
            4,
            20,
            NetworkMode::Shared,
            true,
            false,
            None,
        );
        d.display = None;
        InstanceConfig::new(
            name,
            "omaflake.nixos.dev@apple-banana-cherry",
            "omaflake.nixos.dev@date-eagle-fig",
            "/images/x",
            "aarch64-linux",
            &d,
            2222,
            json::now(),
        )
    }

    #[test]
    fn list_lines() {
        assert_eq!(
            list_header(),
            "NAME            STATUS  KIND   RECIPE       SSH             EPHEMERAL DISK      IMAGE"
        );
        let c = config("dev", GuestKind::Nixos);
        assert_eq!(
            list_row(&c, Some(VmStatus::Running), "2222", "1.5 GiB", 1),
            "dev             running nixos  dev          2222            no        1.5 GiB   omaflake.nixos.dev@date"
        );
        assert!(list_row(&c, None, "–", "clone", 2).starts_with("dev             ?       "));
    }

    #[test]
    fn instance_lines() {
        let mut c = config("dev", GuestKind::Nixos);
        assert_eq!(created_line(&c), "Instance dev created (image omaflake.nixos.dev@date-eagle-fig, SSH port 2222).");
        assert_eq!(running_line(&c, 42), "dev is running (PID 42, SSH 127.0.0.1:2222).");
        assert!(accel_line(Accel::Tcg, false).unwrap().contains("no usable /dev/kvm"));
        assert!(accel_line(Accel::Tcg, true).unwrap().contains("(OMAFLAKE_ACCEL=tcg)"));
        assert_eq!(accel_line(Accel::Kvm, true), None);
        assert_eq!(accel_line(Accel::Hvf, false), None);
        c.ephemeral = true;
        c.kind = GuestKind::Macos;
        assert_eq!(
            created_line(&c),
            "Instance dev created (image omaflake.nixos.dev@date-eagle-fig, SSH on its NAT address, ephemeral)."
        );
        assert_eq!(running_line(&c, 7), "dev is running (PID 7, SSH once it has its NAT address (omaf wait)).");
        c.network = NetworkMode::None;
        assert_eq!(running_line(&c, 7), "dev is running (PID 7).");
        c.microphone = Some(true);
        c.camera = Some(true);
        assert_eq!(set_line(&c), "dev: sound on, microphone on, clipboard on, camera on (from the next start)");
        c.audio = Some(false);
        assert_eq!(set_line(&c), "dev: sound off, microphone off, clipboard on, camera on (from the next start)");
    }

    #[test]
    fn display_choice() {
        let mut c = config("dev", GuestKind::Nixos);
        let gl = |b: bool| move || Ok(b);
        assert_eq!(display_for(false, false, false, &c, gl(true)).unwrap(), DisplayMode::None);
        assert_eq!(display_for(true, false, false, &c, gl(true)).unwrap(), DisplayMode::Gpu);
        assert_eq!(display_for(true, false, false, &c, gl(false)).unwrap(), DisplayMode::Window);
        assert_eq!(display_for(true, false, true, &c, gl(true)).unwrap(), DisplayMode::Window);
        assert_eq!(display_for(true, true, false, &c, gl(true)).unwrap(), DisplayMode::None);
        c.display = Some(DisplayMode::Window);
        assert_eq!(display_for(false, false, false, &c, gl(true)).unwrap(), DisplayMode::Window);
        c.display = Some(DisplayMode::Gpu);
        assert_eq!(display_for(false, false, false, &c, gl(true)).unwrap(), DisplayMode::Gpu);
        c.kind = GuestKind::Macos;
        assert_eq!(display_for(true, false, false, &c, gl(true)).unwrap(), DisplayMode::Window);
    }

    #[test]
    fn doctor_items() {
        let mut p = Prerequisite::new(
            PrerequisiteItem::Nix,
            PrerequisiteStatus::Missing,
            "not found",
            "Install Nix:",
            Some("curl … \\\n  | sh".into()),
        );
        assert_eq!(
            doctor_lines(&p),
            vec![
                "  ✗ nix            not found".to_string(),
                "      Install Nix:".into(),
                "      curl … \\\n        | sh".into()
            ]
        );
        p.status = PrerequisiteStatus::Ok;
        p.hint = String::new();
        p.install = None;
        assert_eq!(doctor_lines(&p), vec!["  ✓ nix            not found".to_string()]);
    }

    #[test]
    fn agent_report() {
        let now = Utc::now();
        let state = AgentState {
            hello: Some(AgentHello {
                agent: "omaflake-agent 0.2.0".into(),
                protocol: ProtocolRange { min: 1, max: 1 },
                os: None,
                os_name: Some("NixOS 26.05".into()),
                kernel: Some("6.12".into()),
                hostname: Some("dev".into()),
                boot_id: None,
                features: None,
            }),
            hello_at: None,
            protocol_version: Some(1),
            status: Some(AgentStatus {
                uptime: Some(61.9),
                load: Some(vec![0.5, 0.25, 0.126]),
                memory: Some(AgentMemory { total: 4 * 1_073_741_824, available: 1_073_741_824 }),
                disk: Some(AgentDisk { path: "/".into(), total: 20 * 1_073_741_824, free: 5 * 1_073_741_824 }),
                addresses: Some(vec![AgentAddress {
                    interface: "eth0".into(),
                    family: "inet".into(),
                    address: "10.0.2.15".into(),
                }]),
                users: Some(vec![]),
            }),
            status_at: Some(now - chrono::Duration::seconds(3)),
            goodbye_at: None,
            problem: None,
            features: None,
        };
        let lines = agent_lines(&state, now);
        assert_eq!(lines[0], format!("Agent:    omaflake-agent 0.2.0, protocol 1{}", older_note(&state)));
        assert_eq!(
            &lines[1..],
            [
                "Guest:    NixOS 26.05, kernel 6.12, dev",
                "Uptime:   61 s",
                "Load:     0.50 0.25 0.13",
                "Memory:   1.0 GB of 4.0 GB available",
                "Disk:     5.0 GB of 20.0 GB free (/)",
                "Address:  10.0.2.15 (eth0)",
                "Reported: 3 s ago",
            ]
        );
    }

    fn older_note(state: &AgentState) -> &'static str {
        if state.agent_is_older() { " (older than this omaflake – rebuild the image)" } else { "" }
    }
}
