// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Prerequisites.swift – work package F5.
//!
//! What the user's Mac brings (spec §11.2): detected without elevated rights, shared by `omaf doctor` and the app's
//! Requirements page. Nothing here installs anything – `install` is a command the user runs in Terminal. The `omaf`
//! command's check (`cli`) and its Install/Uninstall live in cli_link.rs.
//!
//! Not 1:1: `host()` read `ProcessInfo.operatingSystemVersion`; Rust reads `kern.osproductversion` (sysctl, the same
//! value `sw_vers -productVersion` prints) on macOS. Linux hosts (plan B124, issue #2) get their own list: the host
//! (kernel, architecture), KVM, Nix, flakes, QEMU with its firmware, the flake, the SSH key, git – no nix-darwin, Linux
//! builder or macOS items (images build natively, macOS guests need a Mac). The IPSW check asks `IpswDownloader::resume_file_for` (`.partial`, see ipsw_downloader.rs) instead
//! of URLSession's `.resume`. The macOS base check picks the first differing recipe `macUser` in sorted order (Swift: a
//! `Set`, so any).

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize, Serializer};

use crate::builder_maintenance::{shell_quote, BuilderMaintenance, NixVolume};
use crate::cli_link::{CliCheck, CliLink};
use crate::image_store::ImageStore;
use crate::ipsw_downloader::{free_space, gigabytes, IpswDownloader, MacOsRelease};
use crate::mac_base_installer::MacBaseInstaller;
use crate::mac_image_builder::MacImageBuilder;
use crate::models::{GuestKind, RecipeId};
use crate::paths::{Paths, Settings};
use crate::platform::{expand_tilde, home_dir, is_executable_file, kernel_release, machine};
use crate::qemu_host::KvmState;
use crate::qemu_runtime::QemuRuntime;
use crate::shell::{RunOptions, Shell};
use crate::sockets::{Ports, DEFAULT_TIMEOUT};

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

/// One check's result. JSON (docs/api.md §4.4): camelCase keys, the computed `required` and `settablePath` written as
/// fields, optional fields left out when unset; reading ignores `required` and `settablePath`.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Prerequisite {
    pub item: PrerequisiteItem,
    pub status: PrerequisiteStatus,
    /// What was found (path, version) or what is missing
    pub detail: String,
    /// What to do about it (empty when ok)
    pub hint: String,
    /// Command for Terminal that installs it (official installers only), None if there is none
    #[serde(default)]
    pub install: Option<String>,
    /// Space checks: free and needed bytes (for a localized display; `detail` has them in English)
    #[serde(default)]
    pub free_bytes: Option<i64>,
    #[serde(default)]
    pub needed_bytes: Option<i64>,
    /// macOS base: its guest user, and the recipes' macUser when it differs
    #[serde(default)]
    pub base_user: Option<String>,
    #[serde(default)]
    pub expected_user: Option<String>,
}

/// The JSON shape of a `Prerequisite` (api.md §4.4)
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PrerequisiteJson<'a> {
    item: PrerequisiteItem,
    status: PrerequisiteStatus,
    required: bool,
    detail: &'a str,
    hint: &'a str,
    #[serde(skip_serializing_if = "Option::is_none")]
    install: Option<&'a str>,
    settable_path: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    free_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    needed_bytes: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    base_user: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    expected_user: Option<&'a str>,
}

impl Serialize for Prerequisite {
    fn serialize<S: Serializer>(&self, s: S) -> std::result::Result<S::Ok, S::Error> {
        PrerequisiteJson {
            item: self.item,
            status: self.status,
            required: self.required(),
            detail: &self.detail,
            hint: &self.hint,
            install: self.install.as_deref(),
            settable_path: self.settable_path(),
            free_bytes: self.free_bytes,
            needed_bytes: self.needed_bytes,
            base_user: self.base_user.as_deref(),
            expected_user: self.expected_user.as_deref(),
        }
        .serialize(s)
    }
}

impl Prerequisite {
    pub fn new(
        item: PrerequisiteItem,
        status: PrerequisiteStatus,
        detail: &str,
        hint: &str,
        install: Option<String>,
    ) -> Self {
        Prerequisite {
            item,
            status,
            detail: detail.into(),
            hint: hint.into(),
            install,
            free_bytes: None,
            needed_bytes: None,
            base_user: None,
            expected_user: None,
        }
    }

    pub fn id(&self) -> &'static str {
        self.item.as_str()
    }

    /// The user can enter a path for it (`Settings.nix`, `Settings.qemu`, `Settings.flake`)
    pub fn settable_path(&self) -> bool {
        matches!(self.item, PrerequisiteItem::Nix | PrerequisiteItem::Qemu | PrerequisiteItem::Flake)
    }

    /// Needed for every feature; `limited` items still allow some. git and the `omaf` command are optional.
    pub fn required(&self) -> bool {
        !matches!(self.item, PrerequisiteItem::Git | PrerequisiteItem::Cli)
    }
}

#[derive(Debug, Clone)]
pub struct Prerequisites {
    pub paths: Paths,
}

/// `ok ? a : b` for the many status choices
fn status(ok: bool, otherwise: PrerequisiteStatus) -> PrerequisiteStatus {
    if ok {
        PrerequisiteStatus::Ok
    } else {
        otherwise
    }
}

/// Names in a folder (sorted), empty when it cannot be read
fn sorted_names(dir: &Path) -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir(dir)
        .map(|entries| entries.flatten().map(|e| e.file_name().to_string_lossy().into_owned()).collect())
        .unwrap_or_default();
    names.sort();
    names
}

/// The last path component as text (Swift `lastPathComponent`)
fn last_component(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
}

/// Swift `CharacterSet.whitespaces`: spaces and tabs (no newlines)
fn trim_spaces(s: &str) -> &str {
    s.trim_matches(|c: char| c.is_whitespace() && c != '\n' && c != '\r')
}

impl Prerequisites {
    // MARK: Official install commands (typed into Terminal by the user's click)

