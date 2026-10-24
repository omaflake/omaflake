---
title: Linux
nav_order: 11
permalink: /linux/
---

# omaflake on Linux

omaflake runs NixOS VMs on a Linux host too (plan B124, issue #2): QEMU with KVM, images built natively with Nix, the
same CLI, `omaf ui` and a native app. macOS guests need a Mac.

## The host: NixOS VMs on Linux (plan B124)

**What runs:** NixOS guests of the host's own architecture – an x86_64 PC runs `x86_64-linux` images, an aarch64
machine `aarch64-linux` ones (a recipe lists what it can be built for in `systems`, recipe.toml). Everything else is the
same as on a Mac: overlays on images, SSH forwarded to `127.0.0.1:<port>`, the share folder over 9p, the serial log,
QMP, ephemeral VMs, packages (export/import).

**What it needs:**

| | |
|---|---|
| Linux on x86_64 or aarch64 | any distribution; NixOS is the tested one |
| KVM | `/dev/kvm`, read-write for you – otherwise VMs still run, with TCG (below) |
| Nix with flakes | NixOS: `nix.settings.experimental-features = [ "nix-command" "flakes" ];` – elsewhere Nix's installer (`curl -L https://nixos.org/nix/install \| sh -s -- --daemon`) and the same line in `/etc/nix/nix.conf` |
| QEMU with UEFI firmware | `nix profile install nixpkgs#qemu` (brings edk2 for both architectures, virglrenderer, GTK, PipeWire/PulseAudio); a distribution's QEMU works when its OVMF (x86_64) or AAVMF (aarch64) package installs QEMU's firmware descriptors (OPEN: not tried yet) |
| an SSH key | `ssh-keygen -t ed25519` – the images let it in |

No Linux builder, no nix-darwin: `omaf build <recipe>` runs `nix build` of `images.<host system>.<recipe>` right here.
`omaf doctor` (and the app's Doctor) checks all of it and names the fix; it has no macOS section on Linux.

### KVM

The Doctor's `kvm` line is ok when `/dev/kvm` opens read-write (what QEMU does). systemd's own udev rule gives it the
group `kvm` and mode 0666 (`dev-kvm-mode`; nixpkgs keeps that default, so on NixOS everyone may use it); distributions
that build systemd with 0660 need you in the group `kvm`.

- **NixOS:** usually nothing to do once the CPU's module loads (`kvm-intel`/`kvm-amd`, in the generated
  hardware-configuration.nix). If the Doctor says *no access*: `users.users.<you>.extraGroups = [ "kvm" ];`,
  `nixos-rebuild switch`, log in again.
- **Other distributions:** `sudo usermod -aG kvm "$USER"` (the Doctor's Install … runs it in a terminal), then log in
  again; check with `ls -l /dev/kvm` and `id`.
- **No /dev/kvm at all:** turn on virtualization in the firmware settings (Intel VT-x, AMD-V/SVM; on aarch64 Linux
  must run at EL2), load `kvm_intel` or `kvm_amd`; inside a VM the hypervisor must offer nested virtualization.

### TCG – the slow fallback

Without a usable `/dev/kvm` omaflake starts VMs with QEMU's emulator (`-accel tcg -cpu max`) instead of refusing:
`omaf start` prints a line about it, the VM's details show *Accelerator: TCG* with a warning, the Doctor's `kvm` line is
*limited*. Everything works, many times slower: measured in an aarch64 omaflake VM on a Mac (no nested virtualization),
`neovim-minimal` with 4 vCPUs reached SSH after 82–92 s headless (with KVM: seconds); a bigger recipe or a slower CPU
takes longer, so give `omaf wait --timeout` 900 or more. `OMAFLAKE_ACCEL=tcg` forces TCG where KVM works (to compare).

### What QEMU gets

x86_64: `qemu-system-x86_64 -machine q35 -vga none -accel kvm -cpu host`, UEFI from `edk2-x86_64-code.fd` with a copy of
`edk2-i386-vars.fd` as the VM's variable store; aarch64: `qemu-system-aarch64 -machine
virt,highmem=on,gic-version=host` (`max` with TCG), `edk2-aarch64-code.fd`. Windows: GTK (`-display gtk,gl=on` with
`virtio-vga-gl` on x86_64, `virtio-gpu-gl-pci` on aarch64; without 3D `virtio-vga`/`virtio-gpu-pci`). Sound: PipeWire
when its socket is in `$XDG_RUNTIME_DIR`, else PulseAudio, else no sound card. The full list: spec §6.3.

Not on Linux yet (OPEN): the agent bridge (`omaf-runner bridge` is macOS-only – no agent report in the VM's details,
readiness by SSH), the shared clipboard and the camera (QEMU's GTK clipboard with `qemu-vdagent` would be a way), the
share folder writable from both sides (the Mac's QEMU has a guest-owner patch; Linux uses `mapped-xattr`: files from
the host are read-only in the guest).

### Checking on a real machine

What was run so far (plan B124): aarch64 under TCG, end to end. Still to run on an x86_64 PC with KVM (e.g. NixOS):

```sh
# 1. omaflake from this branch, QEMU, a key
nix profile install nixpkgs#qemu          # or: environment.systemPackages = [ pkgs.qemu ];
git clone https://github.com/omaflake/omaflake && cd omaflake && git checkout <branch>
cargo build --release -p omaf             # in: nix shell nixpkgs#cargo nixpkgs#rustc nixpkgs#gcc
export PATH=$PWD/target/release:$PATH
ssh-keygen -t ed25519                     # if there is none
# 2. a clean data folder, the image flake and the recipes
export VM_HOME=$HOME/omaflake-test
omaf config --flake $PWD/nix
omaf sources add https://github.com/omaflake/recipes   # or a local checkout
omaf doctor                               # host Linux x86_64, kvm ok, QEMU (3D) – UEFI …/edk2-x86_64-code.fd
# 3. an x86_64 image: the recipe must list it – systems = ["aarch64-linux", "x86_64-linux"] in its recipe.toml
#    (a local recipes checkout: change it there, then omaf sources add <checkout>)
time omaf build neovim-minimal
# 4. the VM cycle with KVM
omaf new t1 --recipe neovim-minimal
omaf start t1 --headless --wait           # no TCG line; the details say Accelerator KVM
omaf ssh t1 -- uname -m                   # x86_64
cat $VM_HOME/run/t1/serial.log | tail     # console on ttyS0
omaf ssh t1 -- true; omaf stop t1 && omaf start t1 --headless   # straight after the stop
omaf stop t1
omaf start t1 --window                    # GTK window, 3D (virtio-vga-gl); sound through PipeWire
OMAFLAKE_ACCEL=tcg omaf start t1 --headless --wait   # after a stop: the TCG fallback, for comparison
omaf stop t1; omaf rm t1
```

Look for: the boot to SSH time with KVM, the window (3D, resizing, keyboard), sound, the share folder (`/mnt/share`),
and `omaf ui` / the app's Doctor and VM details.

## The app (plan B119)

A native window written in Rust with [iced](https://iced.rs), in omaflake's look (the same design tokens as the macOS
app, `brand/tokens.toml`), plus a tray icon. Like every omaflake interface it holds no logic of its own: it draws the
pages omaflake's core describes (the presentation model, [api §10](api.md#10-views--what-each-page-shows)) and sends
the actions they name. On a Linux host the core runs VMs and builds images itself (above).

### Install and run

With Nix (aarch64-linux, x86_64-linux):

```sh
nix run github:omaflake/omaflake#omaflake-gui          # try it
nix profile install github:omaflake/omaflake#omaflake-gui
```

The package holds the app `omaflake` and the CLI `omaf`, a desktop entry (`omaflake.desktop`) and the icon in the
hicolor theme, so it shows up in the desktop's app list. It needs a GPU driver with Vulkan (or OpenGL), Wayland or X11;
the wrapper brings the libraries iced loads at run time (Vulkan loader, libGL, Wayland, xkbcommon, the X11 libraries,
fontconfig). In a VM without a Vulkan GPU (virgl, as in omaflake's NixOS guests) the only Vulkan device is Mesa's
software rasterizer (lavapipe), which is slow and keeps a core busy: before its window opens the app asks Vulkan for its
adapters and, when every one is a CPU, draws with OpenGL instead (it sets `WGPU_BACKEND=gl` for itself and says so on
stderr). A `WGPU_BACKEND` you set (`vulkan`, `gl`) is used as it is; if OpenGL fails too, iced falls back to its
software renderer.

From a checkout (development): `cargo run -p omaflake-gui` – on a Mac too, as a development aid (the macOS app stays the
SwiftUI one; there the tray does not exist). `VM_HOME=<folder>` points it at another data folder.

### How it connects

At start the app runs `omaf service start --json` (the `omaf` next to it, else the one on `PATH`) and connects to the
per-user core service on its socket ([api §1](api.md#1-transport)) – the same service the CLI uses, so a build started
with `omaf build` shows up in the app and the other way round. When no service can be started it runs the core in its
own process instead (a line under the tabs says so; its jobs then end with the app). `OMAFLAKE_IN_PROCESS=1` forces
that. If the service ends (an update, idle exit), the app connects again.

### What it shows

- **Home:** the header (Doctor with its issues, Settings, the data folder, Quit) and the tabs VMs, Images, Recipes,
  Activity with their badges; each list with its filter, its tools (import, refresh, clear) and rows that open to their
  details: switches (clipboard, sound, microphone, camera), info lines and the row's actions.
- **Pages:** New VM, Build Image (progress, running and quiet time, outcome, the live log), Doctor (versions, updates,
  the checks with their actions), Create macOS Base (shown as the core describes it; macOS guests need a Mac – whether the core
  greys its actions out on a Linux host is OPEN), Settings (recipe sources, general switches, guest values, data folder, and the
  app's own tray switch).
- **Questions:** confirmations and typed values (names, passwords, tokens) as a sheet over the window; folders, files
  and packages through the desktop's file chooser (XDG desktop portal), checked by the core before they count.
- **Desktop:** "Show …" and "Open …" use `xdg-open`; "Open Terminal with SSH" and the Doctor's install commands open
  `$TERMINAL`, else the first of foot, kitty, wezterm, alacritty, gnome-terminal, konsole, xterm. Notifications (VM
  ready, image built or failed, …) go to the desktop's notification service while the window is not in front and
  Settings → Notifications is on.
- **Language:** English or German, from `OMAFLAKE_LANGUAGE` (`en`, `de`), else the locale (`LC_ALL`, `LC_MESSAGES`,
  `LANG`); English for any other. The texts of the pages are translated by their message keys with the macOS app's
  table (crate `omaflake-i18n`, generated from `App/Resources/de.lproj/Localizable.strings` by
  `scripts/translations.py`), the app's own words too, and the core's error messages (by their key, `data.text` of
  docs/api.md §3, plan B125); dates and sizes follow the language. Still English: build logs, the Doctor's details and
  values inside a message (an OS's or a tool's output).
- **Private recipe sources:** the token goes to the desktop's keyring through the Secret Service (GNOME Keyring,
  KWallet, KeePassXC – service `app.omaflake.recipe-source`, account the source's ID). Without one (a server, an SSH
  session without a desktop) it is kept unencrypted in `$XDG_CONFIG_HOME/omaflake/tokens.json` (default
  `~/.config`; the file 0600, its folder 0700), and adding the source says so. The CLI (`omaf sources add … --private`)
  does the same.

### Tray

A StatusNotifierItem, on by default: KDE Plasma, Cinnamon, XFCE and waybar's `tray` module show it; GNOME only
with the AppIndicator extension (not shipped by omaflake). Its tooltip says how many VMs run and builds are under way;
its menu lists the VMs with their status and what can be done now (start, start headless, terminal, shut down), the
running builds, "Open omaflake" and Quit. A click on the icon opens the window.

While the tray is on, closing the window keeps omaflake running in the tray; Quit (in the window or the tray) ends it.
VMs keep running either way, and the service goes on for its other clients and jobs. Switch the tray off in Settings →
This app → Tray icon; the choice is stored per user in `$XDG_CONFIG_HOME/omaflake/linux-app.json` (default
`~/.config/omaflake/`), not in omaflake's own settings.

### waybar

waybar shows the tray icon with its `tray` module:

```jsonc
"modules-right": ["tray"],
"tray": { "spacing": 8 }
```

and omaflake's own status (running VMs, builds) with a `custom` module fed by `omaf status --waybar --follow` – the
snippet is in the [CLI reference](cli.md#waybar).

## Trying it in the lab VM

The development recipe `omaflake/nixos/lab` (developer mode) brings omaflake itself: the app `omaflake`, the CLI `omaf`
and its TUI `omaf ui`, plus waybar's `custom/omaflake` and `tray` modules (plan B123). So the Linux interfaces can be
tried on a real Linux desktop (Hyprland, aarch64) on a Mac:

```sh
omaf build nixos/lab
omaf new lab-omaflake --recipe nixos/lab
omaf start lab-omaflake --window
```

In the VM: Cmd+D → *omaflake* opens the app (its icon appears in waybar's tray), a click on the ❄ in waybar opens
`omaf ui` in WezTerm, and `omaf doctor` / `omaf status` work in the terminal. The core service is started on demand (by
the app, `omaf ui` or the waybar module) – no unit is installed. VMs inside the lab run with TCG only (the Mac offers
no nested virtualization) – slow, but the whole cycle works (plan B124: build, import, start, SSH, stop).

Which omaflake the lab gets: built from a checkout (the image flake is the checkout's `nix/` folder), the code as it is
on disk – omaflake passes `--override-input omaflake <checkout>` (tracked files with their changes; a new file must at
least be `git add`ed); from the app's bundled flake, the revision of `github:omaflake/omaflake` in its `flake.lock`.

## Development

- `cargo run -p omaflake-gui`; `cargo test -p omaflake-gui` – the tests decode every golden page model of the core
  (`crates/omaflake-core/testdata/api/views/`) and draw it, build the tray menu from them and check the terminal choice.
- `OMAFLAKE_DEMO_VIEWS=crates/omaflake-core/testdata/api/views` draws those golden models without a core (for looking
  at pages without VMs); `OMAFLAKE_DEMO_TAB=images`, `OMAFLAKE_DEMO_PAGE=build` (or `build.failed`, `doctor`,
  `settings`, `settings.linux`, `newVm`, `macBase`, `vm`, `image`), `OMAFLAKE_DEMO_VARIANT=linux,loading` (the home
  tabs' `<page>.<variant>.json` where there is one: `home.linux`, `recipes.loading`) and
  `OMAFLAKE_DEMO_EXPAND=vm:research` pick what is shown. Actions do nothing there.
- Icons: [Lucide](https://lucide.dev) (ISC, see `crates/omaflake-gui/NOTICE`), mapped from the views' icon names in
  `src/icons.rs`. Brand logos are not part of Lucide: macOS guests show an apple, GitHub sources a branch.
