// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Models.swift. (Swift's `VMError` is `crate::Error`, see error.rs.)

use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use crate::error::{Error, Result};
use crate::file_format::{FileFormat, Step};
use crate::omaflake_id::{OmaflakeId, OmaflakeIdKind};
use crate::platform::Platform;
use crate::recipe_sources::{RecipeRef, RecipeSource};

/// Network mode from recipe.toml (spec §6.3).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetworkMode {
    /// -netdev user with SSH forwarding
    Shared,
    /// -netdev user,restrict=on – no outside access
    Isolated,
    /// no network card
    None,
}

impl NetworkMode {
    pub const ALL: [NetworkMode; 3] = [NetworkMode::Shared, NetworkMode::Isolated, NetworkMode::None];

    pub fn as_str(self) -> &'static str {
        match self {
            NetworkMode::Shared => "shared",
            NetworkMode::Isolated => "isolated",
            NetworkMode::None => "none",
        }
    }
}

/// Guest kind of a recipe or instance (spec §3): NixOS on QEMU or macOS on Virtualization.framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuestKind {
    Nixos,
    Macos,
}

impl GuestKind {
    /// Swift `GuestKind.allCases`, in their order
    pub const ALL: [GuestKind; 2] = [GuestKind::Nixos, GuestKind::Macos];

    /// Swift `rawValue`: "nixos", "macos"
    pub fn as_str(self) -> &'static str {
        match self {
            GuestKind::Nixos => "nixos",
            GuestKind::Macos => "macos",
        }
    }

    /// Swift `GuestKind(rawValue:)`
    pub fn parse(s: &str) -> Option<GuestKind> {
        match s {
            "nixos" => Some(GuestKind::Nixos),
            "macos" => Some(GuestKind::Macos),
            _ => None,
        }
    }
}

impl std::fmt::Display for GuestKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Display mode at start.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DisplayMode {
    /// headless: serial console + SSH only
    None,
    /// Cocoa window with virtio-gpu-pci (no 3D)
    Window,
    /// Cocoa window with virtio-gpu-gl-pci (virglrenderer, custom QEMU build, M2)
    Gpu,
}

impl DisplayMode {
    pub const ALL: [DisplayMode; 3] = [DisplayMode::None, DisplayMode::Window, DisplayMode::Gpu];

    pub fn as_str(self) -> &'static str {
        match self {
            DisplayMode::None => "none",
            DisplayMode::Window => "window",
            DisplayMode::Gpu => "gpu",
        }
    }
}

/// OpenGL path for `-display cocoa,gl=…`: native macOS OpenGL (CGL, default) or ANGLE (OpenGL ES over Metal).
/// With ANGLE, virgl offers the guest only GLSL 130 (GL 3.0) – kitty then fails to start (plan B22).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GlBackend {
    /// ANGLE
    Es,
    /// CGL
    #[default]
    On,
}

impl GlBackend {
    pub const ALL: [GlBackend; 2] = [GlBackend::Es, GlBackend::On];

    /// The value for `gl=` (Swift `rawValue`)
    pub fn as_str(self) -> &'static str {
        match self {
            GlBackend::Es => "es",
            GlBackend::On => "on",
        }
    }
}

/// How `stop` ended. `Display` gives Swift's raw values ("shut down cleanly", …).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum StopResult {
    AlreadyStopped,
    Powerdown,
    Quit,
    ForcedOff,
    Killed,
}

impl StopResult {
    /// Swift's raw value – the text shown to the user
    pub fn description(self) -> &'static str {
        match self {
            StopResult::AlreadyStopped => "was already stopped",
            StopResult::Powerdown => "shut down cleanly",
            StopResult::Quit => "stopped via QMP quit",
            StopResult::ForcedOff => "forced off (omaf-runner kill)",
            StopResult::Killed => "killed (SIGKILL)",
        }
    }
}

impl std::fmt::Display for StopResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.description())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum VmStatus {
    Stopped,
    Running,
}

impl VmStatus {
    pub fn as_str(self) -> &'static str {
        match self {
            VmStatus::Stopped => "stopped",
            VmStatus::Running => "running",
        }
    }
}

