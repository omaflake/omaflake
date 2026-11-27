# Plan – omaflake (review + implementation plan)

As of 2026-10-16 · Based on spec v0.2 (`docs/spec.md`) · Complements the spec, does not replace it.
Decisions still to be made are marked **DECISION**; anything not verified is marked **OPEN**.

> **Repo layout:** this repo holds the app, the CLI `omaf`, the runner `omaf-runner`, the QEMU build and the
> guest module (`flavors/modules/vm-guest.nix`). The flavors live in `flavors/` (own flake, used as `path:`); personal settings in each developer's git-ignored `flavors/local/` (template: `flavors/local.example/`).
> The flavors `minimal`, `dev`, `research`, `lab` and `mac-minimal`, `mac-office`, `mac-dev` are examples.
> Current state and order of the next steps: **§8**.

---

## 1. Review of the original plan

### 1.1 What holds up

- **Separating content from operation** (Nix defines the VM, app/CLI runs it) is right and keeps the app small.
- **Immutable image + qcow2 overlay** is the right model for "clean at the push of a button".
- **One Swift package for CLI and app** (`OmaflakeCore`) – CLI first, the app as a thin layer on top.
- **Flavors as Nix modules + `flavors.toml`** – clear and extensible.
- "Never delete → `_to_delete/`" is already part of the data model.

### 1.2 Verified findings (on the development Mac, 2026-10-16)

| # | Assumption in spec | Finding | Evidence |
|---|---|---|---|
| B1 | Flake evaluates | ✅ `vm-dev` toplevel evaluates (nixpkgs 26.11, 2026-10-14). `nix flake check` still pending. | `nix eval --no-write-lock-file .#nixosConfigurations.vm-dev.config.system.build.toplevel.drvPath` |
| B2 | Repo is versioned, `flake.lock` committed, image name contains Git rev | ❌ Folder is **not a Git repo**, there is **no `flake.lock`**. So `rev` for `<flavor>-<rev>.qcow2` is unavailable. | `ls -la` |
| B3 | M1 image path OPEN | ✅ **Resolved:** nixpkgs provides `system.build.images.qemu-efi` (also `raw-efi`, `qemu`). `nixos-generators` is not needed. | `nix eval …config.system.build.images --apply builtins.attrNames` |
| B4 | Labels `nixos`/`ESP` | ✅ `make-disk-image.nix` sets `-L nixos` and `mkfs.vfat -n ESP`; `disk-image.nix` expects the same labels as `vm-guest.nix`. Note: set there without `mkDefault` → the `umask=077` option from `vm-guest.nix` does not apply in the image (harmless). | nixpkgs `nixos/modules/virtualisation/disk-image.nix`, `nixos/lib/make-disk-image.nix` |
| B5 | Linux builder available | ❌ No nix-darwin, no `/etc/nix/machines`, `builders` empty. **aarch64-linux images cannot be built yet.** (See `docs/linux-builder.md`.) | `nix config show`, `command -v darwin-rebuild` |
| B6 | Homebrew QEMU supports `virtio-gpu-gl-pci` + `cocoa,gl=es` | ❌ **No.** QEMU 11.1.2 (Homebrew) only knows `virtio-gpu-pci`; `virtio-gpu-gl-pci` is "not a valid device model name". The reference invocation in spec §6.3 does not start. | `qemu-system-aarch64 -device help` |
| B7 | UEFI firmware | ✅ `/opt/homebrew/opt/qemu/share/qemu/edk2-aarch64-code.fd` (64 MiB). There is no `edk2-aarch64-vars.fd` → create `efivars.fd` as an empty 64 MiB file (or copy `edk2-arm-vars.fd`). | `ls` |
| B8 | QEMU needs hypervisor entitlement | ✅ Homebrew QEMU is signed with `com.apple.security.hypervisor`. **An own QEMU build must be too.** | `codesign -d --entitlements -` |
| B9 | — | ⚠️ **Corrected 2026-10-16:** `swap-opt-cmd=on` turns Cmd into *Alt* (not SUPER). Without swapping, Cocoa passes Cmd as `KEY_LEFTMETA` (= SUPER) – but only while the mouse is grabbed (click into the window, release with Ctrl+Alt+G). `-pidfile` is available. | `ui/cocoa.m` (kVK_Command) |
| B10 | Clipboard via QEMU | ❌ A `qemu-vdagent` chardev does not show up in the Homebrew build (`-chardev help`). On the guest side spice-vdagent is X11-oriented anyway. | `-chardev help` |
| B11 | UTM as fallback | ✅ UTM 4.7.5 ships its own QEMU with **virglrenderer, epoxy, EGL/GLESv2 (ANGLE)** and `utmctl` (`list/start/stop/clone/status/ip-address/exec`). UTM is sandboxed (entitlement present; value not read → effect on backing files **OPEN**). | `/Applications/UTM.app/Contents/{MacOS,Frameworks}` |
| B12 | — | ⚠️ Unix socket paths on macOS are limited to < 104 bytes (QEMU error "UNIX socket path … too long"). `~/.omaflake/instances/<name>/qmp.sock` fits; limit instance name length. | Boot probe 2026-10-16 |
| B13 | — | ⚠️ QMP delivers events **before** the response (`POWERDOWN` arrived before the `return` of `system_powerdown`) → `QMPClient` must separate events from responses. | Boot probe |
| B14 | — | ℹ️ `qemu-guest-agent` stays inactive without the virtio-serial port `org.qemu.guest_agent.0` → add it in the CLI (guest IP, `guest-exec`, clean shutdown without ACPI). | `systemctl is-active` in the guest |
| B15 | — | ℹ️ There is no KVM inside the Linux builder; image creation runs under TCG. `images.minimal` (incl. Hyprland) still builds in a few minutes; qcow2: 9.4 GiB virtual, 6.4 GiB used. | Build log |
| B16 | — | ⚠️ The user's SSH key has a passphrase; without a loaded agent SSH fails without prompting. → **DECISION** (M3): dedicated passphrase-less VM key vs. requiring an agent. | `ssh -v` |
| B17 | — | ✅ `-daemonize` works with `-display none`; with Cocoa still OPEN. The CLI uses `posix_spawn` + `POSIX_SPAWN_SETSID` instead. | Boot probe |
| B18 | — | ⚠️ 9p `mapped-xattr`: Mac files appear in the guest with Mac UID 502 and Mac permissions → the guest user (UID 1000) cannot modify them. Guest files carry xattrs `user.virtfs.{uid,gid,mode}`. Workaround: share folder 0777; Mac files remain read-only in the guest (OPEN → M6). | `ls -ln`, `xattr -l` |
| B19 | — | ✅ Hyprland 0.56 prefers `~/.config/hypr/hyprland.lua` (`hyprland.conf` is only a "legacy" fallback); home-manager with `stateVersion` 26.05 generates Lua by default (`configType = "lua"`, every `settings` key → `hl.<name>(…)`), which is why the hyprlang settings broke at `$mod`. **Resolved:** the maintainer's home-manager config is written natively in Lua (`settings.config/monitor/env` + binds/autostart in `extraConfig`), API from `share/hypr/stubs/hl.meta.lua`. `dwindle.pseudotile` no longer exists (dropped). | `hyprctl configerrors` empty, 33 binds, `grim` screenshot without error banner (2026-10-16) |
| B20 | — | ℹ️ `codesign` rejects QEMU after `ninja install` ("resource fork, Finder information … not allowed") → `xattr -cr` before signing. QMP `screendump` is black for GL scanouts; own build without libpng → PPM only. | Build 2026-10-16 |
| B21 | — | ℹ️ `full-grab=on` needs the Accessibility permission for the QEMU binary ("Could not create event tap"); ad-hoc-signed rebuilds lose it → off by default. Cmd+Tab/Cmd+Space therefore stay with the Mac. | `qemu.log` |
| B22 | — | ⚠️ `gl=es` (ANGLE): virgl reports only GLSL 130 (GL 3.0) to the guest → kitty: "EGL: Failed to create context". `gl=on` (native OpenGL, Core 4.1): GLSL 410, kitty runs → **default `gl=on`** (as in try-omarchy). | `qemu.log` "FINAL VALUES", test in guest |
| B23 | — | ⚠️ kitty starts with `gl=on` but under virgl draws **no text** (keys arrive, window stays empty). Correct with `LIBGL_ALWAYS_SOFTWARE=1` (try-omarchy does this); Alacritty and **WezTerm** render cleanly on the GPU → the terminal is now WezTerm (DECIDED). | `grim` screenshots in guest |
| B24 | — | ℹ️ Neovim preconfiguration (maintainer's home-manager config): nvim-treesitter is already the *main* rewrite here (no `configs.setup`, highlighting via `vim.treesitter.start`); catppuccin-nvim 2.0 names the lualine theme `catppuccin-mocha`; `with p;` in `withPlugins` bound `c` to the color table → grammars listed explicitly as `p.<name>`. QMP `send-key` uses US key positions (on QWERTZ, z becomes y). | Tests in guest 2026-10-16 |
| B25 | — | ℹ️ Resolution: upstream Cocoa reports the window size to the guest via `ui_info`; `qemu-cocoa-dynamic-display` (try-omarchy) adds Retina DPI to the EDID → Hyprland `scale = "auto"` picks 2. With `zoom-to-fit=on`, however, QEMU adopted the initial ~640×480 view → window 703×396. Own patch `qemu/patches/omaflake/qemu-cocoa-initial-size` (window follows the guest resolution until resized by the user) + `virtio-gpu…,xres/yres` = 75 % of the main screen in 16:9 → window 1920×1080 pt, guest 3840×2160 @2. | `CGWindowList`, `hyprctl monitors` |
| B26 | — | ✅ `omaf reset` keeps the instance's base image (`instance.json` → `image`); a newly built image only applies on `omaf new` or `omaf upgrade`. Intended (decided 2026-10-17, B52). | `reset gpu` after `build dev` |
| B27 | — | ℹ️ Without an app bundle, QEMU and `omaf-runner` show the generic executable icon in the Dock. `omaf-runner` sets `NSApp.applicationIconImage` itself; QEMU gets own patch `qemu-cocoa-dock-icon` (p4), which loads `OMAFLAKE_DOCK_ICON` – OmaflakeCore writes the icon to `~/.omaflake/.omaflake/dock-icon.png` and sets the variable for window starts. Icon data is embedded (`BrandAssets.swift`, generated by `brand/generate.py`), so no resource bundle is needed. Visible Dock icon: OPEN (check by eye). | `strings qemu-system-aarch64`, `ps -E` of the QEMU process, `BrandTests` |
| B28 | Data folder `~/.omaflake`, movable; relocatable QEMU | ✅ 2026-10-16: default data folder `~/.omaflake` (not Documents/Desktop – no privacy prompt); settings always in `~/.omaflake/config.json` (`home` = data folder); `DataFolder.move` renames only omaflake's entries (same volume), re-registers Nix GC roots (`nix build <store path> --out-link`), rewrites `imagePath`/`sharePath` and rebases qcow2 overlays (`qemu-img rebase -u`). Migrated `~/VMs` → `~/.omaflake`: 10 GC roots re-registered, overlay of `gpu` rebased, NixOS and macOS cycles with a chosen share folder (permissions kept). The own QEMU build had absolute install names into its prefix (`otool -L`) → `qemu/relocate.sh` (`@rpath`, re-signed with the hypervisor entitlement), called by `build.sh`. | `omaf config --home`, `otool -L` |
| B29 | Prerequisites, `omaf doctor`, QEMU as a Nix package | 2026-10-16: `Prerequisites` (OmaflakeCore) detects macOS/arm64, nix (`Settings.nix`, PATH, Nix/nix-darwin/Homebrew dirs), flakes (`nix config show` experimental-features), nix-darwin (`/run/current-system/sw/bin/darwin-rebuild`), Linux builder (`builders = @/etc/nix/machines`, a line with `aarch64-linux`), QEMU and 3D, flake, SSH key, git (never the `/usr/bin/git` shim without developer tools – it would open the CLT install dialog). App: Requirements page, opens on first launch. `qemu/package.nix` overrides nixpkgs' QEMU 11.1.2, virglrenderer 1.3.0, libepoxy; findings: nixpkgs adds Linux-only `libgbm`/`libdrm` with `openGLSupport` (filtered out); ANGLE's bottle names its libraries after `/opt/homebrew/opt/angle` (renamed in the derivation); several startergo virglrenderer patches have wrong hunk counts/path levels that only macOS' `patch` accepts → one clean re-diffed patch `qemu/patches/startergo/virglrenderer-1.3.0-macos.patch`. Size if QEMU were shipped in the app instead (measured on Homebrew QEMU + ANGLE): ~110 MB installed, ~45 MB compressed (64 MB UEFI firmware compresses to 1.6 MB) – not chosen (spec §11.2). | `omaf doctor`, `nix build .#qemu` |
| B30 | Person-specific values from omaflake, not from the flake | ✅ 2026-10-17: flavor flake input `omaflake-guest` (`path:./guest`, neutral defaults); omaflake writes `<data folder>/guest/settings.json` and passes it with `--override-input … --no-write-lock-file` – checked: the lock keeps `./guest`, a changed file takes effect at once, a flake without the input only warns (the app passes it only when flake.nix declares it). Without `local/` all flavors get the Mac's user, keyboard `de`, locale and keys; plain `nix eval` gives `guest`. Keyboard from the HIToolbox preferences: Carbon's input source API must run on the main thread (test run aborted with signal 6). The app cached the flavors from launch, so a changed macUser reached the Doctor but not the build – the build now evaluates the user fresh, Check Again reloads the flavors. | `nix eval … --override-input`, `swift test` |
| B31 | Same flavor name for both guest kinds | ✅ 2026-10-17: `flavors.toml` sections `[nixos.<name>]`/`[macos.<name>]`, modules in `flavors/nixos/`, `flavors/macos/`; flavor ID `<kind>-<name>` for `lib.flavorDefaults` (with `kind`, `name`), `images`, `darwinConfigurations`, image names, `current/` links and instances. CLI resolves a name, `<kind>/<name>` or an ID and asks when a name is ambiguous; app and lists show the name with the guest kind. Existing data migrated by hand (two `current/` links, one instance); older image folders keep their names. Checked: `nix flake check`, NixOS and macOS cycle via `nixos/minimal`, `macos/minimal`. | `omaf flavors`, cycle |
| B32 | Flavor author, image.json, fixed PCI slots | ✅ 2026-10-17: flavor identity `<author>/<kind>/<name>` (ID `<author>.<kind>.<name>`, `author` in flavors.toml, `local` for a developer's own); images are folders with `image.json` (NixOS: + `result` GC root), nothing is parsed from names; `ImageStore.migrateLegacy` converted the two existing images, `current/` links and one instance (overlay rebased), idempotent. The CLI asks in a terminal when several flavors match. A NixOS VM started headless after window starts fell into the UEFI shell: the window adds GPU/USB devices, the disk moved from PCI slot 0x7 to 0x6 and the efivars boot entry no longer matched → fixed slots for all devices + `bootindex=0` on the disk; checked headless and windowed on that VM. One NixOS image build failed once in the builder (`cptofs`: staging root missing after a virtiofsd warning) and succeeded unchanged on retry – sporadic, OPEN. | `omaf images`, cycle |
| B33 | The product deletes, the trash rule is the developers' | ✅ 2026-10-17: decided by the maintainer – `omaf rm/reset/upgrade`, `omaf images rm`, the ephemeral clone of the last run and failed builds/installations are deleted for real (`Disposal.delete`, read-only images and bases made writable first; share folders are kept); `omaf trash` removed; the app asks with "cannot be restored". Deleting images (new): refused while instances use one, `current/` moves to the next newest image of the flavor; app: "Remove Image …" and "Older Images" in the image row's menu. | `swift test` |
| B34 | Cancelling builds | ✅ 2026-10-17: `Cancellation` (task-local): `Shell` registers every process it starts, `cancel()` terminates the running one and runs handlers – the macOS build VM's runner is detached (setsid) and was left running by Ctrl-C before, now it is killed; the half-built folder is deleted. CLI: SIGINT handler (after the password prompt); app: Cancel for both guest kinds, also in the image row. Checked: NixOS (`nix build` gone, no image) and macOS cancelled while the build VM booted (runner gone, folder deleted). **Mistake on the way:** a CLI test build of the same image deleted the build folder of a build the maintainer had started in the app (the new "delete leftover build folder" step could not tell it was in use) – that build failed. → one build per image at a time: `FileLock` on `images/.build-<image>.lock` (app and CLI); only an unlocked leftover folder is deleted. Tests: check for builds of others before running build tests. | CLI, `swift test` |
| B35 | Sound and microphone | ✅ 2026-10-17: per instance `audio` (default on) and `microphone` (default off), also in flavors.toml; `omaf set`, app switches. NixOS: `-audiodev coreaudio` + `intel-hda` (slot 0x9) + `hda-output`, with microphone `hda-micro` (as try-omarchy, there with SDL) – checked: with microphone the guest has `pcm0p` + `pcm0c` and PipeWire offers sink and source, without it only `pcm0p`. macOS: `VZVirtioSoundDeviceConfiguration`, output to `VZHostAudioOutputStreamSink`, input `VZHostAudioInputStreamSource` only with the microphone – checked: guest shows "Apple Virtual Sound Device", 2 output channels, no input. Not checked: actual recording (would prompt for microphone permission) and hearing the output. App Info.plist: NSMicrophoneUsageDescription. | `/proc/asound`, `wpctl`, `system_profiler` |
| B36 | Clipboard for NixOS guests | ✅ 2026-10-17: design as try-omarchy (MIT) – virtio-serial port `org.omaflake.clipboard` instead of spice-vdagent (which is X11-only, spec §8 OPEN item resolved for NixOS). Mac: `omaf-runner bridge --clipboard <clip.sock>`, spawned detached by `QEMURuntime.start`, polls the NSPasteboard change count, writes what comes in; guest: `flavors/modules/clipboard-agent.py` as a systemd user service (`vm-guest.nix`, waits for the Wayland socket, udev gives the port to group users), `wl-paste --watch`/`wl-copy`. JSON lines {mime, data base64}, text and PNG, fingerprints against echo, nothing sent before the first copy. Checked: text both ways with umlauts, PNG Mac → guest byte-identical, bridge starts and ends with the VM. Bug on the way: `connectUnix`'s timeout stayed on the socket as SO_RCVTIMEO, so the bridge took 2 s of silence for "VM gone" and exited – cleared after connecting. Not checked: PNG guest → Mac. | `wl-paste`, `pbpaste` |
| B37 | Camera for NixOS guests | ✅ 2026-10-17: design as try-omarchy (MIT), own code. Per instance `camera` (default off), flavors.toml, `omaf new/set --camera`, app switches. Guest (`vm-guest.nix`): v4l2loopback as `/dev/video42` "Mac Camera" (`exclusive_caps`, a black first frame so browsers find it), system service `omaflake-camera` (`flavors/modules/camera-agent.py`) subscribes to v4l2loopback's client-usage event and sends "start"/"stop" over the virtio-serial port `org.omaflake.camera`. Mac: `omaf-runner bridge --camera <cam.sock>` (`Camera.swift`) runs AVFoundation only between start and stop, 1280×720 NV12, header "<4sBBHII" b"OMFC"; on connect it sends status "ready" so the guest repeats a lost "start". `omaf-runner` carries an embedded Info.plist (`-sectcreate __TEXT __info_plist`, `App/omaf-runner-Info.plist`) with camera and microphone usage descriptions; the app's Info.plist got NSCameraUsageDescription. Checked with the test pattern (`OMAFLAKE_CAMERA_TEST_PATTERN=1`): `v4l2-ctl` sees 1280×720 NV12, frames carry the moving pattern, ~15 fps (the pattern rate), the guest logs start on open and stop on close, the bridge ends with the VM. Real camera checked by the maintainer via the app (built-in camera, `mpv av://v4l2:/dev/video42` in the guest, macOS asked for permission for omaflake). Not checked: 30 fps throughput of virtio-serial (41 MB/s, **OPEN**), a browser in the guest. | `v4l2-ctl`, `journalctl -u omaflake-camera` |
| B38 | Linux builder disk full | ✅ 2026-10-17: an image build failed with "No space left on device" (qemu-img writing the ~9 GiB disk image). nixpkgs' builder profile already collects garbage, but only below `min-free` 1 GiB and up to `max-free` 3 GiB. `flavors/hosts/linux-builder.nix` raises them to 20/50 GiB (`virtualisation.darwin-builder.min-free/max-free`, checked with `nix eval` of the builder's `nix.settings`); docs/linux-builder.md. No app button: the builder is reachable only with sudo. Takes effect after `sudo darwin-rebuild switch`. | `nix eval`, `df` in the builder |
| B39 | mpv shows an empty window in the guest | 2026-10-17: the camera delivered frames (checked with `v4l2-ctl`), but mpv's default output tried Vulkan (virgl offers none – "vdrm_device_connect failed", libplacebo "Found no suitable device") and showed an empty window; `--gpu-api=opengl` and `--vo=wlshm` show the picture. `vm-guest.nix` writes `/etc/mpv/mpv.conf` with `gpu-api=opengl` (libmpv reads `/etc/mpv` – found in the nixpkgs build's libmpv). Checked with the new image: mpv logs `Setting option 'gpu-api' = 'opengl'` and an OpenGL context on Wayland. The camera picture through it: to check by the maintainer in a VM started from the app (a VM started from the agent's terminal gets no camera permission). | screenshots in the guest (`grim`), deleted afterwards |
| B40 | Admin tasks from the app | ✅ 2026-10-17: spec §11.5. `AdminShell` (macOS administrator dialog), `BuilderMaintenance` (apply `#linux-builder`, clean up the builder, root script with pipefail and a log the user owns, no flake.lock written by root), Doctor buttons, `omaf builder apply|clean-up` via sudo, marker `/etc/omaflake/linux-builder` in the builder host. First click failed: root (the dialog's shell) may not read the flake in `~/Documents` – macOS' privacy protection, "Operation not permitted". So "Apply" now does what `darwin-rebuild switch` does in two steps: the user builds `#darwinConfigurations.linux-builder.system` (`nix build --no-link`), root only sets the system profile and runs `activate`. Checked: build, tests (quoting, commands), the user-side build of the builder host. Not checked: Touch ID in the dialog. | `swift test` |
| B41 | Doctor and main window polish | ✅ 2026-10-17: the whole Doctor row opens its sub-line (the detail text no longer takes the click for text selection); "Guide" opens the Linux builder guide in its own window from the app bundle (`Resources/Guides/`, a small Markdown renderer – works offline, links into the repo stay plain text); the Doctor's header no longer says "install in Terminal", its footer shows the app version; after "Apply Settings" the Doctor waits until the restarted builder answers before checking again (it showed red right after); Recent Activity replaces the single message banner in the main window (newest first, time, one "Clear History" button); tooltips after 150 ms instead of about a second (`NSInitialToolTipDelay` registered for this app). First image build after raising `min-free`: auto-GC freed 38 GB and took paths the running build needed ("3 dependencies failed"); the retry built fine – **OPEN** whether that happens again. | by click (maintainer) |
| B42 | Navigation instead of windows; brand look | ✅ 2026-10-17: closing an extra window (Doctor) also closed the menu window, since the click went elsewhere. All pages now open inside the menu window (`AppModel.path`, `PageHeader` with back arrow and Esc), one fixed size 500 × 600 pt for every page (the window grew and shrank before), the first launch opens on the Doctor. Brand look from brand/README.md: `#121212` with a faint red/cyan glow, cyan tint, rounded cards with a hairline border, wordmark with red/cyan offset. Checked by the maintainer (2026-10-17): navigation, size and look. | by click (maintainer) |
| B43 | lab = development flavor | ✅ 2026-10-17: lab is no longer the isolated playground (maintainer's decision) but the flavor to develop with: shared network, share folder, ephemeral, camera and microphone on; "omaflake Test" (GTK 4 app, `flavors/nixos/lab/omaflake-test.py`, Cmd+D → fuzzel): status of camera/sound/microphone/clipboard, camera in mpv, test tone, 4 s recording with level and playback; debug tools (v4l-utils, mpv, ffmpeg, pavucontrol, pactl, alsa-utils, mesa-demos, vulkan-tools, wayland-utils, wl-clipboard, grim, strace, lsof, …). v4l-utils left the other images. GTK 4 also tries Vulkan first under virgl → the app sets `GSK_RENDERER=ngl`. Checked: image builds, tools and desktop entry present, app window renders in a 3D window (screenshot in the guest). The maintainer checked the app in a lab VM (2026-10-17). | `grim` in the guest, by click |
| B44 | One corner radius; New VM chooses an image | 2026-10-17: maintainer's wish – then window corners like Finder/Safari: the window 24 pt (`BrandStyle.windowRadius`, by eye – Apple documents no value, **OPEN** whether it matches exactly), cards and buttons concentric with it (24 − 14 pt margin = 10 pt, `.buttonBorderShape`), buttons no longer capsules; the menu window's system background (larger radius) is hidden (`RoundedWindow`, clear window, its `NSVisualEffectView`s hidden) – whether macOS still clips at its own radius: to check by the maintainer. "New VM" picks an image ("Vorlage"), listing only flavors with an image, with a hint and "Build Image …" when there is none. | by click (maintainer) |
| B45 | Guest settings from lists; margins | 2026-10-17: language/region, keyboard and time zone are pickers ("Like this Mac" = the Mac's value): 152 UTF-8 locales from glibc 2.44 `localedata/SUPPORTED` (NixOS builds only those), 16 XKB layouts, IANA zones from `TimeZone`. So that every XKB layout works on the console too, `base.nix` now sets `services.xserver.xkb.layout` and `console.useXkbConfig` instead of `console.keyMap` (checked: `console.keyMap` becomes the generated `xkb-console-keymap`). macOS guests still know only de/us (BM24). Content 20 pt from the window edge; the version is the Doctor's first row. Checked by the maintainer (pickers, margins); console keymap from XKB checked in a lab VM (`/etc/vconsole.conf`, `localectl`). | `nix eval`, by click |
| B46 | Guest choices = what both guest kinds support | ✅ 2026-10-17 (maintainer's wish): one choice, both meanings. Locales: glibc can build it **and** macOS knows the region (`Locale.availableIdentifiers`) **and** macOS has a user interface in the language (Finder's .lproj, macOS 27; nb counts as no). Keyboards: 16 XKB layouts, each mapped in `darwin-guest.nix` to the country's default macOS layout – names and IDs read from macOS 27's own `AppleKeyboardLayouts-L.dat` (table of name offset + ID; it confirms BM24's de = German-DIN-2137/−18133 and us = ABC/252); a test checks that every offered keyboard has a macOS mapping. Time zones: macOS's list (`TimeZone.knownTimeZoneIdentifiers`), all in NixOS' IANA database. Not checked in a macOS guest: the 14 new layouts (only de/us were, BM24). | `swift test`, data file |
| B47 | New VM kept the previous image's settings | ✅ 2026-10-17: after "+" and choosing another image, camera and disk were missing – the form still had the first image's settings (a macOS one → macOS form). `select` compared with `flavor`, which the picker had already changed; now it compares with the flavor whose settings are loaded. A typed name stays, the suggested one follows the image. | by click |
| B48 | Clipboard for macOS guests | ✅ 2026-10-17: transport decided – a virtio console port (`VZVirtioConsoleDeviceConfiguration`, port name `org.omaflake.clipboard`, `VZFileHandleSerialPortAttachment` on one end of a socket pair) appears in the macOS guest as `/dev/cu.org.omaflake.clipboard` / `tty.…` (crw-rw-rw-); a serial tty, so the agent sets raw mode. The Mac side is the existing `ClipboardBridge`, inside `omaf-runner run --clipboard` (writes off the main queue with a 1 s send timeout: a guest without a reading agent never blocks the runner). Guest: `flavors/modules/clipboard-agent-macos.py`, started by omaflake over SSH once the guest answers (`VZBackend.waitForSSH`, next to the host name; command `omaflake-clipboard-agent` from `darwin-guest.nix`) – first built as a LaunchAgent, changed because of BM25 (macOS announces every LaunchAgent as a background item in each clone). An SSH session reaches the logged-in user's pasteboard (checked). Checked in a `macos/minimal` VM with the agent started by hand: text both ways with umlauts, PNG Mac → guest byte-identical, guest → Mac arrives. Checked by the maintainer in a built `macos/lab` image (2026-10-17): both directions. App: the clipboard switch is on for macOS VMs too. | `ls /dev`, `pbpaste`, `shasum` |
| B49 | Camera for macOS guests – Mac side ready | 2026-10-17: `omaf-runner run --camera [--camera-test-pattern]` adds a second console port `org.omaflake.camera` (guest: `/dev/cu.org.omaflake.camera`) and runs the same `CameraBridge` as for NixOS in the runner; `VZBackend` passes it for instances with `camera` (`omaf new --camera` on macOS says the guest side is missing). Checked with the test pattern in a `macos/minimal` VM: "start" from the guest → frames with the OMFC header, 124 MB in 6 s = 20.7 MB/s = the pattern's 15 fps (the port is not the limit there; 30 fps = 41 MB/s **OPEN**). **Missing (M12):** the guest side – a CoreMediaIO camera extension (`CMIOExtension`) inside a host app in the image: its stream source's start/stop map 1:1 to the protocol's "start"/"stop", frames come from the port. A system extension needs Developer ID signing, the `com.apple.developer.system-extension.install` entitlement and approval in the guest's System Settings; testing before M12 with SIP off in the guest (`systemextensionsctl developer on`) is possible in principle – whether a free Apple account can sign it: **OPEN**. | `/tmp/camprobe2.py` in the guest |
| B50 | lab for macOS guests | 2026-10-17: `[macos.lab]` (minimal + tools, ephemeral, microphone on) with "omaflake Test" – an app bundle from a shell script with AppleScript dialogs (no compiler in the image): sound (two system sounds), microphone (4 s with ffmpeg/AVFoundation, peak level, playback; NSMicrophoneUsageDescription in its Info.plist), clipboard status; in the Dock and via Spotlight. Checked: the darwin system builds, contains the app (valid Info.plist), the clipboard agent command, ffmpeg with the avfoundation input. Not checked: the image (building a macOS image needs the guest password – maintainer) and the app's dialogs in a guest. | `nix build …darwinConfigurations."omaflake.macos.lab".system` |
| B51 | Flavor descriptions, lab mark, developer mode | ✅ 2026-10-17 (spec §11.7): flavors.toml `description`, `description_de`, `lab = true`; `FlavorDefaults.description(in:)`, `isLab`. App: "i" with the description as tooltip next to a flavor's name, the description below the New VM/Build pickers, a flask for lab flavors; Settings → Developer Mode (config.json `developerMode`) – without it lab flavors are hidden in lists and pickers (their VMs still show). CLI: `omaf flavors` hides lab flavors unless developer mode or `--all`, `--long` prints descriptions. | `omaf flavors --long`, by click |
| B52 | Image retention | ✅ 2026-10-17 (M6): after a successful build (CLI and app) a flavor keeps its current image, the one before it (`ImageStore.keepPrevious` = 1, to go back) and every image a VM uses; older ones are deleted (`ImageStore.prune`, printed/logged). Pruning is skipped when the instances cannot be read. B26 decided: `reset` keeps the instance's image on purpose (back to where it started); a new image comes with `upgrade`. Images from before this rule are pruned at their flavor's next build. | `swift test` (RetentionTests) |
| B53 | Image versions in the app | ✅ 2026-10-17: an image row opens (chevron or click) into all its versions, newest first: date, revision, the VMs on it, the current one marked; each can be deleted unless VMs use it (the "Older Images" menu is gone). | by click (maintainer) |
| B54 | Setup by click: Nix and nix-darwin | 2026-10-17 (spec §11.5): installer decided – the official installer of the NixOS community (`github.com/NixOS/nix-installer`, beta): `curl -sSfL https://artifacts.nixos.org/nix-installer \| sh -s -- install --no-confirm`, with an uninstaller (`/nix/nix-installer uninstall`) and a receipt; it installs upstream Nix, which fits nix-darwin's `nix.enable = true` (Determinate Nix would need `nix.enable = false`). Doctor: Nix "Install …" and nix-darwin "Set Up …" through the administrator dialog; `omaf builder setup` via sudo. nix-darwin in two stages as docs/linux-builder.md says: `darwinConfigurations.linux-builder-bootstrap` (default builder – exactly `nixpkgs#darwin.linux-builder` from the binary cache; only three small darwin derivations build locally, checked) then the full host; before the first activation the installer's `/etc/nix/nix.conf`, `/etc/bashrc`, `/etc/zshrc`, `/etc/zshenv` are renamed `*.before-nix-darwin` (test). Checked: both systems build as the user; `omaf builder setup` refuses where nix-darwin exists. **Not checked (needs a Mac without Nix):** the installer run and the first activation – a macOS VM cannot run the Linux builder (no nested virtualization for macOS guests), so stage 2 needs a real Mac; stage 1 can be tried in a macOS VM from the base. **OPEN:** pin the installer version and check its checksum; the flavors flake's location for users without a checkout (M12, preset flavors). | `nix build …linux-builder-bootstrap.system`, `swift test` |
| B56 | Image row: two icons, default version | ✅ 2026-10-17: the "…" menu is gone – "New VM" (greyed out without an image), then the hammer, with a small refresh mark once an image exists (a build makes a new version); in the version list a click on a circle makes that version the default (`current/<flavor>`, new VMs and upgrades use it; `ImageStore.setCurrent` public). | by click (maintainer) |
| B55 | ANGLE from nixpkgs; `--gl es` crashed | ✅ 2026-10-17 (M12 licence item): the QEMU package no longer uses the prebuilt ANGLE bottle without a licence file but nixpkgs' `angle` (built from source, BSD-3-Clause, in the binary cache). On the way: QEMU from the Nix package aborted with `--gl es` ("Couldn't open libEGL.dylib") – also before the change; libepoxy opens ANGLE by bare name, which dyld finds only in Homebrew's prefix. epoxy's `dispatch_common.c` now names ANGLE's store paths. Checked: a lab VM in a 3D window with `--gl on` (virgl, OpenGL 4.1) and `--gl es` (starts; guest core profile falls back to llvmpipe, as B22 expects for ANGLE). `~/.omaflake/qemu/nix` points to the new build. | `eglinfo -B` in the guest, `qemu.log` |
| B57 | Name checks | 2026-10-17: whois – omaflake.com, .org, .io and .de are free; .dev/.app: the registry's whois gave no answer (**OPEN**). A web search finds no product or company "omaflake" (only unrelated cereal brands like Mornflake). Trademark registers (DPMA, EUIPO, USPTO) not searched – **OPEN** (maintainer, or before registering the domain). | `whois`, web search |
| B58 | Build page: layout, progress, log | ✅ 2026-10-17: the "Build Image" page in three rows – the flavor with its guest kind; password (macOS) and Build/Cancel; progress – instead of one overflowing row. `BuildProgressTracker` (OmaflakeCore, tests) reads the log: NixOS 4 steps (evaluate, build packages, write the disk image, copy from the builder), macOS 7 (system on the Mac, clone, boot, Nix in the guest, copy, activate, shut down); while Nix builds or downloads it counts the announced packages ("these N derivations will be built" / "… paths will be fetched" against "building '…'" / "copying path … from https"). The page shows "2/4", the step, "37/57" and a bar over all steps. The log is a terminal: near-black, monospaced, errors red, warnings yellow, Nix steps green, build output grey, omaflake's own lines cyan. | `swift test` (BuildProgressTests), by click |
| B59 | Menu window closed on questions; build liveness | ✅ 2026-10-17: the menu window is a menu bar panel – it closes when another window takes the focus. Questions (remove image/VM, reset, upgrade, quit with running VMs, data folder, Nix/nix-darwin/builder) are now asked inside the window (`AppModel.confirm` async, `ConfirmationOverlay`, Esc/Enter) instead of in an alert window. Still closing it: macOS' own windows – the privacy prompt ("access Documents"), file pickers, the administrator dialog. The Documents prompt comes from the flavors flake in ~/Documents and, while developing, again after every rebuild (ad-hoc signature); a signed app with its flavors outside Documents asks at most once. Build page: a spinner, the running time and "last output … ago" (orange after two quiet minutes, tooltip: some steps print nothing for minutes). | by click |
| B60 | Build queue | 2026-10-17: builds are jobs (`BuildJob`: state, log, progress, password only until start); one build per guest kind at a time, NixOS and macOS side by side; further builds wait in the queue ("Add to Queue"). Each job shows as a sub-row of its image (waiting / step with packages and bar / failed), a click opens the build page with its log; cancel or remove from the queue there or in the row; a failed job stays until hidden. Leaving the build page no longer loses the build. Not checked by click yet. | by click |
| B61 | macOS lab build: "No SSH after 10 minutes" | 2026-10-17, maintainer's build at 15:43: the build VM (a clone of `mac-base-26A434`) never answered on SSH and was forced off. Reproduced the clone by hand twice: DHCP lease and SSH within 11 s (fresh clone, first boot) – the base is fine, the cause is not found. The error now says what was missing: no DHCP lease, port 22 closed, or open without an SSH greeting (`Ports.isOpen`). The retry at about 16:30 built `macos/lab` without trouble; the cause of the first failure stays **OPEN** (the new message will name it if it comes back). | by hand: `omaf-runner clone/run`, `/var/db/dhcpd_leases`, `nc` |
| B62 | Version list did not follow a delete | ✅ 2026-10-17: deleting a second image version left it in the list until the row was closed and opened – the list was read from disk while drawing, which SwiftUI does not observe. The versions are now observed state (`AppModel.imageVersions`, refreshed with the VM list and after every delete). | by click |
| B63 | Build page stays with its build | ✅ 2026-10-17: once a build is started on the page (or the page opens for a flavor whose build waits or runs), the flavor picker becomes a label – no switching to another flavor there; back and "+" or a hammer give a fresh page with the picker. | by click |
| B64 | macOS lab: clipboard agent not running, test app hard to find | 2026-10-17: in the maintainer's `macos/lab` VM the port was there but the agent had never started – the app started it only once, right after the VM start, with no second try (cause of that miss not found; `omaf wait` started it at once). Now the app checks at every refresh (2 s) that each running macOS VM with clipboard has its agent, remembered per runner process (`AppModel.ensureClipboardAgents`) – also after an app restart. The test app's status asked `launchctl` (left from the LaunchAgent version) and always said "not running"; it asks for the process now. The app got omaflake's icon (a copy kept in sync by brand/generate.py) – in the Dock it was a plain generic tile, though pinned. Sound and microphone in `macos/lab` checked by the maintainer. Checked after the rebuild: the app started the agent about a minute after boot (`ps` in the guest); the test app's status was read once when it opened, before that – it now refreshes every 5 s. Rebuild `macos/lab` for that. | by hand in the VM, by click |
| B65 | Opening omaflake.app brought a VM window forward instead | ✅ 2026-10-17: `omaf-runner` lay in `omaflake.app/Contents/MacOS`; a runner with a VM window registers with LaunchServices under the enclosing bundle's path, so `open omaflake.app` (Finder, Dock, Spotlight) activated the VM window and the app did not start while such a VM ran (`lsappinfo` showed the runner under omaflake.app). The runner is now its own helper app, `Contents/Helpers/omaf-runner.app` ("omaflake VM", bundle id `app.omaflake.runner`, signed with its entitlement before the outer bundle); `VZBackend.locateRunner` looks there first. VMs started before the update keep the old registration until they stop. Found while checking B64: the clipboard agent started by the app works once the app really runs (checked). | `lsappinfo list`, `codesign -d --entitlements` |
| B66 | nix-installer pinned | ✅ 2026-10-17: setup by click downloads `nix-installer-aarch64-darwin` of release 2.35.3 (2026-10-14) and checks its SHA-256 from the release (`c4991562…`) before it runs (`set -e`, test); `OMAFLAKE_NIX_INSTALLER_URL` points to a mirror. Checked: the download matches, `--version` 2.35.3, `install --help` has the macOS planner and `--no-confirm`. B54's OPEN item done. | `shasum -a 256 -c`, `swift test` |
| B67 | CI/CD | ✅ 2026-10-17: `.github/workflows/ci.yml` – flakes job (Linux: `nix flake check --all-systems --no-build` for both flakes, guest scripts parse; checked locally on a clean tree without `flavors/local/`) and a macOS job (build, test, ad-hoc bundle as artifact; while the repository is private only by hand – macOS minutes count tenfold); `qemu.yml` – builds `.#qemu`, fills Cachix or an S3 cache when configured (by hand, tags, weekly); `release.yml` – on `v*` tags: version check against Info.plist, Developer ID keychain, `bundle-app.sh` with `SIGN_IDENTITY` (hardened runtime, timestamp), `scripts/notarize.sh` (notarytool, stapler), zip + SHA-256 + `update.json`, draft release; without the secrets an unsigned draft. Runner entitlements gained camera and audio-input (needed under the hardened runtime). actionlint clean. First runs (2026-10-17): the flakes job is green on GitHub. The macOS job fails – GitHub's newest image (`macos-26`) has Xcode 26.6 / Swift 6.3, no macOS 27 SDK (`VZMacGuestProvisioningOptions` missing); the job now checks the SDK and says so. GitHub has a preview image labelled `xcode-27` (macOS 27.0.1, Xcode 27.0 default, 27.1 RC, 27.2 beta – maintainer's hint; there is no `macos-27` label) → the macOS jobs use it with its default Xcode; `MACOS_RUNNER` can still point to another runner. ✅ Run 39047859484 (2026-10-17): flakes job and macOS job green on `xcode-27` – macOS 27.0.1, Xcode 27.0, build, 74 tests, ad-hoc bundle. On the way: Swift 6.3 rejects `@main` in a file named `main.swift` – renamed to `Omaf.swift` / `Runner.swift`. Signing and notarization need the account (M12). | `actionlint`, local `nix flake check` |
| B68 | Update check | ✅ 2026-10-17 (spec §11.6): `UpdateCheck` reads a JSON feed (`update.json` from the release workflow) from Info.plist `OMUpdateFeedURL` (unset until the first release) or `OMAFLAKE_UPDATE_FEED`; the Doctor shows "no feed yet", "up to date – checked …", or "0.2.0 available" with "Download …"; at launch and once a day; Settings → "Check for updates" switches it off (config.json `checkForUpdates`). Installing the update itself: with signed releases (M12). | `swift test` (UpdateCheckTests) |
| B69 | File pickers as sheets | 2026-10-17: Set Path (Doctor), share folder (New VM), IPSW (macOS base) and data folder (Settings) open their panel as a sheet on the menu window instead of a separate window, which closed the menu window (B59). Whether the menu window now stays open: to check by click (maintainer). | by click |
| B70 | Camera throughput measured | ✅ 2026-10-17: test pattern with `OMAFLAKE_CAMERA_TEST_FPS` (new, default 15). NixOS (QEMU virtio-serial → v4l2loopback, `v4l2-ctl`): 300 frames of 1280×720 NV12 in 9.95 s at 30 fps (≈ 41 MB/s) and in 4.98 s at 60 fps (≈ 83 MB/s) – 30 fps fits easily. macOS (Virtualization.framework virtio console, Python reader in the guest): ≈ 37 MB/s asked at 30 fps, ≈ 34 MB/s at 60 → about 26–27 fps at 1280×720; enough for the camera extension (M12), which may read faster than the Python probe or send 25 fps. B37's and B49's OPEN items done. | `v4l2-ctl`, `/tmp/camprobe2.py` in the guest |
| B71 | Release dry run | ✅ 2026-10-17: the release job's steps run locally – tag/Info.plist check, unsigned zip, SHA-256, `update.json` (version, URL, checksum, page); the zip unpacks to a bundle that passes `codesign --verify --strict`, with the runner helper inside. Push of the workflows: refused – the developer token lacks GitHub's `workflow` permission (maintainer). | local |
| B72 | macOS research flavor | 2026-10-17 (maintainer's choice: TeX Live from Nix, VS Code for LaTeX, terminal setup, Typst): `flavors/macos/research.nix` on top of minimal – `texliveFull` (latexmk, biber, …), VS Code with LaTeX Workshop, LTeX+ (language from the guest locale, ltex-ls-plus from Nix), tinymist, GitLens, nix-ide (unfree allowed only for VS Code); WezTerm (JetBrains Mono Nerd Font, opens Zellij session "main"), Zellij, Typst, texlab, pandoc, lazygit; neovim with vimtex; Homebrew via the new flake input `nix-homebrew` (`/opt/homebrew/bin` on PATH); WezTerm and VS Code in the Dock; user configs written once (never overwritten). Checked: the darwin system builds (9.2 GiB closure) with all binaries, both apps, the font, Homebrew setup and the config files in its activation. The maintainer built the image after B74–B76 (2026-10-17). | `nix build …"omaflake.macos.research".system`, image build |
| B73 | New flavors appeared only after a restart | ✅ 2026-10-17: the app read the flavors at launch and with Doctor checks; a flavor added to flavors.toml (macos/research) stayed missing. The refresh (2 s) now compares the modification dates of flavors.toml and local/flavors.toml and reloads the flavors after a change. | by click |
| B74 | macOS image build: Nix installation slow | 2026-10-17, maintainer's `macos/research` build: "Installing Nix" for over 15 minutes – not stuck: the installer (nixos.org, `--daemon`) ended with `nix-channel --update nixpkgs`, downloading the whole channel. Images use flakes, so the build installs without a channel now (`--no-channel-add`, checked in the 2.34.9 installer); the log says the step takes a few minutes. | `ps` in the build VM, installer source |
| B75 | macOS image build: closure copy stood still | 2026-10-17, maintainer's `macos/research` build (9.2 GiB closure): `nix copy --to ssh-ng://…` stopped after 632 MB – Mac and guest idle (0 % CPU, no SSH traffic), a lock on a TeX path in the guest store; cause not found. The copy now has a watchdog (`MacImageBuilder.copyClosure`): `nix copy -v`, no new path for 5 minutes → stop and copy again (already copied paths are skipped), attempts 2 and 3 with the older `ssh://` protocol (`nix-store --serve`); every 100 paths a log line. **To verify:** the next research build. | `ps`, `df`, `nettop` |
| B76 | macOS image build: slow downloads in the build VM | 2026-10-17: the second research build again took long in "Installing Nix": a fresh macOS guest downloads its aerial wallpapers (`WallpaperAerial`, > 400 MB) and assets (`nsurlsessiond`, `softwareupdated`) right after boot, and Nix's own download (≈ 30 MB tarball) crawled meanwhile (`nettop` in the guest). The Mac now downloads Nix's release tarball once into `<data>/cache`, checks it against the SHA-256 nixos.org publishes beside it, and copies it into the guest over SSH; the guest installs from it (`./install --daemon --yes --no-channel-add`) without the internet. The closure comes from the Mac anyway (B75). Stopping the guest's own downloads (wallpaper, assets) in the base: **OPEN**. | `nettop`, `ps` in the build VM |
| B77 | macOS research: VS Code and WezTerm could not start | ✅ fix 2026-10-17: the activation wrote the starting configs with `install -d -o <user>` as root – only the last folder got the user, the ones on the way (`~/Library/Application Support/Code`, `~/.config`, `~/.local`, `~/.local/share`) stayed root's (checked in the maintainer's VM), so VS Code ("EACCES … Code/Backups") and WezTerm could not write their data. Files and folders are now created as the user (`sudo -u`); folders an earlier activation left to root are given back. The darwin system builds with it; the image needs a rebuild (or `sudo chown -R <user>:staff` on those folders in a running VM). | `stat` in the guest |
| B78 | WezTerm in macOS guests | ✅ 2026-10-17: after B77 WezTerm still did not open – its log: "Failed to create window: failed to create NSOpenGLPixelFormat". The paravirtual GPU of a macOS VM offers Metal, not OpenGL. `wezterm.lua` sets `front_end = 'WebGpu'` (Metal); checked in the maintainer's research VM: WezTerm starts, Zellij session "main" runs. | WezTerm log in the guest |
| B79 | NixOS build failed after the builder's auto-GC | ✅ 2026-10-17: a second time (after B41) a large image (research with TeX Live) failed because the Linux builder collected garbage mid-build (min-free) and removed paths the build still needed ("path … ran_toks … is not valid"); the retry built fine. `ImageStore.build` now notices "running auto-GC" in the log and, after a failure, builds once more by itself (log: "The Linux builder cleaned up during the build – building again"). | build logs |
| B80 | NixOS research flavor | ✅ 2026-10-17: TeX Live full (`texliveFull`, was medium), Typst with tinymist, texlab; neovim (home-manager, research only) with vimtex (latexmk continuous, view in zathura, SyncTeX), `<space>tp` = `typst watch` + zathura, LSP texlab and tinymist (`vim.lsp.enable`, nvim-lspconfig); zathura in the desktop colours with SyncTeX. A neovim-only PDF view is not possible in a terminal – zathura beside neovim under Hyprland gives the live view. Checked in a VM: latexmk builds a PDF with biblatex, `typst compile` too, neovim reports vimtex view "zathura" and both LSP configs, zathura/texlab/tinymist present, zathurarc with synctex. Not checked: the live view by eye (window). | in the guest over SSH |
| B81 | Mac keyboard layout in NixOS guests | ✅ 2026-10-17 (maintainer: "it is a Mac app"): NixOS guests used the PC layout – `\` was AltGr+ß, and the left Option key arrived as Alt, so in neovim Alt+key left insert mode. `base.nix` now sets the XKB Mac variant (`de` → `mac`, `ch` → `de_mac`; 13 of the 16 offered layouts have one, es/pl/cz not) with `lv3:alt_switch` (both Option keys for special characters, as on the Mac); Hyprland takes layout, variant and options from it (`osConfig`), the console follows XKB. Checked: `xkbcli how-to-type` – `\` Shift+Option+7, `@` Option+L, `{` Option+8, `[` Option+5, `|` Option+7, as on the Mac; in a lab VM Hyprland's active keymap is "German (Macintosh)". | `xkbcli`, `hyprctl` |
| B82 | Images that their flavor has outgrown | ✅ 2026-10-17: `ImageStore.expectedStorePath` evaluates what a build would give now (`nix eval` of `images.<id>` / `darwinConfigurations.<id>.system`, with the guest settings – nothing is built) and compares it with `storePath` in image.json. App: checked in the background at launch and whenever a file in the flake folder or the guest settings changes (newest modification date, every 5 s) and after builds; an image row then says "Flavor changed – build a new version" (orange), each version older than its flavor is marked "older flavor" (so it can be deleted). CLI: `omaf flavors --check` (≈ 10 s per flavor). Checked with the CLI: macOS minimal, macOS research and NixOS minimal outdated (today's changes), both freshly built lab images current. | `omaf flavors --all --check` |
| B83 | Build log too verbose | ✅ 2026-10-17: a NixOS research build logged 155 667 lines – 135 595 "error while reading directory" and 6 737 "copying path" from the disk image tool (routine, every build), 6 280 downloads and the package lists. `BuildLog.isNoise` leaves those out of the app's and the CLI's log (`omaf build --verbose` shows all); the progress still counts every line; the app writes the full log to `<data>/logs/image-build-<flavor>.log` ("Full Log …") and says how many lines it hid. On that log: 125 of 155 667 lines remain (the disk image tool also lists every store path – hidden too). | real build log, `swift test` |
| B84 | Stale build lock files | ✅ 2026-10-17: every macOS image build left an empty `images/.build-<image>.lock` (eight by today). The per-build lock now removes its file when the build ends (unlinked while still held); the old ones went to `~/.omaflake/_to_delete/`. | `ls -a ~/.omaflake/images` |
| B85 | Empty share folders went on piling up | ✅ 2026-10-17: removing a VM kept its share folder – 24 were left, none with the user's files, only metadata macOS guests write (`.fseventsd`, `.Trashes`); moved to `_to_delete`. Removing a VM now also deletes its default share folder when it holds nothing but such metadata; with files in it, or a folder the user chose, it stays (test). | `swift test` |
| B86 | VM row: share folder icon | ✅ 2026-10-17: a folder icon in the VM row opens its share folder (VMs with sharing, running or not); "Terminal (SSH)" and "Show Share Folder" left the "…" menu – the terminal icon of a running VM does the same. The menu lists "Shared Clipboard" first, apart from Sound, Microphone and Camera. | by click |
| B87 | VM row: two start icons | ✅ 2026-10-17: two start icons, a terminal (headless, left) and a monitor (window, the VM symbol), each with a cyan play sub-symbol; while the VM runs, the terminal (SSH) and stop take their places, so the row keeps the same icons. The flavor's `display` no longer picks a default start in the app. The "…" menu keeps only settings and management: shared clipboard, then sound/microphone/camera, then reset/upgrade/remove. | by click |
| B88 | Colours for start, stop and remove; "Entfernen" | ✅ 2026-10-17: the play sub-symbols are cyan, stop and the image trash are red (brand colours); "Remove …" in the VM menu sits below its own divider, role destructive with red text (whether the macOS menu shows the colour: OPEN, check by click). Remove questions answer with a red "Remove" button. German: "Entfernen" instead of "Löschen" for every remove action and its result; the warnings still say the data "wird gelöscht", since that is what happens. One `swift test` run failed once and passed on four reruns – flaky test: found and fixed in B104. | by click |
| B89 | Image versions: build details | ✅ 2026-10-17: an "i" next to the trash opens the version's details inline under its row (a popover could close the menu window, like alerts in B59): flavor, build time, flavor state ("-dirty" = uncommitted changes), status (default / older flavor / matches – only once the check ran), VMs on it, size on disk (`ImageStore.diskUsage`: allocated blocks, symlinks followed, computed in the background), source, store name, image name; fixed label column and one-line values (a Grid widened the content with long paths). No folder icon (decided). The nixpkgs version and the macOS base of an image are not recorded in image.json yet: OPEN. | by click |
| B90 | Home list jumped sideways | ✅ 2026-10-17: with a mouse attached macOS uses legacy scrollers, which take width once the content is taller than the window (e.g. when build details open). The home scroll view now always uses overlay scrollers (`OverlayScrollers`, set again when the system preference changes). Whether SwiftUI keeps the style on every update: check by click. | by click |
| B91 | Main window as tabs; recipes; notifications | ✅ 2026-10-17: four tabs instead of one long page – VMs, Images (built), Recipes (what can be built, with the hammer), Activity – each its own scrolling list, with badges (running VMs, build running/failed, recipe changed, unread messages); last tab remembered, first launch opens Recipes. The newest message shows for 6 s at the bottom. System notifications (`Notifier`, Settings switch `notifications`, nil = on) while the menu window is closed: VM ready once SSH answers (both guest kinds; also after "create and start"), image ready, build failed, VM ended on its own (not after the app's own stop). UI says "Recipe"/"Rezept" for flavor; Core/CLI strings keep "flavor" in English until the CLI rename (OPEN, before M12). Notifications from the ad-hoc signed app: check by click. | by click |
| B92 | Rename flavor → recipe | ✅ 2026-10-17: the maintainer chose "recipe" (German "Rezept") for what was called a flavor: recipe → image → VM; "flake" and "image" stay. **Renamed:** the flake folder `recipes/` (was `flavors/`), `recipes.toml`, `local/recipes.toml`, `local/recipes/<kind>/<name>.nix`, the flake output `lib.recipeDefaults`, the CLI (`omaf recipes`, `omaf new --recipe/-r`, column headings, hints), Core and app strings, Swift identifiers (`RecipeID`, `RecipeDefaults`, `recipe` properties), docs, CI. **Kept for compatibility:** `omaf flavors` (listed as an alias in the help) and `--flavor/-f` (hidden) still work; `instance.json` and `image.json` are written with `recipe` and still read with the old key `flavor`; a `flake` setting in config.json that points to a `…/flavors` folder that no longer exists is moved to the `recipes` folder next to it on the next start (CLI and app). Recipe IDs and image names are unchanged (`omaflake.nixos.research`), so images stay current. **Not renamed:** the findings B1–B91 above (they describe what was then). Each developer moves their git-ignored `flavors/local/` to `recipes/local/` by hand (`git mv` does not move ignored files). | ✅ |
| B93 | Images tab: built images at the top level | ✅ 2026-10-18: the Images tab lists every built image on its own row instead of recipes with versions below: full image name, guest kind and recipe, build time, the VMs on it (or "no VM"), an orange "outdated" when the recipe evaluates differently now; the circle makes an image its recipe's default, "New VM" sits on the default image only, info and trash as before. Builds that wait, run or failed are rows of their own on top, with the recipe's name. Sorted macOS, then NixOS, by recipe, newest first. The Recipes tab stays as it is until the marketplace (spec §11.3, direction 2026-10-18). | by click |
| B94 | One way for every list row; VM switches in the row | ✅ 2026-10-18: VMs, images and recipes open with a click on the row (chevron on the right, `ExpandableRow`) into a details box of the same look (`DetailBox`, `InfoLine`). VM rows show small icons for what is on (clipboard, sound, microphone and camera in orange, ephemeral, isolated/no network) – what is off has no icon. The VM's "…" menu is gone: its details hold the switches as chips (clipboard | sound, microphone, camera; with "takes effect at the next start" while it runs), image, recipe, resources, network, SSH command, share folder, ephemeral, created, and the buttons reset, switch to the newer image (only when there is one) and remove (red). Image rows open into their build details (the "i" is gone), recipe rows into description, ID, resources, network, what new VMs start with and their images. Guest agent for both guest kinds: direction in spec §11.9. | by click |
| B95 | One symbol per thing, the action as sub-symbol | ✅ 2026-10-18: VM = monitor, image = package, recipe = book; an action is that symbol with a small cyan sub-symbol: new VM = monitor + plus (section header and image row), start = terminal/monitor + play, build image = package + hammer (section header and recipe row; it replaces the hammer and the "plus on two layers" symbol). Own drawn icons for VM, image, recipe and the actions: later (maintainer, 2026-10-18). | by click |
| B96 | omaflake agent: design and NixOS prototype | ✅ 2026-10-18: design in spec §11.9 – Rust in the guest (nixpkgs Rust 1.98.1 cached; Swift 6.2.4 for aarch64-linux is one output with a 5.4 GB closure), one multiplexed port `org.omaflake.agent`, framed protocol v1 (`OMFA` header, hello/welcome/status/goodbye, camera on credits later), trust rules, migration in five steps, update only with the image. **Prototype (NixOS):** `recipes/agent` (Rust, 534 KB binary, closure glibc + libgcc; frame tests run in the Nix build), service `omaflake-agent` in `vm-guest.nix`, started by udev when the port appears (root without capabilities, systemd sandbox). QEMU always attaches the port (`agent.sock`); `omaf-runner bridge --agent` (`AgentLink`, OmaflakeCore) answers the hello, checks every message (limits, error budget, rate limit, text cleaned) and keeps `agent.json` in the instance folder, removed at each start. `omaf agent <name>` shows it; the app's "VM ready" waits for the agent's hello or SSH (`waitForReady`), also without network, and the VM details show agent and guest status. Shared frame vectors (`recipes/agent/testdata/vectors.json`) checked by `cargo test` and `swift test`. **Checked** with a `lab` image built from it: hello 18 s after `omaf start` (guest uptime 6 s), SSH banner after 20 s; with `--network none` hello after 18 s as well; status every 5 s (load, memory, disk, IPv4/IPv6 addresses, the logged-in user); `omaf stop` → goodbye recorded within ~2 s; the clipboard and camera helpers still active beside it. App: notification and details by click (OPEN). Older images without the agent: the app falls back to SSH (by reasoning, not tried). | `omaf agent`, `journalctl -u omaflake-agent`, `bridge.log` |
| B97 | Tags; quieter rows | ✅ 2026-10-18: recipes have `tags` in recipes.toml (lowercase words, e.g. latex, neovim; RecipeDefaults.tags) – all six recipes tagged; nothing of it reaches the guest system, images stay current. The recipe row shows name, author (also omaflake), the lab flask and the tags as small capsules (what doesn't fit becomes "+n", all of them in the tooltip); the description only when the row is open. Image rows say how many VMs use the image ("1 VM", "3 VMs", names in the tooltip) instead of listing them. The row itself has no tooltip (it covered those of its buttons). A VM's actions are framed buttons with a symbol – reset, switch to the newer image, remove in red. | by click |
| B98 | Every row starts with status and guest kind | ✅ 2026-10-18: two symbols open each row – status, then the guest kind (snowflake/Apple): VM = dot (running/stopped), image = circle (default image or not; click to make it the default), recipe = check circle (built) or dashed circle (not yet). The VM's second line no longer ends in "macOS" (the icon says it); SSH shows there when there is an address. | by click |
| B99 | One status lamp; rows two lines high | ✅ 2026-10-18: every row opens with the same lamp (`StatusLamp`), one meaning per colour: green = on / active / built, grey = off / not active / not built, orange = needs attention (recipe changed since its image), cyan pulsing = something is happening (VM starting or stopping, recipe being built). VM: running/stopped/busy; image: default image or not (click the lamp to make it the default); recipe: built, not built, changed, being built. Rows are at most two lines: the recipe row lost its status line ("image built" etc.) – the lamp and its tooltip say it. | by click |
| B100 | VMs from an image row, images from a recipe row; no default image | ✅ 2026-10-18: the section headers lost their free "New VM" and "Build Image" buttons. A VM is created from exactly the image whose row opened the New VM page (`create(name:image:…)`; the page shows that image and its recipe, no picker); an image is built from the recipe whose row (or build row) opened the build page (no picker). There is no default image any more: an image row's lamp is green when at least one VM uses it, "New VM" sits on every image row; a recipe's image in the app is its newest one, and "switch to the newer image" moves a VM to that. The `current/` links stay in the data folder for the CLI (`omaf new --recipe` uses the newest build). | by click |
| B101 | Filters in the lists | ✅ 2026-10-18: where the free "+" was, every list (VMs, images, recipes) has a filter menu: status (VMs running/stopped, images in use/not in use, recipes with/without image), system (NixOS/macOS), plus a tag for recipes and a recipe for images; "Reset Filter". The symbol is filled cyan while a filter is set, the header's count reads "3/6", an empty result says so with a reset button. Filters are kept per tab for the session. The recipes header keeps its folder button next to the filter; Activity keeps "Clear History". | by click |
| B102 | Removing an image from its details | ✅ 2026-10-18: the trash left the image row; "Remove …" (red, with symbol) sits at the bottom of the image's details, as in a VM's – greyed out while VMs use the image (the reason as tooltip on that line). | by click |
| B103 | A folder icon on every recipe and image | ✅ 2026-10-18: each recipe row shows its file in the Finder (yours in local/recipes/ first, else the shared one; not for flake URLs), each image row its folder in the data folder – to look, or to change or remove things by hand. The recipes header lost its folder button (the filter stays). | by click |
| B104 | Flaky test was a real lock bug | ✅ 2026-10-18: the sporadic `swift test` failure (B88: `CancellationTests.secondBuildOfTheSameImageIsRefused`, "busy" when the lock should be free; mostly in the first run after a build) had two causes in `InstanceLock` and `FileLock`. (1) A refused lock threw after every stored property was set, so deinit still ran: it closed the descriptor a second time (perhaps another file's by then) and, with `removeWhenDone`, unlinked the lock file another build was holding – a third build could then start on the same image. Now the properties are set only once the lock is held. (2) The lock files were opened without `O_CLOEXEC`: processes started meanwhile (in the tests the shell of the cancel test; in use QEMU, the runner, nix) inherited the descriptor, and a flock lock lasts while any process has the file open. Now `O_CLOEXEC`. Before: about 1 failure in 5–8 rebuild-and-test runs; after: 0 in 25. New test `refusedLockKeepsTheHoldersFile`. Follow-up the same day: two more double closes – the agent tests' harness closed the guest's end twice (`runEndsWhenThePortCloses`), which once in a while closed the state file `noCommonVersion` was just writing; `QMPClient` closed its socket in init and again in deinit after a wrong greeting. Both fixed, Unix sockets get `FD_CLOEXEC` too. The last rare failure (about 1 in 40 runs, `lsof` a moment later: nobody holds the file): a process another test starts at the same time holds a copy of every open file until its exec closes the close-on-exec ones, so the lock may outlive its holder by milliseconds – harmless in use; the test now allows a second. Also: a lock refused for another reason than `EWOULDBLOCK` now says so instead of "busy". 0 failures in 120 runs. | ✅ |
| B105 | One folder per recipe | ✅ 2026-10-18: a recipe is a folder `recipes/<kind>/<name>/` with `recipe.toml` (its former section of recipes.toml, with `author`), `default.nix` (the former `<kind>/<name>.nix`) and its own files (lab's test app and icon, research's wezterm.lua). The flake finds every folder with a recipe.toml; yours live in `local/recipes/<kind>/<name>/` (one with a shared recipe's name replaces it, author kept); `local/recipes.toml` still overrides single values. The shared recipes.toml is gone (moved to `_to_delete/`); adding new recipes through sections in local/recipes.toml is no longer possible (unreleased). Checked: all six images evaluate to the same store paths as before. The app's folder icon shows the recipe's folder. This is the unit for removing, import/export and the marketplace; omaflake's own recipes move to `omaflake/recipes` and are downloaded at the first start (spec §11.3, decided 2026-10-18). | ✅ |
| B106 | Clipboard and camera over the agent (NixOS); macOS groundwork | ✅ 2026-10-18: agent 0.2.0 carries the clipboard (`0x0100`, JSON {mime, data base64} in 256 KiB `MORE` chunks, ≤ 32 MiB) and the camera (`0x0200`–`0x0203`, frames against credits, newest frame wins on the Mac) for NixOS guests; the Python helpers `clipboard-agent.py` and `camera-agent.py` left the images (moved to `_to_delete/`). Guest: the system agent owns the port, drives `/dev/video42` and serves `/run/omaflake-agent/session.sock` (group users) to `omaflake-agent session` (systemd user service, wl-clipboard); it writes the enabled features to `welcome.json` (the lab app reads it). Mac: `AgentLink` enables in its welcome what the agent offers and the VM's switches allow, drops other features' messages, limits clipboard to 4 contents/s and caps credits at 4; `omaf-runner bridge` moves clipboard and camera from the old ports to the agent once the welcome enables them – older images keep working over their ports (old clipboard port with a 2 s send timeout). Checked in a fresh `nixos/lab` VM: clipboard both ways (also 600 KB in 3 chunks, identical checksum), camera test pattern 52–57 fps at 60 fps once booted (B70: 60 fps over the old port). Found on the way: a session connecting crashed the agent (index past the polled sessions), the Mac chunked camera frames, a fresh zeroed 1 MiB read buffer per 4 KiB virtio read throttled the camera – all fixed, tests added (Rust 6, Swift 11 agent tests). macOS: the agent builds for aarch64-darwin (status from sysctl/getloadavg/utmpx – checked on this Mac; text clipboard via pbcopy/pbpaste and the change count – OPEN in a guest), `omaf-runner run --agent-state` attaches `org.omaflake.agent`, `VZBackend.agentState` reads agent.json. OPEN: how the agent starts in a macOS guest (LaunchDaemon vs. SSH, BM25 – maintainer), host name over the agent, PNG into a macOS guest, a macOS guest check (needs the guest password to build an image). | `cargo test`, `swift test`, lab VM |
| B107 | omaflake/omaflake without recipes; recipe sources | ✅ 2026-10-18: the recipes moved to the new repository `omaflake/recipes` (one folder each, `recipes/<author>/<kind>/<name>/`, its catalog.json, review and contributing rules, CI – one commit, pushed); this repository holds none. `recipes/` became `nix/`, the **image flake**: it gets the recipes as its input `omaflake-recipes` (default: the empty `no-recipes/`) and is shipped in the app (`Contents/Resources/nix`, the default without a flake setting); `local/recipes` and `local/recipes.toml` are gone. **Sources** (Core `RecipeSources.swift`): GitHub repository (public over raw.githubusercontent.com, private over the GitHub API with a fine-grained token, checked and kept in the Keychain), HTTPS URL of a catalog.json, local folder (used as it is on disk, no catalog needed); config.json `sources` (none = omaflake/recipes). Installing downloads a recipe and its `requires`, checks every file's size and SHA-256 and the catalog's paths, writes `<data>/recipes/` and `.installed.json`; removing refuses while another needs it. Before every nix call the recipes are staged into `<data>/cache/recipes/<content hash>/` (local folders first). Image names carry the recipe's revision (local folder's git, or the catalog version). CLI `omaf sources [add/remove/enable/disable/check]`, `omaf recipes [available/install/remove]`; app: Settings → Recipe Sources, the Recipes tab lists what the remote sources offer (fetched when the tab opens), install / update / remove, "Install suggested", official seal. Dev setup: this data folder's only source is the local checkout `../recipes`. Checked: all six images evaluate to the same store paths as before the move; a lab build through the staged recipes; `nix flake check` of nix/ alone and with the recipes; against the real private omaflake/recipes: token check, catalog over the API, install of macos/research with minimal (requires), wrong order refused, token removed from the Keychain with the source; a branch that does not exist: clear error; 95 Swift + 6 Rust tests. Not checked by eye: the new app screens. | CLI, `swift test`, flake checks |
| B108 | Portable packages: backup, stick, sharing | ✅ 2026-10-18 (spec §11.10): VMs, images and recipes as packages (`.omaflake-vm`, `.omaflake-image`, `.omaflake-recipe` – folders the Finder shows as files, document types of the app, `manifest.json`). The data folder itself holds packages: NixOS images self-contained (qcow2 copied out of the Nix store, no GC root), overlays with a relative backing path, runtime files in `run/<name>/` (also keeps socket paths short for VMs on a stick). Core `Packages.swift`, `Transfer.swift` (export: VM only stopped, NixOS with its image; import: same VM or copy with new name/port/macOS identity, the carried image becomes an image of the data folder unless it is there, everything in a `.partial` folder first; run in place, move to a stick, take back, forget), `RecipeLibrary.exportRecipe/importRecipes` (with `requires`, same limits as downloads, source `import`); copies keep sparse files sparse and read-only folders (own recursion – copyfile's created the read-only macOS image folder first and could not fill it). CLI `omaf export vm|image|recipe`, `omaf import [--copy] [--name] [--in-place]`, `omaf move`, `omaf forget`; app: Export … in the details, Import in the header, Run from Stick … / Take Back / Stop Listing, drive icon, double-click and drop (own Apple-event handler: with only a menu bar scene SwiftUI swallowed `application(_:open:)`). Migration from the old layout at the first start. Checked with a real NixOS VM of the old layout (`migtest`, a marker file in the guest) and an APFS disk image as the stick: migration (marker there after boot), export (10 s for 7 GB), the original away → import → boot → marker, `--copy` (new name and port, boots), run in place from the stick and write there, stick ejected → VM gone from the list, attached → back, take back (the data written on the stick came along), `omaf move` and boot from the stick, image export/import, recipe export (research with minimal) and import into another data folder, a recipe without its requirement refused; a 20 GB sparse file takes 16 KB on the stick; double-click import with the app started from a shell. 101 Swift tests. Not checked: macOS VMs (no macOS image without the guest password; export/import of a fake macOS VM in the tests only), the new app screens by eye, double-click with the app started by LaunchServices (each ad-hoc signed rebuild asks again for the Documents folder, and the app waits for that answer at launch). | CLI on real VMs, `swift test` |
| B109 | Import as one concept; external VMs; recipe identity by content | ✅ 2026-10-18 (maintainer's direction, spec §11.10): an import is always a copy into the data folder, nothing moves; running a VM from where it is has its own button (VMs tab) and command (`omaf external add/remove/list`); "Run from Stick", "Take Back", `omaf move`, `omaf forget`, `--copy` and `--in-place` are gone, no texts name a stick. An Import button next to the filter of each tab; its open panel shows only that kind and checks the content before it closes (`PackageChecker`). Conflicts are asked first (`Transfer.conflict`): a VM of the same name – cancel or a new name (a second VM with its own identity, `--name`); the same image – cancel or replace (`--replace`, refused while a VM on it runs); recipes of the same ID – compared by `RecipeLibrary.contentHash` (SHA-256 over sorted paths and each file's SHA-256): identical ones are skipped and keep their source, others cancel or replace. On a network volume the app warns before running a VM from there. VM details: one row, Export left, Stop Listing / Upgrade / Reset / Remove right; Remove of an external VM says that it deletes it at its place. Checked on the real `migtest` and the disk-image stick: import with the name taken refused, under a new name a second VM with its own port boots with the marker, the same image refused without `--replace`, an exported VM run with `omaf external add`, boots, `external remove`; 101 Swift tests (conflicts of all three kinds, identical and changed recipes, a macOS VM under a new name with a stand-in runner). Not checked: the new app screens by eye. OPEN: keeping two recipes of the same `<author>/<kind>/<name>` from different sources side by side (the source as part of the identity – image flake attribute and image names). | CLI on real VMs, `swift test` |
| B110 | Recipe identity by content; omaflake IDs | ✅ 2026-10-18 (maintainer's decisions, spec §11.11): a recipe is author/kind/name + content – the same base with other content is a recipe of its own (also from the same source), the same base and content exists once. The source is part of the recipe's **line** (source + base), which carries current/, retention, outdated and upgrade; image.json records the recipe source. **omaflake IDs** `<kind>:<identifier>:<words>` for recipes (content hash), images (store path) and VMs (random identity, new for a copy under another name) – three words of BIP-39's English list (MIT, SHA-256 of the list checked; a fourth word only on a collision of the same identifier); file form `<author>.<kind>.<name>@<words>` for recipe IDs and image package names. Staging: one tree per recipe with what it requires (same source first) + `recipes.json` read by the image flake (a plain recipes folder still works for `nix flake check`); downloaded/imported recipes in `recipes/<id>/`; a catalog's file list gives the content hash before downloading. Recipe imports have no conflicts any more (`--replace` is for images only). GUI and CLI show a recipe's source only beside another source's recipe of the same base, its words only beside the same source's. Migration of downloaded recipes, images (renamed, overlays rebased), current/ links and VMs (identity). Checked: the six recipes evaluate to the same store paths as before (all three NixOS images and three darwin systems), all four flake checks, the real migration (two images renamed, `migtest` boots with its marker on the rebased overlay), `omaf build nixos/lab` finds its image, two `nixos/lab` from different sources side by side in a scratch data folder (shown with their sources, `omaf build lab` asks with omaflake IDs), an identical import is a no-op; 104 Swift tests. Not checked: the app screens by eye; a macOS build (needs the guest password). OPEN: a recipe's ID does not include the content of what it `requires` (a changed requirement shows as outdated by store path, not as a new ID). | flake checks, store paths, CLI on real data, `swift test` |
| B111 | Twelve words, shortest display; recipe versions | ✅ 2026-10-18 (maintainer's decisions, spec §11.11): omaflake IDs carry twelve words (132 bits, unique worldwide) – the fourth-word fallback of B110 is gone; GUI and CLI show as few words as tell the objects of a kind apart, the same count for every object of that kind (`OmaflakeID.Lengths`), the full ID in tooltips and accepted everywhere with any start of its words (`omaf images rm`, recipe arguments). The day-one three-word IDs are the start of the new ones. Recipe versions: setting `keepRecipeVersions` (Settings, `omaf config --keep-recipe-versions`), off = only a line's newest and every version an image is built from stay; a build keeps a copy of the version it used (local folders too) in `recipes/<id>/`; `RecipeLibrary.versions/pruneVersions/keep/removeLine`; manifest dates with fractions of a second (two versions in one second were unordered). CLI `omaf recipes versions`, `omaf recipes remove [--all]`; app: one row per line, versions always as sub-rows (words, date or "as it is in the folder", newest, images, Export, Remove), "Remove All Versions …", the setting. Migration from three to twelve words (recipes, images renamed with overlays rebased, VMs' identities extended so they start the same). Checked: the six recipes' store paths unchanged, four flake checks, the real migration (`migtest` boots with its marker), `omaf build nixos/lab`, `omaf recipes versions`; 107 Swift tests (shortest lengths, versions kept while used or when asked, a local folder's version kept by a build and pruned after its image is gone, remove all). Not checked: the new app screens by eye. | flake checks, store paths, CLI on real data, `swift test` |
| B112 | Licence AGPL-3.0-or-later | ✅ 2026-10-19 (maintainer's decision, spec §11): omaflake's own code moves from MIT to **AGPL-3.0-or-later** (the newest version is 3 from 2007; "or later" covers a future one) – `LICENSE` is the official text from gnu.org (661 lines), every own source file carries <!-- REUSE-IgnoreStart -->`SPDX-License-Identifier: AGPL-3.0-or-later`<!-- REUSE-IgnoreEnd --> (the BIP-39 word list `MIT`), agent metadata (`Cargo.toml`, `package.nix`: `agpl3Plus`), generator of `BrandAssets.swift` writes the header. The recipes stay MIT (templates to copy; other sources build on them through `requires`); QEMU patches GPL-2.0-or-later, try-omarchy/startergo patches MIT unchanged. No App Store (decided). Website `omaflake/website` and the org profile follow. Before the release: DCO (`Signed-off-by`) in CONTRIBUTING (OPEN). | `swift build`, `swift test`, flake checks |
| B113 | Alpha release 0.1.0 (unsigned) | 🔄 2026-10-20 (maintainer's decision): version 0.1.0, `OMUpdateFeedURL` = `https://omaflake.app/update.json`; the zip and its SHA-256 as assets of a GitHub release in omaflake/omaflake (the repository goes public for it), `update.json` on the website (omaflake/website, Pages on omaflake.app), which shows the download and install steps for an unsigned build (curl without quarantine, or "Open Anyway"). Not GitHub Actions artifacts: they need a login and expire. Open: repositories public (omaflake, recipes, website – maintainer), Pages, imprint and privacy page (OPEN). | release workflow, website preview |
| B114 | File versions and architectures | ✅ 2026-10-21 (maintainer's decisions): every JSON file omaflake writes names its JSON Schema – `"$schema": "https://omaflake.dev/schema/<type>/v<n>.json"` (spec §5.1; types image, instance, settings, installed, macos-vm, manifest, catalog, recipe), one count per type; the schemas are in `docs/schema/`, published with GitHub Pages from `docs/` on omaflake.dev (the developer domain; omaflake.app stays the users' one), a published version is never removed (test). `FileFormat` migrates older files step by step when they are read (0 → 1: "flavor" → "recipe", `kind` always set, `system` added as aarch64-linux/aarch64-darwin – everything before was built on Apple silicon; `.installed.json` becomes `{recipes}`; `version`/`format` of macOS VM folders and manifests give way to `$schema`; the optional legacy fields and `LegacyJSON` are gone) and refuses newer versions, other types and foreign schemas. Architectures are Nix system names (`Platform`): `system` in image.json and instance.json; create, start, upgrade, import and external VMs refuse another one. Recipes: `"$schema"` (recipe/v1) and `systems = [...]` in recipe.toml are required (image flake and tools/catalog.py check them; catalog.json catalog/v2); `<system>.nix` next to default.nix is the recipe's variant; the image flake gives `images.<system>.<id>` and `nixosConfigurations.vm-<system>-<id>`. Minimum versions: every type is read from a minimum on (0.2.0: catalog 1, all others 0 – recipe and catalog v0/v1 are read as Apple silicon, never rewritten); goal from 1.0.0 on: a minimum is raised only with a major release, before that a minor release may (maintainer, 2026-10-22); what is below it or newer is listed with a warning (recipes, images, VMs) and recipes outside the range are left out of staging. After Refresh the app lists newer versions of downloaded recipes (take over / add, B111). CLI `--all-systems`; app: recipes for other architectures hidden, filter "Other Architectures", orange badge, build disabled. x86_64-linux evaluated only (no x86 builder here): an x86 variant evaluates and stays out of aarch64 – building and booting it is OPEN (NixOS host, issue #2). Open (maintainer): Pages for omaflake/omaflake from `main` `/docs`, domain omaflake.dev. | `swift test` (FileFormatTests: what omaflake writes against docs/schema), check-jsonschema on catalog, recipes and migrated files, `nix flake check` with the recipes, `nix eval` of an x86_64 variant |
| B115 | Developer docs on omaflake.dev | ✅ 2026-10-21 (maintainer's decision): GitHub Pages from `main` `/docs` with the domain omaflake.dev (`docs/CNAME`), Jekyll with Just the Docs v0.10.0 as remote theme (no workflow needed; since B157 built and published by `.github/workflows/pages.yml` through Actions, the domain in the Pages settings), colours from brand/ (`docs/_sass/color_schemes/omaflake.scss`). Pages: home, writing recipes (the full format stays in omaflake/recipes CONTRIBUTING.md), CLI, spec, file schemas (`/schema/`), contributing, app, Linux builder, macOS base; `plan.md` is excluded. omaflake.app stays the users' site. | local Jekyll build, every page looked at |
| B116 | QEMU package: libepoxy patch in the repository | ✅ 2026-10-22: the "QEMU package" workflow (first run since the repository is public, tag v0.2.0) failed – startergo/homebrew-libepoxy moved its tag v1.0.5, so the pinned archive hash no longer matched. Compared with the old archive from the Nix store: only Formula, README and a workflow changed, `libepoxy-akihikodaki-egl15.patch` is byte-identical. It now lives in `qemu/patches/startergo/` (NOTICE), `package.nix` and `build.sh` no longer download the tap. `build.sh` still downloads startergo/homebrew-virglrenderer by tag (v1.0.42) – same risk, local script only (package.nix uses the patch in the repo). | fresh `nix build .#qemu`, `virtio-gpu-gl` in `-device help` |
| B117 | Rust core and core API | ✅ 2026-10-23 (maintainer's decision: one shared core in Rust, macOS-only parts stay Swift): all logic of `OmaflakeCore` is now `crates/omaflake-core` (Rust); the CLI `omaf` is Rust (`crates/omaf`, clap) and serves the core API – JSON-RPC 2.0 over stdio, `omaf serve` (docs/api.md, API v1: methods, jobs, events, error kinds). The app is a client of it (`OmaflakeKit`: CoreClient + DTOs) and holds no logic of its own; the build queue, ready/ended detection, freshness, offered recipes and the clipboard agent moved from AppModel into the core. Swift keeps the app, `omaf-runner` (Virtualization.framework) and Brand. Same files and formats as before: the Rust CLI's output was compared with the Swift CLI's for every read-only command on real data (identical, recipe hashes included; free space differs – purgeable space is not counted, OPEN); a NixOS VM created, started, reached by SSH and the agent, stopped through the CLI and through `omaf serve` (vm.ready, vm.ended). Contract tests: golden JSON written by Rust, decoded by the Swift DTOs; an end-to-end Swift test against the real server. Not yet checked: the app by click (all tabs, builds, macOS base, builder tasks), a macOS guest, a macOS image build, the IPSW download. Linux host (issue #2): B124. Core texts: closed – what the pages show by B121's views (every text a message key with its English text, api.md §10.2), the error messages by B125 (`vm_err!`'s format string as the key, `data.text`); still English: the Doctor's found details, log lines, values inside messages, the CLI (by decision). | `cargo test`, `swift test`, CLI comparison, VM cycle |
| B118 | Editor lines: neovim and emacs | 2026-10-23 (maintainer's decision): the base has no editor any more – NixOS, Hyprland, the omaflake agent, WezTerm and the terminal tools. A NixOS recipe picks its line with `omaflake.editor` (`modules/editor.nix`, default `"neovim"`): `home/neovim.nix` (Neovim as before, plus Zellij, `EDITOR=nvim`) or `home/emacs.nix` (Emacs pgtk with Vertico/Orderless/Marginalia/Consult, Corfu, Magit, diff-hl, Eglot, all tree-sitter grammars; `server-start`, `EDITOR=emacs-edit` = the running Emacs, else `emacs -nw`; Cmd+E). Pure Emacs rather than Doom: Doom installs its packages at run time (`doom sync`), against declared and ephemeral VMs; nix-doom-emacs-unstraightened would be a third-party input (**OPEN** whether it builds here). Emacs keeps WezTerm as its terminal (no vterm): faster for long output, a terminal even with a broken Emacs config. Recipes are named after their line: `omaflake/nixos/neovim-minimal`, `neovim-research`, `emacs-minimal` (were `minimal`, `research`). | `nix eval`, image builds |
| B119 | User interfaces per platform | **Done** 2026-10-23 (macOS frames and the Linux app built; clicking them OPEN – see below) (maintainer's decision, agreed with the Linux contributor): one look on every platform – omaflake's brand – but one interface per platform, each a client of the core API (docs/api.md). **macOS** stays SwiftUI and gets a setting "Show omaflake in: menu bar / window / both" (menu bar = today's `MenuBarExtra`, no Dock icon; window = a regular app window with Dock icon; the pages are the same views, only the frame differs). **Linux** gets a native Rust app with **iced** (no Tauri; issue #4): a regular window, linking `omaflake_core::api::Server` in-process with the Rust types of `api::types`. Linux has no menu-bar window (StatusNotifierItem offers an icon and a menu only, Wayland lets no app place its window by the icon), so the menu-bar part comes **with the app**, nothing to install separately: a tray icon over StatusNotifierItem (`ksni`, can be switched off; KDE, Cinnamon, XFCE, waybar's `tray` module; GNOME only with the AppIndicator extension, which omaflake does not ship) with the VMs, start/stop, running builds and "Open omaflake"; for waybar users `omaf status --waybar` (JSON for a `custom` module, the snippet in the docs). Same look: design tokens (colours, spacing, radii, type, symbols) generated by `brand/generate.py` for both (`BrandTokens.swift`, `brand_tokens.rs`) instead of hand-written values – **done** 2026-10-23: `brand/tokens.toml` → `brand/tokens.py` writes `Sources/OmaflakeApp/BrandTokens.swift` and the crate `crates/omaflake-brand` (`src/tokens.rs`, no dependencies; its test and `tokens.py --check` in CI catch stale files); `BrandStyle` and the app's recurring literals use them, same values (brand/README.md). Not decided: the iced app on macOS stays a development aid, not shipped – re-evaluated once it has parity on Linux. Order: core compiles for Linux (`cargo check --target x86_64-unknown-linux-gnu`, OPEN), design tokens, iced skeleton (runs on the Mac against real data for development), host spike on x86_64 NixOS (issue #2). Core on Linux ✅ 2026-10-23: the workspace builds and tests on aarch64-linux (NixOS 26.11, an omaflake VM): `cargo build`, `clippy` and `fmt` clean, all tests green in 12 runs. Fixed on the way: system tools by name on Linux (`platform::host_tool` – NixOS has no /usr/bin/ssh, /bin/chmod, /usr/bin/sudo), macOS-guest tests only on macOS, two recipe versions within one millisecond no longer tie (the newest could be pruned), a job's end work runs before `wait` returns (`omaf call images.build` waited for the build but not its prune), a racy session test. x86_64-linux not compiled yet (OPEN, same code; no x86 machine here). **macOS frame ✅ 2026-10-23** (not clicked yet, OPEN): Settings → "Show omaflake in" Menu Bar / Window / Both (`AppFrame`, UserDefaults `appFrame`, default Menu Bar). `MenuBarExtra(isInserted:)` follows the choice; a `Window` scene `main` shows the same `MainView` (resizable from 500 × 600, `defaultLaunchBehavior` from the choice, no state restoration, frame autosave if SwiftUI sets none); `NSApp.setActivationPolicy` `.regular`/`.accessory` at launch and on change, so no relaunch; the app delegate keeps the app running when the window closes and reopens it from the Dock; app menu Settings…/Quit (`MainCommands`). Questions, file pickers and the first launch's Doctor already used whichever window is visible or key – only comments changed; notifications are held back while the app window is in front too (docs/app.md). **Linux app ✅ 2026-10-23** (docs/linux.md; run on the Mac only, OPEN on Linux): `crates/omaflake-gui` (binary `omaflake`, iced 0.14 with wgpu), a pure client of the presentation model (B121) – it connects to the core service like the CLI (`omaf service start --json`, then `api::client`; else `api::Server` in-process, reconnecting when the service ends), subscribes to home, activity, the open tab's list with its filter, the open page with the form values typed so far and (for the tray) the unfiltered VM list, plus the `jobs` topic for build logs; it draws every page generically from the core's Rust types (rows, lamps, badges, details with switches/info/actions, filters, tools, forms, build progress/outcome/log, Doctor, macOS base, Settings, activity) and runs actions as api.md §10.3 says (asks: text/password sheets, folder/file/package through `rfd` – XDG desktop portal – with the `check`; confirmations as a modal sheet; `after`; `ui.*`: show/back/step/filter/field, reveal/open via `xdg-open`, terminal via `$TERMINAL` else foot/kitty/wezterm/alacritty/gnome-terminal/konsole/xterm, quit). Look: every colour, spacing, radius and text size from `omaflake-brand`; the glow as a generated image (iced has no radial gradient); icons: Lucide font (ISC, `NOTICE`) mapped from the vocabulary; brand glyph and wordmark. Tray (Linux only): `ksni` StatusNotifierItem, on by default, switch in Settings → This app (stored in `$XDG_CONFIG_HOME/omaflake/linux-app.json`), tooltip and menu from the `vms` page and the jobs (VMs with status and start/stop/terminal, running builds, Open, Quit); closing the window keeps the app in the tray (iced `daemon`). Notifications (`notify-rust`) for new activity entries with a notification text while the window is not in front and Settings → Notifications is on. Nix: `packages.{aarch64,x86_64}-linux.omaflake-gui` in the root flake (`linux/package.nix`: `omaflake` + `omaf`, `omaflake.desktop`, hicolor icons, LD_LIBRARY_PATH wrapper with Vulkan loader, libGL, Wayland, xkbcommon, X11 libs, fontconfig). `nix build .#packages.aarch64-linux.omaflake-gui` builds (on the Mac's Linux builder, without warnings; not started there; x86_64-linux evaluated only). Tests: every golden page model decodes and is drawn, rows/forms/choices, tray menu, terminal choice, prefs, texts. Checked on the Mac (temporary `VM_HOME`, screenshots; `OMAFLAKE_DEMO_VIEWS` draws the golden models without a core): the pages match the SwiftUI app's look. Found on the way and fixed in the core: once the catalogs were fetched every `views.*` call deadlocked (`available_cached` locked the catalogs twice). Gaps of the view model it had worked around (page titles, the guide's URL, the macOS wording on Linux) are closed in the model (B121, 2026-10-23); still OPEN: the Doctor's sections and the macOS base page on a Linux host (OPEN, issue #2). OPEN: run on real Linux (Wayland and X11, KDE/GNOME+AppIndicator/waybar tray, portal file chooser, notifications, terminal choice), translations (English only), `omaf status --waybar`, clicking every action on the Mac too. | `cargo test` on aarch64-linux; `swift build`/`swift test`; `cargo test -p omaflake-gui` |
| B120 | One core service per user | **Done** 2026-10-23 (maintainer's decision, then implemented): `omaf serve --socket` serves one `Server` per user and data folder on a Unix socket; each connection is a session (`initialize`, subscription, language) on shared state – jobs, the build queue, the watcher, the caches; events go to every subscribed connection (`events.rs` keeps a subscriber per connection, `recipes.changed` per language). Socket `<data folder>/run/omaf.sock`, too long (> 103 bytes on macOS) → `$TMPDIR`/`$XDG_RUNTIME_DIR`/`omaflake-<hash>.sock`, else `/tmp/omaflake-<uid>-<hash>.sock`; mode 0600 plus a peer-uid check; `<socket>.lock` (flock) refuses a second service (it exits 0, so launchd/systemd do not loop). Ends after `service.restart` once no job runs, after a data-folder move, or with `--idle-exit` (no client, no job). `omaf service install|uninstall|status|start|stop` (launchd agent `app.omaflake.serve`, `KeepAlive.SuccessfulExit=false`; systemd user unit, `Restart=on-failure`; no socket activation – the socket path follows the data folder). Clients start it on demand (`omaf service start`: kickstart the agent, else `omaf serve --socket --idle-exit 300` with setsid). `omaf build` runs as a job of a running service (Ctrl-C → `jobs.cancel`), `omaf call --socket` talks to it; the app connects through the bundled `omaf service start --json` and falls back to a private stdio server; its `shutdown` on quit ends only its session. Update rule: a newer client asks an older service to restart (api.md §1). Tests: Rust `api::service` (two connections share jobs/events, `shutdown` ends one session, restart waits for jobs, lock, idle exit, 0600, path fallback, versions), `omaf` `tests/service.rs` (start/call/stop end to end, long data folder), plist (`plutil -lint`) and unit text; Swift `CoreEndToEndTests.twoClientsShareTheService` (`OMAF_BIN`). Checked by hand with `VM_HOME` in a temporary folder: a CLI build seen by a second client, Ctrl-C cancelling the service's job. OPEN: the launchd agent and systemd unit were not installed in development (only their text is tested) – check `omaf service install` on a Mac and on Linux, that VM windows (QEMU, runner) open from the agent's session and that it finds nix/qemu with launchd's PATH; the other long CLI commands (`macos install`, `builder …`, `import`/`export`, `config --home`) still run in-process; the TUI/Linux app use `client::Client` once they exist; a slow client blocks event delivery for up to 10 s before it is dropped. | `crates/omaflake-core/src/api/service.rs`, `api/client.rs`, `crates/omaf/src/commands/service.rs`, tests above |
| B121 | Shared presentation model and a TUI | **Done** 2026-10-23 – model, TUI, SwiftUI and iced on it; gaps closed. End-to-end: on macOS with real data the service, `omaf status --waybar`, `omaf ui`, a VM started/stopped over the socket; on aarch64-linux (omaflake VM) all workspace tests incl. the iced crate, clippy clean, service, waybar JSON, `omaf ui` (in-process and through the service), Linux wording without Finder/Keychain/Mac. Not clicked: SwiftUI, iced on a real Linux desktop (OPEN) (maintainer's decision 2026-10-23): the interfaces share not widgets but **what each page shows** – a presentation model in Rust, served by the core API (pattern of the Rust framework Crux: one core, thin native shells). Per page (VM list, images, recipes, build, doctor, settings, new VM, activity) the core says which rows there are with their labels, status and badges, which actions are enabled and why not, sorting and filters, the confirmations an action asks and their texts; the interfaces only draw it, with the design tokens (B119). Embedding iced views in the SwiftUI app was considered and rejected: every input (keys, text input methods, scrolling, focus) would have to be passed through, VoiceOver and native sheets/menus would be lost. Interfaces on this model: SwiftUI (macOS), iced (Linux) and a **TUI** with `ratatui` in the `omaf` binary (`omaf ui`), running on **macOS and Linux** – for hosts without a desktop, SSH sessions and terminal users. In the TUI tokens map to terminal colours (truecolor, else the nearest of 256), symbols to Nerd Font glyphs or plain characters, file pickers to path input with completion, notifications to a status line and the activity list. Already a step towards it: the server's ready-made fields (`label`, `upgradable`, `outdated`, blockers). Order: after B119's iced skeleton – the model grows page by page, each page moving out of the SwiftUI views at the same time; the TUI as the check that the model is free of any one interface. **Model done** 2026-10-23: `crates/omaflake-core/src/api/views/` (one module per page: home, vms/vm, images/image, recipes, newVm, build, doctor/macBase, settings, activity, import), served as `views.get {page, params}` and `views.subscribe {pages}` – per connection of the service (B120): each connection subscribes its own pages, built in its language; the activity list is the server's, the same for all – → `view.changed {page, params, model}` (the watcher rebuilds subscribed pages after every look, action, job change and activity line; api.md §10). Generic shapes: actions `{id, label, icon, method, params, enabled, reason, help, destructive, on, busy, badge, ask, confirm, then}` – a core method or a `ui.*` verb (show, back, step, filter, field, reveal, open, terminal, quit) –, confirmations `{title, text, button, destructive, input}`, asks (text, password, folder, file, package with `packages.inspect` as check), lamps, badges, info lines, hints, filters (applied by the core from `params.filter`), form fields (`param` into the action's params, `set` for settings). Texts `{key, text, args}`: the app's English source strings as keys (its de.lproj keeps working), args numbers, strings, dates, sizes, nested texts and asked params. The activity list moved into the core (actions done or refused, jobs ended, VMs ended/ready, messages; with the notification text), plus `activity.clear` and `recipes.installUpdates`. Built on the API's own DTOs (pure builders of a `Ctx`); unit tests per page from the contract fixtures and through a server on a temporary data folder; golden models in `testdata/api/views/` (the Swift contract test ignores the subfolder until the SwiftUI app decodes them). Next, in parallel: SwiftUI on the model (its views move out page by page), the iced app, the TUI (`omaf ui`). OPEN: error kinds still English (§3), locale/keyboard/time-zone names are plain values (UIs name them). **TUI ✅ 2026-10-23** (`omaf ui`, crate `crates/omaflake-tui`, ratatui 0.30 + crossterm 0.29 – only that crate depends on them): a client of the service (`omaf service start` as the CLI; a server in-process when that fails or with `--in-process`; reconnects when the service ends), subscribes home, activity, the tab's page and the page on top, draws the models generically (rows with lamps, kinds, badges, tags, details with switches, info lines, versions and their actions; jobs with gauges; hints with their action; fields of every kind; filters) and sends their actions – asks (text, password hidden and asked twice, folder/file/package as a path with Tab completion and the `check`), confirmations with `input` and `{param}` args, then reveal/terminal/back, and every `ui.*` verb (`terminal` suspends the TUI for the SSH session or install command; reveal/open use `open`/`xdg-open` where there is a desktop and print the path). Build logs live (`jobs.log`, then `job.log`), running and quiet time counted by the TUI. Colours from `omaflake-brand` (truecolor, nearest of 256, none with `NO_COLOR`), the icon vocabulary as plain characters or Nerd Font glyphs (`--nerd-font`/`OMAF_NERD_FONT=1`; a font cannot be detected – OPEN), activity as the status line. `omaf status [--waybar] [--follow]` (waybar `custom` JSON, the snippet in docs/cli.md). Tests: every golden page model drawn to `TestBackend`, keys through asks/checks/confirmations to the requests sent, filter, tabs, form values and settings `set`, password repeat, verbs; status JSON; `omaf ui` started and quit in a pty against a temporary `VM_HOME` (in-process and through the service). OPEN: texts English only (translation by `key` needs a table in the TUI); not run on a Linux host yet (no Linux target here; nothing macOS-specific in the crate); several chosen packages at once (`multiple`) – the TUI asks one path; workarounds for model gaps – closed 2026-10-23, see below. **SwiftUI on the model ✅ 2026-10-23** (not clicked yet, OPEN): the app subscribes to `home`, `activity`, the three lists (with their filters) and the open page (with its form values) and draws only `view.changed` models – `OmaflakeKit/API/Views.swift` (the view types, decoded strictly; `PageModel`, `view.changed` as `ServerEvent.viewChanged`), `ViewTextRendering.swift` (texts by key through de.lproj, args filled in, positional placeholders, fallback to the English text), app `HomeLists.swift`, `Pages.swift`, `ModelViews.swift` (icons, tones, actions, badges, filters, fields, the question overlay with text/password/repeat), `Components.swift` (the old look, unchanged tokens). `AppModel` (1379 → about 760 lines) keeps the session, navigation, form values, the generic action runner (asks as panels/questions, `packages.inspect` in the panel delegate, confirm with input, `ui.*` verbs, `after`), notifications from the activity's `notification`, launch at login, language; gone from the app: blockers, labels, lamps, sorting, filtering, the activity's wording and list, next names, every confirmation text, the update state, the recipe rows/versions, RequirementText, the New VM defaults. Core additions for it: page titles (`doctor`, `settings`, `newVm`, `build`, `macBase`), `activity.notifications`, `sources.add` in the activity (its note, or the refusal), a resubscription sends only new pages (form edits do not resend every page), the B119 notification help text; fixed a deadlock (`available_cached` held the catalogs' lock while taking it again – every page hung once the catalogs were fetched). Tests: Swift `ViewContractTests` (every golden view decodes, every text renders to the core's English `text`, every German translation fits its args), `CoreEndToEndTests` subscribes to every page of a real server; Rust: titles, notifications, resubscription, the deadlock. Behaviour that changed: a macOS build's password, the base's password (twice) and a private source's token are asked in the in-window question when the action starts (were fields on the page); dates in info lines all `abbreviated, shortened`. **Gaps closed** 2026-10-23 (what the three interfaces had worked around, now in the model, api.md §10; all three switched, workarounds removed): (1) `primary` marks a row's or page's default action (VM: terminal/start in window/the Doctor's warning; image: New VM; recipe: build; offered: install; a build row's `open`; the forms' create/build/add) – the TUI's Enter heuristic and the row-click conventions are gone; (2) `shortLabel` for header actions ("Doctor", "Data Folder", "Quit"); (3) form fields name their key in `params.values` (`valueParam`), and `ui.field` names its page (`{page, params, field, valueParam, value?}`) – no "top page"/"field id" conventions; (4) texts for the recipes' `loadingText`, `labHint` (with the way to Settings) and `offeredTitle`; every page has a `title` (home, vm, image, import too) – iced's hard-coded titles are gone; (5) `ui.show guide {file, title, url, path?}` – omaflake.dev's page and, inside omaflake.app, the bundled file; (6) host wording: texts naming the Mac, Finder, the Keychain or the menu window come host-neutral when the core runs on Linux (macOS keys unchanged; German for the new keys); (7) build and macOS base pages give `log: {job, from, lines, …}` (the base's real job id, `jobs.log {id, from}`); (8) the macOS base's IPSW is shown once (its line; no field); (9) `packages.inspect` `reason` is a Text everywhere; (10) activity: unseen stays UI-local – ids are stable and rising, a UI remembers the highest shown and forgets it when a new server starts again lower (api.md §10.6). Goldens added: `home.linux`, `settings.linux`, `recipes.loading`, `macBase.ipsw`. | `crates/omaflake-tui`, `crates/omaf/src/commands/{ui,status}.rs` |
| B122 | The `omaf` command from the Doctor | **Done** 2026-10-23 (installing by click not tried – OPEN, see below): a new optional Doctor item `cli` (not required, not counted in `issues`, like git). Detection (`crates/omaflake-core/src/cli_link.rs`): the PATH a new terminal gets – the user's login shell (`$SHELL`, else the passwd entry) started as `-l -i -c` with a clean environment, printing `env` between markers, at most 4 s (macOS: /etc/paths and /etc/paths.d through path_helper, then the user's files; checked on this Mac: the login PATH comes back complete) –, else the usual folders (/etc/paths, /etc/paths.d, /usr/local/bin, /opt/homebrew/bin, ~/.local/bin, ~/.nix-profile/bin, /run/current-system/sw/bin); the first executable `omaf` there. ok when it is this omaflake's (same canonical file or the same `omaf --version`; on Linux also any omaf from the Nix store – the package), limited when another one comes first (detail: its path and version) or omaflake's link is there but its folder is not on the PATH (detail and hint say how to add it), missing when there is none; `install` is `<this omaf> doctor --install-cli`. "This omaflake's omaf": inside an app bundle `<app>/Contents/MacOS/omaf`, else the running executable when it is `omaf`, else an `omaf` next to it (the Linux app). Install: a symlink – macOS `/usr/local/bin/omaf` as root (`set -e; mkdir -p /usr/local/bin; ln -sfn <omaf> /usr/local/bin/omaf`; the app through `AdminShell`, the CLI through sudo), Linux `~/.local/bin/omaf` without root; replaces a symlink or an older `omaf` file, refuses any other file (`exists`). Uninstall removes only omaflake's link (a symlink to an `omaf`). API: `doctor.installCli` / `doctor.uninstallCli` → job `cli-install` (`{link, installed, message}`), `doctor.check`'s `cli: {link, target?, installed, blocked, onPath, admin}`; the doctor page's item gets Install … / Uninstall … with their questions (link, target, administrator on macOS), its hint names a file in the way or the missing PATH folder; activity lines for the job. CLI: `omaf doctor` lists `cli` with the command; `omaf doctor --install-cli` / `--uninstall-cli`. SwiftUI, TUI and iced draw it unchanged (generic); Swift knows the item `cli` and the job kind `cli-install`; German strings added. Tests: detection with fake PATH folders, a fake login shell and fake omaf scripts (ok/limited/missing, version unknown, not executable, fallback folders, a hanging shell times out), the root script texts, the Linux ~/.local/bin link (install, PATH hint, replace, uninstall, refusing a foreign file), the doctor view (golden `doctor.json`, actions per state). Never run in development: the administrator dialog, `/usr/local/bin/omaf`, sudo. OPEN: click Install …/Uninstall … in the app (dialog, then the item turns ok); `omaf doctor --install-cli` with sudo on a Mac; on Linux a link into the Nix store (`nix run`) breaks after garbage collection – the package installed through Nix is the better way there; the login shell is started on every Doctor check (≈ 1 s with a heavy profile). | `crates/omaflake-core/src/cli_link.rs`, `api/views/doctor.rs`, `crates/omaf/src/commands/instances.rs` |
| B123 | omaflake inside the lab VM | **Done** 2026-10-23 (maintainer's decision): the development recipe `nixos/lab` brings omaflake itself – the app `omaflake`, `omaf`, `omaf ui` – so the Linux interfaces can be tried on a real Linux desktop (Hyprland, aarch64). The image flake has a new input `omaflake` (`github:omaflake/omaflake`, nixpkgs follows) and an overlay `pkgs.omaflake` = the root flake's `omaflake-gui`; Nix fetches it only when a recipe uses it (checked: with a broken lock entry for it every other recipe still evaluates, `neovim-minimal`'s drvPath is unchanged). In a checkout (`nix/` with the root flake.nix and `crates/` above it) the core passes `--override-input omaflake <checkout>` with every nix call that gets `omaflake-recipes` (`ImageStore::omaflake_checkout_arguments` in `guest_arguments`: builds, recipe defaults, expected store paths, the Doctor's flake evaluation); with `.git` as a plain path – the tracked files with their changes, not `target/` (a `path:` would copy the build output too) –, else `path:`; the bundled flake keeps the lock. Lab: `pkgs.omaflake`; the core service is started on demand (app, `omaf ui`, the waybar module), no unit declared; waybar gets `custom/omaflake` (`omaf status --waybar --follow`, click → `omaf ui` in WezTerm) and `tray`; `WGPU_BACKEND=gl`; Cmd+D → fuzzel finds "omaflake" (the package's desktop entry). **Tried** in `lab-omaflake` (window): `omaf --version` 0.2.0, `omaf doctor`, `omaf service start/status`, `omaf status --waybar`, `omaf ui` in a pty (draws home, tabs, Doctor), the app started with `hyprctl dispatch 'hl.dsp.exec_cmd("omaflake")'` (Hyprland 0.56 takes Lua there) and photographed with grim: home, tabs, Recipes with the official catalog; its tray icon in waybar (SNI id `omaflake`). **Found and fixed:** waybar 0.15 refuses `"{icon} {}"` (mixed argument indexing) – the module stayed empty; `{icon} {text}` in the lab and docs/cli.md. The app ticked every 600 ms and so redrew its window all the time – ~95 % of a core in the VM with GL, 250–345 % with wgpu's pick, Mesa's software Vulkan (lavapipe; virgl has no Vulkan); the first try left the VM unreachable over SSH until a clean shutdown (cause not pinned down – OPEN). Now the tick runs only while a page shows something working (`App::animated`, `pulses`: a working lamp, busy action, loading, a queued/checking/running job) or a toast is up – idle 0 % CPU; the lab sets `WGPU_BACKEND=gl`. **Larger, open:** the app should itself avoid a CPU adapter (lavapipe) – every omaflake NixOS guest has virgl without Vulkan, and so do other VMs; `omaf doctor`/the Doctor on a Linux host still checks nix-darwin, the Linux builder, the IPSW and the macOS base (host support, issue #2); `omaf start` right after `omaf stop` was refused twice with "SSH port 2224 is in use" (a few seconds later it started – `Ports::is_free` binds without SO_REUSEADDR on purpose); after a local recipe change `omaf build nixos/lab` asks to choose between the old and new version of the same recipe; the bundled flake's lock points at `main` (403d548), which has no `omaflake-gui` yet – re-lock after `rust-core` is merged; clicking the tray menu and the app's pages in the VM not tried (no input injection). | `nix/flake.nix`, `nix/flake.lock`, `crates/omaflake-core/src/guest_settings.rs`, `crates/omaflake-gui/src/app.rs`, recipes `omaflake/nixos/lab`, docs/linux.md |
| B124 | Linux as a host (issue #2) | **Done** 2026-10-24 for aarch64 under TCG and for x86_64 with KVM (Ubuntu 26.04.1, Ryzen 9 mini PC – NixOS VMs start; gaps found there: docs/linux.md, "Checking on a real machine"): omaflake runs NixOS guests on a Linux host. **QEMU:** `QemuCommand` takes a `Target` (arch, accelerator, window toolkit, sound) – the Mac's (`Target::MAC`) gives the arguments word for word as before (golden test, also run against the previous code); Linux: `qemu-system-<host arch>`, `q35` + `-vga none` (x86_64) or `virt,highmem=on,gic-version=host` / `=max` (aarch64), `-accel kvm -cpu host` when `/dev/kvm` opens read-write, else `-accel tcg -cpu max` (`OMAFLAKE_ACCEL=tcg` forces it), GTK windows (`gl=on`; `virtio-vga[-gl]` on x86_64, `virtio-gpu[-gl]-pci` on aarch64), `-audiodev pipewire` / `pa` when the session has the socket, else no sound card; disk, network, 9p (mapped-xattr: the guest-owner patch is the Mac build's), agent ports, QMP, serial log unchanged. Firmware (`qemu_host::Firmware`): QEMU's edk2 next to the binary, else QEMU's firmware descriptors (`/etc/qemu/firmware`, `/usr/share/qemu/firmware`; UEFI, split flash, no Secure Boot/SMM); x86_64's `efivars.fd` becomes a copy of `edk2-i386-vars.fd` at start (a blank store does not boot OVMF). Verified with nixpkgs' QEMU 11.1.2 on aarch64-linux: `-accel help` (aarch64: kvm, tcg), `-machine help`/`virt,help` (gic-version host/max), `-device help` (virtio-vga, virtio-vga-gl only in the x86_64 binary), `-display help` (gtk, sdl, …), `-audiodev help` (pa, pipewire); nixpkgs' `qemu` for both Linux systems ships `edk2-{x86_64,aarch64}-code.fd` and `edk2-i386-vars.fd` (cache.nixos.org listing). The accelerator is kept in the run folder (`accel`) and shown: `Vm.accelerator`, an info line "Accelerator" (TCG with a warning), `omaf start` prints a line for TCG. **Images:** `nix build images.<host system>.<recipe>` here (unchanged code; no builder items on Linux); the image flake evaluates `images.x86_64-linux."omaflake.nixos.neovim-minimal"` (a recipe copy with `systems` widened); x86_64 guests get `console=tty0 console=ttyS0,115200` (aarch64's firmware names its PL011 itself – aarch64 drvPath unchanged). Found on the way: a guest settings file without `email` (omaflake leaves it out) failed every build – the flake defaults it to null. **Doctor on Linux:** host (`Linux <release>, <arch>`), `kvm` (new item; ok / limited with the fix: NixOS `extraGroups = [ "kvm" ]`, elsewhere `usermod -aG kvm` as Install …, or virtualization in the firmware – systemd's own rule makes /dev/kvm 0666, so NixOS usually needs nothing; verified in systemd's source), Nix (nixos.org installer in a terminal, not the Mac's admin dialog), flakes, QEMU with version/3D/firmware (`nix profile install nixpkgs#qemu`), flake, SSH key, git (`nixpkgs#git`), `cli`; preflight flakeEval and nixSpace only; no macOS section (view and `omaf doctor`). **Port in use after a stop (B123):** cause pinned down – QEMU closes an open SSH session's connection first when the VM stops, leaving TIME_WAIT on `127.0.0.1:<port>`; the old check's plain bind failed on it (`EADDRINUSE`, reproduced in the VM with `ss` and a plain bind) while QEMU's hostfwd binds with SO_REUSEADDR (libslirp). `Ports::is_free` now binds the same way and refuses a port that accepts a connection on 127.0.0.1 (macOS lets a REUSEADDR bind through next to a wildcard listener); unit test. **End to end** (2026-10-24, aarch64-linux omaflake VM `neovim-minimal` on the Mac, 8 vCPU/16 GB, no nested virtualization → TCG; `omaf` built in the VM from this branch, temporary `VM_HOME`): `omaf doctor` (host, kvm limited "no /dev/kvm", QEMU 11.1.2 3D + firmware, no macOS section); `omaf build neovim-minimal` natively **199 s** (substitutes, the agent compiled, the disk image's build VM itself under TCG); an image exported from the Mac imported (121 s over 9p) – usable through `vms.create` only, `omaf new -r` does not find it (its line names the Mac's recipe folder, see OPEN); `omaf new`, `omaf start --headless` (TCG line), `omaf wait`: SSH after **92 s** (82–87 s in three more runs; 4 vCPU, 4 GB), `omaf ssh … uname -a`, `vms.get` accelerator "tcg", `systemctl is-system-running` running, `omaf stop` 7 s, `omaf start` right away – started (also with an SSH session open during the stop, where the old check fails), `omaf rm`. x86_64 arguments probed under TCG in that VM: QEMU accepts the list (q35, pflash with the vars template, `-vga none`, virtio devices at their slots, 9p, agent port, QMP, serial) and OVMF comes up and lists the virtio disk on its serial port. **OPEN:** x86_64 and KVM on real hardware (steps in docs/linux.md – the maintainer's mini PC); a window (GTK, GL) and sound on a Linux desktop (agent bridge, clipboard, camera: B126); distributions' QEMU/OVMF descriptors; an image imported from another computer is not the current one of the local recipe line (`omaf new -r` refuses it); a checkout that is a git worktree, shared into a VM, breaks the `--override-input omaflake` (its `.git` file points outside); texts new in this change: German since B125. | `cargo test` (qemu_command per arch × accelerator × display, qemu_host, prerequisites, views, sockets), `nix eval` of both systems, the TCG cycle above |
| B125 | German everywhere (errors included) | **Done** 2026-10-24: every text a user reads in the macOS app, the Linux app and `omaf ui` has German; the CLI stays English (maintainer's decision, docs/cli.md). **Errors:** `vm_err!` is now a proc macro (`crates/omaflake-macros`, no dependencies) behind omaflake-core's `macro_rules!` wrapper: it keeps the format string as the error's key (`{…}` → `%@`, `{{`/`}}` → braces, `%` → `%%`) and formats each placeholder's value once, exactly as `format!` would (one `format!` of the placeholders joined by a separator; names taken from the scope become named args with the literal's span), and `Error` holds a `Text` (key, English text, args) next to its code; `vm_err!(codes::X, "…", …)` sets a code. `Error::new`/`with_code`/`RpcError::server` call sites in the core and the API (≈120) became `vm_err!` (a script, then by hand); io errors take the OS's text as their key, `with_path`/`context` keep the inner key under a `%@: %@`/`%@%@` frame. The API adds `data.text` to `-32000` errors, `errorText` to failed jobs and `text` to a page's `error` model (api.md §3); activity lines and the build page carry the error as a text arg. The Swift app (`APIError.text`, `describe`, `ViewError.text`), the TUI (`CoreError`, a page's error) and the Linux app (`CallError`, `Model::Error`) render it like any view text. **German:** 13 view texts (the Linux host's Doctor hints, KVM, the accelerator line) and 163 error texts (144 messages, 19 common OS errors) added to `de.lproj` (the generated `omaflake-i18n` table follows); the existing terms (Vorlage, Rezept, VM, Base, Doktor). **Tests:** `api::translated` fails for a view key – from all golden files (new: `doctor.linux`, `vm.linux`) and the pages built for both hosts with every Doctor check in every state – or a `vm_err!` key in the sources without German, and when a German error text does not take its args; the macro's key equals the reader's. Coverage: 360 view keys (6 joints like `%@ · %@` need none) and 181 error keys, all with German. **Left:** errors still built at run time without a key – `qemu_runtime.rs` (lock/port/already running, 8 sites; another branch works there), the `JSON:`/`TOML:`/`HTTP:` wrappers, a builder's stderr, the `-32602` protocol errors (for developers); values inside a message (an OS's or a tool's text, a nested error formatted with `{e}`) stay English; job titles ("Build dev") are data. | `cargo test` (api::translated, error, protocol, views), `swift test`, `OMAF_BIN=… swift test` |
| B126 | Integrations on a Linux host: agent, clipboard, camera, sound | **Done** 2026-10-24 (aarch64 under TCG; a real webcam and x86_64 OPEN): a NixOS VM on a Linux host gets what it gets on a Mac. **Bridge:** `omaf bridge` (hidden; `crates/omaflake-core/src/host_bridge/`) is the Linux `omaf-runner bridge` – `QemuRuntime::start` spawns it detached next to QEMU (`bridge.log`), always for the agent, `--clipboard`/`--camera` as switches (the Mac's runner keeps the old ports' sockets; `bridge_arguments` test); it finds this omaflake's `omaf` (`CliLink::running_omaf`, else PATH). It runs the core's `AgentLink` (welcome, `agent.json`, limits – unchanged), so readiness by the agent and `omaf agent` work; it ends when QEMU closes the port. Only images whose agent carries clipboard and camera (B106 on) – the old ports stay unserved on Linux. **Clipboard** (`clipboard.rs`, as `ClipboardBridge` in Bridge.swift: text and PNG, fingerprints against the echo, nothing sent before the first copy): a `ClipboardBackend` trait; Wayland = `wl-paste --watch` + `wl-paste`/`wl-copy` (needs the data-control protocol), X11 = `xclip` polling the selection's TIMESTAMP every 0.5 s; chosen by `WAYLAND_DISPLAY`/`DISPLAY` and the tools found, `OMAFLAKE_CLIPBOARD` forces it; tools with time limits, the watcher dies with the bridge (PR_SET_PDEATHSIG). Chosen over crates (wl-clipboard-rs, arboard): no new dependencies, the guest agent's proven approach, no root. The Nix package puts wl-clipboard and xclip on the app's PATH (after the user's). **Camera** (`camera.rs`, `v4l2.rs`, `convert.rs`): as Camera.swift – on only between start and stop, frames against credits (`CreditGate`: at most 4, the newest frame waits, no credit for 2 s stops; the start counts as a credit time), status ready/streaming/unavailable/stopped; V4L2 by plain ioctls (mmap streaming, 4 buffers; sizes/offsets/request numbers checked with a C program against the nixpkgs kernel headers on aarch64-linux: v4l2_capability 104, v4l2_format 208, v4l2_requestbuffers 20, v4l2_buffer 88, VIDIOC_* as in the code), the first `/dev/video*` that streams NV12, YUYV or MJPEG (or `OMAFLAKE_CAMERA_DEVICE`), converted to 1280×720 NV12 (nearest neighbour; YUYV chroma of even rows; MJPEG by zune-jpeg – already in Cargo.lock through iced – to RGB, then BT.709 video range); `--camera-test-pattern`. **Sound:** unchanged from B124 – checked now: nixpkgs' QEMU 11.1.2 (`-audiodev help`: pa, pipewire, …; `-device help`: intel-hda, hda-output, hda-micro). **Tests:** a fake guest on a socket pair through the whole bridge (hello → welcome with only the allowed features, status → agent.json, clipboard both ways incl. 600 KB chunked and the echo skipped, camera frames against credits and the 2 s stop, camera off for the VM), a fake clipboard, conversions from synthetic NV12 (padded rows), YUYV (720p and 640×480 scaled) and a real MJPEG frame (`testdata/camera/half.jpg`, made with ffmpeg), the credit gate. **Tried** (2026-10-24, omaflake VM `neovim-minimal` on the Mac as the Linux host, Hyprland session with PipeWire, 8 vCPU; `omaf` built in it from this branch; a nested `neovim-minimal` VM from an image exported from the Mac, TCG): `omaf agent nested` – the report (hello ~2 min after start under TCG), welcome features camera, clipboard, status; clipboard host ↔ guest: text with umlauts both ways, PNG both ways byte-identical (sha256), 600 KB text host → guest identical; sound: the guest has playback and capture (`/proc/asound/pcm`, PipeWire sink and source), the host's PipeWire shows QEMU's output and input streams; camera: the outer VM's own `/dev/video42` (fed by the Mac's test pattern) as the host camera – `dd` of 20 frames in the nested guest took 1.24 s (≈16 fps, the pattern's 15), the frames differ (moving bars), the bridge logged on/off and the guest agent start/stop; `omaf stop` → bridge and its wl-paste gone. X11 (the outer VM's Xwayland, `OMAFLAKE_CLIPBOARD=x11`, nixpkgs' xclip 0.13): the real `omaf bridge` against a fake guest on its agent socket (a Python script: hello, welcome with clipboard) – host → guest and guest → host text with umlauts, no echo sent back; the same script for Wayland. (Two more nested boots under a loaded Mac ended in emergency mode: systemd's 90 s wait for the ESP ran out under TCG – not the bridge's doing.) Also: the keyless errors left in `qemu_runtime.rs` (firmware, already running, base image missing, SSH port, virtio-gpu-gl, lock files) and the busy messages of `FileLock` are `vm_err!` now, with German. Linux-only code from B125 that did not build on Linux (`source_tokens.rs` without `vm_err`, `mac_vm.rs` without `Error`) fixed; clippy clean and `cargo test -p omaflake-core` green on aarch64-linux. **OPEN:** a real webcam (YUYV/MJPEG over V4L2 – unit tests only; v4l2loopback devices need root to add), x86_64; GNOME (Mutter has no data-control: `wl-paste --watch` fails – XWayland's xclip would be the fallback, untried); a core service started by systemd without `WAYLAND_DISPLAY` (bridge says "no desktop session"); 4:3 cameras are stretched, not letterboxed; the bridge's log lines are English (like omaf-runner's). | `cargo test` (host_bridge, qemu_runtime), C ABI probe, the nested VM above |

**macOS track (as of 2026-10-16)** – separate ids **BM…** so they do not collide with the NixOS findings B23–B25 that were recorded in parallel (merged 2026-10-16; BMn = former Bn+22).

BM2–BM7 concern tart; since BM10 (tart replaced) they are background/comparison only.

| # | Assumption | Finding | Evidence |
|---|---|---|---|
| BM1 | nix-darwin can configure Dock, apps, wallpaper, widgets | ✅ `system.defaults.dock.persistent-apps`, `system.primaryUser`, `homebrew.casks`, `homebrew.masApps` (needs App Store sign-in), `homebrew.onActivation.cleanup`, `system.defaults.CustomUserPreferences` exist. ❌ No option for wallpaper or widget content (nothing about `wallpaper/desktopPicture/widget` in `system.defaults`); only show/hide via `system.defaults.WindowManager.StandardHideWidgets`/`StageManagerHideWidgets` (set on the host via System Settings, verified with `defaults read com.apple.WindowManager`). WidgetKit `WidgetCenter` (macOS 27 SDK) can only query/reload the app's own widgets, not place any. A `com.apple.desktopservices` domain with a widget switch is not confirmed. | `nix eval .#darwinConfigurations.host.options.…` |
| BM2 | tart origin and license | ℹ️ tart 2.40.1 is installed from the tap `openai/tools` (homepage github.com/openai/tart), license **FSL-1.1-ALv2** (Functional Source License, not OSI open source). A formula with the same name also exists in `cirruslabs/cli` → `brew info tart` is ambiguous. | `brew info --json=v2 openai/tools/tart` |
| BM3 | Overlay model possible with tart | ✅ `tart clone --stacked` ("stacked disk that uses the source image as an immutable base"); `tart set --random-mac --random-serial` for clones; `tart rename`, `tart stop`, `tart suspend`. | `tart clone/set --help` |
| BM4 | Guest control | ✅ SSH active in the vanilla image (banner `SSH-2.0-OpenSSH_10.3`, `tart ip` → `192.168.64.14`). ⚠️ `tart exec` needs the Tart Guest Agent – according to the help only preinstalled in **non**-vanilla images. Clipboard likewise only with the guest agent (`tart run --no-clipboard` help). Formula `cirruslabs/cli/tart-guest-agent` or `openai/tools/tart-guest-agent`. | `tart exec --help`, `tart run --help`, `brew search` |
| BM5 | Network modes | ℹ️ Default NAT (`192.168.64.x`); `--net-host` (host-only), `--net-bridged`, `--net-softnet` with `--net-softnet-allow/-block/-expose`; `softnet` lives at `/opt/homebrew/bin/softnet`. No "no network" switch. | `tart run --help` |
| BM6 | Storage of a tart VM | ℹ️ `~/.tart/vms/<name>/{config.json,disk.img,nvram.bin,control.sock}`; disk format of the vanilla clone: `asif`. `tart list --format json` returns `Name`, `Running`, `State`, `Source` → status without own PID. | `ls`, `tart get`, `tart list --format json` |
| BM7 | Runtime options | ℹ️ `tart run --no-graphics`, `--dir [name:]path[:ro][,tag=…]` (virtiofs; mount point in guest OPEN), `--capture-system-keys`, `--suspendable`, `--root-disk-opts`, `--nested`. | `tart run --help` |
| BM8 | Own runner possible | ✅ The macOS 27 SDK contains everything needed: `VZMacOSRestoreImage` (`latestSupported`, local), `VZMacOSInstaller`, `VZMacHardwareModel`, `VZMacMachineIdentifier`, `VZMacAuxiliaryStorage`, `VZMacGraphicsDeviceConfiguration`, `VZVirtualMachineView`, `VZNATNetworkDeviceAttachment`, `VZVirtioFileSystemDeviceConfiguration`, `VZSingleDirectoryShare`/`VZMultipleDirectoryShare`, `VZSpiceAgentPortAttachment` (`sharesClipboard`). Entitlement `com.apple.security.virtualization` required per header; bridged networking would additionally need `com.apple.vm.networking`. | `Virtualization.framework/Headers` (MacOSX27.0.sdk) |
| BM9 | tart folder directly readable | ⚠️ `config.json` contains `hardwareModel`/`ecid` as base64 plist (= `dataRepresentation`), `nvram.bin` is the auxiliary storage. But: per header `VZDiskImageStorageDeviceAttachment` supports **RAW only**; `gg-vanilla` uses `asif` → not directly usable. Own VMs therefore use a RAW disk (sparse; APFS `clonefile` for clones). No longer needed after the decision to replace tart. | `config.json`, `VZDiskImageStorageDeviceAttachment.h` |
| BM10 | tart license and open source | ❌ FSL-1.1-ALv2 (BM2) does not fit "everything open source" → **DECIDED (2026-10-16): replace tart entirely**, base built from Apple's IPSW instead of Cirrus Labs images. | Design discussion 2026-10-16 |
| BM11 | Setup Assistant must be done by hand | ✅ **No (from macOS 27):** `VZMacGuestProvisioningOptions` (`fullName`, `username`, `password`, `logsInAutomatically`, `enablesRemoteLogin`) via `VZMacOSVirtualMachineStartOptions.setGuestProvisioning(_:)`; only on the first start after installation, guest must be macOS 27+. → guest user, auto-login, SSH without manual steps; `omaf-runner run --user … --password-stdin`. | `VZMacGuestProvisioningOptions.h` (MacOSX27.0.sdk) |
| BM12 | Ad-hoc signature is enough | ✅ `omaf-runner` ad-hoc signed with `com.apple.security.virtualization`: `VZMacOSRestoreImage.latestSupported` returns macOS 27.0.1 (build `26A434`, 26.6 GB). | `omaf-runner latest` |
| BM13 | Progress during `install` | ⚠️ `Timer` does not fire under `dispatchMain()` (no run loop) → `DispatchSourceTimer`. | First download run |
| BM14 | Installation from IPSW | ✅ `omaf-runner install --latest`: download 26.6 GB in ~18 min, installation of macOS 27.0.1 (`26A434`) in ~4 min; `disk.img` 80 GB thin, ~26 GB used. Fresh installation kept as APFS clone `~/.omaflake/bases/mac-fresh-26A434` (read-only) – provisioning only applies on the first start. | `~/.omaflake/logs/install-mac-test.log` |
| BM15 | Provisioning works | ✅ First start with `--user <name> --auto-login --ssh`: no Setup Assistant, guest user's desktop logged in (confirmed by manual check), SSH banner `OpenSSH_10.3`; NAT IP via `omaf-runner ip` from the DHCP leases. Headless restart: SSH after ~6 s. | Test 2026-10-16 |
| BM16 | Limit of 2 macOS VMs | ✅ Enforced by the framework: third VM → `VZErrorDomain 6` "The maximum supported number of active virtual machines has been reached" (start fails immediately, no crash). | Test with two clones |
| BM17 | Shutdown via `requestStop()` | ⚠️ Acts like the power button: with a logged-in user macOS shows the dialog "Are you sure you want to shut down your computer now?" – headless, the VM then stays up. (First run with a window shut down in ~24 s; reason for the difference OPEN.) → stop as an escalation: shut down via SSH (`sudo shutdown -h now`, NOPASSWD rule via nix-darwin, M9) → `requestStop()` → after timeout `stop()` (hard). Implemented in `VZBackend` (M10). | Screenshot |
| BM18 | Clone with a new identity | ✅ `omaf-runner clone`: `clonefile` per VM file (no extra space), new `VZMacMachineIdentifier` and MAC. A clone of the base boots headless to SSH in ~15 s; base and clone get different NAT addresses. | `omaf-runner clone`, `ip` |
| BM19 | `nix copy` into a macOS guest | ⚠️ Paths built on the Mac are unsigned → the guest refuses them ("lacks a signature by a trusted key"). Needs the guest user in `trusted-users` **and** `nix copy --no-check-sigs`. Store URL `ssh-ng://<user>@<ip>?remote-program=/nix/var/nix/profiles/default/bin/nix-daemon` (non-interactive SSH has no Nix in `PATH`). `nix store info` prints to stderr – use `--json` to check `trusted`. After a reboot `/nix` is mounted only after ~15 s. | `scripts/macos-build-image.sh` |
| BM20 | Image build without manual steps | ✅ 2026-10-16: `scripts/macos-build-image.sh mac-minimal` – clone, Nix 2.34.9 (official multi-user installer as admin with a temporary `NOPASSWD` rule), copy 237 paths/1.8 GiB, activate, shutdown via SSH: ~3 min. nix-darwin moves nothing by itself: `/etc/nix/nix.conf`, `/etc/bashrc`, `/etc/zshrc` are renamed to `*.before-nix-darwin` first. | build log |
| BM21 | Screenshot from inside the guest | ❌ `screencapture` over SSH: "could not create image from display"; `launchctl asuser` not permitted (TCC/audit session). → check Dock/wallpaper in the window, or capture the runner window on the host. | SSH |
| BM22 | Look and Dock by hand → keys | ✅ Set by hand in a guest, then diffed `defaults export` of all domains: dark = `AppleInterfaceStyle=Dark`; graphite = `AppleAccentColor=-1`, `AppleAquaColorVariant=6`, `AppleHighlightColor`; icon style Clear = `AppleIconAppearanceTheme=ClearDark` (nix-darwin option) + `NSGlassTintAmount=0`; folder color = `AppleIconAppearanceTintColor=Graphite`; solid black wallpaper = `~/Library/Application Support/com.apple.wallpaper/Store/Index.plist` (provider `com.apple.wallpaper.choice.color`, `systemColor black`). Applied in `mac-minimal.nix` and checked in a fresh clone. | `defaults export`, `plutil` |
| BM23 | Per-host settings in clones | ⚠️ Every clone gets a new `IOPlatformUUID` (new machine identifier) → `~/Library/Preferences/ByHost/*.<UUID>.plist` (e.g. menu bar items in `com.apple.controlcenter`: `8` = hidden) do not carry over from the image. ✅ Writing the same keys to the any-host domain works: preferences fall back to it while the per-host file has no value (checked in a fresh clone). A login LaunchAgent also worked but adds a background item (BM25). | `ioreg`, `ls ByHost` |
| BM24 | Safari settings and favorites | ✅ Settings live in the sandbox container `~/Library/Containers/com.apple.Safari/Data/Library/Preferences/com.apple.Safari.plist` (exists from the base's first login; writable over SSH). ⚠️ On its first launch Safari imports its default favorites and **overwrites** any `~/Library/Safari/Bookmarks.plist` put there before; it only starts that import once a bookmarks file exists. → build: seed an empty file, launch Safari, wait for the import, quit, then empty Favorites/bookmarks menu with `PlistBuddy` – stays empty across launches. `/Applications/Safari.app` is a symlink (alias arrow in the Dock); the Dock uses `/System/Volumes/Preboot/Cryptexes/App/System/Applications/Safari.app`. | `plutil`, Safari |
| BM25 | "sh can run in the background" | ⚠️ macOS lists `org.nixos.nix-daemon`, `org.nixos.activate-system` (and any `launchd.user.agents`) as background items of `/bin/sh` with "not notified" and notifies in every clone; dismissing does not change the state (`sfltool dumpbtm`). Waiting before shutdown does not help. → no own LaunchAgents; the two Nix items remain (OPEN). 2026-10-17 research: a known open nix-darwin issue – its daemons run through `/bin/sh -c` (waiting for the store), so macOS names them "sh" ([nix-darwin#1678](https://github.com/nix-darwin/nix-darwin/issues/1678), [#558](https://github.com/nix-darwin/nix-darwin/issues/558)); no fix there. Left to try: a configuration profile with managed login items (`com.apple.servicemanagement`) installed once in the base – whether macOS 27 accepts it without MDM is **OPEN** (needs one approval click in the base). 2026-11-27 (B190): **no** – a profile with a `com.apple.servicemanagement` payload (rules `LabelPrefix` `org.nixos.` and `app.omaflake.`) is refused when installed by hand in a macOS 27 guest: "Das Profil muss von einem durch Benutzer:in genehmigten MDM-Server stammen." A launch agent of the user's is announced again at every start of the same VM, also after the notice was dismissed (checked by the maintainer) – so omaflake's own helpers stay started over SSH. **The "sh" notices are gone (2026-11-27):** macOS names a background item after the program it starts (its record in `/private/var/db/com.apple.backgroundtaskmanagement/BackgroundItems-v18-*.btm`, an NSKeyedArchiver file anyone can read: `name`, `disposition` – 0x3 allowed and enabled, 0xb also notified); darwin-guest.nix starts every launchd job that ran `/bin/sh` through `/usr/local/libexec/omaflake` (a script outside /nix, put there by the activation), so they are "omaflake". `AssociatedBundleIdentifiers` (omaflake Tools) was ignored – most likely only with one team for program and app (OPEN, with a Developer ID). Checked on the Mac: a fresh VM of the new image and the `test` VM upgraded in place show no notice at their starts, every item recorded as notified (0xb). Not understood: a fresh VM of an image built before had `nix-daemon` and `darwin-store` at 0x3 (and the notice), those of images built since have them at 0xb – most likely the build VM's run records them (OPEN). Not needed any more: approving the items in the base, writing the database (it is not protected by SIP – writable with sudo also with it on). | `sfltool dumpbtm`, nix-darwin issues |
| BM26 | Desktop widgets | ✅ The base's first login places Calendar, Weather and Photos: `widgets` in `~/Library/Containers/com.apple.notificationcenterui/Data/Library/Preferences/com.apple.notificationcenterui.plist` (`DesktopWidgetPlacementStorage` + `instances`). Without the key Notification Center adds them again; an empty entry (no displays, no instances) stays. `StandardHideWidgets` does not cover "click wallpaper to reveal desktop". Language/region: `AppleLanguages`/`AppleLocale` (user and `/Library/Preferences/.GlobalPreferences`); "German – Standard" = `com.apple.keylayout.German-DIN-2137` (ID −18133). | `plutil`, maintainer in the window |
| BM27 | `VZBackend` (M10) | ✅ 2026-10-16 on `mac-minimal`: `omaf new` (APFS clone 0.4 s) → `start --wait` (SSH after 11–18 s, NAT address in `omaf list`) → `ssh` (user `test`, macOS 27.0.1) → `stop` via SSH shutdown 14 s, no dialog → restart keeps a marker file → `reset` → marker gone → `rm` to the trash. Ephemeral: marker gone after stop/start. Third macOS VM refused before starting with a clear message (BM16). `--window` passes `--window` to the runner. **Decisions:** ephemeral = fresh clone per start (old clone to the trash; ephemeral instances are cloned only at start); `isolated` refused – Virtualization.framework NAT has no restricted mode, `VZVmnetNetworkDeviceAttachment` (macOS 26) needs the vmnet entitlement ("The vmnet framework requires an entitlement", SDK header), not grantable ad hoc. Disk size of the image is kept (APFS resize in the guest: OPEN); `omaf list` shows "clone" instead of a size (the private share of an APFS clone is not cheap to read). Host name in the guest is the flavor's (`mac-minimal`), not the instance name → BM28. | `omaf` cycle, `runner.log`, SDK `VZVmnetNetworkDeviceAttachment.h` |
| BM28 | `omaf macos install`, `omaf build` for macOS, host name per instance (M10) | ✅ 2026-10-16: `omaf build mac-minimal` (`MacImageBuilder`, replaces `scripts/macos-build-image.sh`) – closure on the Mac, clone, Nix 2.34.9, copy, activate, shutdown: ~3 min, image read-only, `current/mac-minimal` switched. Wrong sudo password: clear error after ~30 s, runner killed, build folder to the trash. `omaf macos install` reuses `mac-fresh-26A434`/`mac-base-26A434` (Apple's latest build checked via `omaf-runner latest` first); a new installation goes to `bases/.install-*` and is renamed after its build. Host name: nix-darwin sets `HostName`/`LocalHostName` only in the activation script (`nix eval …activationScripts.networking.text`), so every clone kept `mac-minimal`; `omaf wait`/`start --wait` now set HostName, LocalHostName and ComputerName to the instance name via `scutil` (narrow `NOPASSWD` rule in `darwin-guest.nix`; older images keep the flavor's name). Checked: `new → start --wait → ssh` (all three names `m10a`) → stop/start keeps a marker → `reset` → marker gone → `rm`; ephemeral: marker gone, name set again after every start. The full install from an IPSW was not rerun (the base exists, M7/M8). | `omaf` cycle, `~/.omaflake/logs/build-*.log` |
| BM29 | macOS base without manual steps | ✅ 2026-10-16: `omaf macos install` (`MacBaseInstaller`) reuses `mac-fresh-26A434`, clones it, first boot **headless** with provisioning (works without a window), installs `~/.ssh/*.pub` via password SSH (askpass helper in `cache/`, password only in its environment), checks the key login, shuts down via `sudo -S`, read-only: 56 s with a throwaway user. Moving a read-only folder needs write access on the folder itself (its `..` entry) → `Trash.move` adds `u+w` to the top folder. omaflake distributes no macOS images; whether Apple's license would allow a prepared base: OPEN, not pursued. | `omaf macos install`, `logs/first-boot-*.log` |
| BM30 | First boot of a fresh base without SSH | ⚠️ 2026-10-16: a base created from the app (fresh installation, user = Mac user) got its NAT address at once, but SSH did not answer within 10 minutes; booted again without provisioning, SSH answered after 10 s – provisioning had worked. Cause OPEN (the earlier test base answered on its first boot). → `MacBaseInstaller`: after 8 minutes without SSH the VM is restarted once without provisioning; an unfinished base (no `.omaflake-base-ready` marker) is continued instead of skipped, and only a finished base counts (`newestBase`, Doctor). | `logs/first-boot-*.log`, DHCP leases |

### 1.3 Gaps in the concept

1. **GPU is the critical path but needlessly blocks everything else.** CLI and app can be built and tested entirely
   *headless* (SSH + serial console). → Reorder milestones (§4).
2. **Process lifecycle is missing.** QEMU must outlive app/CLI (restarting the app must not kill VMs).
   State must be reconstructible from files/sockets (`-pidfile`, QMP), not from app memory.
   `-daemonize` with the Cocoa display: **OPEN** (Cocoa needs the main thread) → better detach via `posix_spawn` + `setsid`.
3. **Ephemeral mode designed twice** (`-snapshot` *and* overlay). Simpler and backend-independent:
   ephemeral = **fresh overlay + fresh efivars before every start**. One mechanism, equally testable.
4. **Configuration format.** Swift has no TOML parser in its standard library. Proposal:
   keep `flavors.toml` (Nix can read it too via `builtins.fromTOML` → `disk_gb` feeds `virtualisation.diskSize`, *one* source);
   `instance.toml` becomes `instance.json` (Codable, no dependency). ~~TOML library~~ → done in M3: Nix delivers `flavors.toml` as JSON.
5. **SSH access for automation.** `initialPassword` alone is not enough for `omaf ssh`/readiness checks.
   → Put the user's public SSH key into the image via `users.users.<user>.openssh.authorizedKeys.keys` (public keys are not secrets).
   Host keys change per instance → separate `known_hosts` per instance.
6. **Port allocation** (SSH forwarding) and collision checks are missing.
7. **Image revision without Git** (B2) – either introduce Git (recommended) or use the flake's `narHash`.
8. **App packaging.** `MenuBarExtra` needs an app bundle (`LSUIElement`). A plain SwiftPM executable is not enough → bundle it (see M4). No App Sandbox (spawning processes, `~/.omaflake`, Unix sockets).
9. **Cleanup.** `_to_delete/` grows without bound; overlays whose backing image was moved become unusable → never move images while an instance points to them.

---

## 2. DECISION: graphics backend

Only relevant for desktop flavors; headless works with Homebrew QEMU right away.

| Option | Pros | Cons |
|---|---|---|
| **A – Own QEMU build** (virglrenderer + ANGLE, based on try-omarchy / UTM build scripts) | full control, one invocation chain, matches the spec | build effort, ongoing maintenance on QEMU updates, hypervisor signature needed (B8) |
| **B – UTM as backend** (app generates `.utm` bundles, controls them via `utmctl`) | GPU, clipboard (SPICE), window ready out of the box | dependency on UTM's config format, sandbox vs. overlay backing files **OPEN**, less control |
| **C – Hybrid** | Homebrew QEMU headless, desktop via B; A possible later | two backends |

**DECIDED (2026-10-16): Option A – own QEMU build with virglrenderer + ANGLE.**
Headless operation (M1/M3) keeps using Homebrew QEMU until the own build is ready; the
`VMBackend` protocol stays so that `QEMUBackend` only switches the binary path. UTM remains an emergency fallback only.

---

## 3. Target architecture

```
<repo root>                       Swift package (Swift 6, minimum macOS 15+ – OPEN)
  Package.swift
  Sources/
    OmaflakeCore/
      Model/      Flavor, FlavorCatalog (flavors.toml), Instance, InstanceConfig (instance.json),
                  ImageRef (flavor, rev, path)
      Store/      Paths (~/.omaflake), ImageStore, InstanceStore, Trash (_to_delete, never delete)
      Build/      NixImageBuilder (nix build --print-out-paths, log as AsyncStream)
      Disk/       DiskManager (qemu-img create -b … -F qcow2, info, create efivars)
      Runtime/    VMBackend (protocol), QEMUBackend, (later UTMBackend),
                  QEMUCommand (pure function: config → [String]), ProcessLauncher (posix_spawn, setsid),
                  QMPClient (JSON over Unix socket: qmp_capabilities, query-status, system_powerdown, quit),
                  PortAllocator, StatusProbe (pidfile + QMP + SSH reachability)
      Util/       Shell (async Process), Logger (os.Logger)
    omaf/         CLI (swift-argument-parser)
    OmaflakeApp/  menu bar app (MenuBarExtra + windows), bundled by scripts/bundle-app.sh
  Tests/OmaflakeCoreTests/
```

**Core interface**

```swift
protocol VMBackend: Sendable {
    func start(_ instance: Instance) async throws          // returns once QEMU is running
    func stop(_ instance: Instance, timeout: Duration) async throws   // QMP powerdown → quit → SIGKILL
    func status(_ instance: Instance) async -> VMStatus     // .stopped/.starting/.running/.stopping/.crashed
}
```

**Runtime rules**
- State lives exclusively in `~/.omaflake/instances/<name>/` (`instance.json`, `qemu.pid`, `qmp.sock`, `serial.log`, `qemu.log`).
- An `actor InstanceManager` serializes actions per instance (no double start); a lock file guards against CLI and app acting at the same time.
- `QEMUCommand` is pure and unit-testable (expected argument list per flavor/network mode).

**Data (adjustment to spec §5)**

```
~/.omaflake/images/<flavor>-<rev>.qcow2       read-only (chmod 444); rev = short Git hash (or narHash)
~/.omaflake/images/current/<flavor>.qcow2     symlink
~/.omaflake/instances/<name>/instance.json    instead of instance.toml
~/.omaflake/instances/<name>/{disk.qcow2,efivars.fd,qemu.pid,qmp.sock,serial.log,qemu.log,known_hosts}
~/.omaflake/share/<name>/
~/.omaflake/_to_delete/<timestamp>-<what>/
```

---

## 4. Milestones (revised)

Order: foundation → image → **headless** CLI → app → graphics → comfort. A GPU spike runs early but does not block.

### M0 – Repo foundation
- [x] `git init`, `.gitignore` (`result*`, `.DS_Store`), first commit
- [x] Generate and commit `flake.lock`
- [x] `nix flake check` green
- [x] README: label note (the NixOS installer formats `boot`, `vm-guest.nix` expects `ESP` – for installer-based VMs this is overridden by `hardware-configuration.nix`; document it)
**Done when:** `nix flake check` passes, repo committed.

### M1 – Image build
- [x] nix-darwin host config with `nix.linux-builder.enable = true` (options checked against nix-darwin/nixpkgs source), size prepared as stage 2 (see `docs/linux-builder.md`)
- [x] Activation (`sudo`), then stage 2 (6 cores, 12 GiB, 100 GiB) – the default size ran out of space on the first build
- [x] `flake.nix`: `packages.aarch64-linux.<flavor>-image = nixosConfigurations.vm-<flavor>.config.system.build.images.qemu-efi` (+ alias `images.<flavor>`); flavor list from `flavors.toml`
- [x] The user's public SSH key in the base module (`authorizedKeys.keyFiles`)
- [x] ~~`flavors.toml` → `virtualisation.diskSize`~~ rejected: image stays `auto` (small, faster build); the CLI sets `disk_gb` as the overlay size, `growPartition`/`autoResize` grow it in the guest
- [x] Headless boot probe with Homebrew QEMU (`scripts/bootprobe.sh`): `minimal` boots, SSH with key, labels `ESP`/`nixos`, root grown to 40 GiB, network ok, QMP `system_powerdown` ends QEMU after ~3 s
- [x] `images.dev` built (qcow2 10.8 GiB virtual / 7.6 GiB used) and booted headless: SSH, labels, docker active, no failed units, toolchain present
**Done when:** `nix build .#images.minimal` yields a qcow2 that boots headless and is reachable via `ssh -p <port> <user>@127.0.0.1`; labels verified (`lsblk -f` in the guest).

### M2 – Own QEMU build (option A)
- [x] Build approach: reviewed try-omarchy (MIT); QEMU 11.1.2 + 6 selected try-omarchy patches, virglrenderer 1.3.0 + startergo patches, libepoxy from source, **ANGLE as prebuilt startergo bottle** (DECIDED). No SPICE needed.
- [x] `qemu/build.sh` with sha256 pins; output `~/.omaflake/qemu/<tag>/`, symlink `current` (details in `qemu/README.md`)
- [x] Ad-hoc signed with `com.apple.security.hypervisor` (`qemu/entitlements.plist`); `xattr -cr` needed first (B20)
- [x] `-device help` lists `virtio-gpu-gl-pci`; `-display cocoa,gl=es` starts (ANGLE backend per virgl log)
- [x] `dev` booted: Hyprland 0.56 runs on `virtio_gpu`, renderer "virgl" (Mesa); screenshot via `grim` in the guest
- [x] Bonus: 9p share with `guest_owner_uid/gid` – Mac files writable in the guest (B18 solved with own build)
- [x] Hyprland "smooth": confirmed by manual check in the Cocoa window with `gl=es` (ANGLE) (2026-10-16); default later switched to `gl=on` because kitty does not start under ANGLE (B22)
- [x] Script ran completely without manual intervention (test build with `QEMU_BUILD_BASE`, ~2 min; revealed read-only bottle files → `chmod -R u+w`); test binary boots `minimal` via HVF
**Done when:** own QEMU build reproducible via script, signed, `dev` boots with `virtio-gpu-gl-pci` into Hyprland.

### M3 – CLI MVP (headless, QEMUBackend)
- [x] Package skeleton (Swift 6, swift-argument-parser 1.8), `swift test` green (12 tests), builds without warnings
- [x] `omaf flavors`, `build`, `images`, `new [--ephemeral]`, `start [--window] [--wait]`, `stop`, `reset`, `rm`, `list`, `ssh`
- [x] additionally: `omaf wait`, `log`, `trash`, `config`
- [x] Network modes `shared` / `isolated` (`restrict=on`) / `none`; 9p share; guest agent port
- [x] Optional Cocoa window (`--window`; not verified against the image)
- [x] Tests: `QEMUCommand`, names/socket length, ports, trash, image names, QMP parsing
- [x] On the real image (2026-10-16, `minimal`): build ✅ new ✅ start/wait ✅ ssh ✅ stop (powerdown) ✅ restart keeps data ✅ ephemeral clean ✅ isolated network ✅ share guest→Mac ✅ port conflict detected ✅ VM outlives CLI ✅ reset (marker gone) ✅ rm → trash ✅ – full cycle with image `minimal-d1f95739ba-55pvcxp5`
**Done when:** complete cycle `build → new → start → ssh → reset → start → rm` for `minimal`; ephemeral instance is provably clean after restart (marker file gone); app-independent: quitting the CLI leaves the VM running, `omaf list` finds it again.

**Decisions in M3**
- Ephemeral = QEMU `-snapshot` (replaces §1.3 item 3): nothing needs to be discarded/deleted – fits "never delete".
- Images = Nix GC roots under `~/.omaflake/images/<flavor>-<git-rev>-<store-hash>` instead of copies (saves 6–8 GB per revision; the store hash makes the name content-exact).
- No TOML parser in Swift: Nix reads `flavors.toml` (`.#lib.flavorDefaults`), the CLI receives JSON. Instances store `instance.json`.
- No dedicated VM key: `omaf ssh` uses the ssh-agent; `omaf wait` only checks the SSH banner and needs no key (B16 done).
- The guest user keeps UID 1000 – NixOS forbids `isNormalUser` with UID < 1000 (assertion); share set to 0777 instead (B18).

### M4 – App MVP (menu bar)
- [x] `Sources/OmaflakeApp`: SwiftPM executable + `scripts/bundle-app.sh` instead of an Xcode project (no hand-written `.pbxproj`), `LSUIElement`, no sandbox, ad-hoc signed
- [x] `MenuBarExtra(.window)`: instance list with status dot, start (window/headless), stop, terminal (SSH), show share, reset, switch to new image, remove
- [x] "New VM…" window (flavor, name, ephemeral, resources prefilled from `flavors.toml`)
- [x] "Build image" window with live log (`Shell.stream`), cancel
- [x] Status updates via polling (2 s, from `~/.omaflake`); detects VMs started via CLI. QMP events **not** implemented (polling is enough for now)
- [x] Optional: launch at login (`SMAppService.mainApp`, toggle in the gear menu; untested)
- [x] Also in CLI/OmaflakeCore: `omaf upgrade` (instance to current image), `SSHCommand` shared by CLI and app
- [x] Acceptance by clicking through (confirmed by manual check, 2026-10-16: "works"); follow-up guest issues fixed: shortcuts (B9/B21), kitty (B22), `start-hyprland`
**Done when:** everything from M3 works by clicking; quitting/restarting the app leaves running VMs untouched.

### M5 – Graphics backend in production
- [x] `QEMUBackend` uses the own build from M2 (path configurable, Homebrew QEMU as headless fallback) – 2026-10-17: Nix-built QEMU from config.json/Doctor, Homebrew QEMU 11.1.2 starts a NixOS VM headless (SSH, clipboard port) via `VM_QEMU`
- [x] `flavors.toml`: field `display = "none" | "window" | "gpu"` – 2026-10-17: copied into the instance; `omaf start` without `--window`/`--headless` and the app's start button follow it (none = headless, window = without 3D, gpu = 3D where QEMU can); NixOS flavors gpu, macOS window
**Done when:** `dev` starts by click with smooth Hyprland.

### M6 – Comfort
- [ ] Clipboard Mac↔VM (Wayland: e.g. `wl-clipboard` + small service over virtio-serial or SSH – **OPEN**, see try-omarchy)
- [x] Dynamic resolution (2026-10-16): starts at 75 % of the main screen, Retina scaling automatic, resizing/fullscreen → guest follows (B25; resizing and fullscreen confirmed by manual check)
- [x] Camera bridge for NixOS (2026-10-17, B37); macOS guests after M12
- [x] Image retention (2026-10-17, B52): current + one before + those in use; older ones deleted after a build

---

## 5. Open items (to carry into spec §8)

| Item | Status |
|---|---|
| M1 image path | **resolved** → `system.build.images.qemu-efi` (B3/B4) |
| Linux builder | **decided:** nix-darwin `nix.linux-builder` (see `docs/linux-builder.md`) |
| QEMU with GPU | **implemented:** `qemu/build.sh` (M2) |
| UEFI firmware | **resolved** (B7) |
| UTM sandbox + backing files | dropped (option A) |
| `-daemonize` with Cocoa | OPEN – `posix_spawn` + `setsid` for now |
| TOML library for Swift | dropped – `nix eval --json .#lib.flavorDefaults` (M3) |
| Clipboard / camera | NixOS done (B36, B37); macOS guests OPEN |
| Image retention | OPEN (M6) |
| SSH key for the CLI (B16) | **decided:** ssh-agent; `omaf wait` without key |
| Write access to Mac files in the 9p share (B18) | **resolved** with own QEMU (`qemu-9p-guest-owner`); with Homebrew QEMU still read-only |
| Hyprland Lua configuration (B19) | **resolved:** home-manager generates `hyprland.lua` (Lua API of Hyprland 0.56) |
| License of startergo patches | OPEN – repos without LICENSE, formulas state MIT |
| Minimum macOS version of the app | OPEN – developed on macOS 27; 15+ would be generous |
| macOS guests | **decided (2026-10-16):** in scope; flavors `mac-minimal/office/dev`; base prepared by hand (§7) |
| macOS runtime (BM10) | **decided (2026-10-16):** own runner `omaf-runner` on Virtualization.framework, no tart |
| macOS wallpaper (BM1) | OPEN – via script in M9 |
| macOS widgets (BM1) | **decided (2026-10-16):** off in all VMs (`WindowManager.StandardHideWidgets`/`StageManagerHideWidgets`, M9) |
| macOS guest agent (clipboard, commands) | OPEN – own small agent, after M11 (tart-guest-agent dropped) |
| Open source | **Goal:** everything must be publishable as open source; AGPL-3.0-or-later (recipes MIT, plan B112), repository `omaflake/omaflake` (still private); signing OPEN (spec §11) |

## 6. Risks (addition to spec §10)

| Risk | Mitigation |
|---|---|
| No builder → no image → everything blocked | M1 first, use an existing Linux VM as builder in the interim |
| QEMU build breaks on dependency updates | pin versions, update the build only deliberately |
| Own QEMU build gets outdated | build script in the repo, pin version, update only when needed |
| App and CLI start the same VM simultaneously | lock file per instance in `InstanceManager` |
| Image moved, overlay points to nothing | never move images with active instances; `omaf images` checks references |
| macOS base accidentally contains personal data (Apple ID, tokens) | "no sign-ins" rule in `docs/macos-base.md`; check before M8 acceptance |
| `VZBackend` and `QEMUBackend` drift apart | shared protocol + shared tests for `omaf` commands per backend |

---

## 7. macOS track

The macOS track was developed in parallel with the NixOS track and merged on 2026-10-16.
Its Nix parts (`darwin-guest.nix`, `mac-*` flavors) barely touch the NixOS track;
coordination is mainly needed for `flake.nix`, `flavors.toml` and `OmaflakeCore`.
**Guideline (2026-10-16):** no tart, no Cirrus Labs images – everything must be publishable as open source (BM10).

### 7.1 Layers

```
Apple IPSW ──omaf-runner install──▶ fresh VM ──(by hand, docs/macos-base.md)──▶ bases/mac-base-<os-build>/
mac-base ──build: clonefile, omaf-runner headless, nix copy, darwin-rebuild activate──▶ images/mac-<flavor>-<rev>/   (never started)
images/mac-<flavor>-<rev>/ ──clonefile + new machine-id + new MAC──▶ instances/<name>/vm/
```

### 7.2 Flavors (examples)

| Flavor | Content (draft) | CPU/RAM/disk (draft) |
|---|---|---|
| `mac-minimal` | base only + Nix/nix-darwin, terminal tools, tidy Dock | 4 / 8 GB / 60 GB |
| `mac-office` | Microsoft 365 (Word, Excel, PowerPoint, Outlook, Teams), browser; sign-in per instance | 4 / 8 GB / 80 GB |
| `mac-dev` | VS Code (extensions via home-manager OPEN), Git, Xcode CLT (approach OPEN), Docker replacement OPEN (direction: a Docker VM, B170) | 8 / 16 GB / 120 GB |

Cask names and disk sizes must be verified with `brew info --cask` during implementation.

### 7.3 Backend protocol (extension of §3)

```swift
protocol VMBackend: Sendable {
    var kind: GuestKind { get }                                  // .nixos / .macos
    func create(_ c: InstanceConfig, image: ImageRef) throws     // qcow2 overlay or clonefile + new machine-id
    func start(_ c: InstanceConfig, display: DisplayMode) throws // QEMU or omaf-runner (posix_spawn + setsid)
    func stop(_ c: InstanceConfig, timeout: TimeInterval) throws -> StopResult
    func status(_ c: InstanceConfig) -> VMStatus
    func reset(_ c: InstanceConfig) throws -> URL                // old state → _to_delete
    func remove(_ c: InstanceConfig) throws -> URL
    func sshArguments(_ c: InstanceConfig) -> [String]
}
```

`instance.json` gets a `kind` field (default `nixos` for existing instances); macOS has no SSH port forward (IP from DHCP leases).

### 7.4 Runner `omaf-runner`

Separate executable in the Swift package (`Sources/omaf-runner`), uses `OmaflakeCore` for paths.
One process per VM; window via AppKit (`NSApplication` + `VZVirtualMachineView`), headless without a window.
Controlled via `runner.sock` (line-delimited JSON like QMP: `status`, `stop`, `kill`, `show`).
Reference: Apple's sample code "Running macOS in a virtual machine on Apple silicon". Do not adopt tart code (FSL);
Lume (trycua) only after a license review (OPEN).

### M7 – Runner `omaf-runner`
- [x] Executable `omaf-runner`, ad-hoc signed with `com.apple.security.virtualization` (`scripts/sign-runner.sh`); ad-hoc is enough (BM12)
- [x] `omaf-runner install`: fetch IPSW (`latestSupported` or file, stored in `~/.omaflake/ipsw/`), create RAW disk, `VZMacOSInstaller` with progress (BM14)
- [x] `omaf-runner run`: window and headless; NAT with fixed MAC; virtiofs share (share not yet verified in the guest)
- [x] Guest provisioning on first start (BM11/BM15): guest user, auto-login, SSH – `scripts/macos-first-boot.sh` prompts for the password
- [x] Socket control `status/stop/kill/show`; `show` also brings headless-started VMs to the front. ⚠️ headless `stop` hangs on the confirmation dialog (BM17). Detaching via `setsid` follows with `VZBackend` (M10)
- [x] IP lookup from `/var/db/dhcpd_leases` (format verified, leading zeros per octet are missing; tests)
- [x] Limit of 2 macOS VMs: start a third, record the error message (BM16)
**Done when:** fresh macOS installed from IPSW, boots with a window up to the Setup Assistant, starts headless after setup and is reachable via SSH; `stop` shuts down cleanly.
**Status 2026-10-16:** met except headless `stop` with a logged-in user (BM17) – solution via SSH shutdown in M9/M10. The Setup Assistant is skipped thanks to provisioning.

### M8 – macOS base (manual)
- [x] Work through the checklist in `docs/macos-base.md` (Setup Assistant, guest user, language/region, dialogs off, Remote Login, SSH key) – 2026-10-16: development base `mac-base-26A434` (user `test`); checklist complete, no dialogs to dismiss, base read-only
- [x] DECISION: Nix already in the base, or only in the build script – 2026-10-16: **base stays clean, without Nix**; the M9 build script installs Nix into each image
- [x] Move the folder to `~/.omaflake/bases/mac-base-<os-build>/`, read-only – `mac-base-26A434`
**Done when:** base starts, is reachable via SSH with key as the guest user, checklist complete, no sign-ins included.

### M9 – macOS image via script
- [x] `darwin-guest.nix` module, `mac-minimal` flavor, `darwinConfigurations.mac-minimal` (`flake.nix` filters flavors by `kind`); `nix flake check` green – 2026-10-16: `flavors/modules/darwin-guest.nix`, `flavors/mac-minimal.nix`; the system closure builds on the Mac (`nix build "path:$PWD#darwinConfigurations.mac-minimal.system"`), activated in a guest by the build script (BM20). The module also sets: key-only SSH, `NOPASSWD` for `/sbin/shutdown` (BM17), no sleep, no automatic macOS updates
- [x] macOS build script `<flavor>`: clone base → `omaf-runner run` headless → wait for IP → Nix (if missing) → build closure on the Mac → `nix copy --to ssh://…` → activate → shut down → move to `images/mac-<flavor>-<rev>/` – `scripts/macos-build-image.sh` (BM18–BM20); image `mac-minimal-<rev>-<hash>`, `current/mac-minimal`
- [x] Widgets off (`StandardHideWidgets`, `StageManagerHideWidgets`) in `darwin-guest.nix` – checked in a clone with `defaults read com.apple.WindowManager` (both 1); visual check OPEN
- [x] Check wallpaper (approach from BM1) and Dock; screenshot in a clone – 2026-10-16: black wallpaper, dark/graphite/clear look, Dock (Finder, Apps, Safari, Mail, Terminal, System Settings | Trash; size 50), Finder opens Downloads, menu bar items hidden, no desktop items or widgets, Safari private with google.com and no favorites, language and keyboard from `local/settings.nix` (BM22–BM26); checked by the maintainer in the window. Earlier: Dock checked via `defaults read com.apple.dock persistent-apps` (Terminal, System Settings; no recents); wallpaper not declared yet; screenshot OPEN (BM21). Also checked in the clone: host name, no sleep, no automatic macOS updates, key-only SSH, temporary sudo rule removed, tools and Nix present
- [ ] then `mac-office`, `mac-dev`
**Done when:** the build script runs `mac-minimal` without manual intervention; a clone shows Dock, apps, defaults and wallpaper as declared.

### M10 – `VZBackend` in OmaflakeCore + CLI
- [x] Extract the `VMBackend` protocol from `QEMURuntime`/`InstanceStore` (no behavior change, `swift test` green) – 2026-10-16: `Sources/OmaflakeCore/Backend.swift` (`VMBackend`, `QEMUBackend`, `Backends`); CLI and app use only the protocol; `kind` in `FlavorDefaults`/`instance.json` (default `nixos`); macOS flavors are rejected with a clear message until M9/M10. Checked: full cycle `new → start --wait → ssh → stop → reset → rm` on `minimal`
- [x] `VZBackend`: create (clonefile + new machine-id/MAC) / start / stop / status / reset / remove / ssh via `omaf-runner` – 2026-10-16: `Sources/OmaflakeCore/VZBackend.swift`; runner detached via `setsid` (shared `spawnDetached`), status from `runner.pid` (process path checked), stop via SSH shutdown (BM17), SSH to the NAT address; `Backends` no longer needs QEMU for macOS guests (and vice versa); images of both kinds in `ImageStore`. Checked on `mac-minimal` (BM27)
- [x] `omaf macos install`; `omaf build` runs the M9 pipeline for `kind = macos` – 2026-10-16: `Sources/omaf/MacOS.swift`, `Sources/OmaflakeCore/MacImageBuilder.swift`; scripts `macos-build-image.sh`/`macos-first-boot.sh` replaced (moved to `_to_delete/`); host name per instance (BM28)
- [x] Clarify ephemeral mode and network mode `isolated` (spec §6.3) – 2026-10-16: ephemeral = fresh clone per start; `isolated` refused (BM27)
**Done when:** full cycle `build → new → start → ssh → reset → start → rm` for `mac-minimal`; ephemeral provably clean; `omaf list` shows both guest kinds.

### M11 – App for both guest kinds
- [x] Instance list with guest-kind icon; "New VM…" grouped by guest kind – 2026-10-16: snowflake/Apple logo, NAT address of running macOS guests, flavor picker with sections; macOS: disk "as the image", no `isolated`
- [x] Start check "at most 2 macOS VMs" – 2026-10-16: start buttons disabled with the reason before a third macOS VM; "New VM" creates without starting then
- [x] App launches `omaf-runner` (embedded in the bundle, own entitlement) – 2026-10-16: `scripts/bundle-app.sh` embeds and signs it inside out (`codesign --verify --strict` passes)
- [x] Beyond the list: macOS image build (password once) and "Install macOS Base …" (`MacBaseInstaller`, shared with `omaf macos install`); host name after start (BM28)
- [ ] Acceptance by click (maintainer)
**Done when:** everything from M10 works by clicking.

### M12 – Flavor interface (open-source preparation)
- [ ] Flake output `vmFlavors` (spec §11.1); CLI/app read only this
- [ ] No user-specific names/paths in the Swift sources and the app
- [ ] Dependencies cleaned up per spec §11.2 (e.g. build ANGLE ourselves instead of the startergo bottle)
**Done when:** a second test flake with its own flavor works without touching this repo.

---

## 8. Next steps (as of 2026-11-27)

**Release 0.8.3 (2026-11-27):** B198 (modules can need files nobody may pass on: module.toml v3 `[[files]]`, found and
added to the Nix Store before a build, the build page and `omaf files add` asking for them – the catalog's `xcode`
1.1.0 uses it), B189's double start of the volumes session fixed, B186 written down as a proposal. Gone through and
left, with reasons: B193 (forwards in a macOS VM refused, most likely the local network rule for the agent – the agent
inside omaflake Tools is a change of its own, to be tried with a Docker VM), B197's cause (seen once; 0.8.2's helper
process works around it), what waits for the Developer ID (M12, B191) or for trying on the Mac (B164, B178–B182, B183,
B185).

**Release 0.8.2 (2026-11-27):** fixes for 0.8.1 found the same evening – B194 (the guest helpers start without a
client watching, and an early start is not logged as a failure), B195 (pages taller than the window scroll), B197 (SSH
waits also ask a fresh process) – and B196 (the app icons in Liquid Glass). Checked with the development app before
the tag: B194 and an image build; B195 and B196 looked at by the maintainer ("Beides passt"). OPEN: B197's cause.

**Release 0.8.1 (2026-11-27, published: tag v0.8.1, the unsigned app on GitHub and omaflake.app's update.json,
every crate on crates.io – B192):** B183 (the camera with OBS:
Metal, the agent decoding H.264, the agent's log kept) and B184 (omaflake Tools and omaflake's own camera
extension; unchained copies of images and bases, Recovery without hands, `omaf start --recovery`). Maintainer:
"mache ein Release 0.8.1 fertig, dass ich erstmal lokal testen kann". OPEN until tried: the whole unchain job in the
app, the quiet window's picture, what `csrutil disable` asks word for word, the extension's approval without a
click (a probe). Before the tag, still in 0.8.1 (2026-11-27): B189 (volumes in macOS VMs mounted again, upgrade in place
for macOS, `omaf upgrade --keep` reporting failures), BM25 (no "sh" notice), B190 (volumes under Locations in unchained
VMs through FSKit); B191 proposed.

**Release 0.8.0 (2026-11-25):** everything of the maintainer's list that needs no Developer ID ("Alles heißt wirklich
alles, was keine Developer-ID braucht"): B167 (chrono → jiff), B177 (the dependencies looked through), B164 (a larger
disk), B178 (upgrade in place), B144 (the guided first start, modules by author), B179 (names over mDNS, joining while
running), B180 (ports of other VMs on localhost), B181 (the graph edited by dragging), B182 (the builder checked
daily, its disk's size), B176 (the hung tests). *Left, with reasons:* rules per network and `isolated` for macOS VMs –
a gateway on the switch needs a userspace NAT (gvproxy as a port: configurable subnet by its config file, but a second
DHCP server on the segment, SSH from the Mac to reach, a Go binary to ship – its own piece of work, to be tried with
real VMs); the camera over vsock – H.264 (B174) made the camera smooth at 30 fps, so vsock waits for the measurement on
the Mac (VsockBench) to show it is needed; dragging in the Linux app and `omaf ui`. To try on the Mac: everything
marked OPEN in B164, B178–B182 (they need images built with 0.8.0).

**B186 (2026-11-27, proposed – not decided, not built; parts since built elsewhere: item 3 as volumes, B187, at
`/Volumes/<name>` – not `omaf shares` –, and from item 1 the catalog's modules `omaflake/docker-client` (NixOS, macOS)
with `omaflake/docker` 1.1.0; SSH trust and tunnels remain proposals): several VMs working against one Docker VM.** The
maintainer's goal: a macOS VM for Claude and Xcode, a NixOS VM for Neovim and other agent harnesses, both using one
Docker VM (`nixos/docker`, B170), all in one omaflake network. Not the goal: one share folder for everything.
Wished: several shares per VM, and SSH and tunnels controlled by omaflake.
*Tried (2026-11-27, a macOS VM and a Docker VM in one network; a small Compose stack of a web service, PostgreSQL and
an S3 store):* the network works – 0.5 ms, the Docker VM's address reached, `DOCKER_HOST=tcp://<address>:2375`
with docker-client and docker-compose from nixpkgs (29.8.1 on both sides, buildx 0.35 included), pulls and Compose
builds fine, a published port reached at the Docker VM's address. Friction: (1) bind mounts name paths of the client
(`./init.sh`), which the daemon looks up in the Docker VM – every such Compose file needed an override that bakes the
files into images; (2) Compose files bind ports to `127.0.0.1`, which is the Docker VM's loopback – unreachable from
the client without an override; (3) no SSH from the client to the Docker VM (its key is not in `authorized_keys`);
(4) the client's Docker CLI and context set up by hand. The VM had no B180 forward configured; names over mDNS
(B179) not checked: OPEN.
*Proposal, in order of effect:*
1. **SSH trust inside a network.** Each VM gets a key pair of its own (made at first start, by the agent); joining a
   network puts the members' public keys into each other's `authorized_keys` (a marked block omaflake owns) and their
   host keys into `known_hosts` under `<vm>.<network>.local` – `ssh docker.<network>.local` works without a question,
   leaving removes the lines. Then `DOCKER_HOST=ssh://docker.<network>.local`: TCP 2375 without TLS or login can become
   opt-in in `omaflake/docker` instead of the default, and which members may use Docker becomes a per-member right
   (today every member is root on the Docker VM). A module `omaflake/docker-client` (both platforms): Docker CLI,
   Compose, buildx and a Docker context pointing at the network's Docker VM.
2. **Tunnels controlled by omaflake, container ports forwarded by themselves.** B180's forwards are static. The
   Docker VM's agent follows Docker's events (containers started or stopped with published ports, also those bound to
   127.0.0.1 – the agent listens on its network address and passes on to its loopback) and reports them; omaflake
   forwards them to the localhost of the member that started the container (as dev containers forward ports), so
   Compose files stay unchanged. Tunnels as objects in the network's picture and list: from, to, port, on/off – also
   Mac → VM (spec §11.9's "tunnels from the Mac"). Over the agent, not `ssh -L`: same on both platforms, no keys
   needed; SSH stays for shells and `DOCKER_HOST`. OPEN: the same port published by two members; Compose project
   names colliding when two members start the same repository.
3. **Shares as objects of their own, several per VM.** `omaf shares create <name> <folder>`, attached to any VMs like
   networks; macOS guests through `VZMultipleDirectoryShare` (each share a folder under `/Volumes/My Shared
   Files/<name>`), NixOS guests one 9p `mount_tag` each. A Docker VM mounts the shares of its network's members
   **at the paths the members see them** (for NixOS members a link where the paths differ), so `docker run -v
   "$PWD":/app` and relative Compose mounts work from every member without overrides. To document: changes made in
   one VM reach another without inotify events (watchers in containers need polling), and 9p/virtiofs are slow with
   many small files – `node_modules`, `target/` and databases belong in Docker volumes. OPEN: virtiofs and 9p
   caching when two VMs write the same host folder.

**B193 (2026-11-27, found while testing the catalog's Docker modules; OPEN): ports of other VMs in a macOS VM.** The
catalog's `nixos/docker` 1.1.0 and `docker-client` (both platforms) checked on the Mac with a local source: a Docker
VM, a NixOS and a macOS client in one network (`dockertest`), the forward `2375=<docker VM>`, one volume in all three.
NixOS client: everything works – `docker` through `127.0.0.1:2375`, a container's bind mount of `/Volumes/<name>`
reaching the host, a container with `restart: always` up after the Docker VM's reboot and seeing the volume's files
(the engine started after omaflake-volumes.service). macOS client: the shell has `DOCKER_HOST`, and the Docker VM is
reachable at its network address (`curl` over SSH), **but the forward fails**: the agent's connections to
`10.231.2.10:2375` end in "No route to host" (EHOSTUNREACH), the client sees "connection reset by peer". At the VM's
start that is right (the second card has no address yet), but it stays so: a restarted agent connected once and then
failed again, while `curl` from the SSH session kept reaching the address. Most likely macOS's privacy rule for the
local network refuses the agent (a program without an app's identity, started from SSH and then detached) – OPEN:
the log does not say so in plain words. Ideas: the agent inside omaflake Tools (an app identity, the usage text for
the local network) or an MDM profile (B191); retrying a failed connection in any case. Also OPEN: `DOCKER_HOST` for
apps started from the Finder in a macOS VM (nix-darwin's `launchctl setenv` at activation is gone after the login;
a launch agent would bring BM25's notice).

**B194 (2026-11-27, found by the maintainer – the camera in a macOS VM stopped working after a restart): the guest
helpers come without a client watching.** macOS guests get their helpers – the omaflake agent (camera, MIDI, the
diagnosis), the clipboard helper and the volumes session – over SSH from the host (BM25). After a restart in the guest
none came: `omaf start` without `--wait` does not start them, a start through the API tries once (`wait_for_ssh`) and
does not say when that fails, and the retry (`ensure_clipboard_agents`) ran only in the watcher, which ends when no
client subscribed (menu closed, no `omaf ui`). The runner's log showed no `hello` after the restart; the guest's agent
log no start at all (`~/Library/Logs/omaflake`), and `/Volumes/<name>` came only with the next start over SSH.
*Fixed:* the core service looks every 5 s on its own (`Server::keep_guest_helpers`, called from the service's accept
loop) – cheap: the instances' config files and their runners' pid files, no QEMU lookup, nothing for a runner that got
its helpers; `omaf start` of a macOS VM with a network makes sure the core service runs. Tests: the predicate and a
look without VMs; checked on the Mac with the development app (see B197); a service started with
`--idle-exit` (no launch agent installed) still ends after its idle time – a later restart in the guest then waits for
the next client.

**B195 (2026-11-27, found by the maintainer): pages taller than the window.** The diagnosis (nine checks with hints and
the log), a build with notes and the macOS base page were plain stacks: taller than the window, they grew past its top
– the page header with the way back out of reach, no scrolling. They are now in `PageScroll` (Components.swift):
`ViewThatFits` – the page as it is while it fits (its log fills the room left), scrolled as a whole otherwise (the log
at its minimum height, 160 – the build page's log has one now too). The other pages are forms and scroll themselves.
`swift build` and `swift test` green with Xcode 27.2 (`CoreClientTests.coreIdentityComparison` compared with
`/usr/local/bin/omaf`, which links to the app where it is installed – now a path that exists nowhere). The look
checked by the maintainer in the development app before 0.8.2 ("Beides passt").

**B196 (2026-11-27): the app icons in Liquid Glass.** macOS 26+ draws app icons from layers (Icon Composer's `.icon`
document, compiled into the asset catalog); a flat `.icns` only shows as a picture inside the system's tile.
*Checked on the Mac (macOS 27.2.1, Xcode 27.2 – it is installed there, with Icon Composer 27.2):* `actool` takes a
`.icon` folder as input (`folder.iconcomposer.icon` in Xcode's AssetCatalogCompiler.xcspec; `man actool`: `--app-icon`,
`--standalone-icon-behavior none`, `--output-partial-info-plist`) and writes `Assets.car` (an `IconImageStack` each for
`NSAppearanceNameAqua`, `NSAppearanceNameDarkAqua` and `ISAppearanceTintable`, flat renditions 16–1024 px) and the
partial Info.plist with `CFBundleIconName` (and `CFBundleIconFile`) – the same pair Apple's Icon Composer.app carries.
With the Command Line Tools alone there is no `actool` (`xcrun: unable to find utility "actool"`). Icon Composer's
`Contents/Executables/ictool --export-image --rendition Default|Dark|TintedLight|TintedDark|ClearLight|ClearDark`
renders a document without the app: every rendition looked at.
*Built:* omaflake/brand's `generate.py` writes `macos/omaflake.icon` (keys from Xcode's template and
IconComposerFoundation): `Assets/` – `flake.svg` (the hero's white core, `glass`), `split-red.svg`/`split-cyan.svg`,
`noise.svg`, each the whole canvas, transparent; `icon.json` – the background `#121212` as a fill, pure black for
`dark` (`fill-specializations`), the split and the noise hidden for `tinted` (`hidden-specializations`); `groups[0]`
is drawn on top (checked with ictool). `copy.py` copies it to `brand/macos/omaflake.icon`. `scripts/compile-icon.sh`
compiles it into a bundle's `Assets.car` and sets `CFBundleIconName` from actool's partial plist; the bundle keeps
omaflake/brand's `omaflake.icns` (`CFBundleIconFile`, small glyph at 16/32 px). Without actool: a warning, the
`.icns` alone. Called by `bundle-app.sh` (omaflake.app and the helper `omaf-runner.app`, which now also gets
`omaflake.icns`) and `bundle-tools.sh` (omaflake Tools). omaf-runner no longer sets `applicationIconImage` when its
bundle names an icon (`CFBundleIconName`) – it would cover the glass one; as a bare binary it still does.
"Older systems": every bundle needs macOS 27 (`LSMinimumSystemVersion`), so the `.icns` is a fallback for tools that
read only `CFBundleIconFile`. A test bundle with the compiled catalog showed the glass icon via
`NSWorkspace.icon(forFile:)` (the Mac set to ClearDark). The whole `bundle-app.sh` run (ad hoc, 2026-11-27): all four
bundles (omaflake.app, its omaf-runner.app, both omaflake Tools) carry `Assets.car`, `CFBundleIconName` and
`CFBundleIconFile`, `codesign --verify --deep --strict` passes, and `NSWorkspace` shows the glass icon for the app, the
runner and omaflake Tools. OPEN: the Dock icon of omaflake VM (the runner) while a VM runs, and of omaflake Tools in a
guest, looked at; the icon at 16/32 px
in the catalog's flat renditions (actool renders those from the `.icon`, not the small glyph); the look itself – a
proposal for the maintainer (glass on the white flake, 0.4 translucency, pure black when dark).

**B197 (2026-11-27, found while building the catalog's new `macos/xcode`; OPEN): a core service that no longer reaches
its build VM.** Four macOS image builds in a row ended with "No SSH after 10 minutes (address …, port 22 closed)" –
from the same unchained base that had built an image at 19:28. The build VM was fine: started by hand it reached the
desktop at its first start, and from a shell on the Mac port 22 was open with `SSH-2.0-OpenSSH_10.5`, while the build
in the core service (`omaf serve`, running since 18:33) still waited – `sample` showed it in `provision`'s loop, `lsof`
no TCP socket of the process. Not the other running VM (stopped: same result), not memory (64 GB, 77 % free), no
Local Network denial in the Mac's log. After `omaf service stop` the next service built the image at once. OPEN: why the
long-running service's connections to the VMs' network (192.168.64.0/24 on bridge100) failed – the diagnosis through
the same service had reached the test VM at 19:46; a build could say "reached from this computer but not from the
service" by asking `omaf-runner ip` and probing from a fresh process before giving up.
*Worked around (for 0.8.2):* every wait for a macOS guest's SSH – the image build, the base install and unchain, a
start's `wait_for_ssh` and the service's guest helpers (B194) – asks `Ports::ssh_banner_any`: this process first, then a
fresh `/usr/bin/nc` (macOS only). When only the helper gets the greeting the build goes on – its SSH and `nix copy`
are processes of their own – and its log says so. Test: the helper reads a greeting from a local listener. OPEN: the
cause; whether the helper gets through when the service does not (seen only from a shell, not yet from the service).
*Checked with the development app (`~/.omaflake-dev`, 2026-11-27):* an image built at the first try; B194's VM started
with `omaf start` (no `--wait`, no app) said hello after 37 s, after a restart after 26 s, the diagnosis all green with
the camera ready. Found there: every start logged "the guest helpers did not start" – the service's first try came
before nix-darwin had made `/run/current-system` in the guest. The helpers' command now exits 75 then
(`GUEST_NOT_READY`), which is tried again without a log line.
Also seen: with `omaf config --flake <checkout>/nix` the image flake reads the developer's `nix/local/settings.nix`,
whose `macUser` named the user of an older development base – activation stopped ("primary user … does not exist").
Building with the app's image flake and the local catalog as a source avoids it.

**B198 (2026-11-27, the maintainer: "module können Dateien beim Bauen verlangen, die entweder automatisch aus einem
Verzeichnis geladen werden … und falls nicht gefunden, muss manuell die Datei oder der Ordner ausgewählt werden"):
files a module needs.** The catalog's `macos/xcode` (Xcode 27.2 through `requireFile`) needed the user to run
`nix-store --add-fixed` by hand, and omaflake only passed on Nix's error. *Built:* module.toml v3 (`[[files]]`: name and
hash as the module's `requireFile`, `mode` recursive or flat, `title`, `from` – the names it comes as –, `search`,
`url`; v2 stays readable, only modules with files use v3), `needed_files.rs`: the fixed-output store path computed from
the name and the hash (Nix's `makeFixedOutputPath`, checked against the path Nix gave Xcode and one of a flat file – no
Nix call to know whether it is there), found in `search`, added from a `.xip` (`xip --expand`), `.zip`, folder or app
(cloned to the module's name when it has another), `nix-store --add-fixed`, and refused when it lands elsewhere (taken
out of the store again). `images.build` adds what it finds at the job's start and refuses with what to do when a file
is nowhere; `recipes` parts carry `files`; the build page shows each missing one with "Choose …" (`files.add`, the
ask kind `path`: a file or a folder, an app as one) and "Download …"; `omaf files` / `omaf files add <module> [<path>]`.
*Checked (2026-11-27, a data folder of its own):* `omaf files` saw Xcode in the store; `files add` with a wrong
Xcode.app refused it and left the store as it was; with `~/Downloads/Xcode_27.2.xip` unpacked, added and confirmed in
1:44; with a module whose hash no file has, `images.build` refused with the download and `omaf files add
omaflake/xcode <file>`, and the build page showed the warning with "Choose …" (ask `path`) and "Download …". The
catalog's `xcode` 1.1.0 uses it (search: Downloads and Desktop – not /Applications, where another Xcode may be).
OPEN: the app's panel looked at; a build that adds a found file on its way (the same `files.add` code, not run inside
a build yet).

**B199 (2026-11-27, found by the maintainer – "Ich kann irgendwie nicht die Platte unchained entfernen"): a removed
volume came back.** A volume is also a folder in `<data>/share/` (B187), so removing one deletes its default folder when
it holds only macOS's metadata (`SHARE_METADATA`). The folder of `unchained` held `.Trashes` and `.fseventsd` only, but
the Finder of an unchained VM (FSKit, B190) had made `.Trashes` as `d-wx--x--t` – no read permission, also not for its
owner – so the deletion failed without a word, the folder stayed and the list showed the volume again ("removed" each
time). *Fixed:* `remove_share_if_empty` gives the owner permission on the metadata first (`open_up_for_owner`); when
the folder still stays, `volumes.remove` says so instead of reporting success, and `omaf volumes rm` says whether the
folder stayed. Test: a share with a `.Trashes` of mode 1311 holding a file goes. The maintainer's `unchained` removed
with it.

**B192 (2026-11-27, done with 0.8.1): omaflake's crates on crates.io.** The maintainer: falkOSS's other projects
(kernoid, aktujo, verkodo) have their crates on crates.io, omaflake was the odd one out. *Decided:* every crate is
published, in step with omaflake's version (like aktujo: the crate with the project's name is the program, the rest
`omaflake-…`):
- `omaflake` – the program `omaf` (CLI and `omaf serve`; the package was `omaf`, the program keeps its name);
- `omaflake-core` – the engine (VM control, images, stores, the server), internal – no promise of a stable API;
  `omaflake-brand` (the design tokens, 289 generated lines without dependencies) becomes the module `brand` of
  `omaflake-sdk` (below);
- `omaflake-sdk` – what every interface and every tool of others uses to talk to omaflake: the API's types, page
  models (views), protocol and the client of the service's socket (docs/api.md) – out of `omaflake-core`, which
  depends on it; it follows `apiVersion` and the golden files. Named as in aktujo, where `-sdk` is the layer every
  interface goes through (`aktujo-api` is its HTTP server there); unlike aktujo's it is a client – it needs a running
  service (the app or `omaf service`);
- `omaflake-tui` (`omaf ui`), `omaflake-gui` (the Linux app), `omaflake-i18n`;
- `omaflake-macros` – stays a crate of its own: `vm_err!` is a procedural macro;
- `omaflake-agent` – the guest agent (its own workspace, `nix/agent`): published for transparency, the code in every
  image readable per version.
All names were free (2026-11-27). *Way:* the maintainer prepares crates.io and GitHub; placeholders 0.0.0 published
once by hand with the maintainer's token (Trusted Publishing needs a crate to exist – OPEN whether crates.io takes pending
publishers), then Trusted Publishing from a CI job (as aktujo's `crates` job), 0.8.1 the first real release through it;
the placeholders yanked. OPEN: `omaflake-gui` with `cargo install` (Wayland/X11 libraries).
*Built:* the package `omaf` → `omaflake` (folder `crates/omaflake`, the program stays `omaf`); `omaflake-sdk` with
everything an interface needs – `client` (its errors carry texts like the core's), `protocol` (`RpcError`), `codes`,
`time` (dates), `text` (`Text`, `Arg`), `types` (all of `api::types`), `models` (the kinds they use: `GuestKind`,
`VmStatus`, `NetworkMode`, `DisplayMode`, `StopResult`, `PrerequisiteItem/Status`, `RecipeSourceKind`,
`Requirements`, `LocaleRules`, `Binding`, `Issue`, `ModuleMode`, `Swatches`), `networks` (`Network` and its
switch's status), `diagnostics` (the checks), `views` (every page model with its building blocks – split from the
core's builders, which stay in `api/views` and build these types; `Blockers::of` stayed as `blockers_of`: its hints
are the Doctor's) and `brand` (the design tokens, the crate omaflake-brand before – the core uses none, the
interfaces all; brand/tokens.py writes there). No re-exports: every crate names `omaflake_sdk::…` where it uses the
SDK, so each file shows what is the API's and what the core's own (before 1.0 nothing needs the old paths, B141).
`omaf ui` and the Linux app take everything from the SDK and the core only for its in-process server (when no
service runs). Images the crates embed are copies in their
`assets/` (`omaflake-core`: Dock icon, loading picture; `omaflake-gui`: icons, glyph) – omaflake/brand's `copy.py`
writes them. Every crate has its README, keywords and categories; `cargo package --workspace` and the agent's
package build (checked). CI job `crates` (ci.yml, on a tag `v*`, environment `crates-io`, after linux, dependencies,
licences): checks the tag is on main and names the version, publishes in dependency order (macros, i18n, sdk, core,
tui, omaflake, gui with `--no-verify`, agent), skipping what is there. Placeholders 0.0.0 of all eight published by
hand (2026-11-27), then 0.8.1 of all eight by the `crates` job of the tag v0.8.1 (Trusted Publishing, approved by
the maintainer) and the placeholders yanked; the token is revoked.

**B191 (2026-11-27, proposed for after the Developer ID (M12) – not decided, not built): an MDM of omaflake's own
for macOS guests.** Why: managed login items need a user-approved MDM (a profile installed by hand is refused, BM25,
B190), and an MDM would take away several approvals at once. *Idea:* omaflake runs a small MDM server on the host
(e.g. NanoMDM or MicroMDM); each macOS VM enrolls once (an enrollment profile, approved by the user – user-approved
MDM) and then gets profiles a hand-installed one may not carry: managed login items (no background item notices,
BM25 – own launch agents without that cost), privacy preferences (PPPC – e.g. network volumes, B189's favorites;
Accessibility, Screen Recording), a system extension policy (the camera extension without its click, B184 – with a
team id), managed preferences where nix-darwin has no option (BM22, BM23). *Not solved by it:* the restricted
entitlements (B190's FSKit module) – they need the signature. *Hurdles:* Apple's MDM wakes devices only through APNs,
so the server needs an MDM push certificate – a CSR signed by an MDM vendor (officially the Apple Developer Enterprise
Program) and uploaded with an Apple ID, renewed yearly; for open source every user would need their own (OPEN: a way
without the Enterprise Program); whether an enrolled base carries its enrollment to clones with their new machine
identifiers (BM23) or every clone enrolls and is approved anew (OPEN); a server of its own (certificates, SCEP or
identities, the command queue), the enrollment in image builds, renewals; "this Mac is managed" shown in every guest,
and the server's rights over all VMs. *To decide with M12:* whether it is worth it, and as an option for users who
want it.

**B190 (2026-11-27, built for 0.8.1 in unchained images; checked on the Mac – see *Built*): volumes
under Locations through FSKit.** The
maintainer: each volume should show in the Finder like an external disk, as "My Shared Files" did (B189 made them
favorites for now); the extension may need a Developer ID – "dann ist es erstmal nur für die unchained Images nutzbar".
*What FSKit offers (macOS 27.2 SDK):* `FSPathURLResource` (FSKit V2) – a file system over a folder, no block device;
Info.plist `EXAppExtensionAttributes` keys read from Apple's own modules in a guest (`com.apple.fskit.ftp`:
`FSSupportsPathURLs`, `FSRequiresSecurityScopedPathURLResources`, `FSSupportsGenericURLResources`,
`FSActivateOptionSyntax`); `FSClient.mountSingleVolume` (V3, entitlement `com.apple.developer.fskit.mount`) mounts in
/Volumes; `mount -F -t <FSShortName> <folder> <mount point>` from the shell.
*Prototype (outside the repository, like B184's camera):* a passthrough module in Swift (`FSUnaryFileSystem`,
`FSVolume.Operations` and `.ReadWriteOperations` – deprecated in macOS 27 in favour of the `…Handler` protocols, they
still work), in an app's `Contents/Extensions/*.appex`, sandboxed with a read-write exception for `/Volumes/`, signed
ad hoc. Checked in macOS 27 guests:
- *Enabled:* FSKit runs only modules in `~/Library/Group Containers/group.com.apple.fskit.settings/enabledModules.plist`
  (what the switch in System Settings writes; "Module … is disabled!" otherwise); `fskit_agent` reads it at its start
  (`launchctl kickstart -k user/<uid>/com.apple.fskit.fskit_agent`).
- *Signature:* ad hoc, launchd refuses to start the extension – "Security policy issue" (163) – with System Integrity
  Protection on **and** off: the entitlement `com.apple.developer.fskit.fsmodule` is restricted. It starts with
  `amfi_get_out_of_my_way=1` in the boot arguments (an unchained VM, set by hand). So: a Developer ID with a
  provisioning profile (M12), until then only unchained VMs with that boot argument.
- *Start:* not as a Swift `@main AppExtension` (it trapped in `_EXRunningExtension._shared`), and not with an own
  `main` calling `NSExtensionMain` (endless recursion) – like Apple's modules: entry point `_NSExtensionMain` (linker
  `-e`), an Objective-C principal class (`EXExtensionPrincipalClass`).
- *Result:* `mount -F -t omaflakefs /Volumes/plattentest ~/m` → `omaflakefs, local, …, fskit, mounted by <user>`;
  `NSURLVolumeIsLocalKey` true (the virtio-fs mount below it: false), and **the Finder lists it under Locations like
  an external disk** (seen by the maintainer). Reading, creating, appending, truncating, mode, times, renaming a
  folder with its contents, symlinks and removing reach the host's folder. Reading a fresh 256 MB file: 1.1 GB/s
  through FSKit, 3.6 GB/s through virtio-fs (both partly cached – an indication only).
- *Side effects:* macOS makes `.fseventsd` (and the Finder `.Trashes` when it deletes) on the volume (it lands in the host's folder; `no_log` – OPEN); files the
  sandboxed extension creates carry `com.apple.quarantine` on the host (to take off in the module – OPEN).
*Built:* the module `Sources/omaflake-volumes/` (`VolumeFileSystem`, the prototype's passthrough; files it makes
lose `com.apple.quarantine`), `App/omaflake-volumes-Info.plist` and `.entitlements` (sandbox exception for
`/private/var/omaflake/volumes/`); `scripts/bundle-tools.sh` compiles it with swiftc and puts it into the unchained
omaflake Tools (`Contents/Extensions/omaflake-volumes.appex`) – the same source as a SwiftPM executable trapped in
ExtensionFoundation at its start (OPEN why; Package.swift keeps it as a library target, compiled only). An unchained
image build sets `amfi_get_out_of_my_way=1` in the build VM's NVRAM, which its clones copy (checked), an upgrade in
place to an unchained image does so for its next start. In the guest (darwin-guest.nix) `omaflake-volumes` mounts the
volumes hidden at `/private/var/omaflake/volumes/<tag>` when the module is there and AMFI lets it start, and makes
`/Volumes/<tag>` for the user; `omaflake-volumes-session` (started over SSH with the guest helpers – a launch agent
would bring a notice at every start, BM25) registers the module, mounts each as `omaflakefs` at `/Volumes/<tag>` and
switches the module on when FSKit says it is off (it makes `enabledModules.plist` at its first use in a fresh VM).
Normal images stay at virtio-fs and favorites (B189). Checked on the Mac: a fresh VM of an unchained image with two
volumes mounts both as `omaflakefs` at `/Volumes/<name>` by itself, writes reach the host's folder, and the Finder
lists both under Locations (seen by the maintainer). OPEN: files written through the volume still carry
`com.apple.quarantine` on the host (taking it off in the module did not hold – most likely the sandbox); hard links, xattrs, the `…Handler` protocols, many files
at once, `.fseventsd` on the volume. The Finder copies a file onto the volume and deletes it again (checked by the
maintainer).

**B189 (2026-11-27, fixed before the tag, in 0.8.1): no VM saw its volumes after the upgrade.** The maintainer: "Keine VM hat Zugriff
auf deren Volume derzeit." *Causes:* (1) images from before 0.8.1 have no `omaflake-volumes` daemon, and 0.8.1's runner no longer gives
macOS's automount tag – nothing is mounted there; (2) in a 0.8.1 image the daemon never ran its script: launchd starts
it at boot before the Nix store's volume is mounted, `/bin/sh` found no script in `/nix/store` (`launchctl print`:
last exit code 127, an empty log). B164's `omaflake-grow-disk` had the same fault. *Fixed:* both daemons start as
nix-darwin's own do, `/bin/wait4path /nix/store && exec …`; `omaflake-volumes` also waits up to a minute for the
devices and mounts each as the guest user on a mount point it owns. Checked on the Mac with a rebuilt image and an
ephemeral VM with two volumes: `/Volumes/plattentest` and `/Volumes/docker`, each its own mount ("mounted by <user>"),
writable, no "My Shared Files"; `omaflake-grow-disk` exits 0. A compatibility device under the automount tag
("My Shared Files" for old images) was tried and dropped – the maintainer: "My Shared Files soll es gar nicht mehr
geben". So VMs on images from before this fix see their volumes only after an upgrade to a rebuilt image.
The way there, an upgrade in place (B178), failed for every macOS VM: the guest user was not one of its Nix's
trusted users (`nix store info`: `Trusted: 0`; "lacks a signature by a trusted key") – the image build adds
`trusted-users` to the installer's nix.conf, and nix-darwin's activation writes its own over it. *Fixed:* darwin-guest.nix
sets `nix.settings.trusted-users = [ user ]`; for VMs from before, `InPlace::apply` first makes the user trusted with
the credential's password on sudo's stdin (nix.conf as a file with the line, the daemon restarted), and the switch puts
nix-darwin's symlink back before `activate`. Checked on the Mac: the `test` VM (0.8.0 image) upgraded in place, its
files kept, `/Volumes/test` mounted (also after a restart), nix.conf nix-darwin's symlink again.
A volume did not show in the Finder's sidebar as "My Shared Files" had: macOS lists under Locations only what
DiskArbitration mounts – the guest's virtiofs.fs takes media with the content hint AppleVirtIOFS, and only the device
with the automount tag gets one; a volume mounted with `mount_virtiofs` (by the user or by root) is neither local nor
a server's (`NSURLVolumeIsLocalKey` false), so the Finder shows it nowhere. *Now:* each volume is a favorite in the
sidebar – `omaflake-volume-favorites` (darwin-guest.nix, LSSharedFileList through JXA) adds the mounted ones and takes
out those it added whose volume is gone; omaflake starts it over SSH with the guest helpers after each start
(`guest_helpers_command`). A launch agent could not: its inserts of paths under /Volumes never took (paths under /tmp
did), over SSH they do – most likely the privacy rules for network volumes (OPEN). Checked on the Mac: a fresh VM
with two volumes has both as favorites after its first start, the `test` VM its volume after an upgrade in place.
*Next (the maintainer):* omaflake Tools brings an FSKit file system so that each volume is a volume under Locations
(OPEN: whether FSKit on macOS 27 passes a path through without a block device, and whether the Finder lists it there).
*Fixed for 0.8.3:* the guest helpers ran twice at a start (the favorites log has two lines, a second session mounting
the volumes again: "Resource busy") – the volumes session now starts once per guest start (a mark in the guest's /tmp);
VMs on images from before 0.8.0 (agent 0.7.4) cannot upgrade in place – only with a fresh disk. Fixed since:
`omaf upgrade --keep` printed nothing and exited 0 when the upgrade failed (it read the ended job wrong – now its
result's message or its error and log). Checked on the Mac: a NixOS image from before 0.8.1 mounts no volume (the
device is there, no `omaflake-volumes.service`) – rebuild; a fresh NixOS VM of a rebuilt image with two volumes mounts
both (9p, writable). Its `/Volumes/My Shared Files` link comes from the catalog's `nixos/docker` 1.0.0 – 1.1.0 without
it is in `omaflake/catalog` (pushed 2026-11-27). *Found after the release (2026-11-27):* the agent's share check
(`omaf diagnose`) still looked for a volume under `/Volumes/My Shared Files/<name>` – now `/Volumes/<name>`; takes
effect with images built after the fix (changelog, Unreleased).

**B187 (2026-11-27, built for 0.8.1; checked on the Mac with NixOS and macOS guests, B189): volumes – several shared folders per VM, shared
between VMs (German "Platten").** The maintainer: "Einer VM kann man keinen, einen oder mehrere 'shared' Ordner
zuweisen. Und VMs können sich auch welche teilen", the folder names in `share/` no longer tied to a VM's name, the New
VM form with a new volume, one there is, or none; still in 0.8.1. And: "die Docker-VM ist … nichts besonderes" –
omaflake knows nothing of Docker, all of it is in the catalog's modules `docker` and `docker-client`. Then: not under
"My Shared Files", but each volume on its own under a name the user gives per VM, by default the folder's name.
*Built (spec §11.13):* `volumes.rs` (volumes.json v1, a volume without a record at `share/<name>`, a VM from before
with `sharePath` gives its own volume that folder), instance.json v2 (`volumes: [{volume, as?}]`, migration from
`share`), QEMU one 9p device per volume (tag = the name in the VM, slots `0x7`, `0xe`–`0x14`), `omaf-runner --volume
<name>=<folder>` one virtio-fs device each (no automount tag), the guests mount every tag at `/Volumes/<name>`
(NixOS `omaflake-volumes.service`; macOS launch daemon reading `AppleVirtIOFSTag` from the I/O Registry – checked in a
macOS 26 guest that `ioreg -r -c AppleVirtIOFS` lists the tag and `/sbin/mount_virtiofs` is there), the diagnosis
checks every volume both ways (agent: `share-read/-write` with `volume`), API `volumes.*` and `vms.create`'s
`volumes`, CLI `omaf volumes` and `omaf new --volume`, the Volumes tab in the app, the Linux app and `omaf ui` (the
Credentials page shape: its VMs as lines with Rename and Take Out, Add VM inline), the New VM form's Volume section.
Docker out of omaflake: no `DOCKER_HOST` line in the networks' details, examples neutral; catalog: `nixos/docker`
1.1.0 without the `/Volumes/My Shared Files` link, `nixos/docker-client` and `macos/docker-client` (CLI, Compose,
Buildx, `DOCKER_HOST=tcp://127.0.0.1:2375` through the forward `2375=<docker VM>`, B180).
*OPEN:* a macOS guest's volume mounted by root with `mount_virtiofs` – owner and permissions for the user; NixOS guests
with several 9p devices; images before 0.8.1 mount nothing (rebuild); the Docker VM's daemon starting before the
volumes are mounted (bind mounts of a container started at boot) – fixed in the catalog's `nixos/docker` 1.1.0
(2026-11-27: docker.service after omaflake-volumes.service and started at boot; checked on the Mac, B193).

**B188 (2026-11-27, proposed for later – not this release): vaults and protected VMs.** The maintainer: someone else
at this computer (a partner, a guest) should neither see sensitive files in the Finder nor start a sensitive VM.
*Idea:* a vault is a volume encrypted at rest – on the host a file (`.omsafe`), not a folder; only when a VM that has it
starts, or the app unlocks it, does it appear as a folder under `share/`, and it is locked again when the last VM stops
(or by hand). A protected VM asks the same before it starts (its disk encrypted, or at least its start and its
package). Unlock with a password, Touch ID or a hardware key (YubiKey). *To decide:* the container (an APFS encrypted
sparse bundle on macOS – mounted by `hdiutil`, its key in the Keychain behind Touch ID; on Linux LUKS in a file or
gocryptfs, which needs no root); where the key lives (Keychain item with `.userPresence` / Secure Enclave; a FIDO2
hmac-secret for the YubiKey); what the CLI does without a GUI prompt; the folder while unlocked is visible to the
user's account like any other – the protection is against someone without the unlock, not against a running session.

**B185 (2026-11-27, built for 0.8.1; the Mac OPEN): networks edited where they are seen.** The maintainer: adding a
VM opened a page of its own with one drop-down, taking one out was a row of buttons – "Denk dir was Besseres aus.
Cool wäre auch Drag & Drop, ein Ziehen von Verbindungen". *Built:* a network's detail has one switch per VM
(`member_switch`: on = member, `networks.join`/`networks.leave`, members first, a member that is no VM any more can
only go; the help says "now" for a running NixOS VM – B179's live join – and "from its next start" otherwise); the
macOS app draws switches in a detail as chips that wrap (`DetailActions`), the Linux app as chips (`action_button`
gives a switch to `chip`), `omaf ui` already showed `[x]`. The picture (macOS app): a VM has a handle, a drag from
the VM draws a line to the pointer (dashed until it is over a network, which then lights up), letting go over a
network joins; each connection's address has × (the line turns red while the pointer is on it), its menu stays. The
`networkMember` page stays for the API. Tests: the switches of both networks. OPEN on the Mac: how it feels.
*The Linux app too (same day; the maintainer: "Netz-Bild auch in der Linux-App – allgemein immer beide Varianten
synchron halten"):* `crates/omaflake-gui/src/graph.rs` draws the same picture on iced's canvas (the feature `canvas`
– lyon 1.0.19–1.0.23 and float_next_after 1.0.0 in Cargo.lock, all older than two weeks, `cargo deny` ok), above
the list as in the macOS app: nodes, curves, a line drawn from a VM to the pointer, the network under it lit, × on
each address; `Layout` is plain geometry with a test (rows, ends, what is under a point, the ×). Not tried in a
Linux window here (no display in this VM): OPEN.
*Second round (2026-11-27; the maintainer: below the picture the opened networks were still not good – "alle
vernetzten VMs wie einzelne Item-Zeilen mit Infos und der Möglichkeit zu entfernen", and adding inline, not a page
with one drop-down):* the switches are gone again. A credentials-shaped row's detail has two new general parts:
`items` (`DetailItem`: lamp, icon, name, subtitle, badges, actions – a network's members with address, state,
traffic and "Take Out") and `add` (`ActionMenu`: a button with a list, a choice runs its action – "Add VM" with the
VMs not in it, or why there is none). Drawn in the macOS app (`DetailItems`, `AddMenu`: a Menu), the Linux app
(`detail_items`: lines and a pick list) and `omaf ui` (a sub-row per member with its actions, one for the list).
Tests: the members' lines and both networks' lists. – The maintainer also saw "entfesselt" in orange: not the
code – the new build talked to the core service of the 0.8.1 build before. `CoreIdentity::current` read the
executable on disk at each `initialize`, which an app replaced in place had made the new one, so the old service
passed for the new and was kept (plan B165's check). *Fixed:* `CoreIdentity::at_start`, taken when the server is
made. Before it, a build of the same version needs the old service stopped by hand.
*Also 2026-11-27:* omaflake Tools in every macOS image (the maintainer: "omaflake Tools immer installieren – bitte
auch mit omaflake Icon"): the image build copies the build for its base into /Applications before it shuts down
(`MacBaseInstaller::guest_tools` – the plain one, the one with the extension for an unchained base); both builds
carry omaflake's icon (`brand/macos/omaflake.icns`, `CFBundleIconFile`). German "entfesselt" for unchained.

**B184 (2026-11-26, built for 0.8.1): an "unchained" macOS base.** The maintainer's idea:
beside the base a variant with macOS's protections off, for what a normal guest refuses – first of all omaflake's own
camera extension before there is a Developer ID (B183, M12), then system extensions and drivers in general, debugging
system processes, security research. Nothing of it is a crack: it is what Apple's own switches do (`csrutil disable`
in Recovery, `systemextensionsctl developer on`, perhaps the boot argument that turns the entitlement check off).
The name (the maintainer: "unchained ist gut"): **unchained** (not "cracked" – that reads as a pirated copy; "unlocked" was the plain
alternative, "unsealed" would claim the sealed system volume is opened, which `csrutil disable` does not do).
- *Rules:* never the default; chosen per image or VM and marked in the app like the other orange states; the guest
  still reaches the Mac through the share folder, the clipboard and the network, so the notes say what is off. No
  user must need it for a feature – the signed camera extension stays the way for everyone.
- *Checked:* Virtualization.framework has no switch for it (SDK headers: only `startUpFromMacOSRecovery` and the
  provisioning of the first start); in a guest `systemextensionsctl developer` answers "cannot be used if System
  Integrity Protection is enabled".
- *Built:* `omaf start <name> --recovery` (`StartOptions.recovery` → the runner's `--recovery`, always with
  `--window`; refused for NixOS VMs and for VMs that start from their image every time). Tests: the runner's
  arguments, the CLI's flags. The Mac: OPEN.
- *By hand first (the maintainer, on the Mac):* a VM that keeps its disk → `omaf start <name> --recovery` → Terminal
  in Recovery → `csrutil disable` → restart → in the guest `csrutil status`, `systemextensionsctl developer on`.
- *Tried by hand on the Mac (2026-11-26, omaflake 0.8.0, guest macOS 27.2.1, image macos/minimal; the 0.8.0 runner's
  `run <vm> --window --recovery`):* before `System Integrity Protection status: enabled.`; in Recovery's Terminal
  `csrutil disable` was taken (under two minutes from start to shut down); after a normal start `disabled.` A clone
  with a new identity (`omaf-runner clone`: new machine identifier and MAC address, as images and VMs are cloned)
  started with `disabled.` too – **the state is in the VM's files and clones keep it**, so an unchained base hands it
  to every image and VM made from it. `systemextensionsctl developer` now answers `Developer mode is off` (with
  System Integrity Protection on it refused to answer); `on` needs root (not tried: sudo asks for the password).
  `nvram boot-args` is empty.
- *An ad hoc signed camera extension loads (2026-11-26, the same VM):* a prototype outside the repository – a
  CoreMediaIO extension (one device, 1280×720 NV12, reads raw frames from the agent's `127.0.0.1:47042` and sends
  each on at once; a test picture without the agent) in an app that asks for its activation, both signed ad hoc
  (`codesign --sign -`), the app with `com.apple.developer.system-extension.install`. With System Integrity
  Protection on (the development VM) macOS kills the app at once (exit 137). In the unchained VM it runs, the
  activation asks for the user's approval in System Settings and finishes: `systemextensionsctl list` shows it
  `[activated enabled]` with team ID `-`. **Nothing else was needed** – developer mode stayed off
  (`systemextensionsctl developer on` over SSH: "no user interaction was possible"), no boot argument. Photo Booth
  listed the camera, the Mac's camera light came on (the extension's connection to the agent started it); the
  picture was coloured noise, as expected: the image's 0.8.0 agent serves H.264 there and the prototype reads raw
  frames. *With the agent of B183* (its binary copied into the VM and started in place of the image's): Photo Booth
  shows the Mac's camera through the prototype – the maintainer: "Flüssiges gutes Bild"; the agent at 3.5 % of a
  core, the extension at 1.3 %. So the whole way of M12's camera works without OBS: Mac → H.264 → agent (decodes)
  → extension → app. The delay is not measured yet.
- *The extension in the repository (2026-11-26):* the prototype became `omaflake-camera` (the extension) and
  `omaflake-tools` ("omaflake Tools.app", asks for the activation), two targets of Package.swift; what needs
  no CoreMediaIO is OmaflakeKit's `CameraFrames` (a frame into a pixel buffer's planes, the test picture) and
  `CameraStreamReader` (whole frames from the agent's port), with tests (`CameraFramesTests`).
  `scripts/bundle-tools.sh` builds and signs the bundle (ad hoc; `SIGN_IDENTITY` with `TEAM_ID` for a Developer
  ID – OPEN whether that needs a provisioning profile), `scripts/bundle-app.sh` puts it into omaflake.app
  (`Contents/Resources/Guest`; `codesign --verify --deep --strict` of the app passes). Identifiers
  `app.omaflake.guest` and `app.omaflake.guest.camera`, the device "omaflake Camera". The agent's diagnosis
  knows it (`obs::own_extension_state`; `extension` in its data): active → pass without OBS. *This build in the
  unchained VM (2026-11-26, as "omaflake Tools.app" with `app.omaflake.guest.camera`):* copied to /Applications,
  started with `open`, approved, `FINISHED: completed`, `[activated enabled]` with team ID `-` beside the
  prototype's; `omaf diagnose` (the 0.8.0 app with this agent in the guest): "Ready – omaflake's camera extension
  is active …", and while Photo Booth showed it "omaflake's camera extension shows the Mac's camera (streaming)".
  The maintainer: "Sehe Bild, sehr flüssig, gut, könnte auch die Kamera auf meinem Host sein." Not built: copying the app into a guest and activating it from
  omaflake (the core, once SSH answers) – with the unchained base or the Developer ID, whichever comes first.
- *omaflake Tools (the maintainer's idea, decided 2026-11-26):* not an app for the camera alone but omaflake's app
  in the guest, where later features go that need an app there – key bindings (Accessibility, Input Monitoring),
  MIDI, the setup of sound and camera, a status in the menu bar, and what is not known yet. The names were fixed
  before anyone approved the extension (new identifiers would mean a new approval in every VM): the app "omaflake
  Tools" (known from other VM software), identifiers that do not carry the display name. Rules: the agent stays
  the core; the extension stays its own sandboxed process; nothing is built ahead of need. To mind: ad hoc signed,
  an app with the entitlement to install system extensions is killed at once in an ordinary guest – until the
  Developer ID a Tools app that does more must be built without that entitlement for ordinary guests (OPEN, with
  the first such feature). An app could also enter itself as a login item and start the agent, replacing the start
  over SSH (BM25): OPEN.
- *The unchained base in omaflake (2026-11-26, built – unreleased; the Mac OPEN):* `mac_base_unchain.rs` –
  `MacBaseInstaller::unchain`: a ready base with a credential → `omaf-runner clone` to `bases/mac-unchained-<…>`
  (its own prefix: no search for a credential's base sees it; `omaflake-base.json` v3 with `unchained`) → started
  in a window, `csrutil status` over SSH → when on: shut down, macOS Recovery in a window with the steps in the
  log, the user types `csrutil disable` and shuts down (30 minutes at most), started and checked again → omaflake
  Tools with the extension copied to /Applications (`scp`) and started, the user approves it, `systemextensionsctl
  list` must show it `activated enabled` → shut down, ready marker, read-only. An unfinished one continues (with
  the protection already off it skips Recovery). `omaf macos unchain <base> [--no-tools]`; `omaf macos bases`
  lists and marks them; `omaf macos remove` removes one. Images: `omaf build <recipe> --unchained` takes the
  credential's newest unchained base (also in the service: the base is a parameter there already), and an image
  from an unchained base has words of its own in its name (`MacImageBuilder::image_name`) – without that the build
  would find the normal image of the same system and do nothing. Tests: names and searches, what is refused,
  reading `csrutil` and `systemextensionsctl`, finding the Tools beside `omaf`, the image's name.
  **omaflake Tools in two builds** (the maintainer's idea) until there is a Developer ID: `scripts/bundle-tools.sh`
  builds the plain one for every guest (no restricted entitlement, no extension – it starts and says there is
  nothing to do) and `build/unchained/` with both; omaflake.app carries them as `Contents/Resources/Guest/` and
  `Guest/unchained/`. With a Developer ID one build for all.
- *On the Mac (2026-11-26, the development build's `omaf`, the maintainer's data folder):* `omaf macos unchain
  mac-base-<build>-<credential>` ran from start to end – cloned, started, "System Integrity Protection is on", Recovery,
  started again, "… is off", Tools copied, the extension approved and `activated enabled`, shut down,
  `mac-unchained-<build>-<credential>` read-only; `omaf macos bases` lists both, the unchained one marked. Then `omaf build
  macos/minimal --unchained` with this checkout's image flake: the system built (with this agent, its tests
  passing in the Nix build), "… is unchained: the image runs with System Integrity Protection off", an image name
  of its own, the base cloned, Nix installed, the closure copied – and **"Activation failed"**: `primary user … does not exist`. Not the
  unchained base – the test built with the checkout's `nix/`, whose `local/settings.nix` (the developer's own)
  sets a `macUser` that is not this base's user. The second run built with the image flake in the development
  build's bundle (no `local/`): the image was built from the unchained base ("Activating", "Shutting down", "Image
  ready"), a VM of it started with `System Integrity Protection status: disabled.` and
  `app.omaflake.guest.camera … [activated enabled]` – **the approved extension survives the clones** (base → image
  → VM); Photo Booth asked once for its own access to the camera (as for any camera) and showed the Mac's camera,
  "flüssiges Bild, alles gut"; `omaf diagnose`: "Ready – omaflake's camera extension is active …". The first
  diagnosis right after `start --wait` had no word from the agent yet (it is started after SSH answers – as
  before). Seen there and changed since: the unchained image had become the recipe's current one (its new VMs
  would have been unchained), and `omaf new --camera` still told to install OBS. *Built after it:* `unchained` in
  image.json (v4) – set by the build from an unchained base; such an image is never made current, `prune` leaves
  it, `omaf images` and `omaf new` say so, `omaf build --unchained` ends with how to make a VM from it. Not tried
  on the Mac again (a development build now writes image.json v4, also when it only lists older images – the
  installed 0.8.0 could not read them: tests only with a data folder of their own).
- *The app (2026-11-26, built – unreleased; the Mac OPEN):* all in the core's page models, so the macOS app,
  `omaf ui` and the Linux app show it without code of their own. `macos.unchainBase {name, noTools?}` – a job of
  the kind that creates bases, its log carrying the steps that are the user's; `bases.list` and the API's image
  carry `unchained`; `images.build` takes `unchained`. Images → macOS Bases: an orange "unchained" badge, "Make
  Unchained Copy …" (with a confirm that says what it is and what it needs) on a ready base with a credential and
  no copy yet, the job's row "Making the unchained copy – open it for the steps that are yours"; unchained copies
  are left out of "Create macOS Base" and of every choice of "the credential's base". Build Image: the switch
  "Unchained" once the credential has an unchained base. Images and VMs: the badge; a VM of an unchained image is
  never offered an upgrade (the recipe's current image is never unchained) and its camera switch says "omaflake
  Camera" instead of pointing to OBS. German for all of it; the golden file of the images page and the Swift
  contract test follow. Tests: the texts' test builds the pages with an unchained base, image, VM and job.
- *An image unchained directly (the maintainer's idea, 2026-11-26, built – unreleased; the Mac OPEN):* "Kann man
  nicht einfach einen unchain-Button bei jeder macOS-Vorlage machen?" – and in the unchained base the keyboard was
  the US one, so the password for the extension's approval could not be typed until it was changed by hand.
  `MacBaseInstaller::unchain_image`: the image cloned into `.build-<name>` (as a build's folder), the same steps
  (`unchain_steps`, now shared), its image.json with `unchained`, renamed to the name a build of the same system
  from an unchained base gets – never current. `images.unchain {name, noTools?}` (a `build` job under the image's
  recipe: its build page shows the log), "Make Unchained Copy …" on every macOS image that has no such copy, `omaf
  images unchain`. The base keeps its way for those who build often (the steps once, every build after without);
  it now gets this computer's keyboard before the approval (`UnchainRequest.keyboard` from `hostKeyboard`; the
  layouts are darwin-guest.nix's table, a test reads both), restarting once when Recovery does not restart it
  anyway. Tests: the copy's name, what is refused, the table, the commands.
- *Recovery without hands in omaflake (2026-11-26, built for 0.8.1; the whole job on a Mac OPEN):*
  `recovery_screen.rs` says which screen it is from the runner's `screen` (startup options, a language, a user,
  the utilities, an open menu, the Terminal) and what `csrutil` asks (a question only in the last line – a test
  caught one answered above being answered again); `Session::recovery_automatic` acts on it (the shortcut once
  the utilities looked the same twice, twice, then the menu), types for the layout of the input menu, in the
  Terminal `csrutil status`, `csrutil disable` with its answers, `halt`; gives up after 8 minutes, or without a
  picture, or on an error of csrutil – then the window is shown and the log carries the steps by hand (the user
  may take over any time). Before Recovery `prev-lang:kbd` is set to `en-US:0` in the copy and put back after (or
  deleted when it was not set) – Recovery comes up in English with a US keyboard, no language to choose. Every
  start of the copy uses the runner's new quiet window (`--quiet-window`: on screen for the window server at alpha
  0, no Dock icon, not clickable; `show` makes it a normal window) – shown when the user is needed: Recovery by
  hand, the extension's approval; without a picture for 45 s it is shown too (whether a quiet window is drawn:
  OPEN). The job's progress has six steps (`STEP_TITLES`, German for each, a test checks), the CLI prints them.
  The probe got the same: the shortcut later, `prev-lang:kbd` before and after (`--as-is` without).
- *Shift in Recovery (2026-11-27):* the maintainer's first unchained base with 0.8.1 went through Recovery by itself up
  to the password – csrutil said the authentication failed, and `csrutil disable` by hand worked. Most likely Shift
  did not arrive: ⇧⌘T had done nothing in the probe (by hand it works), `csrutil status` (no capitals) had arrived,
  and the password has capitals or symbols. *Changed:* the runner sends modifiers three ways (`modifiers`: `device`
  – modifier events with the left key's device bit as a keyboard sends them, the default; `plain`; `keys` – the
  flags on the key events only); in the Terminal the core types `echo SHIFTOK` with each until a line reads
  "SHIFTOK" and keeps that one, and hands over to the user when none works and the password needs Shift
  (`recovery_screen::shift_works`, `needs_shift`; tests). Which way works: OPEN until tried. Unchained is marked
  with a tone of its own, `special` (purple, systemPurple – the maintainer: the orange of "outdated" was wrong, it
  should look like an upgrade or experimental), and the icon `unchained` (an open lock).
- *The extension's approval without a click:* OPEN, an idea to try – with System Integrity Protection off root may
  write `/Library/SystemExtensions/db.plist`, where macOS keeps each extension's state; a probe outside the
  repository lets the extension ask, sets its entry to `activated_enabled` and restarts. Whether macOS takes that:
  not known. (VMs of an image or base whose extension was approved need nothing: the approval survives the clones.)
- *Recovery without hands (2026-11-26, the instruments built – unreleased; nothing tried on a Mac):* the
  maintainer asked whether `csrutil disable` can be automated. No switch exists (Virtualization.framework: only
  `startUpFromMacOSRecovery`; the security policy of Apple silicon changes only from Recovery, which has no SSH), so
  the way is to type for the user. What Recovery shows in a VM (the maintainer): a first screen with two large
  icons, the disk and "Options" – choose Options, the VM restarts into Recovery –, then the choice of language, then
  the utilities, from where Terminal opens. *Built:* the runner's control socket has `type {text}`, `key {key}`,
  `click {x, y}` and `screen {png?}` (`Robot.swift`; `omaf-runner ctl <socket> <command> --args <json | ->`, `-`
  reads the args from stdin so a typed password is in no argument). Keys are US key positions (`GuestKeys`, with
  tests) sent to the VM's view as key events; `screen` takes the picture from the view's own layers (an IOSurface –
  no screen recording permission) and recognizes its text on the Mac (Vision), answering each line with its place.
  *First try on the Mac (2026-11-26):* the picture from the view's layers was black (the window's size, not its
  content), and the answer came after the client's 5 s – the probe stopped before it typed anything. *Changed:*
  `screen` tries the view's surface, its drawing and then ScreenCaptureKit (the window as the window server
  composes it – asks once for screen recording), leaves out black pictures and says what each way gave; `ctl`
  waits 90 s for `screen` and `type`; the probe goes on blind with keys only when there is no picture and asks
  after each key what happened. *Second try (2026-11-26, a German image):* the view's surface gave the picture
  (the first try had come before the screen showed anything), text recognition read it, and the probe's clicks
  worked – "Optionen", then "Fortfahren". Recovery came up in German straight into the utilities, **without** a
  language choice (the maintainer had seen one before – both happen), and its keyboard is the German one (the input
  menu of the startup options says "Deutsch"); its Terminal is a root shell (`-bash-3.2#`), no login. The probe
  waited for a language screen and never pressed ⇧⌘T. *Changed:* `GuestKeys.Layout` (`us`, `de` – the Mac's
  German layout: Y and Z swapped, the symbols where they are; tests), `type {text, layout}`; the probe reads the
  screen in a loop and acts on what it sees (startup options, a language choice, a user and password, the
  utilities, the Terminal) and takes the layout from the input menu. *Third try (2026-11-26, the same VM,
  System Integrity Protection already off):* the way ran through without hands – the layout read from the input
  menu ("Deutsch"), "Optionen" and "Fortfahren" clicked, in the utilities ⇧⌘T opened nothing, the menu
  "Dienstprogramme" → "Terminal" clicked did, `csrutil status` typed with the German layout and read back
  ("disabled"), `halt` typed, the VM off – 52 s from start to off. OPEN: why the shortcut does nothing (modifier
  events, or no such shortcut in Recovery); what `csrutil disable` asks word for word and whether the answers
  arrive – that needs a VM with the protection still on. A probe outside the repository walks the way
  with these commands and writes every screen as picture and text; the automatic steps in `mac_base_unchain.rs`
  follow from what it finds, with today's steps by hand as the fallback. The approval of the camera extension in
  System Settings stays the user's (a click in the running macOS): OPEN.
- OPEN, to build: Virtualization.framework's limit of two macOS VMs
  also counts the base being unchained.
- OPEN: the delay of that way in numbers; whether the base build can do the Recovery steps without hands (they ask for an
  administrator's password in Recovery's own windows – no SSH there); the app's side (choosing and marking it).

**B183 (2026-11-26, built for 0.8.1): the camera in a macOS VM at one picture a second, and late.** The
maintainer's first try of 0.8.0 on the Mac (a new macOS VM, OBS 33.0.3): the picture was choppy, came "a scant
second" late (Camo in the same VM: no such feeling), and stayed black until the VM was restarted.
- *One picture a second – OBS's renderer.* OBS's log of that session: `Loading up OpenGL on adapter Apple Inc. Apple
  Software Renderer`, the virtual camera `Total drawn frames: 142 (3962 attempted)`, 96.4 % lagged, the graphics
  thread's median 587 ms. A macOS VM has no OpenGL on its paravirtual device. B173's "94.5 % lagged" was the same,
  taken for a slow start then. OBS 33.0.3 ships `libobs-metal.dylib`; with `[Video] Renderer=Metal` in global.ini it
  starts on the "Apple Paravirtual device": graphics thread median 0.8 ms, the virtual camera 1725 of 1727 drawn
  (0.1 % lagged) with the Mac's real camera. *Built:* `obs::renderer` sets it while OBS does not run (only when the
  Metal library is there), logged; the diagnosis fails with "Quit OBS once …" while OBS runs with OpenGL, and
  carries `obsRenderer`.
- *Late – OBS's H.264 decoder.* Measured in the guest without the Mac: a stand-in serves a test stream on TCP (30
  pictures a second, dark and bright in turn every second, the time of each change logged) to a copy of the scene
  collection, and a small app reads "OBS Virtual Camera" (AVFoundation) and logs when the picture changes; OBS's
  virtual camera started over obs-websocket (`--startvirtualcam` logs "Starting Virtual Camera output" and never
  starts it – with OpenGL as with Metal; why: OPEN). Median of 24 changes each, stand-in → app: H.264 as 0.8.0 wrote
  the source 382 ms (410 in a second run); `probesize=32` 432; hardware decoding 491; without
  `use_wallclock_as_timestamps` 392; `flags=low_delay threads=1` 384 (the source's FFmpeg options reach the
  demuxer, not the decoder); **raw NV12 79 ms**. So about 300 ms are OBS's decoder – FFmpeg's frame threads hold
  back one picture per thread, eight cores here. *Built:* the agent decodes (`h264.rs`: Annex B split, VideoToolbox
  through its C API, synchronous, NV12 out; `camera_mac.rs`: the writer thread decodes each frame and writes the raw
  picture, a new decoder per reader, the credit back also for a frame without a picture), the scene's media source
  is `rawvideo` again and one left as `h264` is changed back. Tests: the split, the access unit, the writer with a
  stand-in decoder, the renderer and the source's format; VideoToolbox itself with `testdata/flip.h264` (ignored by
  default – not in a build's sandbox; passes in the guest). *Tried in the maintainer's VM* with the agent built by
  hand in place of the image's: the Mac's real camera, 30 pictures a second reaching the app, none skipped in OBS,
  the agent at 5 % of a core.
- *Not measured:* the Mac's part (capture, encoder, virtio port) – OPEN with B174's low-latency mode; and the whole
  way with the real camera by eye (the maintainer).
- *Black until a restart:* OPEN. In OBS's log of the black session the camera extension was replaced (33.0.2 →
  33.0.3) 23 s before "Starting Virtual Camera output", and the line "Virtual Camera Start" never came – the output
  did not start. The agent's log of that start was gone (`/tmp`, overwritten at each start). *Built:* the log is
  `~/Library/Logs/omaflake/agent.log` with the time on every line, the four starts before it kept.
- *The own camera extension without a Developer ID* (to have it ready for M12): OPEN – a guest with System Integrity
  Protection off and `systemextensionsctl developer on`; see B184.

**B182 (2026-11-25, built for 0.8.0): the Linux builder checked once a day; its disk's size.** Follow-ups of B175.
`BuilderHealth::check_in_background`: a Doctor check of a running builder starts the probe in a thread when the last
one is 24 h old (`<data>/builder/checked`, written first – two checks start one probe); a refusal sets the marker, so
the Doctor shows Repair … before a build runs into it. `disk_too_small`: the builder's qcow2 (omaflake's own in its
folder at the omaflake stage; nix-darwin's in the daemon's WorkingDirectory when the system is omaflake's) is read
from its header (magic "QFI\xfb", size at byte 24 – no qemu-img) and compared with builder-vm.nix's 100 GiB (a test
reads the file); smaller → "Linux builder running" is Limited (no blocker) with New Disk … (`builder.repair`). The
maintainer's had 20 GB (B175). Tests: the header, the size against builder-vm.nix, the daily due, the Doctor's action.

**B181 (2026-11-25, built for 0.8.0; clicking OPEN): the network graph edited by dragging (macOS app).** A VM node is
`draggable` (its id), a network node a `dropDestination` (outlined while a VM is over it) that sends `networks.join`;
the address on a line has a context menu "Take <VM> Out" (`networks.leave`); a line under the graph says so. Errors are
toasts. `swift build` only – not clicked in the app here (no screen tool in this session). The Linux app and `omaf ui`
keep the lists' Add VM … / Take … Out.

**B180 (2026-11-25, built for 0.8.0; the guests OPEN): ports of other VMs on a VM's localhost.** From spec §11.12's
"later". `InstanceConfig::forwards` ([{port, vm, to}], schema v1 optional field), `PortForward::parse_list` /
`list_text` ("5432=docker:5432, 2375=docker"); `Networks::forward_targets` turns them into `AgentForward {port, to:
"<address>:<port>"}` with the target's address in a network both VMs are in (a VM with none shared is left out). The
agent's welcome carries them (`forwards`, feature `forward` – allowed when there are any, the agent offers it from
0.8.0 on, an older one gets no list): Swift `AgentLink.forwards` (omaf-runner `run`/`bridge --forward`), Rust
`AgentLink.forwards` (Linux `omaf bridge --forward`), the core passes them at the start. The agent (forward.rs)
listens on 127.0.0.1:<port> and passes each connection on (a port stays bound, its target looked up per connection).
Tests: the agent's round trip through a real listener, the welcome with and without "forward" (Swift), parsing,
targets, the VM's details field (a text stored as typed; "" clears – `vms.set` takes null as "none"). OPEN: in guests.

**B179 (2026-11-25, built for 0.8.0; the guests OPEN): names in networks, joining while running.** From spec §11.12's
"later". *Names:* the switch answers mDNS queries for `<vm>.<network>.local` (`packets::mdns_query`/`mdns_answer`:
names with compression pointers, A and ANY, TTL 120, to the group with cache-flush from port 5353, a legacy query from
another port to the asker with its ID and question, IP TTL 255); members come from networks.json, which the switch
reads every 5 s. NixOS guests: Avahi with nss-mdns; read in nss-mdns' source (util.c): `mdns4_minimal` takes only
`<host>.local` (`label_count(name) == 2`) and returns UNAVAIL for other names, so `mdns4` after DNS with
`/etc/mdns.allow` (`.local`) resolves three labels. Tests: the packets, and a switch with three cards (a query from a
macOS card answered with the Docker VM's address; no answer for a name of no member). *Joining while running:* every
NixOS VM starts with four `pcie-root-port`s (0xa–0xd, chassis 10–13) and its cards behind them (`bus=lanport<i>,
id=lancard<i>`); `networks.join`/`leave` of a running NixOS VM plug and unplug over QMP (`LiveCards`, run/lan.json; the
answer says `now`), a macOS VM still at its next start. Checked against QEMU 11.1.2 (TCG, no guest, in this macOS VM):
`device_add` on `pcie.0` is refused ("does not support hotplugging"), on a root port taken; `LiveCards::attach`/`detach`
against a real QEMU (`OMAF_TEST_QEMU`). OPEN: both in running guests (NetworkManager bringing up a plugged card, the
names from a macOS and a NixOS guest).

**B178 (2026-11-25, built for 0.8.0; the Mac OPEN): upgrade a VM in place.** Until now an upgrade gave a VM a fresh
disk from the new image – everything in it gone. Now `vms.upgrade {keep: true}` (a job, `vm-upgrade`; "Upgrade, Keep
Files …" in a running VM's details; `omaf upgrade --keep`): `InPlace` (`in_place.rs`) builds the new image's system
with the image's credential, look and locale (NixOS `nixosConfigurations."vm-<system>-<recipe>".config.system.build.toplevel`,
macOS the image's `storePath` when it is still in the store, else `darwinConfigurations.<recipe>.system`), copies it in
(`nix copy --no-check-sigs --to ssh-ng://<user>@<address>?ssh-key=<omaflake's key>`; the guest's user is a trusted Nix
user – NixOS base.nix `@wheel`, macOS since BM19) and switches: NixOS `nix-env -p /nix/var/nix/profiles/system --set` and
`switch-to-configuration switch` through a sudo rule in vm-guest.nix for exactly these two commands (the user's
password is "changeme" or theirs – omaflake does not know it), macOS `<system>/activate` with the image credential's
password on sudo's stdin, as the image build does. `InstanceConfig::system_image` (instance.json, schema v1 – an
optional field, as `displays` and `midi` were added) names the image whose system runs; `image` stays the disk's base,
and "upgradable" compares with what runs. A NixOS VM from an older image says to upgrade once with a fresh disk. Tests:
attributes, the switch commands, store URLs (a data folder with a space). OPEN, on a Mac: both kinds end to end; a NixOS
switch with a running graphical session.

**B177 (2026-11-25, for 0.8.0): the dependencies looked through.** Maintainer: look at every dependency like chrono –
newer, better, or our own. *Own instead:* `hex` and `base64` (a dozen calls; `encoding.rs`, the same Base64 as the
agent's `b64.rs`, RFC 4648 vectors); `swift-argument-parser` – only `omaf-runner` used it, a tool omaf starts with a
fixed command line: `Arguments.swift` (`--name value`/`--name=value`, repeated options, flags, positional arguments,
unknown options refused with the usage, exit 64), so the Swift package has no dependencies at all (Package.resolved
gone). *Newer* (each the newest release at least 14 days old): libc 0.2.191, toml 1.2.1, ureq 3.5.1, rfd 0.18.0, ksni
0.3.7, lucide-icons 1.71.0, async-executor 1.14.1 (1.14.0 was yanked). *Kept, with reason:* serde/serde_json, sha2 (no
own cryptography), ureq (HTTPS with stall timeouts for the IPSW), security-framework / objc2 (Keychain, Touch ID),
secret-service (the Secret Service on Linux; 6.0 too young), zune-jpeg, clap, ratatui/crossterm, iced/wgpu (wgpu stays
at 27: iced 0.14 is built on it), image; tempfile only in tests. notify-rust stays at 4.18.2: 4.19 brings zbus 6 next to
zbus 5 (secret-service, ksni). jiff → 1.0 once it is 14 days old (B167). `scripts/check-age.py`: the expired NEEDED
entries removed; six young transitive crates set back (jiff-core, pollster, zbus …).

**B176 (2026-11-25, fixed for 0.8.0): CI's macOS job hung for six hours since 0.7.4.** Every run since 2026-11-22
stayed in `swift test` until GitHub ended it after six hours (the default limit) – and held one of the few `xcode-27`
runners meanwhile, so the release of 0.7.6 waited behind four of them. Reproduced with
`LIBDISPATCH_COOPERATIVE_POOL_STRICT=1` (a cooperative pool of one thread, like a runner with few cores):
`CoreClientTests.blockingCallsForPanelDelegates` called `callBlocking` – a semaphore wait – from a task of the pool,
so the request it waited for found no thread. The app calls it from the main thread (an open panel's delegate); the
test now does it from a thread of its own. With a pool of one thread a few CoreClient and Diagnosis tests still wait
for good (no thread blocked – OPEN whether that is the strict mode itself); against any hang every suite has
`.timeLimit(.minutes(1))` and CI's Test step `timeout-minutes: 15`, so a hanging test fails by name.
*The rest (same day):* the next CI run still hung (the 15 minutes ended it), and locally 4 of 6 parallel runs hung,
0 of 6 with `--no-parallel`; skipping either `CoreClientTests` or `PortWriterTests` made 6 of 6 pass. A hung process had
no dispatch worker thread left at all (so not even the time limits fired). `PortWriterTests` were synchronous tests –
on threads of the cooperative pool – that waited on a semaphore for a reader started with
`DispatchQueue.global().asyncAfter`; with the CoreClient tests alongside, that reader found no thread. Now the reader is
a thread of its own and every PortWriter test runs off the pool (`offPool`, shared with DiagnosisTests in
OffPool.swift): 10 of 10 parallel runs pass. On CI one run in two still hung afterwards (4e0917b; swift test's output
comes only at the end there, so it does not say which test) – CI runs `swift test --no-parallel` now (0 of 6 hung here
serially even before the fix); which test still waits in parallel on 3 cores: OPEN.

**Release 0.7.6 (2026-11-25):** B175 (the Linux builder that refuses the Mac's builds: recognized, checked, repaired
– the new disk checked on the Mac) together with the camera fixes B173 (asked again after a stop) and B174 (H.264
through VideoToolbox; vsock measured in the guest). Maintainer: "Mach zusammen mit dem Kamera-Fix … einen Release
0.7.6". Still to be tried on the Mac: the Doctor's Repair … and Test Build, the H.264 camera with a real camera, the
vsock bench; their OPEN items stay.

**B175 (2026-11-25, built – unreleased): NixOS image builds failed with "lacks a signature by a trusted key".** The
maintainer's build of `neovim-minimal` with 0.7.5 (after B172's fix: the agent built and its tests passed) failed in
`system-units`: nix-darwin's builder refused `…-unit-console-getty.service-disabled`, a path the Mac had built earlier
(unsigned). Checked on the Mac: `sudo nix store info --store ssh-ng://builder@linux-builder --json` → `"trusted":
false`; in the builder `/run/current-system` is exactly the system `darwinConfigurations.linux-builder` evaluates to
here (`hdlaxw96…`), whose nix.conf has `trusted-users = root builder` and `require-sigs = true` (`nix eval` of the
`nix/nix.conf` derivation) – but `grep -E "trusted|require-sigs" /etc/nix/nix.conf` in the builder found **nothing**.
So the file the builder reads is not what its system says: most likely a damaged store on its disk (the disk survives
restarts, `ephemeral = false`; the store is an overlay of the store image over the disk; the root's
`binary-cache-v7.sqlite` there is "not a database"; B172's `nix-store --verify` had removed six stale entries). Why it
got damaged: **OPEN** (a hard stop of the VM? GC through Clean Up?). Also seen: the disk is 20 GB, not builder-vm.nix's
100 GiB – it was created before the setting and is only created when missing. *Built:* `BuilderHealth`
(`builder_health.rs`) – a probe (a tiny aarch64-linux build whose aarch64-darwin input the Mac builds just before,
with a nonce: unsigned, so only a trusting builder takes it; through the daemon as the user, so it works for
nix-darwin's builder whose key is root's), a marker `<data>/builder/refused` set when an image build's output shows
the refusal (the build then says to repair it) and cleared by a passing probe or a built image; the Doctor's
`builderRunning` probes again while the marker is set and reports `refuses this Mac's builds` with **Repair …**
(`builder.repair`: a new disk – omaflake's own builder as the user, nix-darwin's through the administrator dialog –,
waits, probes). **Test Build** (`builder.check`) on a running builder; `omaf builder check` / `repair [--force]`;
`scripts/repair-linux-builder.sh` for 0.7.5 (moves the old disk aside instead of deleting it). *Checked on the Mac
(2026-11-25):* after a new disk for nix-darwin's builder the same `neovim-minimal` build went through – the paths it
had refused were copied (`copying 130 paths…`), `system-units`, `etc` and the disk image built, image ready. So a new
disk ends the refusal. Not tried on the Mac yet: the app's Doctor (Repair …, Test Build) and `omaf builder
check`/`repair` – they need a build of this version; this Mac VM has no hypervisor, so only the probe's Mac half ran
here.

**B174 (2026-11-25, built – unreleased; the Mac OPEN): the camera as H.264, and vsock measured.** From B173: the
console carries ~25 raw frames a second; the maintainer: Camo is smooth in the same VM. Decided (maintainer): H.264
through VideoToolbox, and test vsock too.
- *Built:* the agent offers `camera-h264` (macOS guests only; v4l2loopback on Linux takes raw frames); the runner
  enables it unless `OMAFLAKE_CAMERA_RAW=1`. `CameraEncoder` (OmaflakeKit): VTCompressionSession, low-latency rate
  control where the encoder offers it, real time, no frame reordering, Main profile, 8 Mbit/s, key frame every 2 s
  and on request; each frame one Annex B access unit with SPS/PPS before key frames. `CameraBridge` encodes only
  frames with a credit (a frame without one is never encoded, so the stream misses nothing it refers to), asks for a
  key frame with every "start" (a new reader). Agent (`camera_mac.rs`): frames of any size up to 2 MiB, queued in
  order, never replaced, 4 credits; `obs.rs` changes only the media source's `input_format`/`ffmpeg_options` of an
  existing "omaflake Camera" (`h264`, `framerate=30 fflags=nobuffer probesize=262144 analyzeduration=0
  use_wallclock_as_timestamps=1`) and back, only while OBS is not running (the diagnosis says "Quit OBS once").
- *Checked in a macOS guest:* `CameraEncoderTests` (Annex B; VideoToolbox in the guest: SPS+PPS+IDR first, a key
  frame on request, frames under a quarter of a raw one), agent tests (H.264 in order with credits, the source's
  format both ways keeping the user's changes), all Swift tests, agent fmt/clippy/test, REUSE. End to end in the
  guest with a stand-in for the Mac on a pseudo-terminal (`OMAFLAKE_AGENT_PORT`; x264 test picture, 30 fps, 8 Mbit/s,
  frames only against credits) and the new agent: the agent switched OBS's source to `h264`, OBS 33.0.2 showed the
  picture at 30 fps, ~990 KiB/s to OBS, the agent at 4 % of a core (raw: 75 %), 817 frames in 30 s with 820 credits
  and none short. With the 0.7.5 runner the agent switched the source back to `rawvideo` once OBS had quit.
- *vsock:* the macOS guest's kernel knows `AF_VSOCK` (`sys/vsock.h`, family 40), but our VMs have no socket device:
  `socket()` fails with ENODEV. The runner adds a `VZVirtioSocketDeviceConfiguration` and listens on port 47100 when
  `~/.omaflake/developer/vsock-bench` exists (`VsockBench`, sends raw frames' worth of bytes for 10 s; result in
  runner.log); `scripts/vsock-bench.py` in the guest connects and prints MB/s. OPEN: the measurement on the Mac.
- OPEN, on the Mac (needs this runner and an image with this agent): the real camera's smoothness, latency and
  picture at 8 Mbit/s, the encoder's low-latency mode, an image before this version (raw frames still work: the old
  agent offers no `camera-h264`).

**B173 (2026-11-25, fixed – unreleased): the camera in a macOS VM stayed black with OBS; the picture is choppy.** The
maintainer's first try on the Mac (image built with 0.7.5, OBS 33.0.2, the extension approved): OBS connected at its
start, the Mac sent the two frames of the first credits, and the agent's log showed `"no credit from the guest for 2 s",
"stopped"` – OBS hung for seconds at its start (its log: 94.5 % of the virtual camera's frames lagged), the credits came
too late, and nothing asked again: for the agent OBS was still reading. A reader that replaced another did not ask
again either (its credits for frames to the old one were lost). *Fixed:* `CameraRetry` in the agent asks the Mac again
after a "stopped" while an application reads (1 s, 2, 4, then every 8 s; "unavailable" is not retried); a replacing
reader counts as opened (`camera_mac.rs` `events`). Tests: `the_camera_is_asked_again_after_a_stop_with_a_growing_delay`,
`a_reader_replacing_another_counts_as_opened`; agent fmt/clippy/test. Tried in the maintainer's VM with the agent built
by hand (the image's 0.7.5 agent stopped, the new one started as `omaflake-agent-start` does): frames reached OBS at
once, and the maintainer saw a picture, but choppy. *Choppy:* measured 25.6 fps reaching OBS (353,894,400 bytes in 10 s), the agent at 75 % of a core; a raw frame is
1,382,400 bytes and the virtio console carries about 35 MB/s, so ~25 fps is the port's limit, and the Mac's 30 fps lose
every sixth frame unevenly. A test reader in Python got 15 fps. The maintainer: Camo is smooth in the same VM, so it is
our transport. Next: fewer bytes per frame (compressed on the Mac with VideoToolbox) or another channel (vsock for
macOS guests – OPEN whether AF_VSOCK works in a macOS guest); OPEN which.

**Release 0.7.5 (2026-11-25):** B169 (camera in macOS VMs through OBS), B170 (networks between VMs, Docker VM), B171
(several displays for NixOS VMs, QEMU patchset 7) and B172 (the agent test that broke every NixOS image build of
0.7.4). Released before the Mac tests (maintainer: "Committe das und bring 0.7.5 raus") – B169, B170 and B171 are
still to be tried on the Mac; their OPEN items stay.

**Next (2026-11-23, after release 0.7.4):** the maintainer's tests on the Mac – a fresh Mac with Set Up for NixOS
(the Linux builder's two stages, Nix's one administrator question, Full Disk Access: OPEN, B160), creating and
replacing a base per credential (B166), the core handover after an update (B165), a forced stop and the build window,
the diagnosis on NixOS guests and on a Linux host (B162), MIDI with the real piano and Dorico (B161). Not built yet:
the disk size of a VM (B164), upgrading a VM in place, why a macOS build VM once had no SSH (port 22 closed, 2026-11-22
– watch it with the build window when it happens again), and M12 once the Developer ID is there.

**B172 (2026-11-25, fixed for 0.7.5): NixOS images did not build with 0.7.4.** The maintainer's build of
`neovim-minimal` failed in the Linux builder: the agent's checkPhase ran `midi_alsa::tests::ump_info_for_alsa`, which
fed the manufacturer `"00213C"` – `midi::hex_bytes` takes lowercase only (the format agreed with the Mac side, and
`an_invalid_ump_is_ignored_and_the_endpoint_kept` checks that uppercase is refused), so the id became 0 and the assert
failed (left 0, right 0x213C). Nix then reports "path …-omaflake-agent-0.7.4 is not valid" – only the follow-on of the
failed build, not a broken store (first misread as a builder problem; `nix-store --verify` in the builder did remove six
stale entries of its old system, unrelated). Cause it slipped through: the agent is its own Cargo workspace and no check
ran its tests – not the CI, not the checklist's `cargo test` at the root. Fixed: the test uses `"00213c"`; CI's Linux
job runs clippy and the tests of `nix/agent` (on x86_64: OPEN until the first run). Every 0.7.4 install that builds a
NixOS image with a fresh agent hits it – a reason not to wait long with 0.7.5.

**B171 (2026-11-25, built for 0.7.5 – maintainer: "setze alles um, es soll neben Kamera-Fix und Networking Teil von
Release 0.7.5 werden"): several monitors for a VM, each in a window of its own.** Question
from the maintainer: can a VM drive more than one monitor – ideally a second window that can be moved to another
screen? Today every VM has one display and one window (QEMU: one `virtio-gpu-gl-pci` with `-display cocoa,gl=es`;
runner: one `VZMacGraphicsDisplayConfiguration`, one `VZVirtualMachineView`).
- *macOS guests: not possible with Virtualization.framework.* The SDK (macOS 27.2) says "Maximum of one display is
  supported" for `VZMacGraphicsDeviceConfiguration.displays` and one scanout for
  `VZVirtioGraphicsDeviceConfiguration.scanouts`; `VZVirtualMachineView` has no way to pick a display. A
  `validate()` probe on the Mac (macOS VM's hardware model and machine identifier, nothing booted) rejected both ways:
  one device with two displays ("Number of displays is greater than the configured display port count") and two
  devices with one display each ("Number of graphics devices is greater than the maximum number supported"). The
  "display port count" has no public setting. *Private API* (read from the ObjC runtime on macOS 27.2; maintainer:
  try it, hacky is fine): `VZMacGraphicsDeviceConfiguration` has `_setDisplayPortCount:` (default 1) and
  `+_maximumAllowedDisplayCount` (returns 1); `VZVirtualMachineView` has `_setGraphicsDisplay:` (bind a view to one
  display); `VZMacGraphicsDevice` has `_validateDisplayForHotPlug:error:`; `VZMacGraphicsDisplayConfiguration` has
  `_setConnectionType:`, `_setDisplayIdentifier:`; `VZGraphicsDevice` has `_attachDisplay:completionHandler:`
  (hot-plug at run time) and `_displayPortCount`; ParavirtualizedGraphics' `PGDeviceDescriptor` has
  `displayPortCount` and `externalDisplayPortMask`. Probe on the Mac (APFS clone of a macOS VM): port count 2 with
  two displays fails `validate()` ("The display port count is invalid, it must be between 1 and the maximum
  supported"); with `+_maximumAllowedDisplayCount` overridden to 2 in the process it passes, but `start` fails with
  VZErrorDomain 1 "Internal Virtualization error" – the limit is enforced again outside the process. Unattended run
  on the Mac (macOS 27.2.1, M1 Max, 2026-11-25): the control (one port, one display, the same private calls,
  `_setGraphicsDisplay:` on the view) boots and the guest takes the display (`displayDidEndReconfiguration`, 1920×1200);
  every configuration with port count 2 fails 0.2 s after `start` – with one or two displays, with
  `_deviceFeatureLevel` 0–3: the VM's own `com.apple.Virtualization.VirtualMachine` process starts, logs one error
  (text redacted as `<private>`) and the client gets breadcrumb `0xb1ae81a300000829`. Hot-plug with port count 1:
  `_validateDisplayForHotPlug:` fails (VZErrorDomain -12) and `_attachDisplay:` traps the process. So the limit sits in
  Apple's signed XPC service; getting past it would mean patching a system binary (SIP off) – not a way for
  omaflake. **Result: no second display for macOS guests on macOS 27.2;** repeat the probe on new macOS releases
  (the private API shows Apple builds toward it). Private API would keep omaflake off the App Store
  and may break with any macOS update – at most an experimental setting.
- *Linux guests on VZ* (for reference; omaflake runs NixOS on QEMU): two virtio graphics devices with one scanout
  each pass `validate()`, one device with two scanouts does not ("More than one scanout is configured"). Whether the
  second device boots and can be shown in a view of its own: OPEN, not checked.
- *NixOS guests on QEMU: possible, work in our Cocoa frontend.* QEMU 11.1.2's `virtio-gpu` reports
  `num_scanouts = max_outputs` to the guest (up to 16, `hw/display/virtio-gpu-base.c`), so
  `virtio-gpu-gl-pci,max_outputs=2` gives the guest two outputs. `ui/cocoa.m` has one display change listener and one
  `NSWindow`; further consoles are only reachable through the menu / Ctrl+Alt+number in the same window. GTK already
  detaches a console into a window of its own ("Detach Tab", `ui/gtk.c`) – the model for a Cocoa patch: one listener,
  window, GL (ANGLE) context and input routing per console, the window size following each output (as
  `qemu-cocoa-dynamic-display` does for one). On a Linux host GTK may give this almost for free – OPEN, not checked.
  OPEN: whether GNOME/KDE in the guest show the two virtio outputs as two monitors (not seen in a running guest), and
  the effort of the Cocoa patch (needs a prototype).

*Built (same day, in a macOS guest without the hypervisor; not released):*
- *Core:* `instance.json` `displays` (optional, 1 to `InstanceConfig::MAX_DISPLAYS` = 4, schema v1 as MIDI was added –
  before 1.0 no new version, B141); `QemuCommand` adds `max_outputs=<n>` to the GPU in window starts (headless keeps
  one output), GTK gets `show-tabs=on`; `QemuCapabilities.displays` (the string `cocoa-display-gl` in the binary); the
  Doctor calls an omaflake QEMU without it "older patchset" with the update (as for patchset 6).
- *API and UIs:* `Vm.displays`, `vms.set` `displays` (NixOS, stopped, 1–4), `vms.create` `settings.displays`; the VM's
  details and "New VM …" get a number field "Displays" for NixOS VMs – drawn by the app, `omaf ui` and the Linux app
  from the page models, no client code; `omaf new/set --displays N`; German texts.
- *QEMU patch `qemu-cocoa-displays` (patchset 7, ~800 lines in `ui/cocoa.m`):* a `QemuCocoaHeadView` per further
  graphic console with its own listener – 2D (`drawRect`), CGL (its own `CAOpenGLLayer` sharing the GL view context)
  and ANGLE (an EGL surface of its own on the shared context); the guest's cursor per display; a shown window sends its
  size as UI info (`xoff` = widths before it), closing it sends 0×0 (virtio-gpu disconnects the output), the View menu
  entry of its console shows it again; pointer events of every window go to the one tablet in a coordinate space of all
  active displays side by side (`cocoa_queue_abs`, the main window too); the main window tracks the pointer while the
  app is active, so moving between windows needs no click.
- *Checked:* `cargo fmt/clippy/test` (workspace; golden files regenerated), `scripts/translations.py --check`, the
  QEMU package built with Nix (`nix build .#qemu`, cocoa.m without warnings); a smoke start of that QEMU under TCG with
  the firmware, `virtio-gpu-gl-pci,max_outputs=2`, `-display cocoa,gl=on`: two windows ("… – Display 2"), runs, no
  crash. **OPEN (needs the Mac):** a real NixOS VM with 2 displays – both monitors in Hyprland, the pointer and clicks in
  the second window, closing/reopening it, full screen on a second screen, `gl=es`; GNOME/KDE's layout; GTK on Linux.

**B170 (2026-11-25, built for 0.7.5 – maintainer: "Baue nun alles besprochene mit in den geplanten 0.7.5 Release";
real VMs OPEN): Docker for development VMs, and networks between VMs.** Starting point: a development VM without Docker
is of little use. Findings:
- *NixOS guests run Docker themselves* – containers need no hypervisor, only the guest's kernel; the old `images.dev`
  had "docker active" (M1, §4).
- *macOS guests cannot:* Docker on macOS always needs a Linux VM (Docker Desktop, Colima, OrbStack, Apple's
  `container` all use Virtualization.framework), and a macOS guest has no nested virtualisation (§7.2 `mac-dev`).
  Nested virtualisation in VZ (M3 and later, macOS 15) is for Linux guests only, to our knowledge – OPEN, not checked.
- *The host's Docker in the guest* works technically but is rejected as the default: the Docker socket is root on the
  Docker host, and Docker Desktop/OrbStack mount `/Users` – a disposable guest could read and write the whole Mac.

*Spike (same day, in a macOS guest without VMs: simulated cards on sockets):* `gvproxy` 0.8.9 (nixpkgs, Apache-2.0)
puts one QEMU stream client and one vfkit datagram client on one segment (DHCP for both, frames both ways), but a
second client of either kind gets nothing – one VM per kind. A switch of omaflake's own was needed anyway.

*Decided (maintainer, 2026-11-25):* **a second card** – a VM keeps its own NAT (SSH, internet, network mode unchanged)
and joins networks with an extra card on omaflake's switch in Rust (no Go binary, no risk for VMs without networks);
`isolated` for macOS (BM27) stays a later step. **Opt-in, OPEN marked** in 0.7.5 – nothing here could run a VM.

*Built* (spec §11.12): `networks/` in the core (store `networks.json` v1, addressing `10.231.<n>.<host>` with the MAC
`02:6f:6d:<n>:<host>:00`, the switch with ARP/ping/DHCP and `status.json`), `omaf network-switch` (started before a
member VM by `Networks::prepare`, ends a minute after its last port), QEMU's `-netdev stream` cards (slots 0xa–0xd,
`reconnect-ms` checked in QEMU 11.1.2's `qapi/net.json`), the runner's `--lan-mac/--lan-socket/--lan-switch` with
`VZFileHandleNetworkDeviceAttachment` (`NetworkCard` in OmaflakeKit), `omaf networks …` and `omaf new --join`, the API
`networks.*`, the Networks tab (Run group) with its graph in the app (SwiftUI Canvas), the Linux app (cards) and
`omaf ui` (a tree), German texts, a VM's removal taking it out of its networks, `networks.json` moving with the data
folder. Guests: NixOS's NetworkManager takes a new card by itself; macOS gets a launchd daemon
(`networksetup -detectnewhardware` at boot, darwin-guest.nix). Catalog: module `omaflake/nixos/docker` (Docker 29.8.1,
Compose, socket activation on `/run/docker.sock` and `0.0.0.0:2375`, the firewall open for it, the user in `docker`,
`/Volumes/My Shared Files` → `/mnt/share` for bind mounts from macOS VMs) and recipe `nixos/docker` (headless) – both
evaluated with the image flake.

*Found on the way:* on macOS, std's `UnixDatagram::recv_from` takes the last byte of a sender's path for its NUL – a
sender that bound without one (Python) came out a character short and the answers went to `lan0.soc`; the switch now
keeps the raw address (test `answers_reach_a_sender_bound_without_nul`). The Swift test
`CoreClientTests.responsesInAnotherOrderFindTheirRequest` (unchanged) can hang under load: it relies on 100 ms for the
slow request to arrive first; hung `swiftpm-testing-helper` processes of timed-out runs then made later runs hang too
(killed – the run passed). OPEN: make that test wait for the slow request instead of sleeping.

*Checked:* Rust workspace tests (core 494 incl. the switch with a QEMU card and two macOS cards in one process, the
end-to-end with the real `omaf network-switch` process and simulated cards: ARP, DHCP, unicast across kinds, `omaf
networks show` with traffic), `swift test` (incl. `NetworkCardTests` against a stand-in switch and the view contracts),
clippy, fmt, `reuse lint`, the catalog's `--check`, the Docker VM's evaluation. OPEN – on the Mac: a NixOS and a macOS
VM in one network (DHCP in both guests, ping, `docker ps` from macOS with `DOCKER_HOST`), a macOS image built with this
version, QEMU starting while the switch is not yet up, a runner's card after the switch restarted, throughput.
*Later:* rules per network and `isolated` for macOS (a gateway on the switch), DNS names, ports to a client's
`localhost`, joining while running, editing in the graph.

**B169 (2026-11-25, planned – release 0.7.5): the camera in macOS VMs, through OBS until M12.** Maintainer: the
camera switch works for macOS VMs in the app as if we were signed; the only difference is that the guest needs OBS
for now. OBS is a stopgap: nothing may bind omaflake to it in the long run – no catalog module; the agent sets up
OBS's scene when it finds OBS. The design keeps M12 a drop-in: the agent offers the frames at one endpoint in the
guest; today OBS reads it, later omaflake's own camera extension.

1. *Agent, macOS (nix/agent):* offers `camera` on macOS too. New `camera_mac.rs`: a listener on `127.0.0.1:47042`
   (raw NV12 1280×720, one client; a second replaces the first); a client connecting = `camera_start` (credits, as
   on NixOS), disconnecting = stop; frames from `MSG_CAMERA_FRAME` go to the client on their own thread, newest
   frame only, and a credit goes back per frame the client took – so the client's reading paces the Mac.
2. *Agent sets up OBS:* the agent runs as the guest's user (checked: the 0.7.4 agent). With the camera enabled and
   `/Applications/OBS.app` present, it writes the scene collection "omaflake Camera" (one media source as in B168:
   input `tcp://127.0.0.1:47042`, `rawvideo`, `video_size=1280x720 pixel_format=nv12 framerate=30
   use_wallclock_as_timestamps=1`, scaled to OBS's canvas from `basic.ini`) to
   `~/Library/Application Support/obs-studio/basic/scenes/` – only if missing, never touching the user's
   collections and never while OBS runs. Choosing it and "Start Virtual Camera" stay the user's (OBS has no file
   for "start with this collection" that the agent should change – OPEN whether `global.ini` may be touched).
3. *Diagnosis (macOS):* the camera check reports the endpoint, whether a client reads, the Mac's last status, OBS
   found or not, the scene collection written, and the extension's state from `systemextensionsctl list`
   (`activated enabled` / `waiting for user` → "approve OBS Virtual Camera in System Settings → General → Login
   Items & Extensions → Camera Extensions").
4. *Runner (Run.swift):* when the agent's welcome enables `camera`, the camera moves from the port
   `org.omaflake.camera` to the agent (`cam?.useAgent(link)`, `link.onCamera`), as `Bridge.swift` already does for
   NixOS; the port stays for images before 0.7.5 (with B168's `PortWriter`).
5. *Core, API, app:* the switch for macOS VMs (`views/vms.rs` – no more "comes with a signed release"), the camera
   badge, the new-VM field (`views/new_vm.rs`), the recipe summary (`views/recipes.rs`); the help for macOS VMs:
   "In the VM the camera is “OBS Virtual Camera”: OBS, the scene collection “omaflake Camera”, Start Virtual
   Camera". `omaf new/set --camera` drops its M12 note; translations (de.lproj → `scripts/translations.py`); view
   test data.
6. *Docs:* spec §11.9 (macOS camera), cli.md, the app docs, changelog (Added; needs an image built with 0.7.5 and
   OBS in the VM), this entry's results.
7. *Check:* fps with the agent instead of B168's Python helper (goal ≥ 25 at 1280×720; below that OPEN: fewer bytes
   per frame), all checks of CLAUDE.md, a macOS image built with 0.7.5, then on the Mac: switch in the app → camera
   permission → OBS → Photo Booth and FaceTime; an image before 0.7.5 still gets the port.

*Built (same day, in a macOS guest; not released):* 1–6 as planned. The agent's `camera` module comes from
`camera_mac.rs` on macOS (`#[cfg_attr(..., path)]`), both with one interface (`available`, `open(credit)`,
`poll_fds`/`events`, `write_frame` returning the credits to give back), so the main loop has no platform branches;
the writer thread sends its credits through `write_port`. The maintainer allowed the agent to make the collection
OBS's current one (`user.ini` `[Basic] SceneCollection/SceneCollectionFile` since OBS 31, `global.ini` before) – only
when it writes the collection, so a collection the user chose later stays theirs. The diagnosis on the Mac passes the
macOS camera check to the guest (`SKIP_CAMERA_MACOS` is gone); an agent before 0.7.5 still answers "skipped".
Checked: agent `cargo fmt/clippy/test` on macOS (43 tests, among them the stream with a real TCP reader and the OBS
files in a temporary home), workspace `fmt/clippy/test`, `swift build/test` (60), `reuse lint`, `nix flake check` of
the image flake with the catalog, and the agent's Nix package for aarch64-darwin (its tests run in the build). Found
by that build: on macOS an accepted socket inherits the listener's `O_NONBLOCK` and a `try_clone` shares it, so the
writer thread wrote non-blocking and gave up on a reader that fell behind (the test hung under the build's load;
alone it passed by luck). The reader's socket is blocking now; hang-ups are seen with `recv(MSG_DONTWAIT)`; the test
lets the reader fall behind on purpose and has a read timeout. `package.nix` allows loopback networking
(`__darwinAllowLocalNetworking`) for that test where the darwin sandbox is on. OPEN: the agent's Linux build (no Linux builder in the guest – CI), the version
bump to 0.7.5, a macOS image built with it and the whole chain on the Mac with the fps.

**B168 (2026-11-25, prototype; the real Mac camera OPEN): camera in macOS guests through OBS Virtual Camera.** Until
the Developer ID (M12), a signed third-party camera extension can stand in for ours. Checked in a macOS 27.2.1 guest:
OBS 33.0.2's camera extension (team `2MMRE5MTB8`) is approved and loaded in a Virtualization.framework VM
(`systemextensionsctl`: `activated enabled`; the user switches it on in System Settings → General → Login Items &
Extensions → Camera Extensions, with the password) – so a camera extension works in our guests at all, which M12's
own extension needs too. The path: macOS guests still use the old port `org.omaflake.camera` (the runner does not
move the camera to the agent), so a guest-side helper (prototype outside the repository, Python) reads it (raw tty,
`{"type":"start"|"stop"}` lines, `OMFC` messages) and hands the raw NV12 frames to one TCP client on
`127.0.0.1:47042`; the Mac's camera runs only while a client is connected, frames the client does not take at once
are dropped. OBS reads them as a media source: not a local file, input `tcp://127.0.0.1:47042`, input format
`rawvideo`, FFmpeg options `video_size=1280x720 pixel_format=nv12 framerate=30 use_wallclock_as_timestamps=1`; its
virtual camera then shows the guest's own test pattern in Photo Booth. Found on the way: the virtual camera must be
started after the extension's approval (OBS started before shows nothing until "Start Virtual Camera" again); a
process without `com.apple.security.device.camera` (here the Claude Code CLI) gets no camera at all, not even a
prompt. *With the Mac's camera (same day):* the app's switch is greyed out for macOS VMs, but `omaf set <vm>
--camera` turns it on (the CLI only notes M12); at the first start macOS asked on the Mac whether omaflake may use
the camera, and the ad-hoc signed runner (`com.apple.security.device.camera`) got it. The FaceTime HD camera then
showed in OBS and in its virtual camera – about 20 fps, choppy (the maintainer: "very jerky"). Two findings: (1) the
runner gives every write into the camera port 1 s (`SO_SNDTIMEO`, `Run.swift` `portPair`); a slower guest makes
`writeAll` fail, and the bridge then stops the camera **silently** – no status to the guest, half a frame left in
the port, the camera light off until the next "start". The first helper hit this (it wrote to OBS on the reading
thread): one frame, then nothing. *Fixed (same day):* `PortWriter` (OmaflakeKit) writes header and payload as one
message – not a byte written by the timeout: the frame is dropped (live video); begun: finished (up to 5 s), so the
guest's reader stays in step; closed or stuck: failed. The bridge stops the camera only when the guest took no
frame for 2 s or the port failed, and then says so (status "stopped" with the reason). Tests: `PortWriterTests`
(whole message, full port drops without writing, a begun message finished when the guest reads late, stuck halfway
and closed fail). Not yet checked with a VM: the Mac runs the installed runner until the next build there. (2) The guest's port comes with `ctsflow` and `-clocal`; the helper now sets
`clocal`, reads whatever is there and resynchronises on `OMFC`, writes to OBS on its own thread (newest frame only)
and asks for the camera again after 3 s without frames. The ~20 fps are the Python helper at ~85 % of a core over
the virtio console (B70's limit for a Python reader was 26–27 fps) – a native reader (the Rust agent) or fewer bytes
per frame should give more. OPEN: whether the helper moves into the agent as an interim feature before M12, the
smoothness in OBS (its media source buffers), latency.

**B167 (2026-11-23, decided; done 2026-11-25 for 0.8.0): chrono → jiff.** *Done:* chrono is gone from the tree
(`cargo tree -i chrono` finds nothing), `cargo deny` passes again. jiff 0.2.42 (the newest release at least 14 days old;
1.0 came out 2026-11-15 – the move to 1.0 once it is old enough), features `std`, `serde`, `tz-system`,
`tzdb-zoneinfo` (the local zone from the system's zoneinfo, no bundled database). One type for a point in time,
`json::Date` (= `jiff::Timestamp`), and helpers in `json.rs` for what chrono did at the call sites: `whole_seconds`,
`whole_millis`, `seconds_between`, `from_system_time`, `local` / `local_now` (strftime patterns as before); the ISO 8601
formats Swift writes are unchanged (`dates_as_swift_writes_them`). The branch `wip/b167-jiff` (a mechanical first pass)
was not used. Before: After the push of 0.7.4 `cargo deny` failed on
RUSTSEC-2026-0543 ("chrono is unmaintained", no safe upgrade; jiff or time suggested) – CI on main is red for that
alone (fmt, clippy, tests, Linux host, flakes, licences pass). Maintainer: replace chrono with jiff completely (core,
omaf, TUI, the Linux app), not ignore the advisory. Also seen: async-executor 1.14.0 is yanked (through zbus) – a
warning, `cargo update -p async-executor` with the age rule (scripts/check-age.py).

**Release 0.7.4 (2026-11-23):** B159 (`path:` references with a space), B160 (setup by one click, user-level: Nix with
one administrator question, omaflake's own Linux builder as a LaunchAgent, Nix in ~/.zshrc, omaflake's SSH key, every
step visible), B161 (MIDI in macOS and NixOS guests on both hosts, timestamps, UMP endpoints, the pair named after the
VM), B162 (the diagnosis from the UI, the clipboard helper started until it runs), B163 (the classic German layout in
macOS guests), B164 (cores and memory of a VM), B165 (a new app replaces an old core; stop forces off; the build
window), B166 (a macOS base per credential), the Doctor's details translated. Built in a macOS guest without a
hypervisor; tried by the maintainer on the Mac (2026-11-22): a new macOS VM from a 0.7.4 image with MIDI (the
Scarlett 4i4 in the guest, notes arrive), the diagnosis (all but the clipboard passed – fixed before the release),
the build of a macOS image. After the push: the image flake's lock at the 0.7.4 code (as for every release), website
`update.json`.

**Next (2026-11-04, after release 0.7.3):** nothing left to build without the Developer ID – the smaller items of
2026-10-24 (B125) are done, B158 closed the remaining OPEN items, and the 2 s of black between Plymouth and Hyprland
are accepted for now (maintainer, 2026-11-04: the loading screen is fine on macOS). Left: **testing** – the
maintainer tries 0.7.0–0.7.3 (clicks in the apps; a VM's loading screen and keys) and the OPEN items that need a
running guest or a real Linux desktop (docs/linux.md) – and **M12** once the Developer ID is there. GitHub Pages of omaflake and website already publish
through Actions (checked 2026-11-04: `build_type` workflow, domains omaflake.dev and omaflake.app).

**B165 (2026-11-22, built for 0.7.4): a new app keeps talking to the old core service.** Built: `initialize`'s `core {executable, size, modified}`; the app (and the CLI) compare it with their own `omaf` and call `service.restart`, then connect to their own service once the old one ended (VMs keep running); a notice while it waits for jobs. Also in this round-up: a stop whose clean shutdown cannot be asked or takes over 60 s forces the VM off with a log line and an activity warning; a macOS build VM can be shown in a window (`window`). OPEN: tried against a real older service, a forced stop and `--window` on the maintainer's Mac. The maintainer replaced
/Applications/omaflake.app; the running `omaf serve --socket` (started 8 h before by the old app, idle-exit never
reached while a VM runs) stayed, the new app connected to it, and VMs, builds and the diagnosis ran the old code – the
Doctor's "App 0.7.4 · Core 0.7.4" could not tell (same version). Wanted: the core reports a build identity (the
executable's hash or mtime and path), the app compares it with its own bundled `omaf` and, when they differ, asks the
old service to stop after its jobs and starts its own (or says so); the CLI the same. Meanwhile: quit the app,
`pkill -f "omaf serve"`, start it again.

**B166 (2026-11-22, built for 0.7.4): several macOS bases per macOS build, one per credential.** Built: base names per build and credential (the first keeps `mac-base-<build>`), the base record v2 with `ownKey` and `version` (docs/schema/base/v2.json; a v1 record reads as without omaflake's key), `macos.installBase {replace}` (the new base first, then the old one removed), `macos.removeBase` (refused while a build uses it), the build page's credential field and its note, the macBase page's bases list, the Doctor ok when the default credential has a ready base, CLI `omaf macos install --credential --replace`, `omaf macos remove`. OPEN: creating and replacing a base on the maintainer's Mac. A base carries its user (the credential it was made with), so every image of it has that user; with one
base per build a second credential cannot make macOS images. Wanted: bases named per build and credential
(`mac-base-<build>-<credential>`), the build page picking the base of its credential (offering to create it),
"Create Again" for a base (e.g. one from before omaflake's own SSH key, B160 – its builds still need the user's key in
the ssh-agent) and "Remove", the existing base assigned to its credential once.

**B164 (2026-11-22, cores and memory built for 0.7.4; the disk built 2026-11-25 for 0.8.0): a VM's resources cannot be changed.**
*The disk (2026-11-25):* `vms.set {diskGb}` / `omaf set --disk` / a number field in the VM's details (smallest value
= what it has, at most 4096 GB); only larger, only while stopped. NixOS: `qemu-img resize` of the overlay
(`InstanceStore::grow_disk`; an ephemeral VM's overlay is made with the size at its start); the guest grows partition
and root file system at boot (vm-guest.nix `boot.growPartition`, `fileSystems."/".autoResize`, both set already).
macOS: disk.img (raw, sparse) set to the size, config.json says so (`VzBackend::apply_resources`, also after every
clone, so ephemeral VMs keep it). In the guest the system container cannot simply grow: the layout, read in this macOS
27 VM (`diskutil list`), is ISC container (disk1s1, 524 MB), system container (disk1s2), fallback recovery
(Apple_APFS_Recovery, disk1s3, 5.4 GB) at the end – 40 KB short of the disk's size. darwin-guest.nix's boot daemon
`omaflake-grow-disk` finds the system container's physical store, adds up the partitions; with more than 1 GiB not in
any of them it runs `diskutil repairDisk` (the GPT's backup header to the new end), removes the fallback recovery
(`diskutil eraseVolume free free`; the paired recovery volume inside the system container stays) and
`diskutil apfs resizeContainer <store> 0`; log /var/log/omaflake-grow-disk.log. Its reading part ran in this VM (no new
space → nothing done); `sh -n` and shellcheck clean. OPEN, on a Mac: a NixOS VM grown and booted; a macOS VM from a
0.8.0 image grown – whether `eraseVolume free free` takes the recovery container, whether Recovery still starts.
Before: Built: `vms.set {cpus, memoryGb}` for a stopped VM (limits from the host; macOS VMs also write the bundle's config.json, which the runner reads), the VM's details fields, `omaf set --cpus --memory`. `omaf set` and the VM's details change
sound, microphone, clipboard, camera, MIDI and the window, but not CPU cores, memory or the disk size – a VM keeps what
it was created with (the maintainer's development VM: 4 cores, 8 GB, 75 GB, too small for builds). Cores and memory
are a line in the instance's config (VZ and QEMU read them at start); a larger disk needs the image grown and the
guest's file system too (macOS: APFS behind the Recovery container – OPEN how; NixOS: the overlay and the root file
system). Meanwhile: build output on the VM's own disk – `scripts/bundle-app.sh` and `scripts/sign-runner.sh` take
`SWIFT_SCRATCH_PATH` and `CARGO_TARGET_DIR` (2026-11-22): on the shared folder (VirtioFS) linking the core's test binary
alone took over 40 minutes.

**B163 (2026-11-22): the German keyboard in macOS guests.** The maintainer chose German with the German keyboard and
got "˛" for @ in a macOS guest: `darwin-guest.nix` mapped XKB `de` to "German – Standard" (German-DIN-2137, −18133,
BM24/B46 took it as macOS' default), whose keys differ from the Mac's classic "German" (@ = Option+L, as base.nix
assumes; every other country already had its classic layout). Now `de` → "German" (ID 3, the classic layout number
like French 1, British 2, Italian 4; the name checked in macOS 27's AppleKeyboardLayouts-L.dat). OPEN: checked in a
guest built with it.

**B162 (2026-11-22, built for 0.7.4 – maintainer: "the complete diagnosis apparatus"): diagnostics for a VM's
features, from the UI.** Built as drafted below, with the contract in spec §11.9 ("Diagnosis") and api.md
(`diagnostics`): agent messages `0x0600`/`0x0601` (`nix/agent/src/diagnose.rs`; on NixOS clipboard, sound,
microphone and share run in the user's session process – the system agent has no desktop access), the relay on the
runner's control socket and a new `--control` of `omaf-runner bridge` and `omaf bridge` (`run/bridge.sock`), the
core's job `diagnostics` (`crates/omaflake-core/src/diagnostics.rs`, `api/diagnostics.rs`; the host's clipboard is
saved and restored), the page `diagnostics` in the app, `omaf ui` and the Linux app, the VMs tab's `diagnose` action,
`omaf diagnose`. The maintainer's first diagnosis (2026-11-22, a VM from a 0.7.4 image: agent, SSH with omaflake's
key, share, network, MIDI, disk pass; sound heard) failed only the clipboard – the helper did not run: the new
start check `pgrep -f clipboard-agent-macos.py` at the end of the SSH command put the helper's name into that shell's
own command line, and the helper's start script (`pgrep -f clipboard-agent-macos.py && exit 0`) took the shell for a
running helper and did not start; the check then found the same shell. Now the check is
`pgrep -f 'clipboard-agent-macos[.]py'` (checked with pgrep in a macOS guest), and a test keeps the name out of the
command. With it the clipboard bug the maintainer found: in a macOS guest from an image before 0.7.4 the
clipboard helper never started – omaflake's SSH failed silently (the guest's authorized_keys lacks omaflake's key;
OPEN whether the core service lacks the ssh-agent key) –; the helpers are now started until they run (`pgrep` at the
end of the SSH command), failures are logged once per run, the agent always starts, and the Diagnose page's `ssh`
check offers "Add omaflake's Key …". Checked here: Rust (core 474, agent 37) and Swift (47) tests, clippy for macOS and
aarch64-linux, the agent's checks live in this macOS guest (info, network, clipboard both ways; sound and microphone
timed out on this VM's audio device), the relay end to end with the real agent on a pseudo-terminal through a real
control socket (`omaf-runner diagnose-check`). OPEN: everything in a running guest – a full diagnosis from the app,
sound/microphone (TCC for the SSH-started agent), NixOS session checks, the camera status, `omaf-runner bridge
--control` against a QEMU guest, the page looked at on screen. The draft as noted:
After testing MIDI by hand (MIDI Monitor in the guest; `scripts/miditest.swift`, a workspace tool until 2026-11-27, which copies itself into
the VM over `omaf ssh`, runs there and echoes on the Mac, then writes a log): every feature of a VM should be testable
from the app (and `omaf ui`, the CLI), with its result judged and kept. Draft:
- *One job per run:* `diagnostics.run {vm, checks?}` → Job `diagnostics`, one step per check, each with a result
  `pass` | `fail` | `confirm` (only the user can tell: "did you hear it?") | `skipped` (switch off, guest kind, no
  agent) and its numbers; the run's log in `<data>/logs/diagnostics-<vm>-<time>.log`; "Copy Diagnosis" puts the
  result and the log on the clipboard (also wanted by B160 for the setup).
- *Through the agent, not SSH:* new agent messages (`0x06xx`: test request / answer) so a test needs no SSH and no
  tool copied in; SSH stays the fallback for images without them.
- *Checks:* **agent** (hello, round trip); **clipboard** – a token through the Mac's pasteboard into the guest and
  back (text and a small PNG); **MIDI** – the VM's pair looped by the agent (latency, a 1,000-byte SysEx unchanged,
  timed notes keep their spacing), each device asked for its identity (`miditest identity`); **sound** – the agent
  plays a short tone in the guest → `confirm`, plus the guest's output device and errors; **microphone** – the guest
  records 2 s and reports the level while the user speaks (a meter in the UI); **camera** – the guest opens the camera,
  reports resolution, frame rate and a thumbnail; **share folder** – a file written on one side, read on the other;
  **network** – address, DNS, the internet; **SSH**; **disk** – free space. NixOS and macOS guests, both hosts.
- *UI:* a "Diagnose …" action in the VM's details; a page with one row per check (status, numbers, hint, "Again",
  "Heard it" / "Did not"), the Doctor's style; the result's age shown.
OPEN: everything – the agent messages, how a tone and a recording are made in each guest (CoreAudio; PipeWire/ALSA),
whether a check may play sound without asking.

**B161 (2026-11-22): MIDI in macOS guests.** The maintainer ran Dorico in a macOS guest (Steinberg download and
licenser fine) and asked for the stage piano's MIDI. USB passthrough exists since macOS 27 (`VZUSBPassthroughDevice`
with an `AAUSBAccessory`), but AccessoryAccess needs `com.apple.developer.accessory-access.usb`
(`AAUSBAccessoryManager.h`, macOS 27.2 SDK) – a restricted entitlement, so after M12 – and it captures the device
exclusively. Decided (maintainer): a MIDI bridge over the agent instead, as the default; passthrough later only for
what MIDI does not carry (a piano's USB audio, vendor tools). Implemented:
- *Protocol* (spec §11.9): `0x0500`–`0x0504` – endpoint list, data as Universal MIDI Packets in records
  (`nix/agent/src/ump.rs`, `MIDIRecord` in AgentProtocol.swift, shared vectors "midi"), status, ping/pong.
- *Mac* (`Sources/omaf-runner/MIDI.swift`, `omaf-runner run --midi`): all CoreMIDI endpoints except omaflake's own,
  hot-plug through the client's notifications, one input port per protocol (MIDI 1.0 sources stay 1.0, 2.0 stay 2.0),
  a pair "omaflake <VM>" for the Mac's apps; `AgentLink` checks what the guest sends (whole messages, 1 MiB/s, ids of
  the list) and keeps `midi` (endpoints, failed, round trip) in agent.json; `omaf-runner ctl <socket> midi`.
- *Guest* (`nix/agent/src/midi.rs`): CoreMIDI's C API without new crates – virtual sources and destinations with the
  Mac's name, manufacturer, model, protocol and unique id; a destination's receive block is a hand-made global block
  (Blocks ABI); data played in the guest go out from CoreMIDI's thread under the port's write lock. The agent sets raw
  mode on the macOS port (a tty). In macOS guests the agent offers MIDI only – the clipboard stays with its helper
  until the agent's pasteboard is checked there.
- *Start*: darwin-guest.nix `omaflake-agent-start`, run over SSH with the clipboard helper (`guest_helpers_command`),
  also re-checked at every look (B64) when MIDI is on.
- *Switch*: instance/recipe `midi` (default off) – `omaf new/set --midi`, the VM's switches, New VM (macOS only),
  recipe row; schemas instance v1, recipe v2, catalog v5 got the optional `midi` in place (an additive optional field;
  maintainer, 2026-11-22: stays in place, no new schema versions).
- *Checked here* (a macOS VM without hypervisor, so no guest): Rust and Swift tests, the shared vectors, and
  `omaf-runner midi-check --agent <omaflake-agent>` – the agent on a pseudo-terminal (`OMAFLAKE_AGENT_PORT`), the
  runner's bridge on the other end, test endpoints as the device, CoreMIDI on both sides. With the agent built by Nix
  for aarch64-darwin (links CoreMIDI): 500/500 notes each way, latency min 0.06 · median 0.12 · p99 0.25 ms (no
  virtio, no guest scheduling – a lower bound), a 1,000-byte SysEx and a MIDI 2.0 note (16-bit velocity) identical.
  The macOS recipe `minimal` evaluates with `omaflake-agent-start`.
- *C7 (2026-11-22, maintainer: all of it in 0.7.4):* **NixOS guests** – the agent on the ALSA sequencer
  (`nix/agent/src/midi_alsa.rs`, libasound by FFI, UMP symbols by dlsym; `midi1.rs` UMP ↔ MIDI 1.0; vm-guest.nix loads
  `snd-seq`), on a Mac served by `omaf-runner bridge --midi --vm-name` (the same MIDIBridge), on a Linux host by
  `omaf bridge --midi` (`host_bridge/midi.rs`: the host's sequencer ports, libasound loaded at run time, a client
  "omaflake <VM>"); the switch for every guest kind. **Timestamps** – records' flags bit 0 with the sender's monotonic
  ns, a clock mapper on each receiving side (30 s sliding minimum, +2 ms), scheduled delivery (CoreMIDI time stamps,
  ALSA real-time queue). **UMP endpoints** – the Mac reads MIDIUMPEndpointManager (macOS 15+), the Linux host ALSA's
  UMP info, the guest mirrors them (MIDIUMPMutableEndpoint by the ObjC runtime; the sequencer client's UMP info).
  Spec §11.9, shared vectors with `time`. Checked: Rust (core 463, agent 26) and Swift (42) tests, Linux type-check
  and clippy (aarch64-unknown-linux-gnu), `midi-check` with the new agent (see below), flakes evaluate.
  **Checked by the maintainer (2026-11-22, release 0.7.4, a new macOS VM from a 0.7.4 image with MIDI on):** the
  Mac's stage piano → the guest – notes arrive in MIDI Monitor, so CoreMIDI from the SSH-started agent reaches the
  guest's apps. `miditest vm music` (`scripts/miditest.swift`, 2026-11-22): the agent 0.7.4 with 4 endpoints in the guest, round
  trip 0.64 ms (median of 60), the Scarlett 4i4 mirrored by name; found: the Mac's pair was "omaflake vm" for every
  macOS VM (the runner took the bundle's folder name) – fixed before 0.7.4 is out: `omaf-runner run --vm-name`. **OPEN:**
  everything else with a running guest or Linux MIDI hardware – guest → Mac, Dorico, latency
  with the real piano; the ALSA sides (struct layout, subscriptions, the queue's time base,
  UMP client mode on the guest kernel – `SND_SEQ_UMP` in nixpkgs' config –, the byte order of ALSA's manufacturer id);
  UMP info with a real MIDI 2.0 device (none here); whether apps honour future time stamps on a virtual source.

**B160 (2026-11-22): Setup by one click, user-level, always visible (release 0.7.4).** Found on a freshly reset Mac
(maintainer, 2026-11-21, logs of that Mac): the Nix installer failed four times at "Rename /etc/fstab.tmp to
/etc/fstab: Operation not permitted" – as root, from the administrator dialog – and left a half install (APFS volume
mounted, /nix read-only: every retry failed, only a reboot freed /nix); "Mount the Nix Store" failed the same way;
nix-darwin's first activation failed five times at "ln … /etc/pam.d/sudo_local: Operation not permitted" after
`first_activation_command` had already moved /etc/zshrc, /etc/bashrc and /etc/nix/nix.conf aside – nothing put them
back, so new shells had no Nix (the maintainer added the installer's lines to ~/.zshrc by hand); "Apply Settings" built
before the builder answered (`failed to start SSH connection to 'linux-builder'`, `wait_for_builder` gives up after 2
min without a word); the Doctor's Nix row showed nothing while the installer ran (only the Linux builder row is `busy`
for builder jobs), the root script's output reached the job only at its end, errors came as raw osascript text
("7:104: execution error: …") or empty ("Building the Linux builder host failed:"); QEMU and the SSH key were
"Install …" in Terminal; the macOS base's SSH failed until the user's key was in the ssh-agent (BatchMode, B16); flakes
had to be enabled by hand after a manual Nix install; macos-install.log is replaced by every attempt. Why root may not
write /etc/fstab and /etc/pam.d from the dialog is OPEN – most likely macOS privacy protection (TCC, Full Disk Access
of the app that opens the dialog; nix-installer's own help mentions "the interactive Full Disk Access prompt macOS
requires"). Decisions (maintainer, 2026-11-22):
- *Root once, for Nix only:* `nix-installer install --no-confirm --no-modify-profile --enable-flakes --extra-conf
  "trusted-users = root <user>"` (flags checked with the pinned 2.35.3: `install macos --help`). Nothing else of
  omaflake writes to /etc. `trusted-users` lets omaflake hand its builder to the Nix daemon per call (`--builders`).
- *No nix-darwin for omaflake:* the Linux builder is nixpkgs' `darwin.linux-builder` run as the user – a LaunchAgent
  (~/Library/LaunchAgents) runs its `run-builder` with `KEYS` and the disk in `<data>/builder/`; the builder key is
  generated there (the guest reads `$KEYS` at start, checked in nixos/modules/profiles/nix-builder.nix); builds get
  `--builders "ssh-ng://builder@localhost:31022 aarch64-linux <key> … - <host key>"` (Nix 2.34 takes the port in the
  URL – checked: "connect to host localhost port 31022"; the host key is the public default of nix-builder.nix). Two
  stages as before: nixpkgs' builder from the binary cache, then omaflake's (cores, memory, disk, min-free from
  nix/hosts/builder-vm.nix) built by the first. A Mac whose Nix already has an aarch64-linux builder (nix-darwin,
  /etc/nix/machines) keeps using it.
- *Shell:* a marked block (`# omaflake: Nix` … `# omaflake: end`) in ~/.zshrc and ~/.bash_profile, only when a new
  login shell does not find `nix`; the Doctor shows it and removes it. omaflake itself finds Nix by its paths.
- *Own SSH key* (reverses B16): `<data>/ssh/id_ed25519` without a passphrase, created without Terminal, put into every
  guest and offered first with `-i` (no IdentitiesOnly: bases made before keep working with the user's keys); the
  user's ~/.ssh keys still go into guests as before.
- *Always visible:* every row of a running job is `busy` with the job's last line; root output streams into the job;
  waits end with a message; errors in plain words (Operation not permitted → Full Disk Access with a button to the
  setting; a half Nix install → offer `/nix/nix-installer uninstall`); logs keep earlier attempts.
- *One button:* "Set Up for NixOS" on the Doctor runs Nix → shell → SSH key → QEMU → builder → look as one job with
  "Step n of m"; only the credential is asked. IPSW and macOS base stay their own step.
- *Repair:* an /etc file moved aside by an earlier omaflake (`*.before-nix-darwin` without its original) can be put
  back from the Doctor.
- *No hypervisor* (`kern.hv_support` 0, a Mac VM without nested virtualization): the builder rows say so instead of
  failing silently – such a Mac cannot run the builder or NixOS guests.

*B160 done (2026-11-22, not released – with the MIDI work as 0.7.4):* `linux_builder.rs` (LinuxBuilder: key, two
stages, LaunchAgent `app.omaflake.linux-builder`, `--builders` per call via `guest_arguments_for`, `set_up`/`apply`/
`wait`, hypervisor check), flake output `linuxBuilders.aarch64-darwin.{bootstrap,omaflake}` and
nix/hosts/builder-vm.nix (shared with the nix-darwin host), `shell_setup.rs` (ShellSetup), `ssh_identity.rs`
(SshIdentity), Doctor: item `shell`, nix-darwin no longer checked or a build blocker, Set Up for NixOS (`setup.nixos`,
job `setup` with steps), `builder.uninstallNix`, `builder.trust`, `qemu.install`, `shell.add`/`remove`,
`doctor.createSshKey`, rows busy with the job's `lastLine`; `AdminShell::run_streaming` and `AdminShell::error`;
`BuilderMaintenance::reason`; job logs appended; the CLI's `omaf builder setup|apply|clean-up` on the same code.
Checked: cargo fmt/clippy/test (446 core tests), swift build/test, reuse lint, both flakes evaluate
(`nix flake check --no-build`); `linuxBuilders…bootstrap` needs no Linux build (`nix build --dry-run`: only
run-builder.drv, 587 paths fetched); the installer flags with the pinned 2.35.3; the app's pages in a macOS guest.
OPEN – needs a Mac that can run VMs (this was built in a macOS guest without a hypervisor): the builder's two stages,
the LaunchAgent, a NixOS build through `--builders`, Clean Up as the user; Nix install with the new flags and the
shell block on a fresh Mac; whether macOS' "Operation not permitted" is Full Disk Access (TCC).

**B159 (2026-11-22): `path:` references with a space.** Nix parses a `path:` flake reference as a URL: with the
image flake in a macOS guest's share (`/Volumes/My Shared Files/…`) the Doctor's `flakeEval` failed ("getting status of
…/path:/Volumes/My Shared Files/…") and so would every build. `ImageStore::path_ref` percent-encodes every byte outside
RFC 3986's unreserved, sub-delims, `:`, `@` and `/`; `path_of` reverses it. Used for `flake_ref`, the `omaflake-guest`
and `omaflake-catalog` overrides, the checkout's `omaflake` input and the builder host's `darwinConfigurations`
reference; a plain path (a checkout with its own git repository) needs no encoding. Checked: test
`path_ref_encodes_what_a_url_cannot_hold`; `omaf doctor` in a macOS guest with the checkout in the share – flakeEval ✓
(7 recipes). The Linux builder's build (`builder_maintenance`) is not run there (no hypervisor in the guest) – OPEN.

**Release 0.7.3 (2026-11-04):** B158 – the Doctor's check of an older omaflake QEMU, changed modules and key binding
sets with Reinstall, `omaf new --image` and imported images for `-r`, `omaf ui`'s help scrolling, the look studio's
gradient switch, translated clean-up lines, `VM_FLAKE` builds in the terminal. Unsigned, for the maintainer to try
first – OPEN until then.

**B158 (2026-11-04): OPEN items without a Developer ID.**
- *Doctor and an older omaflake QEMU (B155):* `QemuCapabilities.loading_image` – the binary holds
  `OMAFLAKE_LOADING_IMAGE` (patch qemu-cocoa-loading-image, patchset 6); with 3D but without it the QEMU line is
  limited, "(3D, older patchset)", with **Update …** (`nix profile upgrade qemu`, the profile element is named `qemu`).
  Checked: test `doctor_offers_an_update_of_an_older_omaflake_qemu`; the installed QEMU of 0.7.1 holds the marker.
- *`omaf ui`'s help scrolls (B132):* `Modal::Help { top }`, arrows/j/k/Page/Home/End/g/G, the most it can leave out
  comes from the last draw (`App::help_max_top`). Test `the_help_scrolls_in_a_small_terminal` (100 × 14).
- *Changed parts (B143):* `PartStore::folder_hash` – tools/catalog.py's `content_hash` (path NUL sha256-hex LF, sorted
  by path component, without `.DS_Store`) – and `PartStore::changed`; `modules.list` marks `changed` (badge, info
  line, **Reinstall**, in Update All when a source offers the part). Checked against the real catalog (every freshly
  installed module hashes to its catalog hash; a changed and an extra file are found) and, read-only, against the
  maintainer's data folder: 24 modules, 1 key binding set, 4 looks – none changed.
- *Look studio (B150):* the gradient is a switch (`border.gradient`) – off keeps one colour and hides the second field,
  on without a second colour starts from the urgent one. The macOS colour picker could not choose "none".
- *Clean-up lines:* the Nix Store's and the Linux builder's results become translated texts in the activity
  (`clean_up_text`).
- *`VM_FLAKE` and the service (docs/linux.md):* `omaf build` with `VM_FLAKE` builds in the terminal, not in a running
  service (which has the environment it was started with).
- *An imported image for `omaf new` (B124):* `omaf new --image <image>`; `-r <name>` falls back to an image whose
  recipe is not here (its newest – an import has no current link), with standard values (4 cores, 8 GB, 64 GB, shared –
  the New VM page's for a recipe that is gone). Checked end to end in a temporary data folder: `-r research` and
  `--image image:other/nixos/research:milk` create VMs from an image whose recipe source is another computer's folder.


**State 0.3.0 (2026-10-24):** the core is Rust (B117) and serves every interface through one service per user (B120)
and a presentation model (B121): the SwiftUI app (menu bar, window or both), the Linux app (iced) and `omaf ui`
(B119); the Doctor installs the `omaf` command (B122); the lab VM brings omaflake for trying the Linux interfaces
(B123). Linux as a host is in (B124: QEMU with KVM/TCG, native image builds, a Doctor for Linux, the "SSH port in use"
after a stop fixed; run end to end on aarch64 under TCG; **2026-10-24: run on x86_64 with KVM** – 0.3.1 on Ubuntu 26.04.1, a
Ryzen 9 mini PC, headless: build 300 s, SSH after 15 s, the agent, stop/start; at the GNOME desktop: 3D window only with
Mesa from Nix in QEMU's environment (fixed: B127, omaflake sets it itself), no dynamic resolution (fixed: B128), clipboard host → guest missing on GNOME (fixed: B129), sound and microphone
ok; docs/linux.md). Next, in this order: **(1)** Linux host on real hardware –
x86_64 with KVM on the maintainer's mini PC (docs/linux.md, "Checking on a real machine"), a window with GL and sound on
a Linux desktop (the agent bridge, clipboard, camera and sound on Linux: B126, tried under TCG); **(2)** smaller items – tokens of private
sources on Linux (Secret Service, else a 0600 file), the Linux app avoiding a software Vulkan adapter, translations
for `omaf ui` and the Linux app and German error messages everywhere (B125), the choice between two versions of a recipe after a local edit, an imported image
for `omaf new -r` when its recipe line names another computer's folder; click tests of B119/B121 (macOS app, Linux app on a real desktop). Then M12 once the Developer ID
is there.

**falkOSS starterpack (2026-10-29):** B157 – omaflake follows falkoss/starterpack: the brand kit moved to its own
repository omaflake/brand (generator MIT, artwork CC BY-ND 4.0; `copy.py` writes the copies here, in the website and
the catalog; the design tokens stay here and `brand/tokens.py` now also writes `BrandAssets.swift`); REUSE in every
repository (`REUSE.toml`, `LICENSES/`, SPDX headers, CI), DCO sign-offs checked on pull requests, the shared workflows
of starterpack and every action pinned by commit; omaflake.dev published through Actions (`docs/CNAME` gone – the
maintainer switches Settings → Pages → Source to GitHub Actions); `.well-known/security.txt` on omaflake.app; the
organisation's community files in omaflake/.github; Rust 2024 with `rust-toolchain.toml` (1.99.1), exact dependency
versions (`scripts/check-age.py`), `cargo deny`. Deviations and their reasons: CLAUDE.md, "falkOSS starterpack".

**Release 0.7.2 (2026-10-29):** B156 – modules, key binding sets and looks in the data folder that this omaflake cannot
read (installed before 0.7.0) are rows with Update when a source offers them, and Update All updates every one; the
Key Bindings tab shows only its own warnings. Unsigned, for the maintainer to try first – OPEN until then.

**Release 0.7.1 (2026-10-29):** B155 – NixOS guests show omaflake's loading screen from the window's first frame (QEMU
patches `qemu-virgl-2d-flush` and `qemu-cocoa-loading-image`, patchset 6; omaflake's own Plymouth theme;
`plymouth.ignore-serial-consoles`), no wait in the firmware or the boot menu – the desktop at about 11 s instead of
about 20 s of black. The Doctor does not see an older omaflake QEMU (it checks 3D only): the changelog tells users to
run `nix profile upgrade qemu` – the Doctor checks the patchset since B158. Open: the 2 s of black between Plymouth and
Hyprland. Unsigned, for the maintainer to try first – OPEN until then.

**Release 0.7.0 (2026-10-28):** B146–B154 – the IPSW after a macOS update and its download (B146); key bindings with
modes, the keys view, the four groups, the recipe and key binding studios (B147); key caps, the help switch, the menu
bar for running things, the studios as kits with search, chips and pages, a key editor with recording, the catalog tabs
browsing the same way (B148); the Nix Store kept small and cleaned up from the Doctor (B149); the look studio (B150);
the Dock icon with the app window, "Regionen" in German (B151); header and sidebar always there (B152); any number of
key binding sets, a sidebar that never scrolls (B153); the review before the release (B154). Catalog v5 only – the
official catalog (omaflake/catalog, its module v2 commit) and the website's catalog page must be pushed with it.
Unsigned, for the maintainer to try first – OPEN until then.

**Release 0.6.2 (2026-10-26):** the fixes of B145 – the recipe list without Nix (installed recipes stay installed
while Nix is unavailable), QEMU with 3D before the first on PATH, the Doctor's Mount … for a Nix Store that is not at
/nix –, and a recipe's build button greyed out with the Doctor's sign instead of a Doctor button. Unsigned, for the
maintainer to try first (a restart with the Nix Store under /Volumes, Mount …, builds and VMs) – OPEN until then.

**Release 0.6.1 (2026-10-25):** the fix of B143 – parallel installs mixed up the installed modules (each install has its
own work folder now) –, with the Modules tab's rows without a colour frame, "Create macOS Base …" once, the base being
created shown under macOS Bases, and "Install All" for recipes, modules and looks. Checked by the maintainer before
the release: with the modules reinstalled, both NixOS recipes and the macOS recipe built and ran as VMs. Unsigned, for
the maintainer to try. Open after it: the new UI clicked in the app (B143), the ideas of B144.

**Release 0.6.0 (2026-10-25):** recipes as building blocks (B141, B142): recipe v2 is data – a platform, modules,
a suggested look, key bindings, requirements, how a VM runs; the first-party modules and key bindings in omaflake/catalog
(catalog v4); installing a recipe brings its parts; the Modules tab, Profiles, My Recipes (the recipe editor), what a
recipe is made of on its page and the build page; locales (language, time zone, one to three keyboards) and the host
keyboard; installable looks with the Looks tab (B140, B141); the website's catalog page. Settings v5, image.json v3;
recipe v1 and catalogs before v4 are not read (no compatibility before 1.0). Unsigned, for the maintainer to try. Open
after it: the other new recipes built (emacs-minimal, neovim-research, lab, macOS), the new pages clicked in the
SwiftUI and iced apps, a macOS guest with several keyboards (BM24).

**Release 0.5.2 (2026-10-25):** credentials and the image as recipe + credential + look (B137), the look and credential
shown at images and VMs, outdated right after a change of what an image was built with (B136); credentials as the
other lists with Copy Password (Touch ID), locked while used, always with a password; macOS bases in the images tab;
settings "For new images"; one omaflake line in the Doctor (B138). Settings v3, image.json v2. The 0.5.1 draft (B137
only) was discarded after the maintainer tried it; 0.5.2 replaces it. Unsigned, for the maintainer to try. Open after
it: the Touch ID prompt from the core service and the CLI, clicking it in the SwiftUI and iced apps, a macOS build
with a stored password.

**Release 0.5.0 (2026-10-25):** looks for the guests (B135) – omaflake's own as the standard, Catppuccin Mocha, Rosé
Pine Dawn and Vanilla, chosen by the recipe or the user's guest setting (config.json v2); Zellij at a key with keys
that leave Neovim alone, moving and resizing windows by keyboard, every key starting with Cmd or Ctrl (B133, B134);
`omaf ui`'s action menu scrolls (B132). Unsigned, for the maintainer to try the looks in the app. Open after it: each
look seen in a running VM (one omaflake-look VM looked at on the Mac), VS Code in macos/research, Emacs' Meta keys
under lv3:alt_switch.

**Release 0.4.0 (2026-10-24):** the Linux host checked on real x86_64 hardware and fixed where it failed – OpenGL for
QEMU from Nix outside NixOS (B127), the guest following the window (B128), the clipboard on GNOME (B129), the guest
camera (B130), the image flake in the Linux package (B131), the official NixOS recipes for x86_64-linux too. Unsigned.
Open after it: environment variables not reaching the core service, a locked GNOME keyring within 60 s, the Mac's
resizing with Hyprland 0.56 not re-checked, M12.

**Release 0.3.1 (2026-10-24):** the Linux host (B124, B126) and the smaller items below, unsigned, for the maintainer to
try on different hardware (Macs, the x86_64 mini PC with KVM). CI now builds the Linux package for x86_64-linux and
runs the Rust tests on Linux (job `linux`); aarch64-linux is still built only in the lab VM.

**B127 (2026-10-24): OpenGL for QEMU from Nix outside NixOS.** `nix_gl.rs`: QEMU in the Nix store and no
`/run/opengl-driver` → Mesa from QEMU's nixpkgs (the profile manifest's locked URL of the element holding QEMU, else
`nixpkgs`), `nix build --out-link <data>/cache/gl/<QEMU store name>`, environment for glvnd/DRI/GBM on a `--window`
start with 3D; a note in qemu.log. Checked on Ubuntu 26.04 (GNOME 50): the window comes up with 3D without anything set
by hand. Open: the Linux app (wgpu) under Nix outside NixOS – not looked at.

**B128 (2026-10-24): the guest follows the window's size on a Linux host.** QEMU's GTK UI reports the window size
(`gd_set_ui_size`), the guest kernel offers it as the preferred mode, but Hyprland 0.56 (aquamarine) keeps the mode
of an output that stays connected ("Skipping connector Virtual-1, has crtc … and is connected"). `nix/home/default.nix`:
`omaflake-follow-display` (started with Hyprland) watches DRM change events (`udevadm monitor`) and applies the
preferred mode with `hyprctl eval "hl.monitor({…})"` (`hyprctl keyword` refuses the Lua config). Checked on the mini
PC: 1052×619 after a drag. On a Mac nothing changes where the mode already matches; the Mac's resize not re-checked
with Hyprland 0.56 (OPEN).

**B128, continued (2026-10-24):** with `scale = "auto"` in the helper, Hyprland chose scale 2 from 1920×1080 on
(measured: 1920×1080, 2560×1080 and 3840×2160 all gave 2 – QEMU's GTK UI leaves the EDID without a physical size), so
a big window showed a small picture with huge text; the helper now keeps the output's current scale. swaybg kept its
first size after a mode change (half of the window black): the plain background is Hyprland's own
`misc.background_color` now, swaybg only for a wallpaper. Checked by the maintainer: dragging and full screen.

**B130 (2026-10-24): the guest camera's first frame was minutes old.** Frames reached `/dev/video42` (151 in 10 s read
with `dd`, the test pattern's bars), but mpv and ffmpeg (mmap streaming) got one black frame and stopped or waited:
v4l2loopback hands a new reader the buffer written last before its STREAMON, stamped when written – the black frame
the agent wrote when the previous reader closed (`v4l2-ctl --stream-mmap --verbose`: delta 27758 ms). Writing a fresh
one on open loses the race (the reader streams within the same millisecond). `nix/agent`: `Camera::keep_fresh`
rewrites the black frame every second while no application reads. Checked on the mini PC: first delta ≈1.5 s,
ffmpeg 71 frames in 5 s. On a Mac the same agent runs (not re-checked there, OPEN).

**B131 (2026-10-24): the image flake in the Linux package.** linux/package.nix installs `nix/` (without `local/` and
`agent/target/`) to `<prefix>/share/omaflake/nix`; `Settings::bundled_flake` looks there next to `Resources/nix`.
Checked: a fresh data folder's Doctor names it, `omaf build neovim-minimal` builds an x86_64 image with it.

**B132 (2026-10-25): `omaf ui`'s action menu did not scroll.** A running VM's row has eight actions, most with a second
line (help or reason); taller than the terminal, the menu was cut at its bottom and "Remove …" – the last – could be
selected but not seen (found over RDP on the mini PC; `1`–`9` reach only the first nine). `ui.rs`: a modal leaves out
lines at its top until the selected entry fits (`modal_scroll`, for the action menu and a field's choices), and its
frame shows `↑`/`↓` where lines are left out. Test `the_action_menu_scrolls_to_its_selection_in_a_small_terminal`
(100 × 14). The help (`?`) scrolls since B158.

**B133 (2026-10-25): Zellij in the neovim line by itself, with keys that leave Neovim alone.** `nix/home/neovim.nix`:
WezTerm's `default_prog` is `zellij attach --create main` (as the macOS research recipe's wezterm.lua), the zsh alias
`zj` attaches from an SSH shell; the zsh integration stays off, so SSH and the console stay plain. Zellij's default keys
take Ctrl o/n/p/s/h/t before Neovim sees them; `home/zellij-unlock-first.kdl` is Zellij's own "Unlock First" preset
(0.45.1's `presets.rs`, Ctrl as its configuration screen sets it, MIT) as `programs.zellij.extraConfig` – without its
Alt shortcuts: the guests' `lv3:alt_switch` makes both Option keys level 3, no Alt arrives (maintainer's decision: all
through Ctrl g and the modes; moving tabs, swap layouts and pane groups go with them).
Checked: the configuration evaluates (wezterm.lua with the store path, the alias), `zellij setup --check` on the
generated config.kdl: "Well defined". Not yet in a running VM (OPEN).
Changed after a look at the VM (maintainer, 2026-10-25): not every window – a plain terminal stays free for zathura
and the like. WezTerm has no `default_prog` any more; SUPER + SHIFT + Enter (Hyprland, `home/neovim.nix`) opens
`wezterm start -- zellij attach --create main`, `zj` stays. macos/research: the same as a WezTerm key
(`CMD|SHIFT` Enter → `SpawnCommandInNewWindow`, with the look's `options`) and `environment.shellAliases.zj`.

**B134 (2026-10-25): moving and resizing windows in Hyprland by keyboard.** `nix/home/default.nix`: SUPER + SHIFT +
arrows → `hl.dsp.window.move({ direction })`; SUPER + R → submap `resize` (`hl.define_submap`): arrows and h/j/k/l
→ `hl.dsp.window.resize({ x, y, relative = true })` in 40 px steps, `repeating`; escape/Return → `submap("reset")`.
Argument shapes from Hyprland 0.56.2's `LuaBindingsDispatchers.cpp` (the stubs only say `fun(...)`). Checked: the
generated hyprland.lua with `Hyprland --verify-config` (0.56.2, same nixpkgs): "config ok". Not yet pressed in a
VM, nor whether the Mac lets Cmd + Shift + arrows through to the guest (OPEN).

**B135 (2026-10-25): looks for the guests, and a rule for keys.** The maintainer's decisions: an own omaflake look –
black, high contrast, the hero's red/cyan split only as fine accents – becomes the official recipes' standard; Catppuccin
Mocha (the look so far), Rosé Pine Dawn (light, mild, no relation to the brand) and Vanilla (each program's own
colours) stay choosable. A recipe picks the look like its editor line (`omaflake.look`), the user's guest setting
wins over it. macOS guests get no system-wide look, only their terminal's and editors' colours, in step with NixOS.
Every key omaflake binds starts with SUPER/Cmd or Ctrl (optionally Shift); plain keys only inside a mode.
- **Nix:** `nix/looks/<look>.nix` – one source per program (`desktop` colours for NixOS; `wezterm`, `zellij`, `nvim`,
  `emacs` for both kinds), `looks/default.nix` lists them. `modules/look.nix` (NixOS and nix-darwin): option
  `omaflake.look` (enum, default `omaflake`), `settings.look` over it with `mkOverride 40` (checked: wins over a
  recipe's value), read-only `omaflake.lookColors` for recipes (neovim-research's zathura), and the files
  `/etc/omaflake/look/{name,wezterm.lua,nvim.lua,nvim/pack/look/opt/*,emacs.el,zellij/}`. `home/look.nix`: Hyprland
  borders and background, Waybar, Fuzzel, Mako, Starship, BAT_THEME/fzf, GTK (adw-gtk3, dconf `color-scheme`; dconf
  enabled in modules/hyprland.nix) and the pointer (Bibata); WezTerm merges `wezterm.lua`'s `config`, Zellij gets
  `theme` + `theme_dir`; Neovim `dofile`s `nvim.lua` (it `packadd`s the theme plugin and returns lualine's theme),
  Emacs `load`s `emacs.el`. The omaflake look's colours are `[look]` in `brand/tokens.toml`, written by `tokens.py` to
  `nix/looks/omaflake-palette.nix` (`--check` in CI). Neovim: modus-themes.nvim (Modus Vivendi) with `on_colors`;
  Emacs: its own Modus Vivendi with `modus-vivendi-palette-overrides`; Rosé Pine: rose-pine nvim, an own small Emacs
  theme (nixpkgs has none); Zellij themes in its older palette format. Key rule: Print → SUPER + SHIFT + S.
- **Recipes** (omaflake/recipes): `macos/research` – `wezterm.lua` merges the look's file and starts Zellij (on
  Cmd + Shift + Enter, B133) with `options --theme-dir … --theme …` (falls back to its old colours without the file), `init.lua` `dofile`s
  `nvim.lua`; `nixos/neovim-research` colours zathura from `omaflake.lookColors`.
- **Setting:** config.json v2 (`guest.look`, `docs/schema/settings/v2.json`, step `settings_v1_to_v2` without
  changes), `GuestOverrides`/`GuestSettings.look` (left out when unset; the flake reads `ownSettings.look or null`),
  `GuestChoices::LOOKS` (test: the image flake's `names`), API `guest.look` and `choices.looks` (additive), settings
  page field "Look" with "Each recipe's own" as null, `omaf config --guest-look` (an unknown look is refused), Swift
  `GuestValues.look`, `Choices.looks` (optional), German texts.
- **Checked:** all four looks evaluate for `neovim-minimal`, `neovim-research` and the macOS `research`; Hyprland
  0.56.2 `--verify-config` "config ok" for each look's hyprland.lua (on the Linux builder); `zellij setup --check` with
  each look's config and theme folder (a broken theme is caught); headless Neovim loads each `nvim.lua` (omaflake
  #000000/#ffffff, Rosé Pine #faf4ed, lualine takes each theme); batch Emacs 31 loads each `emacs.el` (the Modus
  overrides take: cyan #00E5FF, red #FF3B3B); every generated Lua file parses; research's wezterm.lua with and
  without the look file (a stub `wezterm` module); `omaf config --guest-look` with a temporary `VM_HOME` (v1 → v2);
  `cargo test`, `swift test`. **Not yet seen in a running VM (OPEN):** the Waybar CSS (text-shadow), GTK and the
  pointer, Mako, how each look looks; VS Code's colours in `macos/research` do not follow the look yet (OPEN).

**B136 (2026-10-25): the look at images and VMs, outdated right after a guest change.** Asked after trying 0.5.0: a
changed look needs a new image, but nothing showed which look an image has, and "outdated" came only at the next
freshness check. `ImageInfo.look` (image.json v2, `image_v1_to_v2` without changes, `docs/schema/image/v2.json`) is set
after a build from `ImageStore::look_of` – `nix eval` of `config.omaflake.look` (NixOS `nixosConfigurations."vm-<system>-
<recipe>"`, macOS `darwinConfigurations."<recipe>"`) with the guest arguments; None when the flake has no looks. API
`ImageDto.look`; image details and VM details (through its image) get "Look" (`images::look_line`), with a warning
note "<look> in the settings – a new build brings it" when the guest setting asks for another one, "unknown – built
before looks" without one; `omaf images` prints it. `settings.set` runs `check_freshness` when `guest` changed. The
outdated badge and status name the recipe or the guest settings as the reason. Swift `VmImage.look`. Checked:
`cargo test` (VM detail: the look line and its note; a macOS VM's image from before looks), goldens, `swift test`.
Not yet run: a real build writing the look (the eval it runs was checked by hand), the app after a look change.

**B137 (2026-10-25): credentials; an image = recipe + credential + look.** The maintainer's decisions: several named
credentials (create, edit, remove) on a page of their own (a key in the header); building an image always asks its
credential and look; the settings only hold the default credential and the default look; images and VMs show both
next to the recipe; a credential may keep a password so macOS images build without asking. Mine, to review: a
credential is user, full name, e-mail (+ password) – locale, keyboard and time zone stay global; without one an image
gets this computer's values ("This computer"); the password is the macOS base user's (sudo in the build VM) – NixOS
guests get none; German "Zugang"/"Zugänge".
- **Core:** `credentials.rs` – `Credential {id, name, user?, fullName?, email?}` in config.json (`credentials`,
  `defaultCredential`, settings v3 – `settings_v2_to_v3` turns a set guest user/fullName/email into a credential that
  becomes the default), `Credentials` (list/find/add/update/remove/set_default), the password in the secret store under
  `credential:<id>` (`Tokens::{get,has,set}_account`, the sources' store: Keychain, Secret Service, else the 0600
  file). IDs are random ("c-" + 12 hex): the secret store is the computer's, shared by data folders and tests – a
  counter would let them reach each other's passwords. `guest_settings::BuildChoice {credential, look}`
  (`defaults`, `of_image`, `key`), `GuestSettings::for_choice`, `write(paths, choice)` → `guest/<credential>-<look>/`,
  `ImageStore::guest_arguments_for`; `build`, `expected_store_path`, `look_of`, the macOS `BuildRequest` take a choice.
  image.json v2 (unreleased, extended): `lookChoice`, `credential`, `credentialName`; the freshness check evaluates
  each recipe with its current image's own choice (an image whose credential is gone is not checked).
- **API:** `credentials.list/add/update/remove`, `settings.set defaultCredential`, `images.build credential/look`
  (a macOS password: the given one, else the credential's), `ImageDto.credential/credentialName`,
  `SettingsDto.defaultCredential`; pages: `credentials` (new; home action `credentials`, icon `key`), `build.fields`
  (credential, look; the password asked only without a stored one), image/VM detail lines Credential and Look, list
  subtitles "recipe · credential · look"; settings: Default credential, Default look (user/fullName/email gone).
- **CLI:** `omaf credentials [list|add|edit|rm|default]`, `omaf build --credential --look`, `omaf config
  --default-credential` (replaces `--guest-user`, `--guest-full-name`), `omaf images` prints Credential.
- **UIs:** TUI (page, build fields, icon), iced (page, build fields, form values, Lucide KeyRound), SwiftUI
  (`CredentialsPage`, `CredentialsView` with an actions menu per row, `BuildView` fields, SF Symbol `key`).
- **Checked:** `cargo test` (credentials with a fake store: add/update/remove, passwords, unique names, defaults;
  settings v1/v2 → v3; build page: fields, params, no password ask with a stored one, the ask with "This computer";
  settings fields; home action; VM/image lines), golden files, `swift test`, TUI draws the credentials page and the
  build fields, CLI in a temporary data folder. OPEN: clicking it in the SwiftUI and iced apps; a macOS build with a
  stored password.

**B138 (2026-10-25): credentials as the other lists; bases; settings "For new images"; one omaflake line in the
Doctor.** The maintainer's decisions after trying 0.5.1: a credential can be edited fully only while no image uses it,
else only renamed (the other fields greyed out with the reason) and not removed; every field explains what it is for
(the user is NixOS's login, home and `omaf ssh`; macOS keeps the base's user); rows like VMs/images/recipes – name and
usage, Copy Password as the quick icon (always, after Touch ID / the login password), the rest when opened; every
credential has a password; the macOS bases as a section of the images tab, removable; the settings' guest section
says it is for new images only; the Doctor's "omaflake" and "Core" become one line with both versions, with the
update below.
- **Core:** `credentials::{confirm_owner, copy_to_clipboard}` (LAContext `DeviceOwnerAuthentication` via
  `objc2-local-authentication`; `pbcopy`/`wl-copy`/`xclip`, cleared after 90 s if unchanged), `add` needs a password,
  `update` refuses to clear it; API `credentials.copyPassword`, `bases.list/remove`, `credentials.update/remove`
  refused while used; pages `credentials` (rows with lamp, usedBy, detail), `credential` (form, `locked`), `images.bases`
  (Mac only), settings section "For new images", Doctor `versions` from the client's `initialize` version (per
  connection: the subscriptions' contexts are built per language and client version).
- **CLI:** `omaf credentials copy-password`, `add` always asks the password, `--no-password` gone, `edit`/`rm` refused
  while used. **UIs:** TUI, iced and SwiftUI render the expandable rows, the form, the bases and the Doctor block.
- **Checked:** `cargo test` (credentials rows/lock, bases section, golden files incl. `credential`, `credential.edit`),
  TUI and iced tests, `swift test` (view contracts for the new pages). OPEN: clicking it in the SwiftUI and iced apps.
- **Found in the 0.5.2 draft, fixed:** "New Credential …" showed a loading page forever – `views::PAGES` lacked
  `credential`, so `views.get` refused it (the goldens built the page directly); `every_shown_page_is_served` now
  checks every `ui.show` target of the view goldens. Texts said "1 images" – the counts left out of them.
- **Checked by hand (temporary data folder, `omaf call --socket`):** add (refused without a password), the rows,
  the form new/edit, rename, clearing the password refused; with an image built with the credential the form is
  locked to the name, update of the user and remove refused (API and CLI); `credentials.copyPassword` through the
  service shows the Touch ID / password prompt, copies, the clipboard is empty after 90 s (unanswered, the call waits
  up to 5 minutes and says "not confirmed"); Doctor "App 0.5.2 · Core 0.5.2"; the bases section; "For new images".

**B139 (2026-10-25): a macOS base is created from a credential; no password is ever typed; the Doctor requires a
credential.** The maintainer's decisions: a base's guest user comes from a credential (user, full name, password from
the keychain), the base shows it, macOS builds take it – "never a manual password: you set up the credentials, from
then on omaflake does everything"; the Doctor requires at least one credential (and so a default); the host's user
name only prefills the first credential, after that the host has nothing to do with credentials ("This computer" is
gone).
- **Core:** `mac_base_installer::BaseRecord` – `omaflake-base.json` in the base folder (schema `base` v1: credential,
  credentialName, user), written when an install finishes, or by `assign` into a finished, read-only base (made
  writable for it, then read-only again); `BaseInfo.record`. `credentials::{Login, Usage}`:
  `Credentials::{login, base_login, login_of_base, assign_to_base}` (the base's user must be the credential's),
  `Usage::{of, check_change, check_remove}` (images and bases; a base locks user, full name, e-mail and password).
  `default_credential()` falls back to the first credential, `remove` moves the default on, `set_default` takes an id,
  `add` needs a user; `GuestSettings::for_choice` takes the credential's user as its full name when it has none.
  Prerequisite `credential` (Requirements; a build blocker for both guest kinds).
- **API:** `macos.installBase {credential?, ipsw?}` (user/fullName/password still accepted), `bases.assign`,
  `BaseDto.credential/credentialName`, `images.build` for macOS: the base's credential and password (a `password`
  only for a base without one), no `""` = this computer; `settings.set defaultCredential` needs an id. Pages:
  `baseCredential` (the credential form's shape), `macBase` (a credential choice, no password ask), `build` (macOS:
  the base's credential, fixed; "Assign Credential …" for a base without one; never a password ask), bases rows
  (credential, badge `no-credential`, action `assign`), credentials rows (usage incl. bases), settings (no "This
  computer"), first-credential prefill.
- **CLI:** `omaf macos install --credential`, `omaf macos assign`, `omaf build` without `--password-stdin`, `omaf
  credentials add` needs `--user` after the first, `default computer` gone. **UIs:** TUI, iced and SwiftUI route
  `baseCredential` to the credential form; SwiftUI's macBase page renders its fields generically.
- **Checked:** `cargo test` (base record, login, assign with a user check, a read-only base stays read-only, usage
  locks; pages; golden `baseCredential`), TUI and iced tests, `swift test`. OPEN: a real base installed from a
  credential and a macOS build without a password prompt (needs a new base: ~30 minutes); assigning the credential to
  `mac-base-26A434` on the maintainer's Mac.

**B140 (2026-10-25, decided, not built): one catalog for recipes, looks and later kinds; looks as data.** The
maintainer's decisions after discussing a themes repository:
- **Looks become data, not Nix code:** a look is a `look.toml` (schema `look` v1) – palette (background, surfaces,
  text, accents, the 16 terminal colours) and a few choices (editor themes, cursor, GTK theme, border style such as a
  two-colour gradient; Vanilla: "set no colours"). The image flake keeps the modules that turn a look into Hyprland,
  Waybar, WezTerm, Zellij, Neovim, Emacs, GTK … configuration (`nix/home/looks/`); the schema is the only interface, so
  the modules can change without breaking a look, and a look runs no code. omaflake writes the chosen look into the
  guest folder at build time (like the credential), the flake reads it. The omaflake look and Vanilla stay built into
  the image flake (a build never depends on a reachable source); Catppuccin Mocha and Rosé Pine Dawn move to the
  catalog. OPEN: which fields the omaflake look needs (the red–cyan border, Waybar's underline) without the schema
  becoming a second configuration language.
- **Not a recipe variant:** looks are orthogonal to recipes (any look with any recipe, chosen at the build); a recipe
  still names its default look (`omaflake.look`).
- **One catalog system for several kinds:** a source is a repository with `catalog.json`; catalog v3 lists recipes and
  looks (open for a third kind). Sources may offer one kind or several; official vs. "at your own risk", private
  sources, versions, updates, install/remove work for every kind. Layout: `recipes/<author>/<kind>/<name>/recipe.toml`
  as today, `looks/<author>/<name>/look.toml`.
- **Repository `omaflake/catalog`** (new, MIT) replaces `omaflake/recipes` as the official source; `omaflake/recipes`
  is archived (read-only, marked deprecated) and deleted with the stable 1.0 release. config.json gets a migration
  step that points the official source to the new repository. Named "catalog" because the file (`catalog.json`), the
  code (`RecipeCatalog`) and the docs already use the word ("library" would clash with `RecipeLibrary`, the locally
  installed recipes).
- **App:** a tab "Looks" next to "Recipes" – installed and offered looks, install/remove, usage (the images built with
  a look; removing is refused while one uses it); macOS guests get the same palette for terminal and editor.
- **Defaults where they live (2026-10-25, later the same day):** an image is a recipe + a look + a credential. The
  Doctor requires at least one installed look as it requires a credential (the built-in omaflake look counts), and
  there is always a default look as there is always a default credential. The default credential is set on the
  Credentials page, the default look in the Looks tab ("Make Default") – the settings' "For new images" keeps only
  locale, keyboard and time zone.
- **Website:** one catalog page with two clearly separate catalogs, Recipes and Looks, sharing the same search,
  filters and widgets (looks: appearance, tags, colour swatches from `swatches`); replaces omaflake.app/recipes/.
- **German:** "Look" stays "Look" (plural "Looks") – short, already used in the app, the same word in code and docs.
- **Started (2026-10-25):** `../catalog` – one commit without history (the maintainer's wish), recipes as before,
  `looks/omaflake/{catppuccin,rose-pine-dawn}/look.toml`, `tools/catalog.py` writing catalog v3 with looks, the
  docs (README, CONTRIBUTING "Contributing a look", REVIEW "Looks", FORKING); schemas `look/v1.json` and
  `catalog/v3.json` here (both validate the two looks and the catalog). Not pushed: the repository is created by
  the maintainer. OPEN: the Catppuccin terminal colours against Catppuccin's own ports; omaflake does not read
  catalog v3 or looks yet.
- **Steps:** (1) the maintainer creates the empty repository `omaflake/catalog` (the agent token cannot create
  repositories), (2) the recipes move over with their history, `tools/catalog.py` learns kinds, (3) `look.toml` v1 and
  the four looks as data, the image flake reads them, (4) catalog v3 and the "Looks" tab in core, CLI and the three UIs,
  (5) settings migration to the new official source, docs and website, (6) the maintainer archives `omaflake/recipes`. Done 2026-11-27: `omaflake/recipes` archived and then deleted, before 1.0 (nothing used it: omaflake migrates settings naming it since 0.6.0; it is no longer an official source).

**B141 (2026-10-25, decided; built in B142): recipes as building blocks – a recipe is data: base + modules + look + key
bindings.** The maintainer's decisions (from a German draft with every recipe of then mapped, kept in the private workspace until
2026-11-27; what was decided is here):
- **Guiding rule: the abstraction must not sprawl** – otherwise the building blocks are not worth it and one writes
  whole modules instead. So: a short, fixed list of capabilities; "needs" and "enhances" only; warnings, never refusals;
  no conditions, no plugin system for checks. Versions only as below (none = latest, exact, minimum and/or maximum).
- **Decided later the same day:**
  - **Neovim and Emacs together are reported as incompatible** (both `default-editor`): nobody would work like that,
    and trying both is a second VM. Reconsidered only if the community asks for it explicitly.
  - **Versions of references:** a recipe names no version (the newest of a module, look or key binding set), an exact
    version, or a minimum and/or a maximum (as npm's dependencies) – no other range syntax.
  - **Where references point:** only to the catalogs of the sources set up and to what is installed locally – the
    official catalog always as the fallback and the default. A recipe never loads modules from a source by itself:
    a module of another source must be installed (its source added) by hand first.
  - **Key bindings:** each module declares its actions that can be bound, each with a default key – a module works
    without any key binding set. A key binding set overrides keys of actions of the modules it names. A recipe may
    combine several sets (e.g. a general one and a special one for one module); later ones win for the same action.
    The recipe is checked for overlapping keys. **Modes are part of the abstraction:** a binding has a context – the
    whole desktop, or a program (a module) – and optionally a mode of that module; two bindings overlap only when both
    can be active at once: same context and mode; a desktop-wide one with any unmoded one; an outer program's unmoded
    one with an inner program's (a module says what it runs inside, e.g. Zellij and Neovim inside the terminal).
    Ctrl+P in Zellij's pane mode does not overlap Ctrl+P in Neovim.
  - **macOS in the looks:** a look covers macOS guests' system appearance too – light or dark from `appearance`, the
    rest from an optional `[macos]` (accent, icons, folders, wallpaper); without it graphite and clear icons for a dark
    look, macOS' own for a light one. Only values checked in a guest (BM22) for now: accent multicolor | graphite,
    icons default | clear, folders default | graphite, wallpaper default | black. Done 2026-10-25: schema look/v1,
    tools/catalog.py, `adapter.nix` (`macos`), option `omaflake.lookMacos`, the catalog's `macos/minimal` takes it
    (evaluated with all four looks: omaflake and Catppuccin dark/graphite/clear/black as before, Rosé Pine Dawn and
    Vanilla leave macOS' own). OPEN: the other accent and folder colours, clear icons in a light look and a wallpaper
    in any colour – read in a macOS 27 guest first, then added to the schema; an image built and looked at.
  - **First-party modules live in the catalog** (omaflake/catalog `modules/omaflake/…`); the image flake keeps only
    the bases and turns looks into configuration.
  - **Backward compatibility does not matter before the stable 1.0** (see CLAUDE.md, File versions): formats, API
    params and migrations may change or go without a transition until then.
- **A recipe is always data** (`recipe.toml` v2): `platform` (nixos | macos), `modules`, `look`, `keymap`, `requirements`
  (minimum cpus, memory, disk – a smaller profile is warned about, still allowed), `vm` (display, network, share,
  ephemeral, devices). No Nix in a recipe: an author who needs Nix writes a module. One click installs a recipe with
  its modules, look and key bindings – a working, opinionated image; recipes are safe to share, only bases and
  modules need review. Users can put recipes together in the app (a local source "My recipes").
- **Platforms** (German "Plattform"; first called bases – the word is the installed macOS base's) stay in the image
  flake (omaflake's guest modules: user, locale, SSH, Nix, agent, share); the platform sets the host name from the
  recipe's name. Decided: "platform" – "core"/"Kern" is the core service's name.
- **Modules** ("Modul", not "add-on": sounds commercial) are Nix, reviewed, in the catalog
  (`modules/<author>/<kind>/<name>/` with `module.toml`): `provides` capabilities, `needs` capabilities, `enhances`
  concrete modules (an integration only that module allows: SyncTeX is zathura's, vimtex neovim's). A recipe missing a
  needed capability or holding two providers of an exclusive one is warned about (the app suggests providers); it can
  still be saved and built. Capabilities: a fixed list in the catalog (`capabilities.toml`, like `tags.toml`), each with
  its meaning and promise – first: `desktop` (exclusive), `terminal`, `multiplexer`, `shell` (exclusive), `editor`,
  `default-editor` (exclusive, sets `$EDITOR`), `pdf-viewer` (the default for PDFs, reloads a changed file). The first
  party modules cut from today's image flake and recipes: NixOS hyprland, shell, wezterm, zellij, neovim, emacs,
  zathura, latex, typst, python-data, pandoc, omaflake-dev; macOS desktop, shell, wezterm, zellij, neovim, latex,
  typst, vscode, homebrew, pandoc, lazygit, omaflake-dev.
- **Key bindings** are data (`keymaps/<author>/<name>/keymap.toml`): each module names its actions, a key binding set
  maps them to keys and says which modules it serves (one set may serve several); the catalog checks double keys and
  the key rule. First: `omaflake/default` (today's keys).
- **Looks:** none is built in – every look, also omaflake's and Vanilla, is in the catalog and can be installed and
  removed; the image flake only turns a look into configuration. The look schema covers the first-party modules and is
  open for others (a look may carry a section per module, its schema the module's). The Doctor reports "no look" and
  recommends omaflake and Catppuccin; there is always a default look while one is installed (decided in B140).
- **Profiles** (CPU, RAM) are local like credentials, chosen when a VM is created; not in the catalog.
- **Locales** (decided 2026-10-25): the host's keyboard is a global setting (the physical keyboard – the default
  for new locales). A locale is the user's, created and named like credentials (local only, never in a catalog): one
  system language with region, one time zone, and one or more guest keyboards (e.g. German and Russian; the first is
  the default, switching is a key binding action – Super+Space on NixOS, as XKB's `grp:win_space_toggle`; macOS keeps
  several input sources in `AppleEnabledInputSources`, so far checked with one – BM24, OPEN with several). It is an
  ingredient of an image chosen at the build (default: the default locale), not part of a recipe – but a recipe may
  restrict locales (e.g. `rtl = false`, or an allow-list of languages) and a choice outside gets a warning, the build
  stays possible.
- **An image** is a recipe built with a look, key bindings, a credential and a locale; a VM is an image started with a profile.
- **Order:** (1) looks as data in the image flake (adapter from look data to today's per-program files), (2) the core
  reads catalog v3 and installs looks, the Looks tab, the Doctor's look requirement, defaults in their tabs (B140),
  (3) modules: `module.toml`, the first-party modules moved out of the image flake into the catalog, (4) `recipe.toml`
  v2 and the recipes recomposed, (5) key bindings as data, (6) profiles, (7) the recipe builder in the app, (8) the
  website's catalog page. Each step tested locally before anything is published.
- **Step 1 done (2026-10-25):** `nix/looks/adapter.nix` turns a look's data into the desktop's, WezTerm's, Zellij's,
  Neovim's and Emacs' part (editor themes catppuccin/rose-pine/modus from nixpkgs, else highlight groups and faces made
  from the colours); `modules/look.nix` takes a look's name or its data (`settings.look` as an object). Until step 2
  the four looks are data files in the image flake (`omaflake.toml` generated by `brand/tokens.py`, the others copies
  from the catalog); the old per-look Nix files are gone. Checked: the four looks against the old files (same, except
  – omaflake: Zellij's orange slot is the terminal's yellow; Rosé Pine Dawn: Emacs' cursor and line highlight from the
  schema's colours, Zellij's active frame iris (the accent) instead of pine; Catppuccin: WezTerm and Zellij with
  explicit colours instead of their built-in schemes, GTK adw-gtk3-dark); a NixOS and a macOS configuration evaluated
  with look data in `settings.look`; the colour-made Neovim theme in Neovim and Emacs theme in Emacs (headless:
  Normal/default and String as the look says); `nix flake check` (with the recipes). Catalog: omaflake and Vanilla
  added (`plain = true`, schema look/v1 and catalog/v3 updated), all four validate. OPEN: an image built and looked at.
- **Step 2 done (2026-10-25):** the core reads catalog v3 (`RecipeCatalog::VERSION` 3, `looks`); `looks::LookStore` –
  installed looks in `<data>/looks/<author>.<name>/` with `looks/.installed.json` (schema `installed-looks` v1), a local
  folder source's looks used live, `install` (files checked by size and SHA-256, known files only), `remove`, the
  default (settings `defaultLook`, else the first). Settings v4: the guest look becomes `defaultLook`, the official
  source omaflake/catalog (`.installed.json` recipes from omaflake/recipes count as from it). Builds send the chosen
  look's data (`GuestSettings::look`), images record the look's id; the image flake has no look built in any more
  (`omaflake.look` null = every program its own colours; `brand/tokens.py` writes `brand/omaflake-look.toml`, copied
  into the catalog). API `looks.list/install/installRecommended/remove/setDefault`, job `look-install`; Doctor item
  `look` (a build blocker, "Install Recommended"); pages: the Looks tab (home tab `looks`, rows with a colour strip),
  the build page's look field from the installed looks, settings without the default credential and look; CLI `omaf
  looks`, `omaf build --look <author>/<name>`. TUI, iced and SwiftUI render the tab (colour strips: RGB blocks, iced
  containers, SwiftUI rectangles). Checked: `cargo test` (store: folder looks, install with untrusted entries refused,
  remove, default; settings v3 → v4; pages; golden `looks`), `swift test`, flake checks, the CLI against the public
  catalog (two looks there yet) and a local folder source (all four). OPEN: publishing the catalog commit with
  omaflake's look and Vanilla (until then "Install Recommended" finds omaflake's own only in a local source);
  clicking the tab in the three UIs; a build with an installed look.
- **Locales done (2026-10-25):** `locales::Locales` – config.json `locales` and `defaultLocale` (settings v5: the
  former guest values become a locale "Standard", `hostKeyboard` replaces them in the settings, `guest` is gone);
  images record their locale (image.json v3 `locale`, `localeName`); `BuildChoice` is credential + look + locale.
  `GuestSettings.keyboards` (the image flake: `xkb.layout` "de,ru,gr", a variant each, `grp:win_space_toggle` with
  several, Waybar's `hyprland/language`; nix-darwin: every keyboard in `AppleEnabledInputSources`, new layouts ru, ua
  and gr read from macOS' `AppleKeyboardLayouts-L.dat`). API `locales.list/add/update/remove/setDefault`,
  `settings.set {hostKeyboard}`, `images.build {locale}`; pages `locales` and `locale` (the credentials' shapes, so
  every UI draws them as those; home action `locales`, icon `globe`), the build page's `locale` field, settings "This
  Mac" with the host keyboard only; CLI `omaf locales`, `omaf build --locale`, `omaf config --host-keyboard`. The TUI
  header shows its buttons with labels as far as they fit, the rest as key and symbol. Checked: `cargo test` (store,
  v4 → v5, pages, golden `locales`/`locale`), `swift test`, flake checks, `nix eval` of a NixOS guest with a locale of
  three keyboards (layout "de,ru,gr", options with the toggle, time zone and language), the CLI in a scratch data
  folder. OPEN: several keyboards in a macOS guest (BM24), a recipe's locale restriction (with recipe.toml v2), images
  built with a locale and looked at.

**B142 (2026-10-25, built): the building blocks of B141 – modules, key bindings, recipes v2, profiles, the recipe
editor.**
- **Catalog** (omaflake/catalog, catalog v4): `modules/<author>/<platform>/<name>/` (module.toml – schema module/v1:
  version, title, provides, needs, enhances, runs_in, desktop, own_keys, unfree, `[actions.<name>]` with title, key,
  mode?, desktop?, context? – and default.nix), `keymaps/<author>/<name>/keymap.toml` (keymap/v1), `capabilities.toml`
  (desktop, terminal, multiplexer, shell, editor, default-editor, pdf-viewer; exclusive or not), looks with a `version`,
  recipes v2 (recipe.toml only). First-party modules cut from the image flake and the old recipes: NixOS hyprland,
  shell, wezterm, zellij (its "Unlock First" preset), neovim (with nvim-lspconfig: other modules enable their language
  servers), emacs, zathura (also the default for PDFs), latex, typst, python-data, pandoc, omaflake-dev; macOS desktop,
  shell, wezterm (its wezterm.lua with the session key), zellij, neovim, latex, typst, vscode, homebrew, pandoc,
  lazygit, omaflake-dev; key bindings `omaflake/default` (the defaults written down). `tools/catalog.py` checks
  everything – references with versions against what the catalog has, needs and exclusive capabilities (warnings),
  overlapping keys and the key rule (errors) – and writes catalog.json v4.
- **Key notation:** a chord ("Super+Shift+Return", "Ctrl+g"; Super, Ctrl, Alt, Shift; lowercase letters, named keys,
  `Digit`/`Arrow` placeholders) or a sequence for editors ("Space f f"). Overlap: the same context and mode; a desktop
  one with any key without a mode; a program's key with one of a program running inside it (`runs_in` names a
  capability the outer one provides); prefixes count. Key rule: without a mode one chord starting with Super or Ctrl,
  except an editor's own keys (`own_keys`).
- **Image flake:** input `omaflake-catalog` (was `omaflake-recipes`): a catalog checkout, or the trees omaflake stages
  (`recipes.json`: id → `{tree, recipe}`). It reads recipe v2 (`lib.recipeDefaults` flattens `[requirements]` and
  `[vm]` as before), imports the recipe's modules with an `omaflake` argument (`id`, `keys`, `allKeys`, `has`,
  `keyLib` = `nix/lib/keys.nix`, `recipe`) and the platforms: NixOS `modules/platform.nix` (host name =
  recipe name, `omaflake.terminal`, `omaflake.launchers`), macOS `modules/darwin-platform.nix` (`homeFiles`,
  `homeLinks`, written once as the user), `look.nix` with `lookParts`; `home/` keeps only the user and git. Gone from
  the flake: hyprland.nix, editor.nix, home/look.nix, neovim.nix, emacs.nix, the Zellij preset.
- **Core:** `parts::PartStore` – one store for looks, modules and key bindings (`<data>/<kind>/`, `.installed.json`
  schema installed-parts v1: source, version, content hash; files checked like recipes; local folder sources live);
  `looks` on top of it, `modules` (ModuleStore, capabilities kept from catalogs and read from local folders),
  `keymaps`; `composition` (RecipeSpec = recipe.toml v2, PartRef with version/min/max, Composition: missing parts
  block, needs/incompatible/overlaps/key rule warn, LocaleRules); `parts::install_recipe_parts` (from the first catalog
  that has a part in an allowed version, the official last); RecipeLibrary: recipe v2 only (RECIPE_MINIMUM 2), catalog
  v4 only, `requires` gone, staging with modules and key bindings; `profiles` (settings v5 `profiles`,
  `defaultProfile`); `my_recipes` (`<data>/my-recipes/`, a local folder source first in the order, author `my`).
- **API and pages:** `recipes.installParts`, `recipes.save`, `recipes.delete`, `modules.list/install/remove` (job
  `module-install`), `profiles.*`; `Recipe.parts`; `images.build` refuses missing parts and takes the recipe's look;
  pages `modules` (the looks' shape), `profiles`/`profile` (the credentials'), `recipeEditor` (the credential form's:
  a toggle per module, look, two key binding sets, requirements, VM, live check), recipe details with modules, look,
  key bindings, requirements, `notes` and `keys`, the build page's `notes`, New VM's profile field and requirement
  warning, home tab `modules` and action `profiles`. CLI `omaf modules`, `omaf profiles`, `omaf recipes show
  [--install]`, `omaf new --profile`; `omaf recipes install` brings the parts.
- **Checked:** every recipe's system evaluated before and after (aarch64 and x86_64 NixOS, macOS): the derivations differ
  only as expected – Hyprland's binds as literal strings instead of `mod .. " + …"`, Neovim's `<Space>ff` for
  `<leader>ff` (leader is Space), Waybar's style order, comments, package order, the Dock order (WezTerm first, fixed
  with `mkOrder`), macOS research's init.lua and files written by the platform; `nix flake check` (root, image flake,
  with the catalog); a second key binding set overriding a default (Hyprland `SUPER + T`, Zellij `Ctrl Shift g`);
  without a look and with Vanilla; the core's staged tree evaluated to a NixOS system; catalog.py's checks fire on a
  broken copy (overlap, key rule, missing capability, Neovim + Emacs, a version not allowed); the catalog and every
  recipe, module, key binding set and look validate against the JSON Schemas; parts installed from the catalog
  checkout (`parts::tests`); `cargo test` (416 core tests), `swift test`, TUI and iced tests (the new pages drawn);
  the CLI in a scratch data folder (recipes from the local catalog, `recipes show`, `modules`, `profiles`).
- **Built:** `neovim-minimal` (aarch64-linux, on the Linux builder) with a locale of two keyboards, started headless
  and checked over SSH: host name from the recipe, time zone and `de,ru` with the toggle, the look's files, Hyprland's
  binds (`Hyprland --verify-config`: config ok), Zellij's preset with the recipe's keys (`zellij setup --check`: well
  defined), Neovim's `<Space>ff`, `EDITOR=nvim`. Found and fixed with it: `omaf build` without `--look` took the default
  look instead of the recipe's (now `LookStore::suggested`), and a build through the service lost the locale.
- **OPEN:** images of the other new recipes built and looked at (emacs-minimal, neovim-research, lab; the x86_64 PC;
  macOS research and minimal), a desktop session looked at in a window; the new pages clicked through in the three UIs; installing from the public catalog (needs the catalog
  commit pushed – the public one is still v3, which this omaflake refuses); a kept version of a local folder's recipe
  does not keep its modules; the website's catalog page; recipes' locale restrictions in the catalog (none of
  omaflake's recipes restricts locales yet).

**Found on the way:** environment variables of the CLI (`VM_FLAKE`, …) do not reach the core service – a build that
runs there uses config.json's flake (OPEN).

**Checked at the mini PC's desktop (2026-10-24):** the Linux app with its pages and the tray icon (GNOME's AppIndicator
extension); the token of a private source in the GNOME keyring – Secret Service item "omaflake recipe source
omaflake/recipes" in "login", removed with the source; with the keyring still locked (automatic login) the unlock
dialog appeared, but omaflake fell back to `tokens.json` after 60 s (password typed too late or the unlock not seen –
OPEN); mpv shows the camera's test pattern (B130).

**B143 (2026-10-25, fixed after 0.6.0): installed modules held other modules' files.** Found by the maintainer with a
fresh data folder and the released 0.6.0: `neovim-minimal` failed with "attribute 'session' missing" in
`modules/omaflake/nixos/wezterm/default.nix`, a recipe of their own with "The option `omaflake.homeLinks' does not
exist" in `nixos/typst`. The folders under `<data>/modules/` held the wrong module (`omaflake.nixos.wezterm` the
NixOS zellij module, `omaflake.nixos.typst` the macOS latex one, …). Cause: `PartStore::install` downloaded into
`.download-<pid>` – one work folder for the whole `omaf serve`, so recipes installed at the same time (the app installs
the suggested recipes at once, each with its parts) wrote into and renamed each other's folder; the manifest's
read-modify-write could lose entries the same way. Fixed: a work folder per install (`.download-<uuid>`, as recipes
have), the swap into the part's folder and `.installed.json` under a lock; `parts::tests::parallel_installs_keep_each_
part_in_its_folder` installs four recipes' parts in parallel and compares every module with the catalog (fails 3 of 3
runs with the old work folder). Data folders installed with 0.6.0 must reinstall their modules (remove them in the
Modules tab, or move `<data>/modules/` aside – the Modules tab finds a module whose files differ from its manifest hash since B158).
Asked with it and done: the Modules tab drew the dashed frame of a plain look before every module (its rows are the
looks' shape: `swatches` is optional now, absent for modules); "Create macOS Base …" twice without a base in the
SwiftUI app (the button only without `empty`, as the TUI and iced did); the base being created as a job row in the
macOS bases section (IPSW download or install, its progress, a click opens its page; the create action greyed out
meanwhile) – before, leaving its page looked as if no base were coming; "Install All" (download icon) in the heads of
Recipes, Modules and Looks: what the list offers as filtered (`recipes.installAll`, `modules.installAll` – one job,
one after the other –, `looks.installAll`; a confirmation when one is not official). OPEN: clicked in the app.

**B144 (2026-10-25, ideas of the maintainer – noted, not decided, not built; done 2026-11-25 for 0.8.0).** *Done:* the
navigation split and the key bindings' own page came with B147 (Run and Workshop, `keys`), the catalog with search and
chips with B148; now authors are chips too ("@<author>", when the modules come from more than one), and the first-run
wizard: the page `welcome` (a credential form's shape, so every interface draws it) asks what to run (NixOS, macOS,
both – on Linux NixOS only), whether to start with the suggested recipes and omaflake's looks, the user and full name
while there is no credential (the password asked on Set Up, kept in the keychain), developer mode; `setup.welcome`
creates the credential, sets developer mode and starts the jobs – Set Up for NixOS (it installs a look), the catalog's
suggested recipes of the chosen kinds (lab ones only in developer mode), the macOS base for the credential when there
is none. The app opens it at its first start in front of the Doctor (Set Up and Later lead there); the Doctor offers
"Guided Setup …" while there is no credential. OPEN: clicked through on a fresh Mac. Before:
- **First-run wizard:** a newcomer finds many tabs and choices at once. On the first start (no data folder, or another
  sign of a first run) a short series of questions: minimal or complete (all standard recipes, modules and looks at
  once, or by hand later); what it is for (suggests recipes and modules); NixOS VMs, macOS VMs or both (is a macOS
  base needed – then start creating it); the first credential, locale and profile; lab (developer mode); the shared
  folder; … OPEN: what counts as a first run besides a missing data folder; whether the Doctor's steps become its pages.
- **Structure and navigation:** there are many tabs now. Perhaps the catalog (recipes, modules, looks, key bindings)
  apart from the running side (VMs, images, bases, activity) – a second top-level menu. An opened VM, image or recipe
  with many key bindings gets long and hard to read: key bindings in a window or page of their own instead of inline.
- **Putting a recipe together from the catalog:** not a toggle for every module (does not scale to 100+ modules) but
  picking from the catalog with search and filters (capability, platform, tags, author) – what is picked is the
  recipe, with its needs and conflicts shown as now.

**B145 (2026-10-26, after 0.6.1): after a restart Nix, omaflake's QEMU and the installed recipes were gone.** Found by the
maintainer: after installing 0.6.1 and restarting the Mac the Doctor said Nix and QEMU were not installed, the installed
recipes showed as not installed, and the NixOS VM ran very slowly. Cause on the Mac: `/etc/fstab` and the launchd daemon
`org.nixos.darwin-store` were missing (since when and why is OPEN – the first restart since 2026-10-13; nix-darwin was
reactivated on 2026-10-16), so macOS mounted the "Nix Store" volume at `/Volumes/Nix Store` and `/nix` stayed empty.
Three omaflake faults made it worse, fixed: **(1)** the recipe list came from `nix eval …#lib.recipeDefaults` – without
Nix it failed as a whole, the list was empty and the catalogs offered every installed recipe again. Now
`RecipeLibrary::defaults` reads each recipe.toml and flattens it as the flake does (user, top-level values without the
parts, `[requirements]`, `[vm]`, author/kind/name/systems; checked against `nix eval` on the maintainer's 8 recipes:
`recipes.list` byte-identical); a recipe that cannot be read is named in `unreadable`, the others stay. The flake
evaluation remains the Doctor's `flakeEval` (`ImageStore::evaluate_recipe_defaults`). **(2)** `QemuRuntime::locate` took
the first `qemu-system-*` on PATH: Homebrew's (no 3D) when omaflake's QEMU was gone – and in a terminal always, as
Homebrew's `brew shellenv` comes after Nix's PATH. Now, without VM_QEMU or a path in config.json, the first with 3D
wins (`preferred_binary`; `-device help` remembered per binary and mtime). **(3)** the Doctor said "not found" and
offered to install Nix again: now `NixVolume` (`diskutil info "Nix Store"`) tells "Nix Store volume not at /nix
(mounted at … / locked)" and offers Mount … (`builder.mountNix`, administrator dialog): `nix` in synthetic.conf, the
fstab line, the darwin-store daemon (unlocking with the key in the System keychain when encrypted) – each only when
missing –, then mounts it (unmounting from /Volumes locks an encrypted volume; it unlocks it with the key) and restarts
nix-daemon. Asked with it: a recipe's build button stays, greyed out with the Doctor's sign (`Action::blocked_by`,
`badge`) instead of being replaced by a Doctor button. Tried: the repair by hand on the maintainer's Mac (the same steps;
the daemon mounted the volume at /nix). OPEN: Mount … clicked in the app on a Mac with the volume under /Volumes; the
build button's sign looked at in the macOS app, the Linux app and `omaf ui`.

**B146 (2026-10-28, after 0.6.2): the IPSW after a macOS update; a download that failed at once.** Found by the
maintainer: after macOS 27.2 the Doctor called the IPSW missing (Apple offers 26B123, here 26A434), and Download
seemed to do nothing. The log (`ipsw-download.log`): "Not enough space: 47.8 GB free, about 55.6 GB needed (IPSW,
installation, image)" – the download on its own checked the space of a base and an image too, failed within
milliseconds, and its error reached only the activity log. Fixed: **(1)** `Prerequisites::ipsw` – an older IPSW here
makes it Limited with `IPSW_NEWER` ("A newer macOS is available …"), not Missing; the macOS space check counts the
newer IPSW only when none is here or it is partly downloaded. **(2)** `MacBaseInstaller::download_ipsw(reserve)` – the
download needs what is still to download (less a `.partial`) plus `DOWNLOAD_RESERVE` (2 GB); creating a base passes
`MAC_SPACE_NEEDED` as before. **(3)** The Doctor's IPSW row shows "Download failed: …" from the last
`ipsw-download` job while none runs. Test `doctor_newer_ipsw_and_a_failed_download`, `ipsw_item_without_and_with_a_release`
extended. OPEN: clicked in the app (the maintainer's disk had 26 GiB free on 2026-10-28 – the 26.6 GB IPSW needs
space freed first).

**B147 (2026-10-28, wishes of the maintainer – decided the same day, not built): every key binding with its modes, a
key binding view, a key binding studio, a recipe studio, navigation.** A German draft with recommendations and open
questions was kept in the private workspace until 2026-11-27; what was decided is here. In short: modules describe all their bindings
(module.toml v2: modes, `after` – the mode after an action –, `does` – the program's own command); configurations
such as Zellij's KDL are generated from that data so a key binding set can change a key and its `after`; a keyboard
button at VMs, images and recipes opens their keys grouped by module and mode, showing the chain (Ctrl g › p › n,
then locked); a studio for own key binding sets; a recipe studio picking modules from the catalog with search and
filters (B144); the navigation split into the running side (VMs, images, recipes) and a workshop (studios, catalog,
credentials, locales, profiles). Today: Zellij's `unlock-first.kdl` has about 100 bindings in 12 modes, its
module.toml names 4.
The maintainer's decisions: all recommendations – bindings omaflake sets or changes plus the program defaults one
needs to know (shown, not changeable), not every built-in key; configurations generated from the data; one window
with a sidebar in two groups (running, workshop); order: module.toml v2 with Zellij complete → the key binding view →
navigation → recipe studio → key binding studio.

**B147, step 1 (2026-10-28): module.toml v2 – modes, `after`, `does`; Zellij complete, its KDL generated.**
- **Formats:** module/v2 (`[modes.<name>]` with `title`, `title_de`, `start`; an action's `key` and `mode` one or a
  list, `after`, `does`, `own`, `title_de`), keymap/v2 (an entry is a key, keys, or `{ key, after }`; a program's
  `own` keys are not changed by sets), catalog/v5 (modules carry `modes`; schemas in docs/schema). The field is
  `after`, not `then` – `then` is a Nix keyword. New key names `PageUp`, `PageDown`, `Plus`; a capital is `Shift+h`
  (Zellij: `H`). Only v5/v2 are read (`RecipeCatalog::VERSION`/`MINIMUM` 5, `MODULE_VERSION`, `KEYMAP_VERSION` 2).
- **Image flake:** each module gets `omaflake.bindings` (its actions with the sets over `key` and `after`) and
  `omaflake.modes` besides `keys`; `lib/keys.nix` knows the new names.
- **Zellij (catalog `modules/omaflake/nixos/zellij`, 2.0.0):** all bindings of the "Unlock First" preset as 96
  actions in 12 modes; default.nix writes the keybinds from them (`does`, then `SwitchToMode` to `after` – first for
  the typed-input modes renametab, renamepane, entersearch, as the preset). `unlock-first.kdl` moved to the catalog's
  `_to_delete/`. Which bind wins where the preset binds a key both in a shared block and in a mode: the mode's –
  Zellij 0.45.1 `Keybinds::from_kdl` applies shared blocks first, then overwrites with the mode's (checked in the
  source), so the data lists only what works in each mode. **Checked:** the effective binds per mode and key of the
  old preset and the generated KDL are the same 208 (a script over both files); one deliberate change, the preset's
  `"Shift f"` is `"F"` now like its H/J/K/L; `zellij setup --check` (0.45.1, the flake's nixpkgs): "Well defined";
  a set with `new-pane = { after = "pane" }` changes exactly that bind; `tools/catalog.py` rejects an `after` that is
  not a mode; every module.toml, keymap.toml and catalog.json validate against the new schemas.
- **Core:** `ModuleAction`/`ModuleMode`, `KeyEntry`, `Binding` with `keys`, `modes`, `after`, `does`, `own`,
  `titleDe` (API `recipes.list` bindings; goldens), overlaps where two bindings share a mode; `omaf recipes show`
  prints "(in pane, then locked)". The website's catalog page shows key and mode lists.
- **OPEN:** the generated config in a running VM (Zellij's keys pressed); macOS' Zellij (it has Zellij's default keys,
  not the preset, and no list yet); the other modules' complete lists (Hyprland's resize submap as a mode, Neovim's
  plugins' keys, WezTerm, Emacs, Zathura); the installed 0.6.2 does not read catalog v5 – the local catalog source
  needs a development build until the next release.

**B147, step 2 (2026-10-28): the keys view.** Page `keys` (`views/keys.rs`, params `recipe` | `image` | `vm` – an image
or VM through its recipe, with the note that a module or set updated since the build may differ): modules in the
recipe's order, those with desktop keys first; per module a group for keys without a mode ("Anywhere on the desktop",
"In the program", "In every mode"), then the start mode, then the modes by distance and name; each mode with its way
from the start mode (shortest, keys that only switch preferred); rows doing something before rows that only switch,
each with caps (⌘ ⌃ ⇧ ↩ on a Mac host, "1–9", "←↑→↓"), "then <mode>", `backToStart`, `does`, `from`, `own`, the title
in German where the module has `title_de`. `ModuleUseDto.modes` carries the module's modes; `Ctx.language`. The row
button `keys` (icon `keyboard`) on recipes, images and VMs whose recipe has keys; `RecipeDetail.keys` is gone. macOS
app: `Window` "keys" (`KeysView`, a search field filtering titles, keys, `does` and modes; a click on a row shows
`does` and the set), opened by `ui.show` with page `keys`; Linux app (iced) and `omaf ui`: a page (Enter on a row shows
`does`). **Checked:** `cargo test` (page test `keys_page_by_module_and_mode`, goldens `keys`, `keys.vm`), `swift test`
(contracts), the TUI draws it; with a development build and a scratch data folder on the real catalog,
neovim-minimal's page: Hyprland 14 keys, Zellij 162 rows in 12 modes plus the desktop, Neovim 9. **OPEN:** the macOS
window and the iced page not clicked (no way to drive the apps from here); search in the Linux app and `omaf ui`;
the Linux app as a window of its own.

**B147, step 3 (2026-10-28): navigation – Run and Workshop.** `home`: every tab names its `group`, `groups` lists `run`
(VMs, images, recipes, activity) and `workshop` (modules – with key binding sets –, looks, credentials, locales,
profiles) with `attention` (a tab of it has a badge other than a count); credentials, locales and profiles are tabs now,
not header buttons (a dot at Credentials while there is none). macOS app: the app window has a sidebar (170 pt, both
groups; window 670 pt wide at least), the menu bar window (500 pt) a group switch above the tab bar of the open group,
remembering each group's last tab; the form tabs (credentials, locales, profiles) scroll themselves. Linux app: the
group buttons before the open group's tabs. `omaf ui`: the open group's tabs, the other group's name after them; Tab
goes on across the groups. **Checked:** `cargo test` (home: groups, the workshop's tabs, attention; TUI
`tab_goes_on_into_the_workshop`), `swift test`. **OPEN:** clicked in the apps; the sidebar in the Linux app (a group
switch for now); the recipe studio and the key binding studio join the workshop in steps 4 and 5; the recipes
offered by the sources are still in the Recipes tab (the catalog in the workshop: with the recipe studio).

**B147, step 4 (2026-10-28): the recipe studio.** Page `recipeStudio` (`views/recipe_studio.rs`, was
`recipe_editor.rs`; page `recipeEditor` is gone): `about` (name, platform, description), `recipe` (its modules, hints,
`keys` – the keys page on `{draft: {platform, modules, keymaps, name}}`, computed from the modules and sets here),
`catalog` (search over title, description, reference, tags and capabilities; filters capability, tag, author; the
platform's modules here and offered that are not in the recipe), `dress` (look, two key binding sets), `vm`. The
modules are `values.modules`, set as a whole by each row's `ui.field` (Add / Remove), so every UI keeps them as form
values. Hints: a needed capability none provides – with "Add <module>" (here before offered, official first) –,
exclusive capabilities twice, overlapping keys and the key rule (from the composition of the modules here), modules
still to install. A workshop tab (a new recipe) and the target of "New Recipe …", "Edit …", "Copy to My Recipes …";
the macOS app clears the studio's values when it opens on a recipe. UIs: SwiftUI `RecipeStudioView` (a grouped form
top to bottom), iced `studio`, `omaf ui` `studio` (Enter on a module row adds or removes it). **Checked:** `cargo test`
(`modules_tab_profiles_and_the_recipe_studio`: copy, remove, the needs hint with its action, search, filter; goldens
`recipeStudio`, `keys.draft`; iced draws every golden page; TUI draws it), `swift test`; with a development build on the
real catalog: Hyprland + Emacs + Neovim says "Hyprland needs a terminal → Add WezTerm" and "Emacs + Neovim do not go
together", search "term" finds WezTerm, Shell, Zellij; a draft's keys page shows Hyprland 14 and Zellij 162 rows.
**OPEN:** clicked in the apps; the keys of modules still in the catalog (their actions are in catalog.json, not read
for the preview); two columns (catalog next to the recipe) in a wide window; the recipes the sources offer still in
the Recipes tab.

**B147, step 5 (2026-10-28): the key binding studio.** Page `keymapStudio` (`views/keymap_studio.rs`, params `id?`
– a set here: one of yours ("my.keymap.<name>") is edited, any other copied –, `values`): `about` (name, title,
description, platform, base – the set it starts from), `module` (the platform's modules with actions, "Zellij – 96
keys, 2 changed"), `search`, `hints` (overlaps and the key rule, from the composition of every module of the platform
with the set as chosen), `groups` (the module's keys without a mode, then by mode with the way there as caps) of rows
`{key, after?, changed, reset?, does?, conflict?, own}`. Values `key:<module>:<action>` ("h / Left", checked with
`parse_key`, an error at the field and Save waiting) and `after:<module>:<action>` (a choice of the module's modes,
"stays in <mode>" for its own; only for actions in one mode). The set is the base's entries, then the values; Save
(`keymaps.save`) gets only what differs from the modules' own keys and `after`. Storage: `MyRecipes::save_keymap` /
`delete_keymap` – `<data>/my-recipes/keymaps/my/<name>/keymap.toml` (keymap v2, read back before it counts; the
folder becomes the first source). `ui.field` takes `also: [{valueParam, value}]` (Undo resets the key and the mode at
once; SwiftUI, iced, `omaf ui`). A workshop tab, and "Edit …" / "Copy to My Key Bindings …" on a key binding set's
row in Modules. **Checked:** `cargo test` (`keymap_studio_changes_keys_and_modes`: a copy of a set, its way with the
set's key, own keys read-only, German titles, after and a key changed, the overlap at the row and as a hint, Save's
keys, an invalid key; `key_bindings_save_rename_delete`; goldens; iced and TUI draw it), `swift test`; with a
development build on the real catalog: a copy of omaflake/default with Zellij's `new-pane` after `pane` and
`focus-left` on `n / Left` says the overlap at both rows, Save gets exactly those two; `keymaps.save` wrote
`my/mine`, and a draft recipe naming it shows New pane staying in pane mode, from `my/mine`. **OPEN:** clicked in the
apps; a recipe built with an own set and Zellij's keys pressed in the VM; editing an action that works in several
modes' `after` (left out: one choice would change it in each).

**B147, step 6 (2026-10-28): four groups, by the maintainer after trying the development build.** Run (vms, images,
activity), Workshop (recipes as "Build Images", recipeStudio, keymapStudio), Catalog (recipeCatalog, modules, looks,
keymaps, sources), Profiles (credentials, locales, profiles as "Performance"). Decided here: the activity stays in Run,
modules go into the catalog next to key bindings, the sources leave the settings. Pages: `recipes` has the recipes
here, their updates and the suggestion; `recipeCatalog` (the same shape) what the sources offer, their errors, the lab
hint and Install All; `modules` the modules only, `keymaps` the key binding sets (a quick button opens one in the key
binding studio; "New Key Bindings …"); `sources` the sources section that was the settings' (`settings` has none now).
UIs: SwiftUI `SourcesView` (out of `SettingsView`), iced `sources`, `omaf ui` `sources`; `recipeCatalog` and `keymaps` use
the recipes' and looks' views. **Checked:** `cargo test`, `swift test` (goldens `recipeCatalog`, `keymaps`, `sources`,
`recipes.loading` now from the catalog). **OPEN:** clicked in the apps.

**B148 (2026-10-28): rethinking the UI, by the maintainer after trying the four groups.** The recipe studio still
scrolls through the whole catalog; keys are typed ("Ctrl+g") and changed inline; `⌃g` reads as "^g"; every page's
explanations are always there, some inside the list's box; the menu bar window's groups were small grey text above a
second bar. Decided (all as recommended): the menu bar window shows only Run; caps named as the host's keyboard; search
with suggestions, tag chips and five results a page in the studios **and** the catalog tabs; order: basics → recipe
studio → key binding studio with an editing sheet, then recording. The design: the maintainer's notes outside the
repository.

**B148, step 1 (2026-10-28): caps, the help switch, the menu bar window.**
- **Caps:** `keys::caps` names a modifier as the host's keyboard does – Ctrl, Opt, Shift, Cmd on a Mac (a guest's Super
  is ⌘), Ctrl, Alt, Shift, Super elsewhere; Enter, Esc, PgUp, PgDn; no symbols for modifiers, no "+" between caps. The
  UIs draw each cap as a box of its own (SwiftUI `KeyCaps`/`KeyAlternatives`, iced `widgets::key_caps`, `omaf ui`
  `[Ctrl] [g]`), alternatives joined by "or". Hints name keys as caps (`Hint.keys`) instead of text in the sentence
  (overlaps, the key rule). The modules tab says how many keys a module has and opens them in the keys window
  (`keys` params `module` + `platform`).
- **Help switch:** config.json v6 `help` (off when left out), `settings.set {help}`, `SettingsDto.help`; the home page's
  `help` is the switch (an action with `on`). Help is a page's `intro` and `footer`, a field's `help`, a hint with
  `help: true` (`Hint::help`: the build note of the keys window, the lab hint, the studio's catalog note); warnings,
  errors and what is missing never are. The UIs draw help only while it is on, each in a box of its own outside the
  list's box (SwiftUI `HelpNote`, iced `widgets::help_note`, `omaf ui` a "?" line).
- **Menu bar window:** only the Run group's tabs (`AppModel.menuTab`, apart from the window's tab), large, as one
  segmented switch with a hover fill; below the list "Workshop, Catalog and Profiles …" opens the app window. The
  help switch is in the header of both. The Linux app has the sidebar of the Mac's window now (window 780 × 680, at
  least 680 wide); `omaf ui` lists the switch with the header's actions (letter H).
- **Checked:** `cargo test`, `swift test` (goldens: caps, `help`), clippy. **OPEN:** clicked in the apps; the TUI's
  switch shows no on/off of its own.

**B148, step 2 (2026-10-28): the recipe studio as a kit.** `recipeStudio.catalog` has no choice filters any more:
`search` with `suggest` (up to six while typing – the title's start first; taking one adds it and clears the search),
`tags` (every tag and capability of the platform's modules as a chip, the most common first; several on: all of
them), one page of five `rows` (those in the recipe too, `inside`) with `pages` (Previous/Next as `ui.field` on
`values.page`; a page that is gone is the last), `reset`. A row has `details` (InfoLines) and `detailActions` (its keys
window, a module here with keys). UIs: SwiftUI two columns from 720 pt (finding on the left, the recipe on the right;
one column below, the recipe first), the search with a suggestion list (↑/↓, Enter, Esc), chips in a wrapping
`ChipFlow`, rows opening their details, ✓ or + per row, a pager, the VM folded in a disclosure, the actions in a bar
at the bottom; iced the same in one column (suggestions, wrapping chips, expandable rows, pager); `omaf ui` suggestions
as rows, the chips and the pager as button bars, details with Space. **Checked:** `cargo test` (new
`recipe_studio_pages_through_the_catalog`), `swift test`, clippy. **OPEN:** clicked in the apps.

**B148, step 3 (2026-10-28): the key binding studio with a key editor.** `keymapStudio` has no module choice and no key
text fields any more: `modules` are chips (the set covers those the base set names, else all; `values.modules`; a
module taken out is not saved), `cards` per module that open (the first, or every one the search finds something in)
with their modes as groups that open (the keys without a mode and the start mode), rows read only (`caps`, `after`,
`changed`, `conflict`, `edit`, `reset`). `editor` while `values.edit` names a key: the draft keys (`values.draft`, a
list; or `draftText` typed), `modifiers` and `keyGroups` to put one together, the mode afterwards (`draftAfter`: stay,
then back to the start mode, then the others), what the draft overlaps (it counts in the composition while the editor
is open), the module's own keys, `default`, `cancel`, `apply` (each `ui.field` with `also`, clearing the editor's
values). Key values may be lists now. UIs: SwiftUI sections per module with disclosure groups per mode, chips with
"Add Module", the editor in a sheet – **recording** (`KeyRecorder`: `NSEvent.addLocalMonitorForEvents` for keyDown and
flagsChanged, ⌘ as Super, a sequence's next chord within 1.2 s, the keys swallowed while recording) or putting
together (modifier switches, a grid of keys), "Use as the Key" / "Add as Another", the mode afterwards as two segments
and a menu; iced cards and groups that open, the editor taking the page (modifier switches kept as the client's own
`draftMods`, a key click makes the key; the typed field for several); `omaf ui` the same with the typed field.
**Checked:** `cargo test` (`keymap_studio_changes_keys_and_modes` rewritten: chips, cards, the editor, a draft, apply's
values, a module taken out, a typed key that is not one), `swift test`, clippy. **OPEN:** clicked in the apps; which
keys macOS keeps for itself while recording (Cmd+Tab, Cmd+Space, Mission Control's Ctrl+arrows by the docs – not
tried); recording in the Linux app; swapping a key with the one it overlaps.

**B148, step 4 (2026-10-28): the catalog tabs browse the same way.** `common::browse` (with `Browsable`, `Browse`,
`BrowseTag`, `Pager`, `BROWSE_PAGE` = 10): `FilterParams` has `search`, `tags`, `page` now; `ListHead.browse` on
`recipeCatalog` (the offered recipes after the filter menu's kind and architecture; its tag group became the chips),
`modules`, `keymaps` (tags and capabilities) and `looks` (tags, light or dark) – those here and those offered are one
sequence cut into pages, so a page may hold some of each; nothing matching says so with Reset Filter. `modules`,
`keymaps`, `looks` take `params.filter` now. UIs: SwiftUI `BrowseBar` between a list's title and its box (search,
chips in `ChipFlow`, clear) and `PagerRow` at its bottom (`ListSection.below`; `AppModel.setFilter` sends the search
after 300 ms); iced the search and chips in `section`, the pager at the box's bottom (`SetFilter` for `search` drops the
page); `omaf ui` the search as a field (`Browse.field`, `set` = `ui.filter`, run as the UI's own verb) and the chips and
pages as button bars. **Checked:** `cargo test` (`catalog_tabs_search_chips_and_pages`), `swift test`, clippy.
**OPEN:** clicked in the apps; Install All installs what the page shows, not every match.

**B149 (2026-10-28): the Nix Store filled the Mac.** Found by the maintainer with 6 GB left on a 4 TB disk: the Nix Store
volume held 476 GB, 469 GB of it kept by nothing – 364 GB the disks of 44 NixOS image builds (`nixos-disk-image`), each
copied out into the data folder and never deleted in the store (`nix build --no-link`: no GC root, but no garbage
collection either), the rest what those builds were made of. `nix-collect-garbage` freed 432.8 GiB (75991 paths).
Decided (the maintainer): omaflake removes its own leftovers and the Doctor offers the rest.
- `ImageStore::build` deletes the image's store path after copying the disk out (`nix store delete`, as the user –
  the daemon refuses a path something still uses; a failure is only logged). The system closure stays for faster
  rebuilds. A rebuild of the same version builds the disk again (minutes) and then finds the image.
- Doctor, `nixSpace`: what the store holds where /nix is a volume of its own (`prerequisites::volume_used`: a mount point
  of its own – none on most Linux hosts; on a Mac `df -k`, as `statvfs` there gives the whole APFS container's use –
  3.5 TB for a 5 GB volume, seen in the development build), and "Clean Up …" (`nix.cleanUp`, job
  `nix-clean-up`: `nix-collect-garbage` as the user, profiles' generations stay, not during a build), primary while the
  space is short; the result says the store's size before and after.
- **Checked:** `cargo test` (`doctor_cleans_up_the_nix_store`, `nix_clean_up_says_what_it_freed`), `swift test`,
  clippy. **OPEN:** a build in the app deleting its disk from the store (not run yet); the Linux builder VM's own disk
  (`/private/var/lib/linux-builder/nixos.qcow2`, 99 GB here) does not shrink when its store is cleaned up – the qcow2
  keeps its size; whether to compact it.

**B148, after trying it (2026-10-28):** "Change …" in the key binding studio did nothing – and no choice in a studio
shown as a tab reached the core: the tab was subscribed without its form values (SwiftUI `listRef`, iced `tab_page`,
`omaf ui` `tab_page` – the values went nowhere there: "No form here"). Checks with `omaf call` had passed, as they ask
the core directly. Now a form as a tab is asked with its values (SwiftUI `fullRef`, iced `effective`, `omaf ui` its
`tab_values`); iced shows the newest model of a page while the one for new values comes (`latest`); `omaf ui` sends one
subscription for a `ui.field` with `also` (`holding` – each was a request from a thread of its own, an earlier one could
arrive last). Forms have no grey background any more (`omaflakeForm`: grouped, `scrollContentBackground(.hidden)`),
and the menu bar window's way to the app window is a large cyan button "Open omaflake". **Checked:** `cargo test`
(`ui_field_on_a_studio_tab_keeps_its_values`), `swift test`. **OPEN:** clicked in the apps.

**B150 (2026-10-28): the look studio, by the maintainer – what omaflake reads as data, no Nix, can be made in the
UI.** Recipes, key bindings and now looks have a studio; modules stay Nix. Page `lookStudio` (`views/look_studio.rs`):
name, title, description, light or dark, tags, the look it starts from (yours is edited, any other copied; without
one omaflake's own or the first with colours, else built-in colours), plain; sections colours (8), terminal (16),
desktop (border or gradient, other windows, clock, the prompt's three), editors (the themes look v1 allows, or from the
colours), macOS (accent, icons, folders, wallpaper); what it does not show of the base is kept (split, GTK, pointer).
`FieldKind::Color` ("#rrggbb"). A preview the UIs draw from `preview`; WCAG contrast hints. `looks.save`/`looks.delete`
(`MyRecipes::save_look`: look.toml v1, read back before it counts, renames move). Workshop tab "Look Studio"; the
looks tab's rows open a look in it ("Edit …" / "Copy to My Looks …"), its head "New Look …". UIs: SwiftUI
`LookStudioView` (ColorPicker per colour, the terminal as two rows of swatches, `LookPreviewView`: bar with clock, a
window with the border gradient, a terminal), iced (a swatch and the hex typed, a preview drawn the same way),
`omaf ui` (swatches as coloured blocks, the hex typed). **Checked:** `cargo test` (`look_studio_colours_preview_and_contrast`,
`looks_save_rename_delete`), `swift test`. **OPEN:** clicked in the apps; a look of the studio in a built image.

**B151 (2026-10-28): the app window had no Dock icon, and "Locales" in German.** In the default frame (menu bar) the
app runs as `.accessory`; the window opened with "Open omaflake" was a window of an app without a Dock icon or a place
in the app switcher. `MainWindowHook` now makes omaflake `.regular` while the app window is open (when it is shown and
each time it becomes key – SwiftUI may keep the window) and `.accessory` again when it closes, unless the frame shows
the window anyway. German: the maintainer chose "Region"/"Regionen" for locale(s) (the strings only; English stays
"Locales"); the locale's own field "Language and region" became "Sprache und Format", the build page's choice
"Region". **Checked:** `swift test`, `cargo test` (translations). **OPEN:** tried in the app.

**B152 (2026-10-28): the app window keeps its header and sidebar.** A page opened from a tab or the header (Doctor,
Settings, an image, a VM, New VM …) took the whole window before; now it opens next to the sidebar, below the header
with omaflake and its icons, with its own way back. A click in the sidebar goes to that tab at once and closes the pages
open on top (`AppModel.closePages`; iced `Message::Tab` empties the stack). The menu bar window is unchanged: a page
takes it whole, as it is narrow. **Checked:** `swift test`, `cargo test`. **OPEN:** tried in the apps.

**B153 (2026-10-28): any number of key binding sets in the recipe studio; a sidebar that never scrolls.** The studio
had two fixed choices "Key bindings 1/2"; recipe.toml v2 allows any number (a later one wins). `recipeStudio.keymaps`
is a list like the modules: a row per set with Earlier, Later and Remove, a menu of the sets not in it to add, each a
`ui.field` on `values.keymaps` with the whole list. The app window's sidebar is no scroll view any more: its full height
is the content's least and, with `windowResizability(.contentMinSize)`, the window's (German labels included); the
Linux app's window is at least 720 high. **Checked:** `cargo test`, `swift test`. **OPEN:** tried in the apps.

**B154 (2026-10-28): review before 0.7.0.** Two read-only reviews (the changes since 0.6.2; the three UIs against each
other), then fixed:
- Key binding studio: a set of yours only for macOS was saved empty (the platform was always NixOS, and only the shown
  platform's installed modules were written) – the platform now follows the set's modules, and what the base names for
  modules not shown is kept. "Apply" or "The module's own" on an action without `after` wrote `after = <its mode>` –
  staying in the mode is no change now. Taking out the last key kept the old one; a key "/" ("Ctrl+/") could not be
  typed (keys are apart by " / " now).
- Studio tabs: a studio opened as a page (Edit, Copy, New …) shared its form values with the tab of the same name – the
  tab's values wait aside now and come back (SwiftUI `tabValues`, iced `tab_forms`); Save and Cancel on a tab start it
  afresh (they did nothing there).
- Catalog: Reset Filter of the catalog's recipes reset the Build Images tab; a look of the studio lacked `license` and
  empty `tags` (look v1 asks for both); a key binding set or look that is not one is checked before it replaces the one
  on disk; action names in keymap.toml are quoted; the recipe studio's search starts on the first page (`pageSearch`);
  no build while the Nix Store is cleaned up.
- Linux app: the search in recipeCatalog, modules and keymaps went to the wrong filter (fixed page names); each catalog
  tab fetches its catalogs once (it was one flag for all); the key editor's composed key replaced the keys and lost to
  typed text – now "Use as the Key" or "Add as Another".
- `omaf ui`: a field's help with the help switch on; "↩" for keys that go back to the start mode; chips with their
  count; the catalog tabs recipeCatalog and keymaps fetch their catalogs; icons `down`, `chevron-*`, `update` (also in
  the macOS app, with `arch`). The macOS key editor's "after" switches name staying and going back by their labels.
- Since B158: the look studio's gradient is a switch (back to one colour in the macOS app too); the Nix clean-up's and
  the builder's activity lines are translated.

**B156 (2026-10-29): modules, key bindings and looks too old to read can be updated.** After the update to 0.7.1 the
maintainer's twelve macOS modules (1.0.0, module.toml v1 – installed before 0.7.0, whose catalog v5 has them as 1.0.1
in v2) showed only "cannot be read: module.toml has version 1 – too old for this omaflake", on the Modules tab and,
mixed in, on the Key Bindings tab; a part that cannot be read did not count as here, so neither its update nor
"Install All" reached it (only Refresh, then Install on page 2 or 3). Now (`modules_answer`, `looks_answer`): a part
in the data folder that cannot be read, which an enabled source's catalog has, is a row here – the catalog's entry,
`update` set, `broken` (new in `PartDto` and `LookDto`) saying why – with a warning lamp, only Update and Remove, and
"Cannot be read" in its details; the head has **Update All** (`modules.installAll` / `looks.installAll` with every
update of the tab, on every page). What stays unreadable shows on its own tab only (a key binding set's id has
`.keymap.`), and before the sources were looked into it offers "Look for an Update" (Refresh). The views still never
fetch on their own. **Checked:** `unreadable_parts_are_updates`, the golden files, `cargo test`, `swift test`; a copy
of the maintainer's data folder with a development `omaf serve`: after Refresh the twelve macOS modules were rows with
"update", Update All installed all twelve (1.0.1), no warning left. Recipes: their own updates already were rows of the
Recipes tab; recipe v1 is not read since 0.6.0 (unchanged). **OPEN:** clicked in the apps.

**B155 (2026-10-28): a loading screen while a NixOS guest boots.** The window was black until Hyprland drew. The guest
module (`nix/modules/vm-guest.nix`, option `vm.bootSplash.enable`, default on, every NixOS image) now shows Plymouth's
`spinner` theme with omaflake's white mark (`nix/modules/omaflake-splash.png`, written by brand/generate.py; replaced
on 2026-10-29 by omaflake's own theme, below), loads
`virtio_gpu` in the initrd so it shows from the kernel's start, and boots quiet (`quiet splash`, console log level 3,
udev and systemd status quiet in the initrd) – the kernel lines stay in the journal, the serial log has fewer of them.
macOS guests need none: Virtualization.framework shows Apple's boot screen. Checked: `nix flake check` (both flakes,
with the catalog), `nix eval` of a recipe's `boot` (plymouth, theme, initrd module, params). **OPEN:** tried in a VM
(Plymouth with virgl through ANGLE on a Mac); how long the window stays black before it (firmware, before the kernel) –
a loading screen from QEMU itself (a patch to its window) if that is long.

*Tried in a VM (2026-10-28, image research-walx built from the checkout, window frames every 0.8 s):* Plymouth runs in
the guest (`plymouth-start` 0.2 s after the kernel, `virtio_gpu` from the initrd, fb0) – but with 3D
(`virtio-gpu-gl-pci`, `-display cocoa,gl=on`, the default) the window stays black for about 20 s, until Hyprland draws:
neither the firmware's logo, systemd-boot's menu, Plymouth nor the text console show; that is also why no boot text was
ever seen. `systemd-analyze`: 6.0 s firmware + 5.1 s loader (the menu's 5 s wait) + 0.2 s kernel + 2.8 s initrd + 3.6 s
userspace. Without 3D (`--no-gl`) the firmware's TianoCore logo and progress bar, the boot menu and the console show;
Plymouth's screen was not seen there either (OPEN: whether Plymouth draws through virtio-gpu at all).
- Done: `boot.loader.timeout = 0` (mkDefault) – the boot menu's 5 s of black are gone; a key held at boot still shows
  it (systemd-boot's behaviour, from its documentation – not tried).
- **Plan: a loading screen from QEMU** (`qemu/patches/omaflake/`, on top of try-omarchy's `qemu-texture-borrowing`):
  1. Find why nothing shows before the first GL scanout. The Cocoa GL code does draw the 2D surface while there is
     no borrowed scanout (`cocoa_gl_render`: `surface_gl_render_texture`), so the 2D updates (firmware GOP, fbcon,
     Plymouth's dumb buffers) do not reach it with virgl, or are not flushed – `COCOA_GL_TRACE=1` (the patch's own
     log) shows renders and scanouts; QEMU must get it in its environment (the core starts it: an `omaf` debug switch
     or a `VM_QEMU_ENV`).
  2. If those updates can reach the window: fix that (then the firmware's logo and Plymouth show – the boot looks
     like a normal PC).
  3. Either way: `qemu-cocoa-loading-screen` – the window shows omaflake's loading image (`OMAFLAKE_LOADING_IMAGE`, a
     PNG set by omaf and the app like `OMAFLAKE_DOCK_ICON`; brand/generate.py renders it: the mark on the brand's
     background) from the moment it opens until the guest's first frame that is not empty (the first borrowed
     scanout, or a 2D update that is not all black), with a short fade.
  4. Both QEMU builds (`package.nix` for users, `build.sh` for development), `qemu/README.md`'s patch table, a VM
     check with frames as above. A new QEMU reaches users through the Doctor's QEMU item (`nix profile` of
     `#qemu`).

  *Step 1 done (2026-10-28, QEMU run by hand with the VM's arguments and `COCOA_GL_TRACE=1`):* the 2D frames do reach
  the window – `scanout texture id=1 640x480` (the firmware), then `id=2 3840x2160` with flushes of a centred
  193×58 region and the bottom (Plymouth), about 110 renders in 20 s; Hyprland then flips between its own buffers
  (`id=7/8/9`, 2880×1620). Yet those first frames show black. Tried and dropped: an opaque `CAOpenGLLayer` and alpha
  forced to 1 (patch `qemu-cocoa-opaque-gl`, built and run – still black for 18 s; moved to `_to_delete/`), so it is
  not the layer's alpha: the borrowed texture of a resource the guest fills with 2D transfers is black itself, while
  those of 3D-rendered buffers are not. **OPEN**, next: why the 2D-filled virgl resources' textures stay empty on the
  Mac (virglrenderer's native-OpenGL patch – contexts, `copy_transfer3d` into texture 3553); else step 3 with a
  heuristic end (the loading image until the guest flips between buffers of its own).

  *Cause found (2026-10-28, QEMU with virtio-gpu trace and a texture read-back):* the early frames are 2D resources
  (`RESOURCE_CREATE_2D`, B8G8R8X8). `virgl_cmd_transfer_to_host_2d` uploads them with `glTexSubImage2D` in virgl's
  context 0, and nothing flushes that context; the window samples the texture from its own context of the same CGL
  share group, and CGL shows a shared object's new contents to another context only after the writer flushed – so it
  sees the texture as created, black. 3D clients (Hyprland) flush context 0 with their submissions. With `gl=es`
  (ANGLE's EGL) the firmware showed even before. Fix: patch `qemu-virgl-2d-flush` (p5,
  `virgl_renderer_force_ctx_0(); glFlush();` after each 2D transfer) – with it, in the same run as above, the
  TianoCore logo with "Start boot option" (4.7 s), systemd-boot's menu and the console show; Hyprland as before.
  Plymouth still drew nothing: no buffer of its own in the trace. Its cause: plymouthd forces text ("details") as
  soon as `/sys/class/tty/console/active` lists a serial console (`ply-device-manager.c`,
  `create_devices_from_terminals`) – ours (PL011 from ACPI SPCR, the serial log) is. The guest module now passes
  `plymouth.ignore-serial-consoles` with the splash. *Tried (2026-10-28, QEMU `#qemu` with p5, a new image of
  research-walx from the checkout, frames every 0.8 s):* black until 1.5 s (QEMU's start), the TianoCore logo with its
  progress bar until 6 s, about 1.5 s black (kernel), Plymouth's spinner with omaflake's mark from 8 s, Hyprland at
  13 s (before: about 20 s of black). So step 3 (a loading image in QEMU) is set aside.

  *One picture from the start (2026-10-29):* three more steps, wanted by the maintainer.
  - **The firmware's wait:** the progress bar "Start boot option" was EDK2 counting down its `Timeout` variable (5 s;
    `ArmVirtQemu.dsc`, `PcdPlatformBootTimeOut`). `GetFrontPageTimeoutFromQemu` (QemuBootOrderLib) takes QEMU's
    `-boot menu=on,splash-time=N` instead, in ms rounded up, and 0 skips the countdown (`PlatformBm.c`); omaf passes
    `-boot menu=on,splash-time=0` to every guest. Esc at the start still opens the firmware's setup (OPEN: not tried).
  - **omaflake's own loading screen:** a Plymouth script theme (`nix/modules/plymouth/`, theme `omaflake`): the hero
    flake large in the middle with the word mark under it, the brand's pixels spread over the whole screen, small
    flakes drifting down behind it, the red and cyan layers breathing apart with a short glitch every 3.7 s, a cyan
    bar near the bottom with the boot's progress (`Plymouth.SetBootProgressFunction`, from the last boot's times).
    brand/generate.py renders its images and its layout (`layout.script`, in units of the screen's height), so the
    script and the still image below place everything alike. The functions it uses were checked in Plymouth's
    sources (`script-lib-*.c`, `.script`); assigning a name inside a function changes a global of that name
    (`script_evaluate_var`), hence `global.` for the state.
  - **No firmware screen, no black before the splash:** instead of an own firmware build with omaflake's logo (EDK2
    for aarch64 built on a Linux builder, shipped with QEMU), QEMU hides the firmware: patch
    `qemu-cocoa-loading-image` (p6) shows `OMAFLAKE_LOADING_IMAGE` – the theme's first frame,
    `brand/png/loading/omaflake-loading.png`, embedded in omaf and written to `<data>/cache/loading.png` – from the
    3D window's first frame, through the firmware (its scanouts, resources 1 and 2) and the kernel's start (scanout
    disabled, about 1.9 s), until the guest sets a scanout of its own (Plymouth's, resource 4), then fades in 300 ms;
    after 60 s without one it fades anyway. Measured with QEMU's virtio-gpu trace before (2026-10-29): firmware
    0.45–6.2 s, disable 6.2 s, Plymouth's scanout 8.1 s, back to the console 12.2 s, Hyprland 13.4 s.
  - *Tried (2026-10-29, QEMU `#qemu` with p6 run by hand with a VM's arguments, a new image of research-walx from the
    checkout, frames every 0.6 s):* the loading image from the first frame (0.4 s, while the window is still
    640×480), the theme with its bar and drifting flakes from about 4 s – the hand-over shows no jump –, black from
    8.8 s, Hyprland at about 11 s (before: 13 s). The firmware's screen never shows.
  - **Accepted for now** (maintainer, 2026-11-04 – the loading screen is fine on macOS): the 2 s between Plymouth's end and Hyprland's first frame (getty's auto-login, zsh, Hyprland's start)
    stay black – `plymouth quit --retain-splash` does not help while Hyprland is not yet DRM master; Hyprland would
    have to take over from Plymouth (as GDM does).

**B129 (2026-10-24): clipboard on a GNOME host.** Mutter has no data-control protocol: `wl-paste --watch` ends at once.
`Wayland::start` now treats an end within 500 ms as refused, and `detect` falls back to xclip through Xwayland, which
Mutter keeps in sync (`X11::through_xwayland`, Mutter's `.mutter-Xwaylandauth.*` cookie when XAUTHORITY is unset; its
TIMESTAMP is listed but not served, so the content is compared every 0.5 s). linux/package.nix wraps `omaf` with
wl-clipboard and xclip too. Checked by hand on Ubuntu 26.04 / GNOME 50: both ways.

**Smaller items (2026-10-24, branch `c-small`):**

- ✅ Tokens of private sources on Linux: `source_tokens.rs` – `SourceTokens` over a `SecretStore` trait (Keychain on
  macOS; Secret Service via the pure-Rust crate `secret-service` 5.2, MIT/Apache-2.0, on Linux; same service/account,
  same label), fallback `$XDG_CONFIG_HOME/omaflake/tokens.json` (0600, folder 0700, schema `tokens` v1) when no Secret
  Service answers (no session bus, no service, 5 s/60 s limit on a thread of its own); `sources.add`'s note and
  `omaf sources add` say "stored unencrypted in … – no keyring". Tests with a fake store and temporary folders; the
  Linux code compiles (`nix build .#packages.aarch64-linux.omaflake-gui`), **not tried against a real keyring (OPEN)**.
- ✅ The Linux app avoids a software Vulkan adapter: `crates/omaflake-gui/src/gpu.rs` enumerates the Vulkan adapters
  before the window opens; all CPUs (lavapipe) and no `WGPU_BACKEND` set → `WGPU_BACKEND=gl` for itself. Not run in a
  VM yet (OPEN); once confirmed there, the lab recipe's `WGPU_BACKEND = "gl"` can go.
- ✅ Translations for `omaf ui` and the Linux app: crate `omaflake-i18n` (tables generated from
  `App/Resources/de.lproj/Localizable.strings` by `scripts/translations.py`, `--check` in CI, an up-to-date test),
  filled in like `ViewTextRenderer` (positional args, fit check, English fallback); language from `OMAFLAKE_LANGUAGE`,
  `LC_ALL`, `LC_MESSAGES`, `LANG`; both UIs translate every view text and their own words (German added) and send
  `language` with `initialize`. Tried: `omaf ui` in German in a pty. Linux app in German not looked at (OPEN).
- ✅ Two versions of a recipe after a local edit: a name without words means the newest version of its line
  (`RecipeId::resolve_newest`, `RecipeLibrary::places`); `omaf build`/`export recipe` take it, `omaf new` the newest with
  an image; asked only between lines; words, IDs and omaflake IDs pick older versions.
- Open: "SSH port in use" right after a stop (in the Linux-host work), click tests of B119/B121.

**State:** M0–M4, M7, M8 and M10 done; M11 implemented, acceptance by click open; M9 done for `mac-minimal`; macOS bases, images and instances entirely via
`omaf` (BM27, BM28); M6 dynamic resolution, sound/microphone, clipboard and camera for NixOS done (B35–B37); the app
is one menu window with navigation and the brand look (B42–B47), admin tasks for the Linux builder (B40), `lab` is
the development recipe with the "omaflake Test" app (B43). omaflake icon in the menu bar, the app bundle and the Dock icons of `omaf-runner` and QEMU (B27).

**Before signing (2026-10-17):** everything that needs no Developer ID is implemented. What is left needs the
maintainer – clicks, passwords, decisions – or the signature:

- *Maintainer, to check:* M11 acceptance by click (incl. navigation, versions, developer mode, guest pickers);
  build `macos/lab` (guest password) and try clipboard and "omaflake Test" there; M5 acceptance (`dev` smooth by
  click); setup by click on a Mac without Nix (B54); optional: camera at 30 fps (B37/B49).
- *Decided 2026-10-17:* minimum macOS 27 (Package.swift, Info.plist); startergo's patches under MIT (stated in his README, NOTICE added). Trademark search 2026-10-19: DPMA and TMview without hits for "omaflake".
- *Decided 2026-10-19:* omaflake is a project of **falkOSS** (falkOSS – Falcon Lake Open Source Software
  gUG (haftungsbeschränkt) i. G.); the gUG's Apple Developer organization signs the releases; domains:
  **omaflake.app** (main), omaflake.org/.com/.dev forward to it. omaflake keeps its own name, logo and design – falkOSS
  appears only as the publisher (copyright, "a falkOSS project"). Wording stays VM / image (Vorlage) / recipe (Rezept) / flake, actions "start / create / build" –
  no baking terms. Direction for own icons (later, no hurry): play with snow, the logo being a flake – VM = snowman,
  image = snowball, recipe = snow; the flake itself stays the Nix flake (not shown in the app).
- *Maintainer, to decide:* hosting of releases, cache and installer (spec §11.8, own mirrors – maintainer); BM25 (Nix background items in macOS clones).
- *Done without the signature (2026-10-17):* camera throughput (B70), release dry run (B71), CI/CD workflows (B67, not run yet – the token needs the `workflow` permission to push them), pinned nix-installer (B66), update
  check (B68), file pickers as sheets (B69), BM25 researched (no fix upstream; profile test OPEN).
- *Needs the signature (M12):* Developer ID signing and notarization, Homebrew cask, the privileged helper
  (spec §11.5), the camera extension for macOS guests (B49), self-update with signed releases (spec §11.6), then the
  public repository and the recipe line-up (spec §11.7).

Order (one milestone at a time, acceptance criterion ticked in spec §7):

1. **M11 acceptance** – click through everything from M10 in the app (plan §7 M11), after a clean start from scratch –
   now with the navigation, the guest pickers and the Linux builder actions.
2. **Prerequisites (spec §11.2)** – done 2026-10-16 (B29): `omaf doctor`, Requirements page, `nix build .#qemu`.
   Open: binary cache for the QEMU package; a 3D window with the Nix-built QEMU checked by eye; the startergo
   license question also covers the vendored `qemu/patches/startergo/` patch.
3. **M9 remainder** – macOS and NixOS recipes `dev` and `office` – in omaflake/recipes now (B107): planned, later
   (2026-10-17: research and the app first; the old NixOS `dev` module is in this repository's git history, commit 9b560fd); BM25 (Nix background-item notifications); a base with the
   maintainer's own user (stays local, never in the repo).
4. **M5 close-out** – done except the acceptance by click: `dev` starts by click with smooth Hyprland (maintainer).
5. **M6 comfort** – camera for NixOS done (B37; 30 fps throughput OPEN),
   clipboard for macOS guests done (B48), image retention done (B52), camera throughput measured (B70).
   Camera for macOS guests: the same Mac side, in the guest a CoreMediaIO camera extension – needs Developer ID
   signing (system extension), so after the developer account (M12); a faked USB camera is not possible
   (Virtualization.framework passes through only real USB devices, macOS 27).
6. **Setup by click** (spec §11.5) – implemented (B54); to try on a Mac without Nix. After M12 the privileged helper
   replaces the dialog.
7. **M12 and release** – ANGLE from source done (B55), name checks partly (B57); self-update (spec §11.6); `vmRecipes` interface (spec §11.1), build ANGLE ourselves, startergo license question,
   Developer ID signing and notarization, Homebrew cask, minimum macOS version, name checks (domain, registries,
   trademark), switch the repository to public, GitHub org avatar and social preview (`brand/social/`).
8. **Recipe sources (spec §11.3)** – done (B107): omaflake/recipes as the default source and curated catalog, other
   GitHub repositories (also private, with a token in the Keychain), HTTPS folders and local folders; install, update,
   remove; "Install suggested" at the first start; exporting and importing recipes as packages (B108). Open: CI in
   omaflake/recipes that evaluates and builds each submitted recipe, signing or pinning a catalog, private sources on
   other hosts. Later: generic images with
   first-start personalization and a binary cache for NixOS images; the macOS base stays independent of any recipe.
9. **omaflake agent in every guest (spec §11.9, direction 2026-10-18)** – one agent for NixOS and macOS over one virtio
   port instead of today's clipboard and camera helpers; readiness without SSH, live status, tunnels from the Mac;
   carries the macOS camera once its Camera Extension is signed (M12). Its own milestone. Design done, NixOS
   prototype (B96), clipboard and camera over the agent for NixOS (B106); macOS: Mac side and the agent binary done,
   open how it starts in the guest (BM25 – maintainer's decision) and a check in a macOS guest.

10. **Portable packages (spec §11.10)** – done (B108, B109): export and import (always a copy) of VMs, images and
   recipes, VMs running from where they are. To check by the maintainer: export/import by click, a macOS VM out and
   back. Open: recipes of the same ID from two sources side by side (the source as part of the identity).

11. **User interfaces and the core service (B119, B120)** – macOS: setting menu bar / window / both; Linux: iced app with
   tray (StatusNotifierItem) and a waybar module (`omaf status --waybar --follow` done 2026-10-23, B121); design tokens from `brand/` for both; `omaf serve` as one service
   per user, so every interface and the CLI share one build queue (B120 done 2026-10-23; open: checking the agent).
12. **Presentation model and TUI (B121)** – the core says what each page shows (done 2026-10-23: `views.get`,
   `views.subscribe`, api.md §10); the TUI draws it (done 2026-10-23: `omaf ui`, ratatui, macOS and Linux, and
   `omaf status --waybar`) and the SwiftUI app (done 2026-10-23, click test OPEN); next iced.

Small items on the way: check the Dock icon of QEMU/runner windows by eye (B27). The old QEMU builds p2/p3 and
`~/.omaflake/qemu/src/build-*` are gone (2026-10-18: `~/.omaflake/qemu/` holds only the link to the Nix-built QEMU).
