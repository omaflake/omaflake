// SPDX-License-Identifier: AGPL-3.0-or-later
//! The API's types (docs/api.md §4–§7) – the stable contract, kept apart from the core's internal types so a change
//! inside the core never changes what clients see. JSON: camelCase, optional fields left out when unset, times ISO 8601
//! UTC in whole seconds, sizes in bytes, paths absolute.
//!
//! Plain string enums of the core whose JSON is the contract's (`GuestKind`, `VmStatus`, `NetworkMode`, `DisplayMode`,
//! `StopResult`, `PrerequisiteItem`, `PrerequisiteStatus`, `RecipeSourceKind`) are reused; `tests::core_enums_keep_their_names`
//! pins their spelling so a change there breaks a test instead of the clients.

use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::models::{DisplayMode, GuestKind, NetworkMode, StopResult, VmStatus};
use crate::prerequisites::{PrerequisiteItem, PrerequisiteStatus};
use crate::recipe_sources::RecipeSourceKind;

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

// MARK: Errors and lists

/// A file this omaflake cannot read (too old, too new, damaged; plan B114)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Unreadable {
    /// The VM, image or recipe it belongs to
    pub name: String,
    pub reason: String,
}

/// How many words omaflake IDs show, per kind (plan B111)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct IdLengths {
    pub recipe: usize,
    pub image: usize,
    pub vm: usize,
}

// MARK: VMs

/// A VM (api.md §4.1)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Vm {
    pub name: String,
    pub recipe: String,
    pub image: String,
    pub kind: GuestKind,
    pub system: String,
    pub status: VmStatus,
    /// An action on it is in progress in this server
    pub busy: bool,
    pub cpus: u32,
    #[serde(rename = "memoryGB")]
    pub memory_gb: u32,
    #[serde(rename = "diskGB")]
    pub disk_gb: u32,
    pub network: NetworkMode,
    pub share: bool,
    /// The folder shared (the default share/<name> or the user's) – when `share` is on
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub share_path: Option<String>,
    pub ephemeral: bool,
    /// NixOS guests: the forwarded SSH port
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ssh_port: Option<u16>,
    /// The guest user (SSH)
    pub user: String,
    pub audio: bool,
    pub microphone: bool,
    pub clipboard: bool,
    pub camera: bool,
    /// How a plain start (`vms.start` with `display: auto`) shows it
    pub display: DisplayMode,
    #[serde(with = "crate::json::iso8601")]
    pub created: DateTime<Utc>,
    /// Its omaflake ID words (spec §11.11), as many as `idLengths.vm`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
    /// Its whole omaflake ID (all words; UIs shorten it to `idLengths.vm`)
    pub omaflake_id: String,
    /// As the UI shows it
    pub label: String,
    /// macOS guests while running: NAT address
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub address: Option<String>,
    /// What the omaflake agent last reported (spec §11.9; the agent.json shape), while running
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent: Option<Value>,
    /// The package's place when it runs from outside the data folder (plan B108)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub location: Option<String>,
    /// The newest image of its recipe's line
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_image: Option<String>,
    pub upgradable: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VmList {
    pub vms: Vec<Vm>,
    pub unreadable: Vec<Unreadable>,
}

/// The runtime values of a recipe – what `vms.create` takes
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeSettings {
    pub kind: GuestKind,
    pub cpus: u32,
    #[serde(rename = "memoryGB")]
    pub memory_gb: u32,
    #[serde(rename = "diskGB")]
    pub disk_gb: u32,
    pub network: NetworkMode,
    pub share: bool,
    pub ephemeral: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayMode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StopAnswer {
    pub result: StopResult,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VmSetAnswer {
    pub vm: Vm,
    pub takes_effect_at_next_start: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SshAnswer {
    pub argv: Vec<String>,
    pub shell_line: String,
}

/// How a VM answered: its agent reported in, or SSH answers
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Via {
    Agent,
    Ssh,
}

/// `vms.wait`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct WaitAnswer {
    pub via: Via,
}

/// `vms.log`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LogLines {
    pub lines: Vec<String>,
}

/// `vms.share`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathAnswer {
    pub path: String,
}

/// `vms.nextName`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NameAnswer {
    pub name: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Limits {
    pub mac_max: usize,
    pub mac_running: Vec<String>,
}

// MARK: Images