/// What the QEMU binary supports (custom build vs. Homebrew).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct QemuCapabilities {
    /// `virtio-gpu-gl-pci` available
    pub gl: bool,
    /// 9p option `guest_owner_uid/gid` (patch qemu-9p-guest-owner, plan B18)
    pub guest_owner: bool,
}

/// Runtime defaults of a recipe (its recipe.toml), as `nix eval --json .#lib.recipeDefaults` gives them.
///
/// JSON keys: `memory_gb`, `disk_gb`, `description_de` (into `descriptions["de"]`); `kind` defaults to nixos;
/// `systems` is required (plan B114); `tags` is written only when not empty; optional fields are left out when None.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(from = "RecipeDefaultsJson", into = "RecipeDefaultsJson")]
pub struct RecipeDefaults {
    /// "nixos" if missing
    pub kind: GuestKind,
    /// What it can be built for (`Platform`, plan B114), e.g. ["aarch64-linux", "x86_64-linux"]
    pub systems: Vec<String>,
    pub cpus: u32,
    pub memory_gb: u32,
    pub disk_gb: u32,
    pub network: NetworkMode,
    pub share: bool,
    pub ephemeral: bool,
    /// Login user inside the guest (for SSH); None = the user running omaflake on the Mac
    pub user: Option<String>,
    /// Name shown to the user ("minimal"); the dictionary key is the recipe ID "<author>.<kind>.<name>"
    pub name: Option<String>,
    /// Who publishes the recipe ("omaflake" for the presets, "local" for a developer's own)
    pub author: Option<String>,
    /// Sound from the VM to the Mac (default on)
    pub audio: Option<bool>,
    /// The Mac's microphone in the VM (default off – macOS asks for permission the first time)
    pub microphone: Option<bool>,
    /// Clipboard shared with the Mac (default on)
    pub clipboard: Option<bool>,
    /// The Mac's camera in the VM (default off; NixOS guests – only while an app in the VM uses it)
    pub camera: Option<bool>,
    /// What the recipe is for (shown behind an "i"); `description_de` etc. in the app's language when present
    pub description: Option<String>,
    /// Language → description (only "de" is read and written, from `description_de`)
    pub descriptions: BTreeMap<String, String>,
    /// A development recipe: shown only in developer mode
    pub lab: Option<bool>,
    /// How a plain "start" shows the VM: none (headless), window, gpu (window with 3D where QEMU can) – M5
    pub display: Option<DisplayMode>,
    /// Words to find the recipe by (e.g. "latex", "neovim"); lowercase, shown in the app's recipe row (plan B97)
    pub tags: Vec<String>,
    /// SPDX identifier and where to read more (omaflake/recipes, CONTRIBUTING.md) – shown in the recipe's details
    pub license: Option<String>,
    pub homepage: Option<String>,
}

/// The JSON form of `RecipeDefaults` (Swift's hand-written `init(from:)` / `encode(to:)`)
#[derive(Clone, Serialize, Deserialize)]
struct RecipeDefaultsJson {
    #[serde(default)]
    kind: Option<GuestKind>,
    systems: Vec<String>,
    cpus: u32,
    memory_gb: u32,
    disk_gb: u32,
    network: NetworkMode,
    share: bool,
    ephemeral: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    author: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    audio: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    microphone: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    clipboard: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    camera: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    description_de: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    lab: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    display: Option<DisplayMode>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    homepage: Option<String>,
}

impl From<RecipeDefaultsJson> for RecipeDefaults {
    fn from(j: RecipeDefaultsJson) -> Self {
        let mut descriptions = BTreeMap::new();
        if let Some(de) = j.description_de {
            descriptions.insert("de".to_string(), de);
        }
        RecipeDefaults {
            kind: j.kind.unwrap_or(GuestKind::Nixos),
            systems: j.systems,
            cpus: j.cpus,
            memory_gb: j.memory_gb,
            disk_gb: j.disk_gb,
            network: j.network,
            share: j.share,
            ephemeral: j.ephemeral,
            user: j.user,
            name: j.name,
            author: j.author,
            audio: j.audio,
            microphone: j.microphone,
            clipboard: j.clipboard,
            camera: j.camera,
            description: j.description,
            descriptions,
            lab: j.lab,
            display: j.display,
            tags: j.tags,
            license: j.license,
            homepage: j.homepage,
        }
    }
}

