---
title: Menu bar app
nav_order: 7
permalink: /app/
---

# omaflake – menu bar app (spec M4, M11)

SwiftUI `MenuBarExtra` on top of `OmaflakeCore`. No Dock icon (`LSUIElement`), no App Sandbox, ad-hoc signed.
Deliberately **without an Xcode project**: SwiftPM executable + `scripts/bundle-app.sh` (a hand-written `.pbxproj` would be error-prone).

```sh
scripts/bundle-app.sh             # → build/omaflake.app
scripts/bundle-app.sh --install   # additionally to ~/Applications (old bundle → ~/.omaflake/_to_delete)
open build/omaflake.app
```

## Features

- One window, one size (500 × 600 pt), content 20 pt from the edge: the menu bar window. Doctor, Settings, New VM, Build Image, Create macOS Base
  and the guide open one level deeper inside it with a back arrow (also Esc) – no extra windows, which used to close
  the menu window when they were closed. Closing the menu window keeps the page; long work keeps running. The first
  launch opens on the Doctor. Look: omaflake's brand colours (dark `#121212` with a faint red/cyan glow, cyan accent,
  window corners like Finder/Safari (24 pt), cards and buttons concentric with them (10 pt) – the window's own system background is hidden so the
  brand background's shape is the window's shape – and the wordmark with its red/cyan offset)
- Recipes show an "i" with their description (tooltip; also below the pickers) and a flask when they are lab
  (development) recipes; those appear only with Settings → Developer Mode
- Image rows (plan B93): a click on the row opens its build details (recipe, build time, recipe state with
  uncommitted changes, status, VMs on it, size on disk, source, system/disk image store name) and "Remove …" (red;
  not while VMs use it, plan B102); building
  lives in the Recipes tab; after a build only the current, the previous and used images of a recipe are kept (plan B52)
- "New VM" is opened from an image row and creates from that image (no picker); "Build Image" is opened from a recipe
  row or a build row and builds that recipe (no picker); there is no default image (plan B100)
- Main window: header with icons (Doctor with an orange dot when something needs attention, Settings, data
  folder in Finder, Quit – asks first while VMs are running), then four tabs, each one list that scrolls on its own (plan B91);
  the app remembers the last tab, the very first time (no VM yet) it opens on Recipes:
  - **VMs** (badge: running VMs); a VM is created from an image row (plan B100)
  - **Images** (Vorlagen; badge: spinner while a build runs, red ✕ after a failed one): every built image on its own
    row – full name, recipe, build time, how many VMs use it (green lamp: at least one), "outdated" when its recipe
    changed; "New VM" creates a VM from exactly that image; build details, remove; builds on top (plan B100)
  - **Recipes** (Rezepte; badge: orange dot when a recipe changed since its image): what can be built – name,
    author, lab flask, tags (the description when the row is open), whether its image is up to date, "build image" (package with a hammer); the folder icon
    shows the recipe's folder in the Finder (plan B103). Below the installed ones, what the remote sources offer
    (fetched when the tab opens or with the refresh button – never in the background): grey lamp, a seal on official
    recipes (by omaflake from omaflake/recipes), a download button; details with source, license, requires, files and,
    for unreviewed sources, a warning. Installed recipes show their source and, when downloaded, "Update" (a newer
    catalog version) and "Remove …" (red) in their details. With nothing installed, "Install suggested" brings the
    official recipes (plan B107)
  - **Activity** (badge: dot for unread messages): what happened in this session, newest first, "Clear History"
- The newest message also shows for 6 s at the bottom of the main window (a click opens Activity)
- Notifications (Settings → Notifications, on by default; macOS asks once): VM ready (SSH answers), image ready, build
  failed, VM ended on its own – only while the menu window is closed
- Wording: recipe (Rezept) → image (Vorlage) → VM, the same in the app, the CLI, the files and the docs; until plan
  B92 recipes were called flavors; tooltips appear after 150 ms
