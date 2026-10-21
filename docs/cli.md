---
title: CLI reference
nav_order: 3
permalink: /cli/
---

# omaf – CLI (spec M3)

Swift package (repo root, `Package.swift`): `OmaflakeCore` (logic, shared with the app) + `omaf` (CLI, swift-argument-parser).
Runs headless or 2D with Homebrew QEMU; with the own build (`qemu/`, `omaf config --qemu …`) also with 3D.

## Build & test

```sh
swift test                 # unit tests (QEMU arguments, stores, deletion, QMP)
swift build -c release     # → .build/release/omaf
```

In a checkout once: `omaf config --flake <checkout>/nix` (writes `~/.omaflake/config.json`; the app ships its own
copy and uses it when nothing is set). A folder without its own `.git` is used as `path:<folder>`, so Nix sees
git-ignored settings (`nix/local/`); flake URLs are used as given. Recipes come from the sources (`omaf sources`).

## Commands

| Command | Effect |
|---|---|
| `omaf recipes [--all] [--all-systems] [--long] [--check]` | the recipes omaflake can build – installed ones and those in enabled local folders (via `nix eval <flake>#lib.recipeDefaults` with them as input `omaflake-recipes`) – with guest kind, architectures and current image; recipes for another architecture only with `--all-systems`; recipes of another `recipe.toml` version (`"$schema"`) are named with a hint (plan B114); `--long` with description and source; `--check` marks images their recipe has outgrown |
| `omaf recipes available [--all] [--all-systems]` | what the remote sources offer (fetches their catalogs), with "installed" / "update available"; recipes for another architecture only with `--all-systems` |
| `omaf recipes install <author>/<kind>/<name>` | downloads a recipe and those it `requires` from the first enabled source that has it (every file checked against the catalog's SHA-256) into `<data>/recipes/`; also updates |
| `omaf recipes versions [<recipe>]` | each recipe line (source and author/kind/name) with its versions, newest first – their omaflake IDs (as short as they can be), when they came, and the images built from them (spec §11.11) |
| `omaf recipes remove <recipe> [--all]` | removes a version of a downloaded, imported or kept recipe (by name, or its omaflake ID when there are several), or with `--all` every version of its line; versions images are built from and a local folder's own recipe stay, as does one another recipe requires |
| `omaf sources` | the recipe sources in their order (none set: omaflake/recipes); earlier ones win when two have the same recipe |
| `omaf sources add <repo\|url\|folder> [--private] [--branch b] [--first]` | adds a GitHub repository (`https://github.com/owner/repo`, `owner/repo`, `…/tree/<branch>`), the HTTPS URL of a `catalog.json`, or a folder – checked before it is kept; `--private` reads a fine-grained token (Contents: Read-only) from stdin, checks it against the repository and stores it in the Keychain |
| `omaf sources remove\|enable\|disable <n\|id>` / `omaf sources check` | remove (its token leaves the Keychain), switch on or off, check that every source can be read |
| `omaf build <recipe>` | NixOS: `nix build <flake>#images.<system>."<recipe ID>"` (this computer's system; a recipe that does not list it is refused), the qcow2 copied into the package `~/.omaflake/images/<author>.<kind>.<name>@<words>.omaflake-image/disk.qcow2`, `current/<line>` points to it. macOS: image pipeline (below). A recipe is named by `<name>`, `<kind>/<name>`, `<author>/<kind>/<name>`, or – when there are several of that name (other sources, other versions) – by its omaflake ID `recipe:<author>/<kind>/<name>:<words>` (spec §11.11) |
| `omaf build <mac-recipe> [--base <dir>] [--password-stdin] [--keep-failed]` | system closure built on the Mac → APFS clone of the newest `~/.omaflake/bases/mac-base-*` → headless start → Nix → `nix copy` → activate → shut down → `images/<recipe>-<rev>-<hash>/` (read-only), `current/<recipe>`. Asks once for the guest user's password (sudo, never stored); log in `~/.omaflake/logs/build-<image>.log`; a failed build goes to the trash |
| `omaf macos install [--ipsw <file>] [--user <u>] [--full-name "…"] [--window] [--no-first-boot]` | `omaf-runner install` (latest supported IPSW from Apple unless `--ipsw`) → `bases/mac-fresh-<build>` (read-only, never started) → APFS clone `mac-base-<build>` → first boot (headless; `--window` to watch) with provisioning (user, auto-login, SSH) → your `~/.ssh/*.pub` into the guest → shutdown → read-only. Asks once for the new guest user's password. No manual steps; existing folders are reused (BM29) |
| `omaf images` | built images and which instances use them |
| `omaf set <name> [--audio\|--no-audio] [--microphone\|--no-microphone]` | sound and microphone of an instance, from its next start |
| `omaf new <name> -f <recipe> [--ephemeral] [--no-share] [--share-dir <folder>] [--no-audio] [--microphone] [--user <u>] [--cpus/--memory/--disk/--network]` | create instance (NixOS: overlay + efivars; macOS: APFS clone) + `instance.json`; `--share-dir` shares an existing folder of your choice instead of `share/<name>` (its permissions stay as they are) |
| `omaf start <name> [--window [--gl on\|es] [--no-gl] \| --headless] [--wait]` | start QEMU in its own session (survives CLI/terminal); `--window` with 3D if the binary supports it. Without `--window`/`--headless` the recipe's `display` decides (none, window, gpu). |
| `omaf stop <name> [--timeout 60]` | ACPI powerdown → QMP `quit` → SIGKILL |
| `omaf reset <name>` | deletes overlay, efivars, known_hosts; fresh overlay (macOS: fresh clone) |
| `omaf upgrade <name>` | switch to the current image of the recipe (the old state is deleted) |
| `omaf rm <name>` | deletes the instance (its share folder stays) |
| `omaf images [list]` / `omaf images rm <image>` | images with their omaflake ID, recipe, revision, time and the instances using them / delete an image (by name, omaflake ID or the start of its words) – refused while instances use it; `current/` moves to the next newest image of the recipe's line |
| `omaf list [--long]` | instances with status, guest kind, SSH port, disk usage; VMs that run from elsewhere with their place; `--long` with each VM's omaflake ID (`vm:<name>:<words>`) and its recipe's (spec §11.11) |
| `omaf export vm <name> <folder> [--with-image]` | a stopped VM → `<folder>/<name>.omaflake-vm` (a snapshot); NixOS with its image, macOS with it only when ephemeral or with `--with-image` (spec §11.10) |
| `omaf export image <image> <folder>` / `omaf export recipe <recipe> <folder>` | an image → `<name>.omaflake-image`; a recipe and those it requires → `<author>.<kind>.<name>.omaflake-recipe` |
| `omaf import <path> [--name n] [--replace]` | always a copy into the data folder (spec §11.10), by extension, or a folder of recipes; the source is never changed. A VM comes back as the same VM; its name taken: an error, `--name` imports it as a second VM with its own identity (SSH port, macOS machine identity). The same image or changed recipes of the same ID: an error, `--replace` replaces them; identical recipes are skipped |
| `omaf external [list]` / `omaf external add <package>` / `omaf external remove <name>` | VMs that run from where their package is (an external disk, a NAS, a folder; nothing is copied, a NixOS VM must carry its image) / stop listing one (also while its place is gone; the package stays) |
| `omaf ssh <name> [-- command]` | `ssh <user>@127.0.0.1 -p <port>` (key from ssh-agent) |
| `omaf wait <name>` | wait until sshd sends its banner (without logging in) |
| `omaf agent <name> [--wait 60] [--json]` | what the VM's omaflake agent reports: version, OS, load, memory, disk, addresses, users (NixOS guests; spec §11.9) |
| `omaf log <name> [-n 40] [-f] [--qemu]` | serial console or QEMU log |
| `omaf recipes [--all] [--all-systems] [--long] [--check]` | recipes with their defaults, architectures and current image; lab (development) recipes only in developer mode or with `--all` (marked ⚗); `--long` adds each description; `--check` evaluates each recipe and marks images it has outgrown |
| `omaf doctor` | checks what the Mac brings (spec §11.2): macOS/Apple silicon, Nix, flakes, nix-darwin, Linux builder, QEMU (3D or not), image flake, SSH key, git; then the preflight for NixOS (builder answers, recipes evaluate, space in the Nix store) and macOS (IPSW, base, space); prints hints and the official install command; exit code 1 if something required is missing |
| `omaf builder setup` | on a Mac with Nix but without nix-darwin: nix-darwin with omaflake's Linux builder host in two stages (default builder from the cache, then the full host), via sudo |
| `omaf builder apply [--force]` / `omaf builder clean-up` | Linux builder via `sudo`: build the image flake's `#linux-builder` host as the user, then activate it as root like `darwin-rebuild switch` (refused without `--force` when this Mac's nix-darwin system is not omaflake's), or `nix-store --gc` in the builder with free space before/after; log `<data>/logs/linux-builder.log` |
| `omaf config [--flake p] [--qemu p] [--runner p] [--nix p] [--guest-user u] [--guest-full-name n] [--guest-locale l] [--guest-keyboard k] [--guest-time-zone z] [--home <dir>\|default]` | show/set settings (`~/.omaflake/config.json`); `--guest-…` override the Mac's values passed to the recipes (empty value = the Mac's again, spec §11.1); `--home` moves the data folder (VMs stopped, same volume; overlays name their image relatively, paths in instance.json and settings are rewritten); `--keep-recipe-versions on\|off` keeps older versions of a recipe line (default off: the newest and those images use) |

Guest user for SSH: `--user` on `omaf new`, otherwise `user = "…"` from the recipe's `recipe.toml`, otherwise the Mac user name.

Environment variables: `VM_HOME` (instead of `~/.omaflake`), `VM_FLAKE`, `VM_QEMU`, `VM_RUNNER` (signed `omaf-runner`;
otherwise `omaf config --runner`, otherwise next to `omaf`, otherwise PATH).
For window starts `omaf` passes `OMAFLAKE_DOCK_ICON` to QEMU (omaflake Dock icon, needs QEMU patchset p4 from `qemu/build.sh`).

## Behavior (decisions)

- **Ephemeral** = QEMU `-snapshot`: writes are discarded on power-off, nothing needs to be deleted.
  macOS guests get a fresh APFS clone at every start instead; the previous run's clone is deleted (plan BM27).
- **macOS guests** (`kind = macos`, M10): the same commands; `omaf start` launches `omaf-runner` detached
  (`run/<name>/runner.{sock,pid,log}`), `omaf ssh` connects to the NAT address on port 22 (shown by `omaf list`
  while running), `omaf stop` shuts down via SSH (no confirmation dialog, plan BM17). At most two macOS VMs run at
  the same time – `omaf start` refuses a third. Network `isolated` is not available for macOS guests. Once SSH
  answers (`omaf wait`, `start --wait`), the guest is named after the instance (HostName, LocalHostName,
  ComputerName via `scutil`, plan BM28).
- **Images** are kept: the current one, the one before it and those VMs use; older ones are deleted after a build (plan B52).
- **Images** are self-contained packages (the qcow2 is copied out of the Nix store, plan B108); the name contains the
  store hash → an overlay never points to different content. VMs and images are packages in the data folder, so
  copying `~/.omaflake/instances/<name>.omaflake-vm` (stopped) is already a backup (spec §11.10).
- **Sound and microphone:** sound from the VM is on, the Mac's microphone off by default (`recipe.toml`: `audio`,
  `microphone`). NixOS: Intel HD Audio on CoreAudio (`hda-output`, with the microphone `hda-micro` – without it the
  guest sees no input at all); macOS: Virtualization.framework's sound device, input only with the microphone. macOS
  asks for microphone permission the first time, for the app that started the VM (omaflake or Terminal).
