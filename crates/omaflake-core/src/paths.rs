// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Paths.swift: the data folder, a VM's files and the settings (config.json).

use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{IoContext, Result};
use crate::file_format::{write_atomic, FileFormat, Step};
use crate::models::InstanceConfig;
use crate::packages::{Package, PackageKind};
use crate::platform::{expand_tilde, home_dir, standardize};
use crate::recipe_sources::RecipeSource;
use crate::vm_err;

/// Data storage (spec §5): by default ~/.omaflake. Deliberately not in Documents/Desktop – those are
/// privacy-protected folders, and omaflake asks for no access to them unless the user chooses a folder there
/// (data folder, share folder, image flake). Can be created with a custom root folder for tests.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Paths {
    /// Data folder (images, instances, bases, …)
    pub root: PathBuf,
    /// Settings file; stays at ~/.omaflake/config.json when the data folder is elsewhere (it names the data folder)
    pub config_file: PathBuf,
}

impl Paths {
    /// Top-level entries of the data folder that belong to omaflake – moved by `DataFolder::move_to`; anything else
    /// in the folder stays where it is: the settings file and `developer/` (contributors' own local files).
    pub const DATA_ENTRIES: [&'static str; 12] = [
        "images",
        "instances",
        "bases",
        "ipsw",
        "logs",
        "qemu",
        "share",
        "_to_delete",
        "cache",
        "guest",
        "recipes",
        "run",
    ];

    /// `config_file` None = `<root>/config.json`
    pub fn new(root: impl Into<PathBuf>, config_file: Option<PathBuf>) -> Self {
        let root = root.into();
        let config_file = config_file.unwrap_or_else(|| root.join("config.json"));
        Paths { root, config_file }
    }

    /// ~/.omaflake – default data folder and home of the settings file
    pub fn default_home() -> PathBuf {
        home_dir().join(".omaflake")
    }

    /// Data folder from VM_HOME (settings file inside it), otherwise `home` in ~/.omaflake/config.json,
    /// otherwise ~/.omaflake.
    pub fn standard() -> Paths {
        if let Ok(custom) = std::env::var("VM_HOME") {
            if !custom.is_empty() {
                return Paths::new(custom, None);
            }
        }
        let config = Self::default_home().join("config.json");
        let home = Settings::load_file(&config).home.map(|h| PathBuf::from(expand_tilde(&h)));
        Paths::new(home.unwrap_or_else(Self::default_home), Some(config))
    }

    pub fn images(&self) -> PathBuf {
        self.root.join("images")
    }
    pub fn current_images(&self) -> PathBuf {
        self.images().join("current")
    }
    pub fn instances(&self) -> PathBuf {
        self.root.join("instances")
    }
    pub fn shares(&self) -> PathBuf {
        self.root.join("share")
    }
    pub fn trash(&self) -> PathBuf {
        self.root.join("_to_delete")
    }
    /// macOS: fresh installations (`mac-fresh-<build>`) and bases (`mac-base-<build>`), spec §6.5
    pub fn bases(&self) -> PathBuf {
        self.root.join("bases")
    }
    /// IPSW downloads of `omaf-runner install --latest`
    pub fn ipsw(&self) -> PathBuf {
        self.root.join("ipsw")
    }
    /// Logs of macOS image builds
    pub fn logs(&self) -> PathBuf {
        self.root.join("logs")
    }
    /// Guest values for the image flake (GuestSettings, input `omaflake-guest`)
    pub fn guest(&self) -> PathBuf {
        self.root.join("guest")
    }
    /// App icon as a file for QEMU's Dock icon (written by the core when QEMU starts with a window)
    pub fn dock_icon(&self) -> PathBuf {
        self.root.join("cache/dock-icon.png")
    }
    /// The image QEMU's 3D window shows until the guest draws (written by the core when QEMU starts with one)
    pub fn loading_image(&self) -> PathBuf {
        self.root.join("cache/loading.png")
    }
    pub fn cache(&self) -> PathBuf {
        self.root.join("cache")
    }
    /// Recipes downloaded from remote sources: <id>/ (spec §11.3, plan B107, B110)
    pub fn recipes(&self) -> PathBuf {
        self.root.join("recipes")
    }
    /// Runtime files of the VMs on this Mac – sockets, pid, logs, lock, the agent's report – never in a package
    pub fn run(&self) -> PathBuf {
        self.root.join("run")
    }

