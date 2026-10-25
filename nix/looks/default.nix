# SPDX-License-Identifier: AGPL-3.0-or-later
# The looks of omaflake's guests (plan B135): one file per look, one source per program for NixOS and macOS guests.
# modules/look.nix puts the per-program files under /etc/omaflake/look/; home/look.nix colours the NixOS desktop.
#
# A look is an attribute set:
#   desktop  NixOS only – Hyprland, Waybar, Fuzzel, Mako, the prompt, GTK and the pointer; colours as "RRGGBB";
#            null: each program's own colours
#   wezterm  Lua table constructor merged over WezTerm's configuration
#   zellij   { theme = name; kdl = the theme's KDL, null for a theme built into Zellij }
#   nvim     { plugins = [ { name; plugin; } ] (loaded with :packadd); lua = sets the colours, returns lualine's theme }
#   emacs    { packages = epkgs: [ … ]; elisp = loads the theme }
{ pkgs, lib }:

let
  looks = {
    omaflake = import ./omaflake.nix { inherit pkgs lib; };
    catppuccin = import ./catppuccin.nix { inherit pkgs lib; };
    rose-pine-dawn = import ./rose-pine-dawn.nix { inherit pkgs lib; };
    vanilla = import ./vanilla.nix { inherit pkgs lib; };
  };
in
{
  # The names, in the order the interfaces offer them (crates/omaflake-core/src/guest_choices.rs checks them)
  names = [ "omaflake" "catppuccin" "rose-pine-dawn" "vanilla" ];
  get = name: looks.${name};
}
