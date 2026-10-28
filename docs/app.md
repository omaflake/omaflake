---
title: Menu bar app
nav_order: 7
permalink: /app/
---

# omaflake – menu bar app (spec M4, M11)

SwiftUI `MenuBarExtra` and an app window, a client of omaflake's Rust core. By default no Dock icon (`LSUIElement`), no
App Sandbox, ad-hoc signed.
Deliberately **without an Xcode project**: SwiftPM executable + `scripts/bundle-app.sh` (a hand-written `.pbxproj` would be error-prone).

```sh
scripts/bundle-app.sh             # → build/omaflake.app
scripts/bundle-app.sh --install   # additionally to ~/Applications (old bundle → ~/.omaflake/_to_delete)
open build/omaflake.app
```

## Architecture

The app holds no logic of its own beyond presentation (plans B117, B121): it **draws the core's pages**. It connects to
**`omaf serve`** – the core service on its socket, started on demand with the bundled `omaf service start --json`, else
a private `omaf serve` over stdio (plan B120); the Rust `omaf` sits next to its executable, `Contents/MacOS/omaf`
(development: `OMAF_BIN`, or the repository's `target/{debug,release}/omaf` above the SwiftPM build). `OmaflakeKit`'s
`CoreClient` sends `initialize` (client `omaflake-macos`, the app's version, API version 1, the UI language, the update
feed from Info.plist), subscribes to the `jobs` topic only (for the build and base logs, `job.log`) and connects again
when the connection ends; quitting ends only its session.

**Pages.** `AppModel` subscribes (`views.subscribe`, docs/api.md §10) to `home` (the header's buttons, the tabs and
their badges), `activity`, the three lists `vms`, `images`, `recipes` (with the filter the user chose, so switching tabs
is instant) and the page open one level deeper (`doctor`, `settings`, `newVm {image}`, `build {recipe}`, `macBase`, with
the form values typed so far). Every `view.changed` replaces that page's model (`PageModel`, OmaflakeKit's `Views.swift`);
the SwiftUI views (`HomeLists.swift`, `Pages.swift`, `ModelViews.swift`) draw rows, lamps, badges, info lines, hints,
filters and fields from it with the design tokens (`BrandTokens`, `BrandStyle`, `Components.swift`). What a row says,
which buttons it has, whether they are enabled and why not, sorting, filtering, the questions' texts, the next free VM
name, the form's validation and the activity's wording are the core's.

**Actions** are generic (`AppModel.run`): an action's `ask`s first – a text or a password (twice for `repeat`) in the
in-window question, a folder, file or package in an open panel as a sheet (packages checked by the ask's `check`,
`packages.inspect`, before the panel closes; `multiple`: the action once per chosen package) – put into the params at
their `param`; then its `confirm` (the same in-window question, with an `input` field where it has one); then its
`method`: a core method, or a `ui.*` verb the app does itself – `ui.show`/`ui.back` (pages), `ui.step` (the import step:
`views.get import`, then its action), `ui.filter`, `ui.field` (a form field), `ui.reveal`/`ui.open` (NSWorkspace),
`ui.terminal` (Terminal via AppleScript), `ui.quit`. The answer's `then`: reveal `result.path`, run `result.shellLine`
in Terminal, or leave the page. A refusal shows in the footer line (the activity list has it already).

**Texts** are `{key, text, args}`: the key is this app's English source string, so `de.lproj/Localizable.strings`
translates it (`Bundle.localizedString`); args are filled in by `ViewTextRenderer` – numbers, data, times and sizes in
the app's language, nested texts translated first, asked params – also positional (`%2$@`) for translations that
reorder. A translation whose placeholders do not fit falls back to the core's English `text`.

What the app keeps: the session, which tab and page are open, the filter and form values, panels and questions, the
footer line and system notifications (an activity entry's `notification`, when the activity page says notifications
are on and the app is not in front), unseen activity, launch at login, the app's language, the frame (below), tooltips.
For screenshots and checks without clicks, `OMAFLAKE_PAGE=<page>` (with `OMAFLAKE_PAGE_PARAMS` as JSON) opens the app
on a page.

## Menu bar, window or both (plan B119)

Settings → **Show omaflake in** chooses the frame; the pages inside are the same `MainView` (tabs, pages, questions,
file pickers), only the frame differs. The choice is a per-user UI preference in the app's UserDefaults (`appFrame`), not
a core setting, and applies at once – no relaunch.

