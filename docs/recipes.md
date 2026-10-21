---
title: Writing recipes
nav_order: 2
permalink: /recipes/
---

# Writing recipes

A recipe is not a flake but a module with its metadata: a folder `<author>/<kind>/<name>/` with

- `recipe.toml` – what the recipe is (author, license, description, tags), what it can be built for (`systems`) and
  how a VM from it runs (CPU, memory, disk, network, display, …);
- `default.nix` – a NixOS module (kind `nixos`) or a nix-darwin module (kind `macos`). omaflake adds its guest modules
  around it – guest hardware, the omaflake agent, the user, SSH keys, locale – so a recipe only says what makes it
  different;
- optionally `<system>.nix` – its variant for one architecture, e.g. `x86_64-linux.nix`.

```toml
"$schema" = "https://omaflake.dev/schema/recipe/v1.json"
systems = ["aarch64-linux"]
author = "your-github-name"
license = "MIT"
description = "Rust development: rustup, rust-analyzer and neovim with the Rust LSP."
tags = ["rust", "neovim"]
cpus = 4
memory_gb = 8
disk_gb = 40
network = "shared"
share = true
ephemeral = false
```

The full format – every field, the module arguments, what a recipe must not do, and how to test it – is in the recipe
repository's [CONTRIBUTING.md](https://github.com/omaflake/recipes/blob/main/CONTRIBUTING.md). The machine-readable
form is the [recipe schema]({{ '/schema/' | relative_url }}).

## Architectures

`systems` lists the architectures a recipe is built and tested for, named as Nix names systems:

| Kind | Systems |
|---|---|
| `nixos` | `aarch64-linux` (a Mac with Apple silicon), `x86_64-linux` |
| `macos` | `aarch64-darwin` |

A recipe without `systems` (written before omaflake 0.2.0) is built for Apple silicon. omaflake shows a recipe only
where it runs (the app's filter "Other Architectures" and `omaf recipes --all-systems`
show the others) and builds it only for a system it lists. Images and VMs record their `system`; a VM of another
architecture is not created, started or imported.

## Sources

omaflake ships no recipes. It reads them from **recipe sources**: by default
[omaflake/recipes](https://github.com/omaflake/recipes); other GitHub repositories (also private ones), any HTTPS
folder with a `catalog.json`, or a local folder, whose recipes are used as they are on disk – the way to develop one:

```sh
omaf sources add ~/src/my-recipes --first
omaf build <author>/<kind>/<name>
```

A source of your own starts as a fork of omaflake/recipes: its
[FORKING.md](https://github.com/omaflake/recipes/blob/main/FORKING.md) explains `tools/catalog.py` and the catalog.
