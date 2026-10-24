---
title: Design (spec)
nav_order: 4
permalink: /spec/
---

# Spec – omaflake: clean NixOS and macOS VMs at the push of a button

Status: draft v0.4 · 2026-10-16 · early development, not yet released

> **v0.3:** tart is replaced entirely by an own runner on Virtualization.framework (DECISION, 2026-10-16).
> Guideline: every component must be publishable as open source (§11).

## 1. Goal

On a Mac with Apple Silicon (reference machine: MacBook Pro M1 Max, 64 GB), create, start, reset and throw away
a **clean VM** of a user-defined **recipe** at any time – via CLI and via a menu bar app, with one click.
One app manages both guest kinds:

| Guest kind | Example recipes | Content | Runtime |
|---|---|---|---|
| **NixOS** (+ Hyprland) | minimal, research, lab (dev later) | NixOS modules | own QEMU build (HVF, virgl) |
| **macOS** | minimal, research, lab (office, dev later) | nix-darwin modules on top of a manually prepared base | own runner `omaf-runner` (Virtualization.framework) |

The recipe names above are examples; users define their own recipes in their own flake (§11.1).
VM content is as declarative as possible (Nix). Whatever cannot be declared for macOS goes once into
the **base** and is documented in a checklist (§6.5).
The app/CLI only takes care of **operations**.

## 2. Scope

| In scope | Out of scope |
|---|---|
| Linux guests: NixOS only, the host's architecture (aarch64 on a Mac; x86_64 or aarch64 on a Linux host, plan B124) | other Linux distributions, Omarchy, Windows, guests of another architecture than the host's |
| macOS guests (Apple Silicon) via an own runner on Virtualization.framework | tart, UTM, Lume or similar as runtime; Cirrus Labs images as foundation |
| QEMU with Apple Hypervisor (HVF) for NixOS; on a Linux host QEMU with KVM (TCG as a slow fallback) | Virtualization.framework for Linux guests (no 3D) |
| Recipes as Nix modules (NixOS or nix-darwin) | Ansible, Packer, MDM |
| macOS installation from Apple's IPSW (restore image) | registry/OCI images (`pull`/`push`) |
| Host: macOS on Apple silicon (NixOS and macOS guests); Linux on x86_64 or aarch64 (NixOS guests, plan B124); apps: SwiftUI on macOS, iced on Linux, `omaf ui` on both | Windows host, macOS guests on a Linux host, web UI |
| Local use | multi-user, server operation; distribution only with §11 |

## 3. Terms

- **Flake** – Nix's unit of distribution: a folder or repository with `flake.nix`/`flake.lock` (pinned inputs such as
  nixpkgs). omaflake's **image flake** is `nix/` (shipped in the app): it builds images from recipes and holds the
  guest modules, the omaflake agent and the Linux builder host – but no recipes (plan B107).
- **Recipe** – what a VM is, one folder `<author>/<kind>/<name>/`: `default.nix` (a NixOS or nix-darwin module – the
  content), `recipe.toml` (metadata and runtime defaults) and its own files. Identity `<author>/<kind>/<name>`; as an
  ID `<author>.<kind>.<name>` (flake outputs, images, instances, `current/` links). Users see the name with the guest
  kind; the CLI takes the name, `<kind>/<name>` or the full form and asks when several match.
- **Recipe source** – where recipes come from (§11.3): omaflake/recipes on GitHub by default, other GitHub
  repositories (also private), an HTTPS folder with a `catalog.json`, or a local folder.
- **Base** (macOS only) – VM freshly installed with `omaf-runner` from an Apple IPSW and prepared by hand
  (`mac-base-<os-build>`), built automatically by `omaf macos install`; background in `docs/macos-base.md`.
- **Runner** (`omaf-runner`) – own Swift program that runs exactly one macOS VM via Virtualization.framework
  (window or headless) and can be controlled over a Unix socket; counterpart to the QEMU process.
- **Image** ("Vorlage") – immutable image built from a recipe at one revision – the baked and frozen cake; described by
  its `image.json` (recipe, author, kind, name, flake, revision, store path, time).
  A package `~/.omaflake/images/<id>-<rev>-<hash>.omaflake-image/` (§11.10): NixOS: `disk.qcow2` (UEFI), copied out of
  the Nix store; macOS: the VM folder (RAW disk) that is never started.
- **Instance** – a concrete VM: NixOS = thin qcow2 overlay on top of an image; macOS = APFS clone (`clonefile`) of the image folder
  with a new machine identifier and MAC address.
- **Ephemeral instance** – instance whose changes are discarded on shutdown.
- **Backend** – runtime per guest kind: `QEMUBackend` (NixOS), `VZBackend` (macOS, controls `omaf-runner`).
- **Guest user** – login user inside the guest (for SSH): `user = "…"` in the recipe's `recipe.toml` or `omaf new --user`; default is the Mac user name.

## 4. Architecture

```
recipe sources (§11.3) → <author>/<kind>/<name>/{recipe.toml,default.nix} (downloaded or a local folder)
        │  staged into one folder → input omaflake-recipes of the image flake nix/ (+ modules/*.nix, nixosModules.vm-guest)
        │
        ├── kind = nixos ─────────────────────────────┐
        │  nix build <image flake>#images.<recipe>     │  (Linux builder via nix-darwin)
        │  → images/<recipe>-<rev>-<hash>.omaflake-image/disk.qcow2 (copied out of the store)
        │  omaf new → qemu-img create -b <relative path> … overlay
        │  omaf start → qemu-system-aarch64 -accel hvf … (virtio-gpu-gl, 9p, SSH forwarding)
        │
        └── kind = macos ─────────────────────────────┐
           omaf macos install → fetch IPSW from Apple → VZMacOSInstaller → mac-fresh-<os-build>
           → clone, provisioning, SSH keys, shutdown → mac-base-<os-build> (folder, immutable)
           omaf build <macOS recipe>:
             clonefile mac-base → omaf-runner headless → IP from DHCP leases
             → SSH: nix copy (closure built on the Mac) + darwin-rebuild activate
             → shut down → folder becomes images/<id>-<rev>-<hash>.omaflake-image/
           omaf new → clonefile image folder + new VZMacMachineIdentifier + new MAC
           omaf start → omaf-runner <instance> [--window] (own process, socket, VZVirtualMachineView)
        ▲
Rust core `omaflake-core` (crates/, one backend per guest kind) ── CLI `omaf` (linked)
        ▲  JSON-RPC 2.0 over stdio: `omaf serve` (docs/api.md)
macOS app (SwiftUI, OmaflakeKit's CoreClient) – later the Linux app (Rust, iced; plan B119)
```

The logic lives in the Rust core; every UI is a client of `omaf serve` and holds only presentation (plan B117).
`omaf-runner` stays Swift (Virtualization.framework) and is started by the core.

**Principle:** The image is immutable. "Reset" = discard the instance and create it anew.
"Update" = rebuild the image and switch the instance to the new image. This applies to both guest kinds.

## 5. Data layout

Data folder: `~/.omaflake` by default – deliberately not in Documents or Desktop (privacy-protected; omaflake asks for
no access to them unless the user chooses a folder there). Movable in the app or with `omaf config --home <dir>`;
the settings file always stays at `~/.omaflake/config.json` and records the data folder (`home`).