- Doctor (the first page on the first launch; omaflake's version at the top): requirements, NixOS preflight (Linux builder answers, recipes
  evaluate, space in the Nix store) and macOS preflight (IPSW from Apple – download with pause/continue in its row –,
  base, space); one line per check – rows with actions open with a chevron into a set-off sub-line (hint left,
  buttons right), problems and running work start open. The base is created or
  continued on its own page ("Create macOS Base": IPSW from Apple or a chosen file, guest user, log); the Mac does
  not sleep during downloads and builds. Linux builder row: "Clean Up …" (unused store paths) and "Apply Settings …"
  (`darwin-rebuild switch` with omaflake's builder host; warns first when this Mac's nix-darwin system is not
  omaflake's) – both ask through macOS' administrator dialog (spec §11.5), log in `<data>/logs/linux-builder.log`;
  "Guide" opens the Linux builder guide (bundled, offline)
- What the Doctor reports blocks the actions it concerns: building images of a guest kind (macOS e.g. without a
  finished base, NixOS without an answering Linux builder) and starting NixOS VMs without QEMU. An orange warning sign
  appears on the Doctor icon and in place of the blocked action; it opens the Doctor, its tooltip names the reason.
  Checks run at launch, when the Doctor opens, on "Check Again", after every action and right before a build – not
  while idle
- Instance list of both guest kinds with status dot and guest-kind icon (snowflake = NixOS, Apple logo = macOS),
  read from `~/.omaflake` every 2 s → also shows VMs started via the CLI; SSH port (NixOS) or NAT address (macOS, while running)
- per instance, always in the same places: share folder (folder icon), start without window (terminal with a play
  sub-symbol; the terminal with SSH while running), start in a window (monitor with a play sub-symbol, NixOS: 3D if the
  QEMU supports it; shut down while running). Small icons next to the name show what is on (clipboard, sound,
  microphone and camera in orange, ephemeral, isolated/no network); an orange arrow when a newer image is there
- Each list has a filter in its header (plan B101): status, system, and a tag (recipes) or a recipe (images); the
  header shows "3/6" while filtered. The filter sits right next to the title and count, apart from the list's actions
  (import, refresh, …) on the right
- Every row starts with a status lamp and the guest kind and is at most two lines high (plan B99): green = on / active /
  built, grey = off / not active / not built, orange = needs attention, cyan pulsing = busy.
- Every row (VM, image, recipe) opens with a click into its details (plan B94). A VM's details: its switches as chips
  (clipboard | sound, microphone, camera), image, recipe, resources, network, SSH command, share folder, created, and
  reset, switch to the newer image (when there is one) and remove (red) – there is no "…" menu any more
- Sound, microphone, shared clipboard and camera per VM: switches in "New VM …" and in the VM's details (from the next start). Clipboard: both guest kinds (text, PNG); camera: NixOS guests, on only while an app in the VM uses it
- Packages (spec §11.10, plan B108, B109): "Export …" in the details of a VM (stopped), an image and a recipe, into a
  folder the user picks (an external disk, a NAS, a local folder); the dialogs say who it is for (VMs and images stay
  with their person, recipes are shared). A VM's details have one action row: Export on the left; Stop Listing (external
  VMs), Upgrade, Reset and Remove on the right
- Import – always a copy into the data folder: an Import button among the actions of the VMs, Images and Recipes tab
  (the open panel offers only that kind and checks the content before it closes), and double-click on a package in the
  Finder or dropping it on the app. What is here already is asked about first: a VM of the same name – cancel or a new
  name (a second VM with its own identity); the same image – cancel or replace; recipes of the same ID with other
  content – cancel or replace (identical ones are simply there already). Without the menu window open, questions come as
  an alert
- omaflake IDs (spec §11.11, plan B110, B111): the details of a VM and an image show its omaflake ID, with as few words
  as tell the objects of its kind apart (the whole ID in the tooltip). The Recipes tab has one row per recipe line, by
  name – its source only when a recipe of the same author, kind and name comes from another source; its details list the
  line's versions as sub-rows, also when there is just one: their words, when they came (or "as it is in the folder"),
  newest, the images built from them, Export and Remove (not for what images use or a local folder's own recipe), and
  "Remove All Versions …" below. Settings → "Keep older recipe versions" (off: only the newest and what images use stay)
- External VMs: the VMs tab's second button runs a VM package from where it is (an external disk, a NAS, a folder –
  nothing is copied; a warning for network volumes). They carry a drive icon and "Runs from" in their details; Stop
  Listing drops them, Remove deletes them at their place
