// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Doctor (`doctor`, spec §11.2): versions and updates, the checks in three sections with their hints and what can
//! be done about each (the app's `DoctorView`, `RequirementRow`, `RequirementText`, `UpdateRow`); and the page that
//! creates the macOS base (`macBase`, the app's `MacBaseView`).

use serde_json::{Value, json};

use super::Ctx;
use super::common::*;
use super::text::{Arg, Text, tf};
use crate::api::types::{CliLinkDto, JobDto, JobKind, JobState, PrerequisiteDto, UpdateState};
use crate::prerequisites::{PrerequisiteItem as I, PrerequisiteStatus as S, Prerequisites};

// The page's model (plan B192: in omaflake-sdk); this file builds it
pub use omaflake_sdk::views::doctor::*;

/// The hint for a check; the base/recipe user mismatch names both users (`RequirementText.hint`); this host's texts
pub fn hint(p: &PrerequisiteDto) -> Text {
    hint_on(p, super::HostOs::this())
}

/// The hint for a check on `host` – a Linux host has its own texts for the host, KVM, flakes and QEMU (plan B124)
pub fn hint_on(p: &PrerequisiteDto, host: super::HostOs) -> Text {
    if let (I::MacBase, Some(base), Some(expected)) = (p.item, &p.base_user, &p.expected_user) {
        return tf!(
            "The base's user %@ differs from macUser %@ in your image flake's settings – set macUser to %@ in nix/local/settings.nix, or create the base again with %@.",
            base,
            expected,
            base,
            expected
        );
    }
    let limited = p.status == S::Limited;
    let linux = host == super::HostOs::Linux;
    Text::new(match p.item {
        I::Host if linux => "omaflake runs on Linux with x86_64 or aarch64.",
        I::Kvm if p.detail.starts_with("no /dev/kvm") => {
            "Turn on virtualization (Intel VT-x or AMD-V) in the firmware settings and load the kvm module; in a VM, nested virtualization. Until then VMs run with TCG, many times slower."
        }
        I::Kvm if p.install.is_some() => {
            "Add yourself to the group kvm, then log in again. Until then VMs run with TCG, many times slower."
        }
        I::Kvm => {
            "Add yourself to the group kvm in your NixOS configuration (users.users.<you>.extraGroups = [ \"kvm\" ];), rebuild, then log in again. Until then VMs run with TCG, many times slower."
        }
        I::Flakes if linux => {
            "Add “nix-command flakes” to experimental-features (NixOS: nix.settings.experimental-features; else /etc/nix/nix.conf)."
        }
        I::Qemu if linux && limited => {
            "Works headless and in 2D – without 3D; a QEMU with virglrenderer (nixpkgs#qemu) adds it."
        }
        I::Qemu if linux && p.detail.starts_with("UEFI firmware not found") => {
            "No UEFI firmware for this QEMU: install nixpkgs' QEMU (it brings edk2), or your distribution's OVMF (x86_64) or AAVMF (aarch64) package."
        }
        I::Qemu if linux => "Needed for NixOS guests: install QEMU, or enter the path to qemu-system-*.",
        I::Host if limited => "macOS 27 is needed for automatic macOS bases.",
        I::Host => "omaflake needs a Mac with Apple silicon.",
        I::Nix if p.detail.starts_with(Prerequisites::NIX_NOT_MOUNTED) => {
            "Nix is installed, but its volume is not mounted at /nix – usually after a restart without its mount settings. Mount it; NixOS images, the Linux builder and omaflake's QEMU are back then."
        }
        I::Nix if p.detail == Prerequisites::NIX_HALF => {
            "A Nix install stopped half way. Remove it, restart the Mac if /nix stays busy, then install again."
        }
        I::Nix => {
            "Install Nix, or enter the path to the nix binary. omaflake's install asks for an administrator once."
        }
        I::Flakes => {
            "Add “nix-command flakes” to experimental-features in /etc/nix/nix.conf – omaflake's Nix install does it (nix-darwin: nix.settings.experimental-features)."
        }
        I::Shell if p.detail == "needs Nix" => "Install Nix first.",
        I::Shell => {
            "New terminals do not find nix. omaflake adds the lines that load Nix to your ~/.zshrc – nothing in /etc. omaflake itself does not need them."
        }
        I::NixDarwin => "Set up nix-darwin – it provides the Linux builder for NixOS images.",
        I::LinuxBuilder if p.hint == "needs Nix" => "Install Nix first.",
        I::LinuxBuilder if p.hint.contains("trusted-users") => {
            "omaflake's Linux builder needs you among Nix's trusted-users. omaflake's Nix install sets that; for another Nix, Allow … asks for an administrator once."
        }
        I::LinuxBuilder => {
            "Set up omaflake's Linux builder – it runs as you in the background, no administrator needed. The first time it downloads about 430 MB and builds omaflake's builder: several minutes."
        }
        I::Qemu if limited && p.detail.contains(Prerequisites::QEMU_OLD_PATCHSET) => {
            "An older omaflake QEMU: NixOS VMs start without the loading screen – update it (builds QEMU, about ten minutes)."
        }
        I::Qemu if limited => {
            "Works headless and in 2D – without 3D and without automatic window resolution; omaflake's QEMU adds both."
        }
        I::Qemu => "Needed for NixOS guests: install omaflake's QEMU, or enter a path.",
        I::Flake => "Choose the folder with your recipes (flake.nix).",
        I::SshKey => {
            "omaflake creates its own key for its VMs – no passphrase, only for omaflake. Your keys in ~/.ssh go into the VMs too."
        }
        I::Look => {
            "Install a look – how the guests look: colours, terminal, editors. omaflake's own and Catppuccin are recommended; more are in the Looks tab."
        }
        I::Credential => {
            "Create a credential – who you are in the guests: user, full name and a password, asked once. omaflake builds images and macOS bases with it by itself."
        }
        I::Git => "Optional: without it, image names show “nogit” instead of the flake's revision.",
        I::Cli if limited => {
            "Another omaf comes first on your PATH – install this omaflake's, or remove the other one."
        }
        I::Cli => "Optional: install the omaf command to use omaflake in a terminal.",
        I::BuilderRunning if p.detail.starts_with("no hypervisor") => {
            "This Mac cannot run VMs – it is probably itself a VM without nested virtualization. The Linux builder and NixOS guests need a Mac that can."
        }
        I::BuilderRunning if p.detail == "no Linux builder set up" => "Set up the Linux builder first.",
        I::BuilderRunning if p.hint.contains("LaunchAgent") => {
            "omaflake's Linux builder does not answer. It starts in the background when you log in; its first start takes a few minutes."
        }
        I::BuilderRunning => {
            "The Linux builder VM does not answer – it is started by nix-darwin (launchd: org.nixos.linux-builder)."
        }
        I::FlakeEval => "Your recipes do not evaluate – see the message.",
        I::NixSpace | I::MacSpace => "Not enough free space – free some up first.",
        I::Ipsw if p.hint == Prerequisites::IPSW_NEWER => {
            "A newer macOS is available from Apple. Your bases and the IPSW you have keep working; download it for new bases with the newer macOS."
        }
        I::Ipsw if limited => "Partly downloaded – continues where it stopped.",
        I::Ipsw => "Download macOS from Apple, or choose an IPSW file you already have.",
        I::MacBase if limited => "Unfinished – Create Base continues it; use the same password as before.",
        I::MacBase => "Create it once from the IPSW – about 5 minutes, no manual steps.",
    })
}

