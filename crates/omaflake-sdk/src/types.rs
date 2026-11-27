// SPDX-License-Identifier: AGPL-3.0-or-later
//! The API's types a client needs to talk to the service (docs/api.md §2, §5, §6): the session, jobs and their
//! results, events. JSON: camelCase, optional fields left out when unset, times ISO 8601 UTC, sizes in bytes.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::GuestKind;
use crate::text::Text;
use crate::time::Date;

/// The API version this server speaks (api.md §2)
pub const API_VERSION: u32 = 1;

// MARK: Session

/// `initialize` params
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeParams {
    #[serde(default)]
    pub client: Option<String>,
    #[serde(default)]
    pub client_version: Option<String>,
    #[serde(default)]
    pub api_version: Option<u32>,
    /// The UI's language ("en", "de", …) – recipe descriptions in it where a recipe has one
    #[serde(default)]
    pub language: Option<String>,
    /// The app's update feed URL (Info.plist `OMUpdateFeedURL`) – `updates.check` reads it
    #[serde(default)]
    pub update_feed: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InitializeResult {
    pub api_version: u32,
    pub omaf_version: String,
    pub host: Host,
    pub data_folder: String,
    pub config_file: String,
    /// Which core this is (plan B165): a UI with its own `omaf` compares it and replaces a service of another one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub core: Option<CoreIdentity>,
}

/// The running `omaf`: its resolved path, size and modification time (plan B165) – two cores of the same version
/// still differ (an app replaced in place)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoreIdentity {
    pub executable: String,
    pub size: u64,
    #[serde(with = "crate::time::iso8601_fractional")]
    pub modified: Date,
}

impl CoreIdentity {
    /// The executable at `path` (resolved), None when it cannot be read
    pub fn of(path: &std::path::Path) -> Option<CoreIdentity> {
        let path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
        let meta = std::fs::metadata(&path).ok()?;
        Some(CoreIdentity {
            executable: path.to_string_lossy().into_owned(),
            size: meta.len(),
            // In milliseconds, as it travels – so a core compares equal with what it said in `initialize`
            modified: crate::time::whole_millis(crate::time::from_system_time(meta.modified().ok()?)?),
        })
    }

    /// This process's executable – as it is on disk now
    pub fn current() -> Option<CoreIdentity> {
        CoreIdentity::of(&std::env::current_exe().ok()?)
    }

    /// This process's executable as it was when the process started (its first call; `Server::new` makes it). A
    /// service must answer this: asked later, after its app was replaced in place, `current` reads the new file and
    /// an old service passes for the new one – the app then keeps it (2026-11-27: a new 0.8.1 build talked to the
    /// service of the 0.8.1 build before and showed its colours)
    pub fn at_start() -> Option<CoreIdentity> {
        static AT_START: std::sync::OnceLock<Option<CoreIdentity>> = std::sync::OnceLock::new();
        AT_START.get_or_init(CoreIdentity::current).clone()
    }
}

