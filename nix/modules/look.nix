# SPDX-License-Identifier: AGPL-3.0-or-later
# The look of a guest (plan B135; data since B141) – NixOS and macOS: an installed look's data (look.toml as read),
# which omaflake hands over at the build (`settings.look`); none: every program keeps its own colours. looks/adapter.nix
# makes each program's part of it. Here: the per-program files under
# /etc/omaflake/look/, which the guest's own configuration loads – the modules' on NixOS, the files a macOS module writes once
# into the home folder. A changed look comes with the rebuilt image; the user's files stay.
#
#   /etc/omaflake/look/name           the look's name
#   /etc/omaflake/look/wezterm.lua    returns { config = <merged over WezTerm's>, zellij = <options for a new session> }
#   /etc/omaflake/look/zellij/        Zellij themes that are not built in (theme_dir)
#   /etc/omaflake/look/nvim.lua       sets Neovim's colours (theme plugins in nvim/pack/look/opt), returns lualine's theme
#   /etc/omaflake/look/emacs.el       loads Emacs' theme (its packages: home/emacs.nix)
{ config, lib, pkgs, settings, ... }:

let
  looks = import ../looks { inherit pkgs lib; };
  name = looks.nameOf config.omaflake.look;
  look = looks.get config.omaflake.look;
  dir = "/etc/omaflake/look";
in
{
  options.omaflake.look = lib.mkOption {
    type = lib.types.nullOr (lib.types.attrsOf lib.types.anything);
    default = null;
    description = "The guest's look: an installed look's data (look.toml, schema look/v1) – omaflake sets it from the build's choice; null: every program keeps its own colours.";
  };

  # For modules that colour a program of their own (zathura): the look's desktop colours as
  # "RRGGBB" (base, surface, text, subtext, muted, accent, urgent, …, `dark`), null for vanilla
  options.omaflake.lookColors = lib.mkOption {
    type = lib.types.nullOr (lib.types.attrsOf lib.types.anything);
    readOnly = true;
    default = look.desktop;
    defaultText = lib.literalMD "the `desktop` colours of the look (nix/looks/)";
    description = "The guest look's colours, for modules (read only).";
  };

  # Each program's part of the look (looks/default.nix: desktop, macos, wezterm, zellij, nvim, emacs), for modules
  # (plan B142)
  options.omaflake.lookParts = lib.mkOption {
    type = lib.types.attrsOf lib.types.anything;
    readOnly = true;
    default = look;
    defaultText = lib.literalMD "what nix/looks/ makes of the look";
    description = "Each program's part of the guest look, for modules (read only).";
  };

  # The macOS desktop's part of the look (plan B141): { dark; accent; icons; folders; wallpaper }, null for none
  options.omaflake.lookMacos = lib.mkOption {
    type = lib.types.nullOr (lib.types.attrsOf lib.types.anything);
    readOnly = true;
    default = look.macos;
    defaultText = lib.literalMD "the `macos` part of the look (nix/looks/adapter.nix)";
    description = "The guest look's macOS system settings, for the macOS desktop module (read only).";
  };

  config = {
    # Priority 40: above a module's plain value (100) and even its lib.mkForce (50)
    omaflake.look = lib.mkIf ((settings.look or null) != null) (lib.mkOverride 40 settings.look);

    environment.etc = {
      "omaflake/look/name".text = name + "\n";
      "omaflake/look/wezterm.lua".text = ''
        -- The guest's look (omaflake, plan B135): ${name}. Generated with the image – a rebuild changes it.
        return {
          config = ${lib.trim look.wezterm},
          zellij = { "--theme-dir", "${dir}/zellij", "--theme", "${look.zellij.theme}" },
        }
      '';
      "omaflake/look/nvim.lua".text = ''
        -- The guest's look (omaflake, plan B135): ${name}. Generated with the image – a rebuild changes it.
        -- Sets the colours and returns lualine's theme.
        vim.opt.packpath:prepend("${dir}/nvim")
        ${look.nvim.lua}
      '';
      "omaflake/look/nvim".source = pkgs.linkFarm "omaflake-look-nvim"
        (map (p: { name = "pack/look/opt/${p.name}"; path = p.plugin; }) look.nvim.plugins);
      # Zellij refuses a theme_dir that does not exist; it reads only *.kdl there
      "omaflake/look/zellij/README".text = "Zellij themes of the guest's look (omaflake, plan B135).\n";
      "omaflake/look/emacs.el".text = ''
        ;;; emacs.el --- the guest's look (omaflake, plan B135): ${name}  -*- lexical-binding: t -*-
        ;;; Generated with the image – a rebuild changes it.
        ${look.emacs.elisp}
      '';
    } // lib.optionalAttrs (look.zellij.kdl != null) {
      "omaflake/look/zellij/${look.zellij.theme}.kdl".text = look.zellij.kdl;
    };
  };
}
