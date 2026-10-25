# SPDX-License-Identifier: AGPL-3.0-or-later
# Rosé Pine Dawn (plan B135): light and mild – cream, rose, iris and gold. No relation to omaflake's own look.
# Palette: https://rosepinetheme.com/palette (the "Dawn" variant).
{ pkgs, lib }:

let
  r = {
    base = "faf4ed"; surface = "fffaf3"; overlay = "f2e9e1";
    muted = "9893a5"; subtle = "797593"; text = "575279";
    love = "b4637a"; gold = "ea9d34"; rose = "d7827e"; pine = "286983"; foam = "56949f"; iris = "907aa9";
    hlLow = "f4ede8"; hlMed = "dfdad9"; hlHigh = "cecacd";
  };
  h = c: "#${c}";
in
{
  desktop = {
    dark = false;
    base = r.base;
    surface = r.surface;
    selection = r.hlMed;
    text = r.text;
    subtext = r.subtle;
    muted = r.muted;
    accent = r.iris;
    clock = r.pine;
    urgent = r.love;
    border = [ r.rose r.iris ];
    inactiveBorder = r.hlMed;
    split = null;
    prompt = { ok = r.pine; error = r.love; directory = r.iris; };
    gtk = { name = "adw-gtk3"; package = pkgs.adw-gtk3; };
    cursor = { name = "Bibata-Modern-Ice"; package = pkgs.bibata-cursors; };
  };

  # Rosé Pine's own terminal mapping (rose-pine/wezterm)
  wezterm = ''
    {
      colors = {
        foreground = "${h r.text}", background = "${h r.base}",
        cursor_bg = "${h r.hlHigh}", cursor_fg = "${h r.text}", cursor_border = "${h r.hlHigh}",
        selection_bg = "${h r.hlMed}", selection_fg = "${h r.text}",
        split = "${h r.hlMed}", scrollbar_thumb = "${h r.hlMed}",
        ansi = { "${h r.overlay}", "${h r.love}", "${h r.pine}", "${h r.gold}",
                 "${h r.foam}", "${h r.iris}", "${h r.rose}", "${h r.text}" },
        brights = { "${h r.muted}", "${h r.love}", "${h r.pine}", "${h r.gold}",
                    "${h r.foam}", "${h r.iris}", "${h r.rose}", "${h r.text}" },
      },
    }
  '';

  zellij = {
    theme = "rose-pine-dawn";
    kdl = ''
      themes {
          rose-pine-dawn {
              fg "${h r.text}"
              bg "${h r.hlMed}"
              black "${h r.overlay}"
              white "${h r.text}"
              red "${h r.love}"
              green "${h r.pine}"
              yellow "${h r.gold}"
              blue "${h r.foam}"
              magenta "${h r.iris}"
              orange "${h r.rose}"
              cyan "${h r.foam}"
          }
      }
    '';
  };

  nvim = {
    plugins = [ { name = "rose-pine"; plugin = pkgs.vimPlugins.rose-pine; } ];
    lua = ''
      vim.cmd.packadd("rose-pine")
      require("rose-pine").setup({ variant = "dawn" })
      vim.o.background = "light"
      vim.cmd.colorscheme("rose-pine-dawn")
      return "rose-pine"
    '';
  };

  # nixpkgs has no Rosé Pine for Emacs: a small theme of its own with the palette
  emacs = {
    packages = _: [ ];
    elisp = ''
      (deftheme omaflake-rose-pine-dawn "Rosé Pine Dawn (omaflake).")
      (custom-theme-set-faces 'omaflake-rose-pine-dawn
        '(default ((t (:background "${h r.base}" :foreground "${h r.text}"))))
        '(cursor ((t (:background "${h r.text}"))))
        '(region ((t (:background "${h r.hlMed}"))))
        '(hl-line ((t (:background "${h r.hlLow}"))))
        '(fringe ((t (:background "${h r.base}"))))
        '(vertical-border ((t (:foreground "${h r.hlMed}"))))
        '(line-number ((t (:foreground "${h r.muted}" :background "${h r.base}"))))
        '(line-number-current-line ((t (:foreground "${h r.text}" :background "${h r.hlLow}"))))
        '(mode-line ((t (:background "${h r.overlay}" :foreground "${h r.text}" :box nil))))
        '(mode-line-inactive ((t (:background "${h r.surface}" :foreground "${h r.muted}" :box nil))))
        '(header-line ((t (:background "${h r.surface}" :foreground "${h r.subtle}"))))
        '(minibuffer-prompt ((t (:foreground "${h r.iris}" :weight bold))))
        '(font-lock-comment-face ((t (:foreground "${h r.muted}" :slant italic))))
        '(font-lock-doc-face ((t (:foreground "${h r.subtle}" :slant italic))))
        '(font-lock-string-face ((t (:foreground "${h r.gold}"))))
        '(font-lock-keyword-face ((t (:foreground "${h r.pine}"))))
        '(font-lock-builtin-face ((t (:foreground "${h r.love}"))))
        '(font-lock-function-name-face ((t (:foreground "${h r.rose}"))))
        '(font-lock-variable-name-face ((t (:foreground "${h r.text}"))))
        '(font-lock-type-face ((t (:foreground "${h r.foam}"))))
        '(font-lock-constant-face ((t (:foreground "${h r.gold}"))))
        '(font-lock-warning-face ((t (:foreground "${h r.gold}" :weight bold))))
        '(link ((t (:foreground "${h r.iris}" :underline t))))
        '(error ((t (:foreground "${h r.love}"))))
        '(warning ((t (:foreground "${h r.gold}"))))
        '(success ((t (:foreground "${h r.pine}"))))
        '(isearch ((t (:background "${h r.rose}" :foreground "${h r.base}"))))
        '(lazy-highlight ((t (:background "${h r.hlHigh}"))))
        '(show-paren-match ((t (:background "${h r.hlHigh}" :weight bold))))
        '(completions-common-part ((t (:foreground "${h r.iris}"))))
        '(vertico-current ((t (:background "${h r.hlMed}")))))
      (enable-theme 'omaflake-rose-pine-dawn)
    '';
  };
}