/// Paths and versions as found; space in the UI's units (`RequirementText.detail`)
fn detail(p: &PrerequisiteDto) -> Text {
    match (p.free_bytes, p.needed_bytes, p.used_bytes) {
        // The Nix Store's own size where it is a volume of its own (plan B149)
        (Some(free), Some(needed), Some(used)) => Text::fmt(
            "%@ free, about %@ needed – the Nix Store holds %@",
            vec![Arg::bytes(free), Arg::bytes(needed), Arg::bytes(used)],
        ),
        (Some(free), Some(needed), None) => {
            Text::fmt("%@ free, about %@ needed", vec![Arg::bytes(free), Arg::bytes(needed)])
        }
        // Sentences are translated, paths and versions stay as found (plan B160)
        _ if TRANSLATED_DETAILS.contains(&p.detail.as_str()) => Text::new(&p.detail),
        _ => Text::verbatim(&p.detail),
    }
}

/// The checks' details that are words, not paths or versions – the UI shows them translated
const TRANSLATED_DETAILS: &[&str] = &[
    "needs Nix",
    "not found",
    "not set",
    "no flake set",
    "flakes not enabled",
    "no aarch64-linux builder",
    "omaflake's builder",
    "omaflake's builder (default settings)",
    "no Linux builder set up",
    "no hypervisor (kern.hv_support 0)",
    "loaded by your shell files",
    "omaflake's lines in ~/.zshrc",
    "new terminals do not find nix",
    "omaflake's key",
    "no key yet",
    "half-done Nix install",
    "no /dev/kvm – VMs run with TCG (slow)",
];

/// The last IPSW download's error when it failed and none runs now – it may end before a page shows it (plan B146)
fn download_failure(ctx: &Ctx) -> Option<Text> {
    let last = ctx.jobs.iter().rev().find(|j| j.kind == JobKind::IpswDownload)?;
    (last.state == JobState::Failed).then(|| {
        let error =
            last.error_text.clone().unwrap_or_else(|| Text::verbatim(last.error.as_deref().unwrap_or_default()));
        tf!("Download failed: %@", error)
    })
}

/// The IPSW download: on its own, or while the base's creation downloads
pub(crate) fn download_job(ctx: &Ctx) -> Option<&JobDto> {
    ctx.active_job(JobKind::IpswDownload)
        .or_else(|| ctx.active_job(JobKind::MacosInstall).filter(|j| j.progress.as_ref().is_some_and(|p| p.of > 0)))
}

pub fn download(j: &JobDto) -> Download {
    let (done, of) = j.progress.as_ref().map(|p| (p.done, p.of)).unwrap_or((0, 0));
    Download {
        done,
        of,
        text: if of > 0 { Text::fmt("%@ / %@", vec![Arg::bytes(done), Arg::bytes(of)]) } else { Text::verbatim("…") },
    }
}

fn builder_task(ctx: &Ctx) -> Option<&JobDto> {
    ctx.jobs.iter().rev().find(|j| {
        matches!(
            j.kind,
            JobKind::BuilderInstallNix
                | JobKind::BuilderMountNix
                | JobKind::BuilderUninstallNix
                | JobKind::BuilderTrust
                | JobKind::BuilderCheck
                | JobKind::BuilderRepair
                | JobKind::BuilderSetup
                | JobKind::BuilderApply
                | JobKind::BuilderCleanUp
                | JobKind::QemuInstall
                | JobKind::Setup
        ) && j.state.is_active()
    })
}