/// A built image (api.md §4.2)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImageDto {
    pub name: String,
    pub recipe: String,
    /// The recipe source it was built from (its id), when known
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub kind: GuestKind,
    pub system: String,
    pub revision: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub store_path: Option<String>,
    #[serde(with = "crate::json::iso8601")]
    pub created: DateTime<Utc>,
    pub line: String,
    pub path: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub size_bytes: Option<i64>,
    /// Its omaflake ID as short as it can be
    pub label: String,
    /// Its whole omaflake ID (all words)
    pub omaflake_id: String,
    /// VM names using it
    pub instances: Vec<String>,
    /// It is the newest of its line
    pub current: bool,
    /// Its recipe evaluates to something else now (plan B73/B82); left out while that is not known (the freshness
    /// check runs `nix eval` in the background, and a recipe version that is gone has nothing to compare with)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub outdated: Option<bool>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ImageList {
    pub images: Vec<ImageDto>,
    pub unreadable: Vec<Unreadable>,
}

// MARK: Recipes

/// One version of a recipe line (plan B111)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct VersionDto {
    pub id: String,
    pub source: String,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::json::iso8601::option")]
    pub added: Option<DateTime<Utc>>,
    /// The line's newest version
    pub newest: bool,
    /// In a local folder source (edited in place, not a downloaded copy)
    pub live: bool,
    /// Images built from it
    pub images: Vec<String>,
    /// Its omaflake ID as short as it can be (`idLengths.recipe` words)
    pub label: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledDto {
    pub source: String,
    pub version: String,
    #[serde(with = "crate::json::iso8601")]
    pub installed: DateTime<Utc>,
}

/// A recipe (api.md §4.3): the recipe.toml values plus what omaflake knows about it
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeDto {
    pub id: String,
    pub base: String,
    pub qualified: String,
    pub display_name: String,
    pub label: String,
    pub kind: GuestKind,
    pub systems: Vec<String>,
    pub runs_here: bool,
    pub cpus: u32,
    #[serde(rename = "memoryGB")]
    pub memory_gb: u32,
    #[serde(rename = "diskGB")]
    pub disk_gb: u32,
    pub network: NetworkMode,
    pub share: bool,
    pub ephemeral: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<bool>,
    /// In the session's language where the recipe has it
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
    pub lab: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayMode>,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// The source id it comes from
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub source: Option<String>,
    pub line: String,
    /// What tells it apart from a recipe of the same author/kind/name (plan B110)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub distinction: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// Downloaded ones: from where, which version, when
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub installed: Option<InstalledDto>,
    /// Its line's current image
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
    /// The recipe changed since that image was built (plan B73/B82)
    pub outdated: bool,
    /// Its line's versions, newest first
    pub versions: Vec<VersionDto>,
}

/// `recipes.list` and the `recipes.changed` event
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RecipeList {
    pub recipes: Vec<RecipeDto>,
    pub lines: BTreeMap<String, Vec<VersionDto>>,
    pub id_lengths: IdLengths,
    #[serde(rename = "qemuGL")]
    pub qemu_gl: bool,
    pub unreadable: Vec<Unreadable>,
    /// Lab recipes are left out (developer mode off and no `all`)
    pub lab_hidden: bool,
}

/// A recipe source (api.md §4.4)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SourceDto {
    pub id: String,
    pub kind: RecipeSourceKind,
    pub location: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub branch: Option<String>,
    pub display_name: String,
    pub enabled: bool,
    pub private: bool,
    pub official: bool,
    /// A private repository's token is in the system keychain (false for public sources)
    pub has_token: bool,
}

/// An entry of a source's catalog.json (spec §11.3) plus where it comes from
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CatalogRecipeDto {
    pub id: String,
    pub author: String,
    pub kind: GuestKind,
    pub name: String,
    /// In the session's language where the catalog has it
    pub description: String,
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    pub lab: bool,
    pub systems: Vec<String>,
    pub requires: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    pub version: String,
    /// The recipe's files as catalog.json lists them
    pub files: Vec<CatalogFileDto>,
    pub source: String,
    pub runs_here: bool,
}

/// A file of a catalog entry
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogFileDto {
    /// Relative to the recipe's folder
    pub path: String,
    pub size: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecipeUpdate {
    /// The installed recipe that has a newer version
    pub id: String,
    pub recipe: CatalogRecipeDto,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Available {
    pub offered: Vec<CatalogRecipeDto>,
    pub updates: Vec<RecipeUpdate>,
    pub suggested: Vec<CatalogRecipeDto>,
    pub errors: BTreeMap<String, String>,
}

/// `recipes.pruneVersions`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Pruned {
    /// The recipe versions removed
    pub removed: Vec<String>,
}

/// `recipes.removeLine`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineRemoval {
    /// The versions removed
    pub removed: Vec<String>,
    /// The versions kept (images use them, or they live in a local folder)
    pub kept: Vec<String>,
}

