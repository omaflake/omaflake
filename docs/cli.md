---
title: CLI reference
nav_order: 3
permalink: /cli/
---

# omaf – CLI (spec M3)

Rust workspace (repo root, `Cargo.toml`): `omaflake-core` (logic, shared with every UI through `omaf serve`,
docs/api.md), `omaflake-sdk` (the client and the API's types) and the package `omaflake` (`crates/omaflake`, the
program `omaf` on clap) – all on crates.io: `cargo install omaflake`. Exit codes: 0; 1 for an error, "Error: …" on
stderr; 64 for a usage error; `doctor` and `sources check` 1 when something is missing or failed; `builder` passes on sudo's code).
Runs headless or 2D with Homebrew QEMU; with the own build (`qemu/`, `omaf config --qemu …`) also with 3D.

## Build & test

```sh
cargo test                       # unit tests (core and CLI: argument parsing, output formatting, exit codes)
cargo build -p omaflake --release # → target/release/omaf
```

In a checkout once: `omaf config --flake <checkout>/nix` (writes `~/.omaflake/config.json`; the app ships its own
copy and uses it when nothing is set). A folder without its own `.git` is used as `path:<folder>`, so Nix sees
git-ignored settings (`nix/local/`); flake URLs are used as given. Recipes come from the sources (`omaf sources`).

## Commands

**Commands that change something go through the core service** (`omaf serve --socket`, docs/api.md) – the same way
the app and `omaf ui` change things, under the service's locks, so a change made in a terminal cannot overwrite one
made in the app, and long work (builds, installs, exports, an upgrade in place) is a job the app shows too. The
service is started when it does not run (`omaf service start` does the same by hand; started that way it ends five
minutes after its last client and job) and serves `VM_HOME`'s data folder when that is set. Commands that only read
(`list`, `recipes`, `images`, `ssh`, `log`, `doctor`, …) read the data folder themselves, and so do the few that work
in this terminal by nature: `omaf macos unchain` and `omaf images unchain` (their steps are yours, in the VM's
window), `omaf builder …` and `omaf doctor --install-cli` (sudo in this terminal), `omaf credentials copy-password`,
and `omaf build` with `VM_FLAKE` set.

