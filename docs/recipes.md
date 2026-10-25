---
title: Writing recipes
nav_order: 2
permalink: /recipes/
---

# Writing recipes

A recipe is put together from building blocks (plan B141, B142) – it is data, never code:

| Block | What | Where |
|---|---|---|
| **Platform** | what the guest is: `nixos`, or `macos` on a macOS base – omaflake's own part (user, locale, SSH, the agent) | omaflake's image flake |
| **Module** | a piece of the guest: a desktop, a terminal, an editor, LaTeX … – Nix, reviewed in its catalog | `modules/<author>/<platform>/<name>/` |
| **Look** | colours and a few choices for every program | `looks/<author>/<name>/look.toml` |
| **Key bindings** | keys for the modules' actions | `keymaps/<author>/<name>/keymap.toml` |
| **Recipe** | a platform, modules, a suggested look, key bindings, what a VM needs and how it runs | `recipes/<author>/<platform>/<name>/recipe.toml` |

Your own credential (who you are), locale (language, time zone, keyboards) and profile (CPU, RAM) are never part of a
recipe: you choose them when you build an image or create a VM.

```toml
"$schema" = "https://omaflake.dev/schema/recipe/v2.json"
author = "your-github-name"
license = "MIT"
description = "Rust development: Neovim with the Rust language server, Zellij, the Hyprland desktop."
tags = ["rust", "neovim"]
platform = "nixos"
systems = ["aarch64-linux", "x86_64-linux"]

modules = [
  "omaflake/hyprland",
  "omaflake/shell",
  "omaflake/wezterm",
  "omaflake/zellij",
  { id = "omaflake/neovim", min = "1.0.0" },
  "your-github-name/rust",
]
look = "omaflake/omaflake"
keymaps = ["omaflake/default"]

[requirements]          # the least a VM from it needs – a smaller profile gets a warning
cpus = 4
memory_gb = 8
disk_gb = 40

[vm]                    # how a VM from it runs
display = "gpu"
network = "shared"
share = true
ephemeral = false
```

A reference names the newest version (`"omaflake/neovim"`), an exact one (`{ id = …, version = "1.2.0" }`) or a
minimum and/or maximum (`{ id = …, min = "1.0.0", max = "1.9.0" }`). References point to the catalogs of your sources
and what is installed – the official catalog always as the fallback. You can also put a recipe together in the app:
**New Recipe …** in the Recipes tab (My Recipes).

The full format – every field of recipes, modules and key bindings, the module arguments, the capabilities, what a
module must not do, and how to test – is in the catalog's
[CONTRIBUTING.md](https://github.com/omaflake/catalog/blob/main/CONTRIBUTING.md). The machine-readable form is in the
[file schemas]({{ '/schema/' | relative_url }}).

## Architectures

`systems` lists the architectures a recipe is built and tested for, named as Nix names systems:

| Platform | Systems |
|---|---|
| `nixos` | `aarch64-linux` (a Mac with Apple silicon), `x86_64-linux` |
| `macos` | `aarch64-darwin` |

omaflake shows a recipe only where it runs (the app's filter "Other Architectures" and `omaf recipes --all-systems`
show the others) and builds it only for a system it lists. Images and VMs record their `system`; a VM of another
architecture is not created, started or imported.

## Sources

omaflake ships no recipes, modules or looks. It reads them from **sources**: by default
[omaflake/catalog](https://github.com/omaflake/catalog); other GitHub repositories (also private ones), any HTTPS
folder with a `catalog.json`, or a local folder, whose recipes and modules are used as they are on disk – the way to
develop one:

```sh
omaf sources add ~/src/my-catalog --first
omaf recipes show <author>/<platform>/<name>
omaf build <author>/<platform>/<name>
```

A source of your own starts as a fork of omaflake/catalog: its
[FORKING.md](https://github.com/omaflake/catalog/blob/main/FORKING.md) explains `tools/catalog.py` and the catalog.