    /// nix-darwin's documented flake setup in /etc/nix-darwin (README of nix-darwin)
    /// nix-darwin with omaflake's Linux builder host, in two stages (`omaf builder setup`; the app's "Set Up …")
    pub const INSTALL_NIX_DARWIN: &'static str = "omaf builder setup";
    /// omaflake's patched QEMU from the flake (spec §11.2)
    pub const INSTALL_QEMU: &'static str = "nix profile install github:omaflake/omaflake#qemu";
    pub const INSTALL_SSH_KEY: &'static str = "ssh-keygen -t ed25519";
    pub const INSTALL_GIT: &'static str = "xcode-select --install";
    /// Linux hosts (plan B124): nixpkgs' QEMU (with virglrenderer, GTK and QEMU's edk2 firmware), git, Nix's
    /// official multi-user installer (nixos.org/download)
    pub const INSTALL_QEMU_LINUX: &'static str = "nix profile install nixpkgs#qemu";
    pub const INSTALL_GIT_LINUX: &'static str = "nix profile install nixpkgs#git";
    pub const INSTALL_NIX_LINUX: &'static str = "curl -L https://nixos.org/nix/install | sh -s -- --daemon";
    /// Other distributions than NixOS: the user into the group kvm (log in again afterwards)
    pub const INSTALL_KVM_GROUP: &'static str = "sudo usermod -aG kvm \"$USER\"";

    /// Space the Nix store should have for building NixOS images
    pub const NIX_SPACE_NEEDED: i64 = 20_000_000_000;
    /// Space a macOS installation and one image need besides the IPSW
    pub const MAC_SPACE_NEEDED: i64 = 30_000_000_000;

    pub fn new(paths: Paths) -> Self {
        Prerequisites { paths }
    }

    /// The official installer of the NixOS community (also what the app runs through the administrator dialog):
    /// `sudo sh -c '<BuilderMaintenance::install_nix_command()>'`; on Linux Nix's own installer
    pub fn install_nix() -> String {
        if cfg!(target_os = "linux") {
            return Self::INSTALL_NIX_LINUX.into();
        }
        format!("sudo sh -c '{}'", BuilderMaintenance::install_nix_command())
    }

    /// All checks, in the order shown: host, nix, flakes, nix-darwin, Linux builder, QEMU, flake, SSH key, git, the
    /// `omaf` command.
    pub fn check(&self) -> Vec<Prerequisite> {
        self.check_with_cli().0
    }

    /// `check()`, and what the `omaf` command's check found (for Install/Uninstall, cli_link.rs)
    pub fn check_with_cli(&self) -> (Vec<Prerequisite>, CliCheck) {
        let cli = CliLink::current().check();
        let mut items = self.check_without_cli();
        items.push(cli.item.clone());
        (items, cli)
    }

    fn check_without_cli(&self) -> Vec<Prerequisite> {
        let settings = Settings::load(&self.paths);
        let nix = Self::locate_nix(&settings);
        let config = nix.as_deref().map(Self::nix_config).unwrap_or_default();
        if cfg!(target_os = "linux") {
            return vec![
                self.host(),
                Self::kvm_item(KvmState::detect(), Path::new("/etc/NIXOS").exists()),
                self.nix_item(nix.as_deref()),
                self.flakes(nix.as_deref(), &config),
                self.qemu(),
                self.flake(&settings, nix.as_deref()),
                self.ssh_key(),
                self.credential(),
                self.look(),
                self.git(),
            ];
        }
        vec![
            self.host(),
            self.nix_item(nix.as_deref()),
            self.flakes(nix.as_deref(), &config),
            self.nix_darwin(),
            self.linux_builder(nix.as_deref(), &config),
            self.qemu(),
            self.flake(&settings, nix.as_deref()),
            self.ssh_key(),
            self.credential(),
            self.look(),
            self.git(),
        ]
    }

    // MARK: Checks

    pub(crate) fn host(&self) -> Prerequisite {
        if cfg!(target_os = "linux") {
            return Self::linux_host_item(&kernel_release(), &machine());
        }
        Self::host_item(Self::os_version().as_deref(), &machine())
    }

    /// A Linux host (plan B124): x86_64 and aarch64 run NixOS guests of their own architecture
    pub(crate) fn linux_host_item(release: &str, machine: &str) -> Prerequisite {
        let ok = matches!(machine, "x86_64" | "aarch64");
        Prerequisite::new(
            PrerequisiteItem::Host,
            status(ok, PrerequisiteStatus::Missing),
            &format!("Linux {release}, {machine}"),
            if ok { "" } else { "omaflake runs on Linux with x86_64 or aarch64" },
            None,
        )
    }

    /// /dev/kvm (plan B124): usable – VMs run with KVM; else they run with TCG, many times slower (`limited`). The hint
    /// names the fix: the group kvm (NixOS: declared in the configuration), or virtualization in the firmware.
    pub(crate) fn kvm_item(state: KvmState, nixos: bool) -> Prerequisite {
        match state {
            KvmState::Usable => Prerequisite::new(
                PrerequisiteItem::Kvm,
                PrerequisiteStatus::Ok,
                "/dev/kvm – VMs run with KVM",
                "",
                None,
            ),
            KvmState::NoAccess => Prerequisite::new(
                PrerequisiteItem::Kvm,
                PrerequisiteStatus::Limited,
                "/dev/kvm: no access – VMs run with TCG (slow)",
                if nixos {
                    "Add yourself to the group kvm: users.users.<you>.extraGroups = [ \"kvm\" ]; in your NixOS configuration, nixos-rebuild switch, then log in again. Until then VMs run with TCG, many times slower."
                } else {
                    "Add yourself to the group kvm, then log in again. Until then VMs run with TCG, many times slower."
                },
                (!nixos).then(|| Self::INSTALL_KVM_GROUP.to_string()),
            ),
            KvmState::Missing => Prerequisite::new(
                PrerequisiteItem::Kvm,
                PrerequisiteStatus::Limited,
                "no /dev/kvm – VMs run with TCG (slow)",
                "Turn on virtualization (Intel VT-x or AMD-V) in the firmware settings and load the kvm module (kvm_intel, kvm_amd); in a VM, nested virtualization. Until then VMs run with TCG, many times slower.",
                None,
            ),
        }
    }