impl From<RecipeDefaults> for RecipeDefaultsJson {
    fn from(d: RecipeDefaults) -> Self {
        RecipeDefaultsJson {
            kind: Some(d.kind),
            systems: d.systems,
            cpus: d.cpus,
            memory_gb: d.memory_gb,
            disk_gb: d.disk_gb,
            network: d.network,
            share: d.share,
            ephemeral: d.ephemeral,
            user: d.user,
            name: d.name,
            author: d.author,
            audio: d.audio,
            microphone: d.microphone,
            clipboard: d.clipboard,
            camera: d.camera,
            description: d.description,
            description_de: d.descriptions.get("de").cloned(),
            lab: d.lab,
            display: d.display,
            tags: d.tags,
            license: d.license,
            homepage: d.homepage,
        }
    }
}

impl RecipeDefaults {
    /// Swift's memberwise `init`; `systems` None = what this host runs of `kind`
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        kind: GuestKind,
        systems: Option<Vec<String>>,
        cpus: u32,
        memory_gb: u32,
        disk_gb: u32,
        network: NetworkMode,
        share: bool,
        ephemeral: bool,
        user: Option<String>,
    ) -> Self {
        let systems =
            systems.unwrap_or_else(|| Platform::host_system(kind).map(|s| vec![s.to_string()]).unwrap_or_default());
        RecipeDefaults {
            kind,
            systems,
            cpus,
            memory_gb,
            disk_gb,
            network,
            share,
            ephemeral,
            user,
            name: None,
            author: None,
            audio: None,
            microphone: None,
            clipboard: None,
            camera: None,
            description: None,
            descriptions: BTreeMap::new(),
            lab: None,
            display: None,
            tags: Vec::new(),
            license: None,
            homepage: None,
        }
    }

    pub fn is_lab(&self) -> bool {
        self.lab.unwrap_or(false)
    }

    /// It can be built and run on this host
    pub fn runs_here(&self) -> bool {
        Platform::host_system(self.kind).map(|s| self.systems.iter().any(|x| x == s)).unwrap_or(false)
    }

    /// The description in `language` ("de"), else the English one
    pub fn description_in(&self, language: Option<&str>) -> Option<&str> {
        language.and_then(|l| self.descriptions.get(l)).or(self.description.as_ref()).map(String::as_str)
    }
}

/// instance.json, schema `instance` (plan B114). 0 → 1: "flavor" became "recipe" (plan B92), `kind` is always
/// there (NixOS when missing), `system` added – every VM before it was made on a Mac with Apple silicon
fn instance_v0_to_v1(o: &mut Map<String, Value>) -> Result<()> {
    FileFormat::rename_key("flavor", "recipe", o);
    if !o.contains_key("kind") {
        o.insert("kind".into(), Value::String(GuestKind::Nixos.as_str().into()));
    }
    let kind = o.get("kind").and_then(Value::as_str).and_then(GuestKind::parse).unwrap_or(GuestKind::Nixos);
    o.insert("system".into(), Value::String(Platform::legacy_system(kind).into()));
    Ok(())
}

static INSTANCE_STEPS: [Step; 1] = [instance_v0_to_v1];

/// Contents of instances/<name>.omaflake-vm/instance.json. Settings are copied from the recipe on
/// `omaf new` and belong to the instance afterwards.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstanceConfig {
    pub name: String,
    pub recipe: String,
    /// Image name, e.g. "dev-ab12cd34ef-1a2b3c4d" (folder under ~/.omaflake/images)
    pub image: String,
    /// Absolute path of the base image (backing file of the overlay)
    /// Where the image was when the VM was made (informational since plan B108: the overlay names its image by a
    /// relative path, `InstanceStore::backing_image` finds it)
    #[serde(rename = "imagePath")]
    pub image_path: String,
    pub cpus: u32,
    #[serde(rename = "memoryGB")]
    pub memory_gb: u32,
    #[serde(rename = "diskGB")]
    pub disk_gb: u32,
    pub network: NetworkMode,
    pub share: bool,
    pub ephemeral: bool,
    #[serde(rename = "sshPort")]
    pub ssh_port: u16,
    #[serde(with = "crate::json::iso8601")]
    pub created: DateTime<Utc>,
    /// Login user inside the guest; missing in instances created before this field existed
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    pub kind: GuestKind,
    /// What it runs as – its image's system (`Platform`)
    pub system: String,
    /// Sound to the Mac; None = on
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub audio: Option<bool>,
    /// The Mac's microphone; None = off
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub microphone: Option<bool>,
    /// Clipboard shared with the Mac; None = on
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub clipboard: Option<bool>,
    /// The Mac's camera; None = off
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub camera: Option<bool>,
    /// How a plain start shows the VM (from the recipe); None = headless in the CLI, a window in the app
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display: Option<DisplayMode>,
    /// Share folder the user chose (absolute path); None = the default `share/<name>` in omaflake's folder
    #[serde(rename = "sharePath", default, skip_serializing_if = "Option::is_none")]
    pub share_path: Option<String>,
    /// The VM's identity as three words (its omaflake ID `vm:<name>:<words>`, plan B110): random when it is made, new
    /// for a copy under another name, kept by a restore
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity: Option<String>,
}

