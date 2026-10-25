# SPDX-License-Identifier: AGPL-3.0-or-later
# A look's data (look.toml, schema look/v1; plan B141) → what each program gets: the attribute set default.nix describes
# (desktop, wezterm, zellij, nvim, emacs). A look is data only – everything a program needs is made here from its
# colours and the few choices the schema allows (editor themes from nixpkgs, GTK theme, cursor). Without an editor theme
# the editor's colours are made from the look's colours too.
{ pkgs, lib }:

data:

let
  plain = data.plain or false;
  hex = c: lib.removePrefix "#" c; # "#RRGGBB" → "RRGGBB" (case as the look writes it)
  h = c: "#${c}";
  c = lib.mapAttrs (_: hex) (data.colors or { });
  t = data.terminal or { };
  n = map hex (t.normal or [ ]); # black red green yellow blue magenta cyan white
  b = map hex (t.bright or [ ]);
  at = builtins.elemAt;
  d = data.desktop or { };
  dark = (data.appearance or "dark") == "dark";
  cursor = c.cursor or c.accent;

  # Lualine's theme from the colours: the mode on the left in its colour, the rest quiet
  lualine = ''
    local function mode(bg) return { a = { fg = "${h c.background}", bg = bg, gui = "bold" },
      b = { fg = "${h c.text}", bg = "${h (at n 0)}" }, c = { fg = "${h c.subtext}", bg = "${h c.background}" } } end
    return {
      normal = mode("${h c.text}"), insert = mode("${h c.accent}"), visual = mode("${h (at n 3)}"),
      replace = mode("${h c.urgent}"), command = mode("${h c.text}"),
      inactive = { a = { fg = "${h c.muted}", bg = "${h c.background}" }, b = { fg = "${h c.muted}", bg = "${h c.background}" },
                   c = { fg = "${h c.muted}", bg = "${h c.background}" } },
    }
  '';

  # Neovim's highlight groups from the colours (no theme plugin); tree-sitter's groups link to these by default
  nvimFromColors = ''
    vim.cmd("highlight clear")
    vim.o.background = "${if dark then "dark" else "light"}"
    vim.g.colors_name = "omaflake-look"
    local set = function(group, spec) vim.api.nvim_set_hl(0, group, spec) end
    set("Normal", { fg = "${h c.text}", bg = "${h c.background}" })
    set("NormalFloat", { fg = "${h c.text}", bg = "${h c.surface}" })
    set("FloatBorder", { fg = "${h c.selection}", bg = "${h c.surface}" })
    set("CursorLine", { bg = "${h c.surface}" })
    set("CursorLineNr", { fg = "${h c.text}", bold = true })
    set("LineNr", { fg = "${h c.muted}" })
    set("SignColumn", { bg = "${h c.background}" })
    set("Visual", { bg = "${h c.selection}" })
    set("Search", { fg = "${h c.background}", bg = "${h (at n 6)}" })
    set("IncSearch", { fg = "${h c.background}", bg = "${h c.accent}" })
    set("MatchParen", { bg = "${h cursor}", bold = true })
    set("Cursor", { fg = "${h c.background}", bg = "${h cursor}" })
    set("StatusLine", { fg = "${h c.text}", bg = "${h c.surface}" })
    set("StatusLineNC", { fg = "${h c.muted}", bg = "${h c.background}" })
    set("WinSeparator", { fg = "${h c.selection}" })
    set("Pmenu", { fg = "${h c.text}", bg = "${h c.surface}" })
    set("PmenuSel", { fg = "${h c.text}", bg = "${h c.selection}" })
    set("Comment", { fg = "${h c.muted}", italic = true })
    set("String", { fg = "${h (at n 3)}" })
    set("Constant", { fg = "${h (at n 3)}" })
    set("Keyword", { fg = "${h (at n 2)}" })
    set("Statement", { fg = "${h (at n 2)}" })
    set("Function", { fg = "${h (at n 6)}" })
    set("Type", { fg = "${h (at n 4)}" })
    set("Special", { fg = "${h (at n 5)}" })
    set("PreProc", { fg = "${h (at n 1)}" })
    set("Identifier", { fg = "${h c.text}" })
    set("Directory", { fg = "${h c.accent}" })
    set("Title", { fg = "${h c.accent}", bold = true })
    set("Todo", { fg = "${h c.background}", bg = "${h c.accent}", bold = true })
    set("Error", { fg = "${h c.urgent}" })
    set("ErrorMsg", { fg = "${h c.urgent}" })
    set("WarningMsg", { fg = "${h (at n 3)}" })
    set("DiagnosticError", { fg = "${h c.urgent}" })
    set("DiagnosticWarn", { fg = "${h (at n 3)}" })
    set("DiagnosticInfo", { fg = "${h (at n 4)}" })
    set("DiagnosticHint", { fg = "${h (at n 6)}" })
  '';

  nvimThemes = {
    catppuccin = o: {
      plugins = [ { name = "catppuccin"; plugin = pkgs.vimPlugins.catppuccin-nvim; } ];
      lua = ''
        vim.cmd.packadd("catppuccin")
        require("catppuccin").setup({
          flavour = "${o.flavour or "mocha"}",
          integrations = { gitsigns = true, neotree = true, telescope = { enabled = true },
                           which_key = true, indent_blankline = { enabled = true }, alpha = true },
        })
        vim.cmd.colorscheme("catppuccin")
        return "catppuccin-${o.flavour or "mocha"}"
      '';
    };
    rose-pine = o: {
      plugins = [ { name = "rose-pine"; plugin = pkgs.vimPlugins.rose-pine; } ];
      lua = ''
        vim.cmd.packadd("rose-pine")
        require("rose-pine").setup({ variant = "${o.variant or "main"}" })
        vim.o.background = "${if (o.variant or "main") == "dawn" then "light" else "dark"}"
        vim.cmd.colorscheme("rose-pine-${o.variant or "main"}")
        return "rose-pine"
      '';
    };
    # Modus (WCAG AAA) with the look's own background, text, accent and urgent colour
    modus = o: {
      plugins = [ { name = "modus-themes"; plugin = pkgs.vimPlugins.modus-themes-nvim; } ];
      lua = ''
        vim.cmd.packadd("modus-themes")
        require("modus-themes").setup({
          style = "${o.style or "modus_vivendi"}",
          on_colors = function(colors)
            colors.bg_main = "${h c.background}"
            colors.fg_main = "${h c.text}"
            colors.cyan = "${h c.accent}"
            colors.cyan_intense = "${h c.accent}"
            colors.red = "${h c.urgent}"
            colors.red_intense = "${h c.urgent}"
            colors.bg_hl_line = "${h c.surface}"
            colors.border = "${h c.selection}"
          end,
        })
        vim.o.background = "${if dark then "dark" else "light"}"
        vim.cmd.colorscheme("${o.style or "modus_vivendi"}")
      '' + lualine;
    };
  };

  # Emacs' faces from the colours (no theme package)
  emacsFromColors = ''
    (deftheme omaflake-look "The guest's look (omaflake).")
    (custom-theme-set-faces 'omaflake-look
      '(default ((t (:background "${h c.background}" :foreground "${h c.text}"))))
      '(cursor ((t (:background "${h cursor}"))))
      '(region ((t (:background "${h c.selection}"))))
      '(hl-line ((t (:background "${h c.surface}"))))
      '(fringe ((t (:background "${h c.background}"))))
      '(vertical-border ((t (:foreground "${h c.selection}"))))
      '(line-number ((t (:foreground "${h c.muted}" :background "${h c.background}"))))
      '(line-number-current-line ((t (:foreground "${h c.text}" :background "${h c.surface}"))))
      '(mode-line ((t (:background "${h (at n 0)}" :foreground "${h c.text}" :box nil))))
      '(mode-line-inactive ((t (:background "${h c.surface}" :foreground "${h c.muted}" :box nil))))
      '(header-line ((t (:background "${h c.surface}" :foreground "${h c.subtext}"))))
      '(minibuffer-prompt ((t (:foreground "${h c.accent}" :weight bold))))
      '(font-lock-comment-face ((t (:foreground "${h c.muted}" :slant italic))))
      '(font-lock-doc-face ((t (:foreground "${h c.subtext}" :slant italic))))
      '(font-lock-string-face ((t (:foreground "${h (at n 3)}"))))
      '(font-lock-keyword-face ((t (:foreground "${h (at n 2)}"))))
      '(font-lock-builtin-face ((t (:foreground "${h (at n 1)}"))))
      '(font-lock-function-name-face ((t (:foreground "${h (at n 6)}"))))
      '(font-lock-variable-name-face ((t (:foreground "${h c.text}"))))
      '(font-lock-type-face ((t (:foreground "${h (at n 4)}"))))
      '(font-lock-constant-face ((t (:foreground "${h (at n 3)}"))))
      '(font-lock-warning-face ((t (:foreground "${h (at n 3)}" :weight bold))))
      '(link ((t (:foreground "${h c.accent}" :underline t))))
      '(error ((t (:foreground "${h c.urgent}"))))
      '(warning ((t (:foreground "${h (at n 3)}"))))
      '(success ((t (:foreground "${h (at n 2)}"))))
      '(isearch ((t (:background "${h (at n 6)}" :foreground "${h c.background}"))))
      '(lazy-highlight ((t (:background "${h cursor}"))))
      '(show-paren-match ((t (:background "${h cursor}" :weight bold))))
      '(completions-common-part ((t (:foreground "${h c.accent}"))))
      '(vertico-current ((t (:background "${h c.selection}")))))
    (enable-theme 'omaflake-look)
  '';

  emacsThemes = {
    catppuccin = o: {
      packages = epkgs: [ epkgs.catppuccin-theme ];
      elisp = ''
        (setq catppuccin-flavor '${o.flavour or "mocha"})
        (load-theme 'catppuccin t)
      '';
    };
    # Emacs' own Modus with the look's background, text, accent and urgent colour
    modus = o:
      let style = o.style or "modus-vivendi"; in {
        packages = _: [ ];
        elisp = ''
          (setq ${style}-palette-overrides
                '((bg-main "${h c.background}") (fg-main "${h c.text}") (cyan "${h c.accent}") (cyan-intense "${h c.accent}")
                  (red "${h c.urgent}") (red-intense "${h c.urgent}") (cursor "${h cursor}") (bg-hl-line "${h c.surface}")))
          (load-theme '${style} t)
        '';
      };
  };

  theme = editor: themes: fallback:
    let e = data.${editor} or null; in if e == null then fallback else themes.${e.theme} e;
in
if plain then {
  desktop = null;
  macos = null;
  wezterm = "{}";
  zellij = { theme = "default"; kdl = null; };
  nvim = { plugins = [ ]; lua = ''return "auto"''; };
  emacs = { packages = _: [ ]; elisp = ""; };
} else {
  desktop = {
    inherit dark;
    base = c.background;
    surface = c.surface;
    selection = c.selection;
    text = c.text;
    subtext = c.subtext;
    muted = c.muted;
    accent = c.accent;
    clock = if d ? clock then hex d.clock else c.text;
    urgent = c.urgent;
    border = map hex (d.border or [ c.accent ]);
    inactiveBorder = if d ? inactive_border then hex d.inactive_border else c.selection;
    split = if d ? split then map hex d.split else null;
    prompt = if d ? prompt then {
      ok = hex (d.prompt.ok or c.accent);
      error = hex (d.prompt.error or c.urgent);
      directory = hex (d.prompt.directory or c.text);
    } else null;
    gtk = { name = d.gtk or (if dark then "adw-gtk3-dark" else "adw-gtk3"); package = pkgs.adw-gtk3; };
    cursor = if d ? cursor_theme then { name = d.cursor_theme; package = pkgs.bibata-cursors; } else null;
  };

  wezterm = ''
    {
      colors = {
        foreground = "${h c.text}", background = "${h c.background}",
        cursor_bg = "${h cursor}", cursor_fg = "${h c.background}", cursor_border = "${h cursor}",
        selection_bg = "${h c.selection}", selection_fg = "${h c.text}",
        split = "${h c.selection}", scrollbar_thumb = "${h c.selection}",
        ansi = { ${lib.concatMapStringsSep ", " (x: "\"${h x}\"") n} },
        brights = { ${lib.concatMapStringsSep ", " (x: "\"${h x}\"") b} },
      },
    }
  '';

  # Zellij's palette theme: green is the active frame and ribbon – the look's accent
  zellij = {
    theme = "omaflake-look";
    kdl = ''
      themes {
          omaflake-look {
              fg "${h c.text}"
              bg "${h c.selection}"
              black "${h c.background}"
              white "${h c.text}"
              red "${h (at n 1)}"
              green "${h c.accent}"
              yellow "${h (at n 3)}"
              blue "${h (at n 4)}"
              magenta "${h (at n 5)}"
              orange "${h (at b 3)}"
              cyan "${h (at n 6)}"
          }
      }
    '';
  };

  # macOS guests' system look (the macOS desktop module): light or dark from `appearance`, the rest from [macos] –
  # values checked in a guest only (plan BM22); without them graphite and clear icons for a dark look, macOS' own for
  # a light one
  macos =
    let m = data.macos or { }; in {
      inherit dark;
      accent = m.accent or (if dark then "graphite" else "multicolor");
      icons = m.icons or (if dark then "clear" else "default");
      folders = m.folders or (if dark then "graphite" else "default");
      wallpaper = m.wallpaper or (if dark then "black" else "default");
    };

  nvim = theme "neovim" nvimThemes { plugins = [ ]; lua = nvimFromColors + lualine; };
  emacs = theme "emacs" emacsThemes { packages = _: [ ]; elisp = emacsFromColors; };
}