- Limit of two running macOS VMs (plan BM16): start is disabled before a third, with the reason as tooltip
- macOS guests are named after the instance once SSH answers (plan BM28)
- "New VM …": recipes grouped by guest kind; name, ephemeral, CPU/RAM/disk/network/share prefilled from the recipe's `recipe.toml`
  (macOS: disk as the image, no `isolated` network)
- Builds queue: one per guest kind at a time (NixOS and macOS in parallel), the rest waits; each shows as a sub-row
  of its image with progress, a click opens its log
- "Build image …": recipe (with guest kind) on top, password (macOS) and Build/Cancel below, then progress – step
  "2/4" with its name, packages done of announced, a bar – and the log as a coloured terminal; NixOS `nix build` and the macOS image pipeline (asks for the guest user's password once, never
  stored), live log; both can be cancelled – in that window or with the ✕ in the image row – which stops the running
  step, forces a macOS build VM off and deletes the half-built folder
- "New VM …": share folder = default `share/<name>` or a folder chosen in the open panel – the only way the app
  touches protected folders like Documents or Desktop
- Gear menu: launch at login (`SMAppService.mainApp`), "Install macOS Base …" (`omaf macos install`: user, full name,
  password, live log; fully automatic, BM29), Settings, show the data folder in Finder, quit. An image row's menu
  deletes the image ("Remove Image …", refused while VMs use it) or older images of the recipe. "Build Image …" and
  "New VM …" offer it as well when a macOS recipe has no base yet
- Requirements (opens by itself on the first launch, later via the gear menu or the orange note in the menu when
  something is missing): the checks of `omaf doctor` with status, path/version found and a hint; "Install …" types the
  official command into Terminal (the app never installs with administrator rights), "Set Path …" sets the nix
  binary, QEMU or the image flake, "Guide" opens docs/linux-builder.md
- Settings → Recipe Sources (first section, plan B107): the sources in their order with an on switch, an arrow to move
  one up (earlier wins for the same recipe), remove (its token leaves the Keychain); a lock on private ones (no token:
  a warning lock). Add: GitHub repository, HTTPS URL or folder ("Folder …" opens a sheet); for a GitHub repository
  "Private repository" shows a token field – "Check and Add" checks the repository and the catalog with the token and
  keeps it in the Keychain, never in a file. Footer: recipes from other sources are not reviewed by the omaflake team
- Settings: guest user in the VMs (user, full name, e-mail as text; language and region, keyboard, time zone chosen
  from lists that offer only what NixOS and macOS guests both support (`GuestChoices`, plan B46); "Like this Mac" or empty = the Mac's value;
  SSH keys from ~/.ssh; macOS VMs use the base's user), language (System, English, Deutsch – stored as `AppleLanguages` in the app's defaults, restart button);
  data folder (default `~/.omaflake`, move it elsewhere or back; `DataFolder.move`)

UI languages: English (development language and fallback) and German, `App/Resources/<lang>.lproj/Localizable.strings`
(keys are the English texts). Messages from `OmaflakeCore` (errors) stay English, like the CLI.

`omaf-runner` is embedded as its own helper app (`Contents/Helpers/omaf-runner.app`, shown as "omaflake VM"; inside
`Contents/MacOS` a runner with a VM window made macOS take it for omaflake.app, plan B65), signed by `bundle-app.sh` with its entitlement
`com.apple.security.virtualization`; the app finds it next to its own executable (unless `omaf config --runner` is set).

Icons: the menu bar shows the omaflake template image, the bundle carries `brand/macos/omaflake.icns`
(`CFBundleIconFile`), and VM windows started from the app get the omaflake Dock icon (QEMU patch
`qemu-cocoa-dock-icon`, plan B27). The image data comes from `OmaflakeCore/Brand.swift`.

Quitting the app leaves running VMs running (QEMU in its own session, as with the CLI).
"Terminal (SSH)" controls Terminal.app via AppleScript – macOS asks for permission the first time.
