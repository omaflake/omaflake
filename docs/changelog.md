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

### Changed

- **The menu bar app is a client of omaflake's new Rust core**: it starts `omaf serve` (bundled next to the app) and
  gets VMs, images, recipes, builds and the Doctor's checks from it (docs/api.md); it no longer reads the data folder
  every 2 s itself. Everything you see and do in the app stays the same.
- **App and core agree on every message between them**, pinned by shared contract tests (docs/api.md §9): private
  recipe sources show again whether their token is in the Keychain, the catalog shows a recipe's files, an image tells
  whether its recipe changed since, and activity messages for ended jobs and VMs come once, in the app's language.
  QEMU's Dock icon is written by the core when it starts a VM window; the app writes nothing into the data folder.

### Fixed

- **omaflake's QEMU builds again** (`nix build github:omaflake/omaflake#qemu`): the libepoxy EGL 1.5 patch is now part
  of omaflake instead of a download from its Homebrew tap, whose release tag was moved – the hash check stopped every
  fresh build. The patch itself is unchanged.

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

[Unreleased]: https://github.com/omaflake/omaflake/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/omaflake/omaflake/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/omaflake/omaflake/releases/tag/v0.1.0