    /// A VM: its package (in the data folder, or an external one – e.g. on an external disk – from config.json
    /// `externalVMs`) and its run folder on this Mac (plan B108)
    pub fn instance(&self, name: &str) -> InstancePaths {
        InstancePaths::new(self.instance_dir(name), Some(self.run().join(name)))
    }

    /// A VM's package: `instances/<name>.omaflake-vm`, otherwise an external package of that name, otherwise where
    /// a new one goes
    pub fn instance_dir(&self, name: &str) -> PathBuf {
        let own = self.instances().join(format!("{name}.{}", Package::VM));
        if own.exists() {
            return own;
        }
        if let Some(external) = self
            .external_instances()
            .into_iter()
            .find(|u| Package::name_of(u, PackageKind::Vm).as_deref() == Some(name))
        {
            return external;
        }
        own
    }

    /// VM packages outside the data folder that omaflake runs from where they are (`externalVMs`); those whose
    /// volume is not there are left out
    pub fn external_instances(&self) -> Vec<PathBuf> {
        Settings::load_file(&self.config_file)
            .external_vms
            .unwrap_or_default()
            .into_iter()
            .map(PathBuf::from)
            .filter(|p| p.join("instance.json").exists())
            .collect()
    }

    /// An image's package: `images/<name>.omaflake-image`
    pub fn image_dir(&self, name: &str) -> PathBuf {
        self.images().join(format!("{name}.{}", Package::IMAGE))
    }

    /// Default share folder of an instance (created by omaflake)
    pub fn share(&self, name: &str) -> PathBuf {
        self.shares().join(name)
    }

    /// Share folder of an instance: the folder the user chose (`share_path`), otherwise the default one.
    pub fn share_dir(&self, c: &InstanceConfig) -> PathBuf {
        c.share_path.as_ref().map(PathBuf::from).unwrap_or_else(|| self.share(&c.name))
    }

    /// Creates the default share folder; a folder the user chose must exist and is left as it is.
    pub fn prepare_share(&self, c: &InstanceConfig, mode: Option<u32>) -> Result<PathBuf> {
        let dir = self.share_dir(c);
        if c.share_path.is_some() {
            if !dir.is_dir() {
                return Err(vm_err!("Share folder missing: {}", dir.display()));
            }
            return Ok(dir);
        }
        fs::create_dir_all(&dir).with_path(&dir)?;
        if let Some(mode) = mode {
            fs::set_permissions(&dir, fs::Permissions::from_mode(mode)).with_path(&dir)?;
        }
        Ok(dir)
    }
}

/// A VM's files. `dir` is its package (`<name>.omaflake-vm`, plan B108) – what makes the VM and travels with it:
/// instance.json, the disks, UEFI variables, known host keys, for NixOS maybe its own image. `run` holds what only
/// exists while it runs on this Mac – sockets, pid, logs, lock, the agent's report – so a package on a stick carries
/// none of it and every Mac has its own.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InstancePaths {
    pub dir: PathBuf,
    pub run: PathBuf,
}

impl InstancePaths {
    /// The names of the runtime files – to move them out of a package of the older layout
    pub const RUNTIME_NAMES: [&'static str; 14] = [
        "qemu.pid",
        "qmp.sock",
        "qga.sock",
        "serial.log",
        "qemu.log",
        ".lock",
        "runner.sock",
        "runner.pid",
        "runner.log",
        "clip.sock",
        "cam.sock",
        "bridge.log",
        "agent.sock",
        "agent.json",
    ];

    /// `run` None = the package itself (the layout before plan B108)
    pub fn new(dir: PathBuf, run: Option<PathBuf>) -> Self {
        let run = run.unwrap_or_else(|| dir.clone());
        InstancePaths { dir, run }
    }