    /// The host check from the OS version (`kern.osproductversion`, None when not macOS) and the machine (`uname -m`)
    pub(crate) fn host_item(macos_version: Option<&str>, machine: &str) -> Prerequisite {
        let Some(product) = macos_version else {
            // OPEN (issue #2): Linux hosts
            return Prerequisite::new(
                PrerequisiteItem::Host,
                PrerequisiteStatus::Missing,
                &format!("{} {machine} – not supported yet", Self::os_name()),
                "omaflake runs on macOS with Apple silicon; Linux hosts are not supported yet",
                None,
            );
        };
        let (major, minor, patch) = Self::parse_os_version(product);
        let version = if patch > 0 { format!("{major}.{minor}.{patch}") } else { format!("{major}.{minor}") };
        let arm = machine == "arm64";
        // The app and the CLI need macOS 27 (Package.swift, Info.plist); this check names it for builds run elsewhere
        let ok = arm && major >= 27;
        let hint = if ok {
            ""
        } else if arm {
            "omaflake needs macOS 27 or later"
        } else {
            "omaflake needs a Mac with Apple silicon"
        };
        Prerequisite::new(
            PrerequisiteItem::Host,
            status(ok, PrerequisiteStatus::Missing),
            &format!("macOS {version}, {machine}"),
            hint,
            None,
        )
    }

    /// "27.0.1" → (27, 0, 1); missing or unreadable parts are 0
    pub(crate) fn parse_os_version(text: &str) -> (u32, u32, u32) {
        let mut parts = text.trim().split('.').map(|p| p.parse::<u32>().unwrap_or(0));
        (parts.next().unwrap_or(0), parts.next().unwrap_or(0), parts.next().unwrap_or(0))
    }

    /// The host OS's name for the detail of an unsupported host ("Linux", …)
    fn os_name() -> String {
        match std::env::consts::OS {
            "linux" => "Linux".into(),
            "macos" => "macOS".into(),
            other => other.into(),
        }
    }

    /// macOS' product version (Swift `ProcessInfo.operatingSystemVersion`); None on other systems
    #[cfg(target_os = "macos")]
    fn os_version() -> Option<String> {
        let name = c"kern.osproductversion";
        let mut buf = [0u8; 64];
        let mut len: libc::size_t = buf.len();
        // SAFETY: sysctlbyname writes at most `len` bytes into `buf` and updates `len`; no new value is set
        let rc =
            unsafe { libc::sysctlbyname(name.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) };
        if rc != 0 {
            return Some(String::new());
        }
        let bytes = &buf[..len.min(buf.len())];
        let end = bytes.iter().position(|&b| b == 0).unwrap_or(bytes.len());
        Some(String::from_utf8_lossy(&bytes[..end]).into_owned())
    }

    #[cfg(not(target_os = "macos"))]
    fn os_version() -> Option<String> {
        None
    }

    pub(crate) fn nix_item(&self, nix: Option<&str>) -> Prerequisite {
        let Some(nix) = nix else {
            // Installed, but its volume is not at /nix (after a reboot without /etc/fstab or the mount daemon): mount
            // it, never install Nix again
            if let Some(v) = NixVolume::find().filter(|v| !v.at_nix() && !Path::new("/nix/store").exists()) {
                return Self::nix_not_mounted(&v);
            }
            return Prerequisite::new(
                PrerequisiteItem::Nix,
                PrerequisiteStatus::Missing,
                "not found",
                "Install Nix, or enter the path to the nix binary",
                Some(Self::install_nix()),
            );
        };
        let version = Shell::run(nix, &["--version"], &RunOptions::default())
            .map(|r| r.stdout.trim().to_string())
            .unwrap_or_else(|_| "?".into());
        Prerequisite::new(PrerequisiteItem::Nix, PrerequisiteStatus::Ok, &format!("{version} – {nix}"), "", None)
    }

    /// The start of the Nix item's detail when its volume is here but not at /nix (the Doctor's view offers Mount …)
    pub const NIX_NOT_MOUNTED: &'static str = "Nix Store volume not at /nix";

    pub(crate) fn nix_not_mounted(v: &NixVolume) -> Prerequisite {
        Prerequisite::new(
            PrerequisiteItem::Nix,
            PrerequisiteStatus::Missing,
            &format!("{} ({})", Self::NIX_NOT_MOUNTED, v.state()),
            "Nix is installed, but its volume is not mounted at /nix – mount it (omaflake also restores what mounts it at startup)",
            Some(format!("sudo sh -c {}", shell_quote(&v.repair_command()))),
        )
    }

    pub(crate) fn flakes(&self, nix: Option<&str>, config: &HashMap<String, String>) -> Prerequisite {
        if nix.is_none() {
            return Prerequisite::new(PrerequisiteItem::Flakes, PrerequisiteStatus::Missing, "needs Nix", "", None);
        }
        let ok = Self::flakes_enabled(config);
        let detail = if ok {
            format!("experimental-features: {}", config.get("experimental-features").map(String::as_str).unwrap_or(""))
        } else {
            "flakes not enabled".into()
        };
        Prerequisite::new(
            PrerequisiteItem::Flakes,
            status(ok, PrerequisiteStatus::Missing),
            &detail,
            if ok {
                ""
            } else if cfg!(target_os = "linux") {
                "Add \"nix-command flakes\" to experimental-features (NixOS: nix.settings.experimental-features; else /etc/nix/nix.conf)"
            } else {
                "Add \"nix-command flakes\" to experimental-features (nix-darwin: nix.settings.experimental-features)"
            },
            None,
        )
    }

    pub(crate) fn nix_darwin(&self) -> Prerequisite {
        let rebuild = "/run/current-system/sw/bin/darwin-rebuild";
        let ok = is_executable_file(Path::new(rebuild));
        Prerequisite::new(
            PrerequisiteItem::NixDarwin,
            status(ok, PrerequisiteStatus::Missing),
            if ok { rebuild } else { "not found" },
            if ok { "" } else { "Set up nix-darwin (it provides the Linux builder for NixOS images)" },
            (!ok).then(|| Self::INSTALL_NIX_DARWIN.to_string()),
        )
    }

    pub(crate) fn linux_builder(&self, _nix: Option<&str>, config: &HashMap<String, String>) -> Prerequisite {
        let builders = config.get("builders").map(String::as_str).unwrap_or("");
        let machines = Self::builder_lines(builders, &|f| fs::read_to_string(f).ok());
        let linux = machines.iter().find(|m| Self::is_linux_builder(m));
        let detail = match linux {
            Some(line) => line.split(' ').find(|f| !f.is_empty()).unwrap_or("").to_string(),
            None => "no aarch64-linux builder".into(),
        };
        Prerequisite::new(
            PrerequisiteItem::LinuxBuilder,
            status(linux.is_some(), PrerequisiteStatus::Missing),
            &detail,
            if linux.is_some() {
                ""
            } else {
                "Enable nix.linux-builder in your nix-darwin configuration (docs/linux-builder.md)"
            },
            None,
        )
    }