- **Clipboard (both guest kinds):** text and PNG in both directions, on by default (`--no-clipboard`, `omaf set`,
  recipe.toml `clipboard`). NixOS images from plan B106 on: over the omaflake agent (message `0x0100`, spec §11.9) –
  the agent's session part `omaflake-agent session` (systemd user service, wl-clipboard) ↔ the system agent ↔
  `omaf-runner bridge`. Older NixOS images and macOS guests: the port `org.omaflake.clipboard` with one JSON object
  per line ({mime, data base64}) – NixOS: QEMU virtio-serial (`clip.sock`) ↔ `omaf-runner bridge`, guest helper
  `omaflake-clipboard`; macOS: a virtio console port of `omaf-runner run --clipboard` (the bridge runs inside the
  runner), in the guest `/dev/cu.org.omaflake.clipboard` and the helper `omaflake-clipboard-agent`, started over SSH
  once the guest answers – app always, CLI with `start --wait` (Python, `pbcopy`/`pbpaste`, PNG via `osascript`). The
  Mac side switches to the agent by itself when the guest's agent offers the clipboard. Nothing is sent until something
  is copied.
- **Camera (NixOS):** the Mac's camera as `/dev/video42` ("Mac Camera") in the guest, off by default (`--camera`,
  `omaf set`, recipe.toml `camera`). The Mac's camera runs only while an application in the guest reads the device:
  the omaflake agent (v4l2loopback with `exclusive_caps`) notices readers and asks the `omaf-runner bridge` for frames
  (1280×720 NV12) against credits (spec §11.9); older images use their helper `omaflake-camera` and the port
  `org.omaflake.camera` (`cam.sock`). macOS asks for camera permission the first time, for the app that started the VM
  (omaflake or Terminal). `OMAFLAKE_CAMERA_TEST_PATTERN=1 omaf start …` sends a test pattern instead (no camera, no
  permission; `OMAFLAKE_CAMERA_TEST_FPS` sets its rate). macOS guests: after M12 (plan §8).