    pub fn config(&self) -> PathBuf {
        self.dir.join("instance.json")
    }
    pub fn disk(&self) -> PathBuf {
        self.dir.join("disk.qcow2")
    }
    pub fn efivars(&self) -> PathBuf {
        self.dir.join("efivars.fd")
    }
    pub fn known_hosts(&self) -> PathBuf {
        self.dir.join("known_hosts")
    }
    /// NixOS: the image the overlay is built on, inside the package (exported and external VMs carry it)
    pub fn embedded_image(&self) -> PathBuf {
        self.dir.join(format!("image.{}", Package::IMAGE))
    }
    /// macOS guests (spec §5): clone of the image folder
    pub fn vm(&self) -> PathBuf {
        self.dir.join("vm")
    }

    // Runtime
    pub fn pid_file(&self) -> PathBuf {
        self.run.join("qemu.pid")
    }
    pub fn qmp_socket(&self) -> PathBuf {
        self.run.join("qmp.sock")
    }
    pub fn qga_socket(&self) -> PathBuf {
        self.run.join("qga.sock")
    }
    pub fn serial_log(&self) -> PathBuf {
        self.run.join("serial.log")
    }
    pub fn qemu_log(&self) -> PathBuf {
        self.run.join("qemu.log")
    }
    pub fn lock_file(&self) -> PathBuf {
        self.run.join(".lock")
    }
    pub fn runner_socket(&self) -> PathBuf {
        self.run.join("runner.sock")
    }
    pub fn runner_pid(&self) -> PathBuf {
        self.run.join("runner.pid")
    }
    pub fn runner_log(&self) -> PathBuf {
        self.run.join("runner.log")
    }
    // Guest integration (clipboard, camera): the port's socket and the bridge process' log
    pub fn clip_socket(&self) -> PathBuf {
        self.run.join("clip.sock")
    }
    pub fn camera_socket(&self) -> PathBuf {
        self.run.join("cam.sock")
    }
    pub fn bridge_log(&self) -> PathBuf {
        self.run.join("bridge.log")
    }
    // omaflake agent (spec §11.9): the port's socket and what the agent last reported (AgentState)
    pub fn agent_socket(&self) -> PathBuf {
        self.run.join("agent.sock")
    }
    pub fn agent_state(&self) -> PathBuf {
        self.run.join("agent.json")
    }
    /// The accelerator the running QEMU was started with ("hvf", "kvm", "tcg"; plan B124)
    pub fn accel_file(&self) -> PathBuf {
        self.run.join("accel")
    }
}

/// config.json, schema `settings` (plan B114); 0 → 1 changes nothing
fn settings_v0_to_v1(_: &mut Map<String, Value>) -> Result<()> {
    Ok(())
}

/// 1 → 2 adds `guest.look` (plan B135): nothing to convert – a v1 file has none
fn settings_v1_to_v2(_: &mut Map<String, Value>) -> Result<()> {
    Ok(())
}

/// 2 → 3: credentials (plan B137). The guest's user, full name and e-mail, when set, become a credential (a random ID,
/// named after the user, else "Default") and the default; `guest` keeps locale, keyboard, time zone and look.
fn settings_v2_to_v3(o: &mut Map<String, Value>) -> Result<()> {
    let Some(Value::Object(guest)) = o.get_mut("guest") else { return Ok(()) };
    let mut credential = Map::new();
    for key in ["user", "fullName", "email"] {
        if let Some(v) = guest.remove(key).filter(|v| v.as_str().is_some_and(|s| !s.trim().is_empty())) {
            credential.insert(key.into(), v);
        }
    }
    if guest.is_empty() {
        o.remove("guest");
    }
    if credential.is_empty() {
        return Ok(());
    }
    let name = credential.get("user").and_then(Value::as_str).unwrap_or("Default").to_string();
    let id = crate::credentials::random_id();
    credential.insert("id".into(), Value::String(id.clone()));
    credential.insert("name".into(), Value::String(name));
    o.insert("credentials".into(), Value::Array(vec![Value::Object(credential)]));
    o.insert("defaultCredential".into(), Value::String(id));
    Ok(())
}