/// The row a running setup job works on (plan B160: every row of a running job shows it, never a silent wait) –
/// "Set up for NixOS" by its current step
fn works_on(j: &JobDto, item: I) -> bool {
    match j.kind {
        JobKind::BuilderInstallNix | JobKind::BuilderMountNix | JobKind::BuilderUninstallNix => item == I::Nix,
        JobKind::BuilderTrust
        | JobKind::BuilderCheck
        | JobKind::BuilderRepair
        | JobKind::BuilderSetup
        | JobKind::BuilderApply
        | JobKind::BuilderCleanUp => {
            matches!(item, I::LinuxBuilder | I::BuilderRunning)
        }
        JobKind::QemuInstall => item == I::Qemu,
        JobKind::Setup => {
            let step = j.progress.as_ref().map(|p| p.title.as_str()).unwrap_or("");
            match step {
                "Nix" => item == I::Nix,
                "Nix in your terminal" => item == I::Shell,
                "SSH key" => item == I::SshKey,
                "QEMU" => item == I::Qemu,
                "Look" => item == I::Look,
                "Linux builder" => matches!(item, I::LinuxBuilder | I::BuilderRunning),
                _ => false,
            }
        }
        _ => false,
    }
}

/// The `omaf` command's hint: a file in the way, or the link's folder missing from the PATH; else as `hint`
fn cli_hint(p: &PrerequisiteDto, cli: Option<&CliLinkDto>, ctx_host: super::HostOs) -> Text {
    match cli {
        Some(c) if c.blocked && p.status != S::Ok => {
            tf!("%@ is there and is not omaflake's – remove or rename it, then install.", &c.link)
        }
        Some(c) if c.installed && !c.on_path && p.status != S::Ok => {
            let dir = c.link.rsplit_once('/').map(|(d, _)| d.to_string()).unwrap_or_default();
            tf!("Add %@ to your PATH in your shell's profile (export PATH=\"%@:$PATH\").", &dir, &dir)
        }
        _ => hint_on(p, ctx_host),
    }
}

/// Install … and Uninstall … of the `omaf` command (plan B122)
fn cli_actions(ctx: &Ctx, p: &PrerequisiteDto, cli: Option<&CliLinkDto>) -> Vec<Action> {
    let Some(c) = cli else { return vec![] };
    let running = ctx.active_job(JobKind::CliInstall);
    let busy_reason = running.map(|j| Text::verbatim(&j.title));
    let mut actions = Vec::new();
    if let Some(target) = c.target.as_ref().filter(|_| p.status != S::Ok) {
        let text = if c.admin {
            tf!(
                "omaflake creates the link %@ to its omaf (%@), so omaf works in Terminal. A link that is there already is replaced. macOS asks for an administrator.",
                &c.link,
                target
            )
        } else {
            tf!(
                "omaflake creates the link %@ to its omaf (%@), so omaf works in a terminal. A link that is there already is replaced.",
                &c.link,
                target
            )
        };
        let mut a = Action::call("installCli", Text::new("Install …"), Some("install"), "doctor.installCli", json!({}))
            .confirm(Confirm::new(Text::new("Install the omaf command?"), text).button("Install"))
            .busy(running.is_some())
            .enabled_if(running.is_none(), busy_reason.clone())
            .enabled_if(!c.blocked, Some(Text::verbatim(&c.link)));
        a.help = p.install.as_ref().map(Text::verbatim);
        actions.push(a);
    }
    if c.installed {
        let text = if c.admin {
            tf!(
                "omaflake removes the link %@; omaf itself stays in omaflake. macOS asks for an administrator.",
                &c.link
            )
        } else {
            tf!("omaflake removes the link %@; omaf itself stays in omaflake.", &c.link)
        };
        actions.push(
            Action::call("uninstallCli", Text::new("Uninstall …"), Some("trash"), "doctor.uninstallCli", json!({}))
                .confirm(Confirm::new(Text::new("Uninstall the omaf command?"), text).button("Uninstall").destructive())
                .enabled_if(running.is_none(), busy_reason),
        );
    }
    actions
}

