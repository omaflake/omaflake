<!-- SPDX-License-Identifier: AGPL-3.0-or-later -->
# Porting OmaflakeCore to Rust

`crates/omaflake-core` is the Rust port of `Sources/OmaflakeCore` – every Swift file except `Brand.swift` and
`BrandAssets.swift` (AppKit; they stay with the Swift app). `crates/omaf` becomes the CLI and the JSON-RPC server
(`omaf serve`, docs/api.md). The Swift sources stay untouched until the port is complete.

The foundation is ported (real code and tests). Every other module is a **skeleton**: all types complete (with the
exact JSON keys of the Swift `Codable`), every function signature, bodies `todo!("port from <File>.swift")`. Work
packages F1–F5 fill in the bodies **in parallel** – the signatures are the contract between them.

## Rules for the work packages

1. **Edit only your own module files** (table below). The signatures you see are what other packages call. If one is
   wrong or missing, add the smallest change in your own module and say so in your report; never change another
   package's file. A `pub(crate)` helper may be added freely inside your module.
2. **Port faithfully**: read the Swift file completely; same file names, JSON keys, defaults, error texts, migration
   steps, edge cases. Carry the doc comments over (they hold design decisions, plan/spec references).
3. **Port the Swift tests** of your modules from `Tests/OmaflakeCoreTests/*.swift` into `#[cfg(test)] mod tests` of
   the module (helpers: `crate::paths::tests::temp_paths()`, `crate::models::tests::{config, dev_defaults}`).
4. When a module is done, remove its `#![allow(unused_variables, dead_code)]` line and the "Skeleton" note in its
   module doc, and mark it *done* in the table below.
5. `cargo fmt --all` (rustfmt.toml at the root; every file is formatted, so it only touches what you changed),
   `cargo build`, `cargo test`, `cargo clippy --all-targets` must stay green (warnings from other packages' `todo!`
   are fine). Dependencies are fixed in the workspace `Cargo.toml`; a package needing another crate says so in its
   report instead of adding it (one shared file).
6. SPDX header `// SPDX-License-Identifier: AGPL-3.0-or-later` as the first line of every new Rust file (`word_list.rs`
   is MIT like its BIP-39 source and the Swift file it ports). English everywhere. No personal data. Never delete
   files – move them to `_to_delete/`. Mark what cannot be verified as `OPEN`.

## Conventions

- **Errors**: one type, `crate::Error` (= Swift `VMError`): a message (`Display`) and an optional
  `code: Option<&'static str>` – the error kind of docs/api.md §3 (`crate::error::codes`, kebab-case: `not-found`,
  `exists`, `vm-running`, `vm-stopped`, `in-use`, `mac-limit`, `unsupported-system`, `unreadable-file`, `newer-file`,
  `cancelled`, `prerequisite`, `network`, `auth`, `internal`). `pub type Result<T> = std::result::Result<T, Error>`.
  Build them with `vm_err!("… {}", x)` or `vm_err!(codes::X, "… {name}")` – the format string becomes the message's
  key (`{…}` → `%@`) and needs German in `App/Resources/de.lproj/Localizable.strings` (test `api::translated`, plan
  B125); `Error::new`/`Error::with_code(codes::X, …)` only for a message without a key (built at run time),
  `Error::cancelled()`, `Error::unsupported(what)`. `From` exists for `io::Error` (message = strerror text),
  `serde_json`, `toml`, `ureq`; `IoContext::with_path(&p)` adds the path; `error::os_text(&e)` is Swift's
  `strerror(errno)`. Give your errors the api.md kind where one fits (e.g. "Instance … does not exist" → `not-found`).
- **Localization**: Swift's `String(localized:)` texts become plain English strings. Translation moves to the UIs,
  keyed on `code` – OPEN.
- **Paths**: `PathBuf` / `&Path`. `platform::expand_tilde` = `expandingTildeInPath`, `platform::abbreviate_tilde` =
  `abbreviatingWithTildeInPath`, `platform::standardize` = `standardizedFileURL`, `platform::resolve_symlinks` =
  `resolvingSymlinksInPath()`, `platform::is_executable_file` / `is_writable` = FileManager's checks.