    pub(crate) fn qemu(&self) -> Prerequisite {
        if cfg!(target_os = "linux") {
            return self.linux_qemu();
        }
        let Ok((rt, _)) = QemuRuntime::locate(&self.paths) else {
            return Prerequisite::new(
                PrerequisiteItem::Qemu,
                PrerequisiteStatus::Missing,
                "not found",
                "Needed for NixOS guests: install omaflake's QEMU, or enter a path",
                Some(Self::INSTALL_QEMU.into()),
            );
        };
        let gl = rt.capabilities().gl;
        Prerequisite::new(
            PrerequisiteItem::Qemu,
            status(gl, PrerequisiteStatus::Limited),
            &format!("{}{}", rt.qemu, if gl { " (3D)" } else { " (no 3D)" }),
            if gl {
                ""
            } else {
                "Works headless and in 2D – without 3D and without automatic window resolution; omaflake's QEMU adds both"
            },
            (!gl).then(|| Self::INSTALL_QEMU.to_string()),
        )
    }

    /// Linux (plan B124): `qemu-system-<arch>` with its version, 3D (virtio-gpu-gl) and the UEFI firmware it found
    fn linux_qemu(&self) -> Prerequisite {
        let binary = QemuRuntime::host_arch().qemu_binary();
        let rt = match QemuRuntime::locate(&self.paths) {
            Ok((rt, _)) => rt,
            Err(e) => {
                let firmware = e.code() == Some(crate::error::codes::PREREQUISITE);
                return Prerequisite::new(
                    PrerequisiteItem::Qemu,
                    PrerequisiteStatus::Missing,
                    &if firmware { e.message().to_string() } else { format!("{binary} not found") },
                    if firmware {
                        "No UEFI firmware for this QEMU: install nixpkgs' QEMU (it brings edk2), or your distribution's OVMF (x86_64) / AAVMF (aarch64) package"
                    } else {
                        "Needed for NixOS guests: install QEMU, or enter the path to qemu-system-*"
                    },
                    Some(Self::INSTALL_QEMU_LINUX.into()),
                );
            }
        };
        let version = Shell::run(&rt.qemu, &["--version"], &RunOptions::default())
            .ok()
            .and_then(|r| r.stdout.lines().next().map(|l| l.replace("QEMU emulator version ", "QEMU ")))
            .unwrap_or_else(|| "QEMU ?".into());
        let gl = rt.capabilities().gl;
        Prerequisite::new(
            PrerequisiteItem::Qemu,
            status(gl, PrerequisiteStatus::Limited),
            &format!("{} ({version}, {}) – UEFI {}", rt.qemu, if gl { "3D" } else { "no 3D" }, rt.firmware),
            if gl {
                ""
            } else {
                "Works headless and in 2D – without 3D; a QEMU with virglrenderer (nixpkgs#qemu) adds it"
            },
            (!gl).then(|| Self::INSTALL_QEMU_LINUX.to_string()),
        )
    }

    pub(crate) fn flake(&self, settings: &Settings, _nix: Option<&str>) -> Prerequisite {
        let Ok(flake) = settings.flake_path() else {
            return Prerequisite::new(
                PrerequisiteItem::Flake,
                PrerequisiteStatus::Missing,
                "not set",
                "Choose the flake with your recipes (folder or flake URL)",
                None,
            );
        };
        let is_folder = flake.starts_with('/');
        let exists = !is_folder || Path::new(&flake).join("flake.nix").exists();
        Prerequisite::new(
            PrerequisiteItem::Flake,
            status(exists, PrerequisiteStatus::Missing),
            &flake,
            if exists { "" } else { "No flake.nix in this folder" },
            None,
        )
    }

    pub(crate) fn ssh_key(&self) -> Prerequisite {
        let keys = MacBaseInstaller::default_public_keys();
        let files: Vec<String> = sorted_names(&home_dir().join(".ssh"))
            .into_iter()
            .filter(|n| n.ends_with(".pub"))
            .map(|n| format!("~/.ssh/{n}"))
            .collect();
        let none = keys.is_empty();
        Prerequisite::new(
            PrerequisiteItem::SshKey,
            status(!none, PrerequisiteStatus::Missing),
            &if none { "no public key in ~/.ssh".to_string() } else { files.join(", ") },
            if none { "Create a key (used for omaf ssh, image builds and macOS bases)" } else { "" },
            none.then(|| Self::INSTALL_SSH_KEY.to_string()),
        )
    }

    /// A credential (plan B139): images and macOS bases are made with one – the default unless another is chosen
    pub(crate) fn credential(&self) -> Prerequisite {
        let found = crate::credentials::Credentials::new(self.paths.clone()).default_credential();
        Prerequisite::new(
            PrerequisiteItem::Credential,
            status(found.is_some(), PrerequisiteStatus::Missing),
            &match &found {
                Some(c) => format!("{} ({}, default)", c.name, c.user.as_deref().unwrap_or("–")),
                None => "none yet".to_string(),
            },
            if found.is_some() { "" } else { "Create a credential – who you are in the guests, with a password" },
            None,
        )
    }

    /// An installed look (plan B141): images are built with one – the default unless another is chosen
    pub(crate) fn look(&self) -> Prerequisite {
        let found = crate::looks::LookStore::new(self.paths.clone()).default_look(&Settings::load(&self.paths));
        Prerequisite::new(
            PrerequisiteItem::Look,
            status(found.is_some(), PrerequisiteStatus::Missing),
            &match &found {
                Some(l) => format!("{} (default)", l.title),
                None => "none installed".to_string(),
            },
            if found.is_some() { "" } else { "Install a look – omaflake's own and Catppuccin are recommended" },
            None,
        )
    }

    pub(crate) fn git(&self) -> Prerequisite {
        let git = Self::locate_git();
        Prerequisite::new(
            PrerequisiteItem::Git,
            status(git.is_some(), PrerequisiteStatus::Limited),
            git.as_deref().unwrap_or("not found"),
            if git.is_some() {
                ""
            } else {
                "Optional: image names then show \"nogit\" instead of the flake's revision"
            },
            git.is_none().then(|| {
                if cfg!(target_os = "linux") { Self::INSTALL_GIT_LINUX } else { Self::INSTALL_GIT }.to_string()
            }),
        )
    }

    // MARK: Preflight (before building images)

