# SPDX-License-Identifier: AGPL-3.0-or-later
# The looks of omaflake's guests (plan B135; as data since B141): a look is look.toml data (schema look/v1) and
# adapter.nix turns it into what each program gets. modules/look.nix puts the per-program files under
# /etc/omaflake/look/; home/look.nix colours the NixOS desktop.
#
# What adapter.nix makes of a look – an attribute set:
#   desktop  NixOS only – Hyprland, Waybar, Fuzzel, Mako, the prompt, GTK and the pointer; colours as "RRGGBB";
#            null: each program's own colours
#   wezterm  Lua table constructor merged over WezTerm's configuration
#   zellij   { theme = name; kdl = the theme's KDL, null for a theme built into Zellij }
#   nvim     { plugins = [ { name; plugin; } ] (loaded with :packadd); lua = sets the colours, returns lualine's theme }
#   emacs    { packages = epkgs: [ … ]; elisp = loads the theme }
#
# Until omaflake hands a look's data over (plan B141 step 2), the four looks are here as data and named; then they
# live in the catalog only (omaflake/catalog looks/) and nothing is built in.
{ pkgs, lib }:

let
  adapt = import ./adapter.nix { inherit pkgs lib; };
  # The names, in the order the interfaces offer them (crates/omaflake-core/src/guest_choices.rs checks them)
  names = [ "omaflake" "catppuccin" "rose-pine-dawn" "vanilla" ];
  bundled = lib.genAttrs names (name: builtins.fromTOML (builtins.readFile (./. + "/${name}.toml")));
in
{
  inherit names;
  # A look by its name (one of `names`) or its data (look.toml as read)
  get = look: adapt (if builtins.isString look then bundled.${look} else look);
  # Its name for /etc/omaflake/look/name
  nameOf = look: if builtins.isString look then look else look.title or "look";
}