fn item(ctx: &Ctx, p: &PrerequisiteDto) -> DoctorItem {
    let report = ctx.doctor.as_ref();
    let task = builder_task(ctx);
    let downloading = download_job(ctx);
    let installing = ctx.active_job(JobKind::MacosInstall).is_some();
    let builder_actions = p.item == I::LinuxBuilder && p.status == S::Ok;
    let cli = report.and_then(|d| d.cli.as_ref()).filter(|_| p.item == I::Cli);
    let cli_installed = cli.is_some_and(|c| c.installed);
    let working = task.filter(|t| works_on(t, p.item));
    let busy = (p.item == I::Ipsw && downloading.is_some())
        || (p.item == I::MacBase && installing)
        || working.is_some()
        || (p.item == I::Cli && ctx.active_job(JobKind::CliInstall).is_some());
    let has_subline = p.status != S::Ok || p.settable_path || busy || builder_actions || cli_installed;
    let task_busy = task.map(|t| Text::verbatim(&t.title));
    let no_task = |a: Action| a.enabled_if(task.is_none(), task_busy.clone());

    let mut actions = Vec::new();
    if p.item == I::Cli {
        actions.extend(cli_actions(ctx, p, cli));
    } else if p.item == I::Nix && p.status == S::Missing && p.detail.starts_with(Prerequisites::NIX_NOT_MOUNTED) {
        // Installed, its volume not at /nix: mount it – installing again would not help
        let mut a = no_task(
            Action::call("mountNix", Text::new("Mount …"), Some("install"), "builder.mountNix", json!({})).confirm(
                Confirm::new(
                    Text::new("Mount the Nix Store?"),
                    Text::new("omaflake mounts the Nix Store volume at /nix and restores what mounts it at every startup (/etc/fstab, /etc/synthetic.conf and the background service org.nixos.darwin-store), only where they are missing. Nothing in the store changes. macOS asks for an administrator."),
                )
                .button("Mount"),
            ),
        );
        a.help = p.install.as_ref().map(Text::verbatim);
        actions.push(a);
    } else if p.item == I::Nix && p.status == S::Missing && p.detail == Prerequisites::NIX_HALF {
        // An installer stopped half way: remove what it left, then install again (plan B160)
        let mut a = no_task(
            Action::call("uninstallNix", Text::new("Remove …"), Some("trash"), "builder.uninstallNix", json!({})).confirm(
                Confirm::new(
                    Text::new("Remove the half-done Nix install?"),
                    Text::new("omaflake runs the installer's own uninstall (/nix/nix-installer uninstall): it removes the /nix volume, Nix's build users and its background service. Restart the Mac afterwards if /nix stays busy, then install again. macOS asks for an administrator."),
                )
                .button("Remove")
                .destructive(),
            ),
        );
        a.help = p.install.as_ref().map(Text::verbatim);
        actions.push(a);
    } else if p.item == I::Nix && p.status == S::Missing && ctx.host == super::HostOs::Macos {
        let mut a = no_task(
            Action::call("installNix", Text::new("Install …"), Some("install"), "builder.installNix", json!({})).confirm(
                Confirm::new(
                    Text::new("Install Nix?"),
                    Text::new("omaflake installs Nix with the official installer of the NixOS community (nix-installer): the /nix volume, Nix's build users and its background service, with flakes, and you among Nix's trusted-users (for omaflake's Linux builder). Your shell files stay as they are. It needs the internet; macOS asks for an administrator – the only time omaflake's setup does. Uninstall later with /nix/nix-installer uninstall."),
                )
                .button("Install"),
            ),
        );
        a.help = p.install.as_ref().map(Text::verbatim);
        actions.push(a);
    } else if p.item == I::Shell && p.detail != "needs Nix" {
        if p.status == S::Missing {
            actions.push(
                Action::call("addShell", Text::new("Add"), Some("add"), "shell.add", json!({}))
                    .help(Text::new("Adds omaflake's lines that load Nix to your ~/.zshrc"))
                    .primary(),
            );
        } else if p.detail.starts_with("omaflake's lines") {
            actions.push(
                Action::call("removeShell", Text::new("Remove"), Some("trash"), "shell.remove", json!({}))
                    .help(Text::new("Removes omaflake's lines from your ~/.zshrc")),
            );
        }
    } else if p.item == I::LinuxBuilder && p.status == S::Missing && p.hint.contains("trusted-users") {
        actions.push(no_task(
            Action::call("trust", Text::new("Allow …"), Some("install"), "builder.trust", json!({})).confirm(
                Confirm::new(
                    Text::new("Allow omaflake's Linux builder?"),
                    Text::new("omaflake adds you to Nix's trusted-users (a line in /etc/nix/nix.conf) and restarts Nix's background service. Then omaflake can hand Nix its Linux builder. Trusted users can tell Nix what it trusts – usual on a Mac with one user. macOS asks for an administrator."),
                )
                .button("Allow"),
            ),
        ));
    } else if p.item == I::LinuxBuilder && p.status == S::Missing {
        let nix_missing =
            report.is_some_and(|d| d.prerequisites.iter().any(|x| x.item == I::Nix && x.status == S::Missing));
        actions.push(
            no_task(
                Action::call("setUp", Text::new("Set Up …"), Some("install"), "builder.setup", json!({})).confirm(
                    Confirm::new(
                        Text::new("Set up the Linux builder?"),
                        Text::new("omaflake sets up its own Linux builder – a small Linux VM that builds NixOS images. It runs as you in the background (a LaunchAgent), its files are in the data folder; nothing in /etc, no administrator. The first time it downloads about 430 MB and then builds omaflake's builder: several minutes."),
                    )
                    .button("Set Up"),
                ),
            )
            .enabled_if(!nix_missing, Some(Text::new("Nix"))),
        );
    } else if p.item == I::BuilderRunning && p.detail == Prerequisites::BUILDER_REFUSES {
        // The builder runs but refuses the Mac's paths (plan B175): a new disk
        let own = !report.is_some_and(|d| {
            d.prerequisites.iter().any(|x| x.item == I::LinuxBuilder && !x.detail.starts_with("omaflake's builder"))
        });
        let text = if own {
            "omaflake stops its Linux builder, deletes its disk and starts it with a new one: a fresh Nix store, built paths in it are gone (they are built or downloaded again). Then it checks that the builder accepts this Mac's builds. No administrator."
        } else {
            "omaflake stops nix-darwin's Linux builder, deletes its disk and starts it with a new one: a fresh Nix store, built paths in it are gone (they are built or downloaded again). Then it checks that the builder accepts this Mac's builds. macOS asks for an administrator."
        };
        actions.push(
            no_task(
                Action::call("repair", Text::new("Repair …"), Some("reset"), "builder.repair", json!({})).confirm(
                    Confirm::new(Text::new("Give the Linux builder a new disk?"), Text::new(text))
                        .button("Repair")
                        .destructive(),
                ),
            )
            .primary(),
        );
    } else if p.item == I::BuilderRunning && p.detail == Prerequisites::BUILDER_SMALL_DISK {
        // A disk made before its settings (plan B182): the same new disk as the repair
        actions.push(no_task(
            Action::call("newDisk", Text::new("New Disk …"), Some("reset"), "builder.repair", json!({})).confirm(
                Confirm::new(
                    Text::new("Give the Linux builder a new disk?"),
                    Text::new(
                        "omaflake stops the Linux builder, deletes its disk and starts it with a new one in the size of its settings: what it built is built or downloaded again. Then it checks the builder. nix-darwin's builder asks for an administrator.",
                    ),
                )
                .button("New Disk"),
            ),
        ));
    } else if p.item == I::BuilderRunning && p.status == S::Ok && ctx.host == super::HostOs::Macos {
        actions.push(no_task(
            Action::call("check", Text::new("Test Build"), Some("ok"), "builder.check", json!({}))
                .help(Text::new("Checks that the Linux builder accepts this Mac's builds (a tiny test build)")),
        ));
    } else if p.item == I::SshKey && p.status == S::Missing {
        actions.push(
            Action::call("createKey", Text::new("Create"), Some("add"), "doctor.createSshKey", json!({}))
                .help(Text::new("Creates omaflake's own SSH key in the data folder"))
                .primary(),
        );
    } else if p.item == I::Qemu && p.install.is_some() && ctx.host == super::HostOs::Macos {
        let upgrade = p.install.as_deref() == Some(Prerequisites::UPGRADE_QEMU);
        let mut a = no_task(Action::call(
            "install",
            Text::new(if upgrade { "Update …" } else { "Install …" }),
            Some(if upgrade { "update" } else { "install" }),
            "qemu.install",
            json!({"upgrade": upgrade}),
        ));
        a.help = p.install.as_ref().map(Text::verbatim);
        actions.push(a);
    } else if p.item == I::Look && p.status != S::Ok {
        let installing = ctx.active_job(JobKind::LookInstall).is_some();
        actions.push(
            Action::call(
                "installLooks",
                Text::new("Install Recommended"),
                Some("download"),
                "looks.installRecommended",
                json!({}),
            )
            .help(Text::new("Installs omaflake's own look and Catppuccin from the official catalog"))
            .busy(installing)
            .enabled_if(!installing, None)
            .primary(),
        );
    } else if p.item == I::Credential && p.status != S::Ok {
        actions.push(
            Action::show("credential", Text::new("New Credential …"), Some("add"), "credential", json!({})).primary(),
        );
    } else if let Some(command) = &p.install {
        actions.push(
            Action::call(
                "install",
                Text::new(if command == Prerequisites::UPGRADE_QEMU { "Update …" } else { "Install …" }),
                Some(if command == Prerequisites::UPGRADE_QEMU { "update" } else { "install" }),
                "ui.terminal",
                json!({"line": command}),
            )
            .help(Text::verbatim(command)),
        );
    }
    if builder_actions && p.detail.starts_with("omaflake's builder") {
        // omaflake's own builder: as the user, no administrator (plan B160)
        actions.push(no_task(
            Action::call("cleanUp", Text::new("Clean Up"), Some("clean"), "builder.cleanUp", json!({}))
                .help(Text::new("Removes unused data from the Linux builder's disk")),
        ));
        actions.push(no_task(
            Action::call("apply", Text::new("Apply Settings …"), Some("apply"), "builder.apply", json!({})).confirm(
                Confirm::new(
                    Text::new("Apply Linux builder settings?"),
                    Text::new("omaflake builds its Linux builder again with its settings (nix/hosts/builder-vm.nix and your changes), gives it a new disk and restarts it. This takes a few minutes; no administrator."),
                )
                .button("Apply"),
            ),
        ));
    } else if builder_actions {
        actions.push(no_task(
            Action::call("cleanUp", Text::new("Clean Up …"), Some("clean"), "builder.cleanUp", json!({}))
                .help(Text::new("Removes unused data from the Linux builder's disk (asks for an administrator)")),
        ));
        let replaces = !report.is_some_and(|d| d.builder_system_is_omaflakes);
        let text = if replaces {
            "This Mac's nix-darwin system does not come from omaflake's image flake yet. Applying replaces your current nix-darwin configuration with omaflake's Linux builder host (nix/hosts/linux-builder.nix and your local/darwin.nix). Only continue if you set up nix-darwin for omaflake."
        } else {
            "Runs darwin-rebuild with omaflake's Linux builder host. This takes a few minutes; macOS asks for an administrator."
        };
        let params = if replaces { json!({"force": true}) } else { json!({}) };
        actions.push(no_task(
            Action::call("apply", Text::new("Apply Settings …"), Some("apply"), "builder.apply", params)
                .help(Text::new("darwin-rebuild switch with omaflake's Linux builder host (asks for an administrator)"))
                .confirm(
                    Confirm::new(Text::new("Apply Linux builder settings?"), Text::new(text)).button(if replaces {
                        "Replace and Apply"
                    } else {
                        "Apply"
                    }),
                ),
        ));
    }
    if matches!(p.item, I::LinuxBuilder | I::BuilderRunning) {
        actions.push(ctx.guide("linux-builder", "linux-builder", "Linux Builder Guide"));
    }
    if p.settable_path {
        let (kind, prompt) = match p.item {
            I::Flake => (AskKind::Folder, "Folder with the flake.nix of your recipes"),
            I::Nix => (AskKind::File, "The nix binary"),
            _ => (AskKind::File, ctx.on_host("The qemu-system-aarch64 binary", "The qemu-system-* binary")),
        };
        let item = serde_json::to_value(p.item).unwrap_or_default();
        actions.push(
            Action::call("setPath", Text::new("Set Path …"), Some("folder"), "doctor.setPath", json!({"item": item}))
                .ask(Ask::new("path", kind, Text::new("Set Path …")).prompt(Text::new(prompt))),
        );
    }
    if p.item == I::Ipsw {
        match downloading {
            Some(j) => actions.push(Action::call(
                "pause",
                Text::new("Pause"),
                Some("pause"),
                "jobs.cancel",
                json!({"id": j.id}),
            )),
            None => actions.push(
                Action::call(
                    "download",
                    Text::new(if p.hint == Prerequisites::IPSW_PARTIAL { "Continue Download" } else { "Download" }),
                    Some("download"),
                    "macos.downloadIpsw",
                    json!({}),
                )
                .enabled_if(report.is_some_and(|d| d.latest_release.is_some()), None),
            ),
        }
    }
    // The Nix Store keeps what builds leave until a garbage collection (plan B149): clean it up from here
    if p.item == I::NixSpace {
        let cleaning = ctx.active_job(JobKind::NixCleanUp).is_some();
        let building = ctx.active_job(JobKind::Build).is_some();
        let mut clean = Action::call("cleanUpNix", Text::new("Clean Up …"), Some("clean"), "nix.cleanUp", json!({}))
            .help(Text::new("Removes what nothing uses from the Nix Store (nix-collect-garbage)"))
            .busy(cleaning)
            .enabled_if(!cleaning && !building, building.then(|| Text::new("A build runs")))
            .confirm(
                Confirm::new(
                    Text::new("Clean up the Nix Store?"),
                    Text::new("Removes everything in the Nix Store that nothing uses any more – mostly what earlier builds left. Your images, VMs and Nix profiles stay. The next build may take longer, as parts are built or downloaded again."),
                )
                .button("Clean Up"),
            );
        if p.status != S::Ok {
            clean = clean.primary();
        }
        actions.push(clean);
    }
    if p.item == I::MacBase {
        actions.push(
            Action::show(
                "macBase",
                Text::new(if installing { "Show …" } else { "Create …" }),
                Some("macos"),
                "macBase",
                json!({}),
            )
            .busy(installing),
        );
    }
    let show_detail = p.status != S::Missing || matches!(p.item, I::FlakeEval | I::NixSpace | I::MacSpace | I::Ipsw);
    DoctorItem {
        item: p.item,
        status: p.status,
        title: Text::new(&p.title),
        detail: show_detail.then(|| detail(p)),
        hint: (p.status != S::Ok).then(|| {
            if let Some(failure) = (p.item == I::Ipsw && downloading.is_none()).then(|| download_failure(ctx)).flatten()
            {
                failure
            } else if p.item == I::Cli {
                cli_hint(p, cli, ctx.host)
            } else {
                hint_on(p, ctx.host)
            }
        }),
        expandable: has_subline,
        expanded: has_subline && (p.status != S::Ok || busy),
        busy,
        // What the job does now: its newest line, else its title
        task: working.map(|t| Text::verbatim(t.last_line.as_deref().unwrap_or(&t.title))),
        download: if p.item == I::Ipsw { downloading.map(download) } else { None },
        actions,
        prerequisite: p.clone(),
    }
}

