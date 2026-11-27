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

## [0.8.2] – 2026-11-27

Fixes for 0.8.1 and the app icon in Liquid Glass. The guest agent of a macOS VM – with it the camera, MIDI, the
diagnosis and the volumes under Locations – now starts after every start, also after a restart inside the VM or with
the app closed; long pages scroll; and macOS image builds no longer wait in vain for SSH when the core service has run
for a long time. Rebuild your macOS images to get the agent that checks volumes where they are.

### Changed

- **omaflake/recipes is gone** – the old recipe repository (deleted, replaced by
  [omaflake/catalog](https://github.com/omaflake/catalog) since 0.6.0) no longer counts as an official source; settings
  that still name it keep being moved to the catalog.
- **The app icon in Liquid Glass** – omaflake.app, its VM window and omaflake Tools in macOS guests show omaflake's icon
  in layers, as macOS 26 and later draw app icons: the flake as glass, its own variants for dark and tinted icons. A
  build without Xcode keeps the flat icon.

### Fixed

- **The agent of a macOS VM starts after every start** – after a restart in the guest, or a start with `omaf start`
  while the app was closed, the guest's agent (and with it the camera, MIDI and the diagnosis), the clipboard helper
  and the volumes under Locations did not come; the core service now starts them by itself.
- **macOS builds that waited for SSH in vain** – a core service that had run for hours sometimes no longer reached a
  new VM, and a macOS image build ended with "No SSH after 10 minutes" although the VM was up; omaflake now also asks
  through a fresh helper process and goes on when that one gets through.
- **Long pages scroll** – the diagnosis, a build and the macOS base page grew past the window's top when they were
  taller than the window, and the way back with them; they scroll now.
- **The diagnosis checks volumes where they are** – the guest agent looked for them under `/Volumes/My Shared Files`
  instead of `/Volumes/<name>`, so the share check failed in VMs from 0.8.1 images (takes effect after a rebuild of
  the image).

## [0.8.1] – 2026-11-27

The camera in macOS VMs: smooth with OBS, and without OBS in an "unchained" VM; omaflake Tools, omaflake's own app
inside a macOS VM, with its camera. An unchained copy of an image or a base is made by one click – omaflake goes
through macOS Recovery by itself and asks you only to approve the camera once. And volumes: a VM can have several
shared folders, and VMs can share them – in an unchained macOS VM each under Locations in the Finder, like an external
disk. Much of it is new and only partly tried on a Mac; the notes say where. Rebuild your macOS images to get the new
agent and the volumes, and upgrade their VMs.

### Added

- **Volumes: several shared folders per VM, shared between VMs.** A volume is a folder of this computer – a new one in
  `~/.omaflake/share/<name>` or one of yours. A VM has none, one or several, and several VMs can have the same one: the
  new tab **Volumes** lists them with their VMs, "Add VM" gives one to a VM, "Take Out" takes it away; the New VM form
  asks for a new volume, one there is, or none. A VM sees each volume at `/Volumes/<name>` – the same path in macOS
  and NixOS VMs –, by default the folder's name, or one you give it for that VM ("Rename …"). On the command line:
  `omaf volumes` (`create`, `rm`, `attach`, `detach`, `rename`) and `omaf new … --volume <volume>[=<name>]`. Removing a
  volume never deletes its folder. **Diagnose** checks every volume both ways. Changes count from a VM's next start.
  In the Finder of a macOS VM: in an unchained VM each volume is under Locations like an external disk (omaflake Tools
  brings an FSKit file system of omaflake's; signed ad hoc it needs a boot argument the unchained image sets – with a
  Developer ID later in every VM); in other VMs each volume is a favorite in the sidebar (macOS lists only its own
  "My Shared Files" under Locations). In the app every macOS image has "Make Unchained Copy …": a second image,
  the same but with System Integrity Protection off and omaflake's camera (`omaf images unchain <image>`). For
  many images: Images → macOS Bases → a base's "Make Unchained Copy …" once, then "Build
  Image" with the switch "Unchained"; unchained bases, images and VMs carry an orange badge. On the command line
  `omaf macos unchain <base>` makes a copy of a base
  with System Integrity Protection off (`bases/mac-unchained-…`): you type `csrutil disable` in macOS Recovery,
  which omaflake goes through by itself (it reads Recovery's screen, clicks and types; when it finds no way on it
  shows the window and the log says what to do), and approve omaflake's camera extension once – every image built from it with
  `omaf build <recipe> --unchained`, and every VM of such an image, is then unchained. For what a normal VM refuses:
  first of all omaflake's own camera in a macOS VM without OBS, before omaflake is signed – tried on a Mac: the VM
  of such an image shows the Mac's camera as "omaflake Camera" without another approval of the extension. Never
  the default: an unchained image is never its recipe's current image (`omaf new <name> --image <image>` makes a
  VM from it), and such a VM has macOS's own protection switched off. The job shows its steps (6, two of them
  yours only when needed: Recovery by hand, the camera's approval); the VM's window stays hidden until you are
  needed. Before Recovery omaflake sets its language to English with a US keyboard in the copy (and puts it back
  after). In Recovery's Terminal it first checks that Shift reaches the VM (a password with capitals or symbols
  needs it – in a first try on a Mac it did not, and csrutil said the authentication failed) and tries three ways
  of sending it before it hands over to you. Unchained is marked in purple, not in the orange of warnings. The way through Recovery was tried on a Mac step by step, the whole job in the app not yet.
  image.json and omaflake-base.json have new versions (`unchained`); an older omaflake cannot read them.