- **User and host**: `platform::user_name()` = `NSUserName()`, `platform::full_user_name()` = `NSFullUserName()`,
  `platform::home_dir()` = `homeDirectoryForCurrentUser`, `platform::machine()` = `uname -m`. Environment
  (`VM_HOME`, `VM_FLAKE`, `VM_QEMU`, `VM_RUNNER`, `OMAFLAKE_*`) is read where Swift reads it.
- **Dates**: `chrono::DateTime<Utc>`. JSON dates exactly as Swift's `.iso8601` strategy (`2026-10-21T10:00:00Z`, whole
  seconds): `#[serde(with = "crate::json::iso8601")]`, `crate::json::iso8601::option` for `Option`. The custom
  strategy of RecipeSources.swift (`.installed.json`, written with milliseconds, read with or without) is
  `crate::json::iso8601_fractional`. `json::now()` is "now" in whole seconds (Swift `Date()` that survives a round
  trip).
- **JSON files**: keys identical to the Swift `Codable` output (camelCase where Swift used it, explicit
  `#[serde(rename)]` for `memoryGB`, `diskGB`, `sshPort`, `externalVMs`, …), optional fields left out when None
  (`skip_serializing_if`, = `encodeIfPresent`). Files with a schema go through `FileFormat`: `decode::<T>(&bytes)`
  → `(T, migrated)` (write back when migrated), `encode(&T)` → pretty, sorted keys, `$schema` first, slashes not
  escaped (Swift `[.prettyPrinted, .sortedKeys, .withoutEscapingSlashes]`), `check_writable(path)` before writing
  (refuses a newer version), `write_atomic(path, bytes)` (= `.atomic`). Other JSON Swift wrote with a plain
  `JSONEncoder` `[.prettyPrinted, .sortedKeys]` (guest/settings.json, agent.json) uses
  `json::to_pretty(value, escape_slashes: true)` – Swift escapes `/` as `\/` there. Every reader accepts files the
  Swift version wrote (fixtures in `testdata/`).
- **Migrations**: a `FileFormat` is a `const` with `&'static [Step]`, `Step = fn(&mut Map<String, Value>) -> Result<()>`
  (see `InstanceConfig::FILE_FORMAT`, `ImageInfo::FILE_FORMAT`, `Settings::FILE_FORMAT`,
  `MacVmConfig::FILE_FORMAT`, `PackageManifest::FILE_FORMAT`, `RecipeLibrary::FILE_FORMAT`). `$schema` handling and
  the version checks are in `file_format.rs` (done).
- **Cancellation**: Swift's task-local `Cancellation.current` is an explicit `&Cancellation` parameter on long-running
  functions (`ImageStore::build`, `MacImageBuilder::build`, …); `Cancellation` is a cheap clone (`Arc` inside),
  `Cancellation::new()` is a token nobody cancels. Hand it to `Shell` via `RunOptions { cancel: Some(c), .. }` /
  `StreamOptions`: the running process is SIGTERMed on cancel (Swift `Process.terminate()`); `on_cancel(handler)`
  runs a handler (right away if already cancelled); `check()?` between steps.
- **Callbacks**: log lines `&Log` (`= dyn Fn(&str) + Send + Sync`), progress `&Progress` (`= dyn Fn(i64, i64) + Send +
  Sync`), both in `lib.rs`. Swift's `started: (Process) -> Void` is `started: Option<&dyn Fn(Pid)>` (send SIGTERM
  with `spawn::send_signal` to stop it).
- **Processes**: `Shell::run / check / stream(exe, &args, &opts)` (`RunOptions`, `StreamOptions` – Swift's
  defaulted parameters; `environment: &[(&str, &str)]`). `ShellResult.status` is the exit code or the signal number
  (Swift `terminationStatus`: 15 = SIGTERM). Detached children: `spawn::spawn_detached`, `spawn::wait_for_exit`,
  `spawn::live_pid`, `spawn::executable_path`, `spawn::reap_if_exited` (= `waitpid(WNOHANG) == pid`),
  `spawn::is_alive`, `spawn::send_signal`. `Pid = i32`.
