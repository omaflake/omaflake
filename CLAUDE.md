# CLAUDE.md – project rules for Claude Code

## Context

omaflake: clean NixOS and macOS VMs on Apple silicon (menu bar app, CLI `omaf`, runner
`omaf-runner`). Free software under **AGPL-3.0-or-later** (SPDX header in every own source file; the recipes in
`omaflake/recipes` stay MIT), GitHub organization `omaflake`, repository `omaflake`. A project of falkOSS (Falcon Lake
Open Source Software gUG, in formation).
Design and milestones: **docs/spec.md**; findings and checklists: **docs/plan.md** – read before
any work and keep them up to date.

## Working rules

- **Language:** everything in this repository is English (code, comments, docs, UI, commits).
  Talk to the maintainer in German.
- **Licence header:** every new own source file (Swift, Rust, Nix, shell, Python) starts with
  `SPDX-License-Identifier: AGPL-3.0-or-later` (after a shebang or `swift-tools-version` line); recipes are MIT and
  live in `omaflake/recipes`.
- **Public from day one:** no personal data in tracked files – no names, initials, user names,
  home paths, SSH keys, hostnames. Personal settings belong in `nix/local/`.
- **Repositories:** app work happens only in this checkout (`omaflake/omaflake`). The recipes live in
  `omaflake/recipes` (checked out next to this one as `../recipes`; its own CLAUDE.md) – this repository holds none
  (plan B107). The website is a separate repo, `omaflake/website` (static GitHub Pages site), as `../website`.
- **Developing with recipes:** the data folder's config.json has the local checkout `../recipes` as its recipe
  source (`omaf sources add <path> --first`), not the remote omaflake/recipes – a recipe changed there is built as
  it is on disk. Change a recipe in `../recipes`, run its `tools/catalog.py`, commit there.
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
- **After every change:** `swift build` and `swift test`; after runner changes `scripts/sign-runner.sh`;
  after Nix changes `nix flake check`, `nix flake check "path:$PWD/nix"` and, with the recipes,
  `nix flake check "path:$PWD/nix" --override-input omaflake-recipes "path:$PWD/../recipes/recipes"`. Commit only
  when green.
- **Small steps:** one milestone (spec §7) at a time; check the acceptance criterion explicitly and
  tick it in the docs. Findings get ids `B…` (NixOS/general) and `BM…` (macOS).
- **No secrets** in the repository (passwords, tokens, keys). Developer-local files (e.g. tokens for tooling)
  belong in `~/.omaflake/developer/`, which omaflake never touches.

## Layout

```
Package.swift              OmaflakeCore (library), omaf (CLI), omaf-runner, OmaflakeApp
Sources/OmaflakeCore/      paths, stores, QEMU command, QMP, macOS VM bundle, runner client
Sources/omaf/              CLI
Sources/omaf-runner/       macOS guests on Virtualization.framework (needs entitlement)
Sources/OmaflakeApp/       SwiftUI menu bar app
App/                       Info.plist, runner entitlements
scripts/                   bundle-app.sh, sign-runner.sh, bootprobe.sh
qemu/                      QEMU build with virglrenderer + ANGLE, patches
nix/modules/vm-guest.nix   guest module (root flake output nixosModules.vm-guest)
docs/                      spec, plan, cli, app, macos-base, linux-builder
brand/                     omaflake brand set (icons, favicon, banner, Sources/OmaflakeCore/BrandAssets.swift);
                           edit brand/generate.py, not exports
nix/                       image flake: builds images from the recipes of the sources (input omaflake-recipes),
                           guest modules, the omaflake agent, Linux builder host; use as path:, shipped in the app
nix/local.example/         template for personal settings → nix/local/ (git-ignored)
```

## Next step

State and order: **docs/plan.md §8** (2026-10-18). Everything that needs no Developer ID is implemented; what is left
are the maintainer's checks and decisions listed there, then M12 (signing, notarization, helper, macOS camera
extension, self-update, public release). Development base `mac-base-26A434`; develop with the `lab` recipes
(developer mode in the app).
Workflow for macOS settings without a nix-darwin option: set them by hand in a clone, diff
`defaults export` / container plists against a snapshot, then declare them (plan BM22).