    pub fn nixos_preflight(&self) -> Vec<Prerequisite> {
        let settings = Settings::load(&self.paths);
        if cfg!(target_os = "linux") {
            // Images build here, natively – no Linux builder (plan B124)
            return vec![
                self.flake_eval(&settings),
                self.space(PrerequisiteItem::NixSpace, Path::new("/nix/store"), Self::NIX_SPACE_NEEDED),
            ];
        }
        vec![
            self.builder_running(),
            self.flake_eval(&settings),
            self.space(PrerequisiteItem::NixSpace, Path::new("/nix/store"), Self::NIX_SPACE_NEEDED),
        ]
    }

    /// `latest`: what Apple offers this Mac (`MacOsRelease::latest`), None when unknown (offline).
    pub fn macos_preflight(&self, latest: Option<&MacOsRelease>) -> Vec<Prerequisite> {
        if !cfg!(target_os = "macos") {
            // macOS guests need a Mac (plan B124)
            return Vec::new();
        }
        let ipsw_item = self.ipsw(latest);
        let missing_ipsw = if ipsw_item.status != PrerequisiteStatus::Ok {
            latest.and_then(|l| l.bytes).unwrap_or(27_000_000_000)
        } else {
            0
        };
        vec![
            ipsw_item,
            self.mac_base(),
            self.space(PrerequisiteItem::MacSpace, &self.paths.root, Self::MAC_SPACE_NEEDED + missing_ipsw),
        ]
    }

    pub fn builder_running(&self) -> Prerequisite {
        let target = Self::builder_ssh_target(None);
        let up = target.as_ref().is_some_and(|(host, port)| {
            let host = if host == "localhost" { "127.0.0.1" } else { host.as_str() };
            Ports::ssh_banner(host, *port, DEFAULT_TIMEOUT).is_some()
        });
        let detail = match &target {
            Some((host, port)) => format!("{host}:{port}"),
            None => "no linux-builder in /etc/ssh/ssh_config.d".into(),
        };
        Prerequisite::new(
            PrerequisiteItem::BuilderRunning,
            status(up, PrerequisiteStatus::Missing),
            &detail,
            if up {
                ""
            } else {
                "The Linux builder VM does not answer – it starts with nix-darwin (launchd org.nixos.linux-builder)"
            },
            None,
        )
    }

    pub(crate) fn flake_eval(&self, settings: &Settings) -> Prerequisite {
        let Ok(flake) = settings.flake_path() else {
            return Prerequisite::new(
                PrerequisiteItem::FlakeEval,
                PrerequisiteStatus::Missing,
                "no flake set",
                "",
                None,
            );
        };
        match ImageStore::new(self.paths.clone()).evaluate_recipe_defaults(&flake) {
            Ok(recipes) => {
                let mut nixos: Vec<String> = recipes
                    .iter()
                    .filter(|(_, d)| d.kind == GuestKind::Nixos)
                    .map(|(id, d)| RecipeId::display_name(id, Some(d)))
                    .collect();
                nixos.sort();
                Prerequisite::new(
                    PrerequisiteItem::FlakeEval,
                    PrerequisiteStatus::Ok,
                    &format!("{} recipes ({})", recipes.len(), nixos.join(", ")),
                    "",
                    None,
                )
            }
            Err(e) => Prerequisite::new(
                PrerequisiteItem::FlakeEval,
                PrerequisiteStatus::Missing,
                &e.to_string().chars().take(300).collect::<String>(),
                "The flake does not evaluate – see the message",
                None,
            ),
        }
    }

    pub(crate) fn ipsw(&self, latest: Option<&MacOsRelease>) -> Prerequisite {
        let dir = self.paths.ipsw();
        let local: Vec<String> = sorted_names(&dir).into_iter().filter(|n| n.ends_with(".ipsw")).collect();
        let Some(latest) = latest else {
            return Prerequisite::new(
                PrerequisiteItem::Ipsw,
                if local.is_empty() { PrerequisiteStatus::Missing } else { PrerequisiteStatus::Ok },
                local.last().map(String::as_str).unwrap_or("Apple not reachable"),
                "",
                None,
            );
        };
        let file = IpswDownloader::file(latest, &dir);
        if file.exists() {
            return Prerequisite::new(
                PrerequisiteItem::Ipsw,
                PrerequisiteStatus::Ok,
                &format!("macOS {} ({}) – {}", latest.version, latest.build, last_component(&file)),
                "",
                None,
            );
        }
        let partial = IpswDownloader::resume_file_for(latest, &dir).exists();
        Prerequisite::new(
            PrerequisiteItem::Ipsw,
            if partial { PrerequisiteStatus::Limited } else { PrerequisiteStatus::Missing },
            &format!(
                "macOS {} ({}), {} from Apple",
                latest.version,
                latest.build,
                latest.bytes.map(gigabytes).unwrap_or_else(|| "?".into())
            ),
            if partial { "Partly downloaded – continues where it stopped" } else { "Download it from Apple" },
            None,
        )
    }

    pub(crate) fn mac_base(&self) -> Prerequisite {
        let base = MacImageBuilder::newest_base_in(&self.paths);
        let bases = self.paths.bases();
        let unfinished = sorted_names(&bases)
            .into_iter()
            .filter(|n| n.starts_with("mac-base-"))
            .find(|n| !MacBaseInstaller::is_ready(&bases.join(n)));
        if let (None, Some(unfinished)) = (&base, &unfinished) {
            return Prerequisite::new(
                PrerequisiteItem::MacBase,
                PrerequisiteStatus::Limited,
                &format!("{unfinished} (unfinished)"),
                "Unfinished – Create Base continues it (same password as before)",
                None,
            );
        }
        if let Some(base) = &base {
            if let Some(user) = MacBaseInstaller::user_of(base, &self.paths.logs()) {
                // The macOS recipes' guest user (macUser) must be the base's: builds log in as it, nix-darwin
                // configures it
                let expected: BTreeSet<String> = ImageStore::new(self.paths.clone())
                    .recipe_defaults()
                    .into_values()
                    .filter(|d| d.kind == GuestKind::Macos)
                    .filter_map(|d| d.user)
                    .collect();
                let name = last_component(base);
                if let Some(other) = expected.iter().find(|u| **u != user) {
                    let mut p = Prerequisite::new(
                        PrerequisiteItem::MacBase,
                        PrerequisiteStatus::Limited,
                        &format!("{name} ({user})"),
                        &format!("The base's user {user} differs from the recipes' macUser {other} – make them match"),
                        None,
                    );
                    p.expected_user = Some(other.clone());
                    p.base_user = Some(user);
                    return p;
                }
                let mut p = Prerequisite::new(
                    PrerequisiteItem::MacBase,
                    PrerequisiteStatus::Ok,
                    &format!("{name} ({user})"),
                    "",
                    None,
                );
                p.base_user = Some(user);
                return p;
            }
        }
        Prerequisite::new(
            PrerequisiteItem::MacBase,
            status(base.is_some(), PrerequisiteStatus::Missing),
            &base.as_deref().map(last_component).unwrap_or_else(|| "none yet".into()),
            if base.is_some() { "" } else { "Create it from the IPSW (about 5 minutes, no manual steps)" },
            None,
        )
    }