/// `service.restart`: the service ends once these jobs (waiting or running) are done – at once when 0
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ServiceRestart {
    pub jobs: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Host {
    /// "aarch64-darwin", "aarch64-linux", …
    pub system: String,
    /// "macOS 27.0.1", "NixOS 26.05", …
    pub os: String,
}

// MARK: Jobs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobKind {
    Build,
    MacosInstall,
    IpswDownload,
    BuilderInstallNix,
    /// Mount the Nix Store volume at /nix again and restore what mounts it at startup (`builder.mountNix`)
    BuilderMountNix,
    BuilderSetup,
    BuilderApply,
    BuilderCleanUp,
    /// Remove a Nix install the installer left half done (`builder.uninstallNix`, plan B160)
    BuilderUninstallNix,
    /// Add the user to Nix's trusted-users (`builder.trust`) – for a Nix omaflake did not install
    BuilderTrust,
    /// Does the Linux builder accept this Mac's builds? (`builder.check`, plan B175)
    BuilderCheck,
    /// Give the Linux builder a new disk when it refuses the Mac's builds (`builder.repair`, plan B175)
    BuilderRepair,
    /// Upgrade a running VM in place: the newer image's system copied in and switched to (`vms.upgrade {keep}`, plan
    /// B178)
    VmUpgrade,
    /// Install or update omaflake's QEMU (`qemu.install`, `nix profile`)
    QemuInstall,
    /// Set up everything NixOS guests need, one step after the other (`setup.nixos`, plan B160)
    Setup,
    /// Remove what nothing uses from this computer's Nix Store (`nix.cleanUp`, `nix-collect-garbage`, plan B149)
    NixCleanUp,
    /// Install or uninstall the `omaf` command (`doctor.installCli`, `doctor.uninstallCli`)
    CliInstall,
    Import,
    Export,
    DataMove,
    RecipeInstall,
    /// Install looks (`looks.install`, `looks.installRecommended`, plan B141)
    LookInstall,
    /// Install a module or key binding set (`modules.install`, plan B142)
    ModuleInstall,
    /// A running VM's diagnosis (`diagnostics.run`, plan B162)
    Diagnostics,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum JobState {
    Queued,
    Checking,
    Running,
    Finished,
    Failed,
    Cancelled,
}

impl JobState {
    /// Waiting or running
    pub fn is_active(self) -> bool {
        matches!(self, JobState::Queued | JobState::Checking | JobState::Running)
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobProgress {
    /// Step of `total` (builds; 0 = no steps known)
    pub step: u32,
    pub total: u32,
    pub title: String,
    /// Units of the current step (bytes of a download, items of a copy)
    pub done: i64,
    pub of: i64,
    /// 0…1 overall
    pub fraction: f64,
}

/// A job (api.md §5)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobDto {
    pub id: String,
    pub kind: JobKind,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recipe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub guest_kind: Option<GuestKind>,
    pub state: JobState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub progress: Option<JobProgress>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::time::iso8601::option")]
    pub started: Option<Date>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::time::iso8601::option")]
    pub finished: Option<Date>,
    /// When the job last wrote a line (a stalled build shows it)
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::time::iso8601::option")]
    pub last_output: Option<Date>,
    /// Lines so far (`jobs.log` reads them)
    pub log_lines: usize,
    /// Routine lines left out of the log (still in `logFile`)
    pub hidden_lines: usize,
    /// The newest line – what the job does now (the Doctor shows it in the row, plan B160)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_line: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_file: Option<String>,
    /// It can still be cancelled
    pub cancellable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// `error` as a text (docs/api.md §3, §10.2) when it has a key – a UI shows it in its language
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error_text: Option<Text>,
}

// MARK: Job results (`result` of an ended job, per kind)

/// `build`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BuildResult {
    pub image: String,
    /// Older images of its line removed (plan B52)
    pub removed: Vec<String>,
}

/// `recipe-install`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstallResult {
    /// The recipe versions written (empty: it was up to date)
    pub installed: Vec<String>,
}

/// `import`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportResult {
    pub kind: PackageKindDto,
    /// The VM's or the image's name; recipe ids for recipes (empty: nothing new)
    pub names: Vec<String>,
    pub notes: Vec<String>,
}

/// `export`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExportResult {
    /// The package written
    pub path: String,
}

/// `data-move`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DataMoveResult {
    pub data_folder: String,
}

/// `ipsw-download`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct IpswResult {
    pub file: String,
    pub version: String,
}

/// `macos-install`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BaseResult {
    /// The base's folder
    pub base: String,
    /// It was there already
    pub existing: bool,
    /// Its first boot finished (it can build images)
    pub ready: bool,
}

/// `cli-install`: the link, and whether it is there now (installed) or was removed
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CliResult {
    pub link: String,
    pub installed: bool,
    /// What it did, in English
    pub message: String,
}

/// `builder-install-nix`, `builder-mount-nix`, `builder-setup`, `builder-apply`, `builder-clean-up`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageResult {
    /// What it did, in English ("Linux builder: 18G → 28G free")
    pub message: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobLog {
    pub lines: Vec<String>,
    /// The `from` of the next call
    pub next: usize,
}

// MARK: Events

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmEnded {
    pub name: String,
    /// It was stopped through this server
    pub requested: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct VmReady {
    pub name: String,
    pub via: Via,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct JobChanged {
    pub job: JobDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct JobLines {
    pub id: String,
    pub lines: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Level {
    Info,
    Warning,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Message {
    pub text: String,
    pub level: Level,
}

/// How a VM answered: its agent reported in, or SSH answers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Agent,
    Ssh,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageKindDto {
    Vm,
    Image,
    Recipe,
}
