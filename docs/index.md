---
title: Home
nav_order: 1
permalink: /
---

<img src="{{ '/assets/omaflake-mark-on-dark.svg' | relative_url }}" alt="" width="96" height="96">

# omaflake for developers

omaflake builds clean, disposable NixOS and macOS virtual machines on Apple silicon from **recipes** declared in Nix,
and runs them from a menu bar app or the `omaf` CLI. Images never change: *reset* throws a VM's state away and starts
over from its image.

This site is for people who write recipes, build on omaflake's files or work on omaflake itself. Downloading and using
the app is on [omaflake.app](https://omaflake.app).
{: .fs-5 .fw-300 }

| Page | What it covers |
|---|---|
| [Writing recipes]({{ '/recipes/' | relative_url }}) | what a recipe is, its `recipe.toml`, architectures and variants, recipe sources |
| [CLI reference]({{ '/cli/' | relative_url }}) | every `omaf` command |
| [Design (spec)]({{ '/spec/' | relative_url }}) | how omaflake works: data layout, components, guests, recipe sources |
| [File schemas]({{ '/schema/' | relative_url }}) | JSON Schemas of every file omaflake writes and reads |
| [Contributing]({{ '/contributing/' | relative_url }}) | build, test and the rules of the repository |

**Status:** early development (alpha, unsigned – see the [changelog]({{ '/changelog/' | relative_url }})). Source code:
[omaflake/omaflake](https://github.com/omaflake/omaflake) (AGPL-3.0-or-later); official recipes:
[omaflake/recipes](https://github.com/omaflake/recipes) (MIT).