/// Settings 3 → 4 (plan B141): looks are installed things, not names – the guest look becomes the default look's id
/// (`defaultLook`, the look may still have to be installed: the Doctor says so); the official source is
/// omaflake/catalog instead of omaflake/recipes (plan B140)
fn settings_v3_to_v4(o: &mut Map<String, Value>) -> Result<()> {
    if let Some(Value::Object(guest)) = o.get_mut("guest") {
        let look = guest.remove("look");
        if guest.is_empty() {
            o.remove("guest");
        }
        if let Some(Value::String(name)) = look.filter(|v| v.as_str().is_some_and(|s| !s.is_empty())) {
            o.insert("defaultLook".into(), Value::String(format!("omaflake.look.{name}")));
        }
    }
    if let Some(Value::Array(sources)) = o.get_mut("sources") {
        for s in sources.iter_mut() {
            let github = s.get("kind").and_then(Value::as_str) == Some("github");
            let old =
                s.get("location").and_then(Value::as_str).is_some_and(|l| l.eq_ignore_ascii_case("omaflake/recipes"));
            if github && old {
                s["location"] = Value::String("omaflake/catalog".into());
            }
        }
    }
    Ok(())
}

/// Settings 4 → 5 (plan B141): locales instead of the guest values – language, region, time zone and keyboard set
/// here become a locale "Standard" (missing ones this computer's), the default; the keyboard is also the host's
fn settings_v4_to_v5(o: &mut Map<String, Value>) -> Result<()> {
    let Some(Value::Object(guest)) = o.remove("guest") else { return Ok(()) };
    let text = |k: &str| guest.get(k).and_then(Value::as_str).filter(|s| !s.is_empty()).map(str::to_string);
    if text("locale").is_none() && text("timeZone").is_none() && text("keyboard").is_none() {
        return Ok(());
    }
    let mac = crate::guest_settings::GuestSettings::mac();
    let keyboard = text("keyboard").unwrap_or(mac.keyboard);
    let id = crate::locales::random_id();
    o.insert(
        "locales".into(),
        serde_json::json!([{
            "id": id,
            "name": "Standard",
            "locale": text("locale").unwrap_or(mac.locale),
            "timeZone": text("timeZone").unwrap_or(mac.time_zone),
            "keyboards": [keyboard],
        }]),
    );
    o.insert("defaultLocale".into(), Value::String(id));
    if let Some(k) = text("keyboard") {
        o.insert("hostKeyboard".into(), Value::String(k));
    }
    Ok(())
}

static SETTINGS_STEPS: [Step; 6] =
    [settings_v0_to_v1, settings_v1_to_v2, settings_v2_to_v3, settings_v3_to_v4, settings_v4_to_v5, settings_v5_to_v6];

/// Settings 5 → 6 (plan B148): the help switch `help` is new and off while left out – nothing to change
fn settings_v5_to_v6(_o: &mut Map<String, Value>) -> Result<()> {
    Ok(())
}