    pub(crate) fn space(&self, item: PrerequisiteItem, at: &Path, needed: i64) -> Prerequisite {
        let free = free_space(at).unwrap_or(0);
        let enough = free >= needed;
        let mut p = Prerequisite::new(
            item,
            status(enough, PrerequisiteStatus::Missing),
            &format!("{} free, about {} needed", gigabytes(free), gigabytes(needed)),
            if enough { "" } else { "Free up space first" },
            None,
        );
        p.free_bytes = Some(free);
        p.needed_bytes = Some(needed);
        p
    }

    /// Host and port of the Linux builder from the SSH configuration nix-darwin writes (`Host linux-builder`);
    /// its key is root's, so only the SSH banner is checked. `configs` None = /etc/ssh/ssh_config.d/* and
    /// /etc/ssh/ssh_config.
    pub(crate) fn builder_ssh_target(configs: Option<&[String]>) -> Option<(String, u16)> {
        let read: Vec<String>;
        let texts: &[String] = match configs {
            Some(c) => c,
            None => {
                let dir = Path::new("/etc/ssh/ssh_config.d");
                let mut files: Vec<_> = sorted_names(dir).into_iter().map(|n| dir.join(n)).collect();
                files.push("/etc/ssh/ssh_config".into());
                read = files.iter().filter_map(|f| fs::read_to_string(f).ok()).collect();
                &read
            }
        };
        for text in texts {
            let mut in_block = false;
            let mut host: Option<String> = None;
            let mut port: u16 = 22;
            for raw in text.split('\n').filter(|l| !l.is_empty()) {
                // "<keyword> <value>": split at the first space, further spaces before the value are skipped
                let Some((key, value)) = trim_spaces(raw).split_once(' ') else { continue };
                let value = value.trim_start_matches(' ');
                if key.is_empty() || value.is_empty() {
                    continue;
                }
                match key.to_lowercase().as_str() {
                    "host" => {
                        if in_block {
                            return Some((host.unwrap_or_else(|| "linux-builder".into()), port));
                        }
                        in_block = value.split(' ').any(|h| h == "linux-builder");
                    }
                    "hostname" if in_block => host = Some(value.to_string()),
                    "port" if in_block => port = value.parse().unwrap_or(22),
                    _ => {}
                }
            }
            if in_block {
                return Some((host.unwrap_or_else(|| "linux-builder".into()), port));
            }
        }
        None
    }

    // MARK: Detection helpers

    /// `nix`: `Settings.nix` (must be executable), then PATH and the usual Nix locations (`Shell::which`).
    pub fn locate_nix(settings: &Settings) -> Option<String> {
        if let Some(custom) = settings.nix.as_deref().map(expand_tilde) {
            return is_executable_file(Path::new(&custom)).then_some(custom);
        }
        Shell::which("nix").ok()
    }

    /// git that does not trigger the Command Line Tools install dialog: /usr/bin/git is only a shim that asks
    /// to install them when they are missing, so it counts only when `xcode-select -p` finds developer tools.
    pub fn locate_git() -> Option<String> {
        let found = Shell::search_directories()
            .into_iter()
            .filter(|d| d != "/usr/bin")
            .map(|d| Path::new(&d).join("git"))
            .find(|p| is_executable_file(p));
        if let Some(git) = found {
            return Some(git.to_string_lossy().into_owned());
        }
        let tools = Shell::run("/usr/bin/xcode-select", &["-p"], &RunOptions::default());
        if tools.map(|r| r.status) != Ok(0) {
            return None;
        }
        Some("/usr/bin/git".into())
    }

    /// `nix config show` as key → value
    pub(crate) fn nix_config(nix: &str) -> HashMap<String, String> {
        let out = Shell::run(nix, &["config", "show"], &RunOptions::default()).map(|r| r.stdout).unwrap_or_default();
        Self::parse_config(&out)
    }

    pub(crate) fn parse_config(text: &str) -> HashMap<String, String> {
        text.split('\n')
            .filter_map(|line| line.split_once('='))
            .map(|(key, value)| (trim_spaces(key).to_string(), trim_spaces(value).to_string()))
            .collect()
    }

    pub(crate) fn flakes_enabled(config: &HashMap<String, String>) -> bool {
        config.get("experimental-features").is_some_and(|f| f.split(' ').any(|x| x == "flakes"))
    }

    /// `builders` is a list separated by ";" or newlines, or "@<file>" with one machine per line (`read` reads it;
    /// Swift default: the file's contents)
    pub(crate) fn builder_lines(builders: &str, read: &dyn Fn(&str) -> Option<String>) -> Vec<String> {
        let text = match builders.strip_prefix('@') {
            Some(file) => read(file).unwrap_or_default(),
            None => builders.to_string(),
        };
        text.split(['\n', ';'])
            .map(|l| trim_spaces(l).to_string())
            .filter(|l| !l.is_empty() && !l.starts_with('#'))
            .collect()
    }