- **Cancelling a build:** Ctrl-C during `omaf build` stops the running step (`nix build`, `nix copy`, SSH), forces
  a macOS build VM off and deletes the half-built folder. One build per image at a time (lock
  `images/.build-<image>.lock`, shared by app and CLI); a second one is refused.
- **Deleting:** `rm`, `reset`, `upgrade` and `images rm` delete for real; share folders with files are kept (empty ones go with their VM, plan B85). (Moving to
  `_to_delete/` is a rule for developers' own actions, not for the product – CLAUDE.md.)
- **Resolution:** the window starts at 75 % of the main screen (16:9, `xres/yres` in pixels); dragging or full screen changes the guest resolution, Hyprland scales Retina automatically.
- **Logs** are appended (`append=on`), never overwritten.
- **Lock** per instance (`.lock`, `flock`) against concurrent actions from CLI and app.
- **Share:** with the own QEMU `security_model=none,guest_owner_uid=1000` – writable from both sides.
  With Homebrew QEMU `mapped-xattr` and share folder 0777; files created on the Mac are read-only in the guest (plan B18).

## omaf-runner – macOS guests (spec §6.4, M7)

Own runner on Virtualization.framework, one process per VM (no tart). Needs the entitlement
`com.apple.security.virtualization`; an ad-hoc signature is sufficient (verified 2026-10-16).

```sh
scripts/sign-runner.sh            # build + sign → .build/…/debug/omaf-runner
```

| Command | Effect |
|---|---|
| `omaf-runner latest` | latest IPSW supported by this Mac (version, URL, size) – without downloading |
| `omaf-runner install <dir> (--ipsw <file> \| --latest) [--cpus 4 --memory-gb 8 --disk-gb 80]` | new VM folder + installation via `VZMacOSInstaller`; `--latest` downloads to `~/.omaflake/ipsw/<build>.ipsw` |
| `omaf-runner clone <source> <target>` | APFS clone of a stopped VM (`clonefile`, no extra space) with a new machine identifier and MAC address – refuses a running source |
| `omaf-runner run <dir> [--window] [--socket s] [--pid-file p] [--share dir] [--no-network] [--recovery]` | run the VM until it is off; closing the window = clean shutdown |
| `omaf-runner run … --user <user> --full-name "…" --password-stdin [--auto-login] [--ssh]` | first boot after installation only: account without Setup Assistant (guest and host macOS 27); used by `omaf macos install` |
| `omaf-runner ctl <socket> status\|stop\|kill\|show` | control a running runner (line JSON over Unix socket) |
| `omaf-runner ip <dir>` | NAT IP from `/var/db/dhcpd_leases` via the MAC in `config.json` |

VM folder: `config.json` (CPU, RAM, disk, display, MAC, build), `hardware-model`, `machine-id`, `aux.img`,
`disk.img` (RAW, sparse – Virtualization.framework supports RAW only). tart folders (`asif`) are not compatible.