/// Settings in ~/.omaflake/config.json (optional). Keys as Swift writes them (`externalVMs`, `developerMode`, …);
/// fields that are None are left out.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Settings {
    /// The image flake – omaflake's Nix side that builds images from recipes (folder or flake URL, e.g. <checkout>/nix)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flake: Option<String>,
    /// Recipe sources in their order (spec §11.3, plan B107); None = omaflake/recipes on GitHub
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sources: Option<Vec<RecipeSource>>,
    /// VM packages that live outside the data folder (e.g. on an external disk) and run from there (plan B108)
    #[serde(rename = "externalVMs", default, skip_serializing_if = "Option::is_none")]
    pub external_vms: Option<Vec<String>>,
    /// qemu-system-aarch64; default: the first one with 3D on PATH or in the Nix and Homebrew folders (`QemuRuntime::preferred_binary`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qemu: Option<String>,
    /// omaf-runner (signed with scripts/sign-runner.sh); default: next to the running executable
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    /// nix binary, for unusual installations; default: PATH and the usual Nix locations
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nix: Option<String>,
    /// Named credentials – user, full name, e-mail; their passwords are in the secret store (plan B137)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub credentials: Option<Vec<crate::credentials::Credential>>,
    /// The credential a new image gets unless another one is chosen; None: this computer's values
    #[serde(rename = "defaultCredential", default, skip_serializing_if = "Option::is_none")]
    pub default_credential: Option<String>,
    /// The look new images get unless another is chosen (plan B141): an installed look's id
    #[serde(rename = "defaultLook", default, skip_serializing_if = "Option::is_none")]
    pub default_look: Option<String>,
    /// Locales – language, region, time zone, the guest's keyboards (plan B141, settings v5)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locales: Option<Vec<crate::locales::GuestLocale>>,
    #[serde(rename = "defaultLocale", default, skip_serializing_if = "Option::is_none")]
    pub default_locale: Option<String>,
    /// This computer's keyboard (XKB name): the physical one, preset for new locales; None: as the system says
    #[serde(rename = "hostKeyboard", default, skip_serializing_if = "Option::is_none")]
    pub host_keyboard: Option<String>,
    /// Profiles – CPU cores and RAM a new VM gets (plan B141, B142; settings v5)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profiles: Option<Vec<crate::profiles::Profile>>,
    #[serde(rename = "defaultProfile", default, skip_serializing_if = "Option::is_none")]
    pub default_profile: Option<String>,
    /// Data folder; None = ~/.omaflake. Only read from ~/.omaflake/config.json; change it with `DataFolder::move_to`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub home: Option<String>,
    /// Developer mode: also show lab recipes (omaflake's development recipes and others marked `lab = true`)
    #[serde(rename = "developerMode", default, skip_serializing_if = "Option::is_none")]
    pub developer_mode: Option<bool>,
    /// Look for a newer omaflake (the only network access of its own, spec §11.6); None = on
    #[serde(rename = "checkForUpdates", default, skip_serializing_if = "Option::is_none")]
    pub check_for_updates: Option<bool>,
    /// System notifications (VM ready, image built, …) while the menu window is closed; None = on
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notifications: Option<bool>,
    /// Keep older versions of a recipe's line (plan B111) when a newer one comes (an update, an import, a change in a
    /// local folder); None = no. A version an image was built from is kept either way.
    #[serde(rename = "keepRecipeVersions", default, skip_serializing_if = "Option::is_none")]
    pub keep_recipe_versions: Option<bool>,
    /// The apps' help switch (plan B148): explanations – intros, footers, a field's help, hints marked help – shown;
    /// None = off
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<bool>,
}

impl Settings {
    /// config.json (see `settings_v0_to_v1`, `settings_v1_to_v2`, `settings_v2_to_v3`)
    pub const FILE_FORMAT: FileFormat = FileFormat::new("settings", "config.json", 0, &SETTINGS_STEPS);

    /// Swift `Settings(flake:qemu:runner:)`
    pub fn with(flake: Option<String>, qemu: Option<String>, runner: Option<String>) -> Self {
        Settings { flake, qemu, runner, ..Default::default() }
    }

    /// The settings of `paths` (Swift `Settings.load(_ paths:)`)
    pub fn load(paths: &Paths) -> Settings {
        Self::load_file(&paths.config_file)
    }

    /// The settings, brought up to date (and written back) if older (plan B114); defaults if there are none or they
    /// cannot be read – a file of a newer format is then left alone (`save` refuses it). Swift `load(file:)`.
    pub fn load_file(file: &Path) -> Settings {
        let Ok(data) = fs::read(file) else { return Settings::default() };
        let Ok((value, migrated)) = Self::FILE_FORMAT.decode::<Settings>(&data) else { return Settings::default() };
        if migrated {
            let _ = value.save_file(file);
        }
        value
    }

    pub fn save(&self, paths: &Paths) -> Result<()> {
        self.save_file(&paths.config_file)
    }

    /// The image flake's folder was `flavors/` (until plan B92), then `recipes/` (until B107), now `nix/`: a `flake`
    /// setting pointing to an old folder without a flake.nix moves to the `nix` folder next to it, if that holds one.
    /// Returns the new path.
    pub fn migrate_recipes_folder(paths: &Paths) -> Option<String> {
        let mut s = Self::load(paths);
        let flake = s.flake.clone()?;
        if flake.contains(':') {
            return None;
        }
        let url = standardize(Path::new(&expand_tilde(&flake)));
        let old = url.file_name()?.to_string_lossy().into_owned();
        if !["flavors", "recipes"].contains(&old.as_str()) || url.join("flake.nix").exists() {
            return None;
        }
        let nix = url.parent()?.join("nix");
        if !nix.join("flake.nix").exists() {
            return None;
        }
        // Keep the user's spelling (e.g. ~/…) and only swap the last part
        let trimmed = flake.strip_suffix('/').unwrap_or(&flake);
        let kept = &trimmed[..trimmed.len().saturating_sub(old.len())];
        s.flake = Some(format!("{kept}nix"));
        s.save(paths).ok()?;
        s.flake
    }