```
~/.omaflake/
  images/<recipe-id>-<rev>-<hash>.omaflake-image/
                                   one package per image (§11.10) with image.json (what it is – never parsed from the
                                   name). NixOS: + disk.qcow2 (copied out of the Nix store, read-only, no GC root);
                                   macOS: the VM folder, read-only
  images/current/<recipe-id>       symlink to the current image per recipe (both guest kinds)
  bases/mac-fresh-<os-build>/      macOS: fresh installation from the IPSW, never started, read-only
  bases/mac-base-<os-build>/       macOS: base (automatic, `omaf macos install`), same layout, read-only
  ipsw/<os-build>.ipsw             downloaded restore images
  instances/<name>.omaflake-vm/    one package per VM (§11.10) – what makes up the VM, nothing that only lives while it runs
                                   (every JSON file omaflake writes names its JSON Schema in `$schema`, §5.1; older ones are migrated when read)
    instance.json                  recipe, kind, system, image, CPU/RAM, network, share, ephemeral, user, SSH port (NixOS)
    disk.qcow2, efivars.fd         NixOS only (overlay – its backing file a path relative to the package –, UEFI variables)
    vm/                            macOS only: clone of the image folder
    known_hosts                    the guest's SSH host key
    image.omaflake-image/          only in exported VMs and VMs that run from elsewhere: the image they need
  run/<name>/                      what only lives while a VM runs, outside its package: qemu.pid, qmp.sock, qga.sock,
                                   serial.log, qemu.log, .lock, agent socket and state; macOS: runner.sock, runner.pid,
                                   runner.log (omaf-runner; SSH on the NAT address, port 22)
  recipes/                         downloaded and imported recipes (§11.3), `.installed.json`
  share/<name>/                    default share folder per instance (NixOS: 9p, macOS: virtiofs); a folder the user
                                   chose instead is recorded as `sharePath` in instance.json
  qemu/<tag>/, qemu/current        own QEMU build (qemu/build.sh, relocatable)
  logs/                            image builds, first boot of a base
  cache/                           Dock icon for QEMU, askpass helper (no secrets)
  config.json                      settings (only in ~/.omaflake, even if the data folder is elsewhere)
  developer/                       contributors' own local files (e.g. tokens for their tooling) – never read,
                                   moved or cleaned up by omaflake
  _to_delete/                      only from older versions and development (omaflake itself deletes, see 9.)

macOS VM folder (own format, modeled on Apple's sample code):
  config.json        CPU, RAM, display, MAC address, format version
  hardware-model     VZMacHardwareModel.dataRepresentation
  machine-id         VZMacMachineIdentifier.dataRepresentation (new per instance)
  aux.img            VZMacAuxiliaryStorage
  disk.img           system disk, RAW, sparse (VZDiskImageStorageDeviceAttachment supports RAW only, plan BM9)
```

### 5.1 File versions (plan B114)

Every JSON file omaflake writes starts with `"$schema": "https://omaflake.dev/schema/<type>/v<n>.json"` – what the file
is and its version, one count per type. The JSON Schemas are in `docs/schema/` and published on omaflake.dev (GitHub
Pages from `docs/`); a published version is never removed. omaflake brings an older file up to date when it reads it
(step by step, v<n> → v<n+1>, and writes it back) and refuses a newer one or another type. A file without `$schema` is
version 0 (before B114).

**Compatibility.** Each omaflake reads every version of a type from a **minimum** on, and writes the newest. A new
feature that changes a file gets a new version; older versions stay readable. The goal **from the stable release 1.0.0
on**: a minimum is raised only with a major release (2.0.0, …) – exceptions are possible but rare. Before 1.0.0 a
minor release may still raise it (and says so in the changelog). A file below the minimum or
newer than this omaflake is not read: the app and the CLI list it with a warning (VMs, images, recipes) instead of
leaving it out silently.

| Type | File | Minimum (0.2.0) |
|---|---|---|
| `image` | `image.json` in an image package | 0 |
| `instance` | `instance.json` in a VM package | 0 |
| `settings` | `~/.omaflake/config.json` | 0 |
| `installed` | `<data>/recipes/.installed.json` | 0 |
| `macos-vm` | `config.json` of a macOS VM folder (base, image, VM) | 0 |
| `manifest` | `manifest.json` of an exported package | 0 |
| `catalog` | a source's `catalog.json` (`$schema` from v2; v1 had `"schema": 1`) | 1 |
| `recipe` | `recipe.toml` as `"$schema" = "…/recipe/v1.json"` (TOML; editors with Taplo can use it) | 0 |

Catalogs and recipes come from their sources and are **never migrated** by omaflake – a newer version always comes from
its author. An older one within the minimum is read as it is (version 0/1: no `systems`, so the architectures of Apple
silicon). After *Refresh* in the Recipes tab, newer versions of downloaded recipes are listed and can be taken over, or
added next to the older one when older recipe versions are kept (plan B111).

## 6. Components

### 6.1 Image flake and recipes (Nix)

The image flake `nix/` (this repo, shipped in the app – nix/README.md):

- `modules/vm-guest.nix` (root flake output `nixosModules.vm-guest`) – guest hardware (UEFI, virtio, labels
  `nixos`/`ESP`, 9p share, option `vm.shareFolder.enable`) and the omaflake agent (§11.9)
- `modules/base.nix` – guest user, locale, SSH (your public SSH keys), Nix settings; `modules/hyprland.nix` +
  `home/` – desktop and styling of every NixOS image
- **macOS:** `modules/darwin-guest.nix` (guest user as `system.primaryUser`, Nix, SSH, widgets off)
- inputs `omaflake-guest` (who you are) and `omaflake-recipes` (the recipes of the sources, staged by omaflake);
  outputs `images.<system>.<id>`, `nixosConfigurations.vm-<system>-<id>`, `darwinConfigurations.<id>`, `lib.recipeDefaults`

The recipes (omaflake/recipes and other sources, §11.3): `default.nix` per recipe on top of these modules (apps via
packages or, for macOS, `homebrew.casks`; Dock via `system.defaults.dock.persistent-apps`, settings via
`system.defaults.*`), `recipe.toml` – CPU, RAM, disk, network mode, share, ephemeral, display, switches, tags,
license, optional `user`; a recipe may build on another of its kind (`requires`, `imports = [ ../minimal ]`).

### 6.2 Core library + CLI `omaf`

**Rust workspace** (repo root, `Cargo.toml`): `crates/omaflake-core` (logic) and `crates/omaf` (the CLI and, with
`omaf serve`, the JSON-RPC interface every UI uses – docs/api.md). **Swift package** (`Package.swift`): `OmaflakeKit`
(the client of `omaf serve` with the API's types, the agent protocol and macOS VM bundle for the runner, the brand
images), `omaf-runner` (runner for macOS guests), app `omaflake`.
External tools as child processes: `nix`, `qemu-img`, `qemu-system-aarch64`, `ssh`. No dependency on tart.

| Command | Effect |
|---|---|
| `omaf recipes` | recipes (their `recipe.toml`, with `kind`) + image status |
| `omaf macos install [--ipsw <file>]` | base from Apple's IPSW (default: latest supported), ready to build images from |
| `omaf build <recipe>` | NixOS: `nix build .#images.<system>.<recipe>`, the qcow2 copied into the image package, symlink. macOS: image pipeline §6.5 |
| `omaf export vm\|image\|recipe …` / `omaf import <package>` / `omaf external add <package>` | packages to and from an external disk, a NAS or a folder; a VM run from where it is (§11.10) |
| `omaf new <name> --recipe <f> [--ephemeral] [--user <u>]` | instance folder + overlay (NixOS) or APFS clone with new machine identifier (macOS) |
| `omaf start <name>` | start backend (window or headless); state can be reconstructed from files |
| `omaf stop <name>` | NixOS: QMP `system_powerdown`; macOS: `requestStop()` via the runner socket (with timeout); then hard stop |
| `omaf reset <name>` | delete the instance state, recreate it from the image |
| `omaf upgrade <name>` | switch instance to the current image of its recipe (old state deleted) |
| `omaf rm <name>` | delete the instance (share folder stays) |
| `omaf list` | instances of both guest kinds with status, recipe, image revision |
| `omaf ssh <name>` | NixOS: `ssh -p <port> <user>@127.0.0.1`; macOS: `ssh <user>@<IP from /var/db/dhcpd_leases>` |
| `omaf wait/log/images/config/doctor` | readiness, console, images (list, delete), settings, prerequisites (details: `docs/cli.md`) |

**Backend protocol** (plan §3, §7.3): `start`, `stop`, `status`, `create`, `reset`, `remove`, `ssh`.
`kind` in `instance.json` selects the backend.

### 6.3 QEMU invocation (NixOS, reference)

```sh
qemu-system-aarch64 \
  -machine virt,highmem=on -accel hvf -cpu host \
  -smp $CPUS -m ${MEM}G \
  -drive if=pflash,format=raw,readonly=on,file=$QEMU_SHARE/edk2-aarch64-code.fd \
  -drive if=pflash,format=raw,file=$INST/efivars.fd \
  -drive if=virtio,format=qcow2,file=$INST/disk.qcow2 \
  -device virtio-gpu-gl-pci -display cocoa,gl=es \
  -device qemu-xhci -device usb-kbd -device usb-tablet \
  -netdev user,id=n0,hostfwd=tcp:127.0.0.1:$SSH_PORT-:22 -device virtio-net-pci,netdev=n0 \
  -fsdev local,id=fs0,path=$SHARE,security_model=mapped-xattr \
  -device virtio-9p-pci,fsdev=fs0,mount_tag=share \
  -qmp unix:$INST/qmp.sock,server=on,wait=off \
  -serial file:$INST/serial.log
# ephemeral: additionally -snapshot
```