impl InstanceConfig {
    /// instance.json (see `instance_v0_to_v1`)
    pub const FILE_FORMAT: FileFormat = FileFormat::new("instance", "instance.json", 0, &INSTANCE_STEPS);

    /// Swift's `init(name:recipe:image:imagePath:system:settings:sshPort:created:)`; pass `json::now()` for Swift's
    /// default `created: Date()`
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        name: &str,
        recipe: &str,
        image: &str,
        image_path: &str,
        system: &str,
        settings: &RecipeDefaults,
        ssh_port: u16,
        created: DateTime<Utc>,
    ) -> Self {
        InstanceConfig {
            name: name.into(),
            recipe: recipe.into(),
            image: image.into(),
            image_path: image_path.into(),
            cpus: settings.cpus,
            memory_gb: settings.memory_gb,
            disk_gb: settings.disk_gb,
            network: settings.network,
            share: settings.share,
            ephemeral: settings.ephemeral,
            ssh_port,
            created,
            user: settings.user.clone(),
            kind: settings.kind,
            system: system.into(),
            audio: settings.audio,
            microphone: settings.microphone,
            clipboard: settings.clipboard,
            camera: settings.camera,
            display: settings.display,
            share_path: None,
            identity: None,
        }
    }

    pub fn clipboard_on(&self) -> bool {
        self.clipboard.unwrap_or(true)
    }

    pub fn camera_on(&self) -> bool {
        self.camera.unwrap_or(false)
    }

    pub fn sound_on(&self) -> bool {
        self.audio.unwrap_or(true)
    }

    pub fn microphone_on(&self) -> bool {
        self.sound_on() && self.microphone.unwrap_or(false)
    }

    pub fn guest_kind(&self) -> GuestKind {
        self.kind
    }

    /// User for SSH: the recipe's `user`, otherwise the user running omaflake on the Mac.
    pub fn guest_user(&self) -> String {
        self.user.clone().unwrap_or_else(crate::platform::user_name)
    }
}

/// Recipe identity: <author>/<kind>/<name> – e.g. omaflake/nixos/minimal and omaflake/macos/minimal. As an ID (flake
/// outputs, images, instances, `current/` links) it is "<author>.<kind>.<name>"; users see and type the name, or as much
/// of <author>/<kind>/<name> as makes it unique.
/// A recipe's ID (plan B110), the file form of its omaflake ID (`OmaflakeId`): "<author>.<kind>.<name>@<words>" – the
/// base (author, kind, name) and its content hash (`RecipeLibrary::content_hash`) as words. The same base with other
/// content is another recipe, also from the same source; the same base with the same content exists once. A recipe's
/// **line** is its source and base (`line_key`): a changed recipe is a new recipe in the same line – images are outdated
/// and VMs upgrade within their line. IDs without a hash are from before B110 (migrated) or from the plain recipe layout
/// (`nix flake check` against a recipes checkout).
pub enum RecipeId {}

/// Picks one of several matching recipes (e.g. asks in a terminal); None = none chosen
pub type Chooser<'a> = &'a dyn Fn(&[String]) -> Option<String>;

/// (author, kind, name) of a recipe ID
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecipeParts {
    pub author: String,
    pub kind: GuestKind,
    pub name: String,
}

impl RecipeId {
    /// "<author>.<kind>.<name>" (Swift `make(author:kind:name:)`)
    pub fn make(author: &str, kind: GuestKind, name: &str) -> String {
        format!("{author}.{}.{name}", kind.as_str())
    }