pub(crate) fn page(ctx: &Ctx) -> DoctorPage {
    let report = ctx.doctor.as_ref();
    let section = |id: &str, title: &str, list: Option<&Vec<PrerequisiteDto>>| DoctorSection {
        id: id.into(),
        title: Text::new(title),
        items: list.map(|l| l.iter().map(|p| item(ctx, p)).collect()).unwrap_or_default(),
    };
    DoctorPage {
        title: Text::new("Doctor"),
        versions: vec![InfoLine::new(
            "omaflake",
            match &ctx.client_version {
                Some(app) => tf!("App %@ · Core %@", app.as_str(), ctx.omaf_version.as_str()),
                None => tf!("Core %@", ctx.omaf_version.as_str()),
            },
        )],
        update: update_line(ctx),
        intro: Text::new(ctx.on_host(
            "omaflake uses what your Mac already has. Set up missing parts here or enter their path.",
            "omaflake uses what this computer already has. Set up missing parts here or enter their path.",
        )),
        checking: ctx.checking,
        checked: report.map(|d| d.checked),
        actions: page_actions(ctx),
        sections: {
            let mut sections = vec![
                section("requirements", "Requirements", report.map(|d| &d.prerequisites)),
                section("nixos", "NixOS – before building images", report.map(|d| &d.nixos_preflight)),
            ];
            // macOS guests need a Mac: a Linux host has no macOS section (plan B124)
            if ctx.host == super::HostOs::Macos {
                sections.push(section("macos", "macOS – before building images", report.map(|d| &d.macos_preflight)));
            }
            sections
        },
        issues: report.map(|d| d.issues).unwrap_or(0),
    }
}

