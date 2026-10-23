---
title: Core API
nav_order: 5
permalink: /api/
---

# omaflake core API (v1)

omaflake's logic lives in one **Rust core** (`crates/omaflake-core`). Every user interface talks to it through the
interface described here: the macOS menu bar app (Swift), later the Linux UIs (plan: Tauri, issue #4), scripts and
tests. The CLI `omaf` is built on the same core. No UI holds logic of its own beyond presentation: what a VM, an
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
  scripts and tests.
- The same dispatcher is a Rust API (`omaflake_core::api`), so a Rust UI (Tauri) can link it in-process with exactly
  these methods and JSON shapes.

## 2. Session

The first request is `initialize`; anything before it is refused with `-32002`.

```json
→ {"jsonrpc":"2.0","id":1,"method":"initialize","params":{"client":"omaflake-macos","clientVersion":"0.3.0","apiVersion":1,"language":"de"}}
← {"jsonrpc":"2.0","id":1,"result":{"apiVersion":1,"omafVersion":"0.3.0","host":{"system":"aarch64-darwin","os":"macOS 27.0.1"},"dataFolder":"/Users/…/.omaflake","configFile":"/Users/…/.omaflake/config.json"}}
```

- `apiVersion` is an integer. Within a version, changes are **additive only** (new methods, new optional params, new
  result fields, new events, new enum values that clients show as "unknown"); clients ignore what they do not know.
  Anything else is a new version; the server then serves the older one for at least one minor release.
- `language` (optional): the UI's language (`"en"`, `"de"`, …) – recipe descriptions in that language where a recipe
  has one.
- `events.subscribe` `{topics: [...]}` starts the events (§6); without it the server sends none.
- `shutdown` → `null`, then the server exits.

## 3. Errors

Standard JSON-RPC codes for protocol errors (`-32700` parse error, `-32600` invalid request, `-32601` unknown method,
`-32602` invalid params, `-32002` not initialized). Everything the core refuses or fails at is `-32000` with a
message for people and a stable code for programs:

```json
{"code":-32000,"message":"running – stop it first","data":{"kind":"vm-running"}}
```

`data.kind` (kebab-case) lets a UI react or show its own translated text; the `message` is English. Kinds are added
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
  "share": true, "sharePath": "/…",     // sharePath: the folder shared (the default share/<name> or the user's)
  "ephemeral": false,
  "sshPort": 2201,                      // NixOS
  "user": "alex",                       // guest user (SSH)
  "audio": true, "microphone": false, "clipboard": true, "camera": false,
  "display": "gpu",                     // none | window | gpu (how a plain start shows it)
  "created": "2026-10-20T08:00:00Z",
  "identity": "amber-lake-quiet",       // its omaflake ID words (spec §11.11)
  "omaflakeId": "vm:research:amber-lake-quiet",
  "label": "research",                  // as the UI shows it
  "address": "192.168.64.5",            // macOS guests while running: NAT address
  "agent": { … },                       // what the omaflake agent last reported (spec §11.9), while running
  "location": "/Volumes/…",             // the package's place when it runs from elsewhere (plan B108)
  "latestImage": "…",                   // the newest image of its recipe's line
  "upgradable": true
}
```

### 4.2 Image

`name`, `recipe`, `kind`, `system`, `revision`, `storePath`, `created`, `line`, `path`, `sizeBytes`, `label` (its
omaflake ID as short as it can be), `instances` (VM names using it), `current` (it is the newest of its line).

### 4.3 Recipe

The recipe's `recipe.toml` values (`kind`, `systems`, `cpus`, `memoryGB`, `diskGB`, `network`, `share`, `ephemeral`,
`user`, `name`, `author`, `audio`, `microphone`, `clipboard`, `camera`, `description` – in the session's language when
the recipe has it –, `lab`, `display`, `tags`, `license`, `homepage`) plus:
`id`, `base`, `qualified` (`omaflake/nixos/dev`), `displayName`, `label`, `runsHere`, `source` (source id),
`line`, `distinction` (what tells it apart from a recipe of the same author/kind/name, plan B110), `folder`,
`installed` (`{source, version, installed}` for downloaded ones), `image` (its line's current image), `outdated`
(the recipe changed since that image was built, plan B73/B82), `versions` (its line's versions, newest first).

`RecipeSettings` (what `vms.create` takes): the runtime values of a recipe – `kind`, `cpus`, `memoryGB`, `diskGB`,
`network`, `share`, `ephemeral`, `user`, `audio`, `microphone`, `clipboard`, `camera`, `display`.

### 4.4 Others

- **Source** – `id`, `kind` (`github` | `https` | `folder`), `location`, `branch`, `displayName`, `enabled`,
  `private`, `official`.
- **CatalogRecipe** – an entry of a source's `catalog.json` (spec §11.3) plus `source` and `runsHere`.
- **Prerequisite** – `item` (`host`, `nix`, `flakes`, `nixDarwin`, `linuxBuilder`, `qemu`, `flake`, `sshKey`, `git`,
  `builderRunning`, `flakeEval`, `nixSpace`, `ipsw`, `macBase`, `macSpace`), `status` (`ok` | `limited` | `missing`),
  `required`, `detail`, `hint`, `install` (a command), `settablePath`, `freeBytes`, `neededBytes`, `baseUser`,
  `expectedUser`.
- **MacOSRelease** – `version`, `build`, `url`, `bytes`.
- **Unreadable** – a file this omaflake cannot read (too old, too new, damaged; plan B114): `path`, `type`,
  `version`, `reason`.
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
  "started": "…", "finished": "…",
  "lastOutput": "…",
  "logLines": 412,          // lines so far (jobs.log reads them)
  "hiddenLines": 37,        // routine lines left out of the log (still in logFile)
  "logFile": "/…/logs/image-build-….log",
  "result": { … },          // when finished: e.g. {"image": "…", "removed": ["…"]} for a build
  "error": "…"              // when failed
}
```