| Command | Effect |
|---|---|
| `omaf recipes [--all] [--all-systems] [--long] [--check]` | the recipes omaflake can build – installed ones and those in enabled local folders (via `nix eval <flake>#lib.recipeDefaults` with them as input `omaflake-catalog`) – with guest kind, architectures and current image; recipes for another architecture only with `--all-systems`; recipes of another `recipe.toml` version (`"$schema"`) are named with a hint (plan B114); `--long` with description and source; `--check` marks images their recipe has outgrown |
| `omaf recipes available [--all] [--all-systems]` | what the remote sources offer (fetches their catalogs), with "installed" / "update available"; recipes for another architecture only with `--all-systems` |
| `omaf recipes install <author>/<platform>/<name>` | downloads a recipe from the first enabled source that has it (every file checked against the catalog's SHA-256) into `<data>/recipes/`, and what it is made of that is not here – its modules, key binding sets and look – from the first source that has each in a version the recipe allows (the official catalog last, plan B142); also updates |
| `omaf recipes show <recipe> [--install]` | what a recipe is made of (plan B142): its modules (here, or missing), look, key bindings, what a VM needs, every key with its module and mode, and what is wrong (✗ blocks a build: a missing part; ⚠ warnings: a need no module provides, incompatible modules, keys that overlap or break the key rule; unfree software). `--install` installs what is missing first |
| `omaf recipes versions [<recipe>]` | each recipe line (source and author/kind/name) with its versions, newest first – their omaflake IDs (as short as they can be), when they came, and the images built from them (spec §11.11) |
| `omaf recipes remove <recipe> [--all]` | removes a version of a downloaded, imported or kept recipe (by name, or its omaflake ID when there are several), or with `--all` every version of its line; versions images are built from and a local folder's own recipe stay, as does one another recipe requires |
| `omaf sources` | the recipe sources in their order (none set: omaflake/catalog); earlier ones win when two have the same recipe |
| `omaf sources add <repo\|url\|folder> [--private] [--branch b] [--first]` | adds a GitHub repository (`https://github.com/owner/repo`, `owner/repo`, `…/tree/<branch>`), the HTTPS URL of a `catalog.json`, or a folder – checked before it is kept; `--private` reads a fine-grained token (Contents: Read-only) from stdin, checks it against the repository and stores it in the Keychain (Linux: the keyring through the Secret Service; without one, unencrypted in `$XDG_CONFIG_HOME/omaflake/tokens.json`, mode 0600 – it says so) |
| `omaf sources remove\|enable\|disable <n\|id>` / `omaf sources check` | remove (its token leaves the Keychain), switch on or off, check that every source can be read |
| `omaf build <recipe>` | NixOS: `nix build <flake>#images.<system>."<recipe ID>"` (this computer's system; a recipe that does not list it is refused), the qcow2 copied into the package `~/.omaflake/images/<author>.<kind>.<name>@<words>.omaflake-image/disk.qcow2`, `current/<line>` points to it. macOS: image pipeline (below). A recipe is named by `<name>`, `<kind>/<name>`, `<author>/<kind>/<name>` – which means the **newest version of its line**: a local folder's recipe as it is now, not a copy kept from an earlier build –, or by its omaflake ID `recipe:<author>/<kind>/<name>:<words>` or `<name>@<words>` for an older version (spec §11.11). Only when several lines have that name (other sources, authors or kinds) does `omaf` ask (in a terminal) or list them |
| `omaf build <recipe> [--credential c] [--look l] [--locale l]` | the image's credential (a name or ID; default: the default credential – without any credential nothing is built, plan B139) and look (an installed look: `<author>/<name>`, its name or title; default: the default look, plan B141) and locale (a name or ID; default: the default locale – without any, this computer's values, plan B141). A macOS recipe takes its base's credential (plan B137, B139) |
| `omaf credentials` / `omaf credentials add <name> [--user u] [--full-name n] [--email e] [--password]` / `edit <name> [--rename n] [--user u] [--full-name n] [--email e] [--password]` / `rm <name>` / `default <name>` / `copy-password <name>` | credentials: who you are in a guest (plan B137, B138, B139); `*` marks the default – there is always one while there are credentials. Every credential has a user (the first one defaults to this computer's, later ones need `--user`) and a password: `add` asks it (twice; `--password` reads it from stdin), `edit --password` changes it; it is kept in the Keychain (Linux: the keyring, else `tokens.json`) – omaflake creates macOS bases and builds macOS images with it, nothing else asks. While images use a credential, `edit` only takes `--rename`/`--password`, while a macOS base uses it only `--rename`; `rm` is refused while it is used. `copy-password` asks Touch ID or the login password, copies it and clears the clipboard after 90 s |
| `omaf looks` / `available` / `install <author>/<name>\|recommended` / `rm <look>` / `default <look>` | looks (plan B141): how the guests look – installed from your sources' catalogs (or used from a local folder source), `*` marks the default (there is always one while one is here). `available` looks into the catalogs; `install recommended` installs omaflake's own and Catppuccin; `rm` is refused while images are built with the look |
| `omaf modules` / `available` / `install <author>/<platform>/<name>\|<author>/<name>` / `rm …` | modules and key binding sets (plan B142): what recipes are made of – installed from your sources' catalogs (or used from a local folder source); `install` takes a module (`omaflake/nixos/zathura`) or key bindings (`omaflake/default`). Installing a recipe brings what it needs; `rm` leaves recipes made of it unbuildable until it is back |
| `omaf profiles` / `add <name> [--cpus n] [--memory gb]` / `edit <name> [--rename n] [--cpus n] [--memory gb]` / `rm <name>` / `default <name>` | profiles (plan B142): CPU cores and RAM a new VM gets; `*` marks the default (the first one becomes it); kept in config.json, never in a catalog |
| `omaf locales` / `omaf locales add <name> [--locale l] [--time-zone z] [--keyboards k1,k2,k3]` / `edit <name> [--rename n] [--locale l] [--time-zone z] [--keyboards …]` / `rm <name>` / `default <name>` | locales (plan B141): where a guest lives – one language and region, one time zone, one to three keyboards (the first the default; Super+Space switches on NixOS, macOS shows them in the menu bar). `add` presets this computer's values; `*` marks the default (the first one becomes it). While images are built with a locale, `edit` only takes `--rename` and `rm` is refused. Kept in config.json, never in a catalog |
| `omaf build <mac-recipe> [--credential c] [--base <dir> \| --unchained] [--keep-failed] [--window]` | system closure built on the Mac → APFS clone of the credential's newest `~/.omaflake/bases/mac-base-*` (plan B166; none: create it with `omaf macos install --credential c`) → headless start (`--window`: the build VM in a window, to see where it waits) → Nix → `nix copy` → activate → shut down → `images/<recipe>-<rev>-<hash>/` (read-only), `current/<recipe>`. Logs in with the base's credential and its stored password (sudo) – nothing is asked; a base without one: `omaf macos assign` (plan B139); log in `~/.omaflake/logs/build-<image>.log`; a failed build goes to the trash `--unchained` (plan B184) builds from the credential's newest unchained base (`omaf macos unchain`) instead: the image and its VMs run with System Integrity Protection off, and it is an image of its own (other words in its name, `unchained` in image.json) – never the recipe's current image and not deleted with its older ones: `omaf new <name> --image <image>` makes a VM from it |
| `omaf macos install [--credential c] [--ipsw <file>] [--replace <base>] [--window] [--no-first-boot]` | `omaf-runner install` (latest supported IPSW from Apple unless `--ipsw`) → `bases/mac-fresh-<build>` (read-only, never started) → APFS clone `mac-base-<build>` → first boot (headless; `--window` to watch) with provisioning (user, auto-login, SSH) → your `~/.ssh/*.pub` into the guest → shutdown → read-only. The guest user, its full name and password come from the credential (default: the default credential) – nothing is asked; the base records it in `omaflake-base.json` (plan B139; since B166 with whether it knows omaflake's own SSH key). One base per macOS build and credential: the first of a build is `mac-base-<build>`, one for a further credential `mac-base-<build>-<credential>`; a credential's existing base is reused. `--replace <base>`: the new base is made first, the old one deleted once it is ready (refused while an image build in the service uses it); of the same build and credential, the new one takes the old name. No manual steps; existing folders are reused (BM29) |
| `omaf macos bases` | the macOS bases, newest first: build, version, credential, user; notes for the newest of a credential, unfinished ones, ones without a credential or without omaflake's own SSH key (plan B166) |
| `omaf images unchain <image> [--no-tools]` | the unchained copy of a macOS image (plan B184): a second image, the same as this one, with System Integrity Protection off and omaflake Tools with omaflake's camera extension – nothing is built, what the image holds stays (its keyboard too, for the password the approval asks). The same two steps in the VM's window as for a base (`csrutil disable` in macOS Recovery, the extension's approval), here for each image. The copy has `unchained` in its image.json and is never its recipe's current image: `omaf new <name> --image <copy>`. Runs here, never in the service |
| `omaf macos unchain <base> [--no-tools]` | the unchained copy of a ready base (plan B184): `bases/mac-unchained-<…>`, a clone with System Integrity Protection off and omaflake Tools with omaflake's camera extension (`--no-tools`: without). It goes through macOS Recovery by itself – the VM's window hidden, its screen read (text recognition on this Mac), clicks and keys sent: Options → Utilities → Terminal → `csrutil disable` with its questions → `halt`; Recovery's language set to English with a US keyboard before (`prev-lang:kbd`, put back after). When it finds no way on, it shows the window and prints the steps that are yours. The camera extension is approved by you in System Settings, in the window then shown. Prints its steps as `[n/6] …`. The base it is made from stays; an unfinished one continues. Before the approval the base gets this computer's keyboard (`hostKeyboard`; a base has the US one, and the approval asks for the password). Runs here, never in the service. Never the default: `omaf build <recipe> --unchained` builds an image from it |
| `omaf macos remove <base>` | deletes a base for real – images and VMs made from it stay (they are copies); by the service (`macos.removeBase`), refused while an image build uses it |
| `omaf macos assign <base> <credential>` | a base from before bases kept their credential: records the credential its user was created with (its user must be the base's) – macOS builds then log in with its password (plan B139) |
| `omaf images` | built images and which instances use them |
| `omaf networks` (`net`) / `create <name>` / `rm <name>` / `show <name>` / `join <network> <vm>` / `leave <network> <vm>` | networks between VMs (plan B170, spec §11.12): each member gets a second network card on a switch omaflake runs (`10.231.<n>.0/24`, a fixed address per VM) and reaches the network's other VMs – NixOS and macOS alike; its own NAT (SSH, the internet) stays. Membership counts from the VM's next start; at most 4 networks per VM. `show`: members, addresses, MACs and – while the switch runs – each card's traffic and the bytes between VMs. `rm` takes the network away from its members at their next start. The switch (`omaf network-switch`, hidden) starts before a member VM and ends a minute after the last |
| `omaf volumes` (`vol`) / `create <volume> [--path <folder>]` / `rm <volume>` / `attach <volume> <vm> [--as <name>]` / `detach <volume> <vm>` / `rename <volume> <vm> [--as <name>]` | volumes (plan B187, spec §11.13): folders of this computer that VMs share – a VM has none, one or several, several VMs the same one. Each VM sees a volume at `/Volumes/<name>`, by default the folder's name (`--as` gives another for that VM). Changes count from the VM's next start; `rm` never deletes the folder. `omaf new … --volume <volume>[=<name>]` (repeatable) gives a new VM volumes, making one that does not exist yet |
| `omaf files` / `add <module> [<path>]` | files modules need to be built, which nobody may pass on (plan B198: Xcode) – which are in the Nix Store, found on this computer (added at the next build) or missing, with where to download them. `add` adds one: a download (`.xip`, `.zip`), a folder or an app, unpacked and checked against the module's hash; without a path it takes the one found where the module looks. `<module>` is `omaflake/xcode`, `omaflake.macos.xcode` or `macos/xcode` |
| `omaf set <name> [--audio\|--no-audio] [--microphone\|--no-microphone] [--cpus N] [--memory GB] [--disk GB] [--displays N] [--forwards "5432=db:5432, …"]` | sound and microphone of an instance, from its next start; `--forwards` (plan B180) ports of other VMs in a shared network on its localhost (`""` for none); `--disk` (plan B164) a larger disk, only larger, while stopped – the guest takes the space at its next start (macOS: images built with 0.8.0 or later); `--cpus`/`--memory` (plan B164) only while it is stopped, 1 to this computer's cores, 2 GB to its memory less 2 GB (`vms.set`); `--displays` (plan B171) NixOS VMs only, 1 to 4, while stopped – each display in a window of its own |
| `omaf new <name> -r <recipe>\|--image <image> [--ephemeral] [--share\|--no-share] [--share-dir <folder>] [--volume <volume>[=<name>]]… [--no-audio] [--microphone] [--user <u>] [--profile p] [--cpus/--memory/--disk/--network] [--join <network>]…` | create instance (NixOS: overlay + efivars; macOS: APFS clone) + `instance.json`; CPU cores and RAM from the profile named, else the default profile (plan B142) – `--cpus` and `--memory` win, less than the recipe needs is said; `--share` gives it a volume of its own, `share/<name>` (as the recipe's `share` does; `--no-share`: none), `--share-dir` that volume in an existing folder of your choice (its permissions stay as they are); `--volume` (repeatable) attaches a volume, making it when there is none of that name, at `/Volumes/<name>` in the VM (instead of a volume of its own); a recipe name takes the newest version of its line that has an image; `--image` takes that image (a name or omaflake ID from `omaf images`) – also one imported from another computer, whose recipe is not here: then the VM gets standard values (4 cores, 8 GB, 64 GB, shared network), as `-r` with its recipe's name does when no recipe here has that name; `--join` (once per network) makes it a member of networks between VMs (`--network` is its own NAT's mode); `--displays N` (NixOS, 1 to 4) gives it more monitors, each in a window of its own (plan B171) |
| `omaf start <name> [--window [--gl on\|es] [--no-gl] \| --headless] [--wait] [--recovery]` | start QEMU in its own session (survives CLI/terminal); `--window` with 3D if the binary supports it. Without `--window`/`--headless` the recipe's `display` decides (none, window, gpu). On a Linux host with KVM, or TCG without a usable `/dev/kvm` – then a line says so (emulated, many times slower; SSH may take minutes); `--gl es` is the Mac's ANGLE path, GTK windows always use `gl=on` (docs/linux.md) |
| `omaf stop <name> [--timeout 60]` | ACPI powerdown → QMP `quit` → SIGKILL |
| `omaf reset <name> [-y]` | deletes overlay, efivars, known_hosts; fresh overlay (macOS: fresh clone); asks first in a terminal (`-y`: does not) |
| `omaf upgrade <name> [--keep]` | switch to the current image of the recipe (the old state is deleted); `--keep` (plan B178): the running VM gets the new image's system in place and keeps its disk and files (NixOS VMs with an image from 0.8.0 on) |
| `omaf rm <name> [-y]` | deletes the instance (the folders of its volumes stay); asks first in a terminal (`-y`: does not) |
| `omaf images [list]` / `omaf images rm <image>` | images with their omaflake ID, recipe, revision, time, system, look and the instances using them / delete an image (by name, omaflake ID or the start of its words) – refused while instances use it; `current/` moves to the next newest image of the recipe's line |
| `omaf list [--long]` | instances with status, guest kind, SSH port, disk usage; VMs that run from elsewhere with their place; `--long` with each VM's omaflake ID (`vm:<name>:<words>`) and its recipe's (spec §11.11) |
| `omaf export vm <name> <folder> [--with-image]` | a stopped VM → `<folder>/<name>.omaflake-vm` (a snapshot); NixOS with its image, macOS with it only when ephemeral or with `--with-image` (spec §11.10) |
| `omaf export image <image> <folder>` / `omaf export recipe <recipe> <folder>` | an image → `<name>.omaflake-image`; a recipe and those it requires → `<author>.<kind>.<name>.omaflake-recipe` |
| `omaf import <path> [--name n] [--replace]` | always a copy into the data folder (spec §11.10), by extension, or a folder of recipes; the source is never changed. A VM comes back as the same VM; its name taken: an error, `--name` imports it as a second VM with its own identity (SSH port, macOS machine identity). The same image or changed recipes of the same ID: an error, `--replace` replaces them; identical recipes are skipped |
| `omaf external [list]` / `omaf external add <package>` / `omaf external remove <name>` | VMs that run from where their package is (an external disk, a NAS, a folder; nothing is copied, a NixOS VM must carry its image) / stop listing one (also while its place is gone; the package stays) |
| `omaf ssh <name> [-- command]` | `ssh <user>@127.0.0.1 -p <port>` (key from ssh-agent) |
| `omaf wait <name>` | wait until sshd sends its banner (without logging in) |
| `omaf agent <name> [--wait 60] [--json]` | what the VM's omaflake agent reports: version, OS, load, memory, disk, addresses, users (NixOS guests; spec §11.9) |
| `omaf diagnose <name>` | the running VM's diagnosis (plan B162): agent, SSH, clipboard, volumes, network, sound, microphone, camera, MIDI, disk – one line per check (`✓` pass, `✗` fail, `?` only you can tell, `–` skipped), then the log's path (`<data>/logs/diagnostics-<vm>-<time>.log`); exit 1 when a check failed. Runs in this process, as `omaf call diagnostics.run` |
| `omaf log <name> [-n 40] [-f] [--qemu]` | serial console or QEMU log |
| `omaf doctor` | checks what this computer brings (spec §11.2) – on a Linux host: Linux on x86_64/aarch64, KVM (`/dev/kvm` usable – else limited: TCG, with the fix), Nix, flakes, QEMU with version, 3D and UEFI firmware, image flake, SSH key, git, the `omaf` command, then recipes evaluate and space in the Nix store (no nix-darwin, Linux builder or macOS items; docs/linux.md). On a Mac: macOS/Apple silicon, Nix, flakes, nix-darwin, Linux builder, QEMU (3D or not), image flake, SSH key, git, the `omaf` command on your PATH (`cli`, optional: ok when it is this omaflake's – the same file or `--version` –, on Linux also one from the Nix store; limited when another one comes first or omaflake's link is not on the PATH; the line under it is the command that installs it); then the preflight for NixOS (builder answers, recipes evaluate, space in the Nix store) and macOS (IPSW, base, space); prints hints and the official install command; exit code 1 if something required is missing |
| `omaf doctor --install-cli` / `--uninstall-cli` | the `omaf` command (plan B122): a symlink to this omaf – inside omaflake.app the bundled `Contents/MacOS/omaf` – as `/usr/local/bin/omaf` on macOS (runs sudo; `mkdir -p /usr/local/bin` first) or `~/.local/bin/omaf` on Linux (no root; if that folder is not on your PATH, `omaf doctor` says how to add it). Install replaces a link or an older omaf there and refuses any other file; uninstall removes only omaflake's link. The app's Doctor does the same through the administrator dialog |
| `omaf builder setup` | omaflake's own Linux builder (since 0.7.4, docs/linux-builder.md): runs as you (a LaunchAgent, `<data>/builder/`), no nix-darwin, no sudo, in two stages (default builder from the cache, then omaflake's builder host) |
| `omaf builder apply [--force]` / `omaf builder clean-up` | build omaflake's Linux builder again with its settings, or `nix-store --gc` in the builder with free space before/after; log `<data>/logs/linux-builder.log`. With nix-darwin's builder instead: `apply` activates the image flake's `#linux-builder` host as root like `darwin-rebuild switch` (sudo; refused without `--force` when this Mac's nix-darwin system is not omaflake's) |
| `omaf builder check` / `omaf builder repair [--force]` | does the Linux builder accept this Mac's builds (a tiny test build, plan B175); repair gives a builder that refuses them a new disk – omaflake's as you, nix-darwin's via `sudo` – and checks again (`--force`: also when the check passes) |
| `omaf serve` | the core API (JSON-RPC 2.0, one JSON object per line) on stdin/stdout – a private server for a UI or a test (docs/api.md) |
| `omaf serve --socket[=<path>] [--idle-exit <secs>]` | the **core service** (plan B120): the same API on a Unix socket (default `<data folder>/run/omaf.sock`, a short path under `$TMPDIR`/`$XDG_RUNTIME_DIR` when that is too long – api.md §1), shared by the app, the other interfaces and the CLI: one build queue, one job list, events for every client. One per data folder (a second one exits at once, status 0); `--idle-exit` ends it when no client has been connected and no job has run for that long |
| `omaf service install` / `uninstall` | the service as a user agent: macOS launchd `~/Library/LaunchAgents/app.omaflake.serve.plist` (at login, again after a crash; it runs this `omaf`), Linux systemd `~/.config/systemd/user/omaflake.service`; refused with `VM_HOME` set / stopped and removed |
| `omaf service start [--json]` | start the service unless it runs: the agent if installed, else `omaf serve --socket --idle-exit 300` on its own; restarts an older service once idle (api.md §1) – also one of another omaflake core of the same version (an app replaced in place, plan B165: `initialize`'s `core`), waiting up to 60 s for its jobs; `omaf ui`, `omaf status`, `omaf call --socket` and `omaf build` do the same; prints its socket and version |
| `omaf service stop` / `omaf service status [--json]` | end the service once its jobs are done (`service.restart`) / agent installed?, socket, running, version, jobs waiting or running |
| `omaf call [--socket[=<path>]] <method> [<params as JSON>]` | one API request; prints the result as JSON (exit 1 and the error's message on error) – for scripts and tests. Without `--socket` in this process without a session; with it through the service (started on demand). A method that starts a job waits for it and prints the ended job (Ctrl-C cancels it through the service) |
| `omaf ui [--nerd-font] [--mouse] [--in-process]` | the **terminal interface** (plan B121, macOS and Linux): the app's pages – VMs, images (with running builds), recipes, activity; build, Doctor, macOS base, New VM, settings – drawn from the core's page models (api.md §10) through the core service (started on demand; with `--in-process`, or when the service cannot be reached, a server in this process whose jobs end with the TUI). Keys below. Colours from the brand's design tokens: truecolor with `COLORTERM=truecolor`/`24bit`, else the nearest of 256, none with `NO_COLOR`; symbols as plain characters, Nerd Font glyphs with `--nerd-font` or `OMAF_NERD_FONT=1`. English or German – `OMAFLAKE_LANGUAGE` (`en`, `de`), else the locale (`LC_ALL`, `LC_MESSAGES`, `LANG`), with the macOS app's translations (crate `omaflake-i18n`) |
| `omaf status [--waybar] [--follow]` | running VMs and builds in one line (`2 VMs running (a, b) · building dev 62 %`); `--waybar`: JSON for a waybar `custom` module – `text` (running VMs, the build's percent), `tooltip`, `class` and `alt` (`idle`, `running`, `building`; plus `failed`), `percentage` (the running build); `--follow` stays connected to the service and prints a line whenever it changes (snippet below) |
| `omaf config [--flake p] [--qemu p] [--runner p] [--nix p] [--host-keyboard k] [--home <dir>\|default]` | show/set settings (`~/.omaflake/config.json`); `--host-keyboard` is this computer's own keyboard (empty value = as the system says; preset for new locales; plan B141); the guest's language, time zone and keyboards are a locale's (`omaf locales`); the default credential, look and locale are set with `omaf credentials default`, `omaf looks default` and `omaf locales default` (plan B141); `--home` moves the data folder (VMs stopped, same volume; overlays name their image relatively, paths in instance.json and settings are rewritten); `--keep-recipe-versions on\|off` keeps older versions of a recipe line (default off: the newest and those images use); `--volumes-in-finder on|off`, `--volumes-folder <folder>|default`, `--volumes-name <name>|default` (macOS, plan B200): the folder with every volume in the Finder's sidebar – where it is kept and its name there |

Guest user for SSH: `--user` on `omaf new`, otherwise `user = "…"` from the recipe's `recipe.toml`, otherwise the Mac user name.

Environment variables: `VM_HOME` (instead of `~/.omaflake`), `VM_FLAKE`, `VM_QEMU`, `VM_RUNNER` (signed `omaf-runner`;
otherwise `omaf config --runner`, otherwise next to `omaf`, otherwise PATH). QEMU without `VM_QEMU`/`omaf config --qemu`: `qemu-system-aarch64` on a Mac, `qemu-system-<host arch>` on Linux, from PATH. The core service (`omaf serve`, started by the app, `omaf ui` or a terminal) has the environment it was started with,
not this terminal's: with `VM_FLAKE` set, `omaf build` builds in the terminal instead of handing the build to the service; `VM_QEMU`, `VM_RUNNER` and `OMAFLAKE_ACCEL` reach a VM start only when the service was started with them (`omaf service stop`, then the command again). Linux:
`OMAFLAKE_ACCEL=tcg` starts VMs with TCG even where KVM works (to compare, or when KVM misbehaves).
For window starts `omaf` passes `OMAFLAKE_DOCK_ICON` to QEMU (omaflake Dock icon, needs QEMU patchset p4 from `qemu/build.sh`),
for 3D windows also `OMAFLAKE_LOADING_IMAGE` (the loading image until the guest draws, p6).

### `omaf ui` – keys

| Key | |
|---|---|
| `Tab` / `Shift-Tab` | next / previous tab (VMs, Images, Recipes, Activity) |
| `j`/`k`, `↑`/`↓`, `g`/`G`, `PgUp`/`PgDn` | move |
| `Enter` | the row's default action (drawn bold in the footer: open a terminal, start, build, New VM …); on a form field: edit it |
| `1`–`9` | the selected row's actions, as numbered in the footer and the opened details |
| `a` | every action of the row, with why one is greyed out |
| `Space`, `→`/`←` | open / close a row's details; on a field: switch it, the next / previous value |
| `/` | the list's filter (`←`/`→` change a group, `Enter` apply, `r` reset) |
| lowercase letters | the page's tools, as shown in its title line (`i` Import, `r` Refresh, `c` Clear History …) |
| `D`, `S`, `H` | Doctor, Settings, the data folder (home's actions) |
| `Esc` | back (closes a question or a page) |
| `?` | help: these keys and the selected row's actions |
| `q`, `Ctrl-C` | quit (`q` asks when VMs run – they keep running) |

Questions and asks are the core's (api.md §10.3): a password is never shown (asked twice where the model says so),
folders, files and packages are typed as a path with `Tab` completion and checked (`packages.inspect`) before the step
goes on. "Open Terminal" leaves the TUI for the SSH session and comes back after it; "Show in Finder" and "Open"
use `open`/`xdg-open` where there is a desktop (not over SSH) and always print the path in the status line. New
activity (a VM ready, a build done or failed) shows in the status line and on the Activity tab.

### waybar

```jsonc
// ~/.config/waybar/config.jsonc
"custom/omaflake": {
  "exec": "omaf status --waybar --follow",
  "return-type": "json",
  "format": "{icon} {text}",
  "format-icons": {"idle": "❄", "running": "❄", "building": "⚒"},
  "on-click": "foot -e omaf ui"
}
```

`--follow` keeps one connection to the service (no `interval` needed) and prints a line only when it changed; style it
with `#custom-omaflake.running`, `.building`, `.failed`.

## Behavior (decisions)

- **Language:** the CLI speaks English – its output, its help and the core's error messages (a command line is read,
  searched and pasted into reports; one language keeps them comparable). The interfaces – the macOS app, the Linux
  app and `omaf ui` – show the same texts and errors in English or German (docs/api.md §3, §10.2; plan B125).
- **Ephemeral** = QEMU `-snapshot`: writes are discarded on power-off, nothing needs to be deleted.
  macOS guests get a fresh APFS clone at every start instead; the previous run's clone is deleted (plan BM27).
- **macOS guests** (`kind = macos`, M10): the same commands; `omaf start` launches `omaf-runner` detached
  (`run/<name>/runner.{sock,pid,log}`), `omaf ssh` connects to the NAT address on port 22 (shown by `omaf list`
  while running), `omaf stop` shuts down via SSH (no confirmation dialog, plan BM17). At most two macOS VMs run at
  the same time – `omaf start` refuses a third. `omaf start <name> --recovery` starts into macOS Recovery, always in
  a window (not for VMs that start from their image every time: what Recovery changes would be gone). Network
  `isolated` is not available for macOS guests. Once SSH answers (`omaf wait`, `start --wait`), the guest is named after the instance (HostName, LocalHostName,
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
  permission; `OMAFLAKE_CAMERA_TEST_FPS` sets its rate).
- **Camera (macOS, plan B169):** the same switch; images built with 0.7.5 or later. Until omaflake's own camera
  extension (M12) apps in the VM see the camera as "OBS Virtual Camera": install OBS in the VM and start it once –
  the agent then writes its scene collection "omaflake Camera" and makes it the current one; approve OBS's camera
  extension (System Settings → General → Login Items & Extensions → Camera Extensions) and click "Start Virtual
  Camera". The Mac's camera runs while OBS reads it. `omaf diagnose <vm>` says which step is missing.
- **MIDI (plan B161, every guest kind):** the host's MIDI endpoints as virtual MIDI endpoints of the same name in the
  guest, both ways, off by default (`--midi`, `omaf set`, recipe.toml `midi`). Nothing is captured – the host's apps
  keep the devices – and devices plugged in or out while the VM runs follow. The data are Universal MIDI Packets in the
  protocol of each device over the omaflake agent (spec §11.9), stamped with the time they arrived; the receiver
  delivers them 2 ms after the fastest transport seen in the last 30 s, so notes keep their spacing. Host sides: macOS
  guests `omaf-runner run --midi`; NixOS guests on a Mac `omaf-runner bridge --midi --vm-name <name>` (CoreMIDI, read
  on a port of each device's protocol, nothing converted); NixOS guests on a Linux host `omaf bridge --midi --vm-name
  <name>` – every port of the ALSA sequencer that can be read or written (except omaflake's own clients and the
  kernel's System client), named "client: port"; MIDI 1.0 events become UMP and back, UMP clients (Linux 6.5+,
  alsa-lib 1.2.10+) pass raw (OPEN: checked on a Linux desktop with MIDI hardware). Per VM one more pair on the host,
  "omaflake <name>" (on Linux a sequencer client with the ports "omaflake <name> in" and "… out"), appears in the guest
  as "omaflake Mac" or "omaflake host". In macOS guests omaflake starts the agent over SSH once the guest answers (like
  the clipboard helper; `omaflake-agent-start`, log `~/Library/Logs/omaflake/agent.log` in the guest, the four starts
  before it as `agent.1.log` … `agent.4.log`). `omaf-runner ctl <run/runner.sock> midi` lists what a macOS VM gets; agent.json keeps how many endpoints the guest made and the round
  trip host → agent → host of a ping once a second (`midi.roundTripMs`).
- **Diagnosis (plan B162, every guest kind):** `omaf diagnose <name>` (and the app's "Diagnose …") checks a running
  VM's features; the guest's half through the agent, relayed by whatever holds the agent link – omaf-runner's control
  socket for macOS guests, the bridge's `<run>/bridge.sock` for NixOS guests (`omaf-runner bridge` / `omaf bridge
  --control <socket>`: one JSON object per line each way, `{"command": "status" | "diagnose" | "midi", "check"?,
  "args"?}`). macOS guests get the agent over SSH always now (not only with MIDI), and that start ends with a check
  that the helpers run: a start that did not take is tried again at the next look and logged once per VM run, with
  ssh's stderr, in the instance's runner.log and the activity list.
- **Builds and the service (plan B120):** `omaf build` runs as a job of the core service (started when it does not
  run) – the app and other clients see it, it waits in the same queue as theirs, and an already running build of the
  recipe is followed instead of started twice; the command prints its log (routine lines left out; `--verbose` names
  the log file that has them) and Ctrl-C cancels the job. Only with `VM_FLAKE` set it builds in this process. The
  other long commands are the service's jobs too – `macos install`, `import`, `export`, `config --home`, `upgrade
  --keep`, `recipes install`, `looks install`, `modules install`, `diagnose` –, their log printed as it comes and
  Ctrl-C cancelling them where the job can be cancelled; `builder …` and the unchained copies stay in this terminal
  (sudo, and steps that are yours).
- **Cancelling a build:** Ctrl-C during `omaf build` stops the running step (`nix build`, `nix copy`, SSH), forces
  a macOS build VM off and deletes the half-built folder. One build per image at a time (lock
  `images/.build-<image>.lock`, shared by app and CLI); a second one is refused.
- **Deleting:** `rm`, `reset`, `upgrade` and `images rm` delete for real; the folders of volumes are never deleted – only the empty default folder of a VM's own volume goes with the VM (plan B85, spec §11.13). (Moving to
  `_to_delete/` is a rule for developers' own actions, not for the product – CLAUDE.md.)
- **Resolution:** the window starts at 75 % of the main screen (16:9, `xres/yres` in pixels); dragging or full screen changes the guest resolution, Hyprland scales Retina automatically.
- **Logs** are appended (`append=on`), never overwritten.
- **Lock** per instance (`.lock`, `flock`) against concurrent actions from CLI and app.
- **Volumes (NixOS guests):** one virtio-9p device per volume, mounted at `/Volumes/<name>`; with the own QEMU
  `security_model=none,guest_owner_uid=1000` – writable from both sides. With Homebrew QEMU `mapped-xattr` and the
  folder 0777; files created on the Mac are read-only in the guest (plan B18).

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
| `omaf-runner run <dir> [--window] [--socket s] [--pid-file p] [--volume <name>=<folder>]… [--no-network] [--audio [--microphone]] [--clipboard] [--agent-state <file> [--midi]] [--lan-mac/--lan-socket/--lan-switch …]… [--forward <port>=<address>:<port>]… [--recovery]` | run the VM until it is off; closing the window = clean shutdown. `--volume`: a virtio-fs device per volume, mounted by the guest at `/Volumes/<name>` (plan B187, B189); `omaf-runner run --help` lists every option |
| `omaf-runner run … --user <user> --full-name "…" --password-stdin [--auto-login] [--ssh]` | first boot after installation only: account without Setup Assistant (guest and host macOS 27); used by `omaf macos install` |
| `omaf-runner ctl <socket> status\|stop\|kill\|show` | control a running runner (line JSON over Unix socket) |
| `omaf-runner ip <dir>` | NAT IP from `/var/db/dhcpd_leases` via the MAC in `config.json` |

VM folder: `config.json` (CPU, RAM, disk, display, MAC, build), `hardware-model`, `machine-id`, `aux.img`,
`disk.img` (RAW, sparse – Virtualization.framework supports RAW only). tart folders (`asif`) are not compatible.