| Choice | What you get |
|---|---|
| **Menu Bar** (default) | The `MenuBarExtra` window at the omaflake icon (500 × 600 pt), no Dock icon (activation policy `.accessory`, like `LSUIElement`), no app menu. |
| **Window** | A regular window (`Window` scene `main`): resizable from 500 × 600 pt, remembers its frame, Cmd+W closes it, a click on the Dock icon opens it again. Dock icon and app menu (activation policy `.regular`; Settings… opens the Settings page, Quit asks like the power button when VMs run). Closing the window keeps omaflake running; it quits with Cmd+Q. No menu bar icon. |
| **Both** | The menu bar icon and the window. The menu window has *Open in Window*; the window shows that omaflake is in the menu bar too. Both show the same page. |

At launch the window opens when the choice includes it (`defaultLaunchBehavior`, no state restoration); the first
launch's Doctor opens in whichever frame is shown. Questions appear inside the open window (both frames show them),
file pickers as a sheet on the key window; with no window open, as an alert or a modal panel. Notifications come only
while omaflake is not in front (menu window closed, app window hidden or in the background).

## Features

- One window, one size (500 × 600 pt; the app window can grow), content 20 pt from the edge: the menu bar window. Doctor, Settings, New VM, Build Image, Create macOS Base
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
- Main window: header with icons (Doctor with an orange dot when something needs attention, Settings, data folder in
  Finder, Quit – asks first while VMs are running), then the tabs in two groups (plan B147): **Run** – VMs, Images,
  Recipes, Activity – and **Workshop** – Modules (with key binding sets), Looks, Credentials, Locales, Profiles. The
  app window has a sidebar with both groups; the menu bar window, narrower, switches the group above the tab bar
  (a dot at the other group when a tab of it needs attention). Each tab is one list that scrolls on its own (plan B91);
  the app remembers the last tab, the very first time (no VM yet) it opens on Recipes:
  - **VMs** (badge: running VMs); a VM is created from an image row (plan B100)
  - **Images** (Vorlagen; badge: spinner while a build runs, red ✕ after a failed one): every built image on its own
    row – full name, recipe, build time, how many VMs use it (green lamp: at least one), "outdated" when its recipe
    changed; "New VM" creates a VM from exactly that image; build details, remove; builds on top (plan B100)
  - **Recipes** (Rezepte; badge: orange dot when a recipe changed since its image): what can be built – name,
    author, lab flask, tags (the description when the row is open), whether its image is up to date, "build image" (package with a hammer); the folder icon
    shows the recipe's folder in the Finder (plan B103). Below the installed ones, what the remote sources offer
    (fetched when the tab opens or with the refresh button – never in the background): grey lamp, a seal on official
    recipes (by omaflake from omaflake/catalog), a download button; details with source, license, modules, files and,
    for unreviewed sources, a warning. Installed recipes show their source and, when downloaded, "Update" (a newer
    catalog version) and "Remove …" (red) in their details. With nothing installed, "Install suggested" brings the
    official recipes (plan B107). Since plan B142 a recipe's details say what it is made of – its modules, look, key
    bindings, what a VM needs at least –, what is missing ("Install Missing Parts"; the lamp is orange) and what is
    wrong (warnings); its keys open in a window of their own (the keyboard button, plan B147); "New Recipe …" and
    "Copy to My Recipes …" (on one of yours: "Edit …") open the recipe studio
  - **Recipe Studio** (the workshop, plan B147): a recipe put together like a kit – its name and platform, the
    modules in it with what is missing (and the button that adds the module providing it), what does not go together
    and overlapping keys, the catalog to find more (search, capability, tag, author; Add), look and up to two key
    binding sets, what a VM needs at least and how it runs, its keys before it is saved – saved in My Recipes, a
    source in the data folder
  - **Key Binding Studio** (the workshop, plan B147): a key binding set of your own – a name, the set it starts from
    (or the modules' own keys), then one module at a time by the mode of its program (the way there as key caps):
    each action's keys ("h / Left") and, in a program with modes, the mode it leads to afterwards; a dot marks what
    differs from the module's own, Undo takes back what was chosen, overlaps are said at the row; the program's own
    keys are shown, not changed. Saved in My Recipes' folder (`my/<name>`); a key binding set's row in Modules opens
    it ("Edit …" for one of yours, "Copy to My Key Bindings …" for any other)
  - **Modules** (plan B142): the modules and key binding sets here, with the recipes made of them, and what the
    sources offer – the same rows as Looks (install, update, remove)
  - **Activity** (badge: dot for unread messages): what happened in the core service, newest first (the same list in
    every interface, worded by the core), "Clear History"
- The newest message also shows for 6 s at the bottom of the main window (a click opens Activity)
- Notifications (Settings → Notifications, on by default; macOS asks once): VM ready (SSH answers), image ready, build
  failed, VM ended on its own – only while omaflake is not in front
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
  "Guide" opens the Linux builder guide (the file the core names, bundled and offline; without one its page on omaflake.dev).
  "omaf command (optional)" (plan B122): whether `omaf` is on the PATH a new Terminal gets and is this omaflake's;
  "Install …" links `/usr/local/bin/omaf` to the app's bundled `omaf` through macOS' administrator dialog (the question
  names the link and its target), "Uninstall …" removes that link again; a file there that is not omaflake's is left
  alone and the hint says so. On Linux the link goes to `~/.local/bin` without root
- What the Doctor reports blocks the actions it concerns: building images of a guest kind (macOS e.g. without a
  finished base, NixOS without an answering Linux builder) and starting NixOS VMs without QEMU. An orange warning sign
  appears on the Doctor icon and in place of the blocked action; it opens the Doctor, its tooltip names the reason.
  Checks run at launch, when the Doctor opens, on "Check Again", after every action and right before a build – not
  while idle
- Instance list of both guest kinds with status dot and guest-kind icon (snowflake = NixOS, Apple logo = macOS),
  from the core's `vms.changed` events (it watches `~/.omaflake`) → also shows VMs started via the CLI; SSH port (NixOS) or NAT address (macOS, while running)
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
  content – cancel or replace (identical ones are simply there already). Without a window open, questions come as
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
- Settings: "This Mac" – this computer's own keyboard ("As the Mac says" = the system's; plan B141); Credentials (a tab
  of the workshop): rows as the other lists (in use, Copy Password with Touch ID), a form to create or edit one
  (only the name while images use it), every credential has a password (plan B138) and its own user (only the first
  one starts from this Mac's, plan B139); Locales (a tab of the workshop, plan B141): where a guest lives – one
  language and region, one time zone, one to three keyboards (Super+Space switches on NixOS, macOS shows them in the
  menu bar), drawn like the credentials, only the name changes while images use one; building an image asks its
  credential, look and locale; macOS bases are a section of the images tab (Remove), each with its credential – a base
  is created from one, an older base gets one assigned (plan B139); no password is typed anywhere after a credential's
  creation; looks are a tab of their own next to Recipes – installed from the sources, a strip of their colours, the
  default, update and remove (plan B141) – and the default credential, look and locale are set on their own pages,
  not in the settings; Profiles (a tab of the workshop, plan B142): CPU cores and RAM of a new VM, drawn like the
  credentials – the New VM page offers them (the default preset, "Custom values" to set them by hand) and warns when a
  VM gets less than its recipe needs;
  SSH keys from ~/.ssh; macOS VMs use the base's user), language (System, English, Deutsch – stored as `AppleLanguages` in the app's defaults, restart button);
  data folder (default `~/.omaflake`, move it elsewhere or back; `DataFolder.move`)