/// "Set Up for NixOS" (while something for NixOS guests is missing on a Mac, plan B160) and Check Again. A running
/// setup shows its step instead.
fn page_actions(ctx: &Ctx) -> Vec<Action> {
    let mut actions = Vec::new();
    let setup = ctx.active_job(JobKind::Setup);
    let missing = ctx.doctor.as_ref().is_some_and(|d| {
        d.prerequisites.iter().chain(d.nixos_preflight.iter()).any(|p| {
            p.status == S::Missing
                && matches!(
                    p.item,
                    I::Nix | I::Flakes | I::Shell | I::SshKey | I::Qemu | I::Look | I::LinuxBuilder | I::BuilderRunning
                )
                && !p.detail.starts_with("no hypervisor")
        })
    });
    if ctx.host == super::HostOs::Macos && (missing || setup.is_some()) {
        let label = match setup.and_then(|j| j.progress.as_ref()) {
            Some(p) if p.of > 0 => {
                tf!("Setting up – step %@ of %@", (p.done + 1).min(p.of).to_string(), p.of.to_string())
            }
            _ if setup.is_some() => Text::new("Setting up …"),
            _ => Text::new("Set Up for NixOS …"),
        };
        let other = builder_task(ctx).filter(|j| j.kind != JobKind::Setup);
        actions.push(
            Action::call("setup", label, Some("install"), "setup.nixos", json!({}))
                .confirm(
                    Confirm::new(
                        Text::new("Set up omaflake for NixOS guests?"),
                        Text::new("omaflake sets up what is missing, one step after the other: Nix (macOS asks for an administrator once), Nix in your terminal (~/.zshrc), omaflake's SSH key, its QEMU, a look and its Linux builder (as you, in the background). Steps that are done are skipped. The first time this takes a while – each row shows what happens. Then only your credential is left."),
                    )
                    .button("Set Up"),
                )
                .busy(setup.is_some())
                .enabled_if(setup.is_none() && other.is_none(), other.map(|j| Text::verbatim(&j.title)))
                .primary(),
        );
    }
    // The first start's questions (plan B144) while there is no credential yet – the app opens them by itself once
    if ctx.credentials.is_empty() {
        actions.push(Action::show("welcome", Text::new("Guided Setup …"), Some("install"), "welcome", json!({})));
    }
    actions.push(
        Action::call("check", Text::new("Check Again"), Some("refresh"), "doctor.check", json!({}))
            .busy(ctx.checking)
            .enabled_if(!ctx.checking, None),
    );
    actions
}

