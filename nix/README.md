# Image flake

omaflake's Nix side: the flake that builds VM images from recipes, the **platforms** every image is built on (guest
hardware, the omaflake agent, user, SSH, locale, the look, home-manager – NixOS and macOS) and the Linux builder host.
It holds **no recipes and no modules** – they come from catalogs (spec §11.3, plan B142):
[omaflake/catalog](https://github.com/omaflake/catalog) by default, other GitHub repositories (also private ones),
HTTPS folders, or a local folder. The app ships a copy of this folder (`omaflake.app/Contents/Resources/nix`) and uses
it when no other flake path is set; in a checkout, point omaflake at this folder (`omaf config --flake "$PWD/nix"`).

Two inputs are replaced by omaflake on every `nix` call (`--override-input`, without writing the lock file):

| Input | Default here | What omaflake passes |
|---|---|---|
| `omaflake-guest` | `guest/` – neutral values (user `guest`, no keys) | **who you are and where the guest lives**: the image's credential (user, full name, e-mail), keys, its locale (language, time zone, one to three keyboards) and the look's data; one folder per build choice (plan B137, B141); the macOS guest user is always the one of your macOS base |
| `omaflake-catalog` | `no-catalog/` – empty, so the flake evaluates on its own | the recipes of the sources with what they are made of: a tree per recipe laid out like a catalog – `recipes/`, the `modules/` and `keymaps/` it uses in a version it allows – and `recipes.json` naming them (spec §11.11, plan B142). A catalog checkout works as it is |

A third input is replaced only in development: `omaflake` (default `github:omaflake/omaflake`, its `nixpkgs` follows
this flake's) gives modules `pkgs.omaflake` – the Linux app `omaflake` and the CLI `omaf` (the root flake's
`omaflake-gui`), used by the module `omaflake-dev` (plan B123). When this folder is the `nix/` folder of an omaflake
checkout (the folder above it has the root `flake.nix` and `crates/`), omaflake passes `--override-input omaflake
<checkout>`, so the lab gets the code as it is on disk; the app's bundled copy keeps the locked revision. Nix fetches
the input only when a module uses `pkgs.omaflake`.

## Recipes, modules, key bindings (plan B142)

A **recipe** is data: `recipes/<author>/<platform>/<name>/recipe.toml` (schema recipe/v2) names its platform (`nixos`
or `macos`), its modules, a suggested look, key binding sets, what a VM needs at least and how it runs. Staged by
omaflake its ID is `<author>.<platform>.<name>@<words>` (the words spell its content hash) – the attribute names of
`images` and `darwinConfigurations`; a catalog checkout gives `<author>.<platform>.<name>`.

A **module** is Nix: `modules/<author>/<platform>/<name>/default.nix` with `module.toml` (schema module/v2) – its
version, the capabilities it provides and needs, the modules it enhances, and its actions with default keys. The flake
imports the recipe's modules in its order and hands each an argument `omaflake`:

| `omaflake.…` | |
|---|---|
| `id` | the module's reference, `<author>/<name>` |
| `keys` | its actions' keys: module.toml's defaults, overridden by the recipe's key binding sets in order (a later one wins) |
| `allKeys` | every module's keys in the recipe (`allKeys."omaflake/zellij".session`) |
| `has "<author>/<name>"` | whether the recipe has that module – for what a module enhances (known from the recipe, so it can choose which options to set: `lib.optionalAttrs (omaflake.has …)`) |
| `keyLib` | `lib/keys.nix`: a key in the catalog's notation (`Super+Shift+Return`, `Ctrl+g`, `Space f f`; `Digit`, `Arrow`) for Hyprland, Zellij, WezTerm, Neovim and Emacs, and `expand` for the placeholders |
| `recipe` | `{ name; author; kind; modules; }` |

The platform declares the few options modules share: NixOS `omaflake.terminal` (`open`, `run` – set by the terminal
module) and `omaflake.launchers` (keys that start a program anywhere on the desktop – modules add them, the desktop
binds them); macOS `omaflake.homeFiles` and `omaflake.homeLinks` (written once into the user's home folder, then
theirs). The host name is the recipe's name. **Key bindings** are data: `keymaps/<author>/<name>/keymap.toml`
(`[keys."<author>/<name>"]` action → key).

## By hand

```sh
cd nix
C=../../catalog                             # a checkout of omaflake/catalog, or any folder laid out like one
nix eval --json "path:$PWD#lib.recipeDefaults" --override-input omaflake-catalog "path:$C"
nix build "path:$PWD#images.aarch64-linux.\"omaflake.nixos.neovim-minimal\"" --override-input omaflake-catalog "path:$C"
nix flake check "path:$PWD"                  # without recipes
nix flake check "path:$PWD" --override-input omaflake-catalog "path:$C"
# a recipe with pkgs.omaflake (the module omaflake-dev), from this checkout:
nix eval --raw "path:$PWD#nixosConfigurations.\"vm-aarch64-linux-omaflake.nixos.lab\".config.system.build.toplevel.drvPath" \
  --override-input omaflake-catalog "path:$C" --override-input omaflake "$PWD/.."
```

## Developer extras: `local/` (optional, git-ignored)

```sh
cd nix
cp -R local.example local          # local/ is git-ignored – nothing in it is ever committed
```

| In `local/` | Effect |
|---|---|
| `settings.nix` | optional: overrides single values omaflake passes (user, full name, email, time zone, locale, keyboard, `macUser`, `look`) – a `macUser` that differs from the base's user is flagged by the Doctor |
| `keys/*.pub` | additional public SSH keys for the guest user |
| `nixos.nix` | optional NixOS module added to every NixOS image |
| `home.nix` | optional home-manager module for the guest user |
| `darwin.nix` | optional additions to `darwinConfigurations.linux-builder` |
| `darwin-guest.nix` | optional nix-darwin module added to every macOS image |

`local.example/` has a commented example of each (`*.example` files: rename to use them). Your own recipes and
modules do not go here: put them in a folder laid out like a catalog (or a fork of omaflake/catalog) and add it as a
source – or put recipes together in the app (My Recipes).

## Why `path:`

Nix flakes in a git checkout only see tracked files, so a plain `.#…` reference would ignore `local/`. omaflake uses
the folder as a `path:` flake when it has no `.git` of its own – this folder is part of the omaflake checkout.

## Files

| | |
|---|---|
| `flake.nix` | outputs `images.<system>.<id>`, `lib.recipeDefaults`, `nixosConfigurations.vm-<system>-<id>`, `darwinConfigurations.{linux-builder,linux-builder-bootstrap,<id>}` |
| `guest/`, `no-catalog/` | the two inputs' defaults (above) |
| `modules/vm-guest.nix` | guest hardware for omaflake: UEFI, virtio, labels, 9p share, the omaflake agent (also root output `nixosModules.vm-guest`) |
| `modules/base.nix` | NixOS platform: user (zsh as login shell), locale and keyboards, SSH, Nix |
| `modules/platform.nix`, `modules/darwin-platform.nix` | the options modules share and the host name (above) |
| `modules/darwin-guest.nix` | macOS platform: primary user, language/region/keyboards, key-only SSH, shutdown without password, widgets/sleep/updates off, the clipboard helper |
| `modules/look.nix` | the look (`omaflake.look`, below): its files under `/etc/omaflake/look/`, `omaflake.lookColors`, `lookMacos`, `lookParts` for modules – NixOS and macOS guests |
| `looks/` | `adapter.nix` turns a look's data (look.toml, schema look/v1) into each program's part |
| `lib/keys.nix` | the key notations (above) |
| `home/default.nix` | home-manager, the platform's part: the user and git's identity from the credential; the rest comes from the modules |
| `agent/` | the omaflake agent (Rust, spec §11.9) |
| `hosts/linux-builder.nix` | nix-darwin host with a Linux builder ([docs/linux-builder.md](../docs/linux-builder.md)) |

## Looks

No look is built in (plan B141): omaflake hands the chosen installed look's data over (`settings.look`); without one
every program keeps its own colours. `modules/look.nix` puts one file per program under `/etc/omaflake/look/`
(`wezterm.lua`, `nvim.lua`, `emacs.el`, `zellij/`), which the modules' configurations load, and offers the look's parts
to modules: `omaflake.lookColors` (the desktop's colours as "RRGGBB", null for a plain look), `omaflake.lookMacos` (the
macOS system appearance) and `omaflake.lookParts` (everything `looks/adapter.nix` makes). A changed look comes with
the next build of an image; the user's own files stay.

## Keys

Every key omaflake binds starts with SUPER (the Mac's Cmd) or CTRL, optionally with SHIFT; plain keys only inside a
mode. An editor's own keys (module.toml `own_keys`) stay as they are. The keys of omaflake's modules are in
omaflake/catalog (`keymaps/omaflake/default/keymap.toml`); `omaf recipes show <recipe>` lists a recipe's keys, and
tools/catalog.py and omaflake check that none overlap (the same context and mode; a desktop-wide one with any key
without a mode; a program's key with one of a program that runs inside it).