UI languages: English (development language and fallback) and German, `App/Resources/<lang>.lproj/Localizable.strings`
(keys are the English texts; the core's page texts use the same keys, docs/api.md §10.2). Error messages from the core
are texts too (`data.text`, a failed job's `errorText`, api.md §3, plan B125) and shown in the app's language;
`APIError.text`, `AppModel.describe`. The CLI stays English (docs/cli.md).

`omaf-runner` is embedded as its own helper app (`Contents/Helpers/omaf-runner.app`, shown as "omaflake VM"; inside
`Contents/MacOS` a runner with a VM window made macOS take it for omaflake.app, plan B65), signed by `bundle-app.sh` with its entitlement
`com.apple.security.virtualization`; the app finds it next to its own executable (unless `omaf config --runner` is set).

Icons: the menu bar shows the omaflake template image, the bundle carries `brand/macos/omaflake.icns`
(`CFBundleIconFile`), and VM windows started from the app get the omaflake Dock icon (QEMU patch
`qemu-cocoa-dock-icon`, plan B27). The image data comes from `OmaflakeKit/Brand/Brand.swift`; QEMU's Dock icon
is the core's (it embeds the same PNG and writes it to `<data folder>/cache/dock-icon.png` when it starts QEMU with a
window) – the app writes nothing into the data folder.

Quitting the app leaves running VMs running (QEMU in its own session, as with the CLI).
"Terminal (SSH)" controls Terminal.app via AppleScript – macOS asks for permission the first time.