- **omaflake Tools in every macOS VM, with omaflake's own camera extension.** "omaflake Tools" is omaflake's app
  inside a macOS VM, with omaflake's icon; every macOS image built with this version has it in /Applications. Its
  first part is a camera extension that shows the Mac's camera to the apps in a VM directly, without OBS. macOS loads it only signed with a Developer ID, which omaflake does not have yet; ad hoc
  signed as now it loads in a VM whose System Integrity Protection was switched off in Recovery – tried there: a
  smooth picture in Photo Booth. Until then the Tools in an ordinary VM carry no extension, an unchained VM's do.
  Where the extension is active, **Diagnose** needs no OBS.
- **`omaf start <name> --recovery`** starts a macOS VM into macOS Recovery, in a window – for what only Recovery can
  change. Not for VMs that start from their image every time.

### Fixed

- **Upgrade in place did not work for macOS VMs** – the VM's Nix refused the new system: nix-darwin had dropped the
  guest user from its trusted users. Images now keep it, and VMs from before are let in during the upgrade, which
  also waits for the VM's Nix right after a start. `omaf upgrade --keep` printed nothing and exited 0 when the
  upgrade failed; it now says why and where the log is.
- **A larger disk was never taken by a macOS VM** (0.8.0): the guest's launch daemon ran before the Nix store was
  mounted. Images built with this version wait for it.
- **"sh can run in the background"** at the start of macOS VMs: the image's launchd jobs now start through omaflake's
  own start script and are listed as "omaflake" – no such notice any more.
- **An app replaced by a build of the same version kept the old core service** – it took the service's word for being
  the new one (the service read its identity from the file on disk, which was the new one by then); the new build's
  changes did not show until a restart. The service now takes its identity when it starts.
- **Camera in macOS VMs: one picture a second, and late.** OBS draws with OpenGL unless told otherwise, and a macOS
  VM has no OpenGL on its graphics device: of 3962 pictures of the virtual camera OBS drew 142. The agent now
  chooses OBS's Metal renderer (OBS 32 and later; in OBS's settings while OBS is not running – quit OBS once), and
  **Diagnose** says when OBS still draws with OpenGL: 1725 of 1727 pictures drawn, 30 a second. And OBS's own H.264
  decoder held every picture back for about 300 ms; the agent now decodes the Mac's H.264 itself (VideoToolbox) and
  OBS reads raw pictures again – from the agent to an app in the VM 79 ms instead of 382 (measured with a test
  picture; the Mac's part of the way is not in these numbers). Needs an image built with this version.

### Changed

- **omaflake's crates are on crates.io**: `cargo install omaflake` gives the command line `omaf` (on Linux
  `cargo install omaflake-gui` the app), and `omaflake-sdk` lets a program of yours talk to omaflake's service – with every type of the API and the page
  models omaflake's own interfaces draw.
  The Rust package of the command line is called `omaflake` now (`cargo build -p omaflake`); the program is `omaf`
  as before.
