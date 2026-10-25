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

## [0.6.1] – 2026-10-25

A fix for 0.6.0: installed modules could be mixed up, so images failed to build. **After updating, remove the
modules in the Modules tab (or move `~/.omaflake/modules` aside) and install them again** – "Install All" in the Modules
tab does it in one go.

### Fixed

- Installed modules could hold another module's files when several recipes were installed at once (all of them
  shared one download folder), so builds failed with errors such as "attribute 'session' missing" or "The option
  `omaflake.homeLinks' does not exist". Each install has its own folder now. Modules installed with 0.6.0 should be
  removed and installed again.
- The Modules tab drew an empty dashed frame before every module.
- Without a macOS base, the app showed "Create macOS Base …" twice.

### Added

- A macOS base being created shows its progress under macOS Bases (also after leaving its page); a click opens it.
- "Install All" in the Recipes, Modules and Looks tabs installs everything the list offers, as filtered.

## [0.6.0] – 2026-10-25

**Recipes are put together from building blocks now.** A recipe is data – a platform, the **modules** in it, a
suggested look, **key bindings**, what a VM needs at least and how it runs –, and the Nix lives in modules, reviewed in
their catalog. Installing a recipe brings what it is made of; the new **Modules** tab lists the modules and key
bindings here; **Profiles** say how big a new VM is; and **New Recipe …** puts a recipe of your own together in the
app. Recipes of the old format (recipe.toml v1 with default.nix) and catalogs before version 4 are not read any more
(no compatibility before 1.0): omaflake/catalog has all of omaflake's recipes in the new format.

Looks are installable now, like recipes: the new **Looks** tab lists the looks here with their colours, which images
use them and the default one, and what your sources offer besides. No look is built in any more. The official source
is the new **omaflake/catalog** – recipes and looks; omaflake/recipes is archived. Your settings file becomes version
4: the look you had chosen becomes the default look, and the official source switches over. **Locales** – where a
guest lives: language and region, time zone and up to three keyboards – are created like credentials and chosen when
an image is built; settings version 5 turns your former guest values into a locale "Standard", and image.json
version 3 records an image's locale.

### Added

- **Modules and key bindings:** omaflake's own recipes are made of modules in omaflake/catalog – Hyprland, the shell,
  WezTerm, Zellij, Neovim, Emacs, Zathura, LaTeX, Typst, Python for data, pandoc, omaflake development (NixOS); the
  desktop, terminal tools, WezTerm, Zellij, Neovim, LaTeX, Typst, VS Code, Homebrew, pandoc, lazygit (macOS) – and
  the key bindings `omaflake/default`. A module says what it provides and needs (capabilities: desktop, terminal,
  multiplexer, shell, editor, default-editor, pdf-viewer) and which actions take a key; a recipe names modules and key
  binding sets in any version, an exact one, or a minimum and/or maximum. `omaf modules [available|install|rm]`.
- **What a recipe is made of** shows in its details and on the build page: its modules, look, key bindings, every key,
  what is missing ("Install Missing Parts" – a build waits for it) and warnings (a module needs something no other
  provides, Neovim and Emacs together, keys that overlap or break the key rule, unfree software, a locale the recipe
  is not made for). `omaf recipes show <recipe> [--install]`.
- **The Modules tab:** the modules and key bindings here with the recipes made of them; install, update and remove
  them, and see what your sources offer besides.
- **Profiles** (the gauge in the header): CPU cores and RAM of a new VM. The New VM page offers them, preset with the
  default ("Custom values" sets them by hand), and warns when a VM gets less than its recipe needs. `omaf profiles
  [add|edit|rm|default]`, `omaf new --profile`.
- **My Recipes:** "New Recipe …" in the Recipes tab, or "Copy to My Recipes …" on any recipe, opens the recipe editor
  – a platform, a switch per module, a look, up to two key binding sets, what a VM needs and how it runs, with the
  warnings while you choose. Saved recipes are a source of their own in the data folder and built like any other.
- **The Looks tab:** install looks from your sources (omaflake's own, Catppuccin, Rosé Pine Dawn, Vanilla in the
  official catalog), see a strip of their colours and which images use them, make one the default, update and remove
  them (not while images are built with one). `omaf looks [available|install|rm|default]`.
- **The Doctor wants a look:** without one it offers "Install Recommended" – omaflake's own and Catppuccin.
- **Locales** (the globe in the header): one language and region, one time zone, one to three keyboards – the first is
  the default, Super+Space switches on NixOS (a language indicator in the bar), macOS shows them in the menu bar. The
  build page chooses one, preset with the default; while images are built with a locale only its name can change.
  `omaf locales [add|edit|rm|default]`, `omaf build --locale`. Russian, Ukrainian and Greek keyboards are new.

- **The Doctor requires a credential:** images and macOS bases are made with one, so "Credential" is a requirement
  now – with "New Credential …" while there is none.
- **A macOS base keeps its credential:** a new base is created from a credential – its user, full name and
  password – and remembers it; the bases section shows it ("Work (alex)"). An older base gets one with "Assign
  Credential …" (`omaf macos assign <base> <credential>`); its user must be the base's.

### Changed

- **Recipe format v2 and catalog v4:** a recipe is recipe.toml only (`platform`, `modules`, `look`, `keymaps`,
  `[requirements]`, `[vm]`, `[locales]`); Nix belongs in a module. The image flake's input is `omaflake-catalog` (a
  catalog-shaped tree: recipes, modules, key bindings) instead of `omaflake-recipes`. Older recipes and catalogs are
  not read; `requires` is gone. A recipe package (`.omaflake-recipe`) carries the recipe alone.
- **Keys in the guests come from the recipe:** each module's default keys, overridden by the recipe's key binding sets
  – the same keys as before for omaflake's recipes.
- **A build uses the recipe's look** when it is installed and none is chosen; otherwise the default look.
- **Defaults live in their tabs:** the default credential is set on the Credentials page, the default look in the
  Looks tab, the default locale on the Locales page; the settings keep only this computer's own keyboard ("This
  Mac"). `omaf config` loses `--default-credential`, `--guest-look`, `--guest-locale`, `--guest-keyboard` and
  `--guest-time-zone`; `--host-keyboard` is new.
- **Building asks for an installed look:** the build page offers the looks here, preset with the default; "The
  recipe's own" is gone – a look is chosen for every image (`omaf build --look <author>/<name>`).
- **The official source is omaflake/catalog** (recipes and looks, catalog version 3); recipes installed from
  omaflake/recipes count as from it.
- **No password is ever typed after a credential's creation:** creating a macOS base and building a macOS image take
  the credential's stored password; the build page shows the base's credential and asks for nothing.
  `omaf macos install --credential` replaces `--user`, `--full-name` and the password prompt; `omaf build` no longer
  takes `--password-stdin`.
- **The host has nothing to do with credentials:** only the first credential starts from this computer's user and
  name; "This computer" is gone as a choice – every image is built with a credential (the default unless another is
  chosen), and there is always a default while there are credentials. Every credential names its own user.
- **A credential used by a macOS base** keeps its user, full name and password – only its name can change – and
  cannot be removed while the base is there.

## [0.5.2] – 2026-10-25

An image is now a recipe with a credential and a look: you choose both when you build it, the settings only preset
them. Credentials – who you are in a guest – get a page of their own; each keeps its password, which macOS builds use
and which you can copy whenever a guest asks for it. Your settings file becomes version 3 and image.json version 2;
an older omaflake ignores the settings and no longer reads the images, so update every omaflake that uses this data
folder. (0.5.1 was not published; its changes are part of this release.)

### Added

- **Credentials:** named sets of user, full name and e-mail – "Work", "Private" … – on a page of their own (the key
  next to the settings; `omaf credentials`). Each row shows who uses it ("alex · 1 image"); a click opens its values
  and actions (Edit, Change Password, Make Default, Remove). One is the default for new images.
- **A password for every credential:** asked (twice) when you create one, kept in the Keychain (on Linux in the
  desktop's keyring); it can be changed, not removed. Building a macOS image with the credential asks for no
  password.
- **Copy a credential's password:** the quick icon on each credential copies its password after Touch ID or your
  login password (on Linux at once); the clipboard is cleared after 90 seconds – you never have to remember a guest's
  password (`omaf credentials copy-password`).
- **Credential and look when building:** the build page asks which credential and which look the image gets, preset
  with the defaults (`omaf build --credential … --look …`).
- **Credential and look at images and VMs:** the lists show them next to the recipe ("dev · Work · Rosé Pine Dawn"),
  the details have a line each ("Credential: Work (alex)", "Look: Rosé Pine Dawn"); `omaf images` prints them. Images
  built before show their look as "unknown" and this computer's values as their credential.
- **macOS bases in the images tab:** a section of their own under the images, with Show in Finder and Remove (not
  while a macOS base is installed or a macOS image is built).

### Changed

- **The settings set defaults "For new images":** "Default credential" and "Default look" instead of user, full name
  and e-mail and the look of every VM; the section (formerly "Guests") says that its values – credential, look,
  language, keyboard, time zone – are used when an image is built, while existing images and their VMs keep what they
  were built with. A user, full name or e-mail you had set becomes a credential, which becomes the default.
  `omaf config --default-credential` replaces `--guest-user` and `--guest-full-name`.
- **A used credential stays as it is:** editing is a form whose fields say what they are for (the user name is
  NixOS's login, home folder and `omaf ssh`; macOS guests keep the base's user). While images use the credential
  only its name can change – the other fields are greyed out with the reason –, and it cannot be removed until those
  images are gone.
- **Images turn "outdated" right away** when what they were built with changes – their recipe, the guest settings
  (keyboard, locale, time zone) – not only at the next look at the recipes; the badge and the status say so.
- **Doctor:** one line "omaflake" with the app's and the core's version, right above the update check.

## [0.5.0] – 2026-10-25

The guests get looks: omaflake's own – black, high contrast – is the new standard, Catppuccin Mocha (the look so
far), Rosé Pine Dawn and each program's own colours are a setting away. Your settings file becomes version 2: an older
omaflake ignores it and uses its defaults, so update every omaflake that uses this data folder. Rebuild an image to get
any of this inside the VM.

### Added

- **Looks for the guests:** omaflake (black, white text, red and cyan only as fine accents – the logo's split on the
  active window and workspace), Catppuccin Mocha, Rosé Pine Dawn (light and mild) and Vanilla (each program's own
  colours). Choose one for all your VMs in the settings ("Look") or with `omaf config --guest-look …`; "Each recipe's
  own" leaves it to the recipe (`omaflake.look` in its default.nix). A NixOS VM takes its whole desktop from it –
  Hyprland, the bar, launcher, notifications, prompt, GTK apps and the pointer –, every VM its terminal (WezTerm,
  Zellij) and editors (Neovim, Emacs); the macOS research recipe follows it too.
- **Moving and resizing windows by keyboard in NixOS VMs:** Cmd + Shift + arrows moves a window, Cmd + R opens a
  resize mode (arrows or h/j/k/l, Esc to leave) – like Zellij's modes. Rebuild an image to get it.

### Changed

- **The standard look of NixOS VMs is omaflake's** instead of Catppuccin Mocha; choose Catppuccin in the settings to
  keep the old one.
- **Every key omaflake binds in a VM starts with Cmd or Ctrl** (optionally with Shift): the area screenshot moved
  from Print to Cmd + Shift + S.
- **Zellij in the neovim line at a key:** Cmd + Shift + Enter opens a terminal in the Zellij session "main" – close the
  window and it keeps running; `zj` attaches to it from any shell, also over SSH. Cmd + Enter stays a plain terminal
  (macOS research: the same keys in WezTerm). Zellij uses its
  "Unlock First" keys now, so Neovim gets every Ctrl key: Ctrl G unlocks Zellij's modes (pane, tab, resize, move …).
  Rebuild an image to get it.

### Fixed

- **`omaf ui`: the action menu (`a`) follows its selection.** In a terminal too small for all of a VM's actions, the
  last ones – "Remove …" among them – were cut off and could not be seen. The menu now scrolls with the selection, and
  arrows on its frame show that more is above or below.

## [0.4.0] – 2026-10-24

omaflake on a Linux PC, checked on real hardware: an x86_64 mini PC with Ubuntu 26.04 and GNOME 50 builds the official
NixOS recipes and runs them with KVM – windows with 3D that follow their size, clipboard both ways, sound, microphone
and camera. The Linux package now brings everything `omaf` needs. Still an unsigned alpha. Your VMs, images, recipes and
settings stay as they are; rebuild an image to get the fixes inside the VM (resizing, camera).

### Fixed

- **3D windows on Linux distributions other than NixOS** (Ubuntu, Fedora, …): QEMU from Nix aborted with "Couldn't find
  current GLX or EGL context", because Nix programs find no graphics driver there. omaflake now fetches Mesa from
  QEMU's own Nix packages the first time (a minute, once) and starts the window with it; the VM's QEMU log says so.
- **A NixOS VM's window resizes the guest again:** after dragging the window the guest kept its first resolution (black
  bars) – Hyprland keeps an output's mode while it stays connected. The images now follow the new size themselves
  (rebuild an image to get it). They keep their scale while resizing – a big window no longer jumps to huge text –
  and the background colour fills the whole window.
- **The camera in a NixOS VM shows a picture again in mpv, ffmpeg and browsers:** they got one black frame and then
  waited – their first frame was minutes old. The VM's agent now keeps that frame fresh (rebuild an image to get it).
- **Clipboard from a GNOME host into the VM:** GNOME offers no way to watch its clipboard, so nothing copied there
  reached the VM. omaflake now uses GNOME's Xwayland for it (xclip). The Nix package brings wl-clipboard and xclip for
  `omaf` too.
- With `OMAFLAKE_ACCEL=tcg` the start line and the VM's details no longer claim that /dev/kvm is missing.

### Changed

- **The Linux package brings the image flake:** `omaf` on Linux builds images right after
  `nix profile install github:omaflake/omaflake#omaflake-gui`, without `omaf config --flake …` (as the Mac app does).
- The CLI's help speaks of "this computer" instead of "the Mac" (sound, microphone, clipboard, camera, guest values).

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

[Unreleased]: https://github.com/omaflake/omaflake/compare/v0.4.0...HEAD
[0.4.0]: https://github.com/omaflake/omaflake/compare/v0.3.1...v0.4.0
[0.3.1]: https://github.com/omaflake/omaflake/compare/v0.3.0...v0.3.1
[0.3.0]: https://github.com/omaflake/omaflake/compare/v0.2.0...v0.3.0
[0.2.0]: https://github.com/omaflake/omaflake/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/omaflake/omaflake/releases/tag/v0.1.0
