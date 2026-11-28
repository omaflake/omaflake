# SPDX-License-Identifier: AGPL-3.0-or-later
# The looks of omaflake's guests (plan B135; data since B141): a look is look.toml data (schema look/v1), installed
# from a source's catalog; omaflake hands the chosen look's data over at the build (`settings.look`). Nothing is built
# in – adapter.nix turns a look into what each program gets; modules/look.nix puts the per-program files under
# /etc/omaflake/look/, the modules colour their programs (omaflake.lookParts). Without a look every program keeps its
# own colours.
#
# What adapter.nix makes of a look – an attribute set:
#   desktop  NixOS only – Hyprland, Waybar, Fuzzel, Mako, the prompt, GTK and the pointer; colours as "RRGGBB";
#            null: each program's own colours
#   macos    macOS only – { dark; accent; icons; folders; wallpaper } for the macOS desktop; null: macOS' own
#   wezterm  Lua table constructor merged over WezTerm's configuration
#   zellij   { theme = name; kdl = the theme's KDL, null for a theme built into Zellij }
#   nvim     { plugins = [ { name; plugin; } ] (loaded with :packadd); lua = sets the colours, returns lualine's theme }
#   emacs    { packages = epkgs: [ … ]; elisp = loads the theme }
#
# A look comes from a source, also from other people's: lib/validate.nix checks all of it against look/v1 before
# adapter.nix reads a value – a look that is not one fails the build with a message naming the field.
{ pkgs, lib }:

let
  validate = import ../lib/validate.nix { inherit lib; };
  adapt = import ./adapter.nix { inherit pkgs lib validate; };
in
{
  # A look's data (look.toml as read); null: no look – every program its own colours
  get = look: adapt (if look == null then { plain = true; } else validate.look look);
  # Its name for /etc/omaflake/look/name and for comments: one line of text
  nameOf = look: if look == null then "none" else validate.lookTitle (validate.look look);
}
