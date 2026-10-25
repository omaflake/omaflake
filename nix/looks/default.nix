# SPDX-License-Identifier: AGPL-3.0-or-later
# The looks of omaflake's guests (plan B135; data since B141): a look is look.toml data (schema look/v1), installed
# from a source's catalog; omaflake hands the chosen look's data over at the build (`settings.look`). Nothing is built
# in – adapter.nix turns a look into what each program gets; modules/look.nix puts the per-program files under
# /etc/omaflake/look/, home/look.nix colours the NixOS desktop. Without a look every program keeps its own colours.
#
# What adapter.nix makes of a look – an attribute set:
#   desktop  NixOS only – Hyprland, Waybar, Fuzzel, Mako, the prompt, GTK and the pointer; colours as "RRGGBB";
#            null: each program's own colours
#   wezterm  Lua table constructor merged over WezTerm's configuration
#   zellij   { theme = name; kdl = the theme's KDL, null for a theme built into Zellij }
#   nvim     { plugins = [ { name; plugin; } ] (loaded with :packadd); lua = sets the colours, returns lualine's theme }
#   emacs    { packages = epkgs: [ … ]; elisp = loads the theme }
{ pkgs, lib }:

let
  adapt = import ./adapter.nix { inherit pkgs lib; };
in
{
  # A look's data (look.toml as read); null: no look – every program its own colours
  get = look: adapt (if look == null then { plain = true; } else look);
  # Its name for /etc/omaflake/look/name
  nameOf = look: if look == null then "none" else look.title or "look";
}