fn update_line(ctx: &Ctx) -> UpdateLine {
    let u = &ctx.update;
    let check = Action::call("checkNow", Text::new("Check Now"), Some("refresh"), "updates.check", json!({}));
    let (text, tone, actions) = if u.checking {
        (Text::verbatim("…"), Tone::Normal, vec![])
    } else if let Some(error) = &u.error {
        (tf!("Check failed: %@", error), Tone::Warning, vec![check])
    } else {
        match u.state {
            None | Some(UpdateState::NoFeed) => {
                (Text::new("No update feed yet (comes with the first release)"), Tone::Normal, vec![])
            }
            Some(UpdateState::Off) => (Text::new("Check switched off (Settings)"), Tone::Normal, vec![]),
            Some(UpdateState::UpToDate) => (
                Text::fmt("Up to date – checked %@", vec![Arg::date(u.checked.unwrap_or_else(crate::json::now))]),
                Tone::Normal,
                vec![check],
            ),
            Some(UpdateState::Available) => {
                let r = u.release.as_ref();
                let version = r.map(|r| r.version.clone()).unwrap_or_default();
                let url = r.map(|r| r.page.clone().unwrap_or_else(|| r.url.clone())).unwrap_or_default();
                (
                    tf!("%@ available", version),
                    Tone::Accent,
                    vec![
                        Action::call(
                            "download",
                            Text::new("Download …"),
                            Some("download"),
                            "ui.open",
                            json!({"url": url}),
                        ),
                        check,
                    ],
                )
            }
        }
    };
    UpdateLine { label: Text::new("Updates"), text, tone, busy: u.checking, actions }
}

// MARK: macOS base

/// "macOS 27.0.1 (26A434), 16 GB from Apple – here: …" → "26A434": the build in the first parentheses of the IPSW
/// check's detail
fn build_in(detail: &str) -> Option<String> {
    let open = detail.find('(')?;
    let close = open + detail[open..].find(')')?;
    let build = &detail[open + 1..close];
    (!build.is_empty() && build.chars().all(|c| c.is_ascii_alphanumeric())).then(|| build.to_string())
}

/// A base's row on the macBase page (plan B166)
fn base_row(ctx: &Ctx, b: &crate::api::types::BaseDto, chosen: &str, ipsw: Option<&str>, busy: bool) -> MacBaseRow {
    let credential_name = b.credential.as_ref().map(|id| {
        ctx.credentials
            .iter()
            .find(|c| &c.id == id)
            .map(|c| c.name.clone())
            .or_else(|| b.credential_name.clone())
            .unwrap_or_else(|| id.clone())
    });
    // Create Again with the base's credential – one without a credential yet takes the page's chosen one
    let again_with = b.credential.clone().unwrap_or_else(|| chosen.to_string());
    let again_name = ctx.credentials.iter().find(|c| c.id == again_with).map(|c| c.name.clone()).unwrap_or_default();
    let mut params = json!({"credential": again_with, "replace": b.name});
    if let Some(i) = ipsw {
        params["ipsw"] = json!(i);
    }
    let recreate = Action::call("recreate", Text::new("Create Again …"), Some("refresh"), "macos.installBase", params)
        .confirm(
            Confirm::new(
                tf!("Create “%@” again?", &b.name),
                tf!("A new base is created with the credential “%@” and omaflake's own SSH key; once it is ready, “%@” is deleted. macOS images and VMs made from it stay – they are copies.", &again_name, &b.name),
            )
            .button("Create Again"),
        )
        .enabled_if(!busy, Some(Text::new("A macOS base is being created – its progress is below")))
        .enabled_if(!again_with.is_empty(), Some(Text::new("Create a credential first – the key at the top")))
        .enabled_if(!b.in_use, Some(Text::new("An image build uses it – wait until it ends")));
    let remove = Action::call("remove", Text::new("Remove …"), Some("trash"), "macos.removeBase", json!({"name": b.name}))
        .destructive()
        .confirm(
            Confirm::new(
                tf!("Remove the macOS base “%@”?", &b.name),
                Text::new(
                    "It is deleted and cannot be restored. macOS images and VMs made from it stay – they are copies. New macOS images need a base.",
                ),
            )
            .button("Remove")
            .destructive(),
        )
        .enabled_if(!b.in_use, Some(Text::new("An image build uses it – wait until it ends")));
    let hint = if !b.ready {
        Some(Hint::new(
            "warning",
            Text::new("Not installed completely – it cannot build images; Create Base continues it."),
            Tone::Warning,
        ))
    } else if b.credential.is_none() {
        Some(Hint::new(
            "lock-warning",
            Text::new("No credential yet – assign the one its user was created with (Images → macOS Bases)."),
            Tone::Warning,
        ))
    } else if !b.own_key {
        Some(Hint::new(
            "warning",
            Text::new(
                "Made before omaflake's own SSH key – its builds need your key in the ssh-agent. Create it again.",
            ),
            Tone::Warning,
        ))
    } else {
        None
    };
    MacBaseRow {
        name: b.name.clone(),
        build: b.os_build.clone(),
        version: b.version.clone(),
        credential: credential_name,
        user: b.user.clone(),
        created: b.created,
        own_key: b.own_key,
        hint,
        actions: vec![recreate, remove],
    }
}

