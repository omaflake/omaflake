---
title: Core API
nav_order: 5
permalink: /api/
---

# omaflake core API (v1)

omaflake's logic lives in one **Rust core** (`crates/omaflake-core`). Every user interface talks to it through the
interface described here: the macOS app (SwiftUI; menu bar, window or both – plan B119), the Linux app (native Rust
with iced, plan B119, issue #4, [docs/linux.md](linux.md)), a terminal interface (`omaf ui`, macOS and Linux, plan B121), scripts and tests. The CLI `omaf` is built on the same core. No UI holds logic of its own beyond presentation: what a VM, an
image or a recipe is, what may be done with it and what happens then is decided in the core.

```
SwiftUI app (macOS) ─┐                                   ┌─ omaf-runner (Swift, Virtualization.framework, macOS guests)
Linux UI, TUI     ───┼─ JSON-RPC 2.0 ─ omaf serve --socket ─ omaflake-core (Rust) ─┼─ qemu-system-*, qemu-img, nix, ssh
CLI (omaf build …) ──┤  (Unix socket, NDJSON;            └─ ~/.omaflake (data folder, spec §5)
scripts, tests    ───┘   one service per user)
CLI omaf (other commands) ───────────────────────── omaflake-core (linked)
```

The core reads every state from the data folder (spec §5): VMs started by the CLI show up in a UI and the other way
round, and a UI that quits leaves running VMs alone.

## 1. Transport

The server runs in one of two ways, with the same messages:

- **The core service** (plan B120): **`omaf serve --socket`**, one per user and data folder, on a **Unix socket**.
  Every interface – the macOS app, the Linux app, the TUI, `omaf build`, `omaf call --socket` – connects to it, so
  they share **one server**: one build queue, one job list, the events for everyone. Each connection is a session of
  its own (`initialize`, `events.subscribe`, its language); jobs, the data-folder watcher and the caches are shared,
  and an event goes to every connection subscribed to its topic (a list with translated texts – `recipes.changed` – in
  each connection's language).
- **A private server**: a UI starts **`omaf serve`** as a child process and talks to it over **stdin/stdout** – for
  tests, and as the fallback of a UI that cannot reach the service.

Lines: one JSON object per line (UTF-8, `\n`-terminated, no newline inside an object – NDJSON). The server's
stderr carries its own diagnostics only.

**The socket.** `<data folder>/run/omaf.sock`; when that path is longer than a Unix socket takes (103 bytes on
macOS, 107 on Linux), `$TMPDIR/omaflake-<hash>.sock` (macOS) or `$XDG_RUNTIME_DIR/omaflake-<hash>.sock` (Linux), and
when that is too long or unset `/tmp/omaflake-<uid>-<hash>.sock` – `<hash>`: the first 16 hex digits of the SHA-256
of the data folder's path, so two data folders (`VM_HOME`) never share a service. `omaf serve --socket=<path>` takes
another. The socket is mode 0600, and a connection of another user is closed at once (peer credentials). A lock file
next to it (`<socket>.lock`, flock) refuses a second service; a socket file without a service behind it is replaced.

**Starting it.** `omaf service install` makes it a user agent (macOS: launchd `~/Library/LaunchAgents/
app.omaflake.serve.plist`, started at login and again after a crash, diagnostics in `~/Library/Logs/omaflake/
service.log`; Linux: systemd `~/.config/systemd/user/omaflake.service`, `Restart=on-failure`). A client that finds no
service starts it on demand: `omaf service start` – the installed agent (`launchctl kickstart`, `systemctl --user
start`) unless `VM_HOME` is set (the agent serves the standard data folder), else `omaf serve --socket --idle-exit
300` in a session of its own (setsid, diagnostics in `<data folder>/logs/service.log`) – and waits up to 10 s for
the socket to answer. The macOS app runs the bundled `omaf service start --json` (it prints `{socket, version,
started}`) and connects; when that fails, it starts a private `omaf serve` instead – at every (re)connect, in that
order.

**Its end.** The service ends: after `service.restart` once no job waits or runs; after the data folder moved
(its socket moved with it; once the move job ended); with `--idle-exit <secs>` when no client has been connected and
no job has run for that long. Connected clients see their connection close and connect again (starting the service
anew). A client's `shutdown` ends only its own session.

**Updating.** `initialize` answers with the server's `omafVersion`. A client whose omaf is **newer** than the
service's asks it to restart: `service.restart`, which ends the service once no job waits or runs; the next start
(the agent, or the client) runs the new omaf. `omaf service start` does this once per call, and only when the restart
brings the newer omaf (no agent installed, or the agent runs this omaf – after an update in place of the app; an agent
of another omaf is left alone: run `omaf service install` again). A client older than the service uses it as it is
(API versions are additive, §2).

The rest holds for both:
- Messages follow **JSON-RPC 2.0**: requests (`id`, `method`, `params`), responses (`result` or `error`) and
  notifications (no `id`) from the server – the events (§6). The client sends no notifications.
- Requests run **concurrently**: the server answers each one when it is done, so responses may arrive in another
  order than the requests (match them by `id`). A slow request (stopping a VM) never blocks a fast one (listing).
- A private server ends when stdin closes or after `shutdown`. Long work it still runs (jobs, §5) is cancelled then;
  VMs keep running (they live in their own session). The service's jobs outlive the client that started them.
- `omaf call <method> [<params as JSON>]` runs a single request without a session and prints the result – for
  scripts and tests. It initializes itself and sends no events; a method that starts a job waits for it and prints
  the ended job (its `state` tells whether it finished). `omaf call --socket[=<path>] …` sends it to the service
  instead (started on demand) and follows a job the same way – Ctrl-C cancels it.
- The same dispatcher is a Rust API (`omaflake_core::api`), so a Rust UI (the Linux app, iced) can link it in-process with exactly
  these methods and JSON shapes: `Server::new(paths, notify)` (`notify: Fn(method, params)` receives the events, from
  the server's threads – it must not block on the server), then `server.handle(request)` (a JSON-RPC object in, the
  response out) or `server.call(method, params)` (the result or an `RpcError`); `server.connect(notify)` adds a
  connection with its own session to the same state. `serve_stdio()`, `service::serve_socket(options)` and
  `call_once(method, params)` are what `omaf serve`, `omaf serve --socket` and `omaf call` run; `client::Client` is a
  blocking client of the socket.
- JSON-RPC batches (arrays) are not supported (`-32600`).

## 2. Session

The first request is `initialize`; anything before it is refused with `-32002`.

```json
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"client":"omaflake-macos","clientVersion":"0.6.2","apiVersion":1,"language":"de","updateFeed":"https://…/feed.json"}}
← {"jsonrpc":"2.0","id":1,"result":{"apiVersion":1,"omafVersion":"0.6.2","host":{"system":"aarch64-darwin","os":"macOS 27.0.1"},"dataFolder":"/Users/…/.omaflake","configFile":"/Users/…/.omaflake/config.json"}}
```

- `apiVersion` is an integer. Within a version, changes are **additive only** (new methods, new optional params, new
  result fields, new events, new enum values that clients show as "unknown"); clients ignore what they do not know.
  Anything else is a new version; the server then serves the older one for at least one minor release.
- `language` (optional): the UI's language (`"en"`, `"de"`, …) – recipe descriptions in that language where a recipe
  has one.
- `updateFeed` (optional): the app's update feed URL (its Info.plist `OMUpdateFeedURL`) – `updates.check` asks it
  (`OMAFLAKE_UPDATE_FEED` in the environment takes precedence).
- An `apiVersion` the server does not serve is refused with `-32602`. `initialize` migrates older data folder layouts
  (plan B108) before anything else reads them.
- `events.subscribe` `{topics: [...]}` starts the events (§6); without it the server sends none.
- `shutdown` → `null`, then this session ends: a private server exits; the service closes this connection and goes
  on for its other clients and its jobs.
- `service.restart` → `{jobs}`: the service ends once no job waits or runs (`jobs`: how many it waits for; 0 – at
  once). For updates (§1) and `omaf service stop`. A private server only notes it.

## 3. Errors

Standard JSON-RPC codes for protocol errors (`-32700` parse error, `-32600` invalid request, `-32601` unknown method,
`-32602` invalid params, `-32002` not initialized). Everything the core refuses or fails at is `-32000` with a
message for people and a stable code for programs:

```json
{"code":-32000,"message":"research is running – shut it down first",
 "data":{"kind":"vm-running","text":{"key":"%@ is running – shut it down first",
         "text":"research is running – shut it down first","args":["research"]}}}
```

`data.kind` (kebab-case) lets a UI react; the `message` is English. `data.text` (plan B125) is the message as a
**text** (§10.2): its key is the core's format string (`vm_err!("{} is running – shut it down first", name)` – each
`{…}` becomes `%@`, `%` becomes `%%`), its args the values filled in, as strings (or a text of its own: a message with
a prefix is `"%@%@"` with the prefix and the inner text; an OS error's key is the OS's text, e.g. "No such file or
directory"). A UI shows it in its language like any view text – the macOS app, `omaf ui` and the Linux app do – and
falls back to `message`. An error without a key (a message built at run time) has no `data.text`; one without kind and
key leaves `data` out. The same text is in a failed job's `errorText` (next to `error`, §5) and in a page that cannot
be built (`model.error.text`, §10.1). Protocol errors (`-32xxx` other than `-32000`) are for developers and stay
English. Some methods add fields to `data` next to `kind`: `networkVolume: true` (`vms.useExternal`),
`replacesConfiguration: true` (`builder.apply` without `force`). A panic inside the core is `internal` – the server
keeps serving. Kinds are added
as the core grows; a client treats an unknown kind as a plain error. Defined so far: `not-found`, `exists`,
`vm-running`, `vm-stopped`, `in-use`, `mac-limit`, `unsupported-system`, `unreadable-file`, `newer-file`,
`cancelled`, `prerequisite`, `network`, `auth`, `internal`.

## 4. Types

All fields camelCase; optional fields are left out when unset. Times are ISO 8601 in UTC (`2026-10-23T09:41:00Z`),
sizes in bytes, paths absolute.

### 4.1 Vm

```json
{
  "name": "research",
  "recipe": "omaflake.nixos.dev@velvet-tiger-orbit",
  "image": "omaflake.nixos.dev@velvet-tiger-orbit-…",
  "kind": "nixos",                      // nixos | macos
  "system": "aarch64-linux",
  "status": "running",                  // stopped | running
  "busy": false,                        // an action on it is in progress in this server
  "cpus": 4, "memoryGB": 8, "diskGB": 64,
  "network": "shared",                  // shared | isolated | none
  "share": true, "sharePath": "/…",     // sharePath (when share is on): the default share/<name> or the user's folder
  "ephemeral": false,
  "sshPort": 2201,                      // NixOS only
  "user": "alex",                       // guest user (SSH)
  "audio": true, "microphone": false, "clipboard": true, "camera": false,
  "display": "gpu",                     // none | window | gpu (how a plain start shows it)
  "created": "2026-10-20T08:00:00Z",
  "identity": "amber",                  // its omaflake ID words (spec §11.11), as many as idLengths.vm
  "omaflakeId": "vm:research:amber-lake-…", // the whole ID (all twelve words); UIs shorten it to idLengths.vm
  "label": "research",                  // as the UI shows it
  "address": "192.168.64.5",            // macOS guests while running: NAT address
  "agent": { … },                       // what the omaflake agent last reported (spec §11.9; agent.json), while running
  "accelerator": "kvm",                 // NixOS while running: hvf | kvm | tcg (Linux without /dev/kvm: emulated, slow)
  "location": "/Volumes/…",             // the package's place when it runs from elsewhere (plan B108)
  "latestImage": "…",                   // the newest image of its recipe's line
  "upgradable": true
}
```

### 4.2 Image

`name`, `recipe`, `source` (the recipe source's id, when known), `kind`, `system`, `revision`, `storePath`,
`created`, `line`, `path`, `sizeBytes`, `label` (its omaflake ID as short as it can be), `omaflakeId` (the whole ID),
`instances` (VM names using it), `current` (it is the newest of its line), `outdated` (its recipe evaluates to
something else now, plan B73/B82 – left out while that is not known: the check runs `nix eval` in the background, and
a recipe version that is gone has nothing to compare with; it is checked again right after the guest settings change),
`look` (the guest look it was built with, plan B135 – left out for an image from before looks). `sizeBytes` (allocated blocks, so it walks the image's
files) is only in `images.get`, not in the lists.

### 4.3 Recipe

The recipe's `recipe.toml` values (`kind`, `systems`, `cpus`, `memoryGB`, `diskGB`, `network`, `share`, `ephemeral`,
`user`, `name`, `author`, `audio`, `microphone`, `clipboard`, `camera`, `description` – in the session's language when
the recipe has it –, `lab`, `display`, `tags`, `license`, `homepage`) plus:
`id`, `base`, `qualified` (`omaflake/nixos/dev`), `displayName`, `label`, `runsHere`, `source` (source id),
`line`, `distinction` (what tells it apart from a recipe of the same author/kind/name, plan B110), `folder`,
`installed` (`{source, version, installed}` for downloaded ones), `image` (its line's current image), `outdated`
(the recipe changed since that image was built, plan B73/B82), `versions` (its line's versions, newest first),
`parts?` (what it is made of, plan B142 – left out when its recipe.toml cannot be read): `{modules: [{reference (with
the versions it allows, "omaflake/neovim ≥ 1.0.0"), title?, version?, provides}] (title and version: the module here,
none when it is not here in an allowed version), look?, lookId? (the installed look when it is here), keymaps,
bindings: [{module, action, title, titleDe?, keys (each does it), context ("desktop" or a module), modes? (the module's
modes it works in), after? (the mode afterwards), does? (the program's own command), own? (the program's key, shown
only), from? (the key binding set that gave the key or `after`)}] (plan B147), issues: [{kind: missingModule|missingKeymap {reference, have?} (they block a build) | needs {module,
capability} | incompatible {capability, modules} | keysOverlap {first, second} | keyRule {binding}}], unfree,
requirements: {cpus, memory_gb, disk_gb}, locales: {rtl?, languages?}}`. `cpus`, `memoryGB`, `diskGB` are the
recipe's `[requirements]`, `network`, `share`, … its `[vm]`. `lab` is a boolean here (false when unset).

**Version** – `id`, `source`, `added` (left out for the recipe a local folder has now), `newest` (the line's
newest), `live` (in a local folder source, edited in place), `images` (built from it), `label` (its omaflake ID as
short as it can be).

`RecipeSettings` (what `vms.create` takes): the runtime values of a recipe – `kind`, `cpus`, `memoryGB`, `diskGB`,
`network`, `share`, `ephemeral`, `user`, `audio`, `microphone`, `clipboard`, `camera`, `display`.

### 4.4 Others

- **Source** – `id`, `kind` (`github` | `url` | `folder`), `location`, `branch`, `displayName`, `enabled`,
  `private`, `official`, `hasToken` (a private repository's token is in the system keychain – read without its secret,
  so no keychain prompt; false for public sources).
- **CatalogRecipe** – an entry of a source's `catalog.json` (spec §11.3: `id`, `author`, `kind`, `name`,
  `description` in the session's language, `tags`, `license`, `lab`, `systems`, `modules`, `look?`, `keymaps` – as
  references with the versions they allow –, `homepage`, `version`, `files: [{path, size, sha256}]`) plus `source` and
  `runsHere`.
- **Prerequisite** – `item` (`host`, `kvm`, `nix`, `flakes`, `nixDarwin`, `linuxBuilder`, `qemu`, `flake`, `sshKey`,
  `credential` (at least one credential – the default; images and macOS bases are made with it, plan B139), `look` (at
  least one installed look – the default, plan B141; its action `looks.installRecommended`), `git`,
  `cli`, `builderRunning`, `flakeEval`, `nixSpace`, `ipsw`, `macBase`, `macSpace`; a Linux host checks `host`, `kvm`,
  `nix`, `flakes`, `qemu`, `flake`, `sshKey`, `credential`, `look`, `git`, `cli`, then `flakeEval`, `nixSpace` – no macOS preflight, plan B124), `status` (`ok` | `limited` | `missing`),
  `title` (English; UIs translate by `item`), `required`, `detail`, `hint`, `install` (a command), `settablePath`,
  `freeBytes`, `neededBytes`, `baseUser`, `expectedUser`. `git` and `cli` (the `omaf` command on the user's PATH) are
  optional (`required` false, not counted in `issues`).
- **MacOSRelease** – `version`, `build`, `url`, `bytes`.
- **Unreadable** – a file this omaflake cannot read (too old, too new, damaged; plan B114): `name` (the VM, image or
  recipe it belongs to – the list it comes in tells which), `reason`.
- **Job** – §5.

## 5. Jobs

Work that takes minutes runs as a **job** in the server: image builds, the macOS base, the IPSW download, the Linux
builder tasks, imports and exports, moving the data folder. The method returns at once with the job; its progress
comes as events, and `jobs.*` ask and steer.

```json
{
  "id": "j7",
  "kind": "build",          // build | macos-install | ipsw-download | builder-install-nix | builder-mount-nix | builder-setup
                            // | builder-apply | builder-clean-up | cli-install | import | export | data-move
                            // | recipe-install | look-install | module-install
                            // | recipe-install
  "title": "Build dev",
  "recipe": "omaflake.nixos.dev@…",    // build, recipe-install
  "guestKind": "nixos",
  "state": "running",       // queued | checking | running | finished | failed | cancelled
  "progress": {"step": 3, "total": 5, "title": "Copying the image", "done": 0, "of": 0, "fraction": 0.62},
                            // builds: steps from the log; downloads: step/total 0, done/of in bytes
  "started": "…", "finished": "…",
  "lastOutput": "…",        // when it last wrote a line (a time)
  "cancellable": true,      // jobs.cancel would do something now
  "logLines": 412,          // lines so far (jobs.log reads them)
  "hiddenLines": 37,        // routine lines left out of the log (still in logFile)
  "logFile": "/…/logs/image-build-….log",  // every line of the job, also the routine ones
  "result": { … },          // when finished – per kind, below
  "error": "…",             // when failed
  "errorText": {…}          // the error as a text when it has a key (§3)
}
```

Every job has a `logFile`: builds `logs/image-build-<recipe>.log`, the others `logs/<kind>[-<name>].log` (the data
move writes `data-move.log` next to config.json, as `logs/` moves along). Downloads (`ipsw-download`, the download of
`macos-install`) count `done`/`of` in bytes with `step`/`total` 0.

| Kind | `result` |
|---|---|
| `build` | `{image, removed: [image]}` – older images of its line removed (plan B52) |
| `recipe-install` | `{installed: [recipe id]}` – empty: it was up to date |
| `import` | `{kind: "vm"|"image"|"recipe", names, notes}` – names: the VM's or image's name, the recipe ids |
| `export` | `{path}` – the package written |
| `data-move` | `{dataFolder}` |
| `ipsw-download` | `{file, version}` |
| `macos-install` | `{base, existing, ready}` |
| `builder-install-nix`, `builder-mount-nix`, `builder-setup`, `builder-apply`, `builder-clean-up` | `{message}` – what it did, in English |
| `cli-install` | `{link, installed, message}` – the link, whether it is there now (install) or was removed; `message` in English |

- **Builds** queue: one build per guest kind at a time, NixOS and macOS side by side (plan B60). Right before it
  starts, a build runs the Doctor's checks for its guest kind and fails with the first blocker (`checking` state).
  A finished build prunes older images of its line (current + one before + those in use, plan B52).
- `jobs.cancel` takes a waiting job out of the queue or stops a running one (its subprocesses end, a macOS build VM
  is forced off, a half-built image is deleted). Running imports, exports, recipe installs, the data move and the
  Linux builder tasks cannot be stopped (`cancellable: false`); `macos-install` stops only during its download (OPEN:
  the installer has no cancellation yet).
- `images.build` for a recipe with a waiting or running build returns that job; a new build of a recipe replaces the
  recipe's ended build in the list. `recipes.install` likewise returns a running install of the same recipe.
- The log keeps the newest 5000 lines (`jobs.log` starts at the oldest kept); a build's full log is its `logFile`.
- Jobs live as long as the server: in the service they outlive the client that started them, and every client sees
  them. A private server that restarts loses them (their log files stay).

## 6. Events

`events.subscribe {"topics": ["vms", "images", "recipes", "jobs", "doctor", "messages"]}`. The server watches the data
folder (every 2 s while subscribed, and right after its own actions) and sends a topic's **whole list** when it
changed – a UI replaces what it shows. A newly subscribed topic gets its list at once (`jobs`: a `jobs.changed` per
job); `events.subscribe` replaces the subscription, unknown topics are left out, `[]` stops the events. Subscribing to
`doctor` runs the checks once. Update checks run only when a client asks (`updates.check`).

| Event | Params | When |
|---|---|---|
| `vms.changed` | `{vms: [Vm], unreadable: [Unreadable]}` | a VM came, went, started, stopped, changed |
| `vm.ended` | `{name, requested}` | a running VM is off; `requested` = it was stopped through this server (in the service: by any client) |
| `vm.ready` | `{name, via}` | after a start through this server: the agent reported in (`agent`) or SSH answers (`ssh`) |
| `images.changed` | `{images: [Image], unreadable: [Unreadable]}` | images built, imported, removed |
| `recipes.changed` | `{recipes: [Recipe], …}` (as `recipes.list`) | recipes installed or removed, a source or the flake changed on disk, freshness re-evaluated |
| `jobs.changed` | `{job: Job}` | a job's state or progress changed |
| `job.log` | `{id, lines: [string]}` | new log lines of a job |
| `doctor.changed` | as `doctor.check` | the checks ran (after actions, before builds) |
| `message` | `{text, level}` | something the user should read that a client cannot tell from the other events or its own answers (`info` | `warning` | `error`; English): older recipe versions pruned by `settings.set`, a recipe list that cannot be read (once per new error), a macOS VM created but not started (`vms.create` beyond the limit) |
| `view.changed` | `{page, params, model}` | a page this connection subscribed with `views.subscribe` looks different now (§10) – sent without a topic, only to that connection |

A client that does not use the views (§10) words the rest itself, in its language: the end of a job (`jobs.changed`
with an ended `state`, its `result` or `error`), `vm.ended` and `vm.ready` come without a `message`. With the views,
all of these – and the `message`s, with a key – are lines of the activity page (§10.6), worded by the core.

## 7. Methods

Params and results in short; the types are those of §4.

### Session and settings

| Method | Params | Result |
|---|---|---|
| `initialize` | `client, clientVersion, apiVersion, language?, updateFeed?` | `apiVersion, omafVersion, host: {system, os}, dataFolder, configFile` |
| `events.subscribe` | `topics` | `null` |
| `shutdown` | – | `null` – ends this session (§2) |
| `service.restart` | – | `{jobs}` – the service ends once no job waits or runs (§1, §2) |
| `settings.get` | – | `{settings, guestDefaults, choices: {timeZones, locales, keyboards: [{code, locale}], looks: [{name, title}]}, isDefaultDataFolder}` – `settings`: config.json's values with their defaults applied (`developerMode, checkForUpdates, notifications, keepRecipeVersions, hostKeyboard?, defaultCredential?, defaultLook?, defaultLocale?, flake?, qemu?, nix?, runner?, flakeInUse?, dataFolder`); `choices.looks`: the installed looks (`name` is the look's id, plan B141); `hostKeyboard`: this computer's own keyboard (plan B141; unset: as the system says) – preset for new locales and the guest's keyboard without a locale; `guestDefaults`: this computer's values (what an image without a locale gets) |
| `settings.set` | any of `developerMode, checkForUpdates, notifications, keepRecipeVersions, hostKeyboard, defaultCredential, flake, qemu, nix, runner` (`null` or `""` = back to the default; keys not given stay; `hostKeyboard` takes a keyboard of `choices.keyboards` – the guest's language, time zone and keyboards are a locale's (`locales.*`), the default look is set with `looks.setDefault` (plan B141); `defaultCredential` takes a credential's id – there is always a default while there are credentials, plan B139) | `settings` – turning `keepRecipeVersions` off prunes older recipe versions |
| `looks.list` | `fetch?` | `{looks, offered, unreadable, fetched}` (plan B141) – `looks`: the looks here `[{id, reference, author, name, title, description, tags, appearance?, plain, swatches?, source, sourceName, official, installed, update, isDefault, images}]` (installed from a source, or a local folder source's, used as it is there); `offered`: what the sources' catalogs offer besides (`fetch: true` – or the first call – looks into them) |
| `looks.install` | `id, source` | Job `look-install` – downloads a look (its files checked against the catalog's SHA-256) into `<data>/looks/<author>.<name>/`; the first one becomes the default |
| `looks.installAll` | `items: [{id, source}]` | `[Job]` – job `look-install` per source: the looks the Looks tab offers ("Install All") |
| `looks.installRecommended` | – | Job `look-install` – omaflake's own look and Catppuccin from the official catalog (the Doctor's action) |
| `looks.remove` | `id` | `null` – refused while images are built with it (`in-use`); another one becomes the default |
| `looks.setDefault` | `id` | `null` – the look new images get unless the build chooses another |
| `credentials.list` | – | `[{id, name, user?, fullName?, email?, hasPassword, isDefault}]` (plan B137; a password never leaves the keychain/keyring) |
| `credentials.add` | `name, user, password, fullName?, email?` | `{credential, note?}` – every credential has a user and a password (plan B138, B139); the first one becomes the default; `note` says when the password went to a file (no keyring) |
| `credentials.update` | `id`, any of `name, user, fullName, email, password` (`""` clears a value; a password can be changed, not removed) | `{credential, note?}` – while images use it only `name` and `password` may change, while a macOS base uses it only `name` (`in-use` otherwise) |
| `credentials.remove` | `id` | `null` – its password goes too; refused while images or a macOS base use it (`in-use`); another one becomes the default |
| `credentials.copyPassword` | `id` | `null` – asks the owner first (macOS: Touch ID or the login password, LocalAuthentication), copies the password to the clipboard and clears it after 90 s; refused when not confirmed (plan B138) |
| `locales.list` | – | `[{id, name, locale, timeZone, keyboards, isDefault, images}]` (plan B141) – where a guest lives: one language and region, one time zone, one to three keyboards (the first the default); kept in config.json, never in a catalog |
| `locales.add` | `name, locale, timeZone, keyboards` | `{id}` – the first one becomes the default; names are unique (any case, `exists`); one to three keyboards, none twice |
| `locales.update` | `id`, any of `name, locale, timeZone, keyboards` | `{id}` – while images are built with it only `name` may change (`in-use`) |
| `locales.remove` | `id` | `null` – refused while images are built with it (`in-use`); another one becomes the default |
| `locales.setDefault` | `id` | `null` – the locale new images get unless the build chooses another |
| `profiles.list` | – | `[{id, name, cpus, memoryGB, isDefault}]` (plan B142) – how big a new VM is; kept in config.json |
| `profiles.add` | `name, cpus?, memoryGB?` | `{id}` – the first one becomes the default; names are unique (any case) |
| `profiles.update` | `id`, any of `name, cpus, memoryGB` | `{id}` – VMs created with it keep their size |
| `profiles.remove` | `id` | `null` – another one becomes the default |
| `profiles.setDefault` | `id` | `null` – the profile new VMs get unless another is chosen |
| `modules.list` | `fetch?` | `{parts, offered, unreadable, fetched}` (plan B142) – `parts`: the modules and key binding sets here `[{id, reference, kind: module|keymap, platform?, title, description, version, tags, provides, needs, enhances, keys (a module's actions → default key; a set's "<module> <action>" → key), unfree, source, sourceName, official, installed, update, recipes (the recipes here made of it)}]`; `offered`: what the catalogs offer besides (`fetch: true` – or the first call – looks into them) |
| `modules.install` | `id, source` | Job `module-install` – a module (`<author>.<platform>.<name>`) or key binding set (`<author>.keymap.<name>`), every file checked against the catalog's SHA-256 |
| `modules.installAll` | `items: [{id, source}]` | Job `module-install` – the modules and key binding sets the Modules tab offers, one after the other; one that fails does not keep the others out |
| `modules.remove` | `id` | `null` – recipes made of it cannot be built until it is back |
| `bases.list` | – | `[{name, path, osBuild?, user?, ready, newest, created?, sizeBytes?, credential?, credentialName?}]` – the macOS bases (`bases/mac-base-*`; plan B138), with the credential of their user (plan B139) |
| `bases.remove` | `name` | `null` – deletes the base; refused while a macOS base is installed or a macOS image is built |
| `bases.assign` | `name, credential` | `null` – writes the credential (ID or name) into a base created before bases kept theirs (`omaflake-base.json`, plan B139); its user must be the base's |
| `dataFolder.move` | `target` | Job `data-move` (all VMs must be stopped) |
| `updates.check` | – | `{state: "noFeed"|"upToDate"|"available"|"off", release?: {version, url, sha256?, notes?, page?}}` – a feed that cannot be read is an error (`network`) |

### VMs

| Method | Params | Result |
|---|---|---|
| `vms.list` | – | `{vms: [Vm], unreadable}` |
| `vms.get` | `name` | `Vm` |
| `vms.create` | `name, image, settings: RecipeSettings, sharePath?, start?` | `Vm` – from exactly that image (plan B100); `start` starts it after creating (a macOS VM beyond the limit is created, not started, with a `message`) |
| `vms.start` | `name, display?: "auto"|"none"|"window"|"gpu", gl?: "on"|"es"` | `Vm` – `auto` (default): the UI's start – a window, with 3D where QEMU can; refused beyond the macOS limit (`mac-limit`) |
| `vms.stop` | `name, timeout? (s, 60)` | `{result}` – `alreadyStopped`, `powerdown`, `quit`, `forcedOff`, `killed` |
| `vms.reset` / `vms.remove` | `name` | `null` – refused while running |
| `vms.upgrade` | `name, image?` | `Vm` – to its line's newest image unless `image` is named; refused while running |
| `vms.set` | `name, audio?, microphone?, clipboard?, camera?` | `{vm, takesEffectAtNextStart}` |
| `vms.ssh` | `name, command?: [string]` | `{argv, shellLine}` – what to run in a terminal |
| `vms.agent` | `name, wait? (s)` | the agent's report (spec §11.9; agent.json) or `null` – `wait`: up to that long for a report that says the agent is up |
| `vms.log` | `name, lines? (200), qemu?` | `{lines: [string]}` – NixOS: the serial console (`qemu`: QEMU's own log); macOS: the runner's log |
| `vms.wait` | `name, timeout? (s, 300)` | `{via}` – blocks until SSH or the agent answers; `vm-stopped` when it does not run |
| `vms.share` | `name` | `{path}` – the share folder, created if it is the default one |
| `vms.nextName` | `recipe` | `{name}` – the first free VM name for it (`dev`, `dev-2`, …) |
| `vms.forget` | `name` | `null` – stop listing a VM that runs from elsewhere (its package stays) |
| `vms.useExternal` | `path, confirmNetwork?` | `Vm` – a VM package that runs from where it is; `{networkVolume: true}` in the error data when it is on a network volume and `confirmNetwork` was not set |
| `vms.limits` | – | `{macMax, macRunning: [name]}` |

### Images

| Method | Params | Result |
|---|---|---|
| `images.list` | – | `{images: [Image], unreadable}` |
| `images.get` | `name` | `Image` with `sizeBytes` |
| `images.build` | `recipe, credential?, look?, locale?, base?, keepFailed?` | Job `build` – `credential`: a credential's id, left out (or `""`) the default – without any credential `prerequisite`; `look`: an installed look (id, `<author>/<name>`, name or title), left out (or `""`) the default (plan B137, B141) – its data goes to the image flake (`settings.look`); without any look the Doctor blocks the build; left out, the recipe's suggested look when it is installed (plan B142);
`locale`: a locale's id or name, left out (or `""`) the default – without any locale the image gets this computer's
values (plan B141). A recipe whose modules or key binding sets are not here is refused (`prerequisite`, plan B142). A macOS recipe takes its base's credential and its stored password – nothing is asked (plan B139); a base without a credential is refused (`prerequisite`); `base`: the base folder, default the newest; `keepFailed`: keep a failed build folder) |
| `images.remove` | `name` | `null` – refused while VMs use it (`in-use`) |

### Recipes and sources

| Method | Params | Result |
|---|---|---|
| `recipes.list` | `all?` | `{recipes: [Recipe], lines: {line: [Version]}, idLengths: {recipe, image, vm}, qemuGL, unreadable, labHidden}` – lab recipes only in developer mode or with `all` |
| `recipes.available` | – | fetches the enabled remote sources' catalogs: `{offered: [CatalogRecipe], updates: [{id, recipe: CatalogRecipe}], suggested: [CatalogRecipe], errors: {sourceId: message}}` |
| `recipes.install` | `id, source` | Job `recipe-install` – the recipe and what it is made of that is not here: its modules, key binding sets and look, each from the first source that has it in a version the recipe allows (the official catalog last, plan B142) |
| `recipes.installParts` | `id` | Job `recipe-install` – what a recipe here is made of and is missing (the build page's "Install Missing Parts") |
| `recipes.save` | `name, platform, description?, modules: ["<author>/<name>"], look?, keymaps?, requirements: {cpus, memory_gb, disk_gb}, vm: {display, network, share, ephemeral}, previous?` | `{base}` – a recipe of your own, written into My Recipes (`<data>/my-recipes/recipes/my/<platform>/<name>/recipe.toml`, recipe v2), which becomes the first source; `previous` ("my.nixos.work"): the recipe edited – renamed, its old folder goes |
| `recipes.delete` | `base` | `null` – deletes one of your own recipes ("my.<platform>.<name>"); images built from it stay |
| `keymaps.save` | `name, title?, description?, keys: {"<author>/<name>": {"<action>": key \| [keys] \| {key?, after?}}}, previous?` | `{reference}` ("my/<name>") – a key binding set of your own, written into My Recipes' folder (`keymaps/my/<name>/keymap.toml`, keymap v2; plan B147), which becomes the first source; `previous`: the set edited – renamed, its old folder goes |
| `keymaps.delete` | `name` | `null` – deletes one of your own key binding sets; recipes that name it get each module's own keys |
| `recipes.installSuggested` | – | `[Job]` |
| `recipes.remove` | `id` | `null` |
| `recipes.removeLine` | `line` | `{removed: [id], kept: [id]}` – kept: images use them, or they live in a local folder |
| `recipes.pruneVersions` | – | `{removed: [id]}` |
| `sources.list` | – | `[Source]` |
| `sources.add` | `input, private?, token?, branch?, first?` | `{source, note}` – checked before it is kept (`exists` when it is a source already); a private repository needs `token`, which goes to the system keychain, never into a file |
| `sources.remove` | `id` | `null` (its token leaves the keychain) |
| `sources.setEnabled` | `id, enabled` | `[Source]` |
| `sources.move` | `id, up` | `[Source]` |
| `sources.check` | – | `{results: {sourceId: {ok, note}}}` |

### Packages (spec §11.10)

| Method | Params | Result |
|---|---|---|
| `packages.inspect` | `path, expect?: "vm"|"image"|"recipe", external?` | `{kind, valid, reason?, conflict?: {vmName, suggestedName}|{image}, networkVolume}` – `reason` a text (§10.2; today the core's English message, no key); what an open panel checks before it closes, and what an import would ask about (`suggestedName`: the first free name to import a VM under) |
| `packages.import` | `path, name?, replace?` | Job `import` → `result: {kind, names, notes}` (§5) |
| `packages.export` | `kind: "vm"|"image"|"recipe", name, folder, withImage?` | Job `export` → `result: {path}` (`withImage` unset: as `Transfer::carries_image` decides) |

### Doctor, macOS base, Linux builder

| Method | Params | Result |
|---|---|---|
| `doctor.check` | – | `{prerequisites, nixosPreflight, macosPreflight, latestRelease?, buildBlockers: {nixos, macos}, startBlockers: {nixos, macos}, issues, requirementsMissing, hasMacBase, macBaseUser?, builderSystemIsOmaflakes, cli?, checked}` – blockers are Prerequisites; `builderSystemIsOmaflakes`: this Mac's nix-darwin system comes from omaflake's image flake (else `builder.apply` needs `force`); `cli: {link, target?, installed, blocked, onPath, admin}`: the `omaf` command's link (where Install puts it, this omaflake's `omaf` it points to – absent when the core does not run as `omaf` –, omaflake's link is there, a file that is not omaflake's is in the way, the link's folder is on the login PATH, Install asks for an administrator); a build runs these checks right before it starts, `vms.start` refuses a start blocker of the last check (`prerequisite`) |
| `doctor.setPath` | `item: "nix"|"qemu"|"flake", path` | as `doctor.check` |
| `macos.downloadIpsw` | – | Job `ipsw-download` → `result: {file, version}` (cancel pauses; it continues from there) |
| `macos.installBase` | `credential?, ipsw?` | Job `macos-install` → `result: {base, existing, ready}` – the guest user, full name and password from the credential (ID or name; default: the default credential), recorded in the base (plan B139); `user, fullName, password` directly still work (API version 1) |
| `doctor.installCli` / `doctor.uninstallCli` | – | Job `cli-install` → `result: {link, installed, message}` – the `omaf` command (plan B122): a symlink to this omaflake's `omaf` (inside omaflake.app the bundled `Contents/MacOS/omaf`, else the running executable or the `omaf` next to it). macOS: `/usr/local/bin/omaf` through the administrator dialog (a dismissed dialog cancels the job); Linux: `~/.local/bin/omaf`, no root. Install replaces a link or an older `omaf` there and refuses another file (`exists`); Uninstall removes only omaflake's link (`not-found` otherwise); one at a time (`in-use`) |
| `builder.installNix` / `builder.mountNix` / `builder.setup` / `builder.apply` (`force?`) / `builder.cleanUp` | – | Job → `result: {message}` – each asks for an administrator through the system's own dialog; one at a time (`in-use`); `builder.apply` on a nix-darwin system that is not omaflake's needs `force` (error data `replacesConfiguration`); a dismissed dialog cancels the job |

### Views (§10)

| Method | Params | Result |
|---|---|---|
| `views.get` | `page, params?` | the page's model |
| `views.subscribe` | `pages: [{page, params?}]` | `null` – replaces the subscription; each page comes at once as `view.changed`, then whenever its model changes; `[]` stops them |
| `activity.clear` | – | `null` – empties the activity list |
| `recipes.installAll` | `items: [{id, source}]` | `[Job]` – installs each recipe, with what it is made of ("Install All" of the Recipes tab) |
| `recipes.installUpdates` | – | `[Job]` – installs every newer version the sources have of downloaded recipes ("Update All") |

### Jobs

| Method | Params | Result |
|---|---|---|
| `jobs.list` | – | `[Job]` |
| `jobs.log` | `id, from?` | `{lines, next}` |
| `jobs.cancel` | `id` | `Job` |
| `jobs.dismiss` | `id` | `null` – an ended job leaves the list (`in-use` while it runs) |

## 8. Secrets

Passwords (`images.build`, `macos.installBase`) and tokens (`sources.add`) travel only over the pipe between the UI
and its own `omaf serve`, or over the service's socket (mode 0600, this user's connections only, §1). The server never logs params of these methods, keeps a password only until it is used and
stores tokens only in the system keychain (macOS Keychain; Linux: OPEN).

## 9. Status

API v1 is new with the Rust core (plan B117). The method tables are the contract; the Rust types in
`crates/omaflake-core/src/api/types.rs` are its implementation, and their JSON Schema is OPEN (to be generated from
those types and published under `schema/api/`).

**Contract tests.** `crates/omaflake-core/testdata/api/*.json` holds a representative value of every result, event
and error (and the params `initialize` and `vms.create` get): the Rust tests (`api::contract`) write them from the
types (`OMAF_UPDATE_GOLDEN=1 cargo test -p omaflake-core api::contract` after a deliberate change, otherwise they
compare), and the Swift tests (`APIContractTests`) decode each with the app's types – strictly, a required field is
never defaulted. The page models of §10 are in `testdata/api/views/` (every page from one fixture context, the
`view.changed` event and the params of `views.get`/`views.subscribe`); the interfaces decode against them. `CoreEndToEndTests` starts the real `omaf serve` (`OMAF_BIN=target/debug/omaf swift test`) on a
fresh data folder and decodes every read method and the events.

## 10. Views – what each page shows

The interfaces – the SwiftUI app (macOS), the iced app (Linux) and the terminal interface `omaf ui` (ratatui, macOS
and Linux) – share not widgets but **what each page shows** (plan B121): the core builds a *page model* with the rows,
their labels, lamps and badges, the actions with whether they can be done now and why not, the questions they ask,
filters, sorting and form fields with their defaults and what is wrong with them. A UI draws it with the design tokens
(B119), sends the actions it names and keeps only what is its own: which tab and page are open, which rows are
expanded, the filter it asked with, what the user is typing, whether the activity was seen, launch at login, its
language. The model is the app's presentation as of B117, moved into `crates/omaflake-core/src/api/views/` (one module
per page; the Rust types there are the contract, the JSON below their shape).

### 10.1 Getting and following pages

```json
→ {"jsonrpc":"2.0","id":5,"method":"views.get","params":{"page":"vms","params":{"filter":{"status":true}}}}
← {"jsonrpc":"2.0","id":5,"result":{"title":{"key":"Virtual Machines","text":"Virtual Machines"},"count":1,"of":2,…}}
→ {"jsonrpc":"2.0","id":6,"method":"views.subscribe","params":{"pages":[{"page":"home"},{"page":"vms","params":{"filter":{"status":true}}}]}}
← {"jsonrpc":"2.0","method":"view.changed","params":{"page":"vms","params":{"filter":{"status":true}},"model":{…}}}
```

- `views.get {page, params}` builds a page now. Unknown pages are `-32602`; a VM, image or image for a new VM that is
  not there is `not-found`.
- `views.subscribe {pages}` replaces **this connection's** subscribed pages (a page with other params is another page –
  a UI that changes a filter subscribes again). With the service (§1) every connection subscribes its own pages and
  gets `view.changed` only for them, built in its language (`initialize`); `views.get` works on any connection. Each
  page new in the subscription comes at once as `view.changed {page, params, model}` (one that stays subscribed with the
  same params is not sent again for it), then again whenever its model
  differs from the one sent last: the watcher rebuilds subscribed pages at every look at the data folder (2 s), after
  actions, when a job changes and when the activity list grows (at most every 0.3 s). The whole model comes each
  time – a UI replaces what it shows. A page that cannot be built comes as `model: {"error": {message, kind, text}}`
  (`text` as in §3, null without a key).
  Subscribing to `doctor` or `macBase` runs the Doctor's checks once if they never ran.
- Typical: a UI subscribes to `home`, `activity` and the page it shows (with its params); the build page's log comes
  with the `jobs` topic (`job.log`) or `jobs.log`.
  `omaf ui` (crate `omaflake-tui`) does exactly this and is the reference client of the page models: every page is drawn
  from its model only.

Pages and their params:

| Page | Params | What |
|---|---|---|
| `home` | – | the header's buttons (Doctor with its issue count, Settings, data folder, Quit), the tabs with their badges and `group`, and `groups: [{id, title, icon, attention}]` – `run` (VMs, images, recipes, activity) and `workshop` (modules, looks, credentials, locales, profiles; plan B147) |
| `vms` | `filter?` | the VMs tab (§10.5) |
| `vm` | `name` | one VM: `{row}` as in `vms` (a TUI's detail screen) |
| `images` | `filter?` | the Images tab: build jobs on top, then every image |
| `image` | `name` | one image: `{row}`, with its size |
| `recipes` | `filter?` | the Recipes tab: one row per line, updates, offers, the first start's suggestion |
| `newVm` | `image, values?` | the New VM form for exactly that image (plan B100) |
| `build` | `recipe` | the Build Image page of one recipe |
| `doctor` | – | versions and updates, the checks in three sections with their actions |
| `credentials` | – | the credentials, each with its usage and Copy Password (plan B137, B138) |
| `looks` | – | the Looks tab: the looks here with their colours, usage and the default, what the sources offer besides (plan B141) |
| `modules` | – | the Modules tab: modules and key binding sets here with the recipes made of them, what the sources offer (plan B142; the looks' shape) |
| `locales`, `locale` | `id?, values?` | the locales and one locale's form (plan B141; the credentials' shapes) |
| `profiles`, `profile` | `id?, values?` | the profiles and one profile's form (plan B142; the credentials' shapes) |
| `recipeStudio` | `id?, values?` | a recipe of your own put together from the catalog's modules, a look and key bindings (plan B147; before it `recipeEditor`, plan B142) |
| `credential` | `id?` | one credential's form: new, or to edit (only the name while images use it) |
| `baseCredential` | `name` | the same form for a macOS base from before bases kept their credential: which one its user has (plan B139) |
| `keymapStudio` | `id?, values?` | a key binding set of your own (plan B147): `{title, id?, intro, about (name, title, description, platform, base), module, search, hints, groups: [{title, start, way, rows: [{action, title, key (a text field, "h / Left"), after? (a choice of the module's modes), changed, reset? (ui.field with `also`), does?, conflict?, own}]}], changed, actions}`; values `key:<module>:<action>`, `after:<module>:<action>`; Save (`keymaps.save`) gets only what differs from the modules' own keys |
| `keys` | `recipe` \| `image` \| `vm` | a recipe's keys – of an image or a VM through its recipe – by module and by the mode of its program (plan B147): `{title, subtitle, intro, notes, modules: [{reference, title, groups: [{mode?, title, start, way: [{caps, mode}], rows: [{action, keys, caps, title, after?, backToStart, switches, does?, from?, own}]}]}], keymaps, search}`; `caps` are a key's chords as key caps (⌘ ⌃ ⇧ on a Mac host). Rows of recipes, images and VMs whose recipe has keys open it with `ui.show` (action `keys`, icon `keyboard`); the macOS app shows it in a window of its own |
| `macBase` | `values?` | Create macOS Base |
| `settings` | `values?` | recipe sources (with the new source's form), general switches, guest values, data folder |
| `activity` | – | what happened in this server, newest first |
| `import` | `path, expect?, external?` | a step, not a screen (§10.4) |

### 10.2 Texts

Everything a user reads is a **text**: `{key, text, args}`.

```json
{"key": "Upgrade “%@” to %@?", "text": "Upgrade “research” to omaflake.nixos.dev@mint-river-cloud?",
 "args": ["research", "omaflake.nixos.dev@mint-river-cloud"]}
{"text": "/Volumes/Disk/research.omaflake-vm"}
```

- `key` is the **English source string of the macOS app** (`String(localized:)`, `LocalizedStringKey`) with `%@` for
  a text, a time or a size and `%lld` for a whole number – so the app's `de.lproj/Localizable.strings` translates
  them as they are (`"Reset “%@”?" = "„%@“ zurücksetzen?"`). The Rust interfaces (`omaf ui`, the Linux app) use the same table, generated into the crate
  `omaflake-i18n` by `scripts/translations.py` (`--check` in CI; a test fails when it is stale). A key a UI has no
  translation for is its own English format (fill in the args); a translation whose placeholders do not fit the args
  gives way to the English key, then to `text`.
- `text` is the English result. A UI without translations shows `text`.
- No `key`: data shown as it is – a name, a path, a version, an English message of the core (§3).
- `args`, in order: a number (`%lld`), a string (data, not translated), `{"date": "<ISO 8601>"}` and `{"bytes": n}`
  (formatted by the UI in its language – `text` has `2026-10-21 10:00 UTC`, `12.3 GB`), a text of its own (`{key,
  text, args}` – translated first, then filled in: "Open the Doctor first: %@" with the blockers' titles,
  `"%@ · %@ · %@"` for parts joined by " · "), or `{"param": "target"}` – the value the UI asked for that param
  (a confirmation naming the folder just chosen).
- B117's OPEN about core texts is closed: every page, action, question and activity line has a key, and so has every
  error made with `vm_err!` (§3, plan B125) – an activity line or a build page that shows an error carries it as a text
  arg. Every key has German (`de.lproj`); a test (`api::translated`) fails for a view key – from the golden files of
  both hosts and the pages built for a Mac and a Linux host – or an error key without it. Still English only: the
  details of the Doctor's checks (paths, versions), the `job.log` lines, values inside a message (an OS's or a tool's
  output) and the CLI `omaf` (docs/cli.md).

### 10.3 Building blocks

**Action** – something the user can do:

```json
{"id": "remove", "label": {"key": "Remove …", "text": "Remove …"}, "icon": "trash",
 "method": "vms.remove", "params": {"name": "research"},
 "enabled": false, "reason": {"key": "Shut the VM down first", "text": "Shut the VM down first"},
 "destructive": true,
 "confirm": {"title": {"key": "Remove “%@”?", …}, "text": {…}, "button": {"key": "Remove", …}, "destructive": true}}
```

| Field | |
|---|---|
| `id` | stable within its list (`start`, `startHeadless`, `stop`, `terminal`, `export`, …) |
| `label`, `icon` | what the button says / shows; icon-only UIs use `label` (or `help`) as the tooltip |
| `shortLabel` | what a narrow header or toolbar writes ("Doctor" for "Doctor – requirements and preflight", "Data Folder"); absent: `label` |
| `primary` | the default action of its row or page – what Enter, a double click or a click on a build row does, drawn as the prominent button; at most one per row (`actions`) or page. A row without one has no default (Enter opens its details) |
| `method`, `params` | what to send: a core method (§7) or a `ui.*` verb (§10.4) |
| `enabled`, `reason` | greyed out, and why (the first reason that applies) |
| `help` | tooltip while enabled |
| `destructive` | drawn red |
| `on` | a switch: its state now – the action sets the other one (`params` hold the new value) |
| `busy` | its work runs: a spinner instead of / next to the button |
| `badge` | a count on it (the Doctor's issues) |
| `ask` | values to ask first, each put into `params` at its `param` (a dotted path, e.g. `params.path`) – see Ask |
| `confirm` | the question to ask before sending – every UI asks the same |
| `then` | what to do with the answer: `reveal` (show `result.path` in the file manager), `terminal` (run `result.shellLine` – `result.argv` – in a terminal; a TUI suspends itself), `back` (leave the page) |

Order: ask the `ask` values, then `confirm` (its title may name an asked value), then send. A UI shows a refusal where
it likes (a toast, a status line) – the activity list has it already (§10.6).

**Confirm** – `{title, text, button, destructive, input?}`; `button` answers (Cancel is the UI's own);
`input: {param, default}` is a text edited in the question (the name to import a VM under), put into `params` at
`param`.

**Ask** – `{param, kind, label, prompt?, button?, default?, accept?, multiple?, repeat?, check?}`; `kind`: `text`,
`password` (a secret: never shown, never sent to `views.get`), `folder`, `file`, `package` (an omaflake package;
`accept`: `vm`, `image`, `recipe` – a recipe may also be a plain folder; `ipsw` for a file). `prompt`/`button`: the
open panel's message and button. `multiple`: several may be chosen – the action is done once per value. `repeat`: a
password asked twice, sent only when both agree. `check: {method, params, param}`: before the panel closes, call
`method` with `params` and the chosen value at `param` (`packages.inspect`); `valid: false` refuses it with its
`reason` (a text, §10.2). A TUI asks folders and files as a path with completion.

**Lamp** – `{state: on|off|attention|working, help}`: green on/active/built, grey off, orange needs attention, cyan
pulsing something happens (plan B99). **Badge** – `{id, icon, help, tone, label?}` (`label`: a short text drawn instead
of or with the icon: "x86_64", "outdated"). **Tone** – `normal`, `accent` (cyan), `success`, `warning`, `danger`
(design tokens). **InfoLine** – `{label, value, note?, tone?, full?}`: "Label  value", `note` after the value in
`tone`, `full` the whole value where the UI shortens it. **Hint** – `{icon, text, tone, action?}`: an empty list, a
filter that leaves nothing (with "Reset Filter"), an unreadable file, a source that failed.

**List head** (flattened into `vms`, `images`, `recipes`, `activity`) – `title`, `count`, `of` (all rows, when the
filter leaves some out: "1/2"), `filter`, `actions` (the list's tools: import, refresh, clear), `empty` (a hint instead
of rows).

**Filter** – `{active, groups: [{id, label, kind: choice|toggle, value, options: [{value, label, icon?}]}]}`. The
group ids are the keys of `params.filter`: `status` (true/false/null – running, in use, with image), `kind`
(`nixos`/`macos`), `tag` (recipes), `recipe` (images), `otherSystems` (recipes for other architectures, plan B114).
The core applies them; a UI keeps the chosen values per tab and asks with them.

**Field** – `{id, kind, label, value, help?, placeholder?, choices?, min?, max?, step?, enabled, reason?, error?,
param?, valueParam?, set?, ask?}`; `kind`: `toggle`, `text`, `number`, `choice`, `path` (chosen with `ask`), `info`
(shown, not edited; `value` is a text). On a form page (`newVm`, `macBase`, the new source in `settings`) the UI keeps
what the user set under **`params.values.<valueParam>`** of that page and asks the page again with it (validation:
`error`, `enabled`); the page's action already holds the values it was built with, and `param` says where a field's
value lands in that action's `params` (a UI that sends before the page came back may put it there itself). A field
without `valueParam` is not kept (an `info` field, a Settings field). In Settings `set: {method, params, param}` stores
a change at once: send `method` with `params` and the new value at `param` (`null` = back to the default).

**Icons** are names a UI maps to its symbols (SF Symbols, Nerd Font glyphs or plain characters in a TUI): `vm`,
`image`, `recipe`, `activity`, `nixos`, `macos`, `start-window`, `start-headless`, `stop`, `terminal`, `folder`,
`export`, `import`, `external`, `hide`, `upgrade`, `reset`, `trash`, `build`, `new-vm`, `create`, `cancel`, `install`,
`refresh`, `download`, `pause`, `apply`, `clean`, `guide`, `log`, `doctor`, `settings`, `key`, `quit`, `up`, `add`,
`clipboard`, `sound`, `microphone`, `camera`, `ephemeral`, `network-isolated`, `network-none`, `upgradable`,
`outdated`, `lab`, `arch`, `official`, `lock`, `lock-warning`, `github`, `url`, `warning`, `ok`, `filter`,
`finished`, `failed`, `cancelled`. An unknown name is drawn as nothing (or its `label`).

### 10.4 What a UI does itself (`ui.*`)

| Method | Params | |
|---|---|---|
| `ui.show` | `page, params` | open a page one level deeper (`build {recipe}`, `newVm {image}`, `doctor`, `macBase`, `settings`) – or `guide {file, title, url, path?}`: a guide of the docs, not a core page; `url` its page on omaflake.dev (`https://omaflake.dev/<permalink>/`), `path` the file omaflake.app bundles (macOS, only when the core runs from the app); a UI shows `path` itself or opens `url` |
| `ui.back` | – | leave the page |
| `ui.step` | `page, params` | get that page (only `import`) and run its `action` at once – asking its `confirm` first – or show its `reason`; with an `ask` of `multiple`, once per chosen value |
| `ui.filter` | `page, filter` | set the list's filter (`null`: reset) and ask for the page again |
| `ui.field` | `page, params, field, valueParam, value?, also?` | set `values.<valueParam>` of the form page `page` with `params` (field `field`; `ask` asks the value; `value: null` = back to the page's default) and ask that page again; `also: [{valueParam, value}]` sets more values of the page at once (plan B147) |
| `ui.reveal` | `path` | show in the file manager (a TUI prints the path) |
| `ui.open` | `path` or `url` | open with the system's default app (a log file, the data folder, a release page) |
| `ui.terminal` | `line` | run a command line in a terminal (the Doctor's install commands) |
| `ui.quit` | – | quit the interface (VMs keep running; `confirm` lists the running ones) |

### 10.5 The pages

Rows carry their details (what an expanded row shows) and the API object they come from (`vm`, `image`, `recipe`,
`job`, `version`, `source`, `prerequisite`) for anything else a UI wants to show. The examples are cut short; the whole
models are in `testdata/api/views/*.json`.

Every page has a `title`: the window's for `home` ("omaflake", data), the list's for the tabs, the header of every page
one level deeper (`doctor`, `settings`, `newVm`, `build`, `macBase`), the name for `vm` and `image`, the dialog's for
`import`. A UI draws it as it is and keeps no table of page titles.

**Host wording.** Texts that name the Mac, Finder, the Keychain or the menu window come in the variant for the host
the core runs on (`initialize`'s `host`): on macOS the app's English source strings as before, elsewhere host-neutral
keys – "Show Data Folder in Finder" / "Show Data Folder", "Show in Finder" / "Show in File Manager", "Share a Mac
folder with the VM" / "Share a folder with the VM", "Like this Mac – %@" / "Like this computer – %@", "Not for this
Mac – …" / "Not for this computer – …", "… in the Keychain" / "… in the keyring", the Doctor's intro, the notification
help, the data folder's note, the sources' footer. Both are keys; translations have both.

**home** – `title`, `actions` (doctor with `badge`, credentials (icon `key`, plan B137), locales (icon `globe`, plan B141), profiles (icon `gauge`, plan B142), settings, dataFolder, quit – with `shortLabel`s for a narrow header), `tabs: [{id, label, icon, badge?}]` (vms, images, recipes, modules – icon `module`, plan B142 –, looks – icon `look`, a warning dot without any look, plan B141 – and activity) with
`badge: {kind: count|working|failed|dot, count?, tone, help}` (running VMs; a build runs or failed; a recipe changed
since its image), `firstTab` (`recipes` while there is no VM – a UI opening for the first time starts there).

**vms** – list head (filter `status`: Running/Stopped, `kind`; tools: import, run from there), `rows`, `unreadable`.
A row (sorted by name):

```json
{"name": "research", "lamp": {"state": "on", "help": {"key": "Running", …}},
 "kind": {"id": "kind", "icon": "nixos", "help": {"key": "NixOS guest", …}, "tone": "normal"},
 "badges": [{"id": "external", "icon": "external", …}, {"id": "clipboard", …}, {"id": "sound", …}, {"id": "upgradable", "tone": "warning", …}],
 "subtitle": {"text": "dev (velvet) · 4 CPU · 8 GB · SSH 2201"},
 "actions": [{"id": "showShare", "method": "vms.share", "then": "reveal", …},
             {"id": "terminal", "method": "vms.ssh", "then": "terminal", …},
             {"id": "stop", "method": "vms.stop", "destructive": true, …}],
 "detail": {"switches": [{"id": "clipboard", "method": "vms.set", "on": true, …}, …],
            "switchesNote": {"key": "Changes take effect at the next start.", …},
            "info": [{"label": {"key": "Image", …}, "value": {"text": "omaflake.nixos.dev@cedar-mango-lift"}, "note": {"key": "newer image available", …}, "tone": "warning", …}, …],
            "actions": [{"id": "export", …}, {"id": "forget", …}, {"id": "upgrade", …}, {"id": "reset", …}, {"id": "remove", …}]},
 "vm": {…}}
```

Row actions: busy – none (the lamp works); running – `showShare` (when it shares), `terminal`, `stop`; a start blocker
– the Doctor's warning (`ui.show doctor`, help "Open the Doctor first: …"); else `startHeadless` (`display: none`) and
`startWindow` (`display: auto`), disabled with the macOS limit's reason. Details: switches clipboard, sound,
microphone (turning it on turns sound on), camera (not for macOS guests); info Image, omaflake ID (as short as
`idLengths.vm`), Recipe, Resources, Network, SSH (the command line, or "address while running"), Share folder,
Ephemeral, Created, Runs from, Agent, Guest (load, memory, disk, addresses, users); actions export (asks a folder),
forget (runs from elsewhere), upgrade (to `latestImage`, with its question), reset, remove – all greyed out while it
runs ("Shut the VM down first") or is busy.

**images** – list head (filter `status`: In use/Not in use, `kind`, `recipe`; tool: import), `jobs` (builds that wait,
run, failed or were cancelled: `{id, name, kind, state, title, count?, fraction?, actions: [cancel | dismiss], open,
job}` – `open` is the row's `primary`: a click opens the build page), `bases?` (on a Mac only, plan B138: `{title,
rows: [{name, lamp, subtitle ("macOS 26A434 · Work (alex) · date · size"), badges (newest, no-credential), actions:
[assign (only without a credential: `baseCredential`), reveal, remove]}], empty? (with create as its action), job?
(the base being created – IPSW download or install – as a `jobs` row whose `open` is the macOS base page), create
(the macOS base page; drawn only without `empty`, disabled while `job`), footer}`), `rows` (macOS before NixOS, by recipe name,
newest first: lamp green when a VM uses it, badge `outdated`, subtitle "recipe dev · <date> · 1 VM", actions reveal and
newVm (primary) – or the Doctor's warning (primary) –, details: info omaflake ID,
Recipe, Built, Recipe state, Status, Look (with a note when the settings ask for another look), Used by, Size, Source,
Disk image/System; actions export, remove – disabled while
VMs use it), `unreadable`. **image** – `{title, row}` with the size.

**looks** (plan B141) – list head (tool: refresh – `looks.list {fetch: true}`; `empty` without any look: "Install
Recommended"), `catalogsLoaded`, `loading`, `loadingText?`, `rows` (the looks here: `{id, name, lamp (on while images
are built with it), subtitle ("omaflake/catppuccin · dark · 2 images"), badges (default, official, update, folder),
swatches ("#RRGGBB" for a strip: background, surface, text, accent, urgent and six terminal colours; empty for a plain
look; absent on the modules page), actions (Make Default unless it is the default), detail: {info (About, Source, Used by), actions: [default,
update?, remove (disabled while images use it or for a local folder's)]}}`), `offeredTitle?`, `offered` (the same rows
with Install), `unreadable`, `footer`. The UI runs `refresh` once when the tab opens and the catalogs were not looked
into.

**recipes** – list head (filter `status`: With image/Without image, `kind`, `tag`, `otherSystems`; tools: refresh –
`recipes.available`, busy while it runs –, import and "New Recipe …" – `ui.show recipeStudio`), `catalogsLoaded` (else the UI runs `refresh` when the tab
opens, as the app did; the views never fetch), `loading` with `loadingText` ("Looking into the recipe sources …", next to
a spinner), `updates?` (`{title, all?, rows: [{id, name, kind, source,
action}], note}`; "Add Version"/"Add All" when older versions are kept), `suggestion?` (no recipe yet: "Start with the
official recipes?" with `recipes.installSuggested`), `rows` (one per line, named by its newest version; built ones
first, then by kind and name: lamp – working while it builds, off not built, attention changed since its image, on –,
`secondary` (author, distinction), badges lab and arch, `tags`, actions reveal and build – `ui.show build`; disabled
for another architecture, and disabled with `badge` (the Doctor's sign, reason "Open the Doctor first: …") while the
Doctor reports what building its kind needs (plan B145) –, details: description, info, `versions` (`{id, words,
state, actions: [export, remove], version}`; remove disabled for a folder's version or one images are built from) and
actions update, edit (one of yours) or copy ("Copy to My Recipes …", `ui.show recipeStudio {id}`), removeAll; since
plan B142 also the info lines modules, look, key bindings, needs at least, `notes` (hints: what is missing with
"Install Missing Parts" – `recipes.installParts` –, its look when not installed, warnings, unfree software) and `keys`
(info lines: an action's title, its key, its module and mode); the lamp is attention while parts are missing; build is
the row's primary), `offeredTitle` ("More recipes in your sources", while there are
offers) and `offered` (not installed: lamp off or working, badges official/lab/arch, install (primary), details with
the "not reviewed" warning for recipes not from omaflake/catalog), `unreadable`, `errors` (sources that failed),
`labHidden` with `labHint` (a hint with the way to Settings).

**newVm** – `{image, recipe, kind, subtitle, description?, lab, blockers?, sections: [{title?, fields}], warnings,
valid, actions: [cancel, create]}`. Params `{image, values?: {name, profile, cpus, memoryGB, diskGB, network, share,
sharePath, ephemeral, audio, microphone, clipboard, camera, start}}`; with a profile (plan B142: the chosen one, else
the default; `""` = custom values) CPU cores and RAM are the profile's and those fields disabled; a field `profile`
(the profiles and "Custom values") comes first in the resources while there are profiles, and less than the recipe
needs adds a warning. Missing values are the recipe's (sound and clipboard on,
microphone and camera off where it says nothing), the name the next free one. Fields: name (errors: "A name is
needed", the name rule, "“%@” is here already"), ephemeral, CPU cores (1–10), RAM (1–48 GB), disk (10–500 GB step 10;
`info` "as the image" for macOS), sound, microphone (needs sound), clipboard, camera (NixOS), network (no `isolated`
for macOS), share, share folder (`path`, placeholder the default `share/<name>`), start afterwards. `warnings`: the
macOS limit ("… – the VM is created but not started."). `create` is `vms.create` with `{name, image, settings,
sharePath?, start}` and `then: back` (primary), disabled while a value is wrong. Every editable field has `valueParam`
= the key above (`cpus` → `params.values.cpus`, its `param` `settings.cpus`).

```json
{"id": "cpus", "kind": "number", "label": {"key": "CPU cores: %lld", "text": "CPU cores: 4", "args": [4]},
 "value": 4, "min": 1, "max": 10, "step": 1, "enabled": true, "param": "settings.cpus", "valueParam": "cpus"}
```

**build** – `{recipe, name, kind, badges, description?, state: idle|queued|checking|running|finished|failed|cancelled,
actions, fields?, blockers?, macBase?, notes?, status?, progress?, outcome?, newest?, log?, job?}`. `notes` (plan
B142): the recipe's (as its row's), and a locale the recipe is not made for (a warning); the look is preset with the
recipe's suggestion when it is installed. `build` is disabled while parts are missing ("Install what the recipe is made
of first"). `fields` (while no build
runs, plan B137): `credential` (the credentials), `look` (the looks) and `locale` ("This computer's values" as `""` without
any locale, else the locales), preset with the defaults; their values (`values.credential`, `values.look`,
`values.locale`) go into the build action's params. A macOS
recipe's `credential` is the newest base's, disabled (plan B139). Actions: `build` ("Build", or "Add to Queue" while
another build of the guest kind runs; never asks a password; disabled for another architecture, Doctor blockers, no
credential, a missing macOS base or one without a credential – then `macBase` names it with "Assign Credential …") – or `cancel` ("Cancel"/"Remove from Queue") while it waits or
runs. `progress: {step, title, count?, fraction, started, lastOutput, quietWarning, quietHelp}` – the UI counts the
running time and the quiet time itself (orange after `quietWarning` s). `outcome: {state, icon, tone, title, detail?,
action?}` ("Image ready" with "New VM …", "Build failed" with the error, "Build cancelled"). `log: {job, from, lines,
note?, file?, full?}` – the job's id and where its log is: `jobs.log {id: job, from}` reads what is kept (the newest
5000 lines, `from` the first of them), then `job.log` brings new lines; `lines` the count so far, `full` opens the
whole file. The build is the page's primary action (Cancel is not).

**doctor** – `{versions, update, intro, checking, checked?, actions: [check], sections: [{id: requirements|nixos|macos,
title, items}], issues}`. `versions`: one line "omaflake" – "App <clientVersion> · Core <version>" (the UI's version
from `initialize`, plan B138) – shown with `update` as one block. `update: {label, text, tone, busy,
actions}` from the last `updates.check` (Download …, Check Now). An item: `{item, status, title, detail?, hint?,
expandable, expanded, busy, task?, download?, actions, prerequisite}` – `detail` (paths and versions as found, or "%@
free, about %@ needed"), `hint` while not ok, `expanded` the sub-line's first state (open while there is a problem or
work runs). Actions as the app had them: Install … (Nix: `builder.installNix` with its question – Mount … (`builder.mountNix`) instead when Nix is installed
but its volume is not at /nix, plan B145; nix-darwin:
`builder.setup`; others: `ui.terminal` with the install command), Clean Up … and Apply Settings … (`builder.apply`,
`force` and "Replace and Apply" when this Mac's nix-darwin system is not omaflake's), Guide, Set Path … (asks a file
or folder, `doctor.setPath`), Download / Continue Download / Pause (the IPSW, with `download: {done, of, text}`),
Create … / Show … (`ui.show macBase`), the `omaf` command's Install … / Uninstall … (`doctor.installCli` /
`doctor.uninstallCli`, their questions name the link, its target and the administrator on macOS; Install disabled while
a file that is not omaflake's is in the way, the hint then says so – or how to add the link's folder to the PATH).
Linux builder tasks run one at a time: the others say which runs.

**macBase** – `{title, intro, base?, ipsw: {label, text, download?, actions}, fields: [credential], installing,
actions: [newCredential?, create], log?}`; params `{values?: {credential, ipsw}}` (defaults: the default credential,
Apple's IPSW). The IPSW is shown once, in `ipsw`: its actions "Choose IPSW …" and "Use Apple's" are `ui.field` on
`values.ipsw` of this page. `create` (primary) is `macos.installBase {credential, ipsw?}` – no password is asked: the
credential's (plan B139); without a credential it is disabled and `newCredential` opens the form. `log`: the latest base creation's job, as the build page's (`{job, from, lines, …}` – the job's real id).

**settings** – `{sources, general, guest, dataFolder}`. `sources: {title, rows, add, addAction, footer}` – rows `{id,
name, icon (folder|github|url), badges (official, private with or without token), subtitle, actions: [up, enabled
(a switch), remove], source}`; `add.fields`: input (with "Folder …"), private (only for a GitHub repository – the core
decides from `params.values.input`); `addAction` (primary) is `sources.add {input, private}` and asks the token for a
private one; the two fields keep their values under `values.input` and `values.private`. `general.fields`: notifications, checkForUpdates, keepRecipeVersions, developerMode (toggles with `set`).
`guest` ("This Mac" / "This computer", plan B141) – `fields`: hostKeyboard (this computer's own keyboard; "As the Mac
says – …" / "As the system says – …" as `null`, then the keyboards). The defaults for new images are set in their own
pages: credentials, looks and locales; for new VMs: profiles. `dataFolder: {title, location, isDefault, moving,
actions: [default, move], note}` – both disabled while a VM runs. Launch at login and the UI's language are each UI's
own.

**credentials** (plan B137, B138) – `{title, intro, rows, add, footer}`: rows as the other lists `{id, name, lamp (on
while images use it), subtitle ("user · n images"), badges (default), usedBy (the images' names), actions:
[copyPassword], detail: {info (user, full name, e-mail, password, used by), actions: [edit (`ui.show credential {id}`), password
("Change Password …", asks it twice), default (`settings.set {defaultCredential}`), remove (destructive, confirm;
disabled while used)]}}`; `add` ("New Credential …", primary) opens `credential`; `footer` says where passwords are
kept.

**credential** (params `id?`) – `{title, id?, intro, fields: [name, user, fullName, email], locked?, actions: [cancel,
save | create]}`: each field's `help` says what it is for; while images use the credential `locked` says why and every
field but `name` is disabled with that `reason`. `create` (`credentials.add`) asks the password twice; both go back
(`then: back`).

**locales** (plan B141) – the shape of **credentials** (`{title, intro, rows, add, footer}`; UIs draw it with the same
view): rows `{id, name, lamp (on while images are built with it), subtitle ("de_DE · de + ru · Europe/Berlin · n
images"), badges (default), usedBy, actions: [default], detail: {info (language and region, time zone, keyboards, used
by), actions: [edit (`ui.show locale {id}`), default (`locales.setDefault`), remove (destructive, confirm; disabled while
used)]}}`; `add` ("New Locale …") opens `locale`; `intro` names this computer's keyboard.

**locale** (params `id?`, `values`) – the shape of **credential**: `fields: [name, locale, timeZone, keyboard,
keyboard2, keyboard3]` (choices; the second and third with "none" as `""`; a new one preset with this computer's
values), `locked?` while images are built with it (only `name` then), `actions: [cancel, save (`locales.update`) |
create (`locales.add`)]` – disabled with the field's error (no name, a name taken, a keyboard twice).

**profiles** (plan B142) – the shape of **credentials**: rows `{id, name, lamp (on: the default), subtitle ("2 CPU · 4
GB RAM"), badges (default), actions: [default], detail: {info (CPU cores, RAM), actions: [edit (`ui.show profile {id}`),
default (`profiles.setDefault`), remove (destructive, confirm)]}}`; `add` ("New Profile …") opens `profile`.

**profile** (params `id?`, `values`) – the shape of **credential**: `fields: [name, cpus (1–64), memoryGB (1–512)]`
(a new one 4 CPU, 8 GB), `actions: [cancel, save (`profiles.update`) | create (`profiles.add`)]`.

**modules** (plan B142) – the shape of **looks** (UIs draw it with the same view; rows without `swatches`): the modules
(NixOS, then macOS) and key binding sets here, then `offered`; subtitle "omaflake/hyprland · 1.0.0 · NixOS module · 1
recipe"; badges official, update, unfree; details: about, provides, needs, works with, keys, source, used by; actions
update (`modules.install`), remove (`modules.remove`, confirm naming the recipes made of it; disabled for a local
folder's). Offered rows install with `modules.install {id, source}`. List head: refresh – `modules.list {fetch: true}`;
installAll – `modules.installAll {items}` (the offered rows; confirm when one is not official). **looks** and
**recipes** have the same `installAll` (`looks.installAll`, `recipes.installAll`; the recipes as filtered).

**recipeStudio** (params `id?` – a recipe to edit when it is one of yours (My Recipes), else to copy –, `values`; plan
B147, before it the recipe editor of B142): `{title, id?, intro, about, catalog, recipe, dress, vm, titles, actions}`.
`about`: fields name, platform (nixos | macos), description. `catalog`: `{title, count, search (a text field,
`values.search`), filters (capability, tag, author – choices, "" = any), rows, empty?}` – the platform's modules here
and offered that are not in the recipe and match. `recipe`: `{title, rows, hints, empty?, keys?}` – its modules, what
is wrong (a module that needs a capability none provides, with the action that adds the one to add; exclusive
capabilities twice; keys that overlap or break the key rule; modules still to install), `keys` the keys page on the
draft (`keys {draft: {platform, modules, keymaps, name}}`). A row: `{reference, title, description, subtitle, badges,
tags, action}` – the action is `ui.field` on `values.modules` with the whole new list (add or remove). `dress`: look,
keymap1, keymap2; `vm`: cpus, memoryGB, diskGB, display, network, share, ephemeral. Actions: cancel, delete (one of
yours: `recipes.delete {base}`, confirm), save (`recipes.save`, `then: back`; disabled without a valid name or a
module). It is a tab of the workshop too (a new recipe).

**import** – `{title, path, name, kind, valid, reason?, action?, inspection}`: `action` is `packages.import` (a VM whose name
is taken: the question with `input: {param: "name", default: "<name>-2"}`; the same image: "Replace" with `replace`)
or, with `external`, `vms.useExternal` (on a network volume: "Run the VM over the network?").

### 10.6 Activity

`activity` – list head (tool: clear – `activity.clear`), `notifications` (the user wants system notifications,
Settings) and `entries` (newest first, at most 100):

```json
{"id": 2, "time": "2026-10-23T09:42:00Z", "level": "info", "text": {"key": "%@ is ready", "text": "research is ready", "args": ["research"]},
 "notification": {"title": {"key": "VM ready", …}, "body": {"key": "%@ is running and answers SSH.", …}}}
```

The core words what the app's `AppModel.message` said: an action done (`research deleted`, `research created (SSH port
2201)`, `Image … deleted`, `dev: 2 versions removed`, a source added with its note, …) or refused (`research: <error>`), a job that ended (image
ready, build failed or cancelled, downloads, the base, Linux builder tasks, installs, imports, exports, the data
folder), a VM that ended on its own or is ready, an update, and every `message`. `notification` is what a system
notification says, where the app posted one – the UI posts it when `notifications` is true and its window is not in
front. The list is the server's: with the service (§1) every connection sees the same activity, and it lives as long as
the service runs.

**Seen or not is the UI's** (the model has no unseen badge – every connection would need its own). An entry's `id` is
stable – it never changes and is never reused while the server runs, `activity.clear` included – and rises with each
new entry. A UI marks entries seen by remembering the highest `id` it has shown on its activity tab (or posted as a
notification); entries with a higher `id` are unseen (the tab's dot, the footer line, a TUI's status line). The ids
start again at 1 with a new server: when the newest `id` in the list is lower than the one remembered, the UI forgets
its mark.
