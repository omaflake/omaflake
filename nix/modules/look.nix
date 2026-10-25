# SPDX-License-Identifier: AGPL-3.0-or-later
# The look of a guest (plan B135; data since B141) – NixOS and macOS. A recipe picks one like its editor line
# (`omaflake.look = "catppuccin";`); the user's choice wins: a look's name, or its data (look.toml as read) once
# omaflake hands it over. looks/adapter.nix makes each program's part of it. Here: the per-program files under
# /etc/omaflake/look/, which the guest's own configuration loads – home-manager's on NixOS (home/look.nix, home/neovim.nix, home/emacs.nix), the files a macOS
# recipe writes once into the home folder. A changed look comes with the rebuilt image; the user's files stay.
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
    type = lib.types.either (lib.types.enum looks.names) (lib.types.attrsOf lib.types.anything);
    default = "omaflake";
    description = "The guest's look: a name – omaflake (black, high contrast), catppuccin (Catppuccin Mocha), rose-pine-dawn (light and mild) or vanilla (each program's own colours) – or a look's data (look.toml, schema look/v1).";
  };

  # For recipes that colour a program of their own (neovim-research: zathura): the look's desktop colours as
  # "RRGGBB" (base, surface, text, subtext, muted, accent, urgent, …, `dark`), null for vanilla
  options.omaflake.lookColors = lib.mkOption {
    type = lib.types.nullOr (lib.types.attrsOf lib.types.anything);
    readOnly = true;
    default = look.desktop;
    defaultText = lib.literalMD "the `desktop` colours of the look (nix/looks/)";
    description = "The guest look's colours, for recipes (read only).";
  };

  config = {
    # Priority 40: above a recipe's plain value (100) and even its lib.mkForce (50)
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