- **Builds** queue: one build per guest kind at a time, NixOS and macOS side by side (plan B60). Right before it
  starts, a build runs the Doctor's checks for its guest kind and fails with the first blocker (`checking` state).
  A finished build prunes older images of its line (current + one before + those in use, plan B52).
- `jobs.cancel` takes a waiting job out of the queue or stops a running one (its subprocesses end, a macOS build VM
  is forced off, a half-built image is deleted).
- Jobs live as long as the server. A UI that restarts its server loses them (their log files stay).

## 6. Events

`events.subscribe {"topics": ["vms", "images", "recipes", "jobs", "doctor", "messages"]}`. The server watches the data
folder (every 2 s while subscribed) and sends a topic's **whole list** when it changed – a UI replaces what it shows.

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
| `message` | `{text, level}` | something the user should read in the activity list (`info` | `warning` | `error`) |

## 7. Methods

Params and results in short; the types are those of §4.

### Session and settings

| Method | Params | Result |
|---|---|---|
| `initialize` | `client, clientVersion, apiVersion, language?` | `apiVersion, omafVersion, host, dataFolder, configFile` |
| `events.subscribe` | `topics` | `null` |
| `shutdown` | – | `null` |
| `settings.get` | – | `{settings, guestDefaults, choices: {timeZones, locales, keyboards}, isDefaultDataFolder}` – settings as in config.json; `guestDefaults`: this computer's values the guest overrides fall back to |
| `settings.set` | any of `developerMode, checkForUpdates, notifications, keepRecipeVersions, guest, flake, qemu, nix, runner` (`null` = back to the default) | `settings` |
| `dataFolder.move` | `target` | Job `data-move` (all VMs must be stopped) |
| `updates.check` | – | `{state: "noFeed"|"upToDate"|"available"|"off", release?}` |

### VMs

| Method | Params | Result |
|---|---|---|
| `vms.list` | – | `{vms: [Vm], unreadable}` |
| `vms.get` | `name` | `Vm` |
| `vms.create` | `name, image, settings: RecipeSettings, sharePath?, start?` | `Vm` – from exactly that image (plan B100); `start` starts it after creating |
| `vms.start` | `name, display?: "auto"|"none"|"window"|"gpu", gl?: "on"|"es"` | `Vm` – `auto` (default): the UI's start – a window, with 3D where QEMU can; refused beyond the macOS limit (`mac-limit`) |
| `vms.stop` | `name, timeout? (60)` | `{result}` – `alreadyStopped`, `powerdown`, `quit`, `forcedOff`, `killed` |
| `vms.reset` / `vms.remove` | `name` | `null` – refused while running |
| `vms.upgrade` | `name, image?` | `Vm` – to its line's newest image unless `image` is named |
| `vms.set` | `name, audio?, microphone?, clipboard?, camera?` | `{vm, takesEffectAtNextStart}` |
| `vms.ssh` | `name, command?: [string]` | `{argv, shellLine}` – what to run in a terminal |
| `vms.agent` | `name, wait?` | the agent's report (spec §11.9) |
| `vms.log` | `name, lines?, qemu?` | `{lines}` |
| `vms.wait` | `name, timeout?` | `{via}` – blocks until SSH or the agent answers |
| `vms.share` | `name` | `{path}` – the share folder, created if it is the default one |
| `vms.nextName` | `recipe` | `{name}` – the first free VM name for it (`dev`, `dev-2`, …) |
| `vms.forget` | `name` | `null` – stop listing a VM that runs from elsewhere (its package stays) |
| `vms.useExternal` | `path` | `Vm` – a VM package that runs from where it is; `{networkVolume: true}` in the error data when it is on a network volume and `confirmNetwork` was not set |
| `vms.limits` | – | `{macMax, macRunning: [name]}` |

