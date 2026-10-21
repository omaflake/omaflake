---
title: Contributing
nav_order: 6
permalink: /contributing/
---

# Contributing

omaflake is one Swift package with the menu bar app, the `omaf` CLI and `omaf-runner`, plus the image flake in `nix/`
and the QEMU build in `qemu/`. Recipes are not here: they live in [omaflake/recipes](https://github.com/omaflake/recipes).

## Build and test

```sh
swift build                        # everything
swift test                         # unit tests
scripts/sign-runner.sh             # after changes to omaf-runner (Virtualization.framework entitlement)
scripts/bundle-app.sh              # build/omaflake.app
```

After changes to Nix:

```sh
nix flake check
nix flake check "path:$PWD/nix"
nix flake check "path:$PWD/nix" --override-input omaflake-recipes "path:$PWD/../recipes/recipes"
```

To build images from recipes you are working on, add your checkout of omaflake/recipes as the first recipe source
(`omaf sources add <path> --first`) and point omaflake at the image flake of your checkout
(`omaf config --flake "$PWD/nix"`). Personal Nix settings for your guests go into `nix/local/` (git-ignored; template
`nix/local.example/`).

## Rules

- **English** in code, comments, documentation, UI and commits.
- **Licence header:** every new source file starts with `SPDX-License-Identifier: AGPL-3.0-or-later`.
- **No personal data** in tracked files – no names, user names, home paths, keys, host names.
- **No secrets** in the repository.
- **Verified before claimed:** check Nix options, QEMU parameters and Virtualization.framework API; mark what cannot
  be verified as `OPEN`.
- **Files are an interface:** a change to a file omaflake writes raises its version, adds a migration step and a new
  schema in `docs/schema/` ([file schemas]({{ '/schema/' | relative_url }})).
- Design and decisions are in the [design (spec)]({{ '/spec/' | relative_url }}); findings carry ids `B…` (general)
  and `BM…` (macOS).
