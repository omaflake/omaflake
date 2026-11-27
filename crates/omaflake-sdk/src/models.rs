// SPDX-License-Identifier: AGPL-3.0-or-later
//! The kinds of the API that the core's own models are made of too.

use serde::{Deserialize, Serialize};

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

/// Swift `Prerequisite.Item`; serialized as Swift's raw value (camelCase: "nixDarwin", "linuxBuilder", …)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum PrerequisiteItem {
    // what the Mac brings
    Host,
    /// Linux hosts: /dev/kvm usable (else VMs run with TCG, plan B124)
    Kvm,
    Nix,
    Flakes,
    /// Nix in the user's login shells (plan B160; optional – omaflake finds Nix by its paths)
    Shell,
    NixDarwin,
    LinuxBuilder,
    Qemu,
    Flake,
    SshKey,
    /// At least one credential – the default one (plan B139): who you are in the guests, with its password
    Credential,
    /// At least one installed look – the default one (plan B141): how the guests look
    Look,
    Git,
    /// The `omaf` command on the user's PATH (optional, plan B122)
    Cli,
    // preflight NixOS: builder VM answering, recipes evaluating, space in the Nix store
    BuilderRunning,
    FlakeEval,
    NixSpace,
    // preflight macOS: IPSW from Apple, base, space for installation and images
    Ipsw,
    MacBase,
    MacSpace,
}

impl PrerequisiteItem {
    /// Swift `rawValue` (also the `id`)
    pub fn as_str(self) -> &'static str {
        match self {
            PrerequisiteItem::Host => "host",
            PrerequisiteItem::Kvm => "kvm",
            PrerequisiteItem::Nix => "nix",
            PrerequisiteItem::Flakes => "flakes",
            PrerequisiteItem::Shell => "shell",
            PrerequisiteItem::NixDarwin => "nixDarwin",
            PrerequisiteItem::LinuxBuilder => "linuxBuilder",
            PrerequisiteItem::Qemu => "qemu",
            PrerequisiteItem::Flake => "flake",
            PrerequisiteItem::SshKey => "sshKey",
            PrerequisiteItem::Credential => "credential",
            PrerequisiteItem::Look => "look",
            PrerequisiteItem::Git => "git",
            PrerequisiteItem::Cli => "cli",
            PrerequisiteItem::BuilderRunning => "builderRunning",
            PrerequisiteItem::FlakeEval => "flakeEval",
            PrerequisiteItem::NixSpace => "nixSpace",
            PrerequisiteItem::Ipsw => "ipsw",
            PrerequisiteItem::MacBase => "macBase",
            PrerequisiteItem::MacSpace => "macSpace",
        }
    }
}

/// Swift `Prerequisite.Status`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PrerequisiteStatus {
    Ok,
    Limited,
    Missing,
}

/// Swift `RecipeSource.Kind`
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RecipeSourceKind {
    Github,
    Url,
    Folder,
}

/// What a VM from the recipe needs at least (`[requirements]`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Requirements {
    pub cpus: u32,
    pub memory_gb: u32,
    pub disk_gb: u32,
}

/// The locales a recipe works with (`[locales]`): no right-to-left languages, or only these languages
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LocaleRules {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtl: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
}

impl LocaleRules {
    /// Languages written right to left (ISO 639-1/-3)
    const RTL: [&'static str; 9] = ["ar", "he", "fa", "ur", "yi", "ps", "dv", "sd", "ug"];

    /// Why a locale ("de_DE.UTF-8") is outside these rules – None when it is fine
    pub fn outside(&self, locale: &str) -> Option<Outside> {
        let language = locale.split(['_', '.']).next().unwrap_or(locale);
        if !self.languages.is_empty() && !self.languages.iter().any(|l| l == language) {
            return Some(Outside::Languages(self.languages.clone()));
        }
        if self.rtl == Some(false) && Self::RTL.contains(&language) {
            return Some(Outside::RightToLeft);
        }
        None
    }
}

/// Why a locale is not one a recipe is made for
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outside {
    /// Made for these languages only
    Languages(Vec<String>),
    RightToLeft,
}

impl LocaleRules {
    pub fn is_empty(&self) -> bool {
        self.rtl.is_none() && self.languages.is_empty()
    }
}

/// A key bound to a module's action in a recipe
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    /// `"<author>/<name>"` of the module
    pub module: String,
    pub action: String,
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_de: Option<String>,
    /// Each of them does it
    pub keys: Vec<String>,
    /// Where it works: "desktop" or a module ("omaflake/neovim")
    pub context: String,
    /// The module's modes it works in; empty: whenever its program has the keys
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub modes: Vec<String>,
    /// The mode afterwards (plan B147); None: it stays
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub does: Option<String>,
    /// The program's own key, shown only
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub own: bool,
    /// The key binding set that gave the key; None: the module's default
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

impl Binding {
    /// For people: "Ctrl+g", "h / Left"
    pub fn key_text(&self) -> String {
        self.keys.join(" / ")
    }
}

/// What is wrong with a recipe – blockers (`blocks`) and warnings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Issue {
    /// A module of the recipe is not here (`have`: here, but in a version the recipe does not allow)
    #[serde(rename_all = "camelCase")]
    MissingModule { reference: String, have: Option<String> },
    #[serde(rename_all = "camelCase")]
    MissingKeymap { reference: String, have: Option<String> },
    /// A module needs a capability no module of the recipe provides
    #[serde(rename_all = "camelCase")]
    Needs { module: String, capability: String },
    /// Modules providing the same exclusive capability
    #[serde(rename_all = "camelCase")]
    Incompatible { capability: String, modules: Vec<String> },
    #[serde(rename_all = "camelCase")]
    KeysOverlap { first: Box<Binding>, second: Box<Binding> },
    /// Without a mode a key starts with Super or Ctrl (an editor's own keys excepted)
    #[serde(rename_all = "camelCase")]
    KeyRule { binding: Box<Binding> },
}

impl Issue {
    /// It stops a build: the image flake would not find the part
    pub fn blocks(&self) -> bool {
        matches!(self, Issue::MissingModule { .. } | Issue::MissingKeymap { .. })
    }
}

/// A mode of a module's program (module.toml `[modes.<name>]`): its keys differ there
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModuleMode {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_de: Option<String>,
    /// The program starts in it
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub start: bool,
}

impl ModuleMode {
    pub fn title_in(&self, language: Option<&str>) -> &str {
        match (language, &self.title_de) {
            (Some("de"), Some(t)) => t,
            _ => &self.title,
        }
    }
}

/// The main colours of a look in the catalog, for a preview without downloading it
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Swatches {
    #[serde(default)]
    pub background: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub accent: String,
    #[serde(default)]
    pub urgent: String,
    /// The terminal's 16 colours
    #[serde(default)]
    pub terminal: Vec<String>,
}
