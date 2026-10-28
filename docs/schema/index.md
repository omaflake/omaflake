---
title: File schemas
nav_order: 5
permalink: /schema/
---

# File schemas

Every JSON file omaflake writes names its JSON Schema (draft 2020-12) in `$schema` – what the file is and its version:

```json
{
  "$schema": "https://omaflake.dev/schema/image/v1.json",
  "kind": "nixos",
  "system": "aarch64-linux"
}
```

Each type counts its versions on its own. omaflake brings an older file up to date when it reads it, refuses a newer
one, and never removes a published version from this site. Editors use the URL to check and complete the files.

| Type | File | Schema |
|---|---|---|
| `image` | `image.json` in an image package | [v3](image/v3.json) |
| `instance` | `instance.json` in a VM package | [v1](instance/v1.json) |
| `settings` | `~/.omaflake/config.json` | [v5](settings/v5.json) (credentials, locales, profiles) |
| `installed` | `<data>/recipes/.installed.json` | [v1](installed/v1.json) |
| `macos-vm` | `config.json` of a macOS VM folder | [v1](macos-vm/v1.json) |
| `base` | `omaflake-base.json` in a macOS base folder – the credential of its guest user | [v1](base/v1.json) |
| `manifest` | `manifest.json` of an exported package | [v1](manifest/v1.json) |
| `catalog` | `catalog.json` of a source | [v4](catalog/v4.json) (recipes v2, modules, key bindings, looks; plan B142), [v5](catalog/v5.json) (modules v2, key bindings v2; plan B147) |
| `recipe` | `recipe.toml` of a recipe (TOML, checked as data) | [v2](recipe/v2.json) (data only, plan B142) |
| `module` | `module.toml` of a module (TOML, checked as data; plan B142) | [v1](module/v1.json), [v2](module/v2.json) (modes, after, does; plan B147) |
| `keymap` | `keymap.toml` of a key binding set (TOML, checked as data; plan B142) | [v1](keymap/v1.json), [v2](keymap/v2.json) (key lists, `{ key, after }`; plan B147) |
| `look` | `look.toml` of a look (TOML, checked as data; plan B140, B141) | [v1](look/v1.json) |
| `installed-parts` | `<data>/looks/`, `<data>/modules/`, `<data>/keymaps/` `.installed.json` – what was installed from sources | [v1](installed-parts/v1.json) |
| `tokens` | `$XDG_CONFIG_HOME/omaflake/tokens.json` – tokens of private sources on Linux without a keyring | [v1](tokens/v1.json) |

The schemas live in the repository's [docs/schema/](https://github.com/omaflake/omaflake/tree/main/docs/schema); how
omaflake reads and migrates the files is in the [design]({{ '/spec/' | relative_url }}), §5.1.
