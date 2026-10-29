# CLAUDE.md – project rules for Claude Code

## Context

omaflake: clean NixOS and macOS VMs on Mac and Linux (macOS hosts on Apple silicon: menu bar app and runner
`omaf-runner`, NixOS and macOS guests; Linux hosts with Nix or NixOS, x86_64 or aarch64: native app, NixOS guests;
CLI `omaf` and `omaf ui` on both). Free software under **AGPL-3.0-or-later** (SPDX header in every own source file; the recipes in
`omaflake/recipes` stay MIT), GitHub organization `omaflake`, repository `omaflake`. A project of
falkOSS – Falcon Lake Open Source Software gUG (haftungsbeschränkt) i. G.
Design and milestones: **docs/spec.md**; findings and checklists: **docs/plan.md** – read before
any work and keep them up to date.

## Working rules

- **Language:** everything in this repository is English (code, comments, docs, UI, commits).
  Talk to the maintainer in German.
- **Licence header:** every new own source file (Swift, Rust, Nix, shell, Python) starts with
  `SPDX-License-Identifier: AGPL-3.0-or-later` (after a shebang or `swift-tools-version` line); recipes, modules, key
  bindings and looks are MIT and live in `omaflake/catalog`.
- **Public from day one:** no personal data in tracked files – no names, initials, user names,
  home paths, SSH keys, hostnames. Personal settings belong in `nix/local/`.
- **Repositories:** app work happens only in this checkout (`omaflake/omaflake`). Recipes, modules, key bindings and
  looks live in `omaflake/catalog` (checked out next to this one as `../catalog`; its own CLAUDE.md) – this
  repository holds none (plan B107, B140, B142; `omaflake/recipes` is archived). The website is a separate repo,
  `omaflake/website` (static GitHub Pages site), as `../website`.
- **Developing with the catalog:** the data folder's config.json has the local checkout `../catalog` as its source
  (`omaf sources add <path> --first`), not the remote omaflake/catalog – a recipe or module changed there is built as
  it is on disk. Change it in `../catalog`, run its `tools/catalog.py`, commit there.
- **`nix/local/` is personal:** each developer's user, keys and extras (git-ignored; template
  `nix/local.example/`). Claude may read and edit it when asked, but never copies anything from it
  into tracked files and never commits it. Use the image flake as `path:` (see nix/README.md).
- **Never delete (your own actions):** during development, don't delete files – move them to `_to_delete/` (repo)
  or `~/.omaflake/_to_delete/`. This is a rule for developers and agents, **not for the product**: when the user
  removes something in the app or the CLI (VM, image, reset, upgrade), omaflake deletes it for real (`Disposal`),
  with a clear confirmation in the app; share folders (the user's files) are never deleted.
- **Verified before claimed:** check Nix options (`nix eval`, nixpkgs/nix-darwin sources), QEMU
  parameters (`qemu-system-aarch64 -device help`) and Virtualization.framework API (SDK headers).
  Mark what can't be verified as `OPEN` in code/docs.
- **After every change:** `swift build` and `swift test`; `cargo fmt --all --check`, `cargo clippy --all-targets` and
  `cargo test`; after runner changes `scripts/sign-runner.sh`;
  after Nix changes `nix flake check`, `nix flake check "path:$PWD/nix"` and, with the recipes,
  `nix flake check "path:$PWD/nix" --override-input omaflake-catalog "path:$PWD/../catalog"`; after catalog changes
  `python3 tools/catalog.py --check` there. Commit only
  when green.
- **Small steps:** one milestone (spec §7) at a time; check the acceptance criterion explicitly and
  tick it in the docs. Findings get ids `B…` (NixOS/general) and `BM…` (macOS).
- **File versions:** a change to a file omaflake writes or reads (JSON, recipe.toml, catalog.json) gets a new schema
  version in `docs/schema/` and a migration step; older versions stay readable. Goal from 1.0.0 on: the minimum versions (`FileFormat`,
  `RecipeLibrary.recipeMinimum`, `RecipeCatalog.minimum`, the image flake) are raised only with a major release.
  **Before 1.0.0 backward compatibility does not matter** (the maintainer's decision, plan B141): a release may raise
  the minimum versions, drop old API params, formats and migrations without a transition – noted in the changelog.
- **Changelog:** every user-visible change gets a line under `## [Unreleased]` in `docs/changelog.md` (on omaflake.dev)
  in the same commit; a release turns that section into `## [x.y.z] – date`, and its text is the release's notes.
- **No secrets** in the repository (passwords, tokens, keys). Developer-local files (e.g. tokens for tooling)
  belong in `~/.omaflake/developer/`, which omaflake never touches.

## Layout

```
Cargo.toml, crates/        Rust core: omaflake-core (all logic), omaf (CLI and `omaf serve`, the JSON-RPC API: docs/api.md)
                           omaflake-brand (design tokens, generated from brand/tokens.toml)
                           omaflake-tui (`omaf ui`: ratatui client of the page models, docs/api.md §10)
                           omaflake-gui (the Linux app, iced); omaflake-i18n (translations of omaf ui and the Linux app,
                           generated from App/Resources/de.lproj by scripts/translations.py); omaflake-macros
                           (vm_err!: an error's format string is its translation key, plan B125)
Package.swift              OmaflakeKit (library), omaf-runner, OmaflakeApp
Sources/OmaflakeKit/       Core/ + API/: client of `omaf serve` and the API's types; Runner/: agent protocol, macOS VM
                           bundle, sockets (for omaf-runner); Brand/: icons
Sources/omaf-runner/       macOS guests on Virtualization.framework (needs entitlement)
Sources/OmaflakeApp/       SwiftUI menu bar app – a client of `omaf serve`, presentation only
App/                       Info.plist, runner entitlements
scripts/                   bundle-app.sh, sign-runner.sh, bootprobe.sh
qemu/                      QEMU build with virglrenderer + ANGLE, patches
nix/modules/vm-guest.nix   guest module (root flake output nixosModules.vm-guest)
docs/                      spec, plan, cli, app, macos-base, linux-builder
brand/                     omaflake brand set (icons, favicon, banner, Sources/OmaflakeKit/Brand/BrandAssets.swift);
                           edit brand/generate.py and brand/tokens.toml (design tokens: brand/tokens.py), not exports
nix/                       image flake: builds images from the recipes of the sources with their modules (input
                           omaflake-catalog), the platforms (NixOS, macOS), the omaflake agent, Linux builder host;
                           use as path:, shipped in the app
nix/local.example/         template for personal settings → nix/local/ (git-ignored)
```

## Next step

State and order: **docs/plan.md §8** (2026-10-29, release 0.7.2 – parts too old to read can be updated, B156; 0.7.1: the loading screen of NixOS guests, B155; 0.7.0: key bindings with modes, the studios for recipes, key bindings and looks, the app in four groups, B146–B154): recipes as building blocks – platform, modules, look,
key bindings, requirements (B141, B142; modules and recipes in omaflake/catalog), locales, profiles, My Recipes; an image
is a recipe with a credential, a look and a locale (B137–B141), looks for the guests (B135, installable since 0.6.0), the Linux host checked on an x86_64 PC (0.4.0, B124–B131); next the OPEN items in docs/linux.md and plan §8; M12 (signing, notarization, helper, macOS camera extension, self-update) once the Developer ID is there. Development base `mac-base-26A434`; develop with the `lab` recipes
(developer mode in the app).
Workflow for macOS settings without a nix-darwin option: set them by hand in a clone, diff
`defaults export` / container plists against a snapshot, then declare them (plan BM22).
