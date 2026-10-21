# Image flake

omaflake's Nix side: the flake that builds VM images from recipes, the guest modules every image gets (guest
hardware, the omaflake agent, user, SSH, locale, desktop) and the Linux builder host. It holds **no recipes** – they
come from recipe sources (spec §11.3, plan B107): [omaflake/recipes](https://github.com/omaflake/recipes) by default,
other GitHub repositories (also private ones), HTTPS folders, or a local folder. The app ships a copy of this folder
(`omaflake.app/Contents/Resources/nix`) and uses it when no other flake path is set; in a checkout, point omaflake at
this folder (`omaf config --flake "$PWD/nix"`).

Two inputs are replaced by omaflake on every `nix` call (`--override-input`, without writing the lock file):

| Input | Default here | What omaflake passes |
|---|---|---|
| `omaflake-guest` | `guest/` – neutral values (user `guest`, no keys) | **who you are**: user, full name, keys, locale, keyboard, time zone from your Mac (overridable in the app's settings or with `omaf config --guest-…`); the macOS guest user is always the one of your macOS base |
| `omaflake-recipes` | `no-recipes/` – empty, so the flake evaluates on its own | the recipes of the sources (enabled local folders as they are on disk, downloaded and imported ones), each in a tree of its own with the recipes it requires, and `recipes.json` naming them (spec §11.11) |

A recipe is a folder `<author>/<kind>/<name>/` with `recipe.toml` (how a VM from it runs) and `default.nix` (a NixOS
or nix-darwin module); its fields and rules are in omaflake/recipes' CONTRIBUTING.md. Staged by omaflake its ID is
`<author>.<kind>.<name>@<words>` (the words spell its content hash) – the attribute names of `images` and
`darwinConfigurations`; a plain folder of recipe folders (below) gives `<author>.<kind>.<name>`.

## By hand

```sh
cd nix
R=../../recipes/recipes                      # a checkout of omaflake/recipes, or any folder of recipe folders
nix eval --json "path:$PWD#lib.recipeDefaults" --override-input omaflake-recipes "path:$R"
nix build "path:$PWD#images.aarch64-linux.\"omaflake.nixos.minimal\"" --override-input omaflake-recipes "path:$R"
nix flake check "path:$PWD"                  # without recipes
```

## Developer extras: `local/` (optional, git-ignored)

```sh
cd nix
cp -R local.example local          # local/ is git-ignored – nothing in it is ever committed
```

| In `local/` | Effect |
|---|---|
| `settings.nix` | optional: overrides single values omaflake passes (user, full name, email, time zone, locale, keyboard, `macUser`) – a `macUser` that differs from the base's user is flagged by the Doctor |
| `keys/*.pub` | additional public SSH keys for the guest user |
| `nixos.nix` | optional NixOS module added to every NixOS image |
| `home.nix` | optional home-manager module for the guest user |
| `darwin.nix` | optional additions to `darwinConfigurations.linux-builder` |
| `darwin-guest.nix` | optional nix-darwin module added to every macOS image |

`local.example/` has a commented example of each (`*.example` files: rename to use them). Your own recipes do not go
here: put them in a folder (or a checkout of a fork of omaflake/recipes) and add it as a recipe source.

## Why `path:`

Nix flakes in a git checkout only see tracked files, so a plain `.#…` reference would ignore `local/`. omaflake uses
the folder as a `path:` flake when it has no `.git` of its own – this folder is part of the omaflake checkout.

## Files

| | |
|---|---|
| `flake.nix` | outputs `images.<system>.<id>`, `lib.recipeDefaults`, `nixosConfigurations.vm-<system>-<id>`, `darwinConfigurations.{linux-builder,linux-builder-bootstrap,<id>}` |
| `guest/`, `no-recipes/` | the two inputs' defaults (above) |
| `modules/vm-guest.nix` | guest hardware for omaflake: UEFI, virtio, labels, 9p share, the omaflake agent (also root output `nixosModules.vm-guest`) |
| `modules/darwin-guest.nix` | macOS guests: primary user, language/region/keyboard, key-only SSH, shutdown without password, widgets/sleep/updates off, the clipboard helper |
| `modules/base.nix` | user, locale, SSH, Nix |
| `modules/hyprland.nix` | Hyprland, audio, fonts – every NixOS image has this desktop |
| `home/default.nix` | home-manager: Hyprland config, WezTerm, Neovim, Zellij, Waybar, zsh (Catppuccin Mocha) |
| `agent/` | the omaflake agent (Rust, spec §11.9) |
| `hosts/linux-builder.nix` | nix-darwin host with a Linux builder ([docs/linux-builder.md](../docs/linux-builder.md)) |

## Desktop keys

| Keys | Action |
|---|---|
| SUPER + Enter | terminal (WezTerm) |
| SUPER + D | launcher (fuzzel) |
| SUPER + Q | close window |
| SUPER + F / V | fullscreen / floating |
| SUPER + 1–9 | switch workspace |
| SUPER + SHIFT + 1–9 | move window to workspace |
| SUPER + SHIFT + E | quit Hyprland |
| Print | area screenshot to clipboard |

SUPER is the Cmd key. Neovim (`nvim`, also `vi`/`vim`): leader = space, `␣ff` files, `␣fg` text,
`␣fb` buffers, `␣e` file tree. Zellij starts by hand (`zellij`).