    /// Machine line: `<uri> <systems, comma-separated> …` with aarch64-linux among the systems
    pub(crate) fn is_linux_builder(line: &str) -> bool {
        let fields: Vec<&str> = line.split(' ').filter(|f| !f.is_empty()).collect();
        fields.len() > 1 && fields[1].split(',').any(|s| s == "aarch64-linux")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;

    // `nix config show` excerpt and /etc/nix/machines as written by nix-darwin's linux-builder (2026-10-16)
    const CONFIG: &str =
        "builders = @/etc/nix/machines\nexperimental-features = fetch-tree flakes nix-command\nmax-jobs = auto\n";
    const MACHINES: &str =
        "ssh-ng://builder@linux-builder aarch64-linux /etc/nix/builder_ed25519 6 1 kvm,benchmark,big-parallel - c3No\n";

    fn no_file(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn parses_config_and_flakes() {
        let c = Prerequisites::parse_config(CONFIG);
        assert_eq!(c["builders"], "@/etc/nix/machines");
        assert!(Prerequisites::flakes_enabled(&c));
        let only = HashMap::from([("experimental-features".to_string(), "nix-command".to_string())]);
        assert!(!Prerequisites::flakes_enabled(&only));
        assert!(!Prerequisites::flakes_enabled(&HashMap::new()));
    }

    #[test]
    fn finds_linux_builder_in_machines_file() {
        let lines = Prerequisites::builder_lines("@/etc/nix/machines", &|f| {
            (f == "/etc/nix/machines").then(|| MACHINES.to_string())
        });
        assert!(lines.len() == 1 && Prerequisites::is_linux_builder(&lines[0]));
        assert!(!Prerequisites::is_linux_builder("ssh://mac x86_64-darwin"));
        assert!(Prerequisites::builder_lines("", &no_file).is_empty());
        // inline list, separated by ";"
        assert_eq!(Prerequisites::builder_lines("ssh://a aarch64-linux ; ssh://b x86_64-linux", &no_file).len(), 2);
        // comments and blank lines are left out
        assert_eq!(
            Prerequisites::builder_lines("# x\n\n  ssh://a aarch64-linux\n", &no_file),
            ["ssh://a aarch64-linux"]
        );
    }

    #[test]
    fn linux_builder_item_names_the_machine() {
        let (_d, p) = temp_paths();
        let pre = Prerequisites::new(p);
        let config = HashMap::from([("builders".to_string(), "ssh://a x86_64-linux; ssh-ng://b aarch64-linux".into())]);
        let item = pre.linux_builder(Some("/x/nix"), &config);
        assert_eq!((item.status, item.detail.as_str()), (PrerequisiteStatus::Ok, "ssh-ng://b"));
        let none = pre.linux_builder(None, &HashMap::new());
        assert_eq!((none.status, none.detail.as_str()), (PrerequisiteStatus::Missing, "no aarch64-linux builder"));
    }

    #[test]
    fn flakes_item() {
        let (_d, p) = temp_paths();
        let pre = Prerequisites::new(p);
        assert_eq!(pre.flakes(None, &HashMap::new()).detail, "needs Nix");
        let ok = pre.flakes(Some("/x/nix"), &Prerequisites::parse_config(CONFIG));
        assert_eq!(ok.status, PrerequisiteStatus::Ok);
        assert_eq!(ok.detail, "experimental-features: fetch-tree flakes nix-command");
        assert_eq!(pre.flakes(Some("/x/nix"), &HashMap::new()).status, PrerequisiteStatus::Missing);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn host_is_apple_silicon() {
        assert_eq!(machine(), "arm64");
        // a real Mac's version: at least "<major>.<minor>"
        let v = Prerequisites::os_version().unwrap();
        assert!(Prerequisites::parse_os_version(&v).0 >= 11, "{v}");
    }

    #[test]
    fn host_item_from_version_and_machine() {
        let ok = Prerequisites::host_item(Some("27.0.1"), "arm64");
        assert_eq!(
            (ok.status, ok.detail.as_str(), ok.hint.as_str()),
            (PrerequisiteStatus::Ok, "macOS 27.0.1, arm64", "")
        );
        assert_eq!(Prerequisites::host_item(Some("27.0"), "arm64").detail, "macOS 27.0, arm64");
        assert_eq!(Prerequisites::host_item(Some("27"), "arm64").detail, "macOS 27.0, arm64");
        let old = Prerequisites::host_item(Some("26.4"), "arm64");
        assert_eq!((old.status, old.hint.as_str()), (PrerequisiteStatus::Missing, "omaflake needs macOS 27 or later"));
        let intel = Prerequisites::host_item(Some("27.0"), "x86_64");
        assert_eq!(intel.hint, "omaflake needs a Mac with Apple silicon");
        // OPEN (issue #2): Linux hosts
        let linux = Prerequisites::host_item(None, "aarch64");
        assert_eq!(linux.status, PrerequisiteStatus::Missing);
        assert!(linux.detail.contains("not supported yet"));
    }

    #[test]
    fn linux_host_and_kvm_items() {
        let x86 = Prerequisites::linux_host_item("6.12.1", "x86_64");
        assert_eq!((x86.status, x86.detail.as_str()), (PrerequisiteStatus::Ok, "Linux 6.12.1, x86_64"));
        assert_eq!(Prerequisites::linux_host_item("6.12.1", "aarch64").status, PrerequisiteStatus::Ok);
        assert_eq!(Prerequisites::linux_host_item("6.12.1", "riscv64").status, PrerequisiteStatus::Missing);

        let ok = Prerequisites::kvm_item(KvmState::Usable, true);
        assert_eq!((ok.item.as_str(), ok.status, ok.install.as_deref()), ("kvm", PrerequisiteStatus::Ok, None));
        // TCG still runs VMs: limited, not missing – nothing is blocked
        let nixos = Prerequisites::kvm_item(KvmState::NoAccess, true);
        assert_eq!(nixos.status, PrerequisiteStatus::Limited);
        assert!(nixos.hint.contains("extraGroups") && nixos.install.is_none());
        let debian = Prerequisites::kvm_item(KvmState::NoAccess, false);
        assert_eq!(debian.install.as_deref(), Some(Prerequisites::INSTALL_KVM_GROUP));
        let missing = Prerequisites::kvm_item(KvmState::Missing, false);
        assert_eq!(
            (missing.status, missing.detail.as_str()),
            (PrerequisiteStatus::Limited, "no /dev/kvm – VMs run with TCG (slow)")
        );
        assert!(missing.required());
    }

    /// What each host checks (plan B124): no nix-darwin, Linux builder or macOS items on Linux; KVM only there
    #[test]
    fn items_per_host() {
        let (_d, p) = temp_paths();
        let pre = Prerequisites::new(p);
        let items: Vec<PrerequisiteItem> = pre.check_without_cli().iter().map(|p| p.item).collect();
        let nixos: Vec<PrerequisiteItem> = pre.nixos_preflight().iter().map(|p| p.item).collect();
        use PrerequisiteItem::*;
        if cfg!(target_os = "linux") {
            assert_eq!(items, [Host, Kvm, Nix, Flakes, Qemu, Flake, SshKey, Credential, Look, Git]);
            assert_eq!(nixos, [FlakeEval, NixSpace]);
            assert!(pre.macos_preflight(None).is_empty());
        } else {
            assert_eq!(items, [Host, Nix, Flakes, NixDarwin, LinuxBuilder, Qemu, Flake, SshKey, Credential, Look, Git]);
            assert_eq!(nixos, [BuilderRunning, FlakeEval, NixSpace]);
            assert_eq!(pre.macos_preflight(None).len(), 3);
        }
    }

    #[test]
    fn missing_flake_and_custom_nix_path() {
        let (_d, p) = temp_paths();
        let pre = Prerequisites::new(p);
        let flake = pre.flake(&Settings::default(), None);
        assert!(
            flake.status == PrerequisiteStatus::Missing
                || std::env::var("VM_FLAKE").is_ok()
                || Settings::bundled_flake().is_some()
        );
        let s = Settings { nix: Some("/does/not/exist/nix".into()), ..Default::default() };
        assert_eq!(Prerequisites::locate_nix(&s), None);
        assert_eq!(pre.nix_item(None).install, Some(Prerequisites::install_nix()));
    }

    #[test]
    fn flake_folder_needs_flake_nix() {
        let (d, p) = temp_paths();
        let pre = Prerequisites::new(p);
        if std::env::var("VM_FLAKE").is_ok() {
            return;
        }
        let s = Settings { flake: Some(d.path().to_string_lossy().into_owned()), ..Default::default() };
        assert_eq!(pre.flake(&s, None).hint, "No flake.nix in this folder");
        fs::write(d.path().join("flake.nix"), "{}").unwrap();
        assert_eq!(pre.flake(&s, None).status, PrerequisiteStatus::Ok);
        let url = Settings { flake: Some("github:omaflake/omaflake?dir=nix".into()), ..Default::default() };
        assert_eq!(pre.flake(&url, None).status, PrerequisiteStatus::Ok);
    }

    #[test]
    fn install_nix_wraps_the_pinned_installer() {
        let cmd = Prerequisites::install_nix();
        if cfg!(target_os = "linux") {
            assert_eq!(cmd, Prerequisites::INSTALL_NIX_LINUX);
            return;
        }
        assert!(cmd.starts_with("sudo sh -c 'set -e;"));
        assert!(cmd.ends_with("install --no-confirm'"));
    }

    #[test]
    fn finds_builder_in_ssh_config() {
        let conf = "Host other\n  Port 2\nHost linux-builder\n  User builder\n  Hostname localhost\n  HostKeyAlias linux-builder\n  Port 31022\n";
        let t = Prerequisites::builder_ssh_target(Some(&[conf.to_string()]));
        assert_eq!(t, Some(("localhost".to_string(), 31022)));
        assert_eq!(Prerequisites::builder_ssh_target(Some(&["Host x\n  Port 1\n".to_string()])), None);
        // the block ends at the next Host; without Hostname the alias is the host
        let two = "Host a linux-builder\n\tPort 2222\nHost b\n  Port 1\n";
        assert_eq!(Prerequisites::builder_ssh_target(Some(&[two.to_string()])), Some(("linux-builder".into(), 2222)));
    }

    #[test]
    fn ipsw_item_without_and_with_a_release() {
        let (_d, p) = temp_paths();
        let pre = Prerequisites::new(p.clone());
        let offline = pre.ipsw(None);
        assert_eq!((offline.status, offline.detail.as_str()), (PrerequisiteStatus::Missing, "Apple not reachable"));
        let release = MacOsRelease {
            version: "27.0.1".into(),
            build: "26A434".into(),
            url: "https://updates.cdn-apple.com/x.ipsw".into(),
            bytes: Some(26_600_000_000),
        };
        let missing = pre.ipsw(Some(&release));
        assert_eq!(missing.status, PrerequisiteStatus::Missing);
        assert_eq!(missing.detail, "macOS 27.0.1 (26A434), 26.6 GB from Apple");
        fs::create_dir_all(p.ipsw()).unwrap();
        fs::write(IpswDownloader::resume_file_for(&release, &p.ipsw()), "").unwrap();
        assert_eq!(pre.ipsw(Some(&release)).status, PrerequisiteStatus::Limited);
        fs::write(IpswDownloader::file(&release, &p.ipsw()), "").unwrap();
        let ok = pre.ipsw(Some(&release));
        assert_eq!((ok.status, ok.detail.as_str()), (PrerequisiteStatus::Ok, "macOS 27.0.1 (26A434) – 26A434.ipsw"));
        let local = pre.ipsw(None);
        assert_eq!((local.status, local.detail.as_str()), (PrerequisiteStatus::Ok, "26A434.ipsw"));
    }

    #[test]
    fn free_space_falls_back_for_the_nix_store() {
        // the read-only store volume reports 0 for "important usage" – the plain value is used then
        assert!(free_space(Path::new("/nix/store")).unwrap_or(0) > 0 || !Path::new("/nix/store").exists());
    }

    #[test]
    fn serializes_as_the_api_type() {
        let mut p = Prerequisite::new(
            PrerequisiteItem::NixDarwin,
            PrerequisiteStatus::Missing,
            "not found",
            "Set up",
            Some("omaf builder setup".into()),
        );
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(
            v,
            serde_json::json!({"item": "nixDarwin", "status": "missing", "required": true, "detail": "not found",
                               "hint": "Set up", "install": "omaf builder setup", "settablePath": false})
        );
        p.item = PrerequisiteItem::Git;
        p.status = PrerequisiteStatus::Limited;
        p.install = None;
        p.free_bytes = Some(1);
        p.base_user = Some("u".into());
        let v = serde_json::to_value(&p).unwrap();
        assert_eq!(v["required"], false);
        assert_eq!(v["status"], "limited");
        assert_eq!(v["freeBytes"], 1);
        assert_eq!(v["baseUser"], "u");
        assert!(v.get("install").is_none() && v.get("neededBytes").is_none());
        let back: Prerequisite = serde_json::from_value(v).unwrap();
        assert_eq!(back, p);
        let nix = Prerequisite::new(PrerequisiteItem::Nix, PrerequisiteStatus::Ok, "", "", None);
        assert_eq!(serde_json::to_value(&nix).unwrap()["settablePath"], true);
    }
}