pub(crate) fn mac_base_page(ctx: &Ctx, p: &MacBaseParams) -> MacBasePage {
    let report = ctx.doctor.as_ref();
    let find = |item: I| report.and_then(|d| d.macos_preflight.iter().find(|x| x.item == item));
    let base_item = find(I::MacBase);
    let ipsw_item = find(I::Ipsw);
    let installing = ctx.active_job(JobKind::MacosInstall).is_some();
    let downloading = download_job(ctx);
    // The guest user comes from a credential – the default one unless another is chosen (plan B139)
    let credential = p
        .values
        .credential
        .clone()
        .filter(|id| ctx.credentials.iter().any(|c| &c.id == id))
        .or_else(|| ctx.settings.default_credential.clone())
        .unwrap_or_default();
    let ipsw = p.values.ipsw.clone().filter(|s| !s.is_empty());
    let base = base_item.and_then(|b| match b.status {
        S::Ok => Some(Hint::new("ok", Text::verbatim(&b.detail), Tone::Success)),
        S::Limited => Some(Hint::new("warning", hint_on(b, ctx.host), Tone::Warning)),
        S::Missing => None,
    });
    let mut ipsw_actions = Vec::new();
    match downloading {
        Some(j) => ipsw_actions.push(Action::call(
            "pause",
            Text::new("Pause"),
            Some("pause"),
            "jobs.cancel",
            json!({"id": j.id}),
        )),
        None => {
            if ipsw.is_some() {
                let mut apple =
                    Action::field("useApple", Text::new("Use Apple's"), None, "macBase", json!({}), "ipsw", "ipsw");
                apple.params["value"] = Value::Null;
                ipsw_actions.push(apple);
            }
            ipsw_actions.push(
                Action::field(
                    "chooseIpsw",
                    Text::new("Choose IPSW …"),
                    Some("folder"),
                    "macBase",
                    json!({}),
                    "ipsw",
                    "ipsw",
                )
                .ask({
                    let mut a = Ask::new("value", AskKind::File, Text::new("Choose IPSW …"))
                        .prompt(Text::new("A macOS restore image (.ipsw) from Apple"));
                    a.accept = Some("ipsw".into());
                    a
                }),
            );
        }
    }
    let ipsw_text = match (&ipsw, ipsw_item) {
        (Some(path), _) => Text::verbatim(path.rsplit('/').next().unwrap_or(path)),
        (None, Some(i)) => Text::verbatim(&i.detail),
        (None, None) => Text::verbatim("…"),
    };
    let fields = vec![super::credentials::credential_choice(
        ctx,
        &credential,
        "The base's user gets this credential's user, full name and password; macOS images are built with it, so no password is ever asked.",
    )
    .enabled_if(!installing, None)];
    let mut params = json!({"credential": credential});
    if let Some(i) = &ipsw {
        params["ipsw"] = json!(i);
    }
    // A base per build and credential (plan B166): disabled only when the chosen credential has a base of the IPSW's
    // build already – known up front for Apple's IPSW (its build in the Doctor's line), not for a chosen file
    let ipsw_build = if ipsw.is_none() { ipsw_item.and_then(|i| build_in(&i.detail)) } else { None };
    let existing = ipsw_build.as_deref().and_then(|build| {
        ctx.bases.iter().find(|b| {
            b.ready
                && !b.unchained
                && b.credential.as_deref() == Some(credential.as_str())
                && b.os_build.as_deref() == Some(build)
        })
    });
    let create = Action::call("create", Text::new("Create Base"), Some("create"), "macos.installBase", params)
        .enabled_if(!installing, Some(Text::new("running …")))
        .enabled_if(downloading.is_none(), None)
        .enabled_if(!credential.is_empty(), Some(Text::new("Create a credential first – the key at the top")))
        .enabled_if(
            existing.is_none(),
            existing.map(|b| {
                tf!(
                    "%@ is this credential's base of macOS %@ – Create Again replaces it",
                    &b.name,
                    b.os_build.clone().unwrap_or_default()
                )
            }),
        )
        .primary();
    // Unchained bases (plan B184) are copies of these: made and removed in Images → macOS Bases, not created here
    let bases = ctx
        .bases
        .iter()
        .filter(|b| !b.unchained)
        .map(|b| base_row(ctx, b, &credential, ipsw.as_deref(), installing || downloading.is_some()))
        .collect();
    MacBasePage {
        title: Text::new("Create macOS Base"),
        intro: Text::new(
            "Creates your macOS base from the IPSW: the guest user from your credential, automatic login, SSH with your keys from ~/.ssh. No manual steps and no password to type; a missing IPSW is downloaded first.",
        ),
        base,
        bases,
        ipsw: IpswLine {
            label: Text::new("macOS from Apple (IPSW)"),
            text: ipsw_text,
            download: downloading.map(download),
            actions: ipsw_actions,
        },
        fields,
        installing,
        actions: if ctx.credentials.is_empty() {
            vec![super::credentials::new_credential(), create]
        } else {
            vec![create]
        },
        log: ctx.jobs.iter().rev().find(|j| j.kind == JobKind::MacosInstall).map(super::build::log_ref),
    }
}