### Images

| Method | Params | Result |
|---|---|---|
| `images.list` | – | `{images: [Image], unreadable}` |
| `images.build` | `recipe, password?` | Job `build` (macOS recipes need the guest user's password once – kept in memory only until the build VM uses it) |
| `images.remove` | `name` | `null` – refused while VMs use it (`in-use`) |

### Recipes and sources

| Method | Params | Result |
|---|---|---|
| `recipes.list` | – | `{recipes: [Recipe], lines: {line: [Version]}, idLengths, qemuGL, unreadable}` |
| `recipes.available` | – | fetches the enabled remote sources' catalogs: `{offered: [CatalogRecipe], updates: [{id, recipe: CatalogRecipe}], suggested: [CatalogRecipe], errors: {sourceId: message}}` |
| `recipes.install` | `id, source` | Job `recipe-install` (with what it requires) |
| `recipes.installSuggested` | – | `[Job]` |
| `recipes.remove` | `id` | `null` |
| `recipes.removeLine` | `line` | `{removed, kept}` |
| `recipes.pruneVersions` | – | `{removed}` |
| `sources.list` | – | `[Source]` |
| `sources.add` | `input, private?, token?, branch?, first?` | `{source, note}` – checked before it is kept; a private repository's token goes to the system keychain, never into a file |
| `sources.remove` | `id` | `null` (its token leaves the keychain) |
| `sources.setEnabled` | `id, enabled` | `[Source]` |
| `sources.move` | `id, up` | `[Source]` |
| `sources.check` | – | `{results: {sourceId: {ok, note}}}` |

### Packages (spec §11.10)

| Method | Params | Result |
|---|---|---|
| `packages.inspect` | `path, expect?: "vm"|"image"|"recipe", external?` | `{kind, valid, reason?, conflict?: {vmName}|{image}, networkVolume}` – what an open panel checks before it closes, and what an import would ask about |
| `packages.import` | `path, name?, replace?` | Job `import` → `result: {kind, names}` |
| `packages.export` | `kind: "vm"|"image"|"recipe", name, folder, withImage?` | Job `export` → `result: {path}` |

### Doctor, macOS base, Linux builder

| Method | Params | Result |
|---|---|---|
| `doctor.check` | – | `{prerequisites, nixosPreflight, macosPreflight, latestRelease, buildBlockers: {nixos, macos}, startBlockers: {nixos, macos}, issues}` |
| `doctor.setPath` | `item: "nix"|"qemu"|"flake", path` | as `doctor.check` |
| `macos.downloadIpsw` | – | Job `ipsw-download` (cancel pauses; it continues from there) |
| `macos.installBase` | `user, fullName, password, ipsw?` | Job `macos-install` |
| `builder.installNix` / `builder.setup` / `builder.apply` (`force?`) / `builder.cleanUp` | – | Job – each asks for an administrator through the system's own dialog |

### Jobs

| Method | Params | Result |
|---|---|---|
| `jobs.list` | – | `[Job]` |
| `jobs.log` | `id, from?` | `{lines, next}` |
| `jobs.cancel` | `id` | `Job` |
| `jobs.dismiss` | `id` | `null` – a finished job leaves the list |

## 8. Secrets

Passwords (`images.build`, `macos.installBase`) and tokens (`sources.add`) travel only over the pipe between the UI
and its own `omaf serve`. The server never logs params of these methods, keeps a password only until it is used and
stores tokens only in the system keychain (macOS Keychain; Linux: OPEN).

## 9. Status

API v1 is new with the Rust core (plan B117). The method tables are the contract; the Rust types in
`crates/omaflake-core/src/api/` are its implementation, and their JSON Schema is OPEN (to be generated from those
types and published under `schema/api/`).
