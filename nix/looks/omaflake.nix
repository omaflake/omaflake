# SPDX-License-Identifier: AGPL-3.0-or-later
# The omaflake look (plan B135): black, high contrast, white text. Red and cyan – the hero's RGB split – only as fine
# accents: the active window's border, the active workspace, the cursor, matches. Colours: omaflake-palette.nix,
# generated from brand/tokens.toml ([look]).
{ pkgs, lib }:

let
  p = import ./omaflake-palette.nix;
  h = c: "#${c}";
in
{
  desktop = {
    dark = true;
    base = p.base;
    surface = p.surface;
    selection = p.border;
    text = p.text;
    subtext = p.subtext;
    muted = p.muted;
    accent = p.cyan;          # matches, the bar's highlights
    clock = p.text;
    urgent = p.red;
    border = [ p.red p.cyan ]; # the active window: the split as a gradient
    inactiveBorder = p.overlay;
    split = [ p.red p.cyan ];  # the active workspace's number: a 1 px red/cyan shadow each side
    prompt = { ok = p.cyan; error = p.red; directory = p.text; };
    gtk = { name = "adw-gtk3-dark"; package = pkgs.adw-gtk3; };
    cursor = { name = "Bibata-Modern-Classic"; package = pkgs.bibata-cursors; };
  };

  wezterm = ''
    {
      colors = {
        foreground = "${h p.text}", background = "${h p.base}",
        cursor_bg = "${h p.cyan}", cursor_fg = "${h p.base}", cursor_border = "${h p.cyan}",
        selection_bg = "${h p.border}", selection_fg = "${h p.text}",
        split = "${h p.border}", scrollbar_thumb = "${h p.border}",
        ansi = { "${h p.overlay}", "${h p.red}", "${h p.green}", "${h p.yellow}",
                 "${h p.blue}", "${h p.magenta}", "${h p.cyan}", "${h p.subtext}" },
        brights = { "${h p.muted}", "${h p.red}", "${h p.green}", "${h p.yellow}",
                    "${h p.blue}", "${h p.magenta}", "${h p.cyan}", "${h p.text}" },
      },
    }
  '';

  zellij = {
    theme = "omaflake";
    # Zellij's palette themes: green is the active frame and ribbon – cyan here
    kdl = ''
      themes {
          omaflake {
              fg "${h p.text}"
              bg "${h p.border}"
              black "${h p.base}"
              white "${h p.text}"
              red "${h p.red}"
              green "${h p.cyan}"
              yellow "${h p.yellow}"
              blue "${h p.blue}"
              magenta "${h p.magenta}"
              orange "${h p.orange}"
              cyan "${h p.cyan}"
          }
      }
    '';
  };

  nvim = {
    plugins = [ { name = "modus-themes"; plugin = pkgs.vimPlugins.modus-themes-nvim; } ];
    # Modus Vivendi (black, WCAG AAA) with the omaflake accents
    lua = ''
      vim.cmd.packadd("modus-themes")
      require("modus-themes").setup({
        style = "modus_vivendi",
        on_colors = function(c)
          c.bg_main = "${h p.base}"
          c.fg_main = "${h p.text}"
          c.cyan = "${h p.cyan}"
          c.cyan_intense = "${h p.cyan}"
          c.red = "${h p.red}"
          c.red_intense = "${h p.red}"
          c.bg_hl_line = "${h p.surface}"
          c.border = "${h p.border}"
        end,
      })
      vim.o.background = "dark"
      vim.cmd.colorscheme("modus_vivendi")
      local function mode(bg) return { a = { fg = "${h p.base}", bg = bg, gui = "bold" },
        b = { fg = "${h p.text}", bg = "${h p.overlay}" }, c = { fg = "${h p.subtext}", bg = "${h p.base}" } } end
      return {
        normal = mode("${h p.text}"), insert = mode("${h p.cyan}"), visual = mode("${h p.yellow}"),
        replace = mode("${h p.red}"), command = mode("${h p.text}"),
        inactive = { a = { fg = "${h p.muted}", bg = "${h p.base}" }, b = { fg = "${h p.muted}", bg = "${h p.base}" },
                     c = { fg = "${h p.muted}", bg = "${h p.base}" } },
      }
    '';
  };

  emacs = {
    packages = _: [ ];
    # Emacs' own Modus Vivendi with the omaflake accents
    elisp = ''
      (setq modus-vivendi-palette-overrides
            '((bg-main "${h p.base}") (fg-main "${h p.text}") (cyan "${h p.cyan}") (cyan-intense "${h p.cyan}")
              (red "${h p.red}") (red-intense "${h p.red}") (cursor "${h p.cyan}") (bg-hl-line "${h p.surface}")))
      (load-theme 'modus-vivendi t)
    '';
  };
}