/// `sources.check`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceChecks {
    /// Source id → its check
    pub results: BTreeMap<String, SourceCheck>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceAdded {
    pub source: SourceDto,
    pub note: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SourceCheck {
    pub ok: bool,
    pub note: String,
}

// MARK: Doctor

/// One check of the Doctor (api.md §4.4)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PrerequisiteDto {
    pub item: PrerequisiteItem,
    pub status: PrerequisiteStatus,
    /// What the check is about, in English (UIs translate by `item`)
    pub title: String,
    pub required: bool,
    pub detail: String,
    pub hint: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub install: Option<String>,
    pub settable_path: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub free_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub needed_bytes: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub base_user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub expected_user: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacOsReleaseDto {
    pub version: String,
    pub build: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bytes: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PerKind<T> {
    pub nixos: T,
    pub macos: T,
}

/// `doctor.check` and the `doctor.changed` event
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DoctorReport {
    pub prerequisites: Vec<PrerequisiteDto>,
    pub nixos_preflight: Vec<PrerequisiteDto>,
    pub macos_preflight: Vec<PrerequisiteDto>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub latest_release: Option<MacOsReleaseDto>,
    pub build_blockers: PerKind<Vec<PrerequisiteDto>>,
    pub start_blockers: PerKind<Vec<PrerequisiteDto>>,
    /// Items that need attention (the Doctor's warning badge)
    pub issues: usize,
    /// Something every feature needs is missing
    pub requirements_missing: bool,
    /// A macOS base exists (needed to build macOS images)
    pub has_mac_base: bool,
    /// The guest user of the macOS base
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mac_base_user: Option<String>,
    /// This Mac's nix-darwin system comes from omaflake's image flake – `builder.apply` without `force` refuses to
    /// replace one that does not
    pub builder_system_is_omaflakes: bool,
    #[serde(with = "crate::json::iso8601")]
    pub checked: DateTime<Utc>,
}

// MARK: Settings and updates

/// config.json's settings with their defaults applied (api.md §7 `settings.get`)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsDto {
    pub developer_mode: bool,
    pub check_for_updates: bool,
    pub notifications: bool,
    pub keep_recipe_versions: bool,
    /// The guest values set here (an unset one is the computer's, `guestDefaults`)
    pub guest: GuestValues,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flake: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub qemu: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nix: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub runner: Option<String>,
    /// The image flake in use (the setting, `VM_FLAKE` or the bundled one)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub flake_in_use: Option<String>,
    pub data_folder: String,
}

/// Guest values: the overrides in config.json (each optional) or this computer's (`guestDefaults`, all set)
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct GuestValues {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full_name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub time_zone: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub locale: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub keyboard: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Keyboard {
    pub code: String,
    /// The locale whose name it carries ("de_CH")
    pub locale: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choices {
    pub time_zones: Vec<String>,
    pub locales: Vec<String>,
    pub keyboards: Vec<Keyboard>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsAnswer {
    pub settings: SettingsDto,
    pub guest_defaults: GuestValues,
    pub choices: Choices,
    pub is_default_data_folder: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReleaseDto {
    pub version: String,
    pub url: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub page: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum UpdateState {
    NoFeed,
    UpToDate,
    Available,
    Off,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UpdateAnswer {
    pub state: UpdateState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub release: Option<ReleaseDto>,
}

// MARK: Packages

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PackageKindDto {
    Vm,
    Image,
    Recipe,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConflictDto {
    /// A VM of that name is here already
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub vm_name: Option<String>,
    /// The first free name to import it under instead
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub suggested_name: Option<String>,
    /// The same image is here already
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub image: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Inspection {
    pub kind: PackageKindDto,
    pub valid: bool,
    /// Why it is not valid – a text of the views (docs/api.md §10.2; the core's English message has no key yet)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<super::views::Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conflict: Option<ConflictDto>,
    pub network_volume: bool,
}

// MARK: Jobs

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum JobKind {
    Build,
    MacosInstall,
    IpswDownload,
    BuilderInstallNix,
    BuilderSetup,
    BuilderApply,
    BuilderCleanUp,
    Import,
    Export,
    DataMove,
    RecipeInstall,
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
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::json::iso8601::option")]
    pub started: Option<DateTime<Utc>>,
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::json::iso8601::option")]
    pub finished: Option<DateTime<Utc>>,
    /// When the job last wrote a line (a stalled build shows it)
    #[serde(default, skip_serializing_if = "Option::is_none", with = "crate::json::iso8601::option")]
    pub last_output: Option<DateTime<Utc>>,
    /// Lines so far (`jobs.log` reads them)
    pub log_lines: usize,
    /// Routine lines left out of the log (still in `logFile`)
    pub hidden_lines: usize,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub log_file: Option<String>,
    /// It can still be cancelled
    pub cancellable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
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

/// `builder-install-nix`, `builder-setup`, `builder-apply`, `builder-clean-up`
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn s<T: Serialize>(v: T) -> Value {
        serde_json::to_value(v).unwrap()
    }

    #[test]
    fn core_enums_keep_their_names() {
        assert_eq!(s(GuestKind::Macos), json!("macos"));
        assert_eq!(s(VmStatus::Running), json!("running"));
        assert_eq!(s(NetworkMode::Isolated), json!("isolated"));
        assert_eq!(s(DisplayMode::Gpu), json!("gpu"));
        assert_eq!(s(StopResult::AlreadyStopped), json!("alreadyStopped"));
        assert_eq!(s(StopResult::ForcedOff), json!("forcedOff"));
        assert_eq!(s(PrerequisiteItem::BuilderRunning), json!("builderRunning"));
        assert_eq!(s(PrerequisiteItem::SshKey), json!("sshKey"));
        assert_eq!(s(PrerequisiteStatus::Limited), json!("limited"));
        assert_eq!(s(RecipeSourceKind::Github), json!("github"));
        assert_eq!(s(RecipeSourceKind::Url), json!("url"));
        assert_eq!(s(JobKind::BuilderCleanUp), json!("builder-clean-up"));
        assert_eq!(s(JobKind::MacosInstall), json!("macos-install"));
        assert_eq!(s(UpdateState::NoFeed), json!("noFeed"));
    }

    #[test]
    fn job_leaves_out_what_is_unset() {
        let job = JobDto {
            id: "j1".into(),
            kind: JobKind::Build,
            title: "Build dev".into(),
            recipe: Some("omaflake.nixos.dev".into()),
            guest_kind: Some(GuestKind::Nixos),
            state: JobState::Queued,
            progress: None,
            started: None,
            finished: None,
            last_output: None,
            log_lines: 0,
            hidden_lines: 0,
            log_file: None,
            cancellable: true,
            result: None,
            error: None,
        };
        assert_eq!(
            s(&job),
            json!({"id": "j1", "kind": "build", "title": "Build dev", "recipe": "omaflake.nixos.dev",
                   "guestKind": "nixos", "state": "queued", "logLines": 0, "hiddenLines": 0, "cancellable": true})
        );
        let back: JobDto = serde_json::from_value(s(&job)).unwrap();
        assert_eq!(back, job);
    }

    #[test]
    fn vm_uses_the_documented_keys() {
        let vm = Vm {
            name: "research".into(),
            recipe: "omaflake.nixos.dev@a".into(),
            image: "omaflake.nixos.dev@b".into(),
            kind: GuestKind::Nixos,
            system: "aarch64-linux".into(),
            status: VmStatus::Running,
            busy: false,
            cpus: 4,
            memory_gb: 8,
            disk_gb: 64,
            network: NetworkMode::Shared,
            share: true,
            share_path: Some("/s".into()),
            ephemeral: false,
            ssh_port: Some(2201),
            user: "alex".into(),
            audio: true,
            microphone: false,
            clipboard: true,
            camera: false,
            display: DisplayMode::Gpu,
            created: crate::json::parse_date("2026-10-20T08:00:00Z").unwrap(),
            identity: Some("amber-lake".into()),
            omaflake_id: "vm:research:amber-lake".into(),
            label: "research".into(),
            address: None,
            agent: None,
            location: None,
            latest_image: Some("omaflake.nixos.dev@c".into()),
            upgradable: true,
        };
        let v = s(&vm);
        for key in ["memoryGB", "diskGB", "sshPort", "sharePath", "omaflakeId", "latestImage"] {
            assert!(v.get(key).is_some(), "{key}");
        }
        assert_eq!(v["created"], json!("2026-10-20T08:00:00Z"));
        assert!(v.get("address").is_none() && v.get("agent").is_none());
        let settings: RecipeSettings = serde_json::from_value(json!({"kind": "nixos", "cpus": 2, "memoryGB": 4,
            "diskGB": 20, "network": "none", "share": false, "ephemeral": true}))
        .unwrap();
        assert_eq!(settings.memory_gb, 4);
        assert_eq!(s(&settings).get("audio"), None);
    }
}