- **The share folder is a volume now** – no longer at `/Volumes/My Shared Files` (macOS VMs) or `/mnt/share` (NixOS
  VMs), but at `/Volumes/<name>`, by default the name of its folder: a VM from before keeps its folder as a volume
  named after the VM. **Rebuild your images**: an image built before 0.8.1 does not mount volumes. instance.json has a
  new version (`volumes`); an older omaflake cannot read it.
- **omaflake knows nothing of Docker.** A Docker VM is a VM from the catalog's recipe like any other; the networks'
  details no longer show a `DOCKER_HOST` line. The catalog's new module `docker-client` (macOS and NixOS) brings the
  Docker command and points it at `127.0.0.1:2375` – give the VM the port of the Docker VM (`2375=<docker VM>` in its
  ports of other VMs), and give both VMs the same volume under the same name for bind mounts.
- **Networks are edited where you see them.** In the picture of the Networks tab you draw a line from a VM's dot to a
  network to add it, and × on a line takes it out (the line turns red while the pointer is on its ×). An opened
  network lists its VMs a line each – state, address, traffic and "Take Out" – and "Add VM" opens the list of the
  others right there; the page with one drop-down and the row of "Take … Out" buttons are gone. A running NixOS VM
  changes at once, a macOS VM at its next start (the help of each entry says which). The same in the macOS app and
  the Linux app, which now draws the picture too.
- **German: "entfesselt"** for unchained images, bases and VMs.
- **The agent's log in macOS VMs** is `~/Library/Logs/omaflake/agent.log` instead of `/tmp/omaflake-agent.log`,
  with the time on every line; the logs of the four starts before stay beside it (`agent.1.log` …), so what
  happened before a restart can still be read.

## [0.8.0] – 2026-11-25

A guided first start; VMs that grow, upgrade without losing their files and find each other by name; ports of other
VMs on localhost; networks edited by dragging; the Linux builder checked by itself; and omaflake with fewer
dependencies. Much of it needs images built with this version and is not yet tried on a Mac – the notes say where.

### Added

- **A larger disk for a VM:** the VM's details and `omaf set --disk <GB>` grow a stopped VM's disk (never shrink it).
  The VM takes the new space at its next start – NixOS VMs by themselves; macOS VMs need an image built with this
  version, which moves its system into the new space (it removes the fallback recovery behind it; the recovery inside
  the system stays). Not yet tried on a Mac.

- **The Linux builder checked by itself:** once a day omaflake checks in the background that the builder accepts this
  Mac's builds, so a broken one shows in the Doctor before a build runs into it; a builder disk smaller than its
  settings (made before them) gets "New Disk …".
- **Networks edited in the picture:** in the Networks tab of the macOS app, drag a VM onto a network to add it; the
  menu of an address takes it out again.
- **Ports of other VMs on localhost:** "Ports of other VMs" in a VM's details (`omaf set --forwards "5432=docker:5432"`)
  makes `localhost:5432` in the VM reach port 5432 of the VM docker in a shared network – handy for tools that only
  talk to localhost. From the next start; images built with this version.
- **Names and live joins in networks:** a VM reaches the others of a network by name – `<vm>.<network>.local` (macOS
  VMs at once, NixOS VMs with an image built with this version) – and a running NixOS VM joins or leaves a network at
  once instead of at its next start.
- **A guided first start:** a few questions – NixOS VMs, macOS VMs or both, omaflake's recipes and looks to start
  with, your user in the guests, developer mode – and one Set Up that does it all; the Doctor shows each step. The app
  opens it at its first start; the Doctor offers it while you have no credential ("Guided Setup …").
- **Modules by author:** the recipe studio's catalog has a chip per author ("@omaflake") when modules come from more
  than one.