- **Time spans**: Swift `TimeInterval` parameters are `std::time::Duration`.
- **Numbers**: Swift `Int` fields are `u32` for counts and sizes (cpus, memory/disk GB, display), `u16` for ports,
  `i64` for byte counts that may be compared with free space, `u64` for file sizes.
- **Namespaces**: a Swift `enum X {}` used as a namespace is a Rust `pub enum X {}` with associated functions
  (`RecipeId::base(…)`, `Platform::host_system(…)`, `Shell::run(…)`). Overloads get distinct names (see the symbol
  map). Tuples in public Swift signatures became small structs where they are passed around (`AvailableRecipe`,
  `ParsedId`, `RecipeParts`).
- **Protocols**: `VMBackend` is the object-safe trait `VmBackend` (backend.rs). `QemuBackend` (F2) and `VzBackend`
  (F4) implement it in their own files; `Backends::backend(kind)` returns `&dyn VmBackend`. Default methods
  (`wait_for_ready`, `agent_state`) are on the trait as in Swift's extension.
- **macOS-only APIs** (Keychain via `security-framework`, `clonefile`, `copyfile`, `proc_pidpath`, dhcpd leases,
  `osascript`, `codesign`) sit behind `#[cfg(target_os = "macos")]`, with `Error::unsupported("…")` elsewhere. No
  macOS-only types in public signatures – the crate must build for Linux too (OPEN: not yet compiled for a Linux
  target).
- **unsafe**: only for libc calls (spawn/setsid, flock, sockets, waitid, clonefile/copyfile, getpwuid_r, uname,
  access), each with a `// SAFETY:` comment.
- **Blocking**: no async runtime. Swift `async` functions (recipe downloads, update check) are blocking (`ureq`);
  where Swift used tasks, use threads (`std::thread::scope`).