    /// "omaflake.nixos.minimal" + a content hash (hex) → "omaflake.nixos.minimal@velvet-tiger-orbit-…" (Swift
    /// `make(base:contentHash:words:)`; `words` is `OmaflakeId::LENGTH` there by default)
    pub fn make_with_hash(base: &str, content_hash: &str, words: usize) -> String {
        format!("{base}@{}", OmaflakeId::words_hex(content_hash, words))
    }

    /// "omaflake.nixos.minimal@velvet-tiger-orbit" → "omaflake.nixos.minimal"
    pub fn base(id: &str) -> &str {
        // Swift's split(maxSplits: 1) leaves out empty parts
        let trimmed = id.trim_start_matches('@');
        match trimmed.split_once('@') {
            Some((b, _)) => b,
            None if trimmed.is_empty() => id,
            None => trimmed,
        }
    }

    /// "omaflake.nixos.minimal@velvet-tiger-orbit" → "velvet-tiger-orbit"; None without words
    pub fn words(id: &str) -> Option<&str> {
        let trimmed = id.trim_start_matches('@');
        let (_, w) = trimmed.split_once('@')?;
        let w = w.trim_start_matches('@');
        (!w.is_empty()).then_some(w)
    }

    /// The line of a recipe – its source (`RecipeSource::id`) and base – as a file-name-safe key ("<base>~<source tag>")
    pub fn line_key(base: &str, source: &str) -> String {
        format!("{}~{}", Self::base(base), RecipeSource::tag_for_key(source))
    }

    /// (author, kind, name) of an ID, with or without its hash; None for anything else
    pub fn parts(id: &str) -> Option<RecipeParts> {
        let p: Vec<&str> = Self::base(id).split('.').collect();
        if p.len() != 3 {
            return None;
        }
        let kind = GuestKind::parse(p[1])?;
        Some(RecipeParts { author: p[0].into(), kind, name: p[2].into() })
    }

    /// What to show next to a recipe's name so its line can be told apart (plan B110, B111): its source when a recipe of
    /// the same base comes from another source; None otherwise. Versions of one line are told apart by their words (shown
    /// as short as `OmaflakeId` lengths allow). `entries`: the recipes there are, ID → source.
    pub fn distinction(
        id: &str,
        entries: &HashMap<String, String>,
        source_name: &dyn Fn(&str) -> String,
    ) -> Option<String> {
        let source = entries.get(id)?;
        let other = entries.iter().any(|(k, v)| k != id && Self::base(k) == Self::base(id) && v != source);
        other.then(|| source_name(source))
    }

    /// Name to show: the recipe's `name`, else the name part of the ID
    pub fn display_name(id: &str, d: Option<&RecipeDefaults>) -> String {
        d.and_then(|d| d.name.clone()).unwrap_or_else(|| Self::name_of_id(id))
    }

    /// "omaflake.nixos.minimal" → "minimal"; other strings stay as they are
    pub fn name_of_id(id: &str) -> String {
        Self::parts(id).map(|p| p.name).unwrap_or_else(|| id.to_string())
    }

    /// "omaflake.nixos.minimal" → "omaflake/nixos/minimal"
    pub fn qualified(id: &str) -> String {
        Self::parts(id)
            .map(|p| format!("{}/{}/{}", p.author, p.kind.as_str(), p.name))
            .unwrap_or_else(|| id.to_string())
    }

    /// Recipes matching what the user typed: an ID, <author>/<kind>/<name>, <kind>/<name> or <name>
    /// (Swift `candidates(_:in:)` over a recipe-defaults dictionary).
    pub fn candidates_in(input: &str, defaults: &HashMap<String, RecipeDefaults>) -> Vec<String> {
        let ids: Vec<String> = defaults.keys().cloned().collect();
        Self::candidates(input, &ids)
    }