- **Upgrade a VM and keep its files:** "Upgrade, Keep Files …" in a running VM's details (`omaf upgrade --keep`)
  copies the new image's system into the VM and switches to it – the disk, the files and the settings stay. NixOS VMs
  need an image built with this version once (it allows the switch without a password). Not yet tried on a Mac.

### Changed

- **No more packages of others for the macOS runner:** `omaf-runner` reads its command line itself, and omaflake
  writes hex and Base64 itself – three dependencies fewer; the others are at their newest versions at least two weeks
  old (rfd, ksni, lucide-icons, libc, toml, ureq).
- **Dates without chrono:** omaflake handles dates with jiff now – chrono is no longer maintained
  (RUSTSEC-2026-0543). Files and the API keep their date format.

## [0.7.6] – 2026-11-25

A fix release: NixOS images build again on a Mac whose Linux builder stopped trusting it – the Doctor recognizes it and
repairs it –, and the camera in macOS VMs is smooth (H.264) and no longer stays black when OBS starts slowly.

### Changed

- **Smooth camera in macOS VMs** (new, not yet tried with a real camera on a Mac): the Mac now sends the camera as H.264 instead of raw frames – the full 30 frames a
  second instead of about 25 with gaps, and a fraction of the load in the VM. omaflake switches OBS's source in
  "omaflake Camera" to the new format by itself while OBS is not running (quit OBS once after updating). Needs an
  image built with this version; NixOS VMs keep raw frames.

### Fixed

- **NixOS image builds failed with "lacks a signature by a trusted key":** a Linux builder whose Nix store got
  damaged stops trusting the Mac and refuses everything the Mac built itself. omaflake now recognizes it – a failed
  build says so, and the Doctor's "Linux builder running" turns red – and repairs it: **Repair …** gives the builder a
  new disk (omaflake's own builder as you, nix-darwin's with an administrator) and checks it again. **Test Build** in
  the Doctor and `omaf builder check` / `omaf builder repair` do the same by hand; `scripts/repair-linux-builder.sh`
  works without the new version.
- **Camera in macOS VMs stayed black when OBS started slowly:** the Mac stops the camera when the VM takes no frame for
  2 s, and nothing started it again while OBS kept reading. The agent now asks the Mac again (after 1 s, then up to
  every 8 s), and also when OBS reconnects. Needs an image built with this version.

## [0.7.5] – 2026-11-25