- **Version**: `crate::VERSION` (workspace version 0.2.0 = App/Info.plist `CFBundleShortVersionString`) replaces
  `Bundle.main` version lookups (e.g. the package manifest's `omaflake` field).

## Modules, status, work packages

| Swift file | Rust module | Status | Package |
|---|---|---|---|
| Models.swift | `models` | done | foundation |
| FileFormat.swift | `file_format` | done | foundation |
| Platform.swift | `platform` (+ user/home/path helpers) | done | foundation |
| Paths.swift | `paths` (`Paths`, `InstancePaths`, `Settings`) | done | foundation |
| OmaflakeID.swift | `omaflake_id` | done | foundation |
| WordList.swift | `word_list` | done | foundation |
| Shell.swift | `shell` | done | foundation |
| Spawn.swift | `spawn` | done | foundation |
| Cancellation.swift | `cancellation` | done | foundation |
| Trash.swift | `trash` (`Disposal`) | done | foundation |
| Sockets.swift | `sockets` | done | foundation |
| SSHCommand.swift | `ssh_command` | done | foundation |
| Packages.swift | `packages` | done | foundation |
| – (Foundation behaviour) | `error`, `json` | done | foundation |
| InstanceStore.swift | `instance_store` | done | F1 stores |
| ImageStore.swift | `image_store` | done | F1 stores |
| ImageMigration.swift | `image_migration` | done | F1 stores |
| DataFolder.swift | `data_folder` | done | F1 stores |
| Transfer.swift | `transfer` | done | F1 stores |
| QMPClient.swift | `qmp_client` | done | F2 runtime |
| QEMUCommand.swift | `qemu_command` | done | F2 runtime |
| QEMURuntime.swift | `qemu_runtime` | done (+ `set_main_screen_resolution`) | F2 runtime |
| Backend.swift | `backend` | done | F2 runtime |
| AgentProtocol.swift | `agent_protocol` | done | F2 runtime |
| BuildProgress.swift | `build_progress` | done | F2 runtime |
| RecipeSources.swift | `recipe_sources` | done | F3 recipes |
| GuestSettings.swift | `guest_settings` | done | F3 recipes |
| GuestChoices.swift | `guest_choices` | done | F3 recipes |
| UpdateCheck.swift | `update_check` | done | F3 recipes |
| MacVM.swift | `mac_vm` | done | F4 macOS |
| VZBackend.swift | `vz_backend` | done | F4 macOS |
| IPSWDownloader.swift | `ipsw_downloader` | done | F4 macOS |
| MacBaseInstaller.swift | `mac_base_installer` | done | F4 macOS |
| MacImageBuilder.swift | `mac_image_builder` | done | F4 macOS |
| Prerequisites.swift | `prerequisites` | done | F5 doctor |
| BuilderMaintenance.swift | `builder_maintenance` (with `AdminShell`) | done | F5 doctor |
| Brand.swift, BrandAssets.swift | – | stay in Swift (AppKit) | – |

Tests to port per package (Tests/OmaflakeCoreTests/OmaflakeCoreTests.swift suites): F1 – RecipeRenameTests
(`oldFlavorKeyIsRead`), InstanceStoreTests, ImageStoreTests, DataFolderTests, `removingAnImageIsRefusedWhileUsed…`,
RetentionTests, ShareCleanupTests, PackageTests.`oldLayoutBecomesPackages`, TransferTests, RecipeIDTests
(`legacyNamesMapToOmaflakeIDs`, `migratesLegacyMacImageCurrentLinkAndInstance`), VZBackendTests.`imageRecipeFromName`,
`macImageIsAVMFolder`, FileFormatTests (`imageVersionZeroIsMigratedAndWrittenBack`, `unreadableImagesAreListed`).
F2 – AgentProtocolTests.swift (with nix/agent/testdata/vectors.json), QEMUCommandTests, QMPParseTests,
BuildProgressTests, BuildLogTests. F3 – RecipeSourceTests, GuestSettingsTests, GuestChoicesTests, UpdateCheckTests,
RecipeVersionTests, FileFormatTests (`installedIndexIsWrapped`, `recipeVersionIsReadFromTheToml`,
`catalogVersionOneIsRead`, `everyVersionHasItsSchema`, `whatOmaflakeWritesMatchesItsSchema` – needs F1/F4 types).
F4 – MacCloneTests, MacVMTests, VZBackendTests (rest), BaseUserTests, PreflightTests.`parsesRunnerLatest`,
FileFormatTests.`macVMVersionGivesWayToSchema`. F5 – PrerequisitesTests, PreflightTests (rest), BuilderMaintenanceTests,
SetupCommandTests, NixInstallerPinTests.

### Who calls whom (the contract)

- F1 → F2: `QemuRuntime::locate` (qemu-img), `DataFolder::is_running` uses `spawn::live_pid` only. F1 → F3:
  `RecipeLibrary::{available, installed, keep, prune_versions, folder, content_hash, save_manifest, import_recipes,
  recipes_in}`, `ImageStore::guest_arguments` (in guest_settings.rs, F3). F1 → F4: `MacVmBundle`, `VzBackend.runner`
  (Transfer). F1 → F5: `Prerequisites::locate_git` (git revision).
- F2 → F1: `InstanceStore::{create, reset, upgrade, remove, backing_image, check_config_runs_here, check_runs_here}`.
  F2 → F4: `VzBackend::locate_runner` (bridge, `Backends::new`).
- F3 → F1: `ImageStore::list`, `ImageInfo`. F3 → F4: `MacImageBuilder::newest_base_in`, `MacBaseInstaller::{user_of,
  default_public_keys}`.
- F4 → F1: `InstanceStore::{validate, exists, save, list, remove, replace_embedded_image}`, `ImageStore::{image,
  set_current, write_info, recipe_revision, flake_ref, image_name, guest_arguments}`. F4 → F3: `RecipeLibrary::{available,
  keep, prune_versions}`. F4 → F5: `Prerequisites::MAC_SPACE_NEEDED`.
- F5 → F1: `ImageStore::recipe_defaults`. F5 → F2: `QemuRuntime::{locate, capabilities}`. F5 → F4:
  `MacImageBuilder::newest_base_in`, `MacBaseInstaller::{is_ready, user_of, default_public_keys}`,
  `IpswDownloader::{file, resume_file_for}`, `free_space`, `gigabytes`.

## Symbol map (Swift → Rust, where not obvious)

| Swift | Rust |
|---|---|
| `VMError` | `crate::Error` (`vm_err!`) |
| `QEMUCommand`, `QEMURuntime`, `QEMUBackend`, `QEMUCapabilities` | `QemuCommand`, `QemuRuntime`, `QemuBackend`, `QemuCapabilities` |
| `QMPClient(socket:timeout:)` | `QmpClient::connect(socket, timeout)` |
| `VZBackend`, `VMBackend`, `VMStatus`, `GLBackend` | `VzBackend`, trait `VmBackend`, `VmStatus`, `GlBackend` |
| `MacVMConfig`, `MacVMConfig.Display`, `MacVMBundle`, `bundle.clone(to:)` | `MacVmConfig`, `MacDisplay`, `MacVmBundle`, `clone_to` |
| `DHCPLeases.ip(for:in:)`, `DHCPLeases.ip(for:)` | `DhcpLeases::ip_in(mac, text)`, `DhcpLeases::ip(mac)` |
| `MACAddress.randomLocallyAdministered()` | `MacAddress::random_locally_administered()` |
| `IPSWDownloader`, `MacOSRelease` | `IpswDownloader`, `MacOsRelease` (`url: String`) |
| `MacBaseInstaller.Request`, `.Outcome`, `.readyMarker`, `user(of:logs:)`, `build(fromLatest:)` | `BaseRequest`, `BaseOutcome`, `READY_MARKER`, `user_of`, `build_from_latest` |
| `MacImageBuilder.Request`, `newestBase(_ paths:)`, `BuildVM` | `BuildRequest`, `newest_base_in(&paths)`, `BuildVm` |
| `SSHCommand.arguments(_:paths:host:port:command:)` | `SshCommand::arguments(c, paths, host, port, command)` (`SshCommand::DEFAULT_HOST`) |
| `OmaflakeID`, `.Kind`, `.Lengths`, `.list`, `.length` | `OmaflakeId`, `OmaflakeIdKind`, `Lengths`, `OmaflakeId::list()`, `OmaflakeId::LENGTH` |
| `OmaflakeID.words(_:count:)`, `words(hex:)`, `words(of:)` | `OmaflakeId::words(&bytes, n)`, `words_hex`, `words_of` |
| `OmaflakeID.parse(_:)` (tuple) | `OmaflakeId::parse` → `ParsedId { kind, identifier, words }` |
| `OmaflakeID.recipe/image/vm(_:words:)` | `OmaflakeId::recipe(id, n)`, `image(&img, n)`, `vm(&c, n)` (Swift default n = `LENGTH`) |
| `RecipeID.make(author:kind:name:)`, `make(base:contentHash:words:)` | `RecipeId::make`, `RecipeId::make_with_hash(base, hash, n)` |
| `RecipeID.parts` (tuple), `name(ofID:)` | `RecipeId::parts` → `RecipeParts`, `RecipeId::name_of_id` |
| `RecipeID.candidates(_:in:)`, `candidates(_:among:)` | `RecipeId::candidates_in(input, &map)`, `RecipeId::candidates(input, &ids)` |
| `RecipeID.resolve(_:in:choose:)`, `resolve(_:among:choose:)` | `RecipeId::resolve_in`, `RecipeId::resolve` (`Option<Chooser>`) |
| `RecipeDefaults.description(in:)` | `description_in(Some("de"))` |
| `RecipeDefaults(kind:systems:cpus:…)` | `RecipeDefaults::new(kind, systems, cpus, memory_gb, disk_gb, network, share, ephemeral, user)` |
| `InstanceConfig(name:…created:)` | `InstanceConfig::new(…, created)` (pass `json::now()`) |
| `StopResult.rawValue` | `StopResult::description()` / `Display` |
| `Paths(root:configFile:)`, `Paths.standard()`, `Paths.defaultHome` | `Paths::new(root, Option<config>)`, `Paths::standard()`, `Paths::default_home()` |
| `Settings.load(_ paths:)`, `load(file:)`, `save(file:)` | `Settings::load(&paths)`, `Settings::load_file(path)`, `save_file(path)` |
| `Settings(flake:qemu:runner:)` | `Settings::with(flake, qemu, runner)` / `Settings { .., ..Default::default() }` |
| `Settings.recipeSources` | `settings.recipe_sources()` |
| `InstancePaths.runtimeNames` | `InstancePaths::RUNTIME_NAMES` |
| `ImageStore.Info`, `ImageStore.Image` | `ImageInfo`, `Image` (`image.recipe()`, `image.kind()`) |
| `ImageStore.line(of:)`, `currentName(recipe:)`, `currentName(line:)` | `line_of`, `current_name`, `current_name_in_line` |
| `ImageStore.image(named:)`, `image(at:)`, `images(of:)`, `images(inLine:)` | `image(name)`, `image_at(dir)`, `images_of`, `images_in_line` |
| `ImageStore.recipe(ofImage:)`, `imageName(ofLink:)` | `recipe_of_image`, `image_name_of_link` |
| `InstanceStore.checkRunsHere(_: Image)`, `(_: InstanceConfig)` | `check_runs_here(&image)`, `check_config_runs_here(&c)` |
| `InstanceStore.nextSSHPort(isFree:)` | `next_ssh_port(&Ports::is_free)` |
| `InstanceStore.encoder/decoder` | `InstanceConfig::FILE_FORMAT.encode/decode` |
| `DataFolder.move(from:to:settingsFile:log:)` | `DataFolder::move_to(old, target, settings_file, log)` |
| `Transfer(_ backends:)`, `Transfer.kind(of:)` | `Transfer::from_backends`, `Transfer::kind_of` |
| `Transfer.Imported`, `.Conflict` | `transfer::Imported`, `transfer::Conflict` |
| `Package.Kind`, `Package.Manifest`, `name(of:kind:)`, `kind(of:)` | `PackageKind`, `PackageManifest` (`packages::Manifest`), `Package::name_of`, `Package::kind_of` |
| `Disposal` (Trash.swift) | `trash::Disposal` |
| `Ports`, `connectUnix(_:timeout:)` | `sockets::Ports` (`DEFAULT_TIMEOUT`), `connect_unix(path, timeout) -> UnixStream` |
| `spawnDetached`, `waitForExit`, `executablePath(of:)`, `livePid(file:executable:)` | `spawn::spawn_detached`, `wait_for_exit`, `executable_path`, `live_pid` |
| `Shell.Result`, `LineSplitter` | `ShellResult`, `shell::split_lines` |
| `AgentProtocol.Message`, `AgentLink.End`, `AgentLink.CameraRequest` | `AgentMessage`, `LinkEnd`, `CameraRequest` |
| `AgentStatus.Memory/Disk/Address`, `AgentHello.ProtocolRange` | `AgentMemory`, `AgentDisk`, `AgentAddress`, `ProtocolRange` |
| `AgentLink.send` (thread-safe) | `AgentLink::sender()` → `AgentSender::send` |
| `AgentDecoder.next()` | `AgentDecoder::next_frame()` |
| `JSONEncoder.agent` / `JSONDecoder.agent` | `json::to_pretty(…, true)` / serde with `json::iso8601` |
| `RecipeSource.Kind`, `RecipeSource.official`, `tag(forKey:)` | `RecipeSourceKind`, `RecipeSource::official()`, `RecipeSource::tag_for_key` |
| `RecipeCatalog.Source/File/Recipe` | `CatalogSource`, `CatalogFile`, `CatalogRecipe` |
| `RecipeLibrary.available` (tuples) | `Vec<AvailableRecipe { id, folder, source }>` |
| `RecipeLibrary.version(of:)`, `requires(of:)`, `recipes(in:)` | `version_of`, `requires_of`, `recipes_in` |
| `RecipeLibrary.Installed`, `InstalledFile`, `Version` | `recipe_sources::{Installed, InstalledFile, Version}` |
| `SourceTokens.set(_:for:)` | `SourceTokens::set(Option<&str>, &source)` |
| `UpdateCheck.feedURL(bundle:)`, `newer(than:feed:)` | `UpdateCheck::feed_url(info_plist_value)`, `newer(current, feed_url)` |
| `Prerequisite.Item`, `.Status`, `Prerequisites.installNix` | `PrerequisiteItem`, `PrerequisiteStatus`, `Prerequisites::install_nix()` |
| `Prerequisites.machine()` | `platform::machine()` |
| `BuilderMaintenance.installNixCommand`, `nixInstallerURL` | `install_nix_command()`, `nix_installer_url()` |
| `AdminShell.Cancelled` | error code `codes::CANCELLED` |
| `KeepAwake.during`, `freeSpace(at:)`, `gigabytes(_:)`, `tail(_:lines:)` | `KeepAwake::during`, `free_space`, `gigabytes`, `qemu_runtime::tail` |
| `BuildProgressTracker.macSteps` | `BuildProgressTracker::MAC_STEPS` |
| `LastPercent` | `mac_base_installer::LastPercent` |

## Not mapped 1:1 (and why)

- **Brand**: the Dock icon moved into the core: `QemuRuntime::start` writes the embedded 512 px PNG (`DOCK_ICON`,
  `include_bytes!` from `brand/`) to `Paths::dock_icon()` and sets `OMAFLAKE_DOCK_ICON` to it; the app writes nothing
  into the data folder.
- **Screen size**: `QEMURuntime.mainScreenResolution` used CoreGraphics. On macOS the core asks `system_profiler
  SPDisplaysDataType -json` once (pixels and points – Swift's result); the UIs pass nothing (decided: pixels alone would
  be a few pixels off). `qemu_runtime::set_main_screen_resolution` stays for an in-process UI or another host. OPEN: system_profiler's JSON keys are not an API.
- **Foundation locale data**: `GuestChoices` (`Locale.availableIdentifiers`, `TimeZone.knownTimeZoneIdentifiers`,
  localized names) and `GuestSettings.mac()` (current time zone, locale, HIToolbox keyboard) – OPEN, see the module docs.
- **URLSession downloads**: the IPSW downloader resumes with HTTP `Range` from `<build>.ipsw.partial` instead of
  URLSession resume data in `.resume` (a Swift `.resume` file is ignored). `KeepAwake` and `free_space` – OPEN (see
  ipsw_downloader.rs).
- **Bundle**: the bundled image flake, the runner next to the executable and the package manifest's version used
  `Bundle.main`; Rust uses `std::env::current_exe()` (`<exe>/../../Resources/nix`) and `crate::VERSION`. The update
  feed URL from Info.plist is passed in by the app.
- **Volumes**: `DataFolder` compares `st_dev` instead of `volumeIdentifierKey`; `Transfer::is_on_network_volume` uses
  `statfs`: macOS `MNT_LOCAL` not set, Linux `f_type` of known network file systems (FUSE such as sshfs: OPEN).
- **Shell**: stdout and stderr are read concurrently (Swift could block on a full stderr pipe); a cancelled process is
  waited for with `waitid(WNOWAIT)` before it is untracked, so no reused PID is ever signalled.
- **Sockets**: `connect_unix` returns an owned `UnixStream` (close-on-exec) instead of a descriptor; `Ports::is_free`
  binds without `SO_REUSEADDR` like Swift.
- **AgentLink** owns a `UnixStream` and hands out `AgentSender` for writes from other threads.
- **resolve_symlinks** drops a leading `/private` when the existing part of the path exists without it (Swift decided
  on the whole path) – consistent for files that do not exist yet.
- **Error messages of decoding** carry serde's text instead of Foundation's `localizedDescription`.
- **omaf-runner** (Swift) still uses `OmaflakeCore` itself (e.g. `AgentLink` in its bridge); it is not part of this
  port – OPEN: which parts the runner keeps in Swift once the app talks to `omaf serve`.