    /// "…@<words>" (or the start of them) in the input picks one; so does a recipe's omaflake ID.
    pub fn candidates(input: &str, ids: &[String]) -> Vec<String> {
        if ids.iter().any(|i| i == input) {
            return vec![input.to_string()];
        }
        if let Some(o) = OmaflakeId::parse(input) {
            if o.kind == OmaflakeIdKind::Recipe {
                if let Some(base) = RecipeRef::public_id(&o.identifier) {
                    let mut found: Vec<String> = ids
                        .iter()
                        .filter(|id| Self::base(id) == base && Self::words(id).is_some_and(|w| w.starts_with(&o.words)))
                        .cloned()
                        .collect();
                    found.sort();
                    return found;
                }
            }
        }
        let wanted = Self::words(input);
        let hash_matches = |id: &str| wanted.map(|w| Self::words(id).is_some_and(|x| x.starts_with(w))).unwrap_or(true);
        let p: Vec<&str> = Self::base(input).split('/').filter(|s| !s.is_empty()).collect();
        if p.len() == 1 && input.contains('.') && ids.iter().any(|id| Self::base(id) == Self::base(input)) {
            let mut found: Vec<String> =
                ids.iter().filter(|id| Self::base(id) == Self::base(input) && hash_matches(id)).cloned().collect();
            found.sort();
            return found;
        }
        let unique: HashSet<&String> = ids.iter().collect();
        let mut found: Vec<String> = unique
            .into_iter()
            .filter(|id| {
                let Some(f) = Self::parts(id) else { return false };
                if !hash_matches(id) {
                    return false;
                }
                match p.len() {
                    3 => f.author == p[0] && f.kind.as_str() == p[1] && f.name == p[2],
                    2 => f.kind.as_str() == p[0] && f.name == p[1],
                    1 => f.name == p[0],
                    _ => false,
                }
            })
            .cloned()
            .collect();
        found.sort();
        found
    }

    /// The single recipe for `input` among the recipe defaults (Swift `resolve(_:in:choose:)`)
    pub fn resolve_in(
        input: &str,
        defaults: &HashMap<String, RecipeDefaults>,
        choose: Option<Chooser>,
    ) -> Result<String> {
        let ids: Vec<String> = defaults.keys().cloned().collect();
        Self::resolve(input, &ids, choose)
    }