The camera reaches macOS VMs (through OBS until omaflake's own camera extension is signed), VMs can share networks of
their own – with a Docker VM for the macOS VMs –, and a NixOS VM can have several monitors, each in its own window. And
a fix: NixOS images could not be built with 0.7.4.

### Added

- **Camera in macOS VMs:** the camera switch now works for macOS VMs too (the VM's details, "New VM …",
  `omaf new/set --camera`, recipe.toml `camera`). Until omaflake's own camera extension comes with a signed release,
  apps in the VM see the camera as "OBS Virtual Camera": install OBS in the VM and start it once – omaflake's agent
  sets up OBS's scene collection "omaflake Camera" and chooses it; then approve OBS's camera extension in System
  Settings and click "Start Virtual Camera". The Mac's camera is on only while OBS reads it. **Diagnose** says which
  step is missing. Needs an image built with this version.

- **Networks between VMs** (new, not yet tried with real VMs): VMs of both kinds – NixOS and macOS – can talk to each
  other. Create a network in the new **Networks** tab (or `omaf networks create dev`) and add VMs to it (`omaf networks
  join dev <vm>`, `omaf new … --join dev`): from its next start each member gets a second network card there with a
  fixed address (`10.231.<n>.<host>`); its own network – SSH, the internet – stays as it is. omaflake runs each
  network's switch itself, without administrator rights. The tab shows who is connected to what, and the traffic.
  macOS VMs need an image built with this version (it sets up the new card at boot).
- **Several displays for NixOS VMs** (new, not yet tried on a Mac): a NixOS VM can have up to four monitors, each in a
  window of its own that you can move to another screen or make full screen there – set it in the VM's details or under
  "New VM …" (stopped VMs; `omaf new/set --displays 2`). Each window's size is that monitor's resolution; closing a
  window unplugs its monitor, QEMU's View menu plugs it in again. Needs omaflake's QEMU with patchset 7 (the Doctor
  offers the update). macOS VMs keep one display: Virtualization.framework allows no more (tried, also through its
  private interfaces).
- **Docker VM:** the catalog's new recipe `nixos/docker` – a VM without a window that runs Docker with Compose, for
  itself and for the macOS VMs in its networks (`DOCKER_HOST=tcp://<its address>:2375`, shown in the network's details;
  every VM in the network can use it as root). With the same folder of this Mac shared into both VMs,
  `docker run -v "$PWD":/app` works from the macOS VM.

### Fixed

- **NixOS images could not be built with 0.7.4:** a test of the guest agent's MIDI code expected an uppercase
  manufacturer id, which the agent rightly refuses; the test runs while the image is built, so every NixOS image whose
  agent was not already built failed ("builder failed with exit code 1", then "path … omaflake-agent-0.7.4 is not
  valid"). The test is fixed, and CI now runs the agent's tests.
- **Camera port:** a guest that read the camera's frames more slowly than for a moment (more than 1 s for one write)
  lost the camera for good – the Mac stopped it without a word and left half a frame in the port. Frames now go out
  whole or not at all; a guest that reads nothing for 2 s gets the status "stopped" and can ask again. Concerns the
  port `org.omaflake.camera` (macOS VMs, NixOS images from before the agent carried the camera).

## [0.7.4] – 2026-11-23

Setting up is one button now, and it shows what it does: **Set Up for NixOS** in the Doctor installs Nix (the only
time macOS asks for an administrator), sets up omaflake's own Linux builder as you – no nix-darwin, nothing in /etc –,
its SSH key, QEMU and a look. And MIDI: this computer's MIDI devices in macOS and NixOS VMs, both ways, with their
timing.

### Added

- **A macOS base per credential:** each credential can have its own base of a macOS version, so macOS images can be
  made for several users. A base can be created again (e.g. one from before omaflake's own SSH key) and removed; the
  build page picks the base of the credential you choose.
- **Cores and memory of a VM** can be changed while it is stopped (the VM's details, `omaf set --cpus --memory`).
- **Show the build VM:** a macOS image's build VM in a window, to see where it waits (`omaf build --window`).
- **Diagnose** for every running VM: one click (the VMs tab, `omaf diagnose <vm>`) checks the agent, SSH, the
  clipboard both ways, the share folder, the network, sound, microphone, camera, MIDI and the disk – through the
  omaflake agent, not SSH. Each check says what it found and what to do; sound and microphone ask you. The log is on
  the page like a build's, copied with one click and kept in the data folder's logs.
- **MIDI in macOS VMs:** this computer's MIDI devices – a stage piano, a keyboard, a synthesizer, Bluetooth or network
  MIDI, another app's virtual port – appear in the VM as MIDI devices of the same name, both ways, while the computer's
  own apps keep using them; devices plugged in or out come and go in the VM. MIDI 1.0 and MIDI 2.0 pass unchanged
  (Universal MIDI Packets over the omaflake agent). Each VM also gets a pair "omaflake <VM>" on this computer, to
  connect its apps with the VM's. Off by default: the VM's MIDI switch in the app, `omaf new/set --midi`, recipe.toml
  `midi` (vm table). Needs an image built with this version.
- **MIDI in NixOS VMs** too – on a Mac (CoreMIDI) and on a Linux host (the ALSA sequencer): the devices appear in the
  guest's ALSA sequencer, MIDI 2.0 as UMP where the guest's kernel has it.
- MIDI keeps its timing: data carry the time they were played, and the other side plays them at that spacing (up to
  2 ms of the transport's jitter smoothed). MIDI 2.0 devices appear with their UMP endpoint and function blocks where
  the guest can show them (macOS 15+, Linux 6.5+).

### Changed

- **Set Up for NixOS** – one button in the Doctor sets up what NixOS guests need, one step after the other: Nix, Nix in
  your terminal, omaflake's SSH key, its QEMU, a look and the Linux builder. Steps that are done are skipped; each row
  shows what happens. macOS asks for an administrator once, to install Nix; then only your credential is left.
- omaflake no longer uses nix-darwin and no longer changes /etc/zshrc, /etc/bashrc or /etc/nix/nix.conf. It runs its
  own Linux builder as you, in the background (a LaunchAgent; `omaf builder setup`), with its files in the data folder.
  A Mac whose Nix already has a Linux builder (nix-darwin) keeps using it. See the Linux builder guide.
- Nix is installed with flakes, without touching your shell files, and with you among Nix's trusted-users (so omaflake
  can hand it its builder). "Nix in your terminal" adds the lines that load Nix to your ~/.zshrc instead.
- omaflake has its own SSH key for its VMs (`<data folder>/ssh`, no passphrase), created with a click. Image builds and
  macOS bases no longer need a key in ~/.ssh or an ssh-agent; your keys still go into the VMs.
- QEMU installs and updates in the app with its progress, no longer in Terminal.

### Fixed

- A new omaflake app no longer keeps using an older core service that still runs in the background: it asks it to end
  after its jobs and takes over (VMs keep running). The `omaf` command does the same.
- Stopping a VM whose clean shutdown cannot be asked (e.g. SSH refused) or does not end within 60 s forces it off and
  says so, instead of waiting.
- macOS VMs: the clipboard helper and the omaflake agent are started until they run, and a start that fails says why
  (in the VM's log and the activity list) – a VM from an older image could stay without clipboard and nobody knew. The
  Diagnose page offers to add omaflake's key to such a VM.
- macOS VMs with the German keyboard get the Mac's classic "German" layout (@ on Option+L) instead of "German –
  Standard", where @ and other keys sit elsewhere. Needs an image built with this version.
- A VM's own MIDI pair on this computer is named after the VM – macOS VMs all named it "omaflake vm", so two VMs with
  MIDI on had two pairs of the same name.
- The Doctor shows every running setup step on its row with what it does now – installing Nix no longer looks like
  nothing happens. The installer's output streams into the job.
- Errors of administrator steps come in plain words; "Operation not permitted" says to give omaflake Full Disk Access.
- A Nix install that stopped half way is recognised and can be removed from the Doctor (it made every new install fail).
- Waiting for the Linux builder ends with a message instead of building on and failing; a failed builder build names its
  first error.
- The Doctor says when a Mac cannot run VMs (no hypervisor, e.g. a macOS VM) instead of a builder that never answers.
- Job logs keep earlier attempts instead of replacing them.

- An image flake, a recipe source or the data folder in a folder whose path has a space (such as an external disk or
  a macOS guest's "My Shared Files") no longer stops the Doctor and the builds: `path:` references for Nix are
  percent-encoded.

## [0.7.3] – 2026-11-04

Small things that were still open: the Doctor offers to update an omaflake QEMU from before 0.7.1, the Modules and
Key Bindings tabs find parts whose files were changed, `omaf new` takes an image (also one imported from another
computer), and `omaf ui`'s keys scroll.

### Added

- `omaf new <name> --image <image>` creates a VM from that image. An image imported from another computer works this
  way, and with `-r <its recipe's name>` too, when no recipe here has that name: the VM gets standard values.
- The Doctor tells an omaflake QEMU from before 0.7.1 (without the loading screen) and offers **Update …**
  (`nix profile upgrade qemu`).
- The Modules and Key Bindings tabs find a module or key binding set whose files are not what was installed (changed
  by hand, or mixed up by the parallel installs of 0.6.0) and mark it *changed*, with **Reinstall**; Update All
  includes it.

### Changed

- The look studio's gradient of the active window's border is a switch: off goes back to one colour (the macOS app's
  colour picker could not).
- With `VM_FLAKE` set, `omaf build` builds in the terminal instead of in a running omaflake service, which would have
  built with config.json's flake.

### Fixed

- `omaf ui`: the keys (`?`) scroll in a small terminal – arrows, j/k, Page Up/Down, Home and End.
- The activity lines of the Nix Store and Linux builder clean-ups are translated (they were English in German too).

## [0.7.2] – 2026-10-29

Modules, key bindings and looks installed before 0.7.0 that this version can no longer read are updated from the app
now: Refresh in their catalog tab, then **Update All**.

### Fixed

- Modules, key bindings and looks installed in a format this omaflake no longer reads (the macOS modules installed
  before 0.7.0, say) can be updated: after Refresh they are rows with Update, and **Update All** updates every one at
  once – instead of only a warning. The Key Bindings tab no longer shows the modules' warnings.

## [0.7.1] – 2026-10-29

NixOS VMs no longer start in a black window: omaflake's loading screen shows from the window's first frame until the
desktop, and the desktop comes about ten seconds sooner. It needs omaflake's new QEMU – the Doctor does not offer it
yet, update it once in a terminal with `nix profile upgrade qemu` (it builds QEMU, about ten minutes) – and the
loading screen in the guest needs images built from this version on.

### Added

- NixOS VMs show omaflake's loading screen from the window's first frame until the desktop instead of a black window:
  the omaflake flake large in the middle, small flakes drifting behind it and a progress bar (QEMU patches
  `qemu-virgl-2d-flush` and `qemu-cocoa-loading-image`; images built from now on).
- NixOS VMs start about ten seconds sooner: neither the firmware nor the boot menu waits any longer (images built from
  now on for the boot menu; hold a key at boot to see it).

## [0.7.0] – 2026-10-28

Everything omaflake reads as data can now be made in the app: a **Recipe Studio**, a **Key Binding Studio** and a
**Look Studio** in the new Workshop. Key bindings know the modes of their programs – Zellij's are complete – and every
recipe, image and VM shows its keys as key caps by module and mode. The app is in four groups – Run, Workshop, Catalog,
Profiles – with a sidebar in its window; the menu bar keeps what runs. Image builds no longer fill the Nix Store, and
the Doctor cleans it up. **This version reads catalog v5 only** (module.toml v2, keymap v2): Refresh in the catalog
tabs gets the official catalog anew; config.json becomes version 6 (the help switch).

### Added

- A keyboard button at recipes, images and VMs opens their keys: by module and by the mode of its program, with
  the way to each mode (Zellij: Ctrl+g › p › Pane), where a key leads afterwards ("then Locked") and, on a click, what
  it does in the program's own words. The macOS app shows them in a window of its own, the Linux app and `omaf ui` on
  a page. The keys no longer fill a recipe's details.
- The tabs come in four groups: **Run** (VMs, images, activity), the **Workshop** (Build Images, Recipe Studio,
  Key Binding Studio), the **Catalog** (recipes the sources offer, modules, looks, key bindings, sources) and
  **Profiles** (credentials, locales, performance – before "Profiles"). The app window shows them in a sidebar; the menu
  bar window shows only Run, as three large tabs, with a button to the window for the rest; the Linux app has the
  sidebar too, `omaf ui` switches the group next to the tabs. Credentials, locales and profiles moved
  from the header, the sources from the Settings; the recipes here and those offered are tabs of their own, as are
  modules and key bindings.
- The Recipe Studio replaces the recipe editor: modules are found in the catalog like a kit – a search that suggests
  names while you type, tags and capabilities as chips, five modules a page that open their details – and added with
  one click instead of a switch each; the app shows the catalog and the recipe side by side. The recipe says while it is put
  together what is missing (with a button that adds the module providing it), what does not go together and which
  keys overlap, and shows its keys before it is saved. It is the workshop's first tab and opens from "New Recipe …",
  "Edit …" and "Copy to My Recipes …".
- The Key Binding Studio: key binding sets of your own. Starting from a set or from the modules' own keys, the modules
  it covers are cards that open to their keys by the mode of their program. "Change …" opens a key editor: record the
  key (macOS app) or put it together from modifiers and a key, add more keys for the same, and choose where it leads
  afterwards (Zellij: whether `n` in pane mode locks again or stays in pane mode). Overlaps are said while you choose. Only what differs from the
  modules' own keys is saved, in My Recipes' folder as `my/<name>`, which recipes name like any set.

- A help switch in the header, off by default: the pages' explanations show only while it is on, each in a box of
  its own. Warnings and what is missing always show. It is kept in config.json (now version 6).
- The modules tab opens a module's keys in the keys window.
- The catalog tabs – recipes, modules, looks, key bindings – are searched and narrowed by tags as chips, ten a page,
  instead of one long list.

- The Look Studio: looks of your own – the colours with the colour picker, the terminal's sixteen, the desktop's
  borders and clock, the editors' themes and macOS guests' system look, with a preview and a word when text is hard to
  read. A look of the catalog opens there as a copy. Everything omaflake reads as data – recipes, key bindings, looks –
  can now be made in the app; only modules are Nix.

### Changed

- The Recipe Studio takes as many key binding sets as you like – added from a menu, moved and taken out like
  modules – instead of two fixed choices.
- The app window is never shorter than its sidebar: no scrolling to reach a tab.

- In the app window the header and the sidebar stay while a page is open – the Doctor, the settings, an image –
  so another tab is always one click away.

- Keys are drawn as key caps everywhere – |Ctrl| |g| – named as on your keyboard (Ctrl, Opt, Shift, Cmd on a Mac;
  Ctrl, Alt, Shift, Super on Linux), never as "⌃g" or "Ctrl+g". Overlapping keys are shown the same way.

- Modules list all their key bindings, with the modes they work in and the mode they lead to (module.toml v2:
  `[modes]`, several keys and modes per action, `after`, `does`, `own`). Zellij's are complete – about 100 in 12
  modes – and its configuration is written from them, so a key binding set can also change where a key leads
  (`new-pane = { after = "pane" }` stays in pane mode). Key binding sets v2: a key, several keys, or `{ key, after }`.
  This omaflake reads catalog v5 only; older catalogs are not read (before 1.0 without a transition).

### Fixed

- The app window opened from the menu bar ("Open omaflake") is in the Dock and the app switcher while it is open.
- German: "Locales" are "Regionen" now.

- In the studios shown as tabs, choices did nothing – "Change …" at a key, the search, adding a module: the tab was
  asked without them.

- Building images no longer fills the Nix Store: each build left its disk there (several GB) after copying it into
  the data folder. omaflake now removes it after the build, and the Doctor shows what the Nix Store holds with
  "Clean Up …" for what earlier builds left.

- After a macOS update the Doctor no longer calls the IPSW missing: an older one keeps creating bases, and a newer
  macOS from Apple is offered as an optional download.
- Downloading macOS on its own needs only the space of the IPSW, not that of a base and an image too. A download
  that fails says why at the Doctor's row instead of only in the activity log.

## [0.6.2] – 2026-10-26

A fix for a Mac whose Nix Store was not mounted at `/nix` after a restart: Nix, omaflake's QEMU and the installed
recipes seemed gone, and NixOS VMs ran slowly without 3D. **If the Doctor says "Nix Store volume not at /nix", click
Mount …** – nothing has to be installed again.

### Fixed

- Recipes no longer show as not installed when Nix is unavailable (e.g. its volume is not mounted after a restart):
  the recipe list is read from the recipes' own files, not by evaluating them with Nix. Only building needs Nix.
- Without a QEMU chosen in the settings, omaflake takes the first QEMU with 3D it finds (its own in the Nix profile)
  instead of the first on `PATH` – a Homebrew QEMU earlier on a terminal's `PATH` no longer starts NixOS VMs without 3D.

### Changed

- A recipe's build button stays when the Doctor reports what building needs (Nix for NixOS recipes, a macOS base for
  macOS recipes): greyed out with a warning sign, the reason in its tooltip – instead of a Doctor button in its place.
- The Doctor tells a Nix that is installed but not mounted at `/nix` from a missing one ("Nix Store volume not at
  /nix") and offers **Mount …**: it mounts the volume and restores what mounts it at startup (`/etc/fstab`,
  `/etc/synthetic.conf`, the `org.nixos.darwin-store` service) through the administrator dialog. API:
  `builder.mountNix`, job kind `builder-mount-nix`.

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
