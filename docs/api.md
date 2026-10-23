---
title: Core API
nav_order: 5
permalink: /api/
---

# omaflake core API (v1)

omaflake's logic lives in one **Rust core** (`crates/omaflake-core`). Every user interface talks to it through the
interface described here: the macOS app (SwiftUI; menu bar, window or both – plan B119), later the Linux app (native Rust
with iced, plan B119, issue #4), a terminal interface (`omaf ui`, macOS and Linux, plan B121), scripts and tests. The CLI `omaf` is built on the same core. No UI holds logic of its own beyond presentation: what a VM, an
image or a recipe is, what may be done with it and what happens then is decided in the core.

```
SwiftUI app (macOS) ─┐                          ┌─ omaf-runner (Swift, Virtualization.framework, macOS guests)
Linux UI (later)  ───┼─ JSON-RPC 2.0 ─ omaf serve ─ omaflake-core (Rust) ─┼─ qemu-system-*, qemu-img, nix, ssh
scripts, tests    ───┘   (stdio, NDJSON)        └─ ~/.omaflake (data folder, spec §5)
CLI omaf ───────────────────────────────────────── omaflake-core (linked)
```

The core reads every state from the data folder (spec §5): VMs started by the CLI show up in a UI and the other way
round, and a UI that quits leaves running VMs alone.

## 1. Transport

- A UI starts **`omaf serve`** as a child process and talks to it over **stdin/stdout**: one JSON object per line
  (UTF-8, `\n`-terminated, no newline inside an object – NDJSON). stderr carries the server's own diagnostics only.
- Messages follow **JSON-RPC 2.0**: requests (`id`, `method`, `params`), responses (`result` or `error`) and
  notifications (no `id`) from the server – the events (§6). The client sends no notifications.
- Requests run **concurrently**: the server answers each one when it is done, so responses may arrive in another
  order than the requests (match them by `id`). A slow request (stopping a VM) never blocks a fast one (listing).
- The server ends when stdin closes or after `shutdown`. Long work it still runs (jobs, §5) is cancelled then;
  VMs keep running (they live in their own session).
- `omaf call <method> [<params as JSON>]` runs a single request without a session and prints the result – for
  scripts and tests. It initializes itself and sends no events; a method that starts a job waits for it and prints
  the ended job (its `state` tells whether it finished).
- The same dispatcher is a Rust API (`omaflake_core::api`), so a Rust UI (the Linux app, iced) can link it in-process with exactly
  these methods and JSON shapes: `Server::new(paths, notify)` (`notify: Fn(method, params)` receives the events, from
  the server's threads – it must not block on the server), then `server.handle(request)` (a JSON-RPC object in, the
  response out) or `server.call(method, params)` (the result or an `RpcError`). `serve_stdio()` and
  `call_once(method, params)` are what `omaf serve` and `omaf call` run.
- JSON-RPC batches (arrays) are not supported (`-32600`).

## 2. Session

The first request is `initialize`; anything before it is refused with `-32002`.

```json
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"client":"omaflake-macos","clientVersion":"0.3.0","apiVersion":1,"language":"de","updateFeed":"https://…/feed.json"}}
← {"jsonrpc":"2.0","id":1,"result":{"apiVersion":1,"omafVersion":"0.3.0","host":{"system":"aarch64-darwin","os":"macOS 27.0.1"},"dataFolder":"/Users/…/.omaflake","configFile":"/Users/…/.omaflake/config.json"}}
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
- `shutdown` → `null`, then the server exits.

## 3. Errors

Standard JSON-RPC codes for protocol errors (`-32700` parse error, `-32600` invalid request, `-32601` unknown method,
`-32602` invalid params, `-32002` not initialized). Everything the core refuses or fails at is `-32000` with a
message for people and a stable code for programs:

```json
{"code":-32000,"message":"running – stop it first","data":{"kind":"vm-running"}}
```

`data.kind` (kebab-case) lets a UI react or show its own translated text; the `message` is English. Errors without a
kind leave `data` out. Some methods add fields to `data` next to `kind`: `networkVolume: true` (`vms.useExternal`),
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
a recipe version that is gone has nothing to compare with). `sizeBytes` (allocated blocks, so it walks the image's
files) is only in `images.get`, not in the lists.

### 4.3 Recipe

The recipe's `recipe.toml` values (`kind`, `systems`, `cpus`, `memoryGB`, `diskGB`, `network`, `share`, `ephemeral`,
`user`, `name`, `author`, `audio`, `microphone`, `clipboard`, `camera`, `description` – in the session's language when
the recipe has it –, `lab`, `display`, `tags`, `license`, `homepage`) plus:
`id`, `base`, `qualified` (`omaflake/nixos/dev`), `displayName`, `label`, `runsHere`, `source` (source id),
`line`, `distinction` (what tells it apart from a recipe of the same author/kind/name, plan B110), `folder`,
`installed` (`{source, version, installed}` for downloaded ones), `image` (its line's current image), `outdated`
(the recipe changed since that image was built, plan B73/B82), `versions` (its line's versions, newest first).
`lab` is a boolean here (false when unset).

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
  `description` in the session's language, `tags`, `license`, `lab`, `systems`, `requires`, `homepage`, `version`,
  `files: [{path, size, sha256}]`) plus `source` and `runsHere`.
- **Prerequisite** – `item` (`host`, `nix`, `flakes`, `nixDarwin`, `linuxBuilder`, `qemu`, `flake`, `sshKey`, `git`,
  `builderRunning`, `flakeEval`, `nixSpace`, `ipsw`, `macBase`, `macSpace`), `status` (`ok` | `limited` | `missing`),
  `title` (English; UIs translate by `item`), `required`, `detail`, `hint`, `install` (a command), `settablePath`,
  `freeBytes`, `neededBytes`, `baseUser`, `expectedUser`.
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
  "kind": "build",          // build | macos-install | ipsw-download | builder-install-nix | builder-setup
                            // | builder-apply | builder-clean-up | import | export | data-move | recipe-install
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
  "error": "…"              // when failed
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
| `builder-install-nix`, `builder-setup`, `builder-apply`, `builder-clean-up` | `{message}` – what it did, in English |

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
- Jobs live as long as the server. A UI that restarts its server loses them (their log files stay).

## 6. Events

`events.subscribe {"topics": ["vms", "images", "recipes", "jobs", "doctor", "messages"]}`. The server watches the data
folder (every 2 s while subscribed, and right after its own actions) and sends a topic's **whole list** when it
changed – a UI replaces what it shows. A newly subscribed topic gets its list at once (`jobs`: a `jobs.changed` per
job); `events.subscribe` replaces the subscription, unknown topics are left out, `[]` stops the events. Subscribing to
`doctor` runs the checks once. Update checks run only when a client asks (`updates.check`).

| Event | Params | When |
|---|---|---|
| `vms.changed` | `{vms: [Vm], unreadable: [Unreadable]}` | a VM came, went, started, stopped, changed |
| `vm.ended` | `{name, requested}` | a running VM is off; `requested` = it was stopped through this server |
| `vm.ready` | `{name, via}` | after a start through this server: the agent reported in (`agent`) or SSH answers (`ssh`) |
| `images.changed` | `{images: [Image], unreadable: [Unreadable]}` | images built, imported, removed |
| `recipes.changed` | `{recipes: [Recipe], …}` (as `recipes.list`) | recipes installed or removed, a source or the flake changed on disk, freshness re-evaluated |
| `jobs.changed` | `{job: Job}` | a job's state or progress changed |
| `job.log` | `{id, lines: [string]}` | new log lines of a job |
| `doctor.changed` | as `doctor.check` | the checks ran (after actions, before builds) |
| `message` | `{text, level}` | something the user should read that a client cannot tell from the other events or its own answers (`info` | `warning` | `error`; English): older recipe versions pruned by `settings.set`, a recipe list that cannot be read, a macOS VM created but not started (`vms.create` beyond the limit) |

A client words the rest itself, in its language: the end of a job (`jobs.changed` with an ended `state`, its `result`
or `error`), `vm.ended` and `vm.ready` come without a `message`. OPEN: a `message` has no stable kind a UI could translate by
(as errors have, §3) – its English text is shown as it is.

## 7. Methods

Params and results in short; the types are those of §4.

### Session and settings

| Method | Params | Result |
|---|---|---|
| `initialize` | `client, clientVersion, apiVersion, language?, updateFeed?` | `apiVersion, omafVersion, host: {system, os}, dataFolder, configFile` |
| `events.subscribe` | `topics` | `null` |
| `shutdown` | – | `null` |
| `settings.get` | – | `{settings, guestDefaults, choices: {timeZones, locales, keyboards: [{code, locale}]}, isDefaultDataFolder}` – `settings`: config.json's values with their defaults applied (`developerMode, checkForUpdates, notifications, keepRecipeVersions, guest: {user?, fullName?, email?, timeZone?, locale?, keyboard?}, flake?, qemu?, nix?, runner?, flakeInUse?, dataFolder`); `guestDefaults`: this computer's values the guest overrides fall back to |
| `settings.set` | any of `developerMode, checkForUpdates, notifications, keepRecipeVersions, guest, flake, qemu, nix, runner` (`null` or `""` = back to the default; keys not given stay) | `settings` – turning `keepRecipeVersions` off prunes older recipe versions |
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
| `images.build` | `recipe, password?` | Job `build` (macOS recipes need the guest user's password once – kept in memory only until the build VM uses it; without it `-32602`) |
| `images.remove` | `name` | `null` – refused while VMs use it (`in-use`) |

### Recipes and sources

| Method | Params | Result |
|---|---|---|
| `recipes.list` | `all?` | `{recipes: [Recipe], lines: {line: [Version]}, idLengths: {recipe, image, vm}, qemuGL, unreadable, labHidden}` – lab recipes only in developer mode or with `all` |
| `recipes.available` | – | fetches the enabled remote sources' catalogs: `{offered: [CatalogRecipe], updates: [{id, recipe: CatalogRecipe}], suggested: [CatalogRecipe], errors: {sourceId: message}}` |
| `recipes.install` | `id, source` | Job `recipe-install` (with what it requires) |
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
| `packages.inspect` | `path, expect?: "vm"|"image"|"recipe", external?` | `{kind, valid, reason?, conflict?: {vmName, suggestedName}|{image}, networkVolume}` – what an open panel checks before it closes, and what an import would ask about (`suggestedName`: the first free name to import a VM under) |
| `packages.import` | `path, name?, replace?` | Job `import` → `result: {kind, names, notes}` (§5) |
| `packages.export` | `kind: "vm"|"image"|"recipe", name, folder, withImage?` | Job `export` → `result: {path}` (`withImage` unset: as `Transfer::carries_image` decides) |

### Doctor, macOS base, Linux builder

| Method | Params | Result |
|---|---|---|
| `doctor.check` | – | `{prerequisites, nixosPreflight, macosPreflight, latestRelease?, buildBlockers: {nixos, macos}, startBlockers: {nixos, macos}, issues, requirementsMissing, hasMacBase, macBaseUser?, builderSystemIsOmaflakes, checked}` – blockers are Prerequisites; `builderSystemIsOmaflakes`: this Mac's nix-darwin system comes from omaflake's image flake (else `builder.apply` needs `force`); a build runs these checks right before it starts, `vms.start` refuses a start blocker of the last check (`prerequisite`) |
| `doctor.setPath` | `item: "nix"|"qemu"|"flake", path` | as `doctor.check` |
| `macos.downloadIpsw` | – | Job `ipsw-download` → `result: {file, version}` (cancel pauses; it continues from there) |
| `macos.installBase` | `user, fullName, password, ipsw?` | Job `macos-install` → `result: {base, existing, ready}` |
| `builder.installNix` / `builder.setup` / `builder.apply` (`force?`) / `builder.cleanUp` | – | Job → `result: {message}` – each asks for an administrator through the system's own dialog; one at a time (`in-use`); `builder.apply` on a nix-darwin system that is not omaflake's needs `force` (error data `replacesConfiguration`); a dismissed dialog cancels the job |

### Jobs

| Method | Params | Result |
|---|---|---|
| `jobs.list` | – | `[Job]` |
| `jobs.log` | `id, from?` | `{lines, next}` |
| `jobs.cancel` | `id` | `Job` |
| `jobs.dismiss` | `id` | `null` – an ended job leaves the list (`in-use` while it runs) |

## 8. Secrets

Passwords (`images.build`, `macos.installBase`) and tokens (`sources.add`) travel only over the pipe between the UI
and its own `omaf serve`. The server never logs params of these methods, keeps a password only until it is used and
stores tokens only in the system keychain (macOS Keychain; Linux: OPEN).

## 9. Status

API v1 is new with the Rust core (plan B117). The method tables are the contract; the Rust types in
`crates/omaflake-core/src/api/types.rs` are its implementation, and their JSON Schema is OPEN (to be generated from
those types and published under `schema/api/`).

**Contract tests.** `crates/omaflake-core/testdata/api/*.json` holds a representative value of every result, event
and error (and the params `initialize` and `vms.create` get): the Rust tests (`api::contract`) write them from the
types (`OMAF_UPDATE_GOLDEN=1 cargo test -p omaflake-core api::contract` after a deliberate change, otherwise they
compare), and the Swift tests (`APIContractTests`) decode each with the app's types – strictly, a required field is
never defaulted. `CoreEndToEndTests` starts the real `omaf serve` (`OMAF_BIN=target/debug/omaf swift test`) on a
fresh data folder and decodes every read method and the events.
