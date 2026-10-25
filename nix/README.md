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
| `omaflake-guest` | `guest/` – neutral values (user `guest`, no keys) | **who you are**: the image's credential (user, full name, e-mail; else your Mac's), keys, locale, keyboard, time zone (overridable in the app's settings or with `omaf config --guest-…`) and the image's look; one folder per credential and look (plan B137); the macOS guest user is always the one of your macOS base |
| `omaflake-recipes` | `no-recipes/` – empty, so the flake evaluates on its own | the recipes of the sources (enabled local folders as they are on disk, downloaded and imported ones), each in a tree of its own with the recipes it requires, and `recipes.json` naming them (spec §11.11) |

A third input is replaced only in development: `omaflake` (default `github:omaflake/omaflake`, its `nixpkgs` follows
this flake's) gives recipes `pkgs.omaflake` – the Linux app `omaflake` and the CLI `omaf` (the root flake's
`omaflake-gui`), used by `nixos/lab` (plan B123). When this folder is the `nix/` folder of an omaflake checkout (the
folder above it has the root `flake.nix` and `crates/`), omaflake passes `--override-input omaflake <checkout>`, so the
lab gets the code as it is on disk (the tracked files with their changes – not `target/`); the app's bundled copy keeps
the locked revision. Nix fetches the input only when a recipe uses `pkgs.omaflake`.

A recipe is a folder `<author>/<kind>/<name>/` with `recipe.toml` (how a VM from it runs) and `default.nix` (a NixOS
or nix-darwin module); its fields and rules are in omaflake/recipes' CONTRIBUTING.md. Staged by omaflake its ID is
`<author>.<kind>.<name>@<words>` (the words spell its content hash) – the attribute names of `images` and
`darwinConfigurations`; a plain folder of recipe folders (below) gives `<author>.<kind>.<name>`.

## By hand

```sh
cd nix
R=../../recipes/recipes                      # a checkout of omaflake/recipes, or any folder of recipe folders
nix eval --json "path:$PWD#lib.recipeDefaults" --override-input omaflake-recipes "path:$R"
nix build "path:$PWD#images.aarch64-linux.\"omaflake.nixos.neovim-minimal\"" --override-input omaflake-recipes "path:$R"
nix flake check "path:$PWD"                  # without recipes
# a recipe with pkgs.omaflake, from this checkout:
nix eval --raw "path:$PWD#nixosConfigurations.\"vm-aarch64-linux-omaflake.nixos.lab\".config.system.build.toplevel.drvPath" \
  --override-input omaflake-recipes "path:$R" --override-input omaflake "$PWD/.."
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
| `modules/look.nix` | the look (`omaflake.look`, below): its files under `/etc/omaflake/look/` – NixOS and macOS guests |
| `looks/` | `adapter.nix` turns a look's data (look.toml, schema look/v1, plan B141) into the desktop's, WezTerm's, Zellij's, Neovim's and Emacs' part. No look is built in: omaflake hands the chosen installed look's data over (`settings.look`); without one every program keeps its own colours |
| `home/default.nix` | home-manager: Hyprland config, WezTerm, Waybar, Fuzzel, Mako, zsh – layout and keys; colours: `home/look.nix` |
| `home/neovim.nix`, `home/emacs.nix` | the editor lines (`omaflake.editor`); Zellij with the neovim line, its keys in `home/zellij-unlock-first.kdl` |
| `agent/` | the omaflake agent (Rust, spec §11.9) |
| `hosts/linux-builder.nix` | nix-darwin host with a Linux builder ([docs/linux-builder.md](../docs/linux-builder.md)) |

## Looks

Every guest has a look (plan B135): `omaflake` (default – black, high contrast, red and cyan only as fine accents),
`catppuccin` (Catppuccin Mocha), `rose-pine-dawn` (light and mild) or `vanilla` (each program's own colours). A recipe
picks one like its editor line (`omaflake.look = "catppuccin";`); the user's choice in the app's settings or
`omaf config --guest-look …` wins over it. NixOS guests take the whole desktop from it (Hyprland, Waybar, Fuzzel,
Mako, the prompt, GTK, the pointer), every guest its terminal's and editors' colours: `modules/look.nix` puts one file
per program under `/etc/omaflake/look/` (`wezterm.lua`, `nvim.lua`, `emacs.el`, `zellij/`), which home-manager's
configuration loads – and the files a macOS recipe writes once into the home folder (`macos/research`). A changed
look comes with the next build of an image; the user's own files stay. Recipes that colour a program of their own
read the colours from `config.omaflake.lookColors` (null for vanilla).

## Desktop keys

Every key omaflake binds starts with SUPER (the Mac's Cmd) or CTRL, optionally with SHIFT; plain keys only inside a
mode (SUPER + R below, Zellij's modes after Ctrl + G). The editors' own keys stay as they are.

| Keys | Action |
|---|---|
| SUPER + Enter | terminal (WezTerm, a plain shell) |
| SUPER + SHIFT + Enter | terminal in the Zellij session `main` (neovim line) |
| SUPER + D | launcher (fuzzel) |
| SUPER + Q | close window |
| SUPER + F / V | fullscreen / floating |
| SUPER + arrows | focus the window that way |
| SUPER + SHIFT + arrows | move the window that way |
| SUPER + R, then arrows or h j k l | resize mode: right/down grow, left/up shrink (held down: repeats); Esc or Enter leaves |
| SUPER + 1–9 | switch workspace |
| SUPER + SHIFT + 1–9 | move window to workspace |
| SUPER + SHIFT + E | quit Hyprland |
| SUPER + SHIFT + S | area screenshot to clipboard |

SUPER is the Cmd key. Neovim (`nvim`, also `vi`/`vim`): leader = space, `␣ff` files, `␣fg` text,
`␣fb` buffers, `␣e` file tree.

Zellij (the neovim line): SUPER + SHIFT + Enter opens a terminal in the session `main` (closing the window keeps it);
`zj` attaches to it from any shell, also over SSH. A plain terminal (SUPER + Enter) stays free for anything else. Its keys use Zellij's "Unlock First" preset (`home/zellij-unlock-first.kdl`), so Neovim
keeps its Ctrl keys:

| Keys | Action |
|---|---|
| Ctrl + G | unlock (then once more: lock again; Esc or Enter too) |
| then P / T / R / M / S / O | pane / tab / resize / move / scroll / session mode – h j k l or arrows inside |
| then P, then N / W | new pane / floating panes |
| Ctrl + Q (unlocked) | quit Zellij |

The preset's Alt shortcuts are left out: Option is level 3 on the guests' Mac layout, so no Alt reaches Zellij.