    /// Swift `save(file:)`: refuses a newer format's file, pretty-printed with sorted keys
    pub fn save_file(&self, file: &Path) -> Result<()> {
        Self::FILE_FORMAT.check_writable(file)?;
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).with_path(dir)?;
        }
        write_atomic(file, &Self::FILE_FORMAT.encode(self)?)
    }

    /// Flake path: VM_FLAKE > config.json > the image flake that comes with omaflake.app
    pub fn flake_path(&self) -> Result<String> {
        if let Ok(env) = std::env::var("VM_FLAKE") {
            if !env.is_empty() {
                return Ok(env);
            }
        }
        if let Some(flake) = &self.flake {
            return Ok(expand_tilde(flake));
        }
        if let Some(bundled) = Self::bundled_flake() {
            return Ok(bundled.to_string_lossy().into_owned());
        }
        Err(vm_err!("Flake path unknown. Set it with: omaf config --flake <checkout>/nix  (or VM_FLAKE)"))
    }

    /// The image flake that comes with omaflake.app (Contents/Resources/nix, scripts/bundle-app.sh): the default when
    /// no path is set – for the app itself, and for the CLI next to an installed app. Swift's `Bundle.main.resourceURL`
    /// is the `Resources` folder next to the running executable's `MacOS` folder. The Linux package has it in
    /// `<prefix>/share/omaflake/nix` next to `<prefix>/bin` (linux/package.nix, plan B131).
    pub fn bundled_flake() -> Option<PathBuf> {
        let mut candidates: Vec<PathBuf> = Vec::new();
        if let Ok(exe) = std::env::current_exe() {
            let exe = fs::canonicalize(&exe).unwrap_or(exe);
            if let Some(contents) = exe.parent().and_then(Path::parent) {
                candidates.push(contents.join("Resources/nix"));
                candidates.push(contents.join("share/omaflake/nix"));
            }
        }
        candidates.push(PathBuf::from("/Applications/omaflake.app/Contents/Resources/nix"));
        candidates.push(home_dir().join("Applications/omaflake.app/Contents/Resources/nix"));
        candidates.into_iter().find(|c| c.join("flake.nix").exists())
    }

    /// The guides omaflake.app bundles (Contents/Resources/Guides, scripts/bundle-app.sh) next to the running
    /// executable – None outside the app
    pub fn bundled_guides() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let exe = fs::canonicalize(&exe).unwrap_or(exe);
        let dir = exe.parent()?.parent()?.join("Resources/Guides");
        dir.is_dir().then_some(dir)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::models::tests::config;

    /// Temporary ~/.omaflake replacement per test (Swift `tempPaths()`); keep the TempDir alive
    pub fn temp_paths() -> (tempfile::TempDir, Paths) {
        let dir = tempfile::Builder::new().prefix("vmtest-").tempdir().unwrap();
        let paths = Paths::new(dir.path().to_path_buf(), None);
        (dir, paths)
    }

    #[test]
    fn default_root_is_hidden_folder_in_home() {
        if std::env::var("VM_HOME").map(|v| !v.is_empty()).unwrap_or(false) {
            return;
        }
        assert_eq!(Paths::standard().root.file_name().unwrap(), ".omaflake");
    }

    #[test]
    fn default_share_is_created_with_mode() {
        let (_t, p) = temp_paths();
        let c = config("s1");
        let dir = p.prepare_share(&c, Some(0o777)).unwrap();
        assert_eq!(dir, p.share("s1"));
        assert_eq!(fs::metadata(&dir).unwrap().permissions().mode() & 0o7777, 0o777);
    }

    #[test]
    fn chosen_share_must_exist_and_keeps_its_permissions() {
        let (_t, p) = temp_paths();
        let mut c = config("s2");
        let mine = p.root.join("mine");
        c.share_path = Some(mine.to_string_lossy().into_owned());
        assert!(p.prepare_share(&c, None).is_err());
        fs::create_dir_all(&mine).unwrap();
        fs::set_permissions(&mine, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(p.prepare_share(&c, Some(0o777)).unwrap(), mine);
        assert_eq!(fs::metadata(&mine).unwrap().permissions().mode() & 0o7777, 0o700);
        assert!(!p.share("s2").exists());
    }

    /// config.json pointing to <checkout>/flavors or <checkout>/recipes moves to <checkout>/nix once that has the flake
    #[test]
    fn flake_setting_follows_the_folder() {
        let (_t, paths) = temp_paths();
        let checkout = paths.root.join("checkout");
        let nix = checkout.join("nix");
        fs::create_dir_all(&nix).unwrap();
        for old in ["flavors", "recipes"] {
            let s = Settings { flake: Some(checkout.join(old).to_string_lossy().into_owned()), ..Default::default() };
            s.save(&paths).unwrap();
            let _ = fs::remove_file(nix.join("flake.nix"));
            assert_eq!(Settings::migrate_recipes_folder(&paths), None); // no flake.nix in nix/ yet: nothing moves
            fs::write(nix.join("flake.nix"), "{}").unwrap();
            let nix_path = nix.to_string_lossy().into_owned();
            assert_eq!(Settings::migrate_recipes_folder(&paths), Some(nix_path.clone()));
            assert_eq!(Settings::load(&paths).flake, Some(nix_path));
            assert_eq!(Settings::migrate_recipes_folder(&paths), None); // done once
        }
        // a flake URL is left alone
        let s = Settings { flake: Some("github:omaflake/omaflake?dir=flavors".into()), ..Default::default() };
        s.save(&paths).unwrap();
        assert_eq!(Settings::migrate_recipes_folder(&paths), None);
    }

    #[test]
    fn newer_settings_are_not_overwritten() {
        let (_t, p) = temp_paths();
        fs::write(&p.config_file, br#"{"$schema":"https://omaflake.dev/schema/settings/v99.json"}"#).unwrap();
        assert!(Settings::default().save(&p).is_err()); // a newer omaflake's settings are not overwritten
        assert_eq!(Settings::load(&p), Settings::default());
    }

    /// config.json as the Swift version wrote it, now v6 with a default look, a credential and a locale
    /// (testdata/config-v6.json); the same as v5 is read as it is (5 → 6 adds only the help switch)
    #[test]
    fn swift_settings_fixture_round_trips() {
        let (v5, migrated) =
            Settings::FILE_FORMAT.decode::<Settings>(include_bytes!("../testdata/config-v5.json")).unwrap();
        assert!(migrated && v5.help.is_none());
        let data = include_bytes!("../testdata/config-v6.json");
        let (s, migrated) = Settings::FILE_FORMAT.decode::<Settings>(data).unwrap();
        assert!(!migrated);
        assert_eq!(s.default_credential.as_deref(), Some("c-1"));
        assert_eq!(s.credentials.as_ref().unwrap()[0].full_name.as_deref(), Some("Jane Doe"));
        assert_eq!(s.default_look.as_deref(), Some("omaflake.look.rose-pine-dawn"));
        assert_eq!(s.external_vms.as_deref(), Some(&["/Volumes/stick/research.omaflake-vm".to_string()][..]));
        assert_eq!(s.developer_mode, Some(true));
        let l = &s.locales.as_ref().unwrap()[0];
        assert_eq!((l.time_zone.as_str(), l.keyboards.join(",")), ("Europe/Berlin", "de,ru".to_string()));
        assert_eq!((s.default_locale.as_deref(), s.host_keyboard.as_deref()), (Some("l-1"), Some("de")));
        assert_eq!(s.sources.as_ref().unwrap()[2].is_private, Some(true));
        let written = Settings::FILE_FORMAT.encode(&s).unwrap();
        assert_eq!(String::from_utf8(written).unwrap(), String::from_utf8_lossy(data).trim_end());
    }

    /// A v1 file (before looks) and a v2 file (before credentials) are read as they are and written as v3: the
    /// guest's user and full name become the default credential
    #[test]
    fn older_settings_become_version_five() {
        for data in
            [&include_bytes!("../testdata/config-v1.json")[..], &include_bytes!("../testdata/config-v2.json")[..]]
        {
            let (s, migrated) = Settings::FILE_FORMAT.decode::<Settings>(data).unwrap();
            assert!(migrated);
            let c = &s.credentials.as_ref().unwrap()[0];
            assert!(c.id.starts_with("c-") && c.id.len() == 14);
            assert_eq!((c.name.as_str(), c.user.as_deref()), ("jane", Some("jane")));
            assert_eq!(c.full_name.as_deref(), Some("Jane Doe"));
            assert_eq!(s.default_credential.as_deref(), Some(c.id.as_str()));
            // 4 → 5: the guest values become a locale "Standard", the default (plan B141)
            let l = &s.locales.as_ref().unwrap()[0];
            assert_eq!((l.name.as_str(), l.time_zone.as_str()), ("Standard", "Europe/Berlin"));
            assert_eq!(s.default_locale.as_deref(), Some(l.id.as_str()));
            let written = String::from_utf8(Settings::FILE_FORMAT.encode(&s).unwrap()).unwrap();
            assert!(written.starts_with("{\n  \"$schema\" : \"https://omaflake.dev/schema/settings/v6.json\""));
            assert!(!written.contains("\"user\" : \"jane\",\n    \"look\""));
        }
        // 3 → 4 (plan B141): the guest look becomes the default look's id, omaflake/recipes becomes omaflake/catalog
        let (s, migrated) =
            Settings::FILE_FORMAT.decode::<Settings>(include_bytes!("../testdata/config-v3.json")).unwrap();
        assert!(migrated);
        assert_eq!(s.default_look.as_deref(), Some("omaflake.look.rose-pine-dawn"));
        assert_eq!(s.locales.as_ref().unwrap()[0].time_zone, "Europe/Berlin");
        let (s, _) = Settings::FILE_FORMAT
            .decode::<Settings>(
                br#"{"$schema":"https://omaflake.dev/schema/settings/v3.json","guest":{"look":"vanilla"},"sources":[{"kind":"github","location":"omaflake/recipes"}]}"#,
            )
            .unwrap();
        assert!(s.locales.is_none() && s.default_look.as_deref() == Some("omaflake.look.vanilla"));
        assert_eq!(s.sources.unwrap()[0].location, "omaflake/catalog");
        // Without a user of its own: no credential, this computer's values stay the default
        let (s, _) = Settings::FILE_FORMAT
            .decode::<Settings>(
                br#"{"$schema":"https://omaflake.dev/schema/settings/v2.json","guest":{"keyboard":"de"}}"#,
            )
            .unwrap();
        assert!(s.credentials.is_none() && s.default_credential.is_none());
    }

    #[test]
    fn version_zero_settings_get_a_schema() {
        let (_t, p) = temp_paths();
        fs::write(&p.config_file, br#"{"flake":"/f","developerMode":true}"#).unwrap();
        let s = Settings::load(&p);
        assert_eq!(s.flake.as_deref(), Some("/f"));
        let text = fs::read_to_string(&p.config_file).unwrap();
        assert!(text.starts_with("{\n  \"$schema\" : \"https://omaflake.dev/schema/settings/v6.json\""));
    }

    #[test]
    fn instance_paths() {
        let p = Paths::new("/d", None);
        let ip = p.instance("a");
        assert_eq!(ip.dir, PathBuf::from("/d/instances/a.omaflake-vm"));
        assert_eq!(ip.pid_file(), PathBuf::from("/d/run/a/qemu.pid"));
        assert_eq!(ip.embedded_image(), PathBuf::from("/d/instances/a.omaflake-vm/image.omaflake-image"));
        assert_eq!(p.image_dir("x"), PathBuf::from("/d/images/x.omaflake-image"));
        assert_eq!(p.config_file, PathBuf::from("/d/config.json"));
    }
}