    /// The single recipe for `input`; `choose` decides between several (e.g. asks in a terminal), None = error.
    pub fn resolve(input: &str, ids: &[String], choose: Option<Chooser>) -> Result<String> {
        let found = Self::candidates(input, ids);
        if found.len() == 1 {
            return Ok(found[0].clone());
        }
        if found.is_empty() {
            return Err(Error::new(format!("Recipe \"{input}\" is not in your recipes")));
        }
        if let Some(choose) = choose {
            if let Some(picked) = choose(&found) {
                return Ok(picked);
            }
        }
        // As short as the words tell them apart, per base
        let mut groups: HashMap<&str, Vec<String>> = HashMap::new();
        for id in &found {
            groups.entry(Self::base(id)).or_default().extend(Self::words(id).map(str::to_string));
        }
        let n = OmaflakeId::shortest(groups.values().cloned());
        let names: Vec<String> = found.iter().map(|id| OmaflakeId::recipe(id, n)).collect();
        Err(Error::new(format!("Recipe \"{input}\" exists several times – choose one: {}", names.join(", "))))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::json::parse_date;

    pub fn dev_defaults() -> RecipeDefaults {
        RecipeDefaults::new(GuestKind::Nixos, None, 8, 16, 100, NetworkMode::Shared, true, false, None)
    }

    /// Swift test helper `config(_:network:ephemeral:share:)`
    pub fn config(name: &str) -> InstanceConfig {
        InstanceConfig::new(
            name,
            "dev",
            "dev-abc-12345678",
            "/img.qcow2",
            "aarch64-linux",
            &dev_defaults(),
            2230,
            parse_date("1970-01-01T00:00:00Z").unwrap(),
        )
    }

    #[test]
    fn decodes_flake_json() {
        // Output of `nix eval --json .#lib.recipeDefaults` (shortened, 2026-10-16)
        let json = r#"{"lab":{"systems":["aarch64-linux"],"cpus":4,"disk_gb":60,"ephemeral":true,"memory_gb":8,"network":"isolated","share":false}}"#;
        let d: HashMap<String, RecipeDefaults> = serde_json::from_str(json).unwrap();
        let expected = RecipeDefaults::new(
            GuestKind::Nixos,
            Some(vec!["aarch64-linux".into()]),
            4,
            8,
            60,
            NetworkMode::Isolated,
            false,
            true,
            None,
        );
        assert_eq!(d["lab"], expected);
    }

    #[test]
    fn decodes_kind() {
        let json = r#"{"omaflake.macos.minimal":{"kind":"macos","systems":["aarch64-darwin"],"author":"omaflake","name":"minimal","cpus":4,"disk_gb":60,"ephemeral":false,"memory_gb":8,"network":"shared","share":true,"user":"me"}}"#;
        let d: HashMap<String, RecipeDefaults> = serde_json::from_str(json).unwrap();
        let m = &d["omaflake.macos.minimal"];
        assert!(m.kind == GuestKind::Macos && m.author.as_deref() == Some("omaflake"));
        assert_eq!(m.user.as_deref(), Some("me"));
    }

    #[test]
    fn display_description_lab_from_toml_json() {
        let json = r#"{"systems":["aarch64-linux"],"cpus":4,"memory_gb":4,"disk_gb":40,"network":"shared","share":true,"ephemeral":false,"display":"gpu","description":"NixOS","description_de":"NixOS (de)","lab":true}"#;
        let d: RecipeDefaults = serde_json::from_str(json).unwrap();
        assert!(d.display == Some(DisplayMode::Gpu) && d.is_lab());
        assert_eq!(d.description_in(Some("de")), Some("NixOS (de)"));
        assert_eq!(d.description_in(Some("fr")), Some("NixOS"));
        let c = InstanceConfig::new("a", "f", "i", "/i", "aarch64-linux", &d, 2222, crate::json::now());
        assert_eq!(c.display, Some(DisplayMode::Gpu));
        // encoding keeps the fields (round trip)
        let again: RecipeDefaults = serde_json::from_slice(&serde_json::to_vec(&d).unwrap()).unwrap();
        assert_eq!(again, d);
        let text = serde_json::to_string(&d).unwrap();
        assert!(text.contains("\"description_de\"") && !text.contains("\"tags\"") && !text.contains("\"user\""));
    }

    #[test]
    fn recipes_need_systems() {
        let without = r#"{"cpus":1,"memory_gb":1,"disk_gb":1,"network":"none","share":false,"ephemeral":false}"#;
        assert!(serde_json::from_str::<RecipeDefaults>(without).is_err());
        let x86 = r#"{"systems":["x86_64-linux"],"cpus":1,"memory_gb":1,"disk_gb":1,"network":"none","share":false,"ephemeral":false}"#;
        let d: RecipeDefaults = serde_json::from_str(x86).unwrap();
        assert_eq!(d.runs_here(), Platform::host_system(GuestKind::Nixos) == Some("x86_64-linux"));
    }

    #[test]
    fn instance_without_share_path_decodes() {
        let data = serde_json::to_vec(&config("s3")).unwrap();
        let c: InstanceConfig = serde_json::from_slice(&data).unwrap();
        assert_eq!(c.share_path, None);
        assert_eq!(c, config("s3"));
    }

    /// instance.json before B114: no $schema, no system, maybe no kind and the old "flavor" key
    #[test]
    fn instance_version_zero_is_migrated() {
        let old = r#"{"name":"a","flavor":"dev","image":"i","imagePath":"/i","cpus":2,"memoryGB":2,"diskGB":10,"network":"shared","share":false,"ephemeral":false,"sshPort":2222,"created":"2026-10-01T00:00:00Z"}"#;
        let (c, migrated) = InstanceConfig::FILE_FORMAT.decode::<InstanceConfig>(old.as_bytes()).unwrap();
        assert!(migrated);
        assert!(c.recipe == "dev" && c.kind == GuestKind::Nixos && c.system == "aarch64-linux");
        let current = InstanceConfig::FILE_FORMAT.encode(&c).unwrap();
        assert!(!InstanceConfig::FILE_FORMAT.decode::<InstanceConfig>(&current).unwrap().1);
    }

    /// A file the Swift version wrote (testdata/instance-v1.json) reads, and writing it again gives the same bytes
    #[test]
    fn swift_instance_fixture_round_trips() {
        let data = include_bytes!("../testdata/instance-v1.json");
        let (c, migrated) = InstanceConfig::FILE_FORMAT.decode::<InstanceConfig>(data).unwrap();
        assert!(!migrated);
        assert_eq!(c.memory_gb, 16);
        assert_eq!(c.share_path.as_deref(), Some("/Volumes/data/share"));
        assert_eq!(crate::json::iso8601::format(&c.created), "2026-10-21T10:00:00Z");
        let written = InstanceConfig::FILE_FORMAT.encode(&c).unwrap();
        assert_eq!(String::from_utf8(written).unwrap(), String::from_utf8_lossy(data).trim_end());
    }

    fn d(author: &str, kind: GuestKind, name: &str) -> RecipeDefaults {
        let mut f = RecipeDefaults::new(kind, None, 4, 4, 40, NetworkMode::Shared, true, false, None);
        f.name = Some(name.into());
        f.author = Some(author.into());
        f
    }

    fn defaults() -> HashMap<String, RecipeDefaults> {
        HashMap::from([
            ("omaflake.nixos.minimal".to_string(), d("omaflake", GuestKind::Nixos, "minimal")),
            ("omaflake.macos.minimal".to_string(), d("omaflake", GuestKind::Macos, "minimal")),
            ("omaflake.nixos.dev".to_string(), d("omaflake", GuestKind::Nixos, "dev")),
            ("alice.nixos.dev".to_string(), d("alice", GuestKind::Nixos, "dev")),
        ])
    }

    #[test]
    fn resolves_as_much_as_needed() {
        let defaults = defaults();
        assert_eq!(RecipeId::resolve_in("omaflake.nixos.dev", &defaults, None).unwrap(), "omaflake.nixos.dev");
        assert_eq!(RecipeId::resolve_in("macos/minimal", &defaults, None).unwrap(), "omaflake.macos.minimal");
        assert_eq!(RecipeId::resolve_in("alice/nixos/dev", &defaults, None).unwrap(), "alice.nixos.dev");
        assert!(RecipeId::resolve_in("minimal", &defaults, None).is_err()); // two guest kinds
        assert!(RecipeId::resolve_in("nixos/dev", &defaults, None).is_err()); // two authors
        assert!(RecipeId::resolve_in("nope", &defaults, None).is_err());
        // the chooser decides between several
        let last = |c: &[String]| c.last().cloned();
        assert_eq!(RecipeId::resolve_in("dev", &defaults, Some(&last)).unwrap(), "omaflake.nixos.dev");
        assert_eq!(RecipeId::candidates_in("dev", &defaults), vec!["alice.nixos.dev", "omaflake.nixos.dev"]);
    }

    #[test]
    fn parts_and_names() {
        assert_eq!(RecipeId::parts("omaflake.macos.minimal").unwrap().kind, GuestKind::Macos);
        assert!(RecipeId::parts("minimal").is_none() && RecipeId::parts("x.windows.y").is_none());
        assert_eq!(RecipeId::name_of_id("omaflake.macos.minimal"), "minimal");
        assert_eq!(RecipeId::qualified("alice.nixos.dev"), "alice/nixos/dev");
        assert_eq!(RecipeId::base("a.nixos.b@x-y"), "a.nixos.b");
        assert_eq!(RecipeId::words("a.nixos.b@x-y"), Some("x-y"));
        assert_eq!(RecipeId::words("a.nixos.b"), None);
    }

    /// The source next to a name only when another source has a recipe of the same base (plan B110, B111)
    #[test]
    fn distinction_only_when_needed() {
        let (a, b, c) = ("o.nixos.m@abandon", "o.nixos.m@zoo", "o.nixos.x@zoo");
        let name = |s: &str| s.to_uppercase();
        let m = |pairs: &[(&str, &str)]| {
            pairs.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<HashMap<_, _>>()
        };
        assert_eq!(RecipeId::distinction(c, &m(&[(a, "s1"), (b, "s2"), (c, "s1")]), &name), None);
        assert_eq!(RecipeId::distinction(a, &m(&[(a, "s1"), (b, "s2")]), &name), Some("S1".into()));
        assert_eq!(RecipeId::distinction(a, &m(&[(a, "s1"), (b, "s1")]), &name), None);
        // versions: words in sub-rows
    }

    #[test]
    fn stop_result_texts() {
        assert_eq!(StopResult::Powerdown.to_string(), "shut down cleanly");
        assert_eq!(StopResult::ForcedOff.to_string(), "forced off (omaf-runner kill)");
    }
}
