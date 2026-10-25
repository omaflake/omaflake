# SPDX-License-Identifier: AGPL-3.0-or-later
# Catppuccin Mocha (plan B135) – the look of the NixOS guests before the omaflake look, unchanged
{ pkgs, lib }:

let
  c = {
    base = "1e1e2e"; mantle = "181825"; crust = "11111b";
    surface0 = "313244"; surface1 = "45475a"; overlay0 = "6c7086";
    text = "cdd6f4"; subtext = "a6adc8";
    blue = "89b4fa"; mauve = "cba6f7"; green = "a6e3a1";
    red = "f38ba8"; peach = "fab387"; yellow = "f9e2af";
  };
in
{
  desktop = {
    dark = true;
    base = c.base;
    surface = c.base;
    selection = c.surface1;
    text = c.text;
    subtext = c.subtext;
    muted = c.overlay0;
    accent = c.mauve;
    clock = c.blue;
    urgent = c.red;
    border = [ c.mauve c.blue ];
    inactiveBorder = c.surface0;
    split = null;
    prompt = null;   # starship's own colours, as before
    gtk = null;
    cursor = null;
  };

  wezterm = ''{ color_scheme = "Catppuccin Mocha" }'';

  zellij = { theme = "catppuccin-mocha"; kdl = null; };

  nvim = {
    plugins = [ { name = "catppuccin"; plugin = pkgs.vimPlugins.catppuccin-nvim; } ];
    lua = ''
      vim.cmd.packadd("catppuccin")
      require("catppuccin").setup({
        flavour = "mocha",
        integrations = { gitsigns = true, neotree = true, telescope = { enabled = true },
                         which_key = true, indent_blankline = { enabled = true }, alpha = true },
      })
      vim.cmd.colorscheme("catppuccin")
      return "catppuccin-mocha"
    '';
  };

  emacs = {
    packages = epkgs: [ epkgs.catppuccin-theme ];
    elisp = ''
      (setq catppuccin-flavor 'mocha)
      (load-theme 'catppuccin t)
    '';
  };
}