Minimal boot probe with these arguments: `scripts/bootprobe.sh`.

On a **Linux host** (plan B124, `qemu_command::Target`) the same list with the host's machine, accelerator, window and
sound – everything else (disk, network, share, agent ports, QMP, serial log, pid file) is word for word the same:

| | x86_64 host | aarch64 host |
|---|---|---|
| binary | `qemu-system-x86_64` | `qemu-system-aarch64` |
| `-machine` | `q35` (plus `-vga none`) | `virt,highmem=on,gic-version=host` (KVM) / `gic-version=max` (TCG) |
| `-accel` / `-cpu` | `kvm` / `host` when `/dev/kvm` opens read-write, else `tcg` / `max` | same |
| firmware | `edk2-x86_64-code.fd`; the VM's `efivars.fd` is a copy of `edk2-i386-vars.fd` | `edk2-aarch64-code.fd`, blank 64 MiB store |
| window (2D / 3D) | `virtio-vga` / `virtio-vga-gl`, `-display gtk[,gl=on],zoom-to-fit=on` | `virtio-gpu-pci` / `virtio-gpu-gl-pci`, GTK |
| sound | `-audiodev pipewire` (socket `$XDG_RUNTIME_DIR/pipewire-0`), else `pa`, else no sound device | same |
| share | `security_model=mapped-xattr` (the guest-owner patch is the Mac build's) | same |

Firmware: next to the binary (`<prefix>/share/qemu/`, as nixpkgs' `qemu` ships it), else QEMU's firmware descriptors
(`/etc/qemu/firmware`, `/usr/share/qemu/firmware`; UEFI, split flash, no Secure Boot/SMM). The accelerator a VM was
started with is kept in its run folder (`accel`) and shown with the VM. No omaf-runner bridge on Linux (agent report,
clipboard, camera – OPEN), so readiness is SSH there. Verified with `-accel help`, `-machine help`, `-device help`,
`-display help`, `-audiodev help` of nixpkgs' QEMU 11.1.2 (aarch64-linux).

Network modes from `recipe.toml`:

| Mode | NixOS (QEMU) | macOS (omaf-runner) |
|---|---|---|
| `shared` | `-netdev user` with SSH forwarding | `VZNATNetworkDeviceAttachment` (`192.168.64.x`) |
| `isolated` | `-netdev user,restrict=on` + SSH forwarding | **not available** (M10, plan BM27): NAT has no outbound block; a vmnet host-mode network needs a restricted entitlement – `omaf new`/`start` refuse it. Later: packet filter or `VZFileHandleNetworkDeviceAttachment` (OPEN) |
| `none` | no network card | no network card (SSH not possible then) |

### 6.4 Runner `omaf-runner` (macOS, to be verified in M7)

One process per running macOS VM (a `VZVirtualMachine` lives only as long as its process).
Started like QEMU via `posix_spawn` + `setsid`, so that app and CLI do not take it down with them.

| Building block | Virtualization.framework |
|---|---|
| Platform | `VZMacPlatformConfiguration` (`hardwareModel`, `machineIdentifier`, `auxiliaryStorage`) |
| Disk | `VZVirtioBlockDeviceConfiguration` + `VZDiskImageStorageDeviceAttachment` (RAW) |
| Graphics, input | `VZMacGraphicsDeviceConfiguration`, `VZMacKeyboardConfiguration`, pointing device; window with `VZVirtualMachineView` |
| Network | `VZVirtioNetworkDeviceConfiguration` + `VZNATNetworkDeviceAttachment`, fixed MAC per instance |
| Share | `VZVirtioFileSystemDeviceConfiguration` + `VZSingleDirectoryShare` (macOS guest: automount, OPEN) |
| Installation | `VZMacOSRestoreImage` (local or `latestSupported`) + `VZMacOSInstaller` |
| Control | Unix socket `runner.sock`: `status`, `stop` (`requestStop()`), `kill` (`stop()`), `show` (bring window to front) |
| Signing | entitlement `com.apple.security.virtualization` (required per SDK header, plan BM8) |

```sh
omaf-runner run   <vm-dir> [--window] --socket <instance>/runner.sock   # blocks until the VM is off
omaf-runner install <vm-dir> --ipsw <file> --disk-gb 80                 # fresh macOS installation
```

### 6.5 macOS image pipeline

1. **Installation (automatic, `omaf macos install`):** download IPSW from Apple or use an existing file,
   install into a new VM folder with `VZMacOSInstaller`.
2. **Base (automatic, once per macOS version, plan BM29):** clone of the fresh installation, first boot headless
   with provisioning (guest user, automatic login, SSH), the Mac user's SSH public keys into `authorized_keys`,
   shutdown, read-only → `bases/mac-base-<os-build>/`. **No Apple ID**, no license sign-ins.
   omaflake distributes no macOS images: the IPSW always comes from Apple and the base is built on the user's Mac
   (whether Apple's license would allow distributing a prepared base: OPEN, not pursued).
3. **Image (automatic, `omaf build`):** clone the base, start headless,
   install Nix via SSH (only if not already in the base), **build** the closure of `darwinConfigurations.<id>`
   **on the Mac** and transfer it with `nix copy`, activate in the guest, shut down, store as image.
4. **Optional finishing (manual, per recipe):** permissions (TCC) for installed apps that can only be granted by hand.
5. **Instances:** APFS clones; Apple ID, Microsoft 365 sign-in etc. per instance.

### 6.6 Menu bar app (SwiftUI, `MenuBarExtra`)

Planned (plan B119): a setting "Show omaflake in: menu bar / window / both" – the same pages in a regular app window
with a Dock icon. Linux gets its own app (Rust, iced) in the same brand look: a window, a tray icon over
StatusNotifierItem and a waybar module, all coming with omaflake.
Planned (plan B121): the core also says what each page shows (a presentation model); SwiftUI, iced and a terminal
interface (`omaf ui`, ratatui, macOS and Linux) only draw it.

- List of instances of both guest kinds with status dot and guest-kind icon; click = start or bring window to front
- "New VM…" (recipes grouped by guest kind, name, ephemeral yes/no), "Reset", "Stop", "Rebuild image" (with log view)
- Limit of 2 running macOS VMs: check before starting and show clearly (instead of passing the error through)
- omaflake icon everywhere: menu bar (template image), app bundle (`omaflake.icns`), Dock icon of `omaf-runner` and of QEMU windows (plan B27)
- Signing: locally ad-hoc; entitlements only as needed (QEMU and `omaf-runner` run as separate processes; only `omaf-runner` needs `com.apple.security.virtualization`)

## 7. Milestones & acceptance criteria

| # | Content | Done when … |
|---|---|---|
| M0 | Repo foundation | ✅ 2026-10-16: Git repo, `flake.lock` committed, `nix flake check` green |
| M1 | Image build | `nix build .#images.dev` produces a UEFI qcow2 that boots with the QEMU reference invocation (headless, `scripts/bootprobe.sh`); labels match `vm-guest.nix`. UTM boot dropped (option A). ✅ 2026-10-16 (`minimal` and `dev`) |
| M2 | QEMU with graphics acceleration | QEMU on the Mac starts an image with `virtio-gpu-gl-pci`; Hyprland runs smoothly. ✅ 2026-10-16: own build (`qemu/`), `dev` with virgl/ANGLE, confirmed smooth by the maintainer |
| M3 | CLI MVP | `omaf build/new/start/stop/reset/rm/list/ssh` work; ephemeral instance is clean after restart. ✅ 2026-10-16 (full cycle on image `minimal`, plan §4) |
| M4 | App | Menu bar app can do everything from M3 by click. ✅ 2026-10-16 (app `omaflake`, accepted by click) |
| M5 | Graphics backend production-ready | plan §4 |
| M6 | Comfort (NixOS) | clipboard Mac↔VM, dynamic resolution, optional camera bridge |
| M7 | Runner `omaf-runner` | `omaf-runner install` installs macOS from an IPSW into a new VM folder; `omaf-runner run` starts it with window and headless; control via socket (status/stop); ad-hoc signed with `com.apple.security.virtualization`. ✅ 2026-10-16 except headless `stop` (confirmation dialog in the guest, plan BM17 → SSH shutdown in M9/M10) |
| M8 | macOS base | `bases/mac-base-<os-build>/` created per `docs/macos-base.md`; reachable via SSH with key as the guest user; checklist fully ticked. ✅ 2026-10-16 (development base `mac-base-26A434`, plan §7) |
| M9 | macOS image by script | the build script for `mac-minimal` produces `images/mac-minimal-<rev>/` without manual intervention; a clone shows Dock, apps, defaults and wallpaper as declared; then `mac-office`, `mac-dev`. ✅ 2026-10-16 for `mac-minimal` (BM18–BM26); `mac-office`, `mac-dev` open |
| M10 | `VZBackend` in OmaflakeCore + CLI | `omaf macos install`, `omaf build/new/start/stop/reset/rm/list/ssh` for `mac-minimal`; ephemeral instance demonstrably clean; `omaf list` shows both guest kinds. ✅ 2026-10-16 for `new/start/stop/reset/rm/list/ssh` incl. ephemeral and the 2-VM limit (plan §7); `omaf macos install` and `omaf build` for macOS ✅ 2026-10-16 (plan BM28) |
| M11 | App for both guest kinds | app can do everything from M10 by click; limit of 2 macOS VMs is shown before starting |
| M12 | Recipe interface | recipes are read only via the defined flake outputs (§11.1); no user-specific paths/names in the Swift code |

## 8. Open items (OPEN – clarify before implementation)

> **Review 2026-10-16** (details, evidence and revised milestones: plan §1.2, §4, §7):
> NixOS track M0–M4 done. macOS: nix-darwin options for Dock/apps checked (BM1); tart replaced, Virtualization.framework API checked against the macOS 27 SDK (BM8–BM10).

- **Linux builder:** a nix-darwin Linux builder is required to build NixOS images on the Mac; see `docs/linux-builder.md`.
- **M2 QEMU build:** DECIDED 2026-10-16 – own build with virglrenderer + ANGLE (plan B6). Template github.com/omacom/try-omarchy (MIT license, observe its license notices).
- **UEFI firmware:** located per QEMU at `<prefix>/share/qemu/edk2-aarch64-code.fd`; the CLI derives the path from the binary.
- **Clipboard (NixOS):** spice-vdagent is X11-oriented; under Hyprland possibly an own solution is needed (template: try-omarchy).
- **Camera bridge:** DECIDED (2026-10-17) for NixOS – as try-omarchy: AVFoundation on the Mac → virtio-serial → v4l2loopback in the guest, on demand (plan B37). macOS guests: the same Mac side over a virtio console port, ready (plan B49); in the guest a CoreMediaIO camera extension after M12.
- **Image retention:** How many old image revisions are kept? (both guest kinds)
- **macOS – wallpaper:** no nix-darwin option (BM1) → script (`desktoppr` or `osascript`) in the GUI session (LaunchAgent). Which approach works on macOS 27: OPEN (M8).
- **macOS – widgets:** DECIDED (2026-10-16): off in all macOS VMs – widgets are used on the host. Implemented in the darwin guest module: `system.defaults.WindowManager.StandardHideWidgets = true` and `StageManagerHideWidgets = true` (M8). Content/arrangement is not declarable (`WidgetCenter` can only query/reload its own widgets, plan BM1) and is therefore dropped.
- **macOS – ad-hoc signature:** sufficient locally (plan BM12).
- **macOS – clipboard:** `VZSpiceAgentPortAttachment.sharesClipboard` exists, but needs a SPICE agent in the guest; for macOS guests an own small agent (e.g. over virtio-vsock) is needed: OPEN (after M11).
- **macOS – ephemeral:** DECIDED (M10, plan BM27): fresh APFS clone at every start, the previous run's clone is deleted. Only changed blocks take space.
- **macOS – limit of 2 VMs:** resolved – the framework enforces it, error `VZErrorDomain 6` (plan BM16).
- **macOS – shutdown:** `requestStop()` triggers a confirmation dialog when a user is logged in (plan BM17) → `omaf stop`: SSH `sudo -n /sbin/shutdown -h now` → runner `stop` (without network) → runner `kill` → SIGKILL (implemented in M10).
- **macOS – IP address:** from `/var/db/dhcpd_leases` via the MAC – verified (M7).
- **macOS – credentials:** own installation → the guest user is created at first boot by provisioning (no Setup Assistant); SSH key-only: DECISION in M8.
- **macOS – Cirrus Labs image `gg-vanilla`:** only for experiments; not the foundation of the base (license/provenance unclear, §11).

## 9. Rules & constraints

- **Deleting (DECIDED 2026-10-17):** what the user removes – instances, their old state on reset/upgrade, images,
  failed builds – omaflake deletes for real (the app asks first and says it cannot be undone); share folders are never
  deleted. "Never delete, move to `_to_delete/`" is a rule for developers' and agents' own actions (CLAUDE.md).
- **Verify before claiming:** check Nix options, QEMU parameters and Virtualization.framework API against source/SDK headers; mark anything unverified as OPEN.
- **Open-source ready:** no dependency that cannot be published as open source or obtained freely by the user (§11.2).
- No credentials in the repo. Initial password `changeme` (NixOS) only until first login.
- **macOS license:** at most 2 concurrently running macOS VMs per Mac.
- **No Apple ID and no license sign-ins in base or images** – only in instances.

## 10. Risks

| Risk | Mitigation |
|---|---|
| QEMU build with graphics on macOS is laborious | clarify early in M2; fallback UTM |
| nixpkgs-unstable breaks options | commit `flake.lock`, update deliberately |
| Disk usage from images | overlays/APFS clones instead of copies; retention rule (OPEN) |
| Hyprland unstable with virgl | blur/shadows off; fallback lighter compositor |
| macOS update makes the base unusable | rebuild the base per macOS version with `omaf macos install`; rebuild images from it |
| Base drifts / is not reproducible | built by `omaf macos install` only (BM29); nothing is done by hand |
| Own runner is extra work (installation, network, window) | Apple's sample code as template; Lume (trycua) only after license check (OPEN); do not adopt tart code (FSL) |
| Virtualization.framework changes with macOS versions | check availability with `@available`; define a minimum version (plan §5) |

## 11. Open-source release

Goal: others define their own recipes in their own flake; the app remains a pure macOS app
for NixOS and macOS guests. Order: first finish the core feature set (M11), then the interface (M12), then release.

### 11.1 Recipe interface (DECIDED 2026-10-18, plan B105/B107)

A recipe is not a flake but a module with its metadata: a folder `<author>/<kind>/<name>/` with `recipe.toml` and
`default.nix` (format: omaflake/recipes, CONTRIBUTING.md; `recipe.toml` (`"$schema"` recipe/v1, §5.1) names the architectures it
is built for in `systems`, a `<system>.nix` next to `default.nix` is its variant for one of them; `catalog.json` schema 2 lists a
source's recipes). omaflake
builds them with its own **image flake** (`nix/`, shipped in the app), which takes the recipes as its input
`omaflake-recipes` and gives

```nix
images.<system>."<author>.<kind>.<name>@<words>"       # kind nixos: the UEFI qcow2 (aarch64-linux, x86_64-linux)
darwinConfigurations."<author>.<kind>.<name>@<words>"  # kind macos: activated on a clone of the base
lib.recipeDefaults."<author>.<kind>.<name>@<words>"    # each recipe.toml plus author, kind, name
```

(Staged by omaflake with `recipes.json`, §11.11; a plain folder of `<author>/<kind>/<name>/` recipes – a checkout for
`nix flake check` – gives the IDs without `@<words>`.)

Another image flake (`omaf config --flake …`) works if it has the same inputs (`omaflake-guest`, `omaflake-recipes`)
and outputs.

**Person-specific values never live in a recipe (DECIDED and implemented 2026-10-17).** User, full name, e-mail, time
zone, locale, keyboard, SSH public keys and the macOS guest user come from omaflake: the image flake declares the input
`omaflake-guest` (a folder with `settings.json`, flake = false, neutral defaults in the repository); omaflake replaces
it on every `nix` call with `<data folder>/guest` (`--override-input … --no-write-lock-file`, so the lock file keeps
only the relative default). Sources: the Mac (user, full name, time zone, locale, keyboard from the HIToolbox
preferences, `~/.ssh/*.pub`), the macOS base (its guest user – images always match their base), and the overrides in
the app's settings / `omaf config --guest-…`. A developer's `local/settings.nix` still overrides single values on top;
the Doctor flags a macOS user that then differs from the base's. Flakes without the input keep working (nothing is
passed). With preset recipes (§11.3) a user therefore needs no settings file at all; later generic images move the same
values to the first start of an instance.

### 11.2 What the user brings (DECIDED 2026-10-16)

omaflake ships only itself (app, `omaf`, `omaf-runner`). The user's Mac brings:

| Prerequisite | Why | Without it |
|---|---|---|
| Apple silicon, macOS 27 | Virtualization.framework, guest provisioning for the automatic macOS base (BM11) | – |
| Nix with flakes | recipes are evaluated and all images are built with Nix | nothing can be built |
| nix-darwin with `nix.linux-builder` | Linux builder for NixOS images; required so that every feature works | – |
| QEMU (NixOS guests) | runtime of NixOS guests; 3D needs omaflake's patched build | Homebrew/nixpkgs QEMU: headless and 2D only |
| SSH key in `~/.ssh` | `omaf ssh`, image builds, base provisioning | – |

QEMU is **not** shipped in the app (DECIDED): omaflake's patched build (virglrenderer + ANGLE, `qemu/`) is a Nix
package of the omaflake flake (`packages.aarch64-darwin.qemu`, `qemu/package.nix`), installable with one command or
one line in the nix-darwin configuration; a binary cache so nobody compiles it: OPEN. Implemented: `Prerequisites`
(OmaflakeCore), `omaf doctor`, the app's Requirements page. The app moderates all prerequisites on one page:
detect existing installations (configured path, PATH, Nix profile, nix-darwin system, Homebrew), show what they can do
(e.g. 3D), let the user enter a path by hand, and offer "Install" – it opens Terminal with the official command, so the
user sees what runs and enters the admin password there. Superseded for admin tasks by §11.5 (2026-10-17): the app may
run them itself through macOS' administrator dialog.

### 11.3 Recipe sources, presets, marketplace (DECIDED and implemented 2026-10-18, plan B107)

omaflake ships **no recipes**. It reads them from recipe sources:

- **Kinds:** a GitHub repository (`https://github.com/owner/repo`, `owner/repo`, `…/tree/<branch>`; default branch
  `main`), the HTTPS URL of a `catalog.json` (any static host), or a **local folder** (a checkout with
  `recipes/<author>/<kind>/<name>/`, or a folder of such folders). Kept in config.json (`sources`, in order); none set
  = omaflake/recipes on GitHub. Settings → Recipe Sources and `omaf sources` add, remove, reorder and switch them.
- **Private GitHub repositories:** "Private repository" next to the GitHub address shows a token field – a fine-grained
  token with Contents: Read-only on that repository. omaflake checks it (the repository over the GitHub API, then the
  catalog) before it keeps the source, stores it in the login Keychain (service `app.omaflake.recipe-source`, never in
  a file) and reads the repository over the GitHub API (`contents/…`, raw) with it. Public ones are read from
  raw.githubusercontent.com without a token. Other private hosts: OPEN.
- **Remote sources** are read only when asked – opening the Recipes tab, its refresh button, `omaf recipes
  available`/`install` – never in the background (the daily update check stays the only network access omaflake makes
  on its own, §11.6). Their `catalog.json` lists every recipe with metadata, a `version` and each file with size and
  SHA-256. **Installing** downloads a recipe and those it `requires` into `<data folder>/recipes/<author>/<kind>/<name>/`
  (`.installed.json`: source, version, requires), checking every file's size and SHA-256 and treating the catalog as
  untrusted (paths must stay inside the recipe's folder; at most 1 MiB per file, 5 MiB per recipe); a mismatch installs
  nothing. A newer `version` in the catalog shows as an update. **Removing** deletes the folder (refused while another
  installed recipe requires it); images built from it stay.
- **Local folders** need no catalog: omaflake uses their recipes as they are on disk, so a changed file is built the
  next time – how recipes are written and tested. During omaflake's own development the data folder's source is the
  local checkout of omaflake/recipes (CLAUDE.md), not the remote repository.
- **Into the build:** before every `nix` call omaflake puts the recipes together – the enabled local folders first (in
  their order), then the downloaded ones; the first with an ID wins – into `<data folder>/cache/recipes/<content
  hash>/` and passes it as `--override-input omaflake-recipes path:…` (the same content gives the same folder; the
  last 8 are kept). An image's revision is its recipe's: git revision of a local folder (`-dirty` with changes in the
  recipe's folder), the catalog version of a downloaded one.
- **Presets and the first start:** omaflake's recipes live in [omaflake/recipes](https://github.com/omaflake/recipes)
  (also `lab`). With nothing installed, the Recipes tab offers "Install suggested": the official ones (no lab recipes).
  **Official** means by `omaflake` *and* from omaflake/recipes on GitHub – a fork's copy or an author name alone is not;
  recipes from other sources carry a note that the omaflake team has not reviewed them.
- **Marketplace:** omaflake/recipes is the curated catalog: other developers submit recipes by pull request; the core
  team reviews them for security, fitness and added value (REVIEW.md there), CI checks the format (`tools/catalog.py
  --check`); evaluating and building each submitted recipe in CI: OPEN (needs omaflake/omaflake public or a token).
  Tags come from one list (`tags.toml`), so the Recipes tab can filter by them. Anyone can run a source of their own
  (FORKING.md there). Export (a recipe's folder as a file) and signing or pinning a catalog: OPEN.
- **Generic images:** nothing personal in an image; user, SSH keys, language and keyboard reach the instance at its
  first start (NixOS: e.g. systemd credentials via SMBIOS – OPEN; macOS: over SSH, as the host name today). Generic
  NixOS images from a public flake can then come from a binary cache instead of being built.
- **macOS stays local:** IPSW from Apple, base and images are always built on the user's Mac.
- **The base always works:** `omaf macos install` needs no flake and no recipe – a macOS base can be created on any
  Mac that meets §11.2, whatever recipes are installed.

### 11.5 Admin tasks from the app (DECIDED 2026-10-17)

Installing Nix (APFS volume `/nix`, `/etc/synthetic.conf`, `nixbld` users, nix-daemon), activating nix-darwin and
setting up or changing the Linux builder (a launchd system daemon whose SSH key only root can read) all need root.
Everyday work (QEMU from Nix, macOS base, images, VMs) does not.

- **Now:** the app runs admin tasks itself with macOS' standard administrator dialog (AppleScript `do shell script …
  with administrator privileges`, `AdminShell` in OmaflakeCore; the command is passed as an argument, never spliced into
  script source). Every task is confirmed in the app first and logs to `<data>/logs/`. The CLI runs the same scripts
  through `sudo`. Implemented: Doctor → Linux builder → "Apply Settings …" (build `…#linux-builder` as the user, then
  set the system profile and activate as root – root may not read `~/Documents`; warns when the running nix-darwin system is not omaflake's – marker `/etc/omaflake/linux-builder`)
  and "Clean Up …" (`nix-store --gc` in the builder); `omaf builder apply|clean-up`. Whether the dialog offers Touch ID
  on macOS 27: OPEN.
- **Implemented 2026-10-17 (plan B54), to try on a Mac without Nix:** Nix with the NixOS community's official
  installer, nix-darwin with the builder host in two stages. Earlier plan text:
- **Planned (milestone before release, with M12):** set up the prerequisites by click – Nix, nix-darwin with the
  builder host – from the Doctor, each through the administrator dialog, so a Mac without Nix gets from "download the
  app" to "first VM" without Terminal. To clarify first (OPEN): which installer (nixos.org multi-user script or
  Determinate Systems' `.pkg`), whether Determinate Nix and nix-darwin both managing `/etc/nix/nix.conf` fit together,
  pinning installer versions/checksums, detecting and leaving alone an existing Nix or nix-darwin configuration.
- **After Developer ID signing (M12):** a privileged helper (`SMAppService` launch daemon in the app bundle) replaces
  the dialog: the user approves it once (System Settings → General → Login Items), afterwards admin tasks run without
  a prompt. The helper accepts requests only from the signed omaflake app and only a fixed list of tasks; the app
  still confirms big steps itself.

### 11.7 Recipe line-up (DIRECTION 2026-10-17, shaped last – once everything runs)

- **Standard recipes**, always there, for both guest kinds: `base`, `minimal`, `lab`. What `base` holds as a recipe
  (vs. `minimal`): OPEN, to shape at the end.
- **lab = development recipes:** omaflake's own (test app, debug tools) and any recipe marked `lab = true` by its
  author; the app shows them only in **developer mode** (Settings; `config.json` `developerMode`; `omaf recipes --all`).
  In the app a flask marks them. Implemented 2026-10-17 (plan B51).
- **Custom recipes by omaflake:** `research` first (NixOS and macOS); `dev` (Development) and `office` are planned for
  later (taken out 2026-10-17 to perfect research and the app first) – the first examples of what others can
  publish (§11.3).
- Every recipe has a **description** (`description`, `description_<language>` in recipe.toml), shown behind an "i"
  next to its name and below the pickers. Implemented 2026-10-17.

### 11.8 Hosting (DIRECTION 2026-10-17: own mirrors run by the maintainer)

| What | Why | Where (suggestions; check current terms) |
|---|---|---|
| App releases (signed, notarized .dmg/.zip) + checksums | download, Homebrew cask, self-update | GitHub Releases (primary); mirror: Cloudflare R2 (S3-compatible, no egress fees) or Hetzner Object Storage (EU) |
| Update feed (appcast, signed) | self-update (§11.6) | static file on the website (GitHub Pages) or the same bucket |
| Nix binary cache: `qemu-omaflake` with virglrenderer/libepoxy (aarch64-darwin) | nobody compiles QEMU (~20 min) | Cachix (hosted, free for open source); self-hosted: a plain S3 binary cache (`nix copy --to s3://…`, public bucket on R2/Hetzner/B2, no server) or Attic (server + S3) |
| Nix binary cache: omaflake's own guest parts (agents, test app, recipe packages) for aarch64-linux and aarch64-darwin | faster image builds | same cache; NixOS images themselves are personal (user, keys) and do not cache – generic images later (§11.3) |
| Pinned nix-installer binary + checksum | setup by click without a moving target (B54) | same bucket; source: NixOS/nix-installer releases |
| Website, docs, appcast | – | GitHub Pages (omaflake/website), domain via any registrar (e.g. INWX, Hetzner), DNS e.g. Cloudflare |
| Homebrew tap (cask) | `brew install --cask omaflake` | GitHub repository `omaflake/homebrew-tap` |
| Recipe source omaflake/recipes (§11.3) | presets and the curated catalog | GitHub repository; optional mirror on Codeberg (a source can be any HTTPS host) |
| CI builders | build and sign releases, fill the cache | GitHub Actions: macOS arm64 runners (app, QEMU), Linux arm64 runners (guest parts) – free for public repositories (check) |

**Not to mirror:** Apple's IPSW restore images (Apple's licence – the app downloads them from Apple), nixpkgs
(`cache.nixos.org` already serves it). The cache needs a signing key (`nix key generate-secret`); its public key and
URL go into the builder host's `nix.settings` (`extra-substituters`, `extra-trusted-public-keys`), so setup by click
configures it.

### 11.6 Self-update (DECIDED 2026-10-17, implementation with M12)

omaflake updates itself when a new release exists. The Doctor shows the running version and the update check against the release's `update.json` (implemented, plan B68) and, with
releases, the newest one with "Update" and a switch for automatic updates. Everything else works offline; the update
check is the only network access the app makes on its own, and it can be switched off. Needs signed, notarized
releases (M12); mechanism (Sparkle with signed appcast, or GitHub releases plus signature check of our own): OPEN.
Updating the app never touches the data folder; migrations of `~/.omaflake` run on the first start of the new version
(as `ImageStore.migrateLegacy` today).

### 11.9 omaflake agent in every guest (DIRECTION 2026-10-18, design and NixOS prototype 2026-10-18, plan B96; clipboard and camera for NixOS, macOS groundwork: plan B106)

Before it, three single-purpose helpers talked to the Mac, each over its own virtio port: the clipboard helper for
NixOS and for macOS and the camera helper for NixOS (Python, ports `org.omaflake.clipboard` and
`org.omaflake.camera`); readiness and the macOS host name went over SSH. NixOS images from B106 on use the agent;
macOS guests still use their clipboard helper (`nix/modules/clipboard-agent-macos.py`). Direction: **one omaflake agent**, part of every
image (the guest modules `vm-guest.nix` and `darwin-guest.nix`, so no recipe can leave it out), talking to omaflake on
the Mac over one channel.

- **Transport:** a virtio port, as today – omaflake's QEMU has no vsock device (`-device help` lists only
  `virtconsole`/`virtserialport`), Virtualization.framework has both (`VZVirtioSocketDevice`). One port with a framed,
  versioned protocol (hello with agent and protocol version, then messages per feature); the Mac side lives in
  `omaf-runner` (macOS) and in omaflake next to QEMU (NixOS). On a Linux host the same host side is `omaf bridge`
  (Rust, the core's `AgentLink`; host clipboard and V4L2 camera – plan B126).
- **What it carries:** the existing clipboard and camera streams; readiness without SSH (the agent's hello = "VM ready",
  also with network off); live status (load, memory, disk, addresses, logged-in user); host name and guest settings at
  first start (fits the generic images of §11.3); a clean shutdown request; later forwarding of ports ("tunnels")
  started from the Mac, opening a URL on the Mac, notifications from the guest.
- **macOS camera:** the agent is the transport only. A camera inside a macOS guest still needs a Camera Extension
  (CoreMediaIO system extension) there, signed and approved by the user – that stays with M12 and the signed release.
- **Trust:** the guest is untrusted. The Mac side accepts only the defined messages, everything the guest may cause on
  the Mac is switched per VM in the app (as clipboard and camera today), and tunnels are started from the Mac only –
  nothing in the guest can run commands on the Mac.
- **Versions:** an image carries the agent it was built with; the Mac side speaks the current and the previous protocol
  version, the app shows when a VM's agent is older than that.

**Design (DECIDED 2026-10-18; prototype for NixOS: hello and status, plan B96)**

- **Language: Rust in the guest, Swift on the Mac.** Checked against the image flake's nixpkgs (2026-10-18): Rust
  1.98.1 is in cache.nixos.org for aarch64-linux; the prototype `omaflake-agent` is a 534 KB binary whose closure is
  glibc and libgcc, which every NixOS system has anyway. nixpkgs' Swift 6.2.4 builds for aarch64-linux too, but it is
  one output (`out`, plus `doc`/`man`) with a 5.4 GB closure (corelibs Foundation alone 186 MB) – a dynamically
  linked agent would drag the toolchain into every image; linking the Swift runtime statically under nixpkgs was not
  tried (OPEN, not pursued). Go would fit as well (small static binary); Rust because it has no garbage collector
  pausing the camera stream and reaches the macOS pasteboard through the `objc2` crates (macOS port: OPEN). The cost –
  the protocol exists twice – is held in check by shared test vectors (`nix/agent/testdata/vectors.json`), checked
  by `cargo test` (runs in the Nix build) and `swift test`.
- **One port, multiplexed:** every feature goes over the one port `org.omaflake.agent` (QEMU `virtserialport`;
  Virtualization.framework `VZVirtioConsolePortConfiguration`, like the clipboard port today). One device per VM, the
  version agreed once, features switched in the welcome instead of by attaching ports at start – so switches can take
  effect while the VM runs. What a single port costs, and the remedy: head-of-line blocking – large messages go in
  chunks of at most 256 KiB, the camera runs on credits (below), so a control message waits at most one frame
  (≈ 1.4 MB at ~41 MB/s, B70: about 35 ms); a per-VM switch is enforced by the Mac side dropping that feature's
  messages, no longer by a missing port. A second port for bulk data, offered in the welcome, only if measurements ask
  for it (OPEN). Not vsock: omaflake's QEMU 11.1.2 has none (`-device help`: `virtconsole`, `virtserialport`,
  `virtio-serial-*` only). Not qemu-guest-agent (`org.qemu.guest_agent.0`, in the images for QEMU's own use): QEMU only,
  commands polled by the host, no streams, nothing for macOS guests.
- **Wire protocol, version 1.** A frame is a 12-byte header and its payload, integers little endian:

  | Offset | Field | |
  |---|---|---|
  | 0 | magic | `OMFA` |
  | 4 | type | u16, table below |
  | 6 | flags | u16; bit 0 `MORE`: the payload continues in the next frame of the same type |
  | 8 | length | u32 payload bytes, at most 2 MiB (a 1280×720 NV12 frame is 1,382,400) |

  The header never changes (a different header would get a different magic); the protocol version is agreed per
  connection. Control payloads are JSON (UTF-8, camelCase keys); camera frames are binary. A receiver skips types it
  does not know by their length and drops bytes up to the next magic after a bad header.

  | Type | Direction | Message | |
  |---|---|---|---|
  | `0x0001` | guest → Mac | hello | agent version, protocol range `{min, max}`, OS, kernel, host name, boot id, features |
  | `0x0002` | Mac → guest | welcome | chosen protocol, enabled features, status interval, Mac side's name |
  | `0x0003` | guest → Mac | status | uptime, load, memory, disk, addresses, users with a session |
  | `0x0004` | guest → Mac | goodbye | the agent stops (shutdown, restart) |
  | `0x0010` | Mac → guest | shutdown | clean shutdown request (replaces ACPI/SSH, later) |
  | `0x0011` | Mac → guest | settings | host name and guest settings at first start (later, §11.3) |
  | `0x0100` | both | clipboard | `{"mime", "data"}` (text/plain;charset=utf-8 or image/png, base64), in `MORE` chunks of 256 KiB, at most 32 MiB of content |
  | `0x0200` | guest → Mac | camera control | `{"action": "start" \| "stop"}`: an application opened or closed `/dev/video42` |
  | `0x0201` | guest → Mac | camera credit | `{"credits": n}`: n more frames may come (the Mac caps them at 4) |
  | `0x0202` | Mac → guest | camera frame | one 1280×720 NV12 frame (1,382,400 bytes), always one frame, never chunked |
  | `0x0203` | Mac → guest | camera status | `{"status", "detail"}` (ready, streaming, unavailable, stopped) |
  | `0x03xx`, `0x04xx` | | tunnels; open URL, notifications | later |

  Implemented (B96, B106): `0x0001`–`0x0004`, `0x0100`, `0x0200`–`0x0203`. **Connection:** the guest says hello when the Mac side is there (Linux: the port's
  `POLLHUP` clears) and again every 5 s until a welcome comes; the Mac answers each hello with a welcome. Version: the
  highest both speak; with none in common there is no welcome, and the app shows why. **Flow control for the camera:**
  credits – the guest grants as many frames as it has buffers free (2) and returns one per frame written to
  v4l2loopback; the Mac sends a frame only against a credit, otherwise it keeps the newest one and sends it with the
  next credit (older ones are dropped – live video is never queued); no credit for 2 s stops the Mac camera. That bounds
  the Mac → guest queue at two frames.
- **Trust on the Mac side (the guest is untrusted):**
  1. Limits in the parser: 2 MiB per frame, resynchronisation after bad headers, an error budget (64 bad frames,
     malformed JSON or unknown types) – then the Mac side stops listening to that VM's agent and records why.
  2. Guest → Mac only the listed types, decoded into fixed structures; text loses control characters and is cut at
     256 characters, lists are capped, numbers must be finite and not negative. Nothing from the guest becomes a
     command, a path or a file name on the Mac.
  3. Rate limit per type (hello, status, goodbye: at most every 0.5 s); clipboard: at most 4 contents a second;
     camera credits capped at 4; a feature's messages are dropped unless the welcome enabled it.
  4. Effects: hello and status only update the instance's `agent.json` (shown by CLI and app). Anything else needs
     the VM's switch for that feature (clipboard writes the pasteboard, camera start turns on the Mac camera with the
     system's indicator); later, "open a URL" only for http/https and after a confirmation, notifications with the
     VM's name and rate-limited; tunnels only started from the Mac.
  5. Readiness and the reported host name are for display; no security decision rests on them (SSH keeps its own
     host keys).
  6. The Mac side runs as the user (NixOS: the `omaf-runner bridge` process next to QEMU; macOS: inside
     `omaf-runner`), without extra rights for the agent.
- **Guest side.** NixOS: the system service `omaflake-agent`, started by udev when its port appears, as root without
  capabilities and in systemd's sandbox; features that need root later (host name, shutdown) get exactly their
  capabilities. The clipboard needs the user's Wayland session: the session mode of the same binary
  (`omaflake-agent session`, a systemd user service) connects to the system agent over `/run/omaflake-agent/session.sock`
  (group `users`) – one owner of the port, local clients. The camera: the system agent itself drives
  `/dev/video42` (v4l2loopback). It writes what the welcome enabled to `/run/omaflake-agent/welcome.json` for the
  guest's own tools. macOS: the same binary builds for aarch64-darwin (status from sysctl, getloadavg, utmpx; port
  `/dev/cu.org.omaflake.agent`; runtime folder `/var/run/omaflake-agent`; session clipboard via pbcopy/pbpaste and the
  pasteboard's change count, text only – PNG OPEN). How it starts in the guest – a LaunchDaemon, which nix-darwin's
  launchd items announce as a background item in every clone (BM25), or over SSH like today's clipboard helper – is the
  maintainer's decision (OPEN); until then macOS guests keep the old clipboard helper.
- **Migration** in steps, each one working with old and new images:
  1. Agent beside the helpers – hello and status, NixOS VMs ready without SSH, `omaf agent` (done, B96).
  2. Clipboard into the agent (NixOS) – done (B106). The old ports stay attached for images built before; the Mac side
     serves the old port and switches to the agent when its welcome enables `clipboard` (such an image no longer runs
     the old helper).
  3. Camera into the agent (NixOS), with credits – done (B106).
  4. macOS guests: the Mac side is done (B106: `omaf-runner run --agent-state` attaches the port, `agent.json` is read by
     CLI and app, the clipboard moves to the agent when the welcome enables it); the agent builds for macOS. Open: how it
     starts in the guest (BM25, above), the host name over the agent (replaces `VZBackend.applyHostName`), PNG in the
     guest's pasteboard; the camera once the Camera Extension is signed (M12).
  5. The Python helpers and their ports go (during development: to `_to_delete/`) when images without the agent are no
     longer supported.
- **Update path:** the agent is part of the image, built by the same image flake; it never updates itself inside
  the guest (a guest fetching and running code from outside would break both reproducibility and the trust direction).
  A newer agent comes with a rebuilt image and `upgrade`. The Mac side speaks the current and the previous protocol
  version; the app shows "older than this omaflake – rebuild the image" when the agent's newest version is older than
  the current one, and the link stays off only when there is no version in common. New message types and new optional
  JSON fields do not raise the version (receivers skip and ignore them); changed meanings or removed fields do.

### 11.10 Portable packages: backup, external VMs, sharing (DECIDED and implemented 2026-10-18, plan B108, B109)

VMs, images and recipes leave omaflake and come back as **packages**: a folder with an extension that the Finder
shows as one file (registered by the app as document types, `com.apple.package`) – `<name>.omaflake-vm`,
`<name>.omaflake-image`, `<author>.<kind>.<name>.omaflake-recipe`. Inside: what the data folder keeps anyway, plus a
`manifest.json` (format, kind, name, time, omaflake version; informational – the content describes itself). No archive
format: copying a package is a backup, on the same APFS volume a clone (instant, no extra space); across volumes files
stay sparse (an 80 GB macOS disk takes on an external disk what it uses).

- **The data folder is the transport format.** Images and VMs are stored as packages in `images/` and `instances/`;
  NixOS images are self-contained (`disk.qcow2` copied out of the Nix store – no GC root, no Nix store needed to run
  them; about 7 GB more on disk until Nix collects the store path), and an overlay names its image by a path relative to
  its package. Runtime files live outside the package in `run/<name>/`. So the user can also copy packages in the
  Finder without app or CLI.
- **Who a package is for (policy):** a VM carries its user, SSH keys and data – it goes to the same person only (a
  backup, their other Mac, their external disk); an image carries the user it was built for – kept, not shared; **recipes are
  what people share**. Export dialogs say so.
- **Export** (`omaf export vm|image|recipe`, app: Export … in the details): a VM only when stopped (a snapshot). A NixOS
  VM always carries its image (`image.omaflake-image/` inside, the overlay rebased onto it), a macOS VM only when it
  needs it to start (ephemeral) or with `--with-image` (for a reset elsewhere). A recipe goes with the recipes it
  `requires` (`recipes/<author>/<kind>/<name>/`).
- **Import is always a copy into the data folder** (plan B109; `omaf import`, app: the Import button of each tab,
  double-click, drop on the app). Nothing is moved, the source is never changed; everything is assembled in a hidden
  `.partial` folder and moved into place at the end. By extension; a plain folder with recipes
  (`<author>/<kind>/<name>/recipe.toml`, a checkout, one recipe folder) imports as recipes. The tab's open panel offers
  only its kind and checks the content before it closes (a VM needs `instance.json`, an image `image.json` and its
  disk, recipes `recipe.toml`). What is here already is asked about first:
  - **VM** of the same name: cancel ("I have it already") or a new name – then it is a second VM with its own identity
    (SSH port; macOS machine identifier and MAC through `omaf-runner clone`). Under its own name it is the same VM
    again (a backup coming back). The image it carries becomes an image of the data folder unless that one is there.
  - **Image** of the same name – the same content, names carry the store hash: cancel or replace (refused while a VM
    on it runs).
  - **Recipes** never conflict (§11.11, plan B110): the same author, kind, name and content is here already – nothing
    to import; other content is a recipe of its own next to it (source `import`), nothing is replaced.
  Remote recipe sources are set in Settings; images are never used from elsewhere (an image elsewhere makes no sense –
  a VM carries the one it needs).
- **External VMs – run from where they are** (app: the second button of the VMs tab; `omaf external add`): a VM
  package on an external disk, a stick used on several Macs, a NAS or any folder is registered in `config.json`
  (`externalVMs`), nothing is copied. A NixOS VM must carry its image (an exported one does). On a network volume the
  app warns first (slower, breaks with the connection). While its place is there the VM is listed (marked with a
  drive, "Runs from" in its details); gone, it disappears. Stop Listing / `omaf external remove` drops the registration
  (also while the place is gone), Remove deletes the VM at its place (the dialog says so), Import copies it into the
  data folder and ends the registration. Sockets stay on the Mac (`run/<name>/`), so long volume paths do not hit the
  Unix socket limit (B12); the SSH port moves when the Mac uses it already. OPEN: two Macs using the same external VM
  at the same time is not prevented across machines (the lock is per Mac).
- **Migration:** at the first start of this version (app or CLI) image folders become packages (NixOS: the qcow2 is
  copied out of the store, the GC root goes), `current/` links follow, VM folders become packages with their runtime
  files moved to `run/` and their overlays rebased onto relative paths; skipped while a VM of the old layout runs.

### 11.11 omaflake IDs and recipe identity (DECIDED and implemented 2026-10-18, plan B110, B111)

One readable ID form for the three kinds of objects – `<kind>:<identifier>:<words>`:

| Kind | Example (as shown) | Identifier | Words from |
|---|---|---|---|
| recipe | `recipe:omaflake/nixos/minimal:regular` | author/kind/name | the content hash |
| image | `image:omaflake/nixos/lab:myth` | the recipe's author/kind/name | the store path (NixOS image, macOS: the darwin system) |
| vm | `vm:research:sister` | the VM's name | its identity: random when it is made, new for a copy under another name, kept by a restore |

**Twelve words, shown as few as needed (B111).** An ID has twelve words of BIP-39's English list (2048 words, MIT;
checked against its published SHA-256) – 132 bits, unique worldwide like a UUID, also for recipes and images shared
between people. The GUI and the CLI show only as many words as tell the objects of a kind apart: one word; as soon as two
objects of the same kind and identifier (two versions of one recipe, two images of one recipe) share their first word,
every object of that kind shows two words, and so on (`OmaflakeID.Lengths`). The full ID is in the tooltip and is
accepted everywhere, as is any start of its words (`@myth`, `image:omaflake/nixos/lab:myth`). The three-word IDs of the
first day are the start of the twelve-word ones (the same hash), so they keep working as short forms.
Files and Nix attributes use the full **file form** `<author>.<kind>.<name>@<twelve words>` – recipe IDs (`RecipeID`;
staged folders, `images."…"`, `image.json`, `instance.json`) and image package names (the revision is in image.json);
a VM's package keeps its name (`<name>.omaflake-vm`), its words are in instance.json.

**Recipe identity.** A recipe is its author, kind, name and content: the same base with other content is another
recipe, also from the same source; the same base with the same content exists once (the first source's). The source is
not part of the ID but of the recipe's **line** – source + author/kind/name: images of a line replace each other
(`current/<line>`, retention, "outdated" by store path), VMs upgrade within their line, and image.json records the
recipe source. **Versions (B111):** a line's versions are kept by the setting "Keep older recipe versions" (Settings,
`omaf config --keep-recipe-versions on|off`, default off): off, only the newest stays when a newer one comes (a catalog
update, an import, a changed local folder) – and **every version an image is built from, as long as that image is
there**. So that this holds for local folders too, a build keeps a copy of the version it used (and of what it
requires) in `<data>/recipes/<id>/` – a few KB; the folder can change afterwards. Versions are removed one by one or all
of a line at once (what images use and a local folder's own recipe stay). Content hash: SHA-256 over the recipe's sorted file paths and
each file's SHA-256 – the same from a catalog's file list (`RecipeCatalog.Recipe.contentHash`), so an offered recipe
that is here already (from any source) is recognised before it is downloaded.

**Shown only when needed.** The app and `omaf recipes` show a recipe line by its name, its source only when a recipe
of the same author, kind and name comes from another source; its versions are always sub-rows with their words, so
the words, export and remove always sit in the same place. Local folders count by their absolute path, imported recipes by where they are kept
(`<data>/recipes`). The details show each object's omaflake ID; `omaf list --long`, `omaf images`,
`omaf recipes --long` print them; recipe arguments accept the omaflake ID or `<…>@<words>`.

**Staging.** Every recipe gets a tree of its own, `<id>/<author>/<kind>/<name>/`, with the recipes it `requires` next to
it (same source first, else the first there is) – recipes import those by relative paths (`../minimal`), so each sees
its own; `recipes.json` maps ID → folder for the image flake. Downloaded and imported recipes live in
`<data>/recipes/<id>/`.

**Migration** (first start): downloaded recipes move to `recipes/<id>/`; images, `current/` links and VMs of old IDs
get the ID and source of the recipe of that base there is now; image packages are renamed to `<base>@<words>` (VMs on
them follow – their overlays are rebased); every VM gets an identity. Checked: the six recipes give the same store
paths as before.

### 11.4 Prerequisites for release

- **Repository:** `omaflake/omaflake` on GitHub (currently private; switch to public: OPEN) holds the public part: app, `OmaflakeCore`, CLI, `omaf-runner`,
  QEMU build script, flake module (`nixosModules.vm-guest`), the image flake `nix/` (used as `path:`, shipped in the
  app). The recipes live in `omaflake/recipes` (§11.3); SSH keys and user settings for development in each
  developer's git-ignored `nix/local/` (template `nix/local.example/`).
- **License (DECIDED 2026-10-19, plan B112):** **AGPL-3.0-or-later** for omaflake's own code (app, CLI, runner, agent,
  image flake, scripts – SPDX header in each source file); the recipes (`omaflake/recipes`) stay **MIT**, so recipes can
  be copied and adapted into any source; QEMU patches GPL-2.0-or-later like QEMU; try-omarchy and startergo patches
  MIT; the BIP-39 word list MIT. No App Store distribution (its terms conflict with the AGPL; the sandbox would not
  allow omaflake anyway). Images built by omaflake contain the guest agent (AGPL) – whoever passes images on passes
  its licence on.
- **Dependencies:**
  - tart (`FSL-1.1-ALv2`, BM2): **dropped** – own runner; do not copy tart code as a template.
  - Cirrus Labs images: **dropped** – base from Apple's IPSW (downloaded by the user, Apple license terms apply).
  - QEMU (GPL-2), virglrenderer, libepoxy, ANGLE: open source; not shipped in the app (§11.2), built from the flake's sources.
  - **ANGLE:** from nixpkgs, built from source, BSD-3-Clause (2026-10-17, plan B55) – the prebuilt bottle is gone.
  - **startergo patches** (virglrenderer macOS patch, libepoxy EGL patch): no LICENSE file, but his README and formulae state MIT (checked 2026-10-17) – used under MIT with attribution (`qemu/patches/startergo/NOTICE`). Nice to have: ask him to add a LICENSE file.
  - try-omarchy (MIT): carry over its notices.
- **Owner (DIRECTION 2026-10-19):** omaflake becomes a project of **falkOSS** (falkOSS – Falcon Lake Open Source
  Software gUG (haftungsbeschränkt) i. G., a non-profit, github.com/falkoss). The gUG signs the app as an Apple organization
  before the first public release, so the Team ID does not change after it (a later change means re-signing; users
  grant camera, microphone and the helper once more).
- **Signing/notarization (OPEN):** Developer ID, entitlements `com.apple.security.hypervisor` (QEMU) and `com.apple.security.virtualization` (`omaf-runner`); distribution e.g. via Homebrew cask.
- **User prerequisites (OPEN: document, check in the app):** Nix, nix-darwin with Linux builder (for NixOS images); nothing additional for macOS guests.
- **Name:** **omaflake** (2026-10-16), CLI **`omaf`**, runner `omaf-runner`.
  GitHub organization **`omaflake`**, repository **`omaflake/omaflake`**.
  Domain, package registries (npm, PyPI, crates.io, RubyGems, nixpkgs, Homebrew, App Store) and name collisions: OPEN.
  Trademark search (DPMA/EUIPO/USPTO): OPEN.
