---
title: Changelog
nav_order: 10
permalink: /changelog/
---

# Changelog

What changed in each release of omaflake – the app, the `omaf` CLI and the files they read and write. The format
follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/); versions follow [Semantic
Versioning](https://semver.org) (before 1.0, a new minor version may break compatibility). Changes not yet released
are collected under *Unreleased*. Releases: [GitHub](https://github.com/omaflake/omaflake/releases).

## [Unreleased]

### Fixed

- **3D windows on Linux distributions other than NixOS** (Ubuntu, Fedora, …): QEMU from Nix aborted with "Couldn't find
  current GLX or EGL context", because Nix programs find no graphics driver there. omaflake now fetches Mesa from
  QEMU's own Nix packages the first time (a minute, once) and starts the window with it; the VM's QEMU log says so.

## [0.3.1] – 2026-10-24

omaflake runs on Linux as a host: NixOS VMs with KVM on x86_64 and aarch64 machines, with the same clipboard, camera,
sound and agent as on a Mac. The Linux app and `omaf ui` speak German, and so do error messages everywhere. Still an
unsigned alpha – for trying on different hardware. Your VMs, images, recipes and settings stay as they are.

### Added

- **Private recipe sources on Linux:** the token of a private GitHub source is kept in the desktop's keyring (Secret
  Service: GNOME Keyring, KWallet, KeePassXC). Without one – a server, an SSH session – it goes to
  `~/.config/omaflake/tokens.json` (readable only by you, unencrypted), and adding the source says so. File schema
  `tokens` v1.
- **`omaf ui` and the Linux app speak German:** with a German locale (or `OMAFLAKE_LANGUAGE=de`) every page, action
  and question is shown in German – the same translations as the macOS app –, dates and sizes too.
- **German everywhere:** error messages are translated too – in the macOS app, the Linux app and `omaf ui` (a VM that
  is running, an image or recipe that is missing, a source, a package, a build, the Doctor; common system errors such
  as "No such file or directory"), and so are the Linux host's Doctor checks and the accelerator line. The API puts the
  message as a text next to its kind (`data.text`, a failed job's `errorText`). The CLI `omaf` stays English.
- **Clipboard, camera, sound and the agent for VMs on a Linux host** (docs/linux.md): the VM's switches work as on a
  Mac – text and images copied both ways (Wayland with wl-clipboard, X11 with xclip), the host's V4L2 camera while an
  app in the VM uses it, sound and the microphone through PipeWire or PulseAudio, and the agent's report ("VM ready"
  without SSH, `omaf agent`, the VM's details).
- **NixOS VMs on a Linux host** (issue #2, docs/linux.md): omaflake runs NixOS guests of the host's own architecture
  (x86_64 or aarch64) with QEMU and **KVM**; without a usable `/dev/kvm` it falls back to TCG – emulated, many times
  slower – and says so (`omaf start`, the VM's details, the Doctor). Images build natively with Nix, no Linux builder.
  Windows open with GTK (3D with virtio-gpu-gl where QEMU has it), sound goes to PipeWire or PulseAudio. The Doctor on
  Linux checks the host, KVM (with the fix: the group `kvm`, or virtualization in the firmware), Nix and flakes, QEMU
  with its version, 3D and UEFI firmware, the recipes, the SSH key and git – no nix-darwin, Linux builder or macOS
  items. x86_64 images boot with their console on the serial port too.

### Changed

- **The Linux app avoids a software GPU:** when Vulkan offers only Mesa's software rasterizer (lavapipe – e.g. in a VM
  with virgl), the app draws with OpenGL instead; `WGPU_BACKEND` is no longer needed for that.
- **A recipe name means the newest version:** after you change a recipe in a local folder, `omaf build lab` (and
  `omaf new`, `omaf export recipe`) takes the recipe as it is in the folder instead of asking between it and the copy
  kept from the last build. `omaf new` takes the newest version that has an image. Words or the omaflake ID still pick
  an older version; omaf asks only when recipes of that name come from different sources or authors.

### Fixed

- Images build again with a data folder that has no guest email set (`attribute 'email' missing` – omaflake leaves the
  field out then; the image flake now treats it as unset).
- `omaf start` right after `omaf stop` no longer says "SSH port … is in use": the check now binds the port the way QEMU
  does (with SO_REUSEADDR), so connections of the stopped VM still in TIME_WAIT do not count; a port another program
  listens on is still refused.

## [0.3.0] – 2026-10-24

omaflake's logic moved into one core written in Rust; the macOS app, the new Linux app, the terminal interface and the
`omaf` command all use it. Your VMs, images, recipes and settings stay as they are – every file is read as before.

### Added

- **omaflake on Linux: a native app** (`omaflake`, docs/linux.md): the same pages, rows, actions and questions as the
  macOS app, in omaflake's look. It opens files and folders with the desktop's apps, SSH in your terminal emulator,
  asks for files through the desktop portal and posts notifications. A tray icon (StatusNotifierItem: KDE, Cinnamon,
  XFCE, waybar) lists the VMs with start/stop and running builds; with it, closing the window keeps omaflake running.
  Nix: `nix run github:omaflake/omaflake#omaflake-gui`. Running VMs on a Linux host is not there yet (issue #2).
- **`omaf ui` – omaflake in the terminal** (macOS and Linux): VMs, images with running builds, recipes, activity, the
  build page with its live log, the Doctor, the macOS base, New VM and Settings – keyboard-driven, in the brand's colours
  (truecolor, 256 colours or none with `NO_COLOR`; Nerd Font symbols with `--nerd-font`). English only for now.
- **`omaf status [--waybar] [--follow]`**: running VMs and builds in one line, or as JSON for a waybar `custom` module
  that updates itself (snippet in the CLI docs).
- **omaflake in a window.** Settings → *Show omaflake in*: Menu Bar (as before), Window (a regular window with a Dock
  icon and the app menu) or Both. The change applies at once.
- **One core service for every interface.** omaflake's core runs once per user (`omaf serve --socket`): the app,
  `omaf ui`, `omaf build` and the Linux app share it, so a build started in the terminal shows up in the app and waits in
  the same queue. It starts on demand and ends five minutes after its last client, or runs as a user agent
  (`omaf service install|uninstall|start|stop|status`).
- **The `omaf` command from the Doctor.** A new, optional check says whether `omaf` is on your PATH and is this
  omaflake's. *Install …* links `/usr/local/bin/omaf` to the app's `omaf` (macOS asks for an administrator); on Linux
  the link goes to `~/.local/bin`. In the terminal: `omaf doctor --install-cli` / `--uninstall-cli`.
- **omaflake in the lab VM** (developer mode): the development recipe `nixos/lab` brings the Linux app, `omaf` and
  `omaf ui`, with omaflake's status and the tray in waybar – to try the Linux interfaces on a Mac (VMs cannot start
  inside it). Recipes can use omaflake as `pkgs.omaflake`.
- **The core API** (docs/api.md, on omaflake.dev): JSON-RPC over `omaf serve` – lists, actions, jobs, events and a
  description of every page – for scripts and other interfaces (`omaf call <method> [<params>]`).

### Changed

- **Editor lines for NixOS recipes.** The desktop no longer brings an editor; a recipe picks the neovim line
  (Neovim and Zellij, as before) or the new emacs line (`omaflake.editor = "emacs"`). The official recipes are named
  after their line: `nixos/minimal` is now `nixos/neovim-minimal`, `nixos/research` is `nixos/neovim-research`, and
  `nixos/emacs-minimal` is new. Images and VMs made from the old names stay as they are. The renamed recipes need
  omaflake 0.3.0.
- **The app and `omaf` run on the new core.** The app shows what the core says for every row, button, question and
  message; it no longer reads the data folder every 2 s itself. A macOS build asks the guest user's password, the macOS
  base its password (twice) and a private source its token in a question when you start them.
- **The Build Image page says how a build ended**: image ready (with "New VM …"), failed with its error, or cancelled;
  its Build button sits on the recipe's line. Finished builds in the Images list have a check mark.
- **Linux wording.** On Linux, omaflake speaks of "this computer", the file manager and the keyring instead of the
  Mac, Finder and the Keychain.
- Core messages (errors) are in English for now; the app's own texts stay translated.

### Fixed

- **macOS images build again with long recipe IDs**: the build VM's socket path was longer than a Unix socket allows
  ("Socket path too long").
- **Recipe versions that came within the same millisecond** no longer tie – the newest could be taken for an older one
  and removed.
- **omaflake's QEMU builds again** (`nix build github:omaflake/omaflake#qemu`): the libepoxy EGL 1.5 patch is now part
  of omaflake instead of a download from its Homebrew tap, whose release tag was moved. The patch itself is unchanged.

## [0.2.0] – 2026-10-22

Alpha, unsigned. **Update from 0.1.0:** omaflake 0.1.0 cannot read the new official recipe catalog. Your images, VMs
and settings are brought up to date automatically; recipes and catalogs of 0.1.0 still work.

### Added

- **Architectures.** Recipes say what they can be built for (`systems = ["aarch64-linux", …]` in `recipe.toml`,
  named as Nix names systems); images and VMs record their `system`. A recipe can carry a variant per architecture
  (`<system>.nix` next to `default.nix`). omaflake shows and builds only what runs on this computer – the app's filter
  *Other Architectures* and `omaf recipes --all-systems` show the rest, marked with their architecture; creating,
  starting, upgrading or importing a VM of another architecture is refused.
- **File schemas.** Every JSON file omaflake writes names its JSON Schema (`"$schema":
  "https://omaflake.dev/schema/<type>/v<n>.json"`); editors can check and complete them. Older files are brought up
  to date when they are read, newer ones are refused instead of misread.
- **Recipe updates at a glance.** After *Refresh* in the Recipes tab, newer versions of your downloaded recipes are
  listed – take them over, or add them next to the older ones when you keep older recipe versions.
- **Compatibility.** omaflake reads older files from a minimum version on and warns about VMs, images and recipes it
  cannot read (too old, or from a newer omaflake) instead of leaving them out.
- **Developer documentation** on [omaflake.dev](https://omaflake.dev): writing recipes, CLI reference, design, file
  schemas, contributing, and this changelog.

### Changed

- **Recipe format** (`recipe.toml` version 1): `"$schema" = "https://omaflake.dev/schema/recipe/v1.json"` and
  `systems`. Recipes without them (version 0) still build, for Apple silicon; new ones need them (`tools/catalog.py`).
- **Catalog format** (`catalog.json` version 2) with `$schema` and `systems` per recipe; version 1 catalogs still work.
  Sources of your own: update `tools/catalog.py` from [omaflake/recipes](https://github.com/omaflake/recipes) when you
  want to publish recipes for other architectures.
- The image flake builds `images.<system>.<recipe>` (was `images.<recipe>`).
- `.installed.json` keeps the installed recipes under `recipes`; a macOS VM folder's `config.json` and a package's
  `manifest.json` carry `$schema` instead of `version` / `format`.

### Migrating from 0.1.0

Nothing to do. Recipes downloaded with 0.1.0 keep working; *Refresh* in the Recipes tab shows their newer versions.

## [0.1.0] – 2026-10-19

First alpha, unsigned: NixOS guests on QEMU (3D with virglrenderer) and macOS guests on Virtualization.framework, from
the menu bar app and the `omaf` CLI; images built from recipes, recipe sources (GitHub, HTTPS, local folders),
portable VM, image and recipe packages, sound, microphone, clipboard, camera (NixOS) and shared folders, the Doctor,
and an update check.

[Unreleased]: https://github.com/omaflake/omaflake/compare/v0.3.1...HEAD
[0.3.1]: https://github.com/omaflake/omaflake/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/omaflake/omaflake/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/omaflake/omaflake/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/omaflake/omaflake/releases/tag/v0.1.0
