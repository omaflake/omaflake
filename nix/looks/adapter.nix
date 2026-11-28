# SPDX-License-Identifier: AGPL-3.0-or-later
# A look's data (look.toml, schema look/v1; plan B141) → what each program gets: the attribute set default.nix describes
# (desktop, wezterm, zellij, nvim, emacs). A look is data only – everything a program needs is made here from its
# colours and the few choices the schema allows (editor themes from nixpkgs, GTK theme, cursor). Without an editor theme
# the editor's colours are made from the look's colours too.
#
# looks/default.nix has checked the data (lib/validate.nix). All the same nothing reaches a program's configuration
# here unchecked: a colour goes through `hex` (six hex digits or the build fails), a choice through `pick` (one of the
# set or the build fails) – so no value of a look can end a string or a comment of Lua, Emacs Lisp or KDL.
{ pkgs, lib, validate ? import ../lib/validate.nix { inherit lib; } }:

data:

let
  plain = (data.plain or false) == true;
  hex = validate.hexOf "look: a colour"; # "#RRGGBB" → "RRGGBB" (case as the look writes it)
  choices = validate.lookChoices;
  # A table's choice, or the default: one of the set
  pick = what: set: table: key: default: validate.chosen "look: ${what}.${key}" set (table.${key} or default);
  h = c: "#${c}";
  c = lib.mapAttrs (_: hex) (data.colors or { });
  t = data.terminal or { };
  n = map hex (t.normal or [ ]); # black red green yellow blue magenta cyan white
  b = map hex (t.bright or [ ]);
  at = builtins.elemAt;
  d = data.desktop or { };
  dark = pick "look" choices.appearance data "appearance" "dark" == "dark";
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
    catppuccin = o: let flavour = pick "neovim" choices.neovim.catppuccin.flavour o "flavour" "mocha"; in {
      plugins = [ { name = "catppuccin"; plugin = pkgs.vimPlugins.catppuccin-nvim; } ];
      lua = ''
        vim.cmd.packadd("catppuccin")
        require("catppuccin").setup({
          flavour = "${flavour}",
          integrations = { gitsigns = true, neotree = true, telescope = { enabled = true },
                           which_key = true, indent_blankline = { enabled = true }, alpha = true },
        })
        vim.cmd.colorscheme("catppuccin")
        return "catppuccin-${flavour}"
      '';
    };
    rose-pine = o: let variant = pick "neovim" choices.neovim.rose-pine.variant o "variant" "main"; in {
      plugins = [ { name = "rose-pine"; plugin = pkgs.vimPlugins.rose-pine; } ];
      lua = ''
        vim.cmd.packadd("rose-pine")
        require("rose-pine").setup({ variant = "${variant}" })
        vim.o.background = "${if variant == "dawn" then "light" else "dark"}"
        vim.cmd.colorscheme("rose-pine-${variant}")
        return "rose-pine"
      '';
    };
    # Modus (WCAG AAA) with the look's own background, text, accent and urgent colour
    modus = o: let style = pick "neovim" choices.neovim.modus.style o "style" "modus_vivendi"; in {
      plugins = [ { name = "modus-themes"; plugin = pkgs.vimPlugins.modus-themes-nvim; } ];
      lua = ''
        vim.cmd.packadd("modus-themes")
        require("modus-themes").setup({
          style = "${style}",
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
        vim.cmd.colorscheme("${style}")
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
    catppuccin = o: let flavour = pick "emacs" choices.emacs.catppuccin.flavour o "flavour" "mocha"; in {
      packages = epkgs: [ epkgs.catppuccin-theme ];
      elisp = ''
        (setq catppuccin-flavor '${flavour})
        (load-theme 'catppuccin t)
      '';
    };
    # Emacs' own Modus with the look's background, text, accent and urgent colour
    modus = o:
      let style = pick "emacs" choices.emacs.modus.style o "style" "modus-vivendi"; in {
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
    let e = data.${editor} or null; in
    if e == null then fallback
    else themes.${validate.chosen "look: ${editor}.theme" (builtins.attrNames themes) (e.theme or null)} e;
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
    border = if d ? border then map hex d.border else [ c.accent ];
    inactiveBorder = if d ? inactive_border then hex d.inactive_border else c.selection;
    split = if d ? split then map hex d.split else null;
    prompt = if d ? prompt then {
      ok = if d.prompt ? ok then hex d.prompt.ok else c.accent;
      error = if d.prompt ? error then hex d.prompt.error else c.urgent;
      directory = if d.prompt ? directory then hex d.prompt.directory else c.text;
    } else null;
    gtk = { name = pick "desktop" choices.gtk d "gtk" (if dark then "adw-gtk3-dark" else "adw-gtk3"); package = pkgs.adw-gtk3; };
    cursor = if d ? cursor_theme
      then { name = pick "desktop" choices.cursorTheme d "cursor_theme" null; package = pkgs.bibata-cursors; } else null;
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
      accent = pick "macos" choices.macos.accent m "accent" (if dark then "graphite" else "multicolor");
      icons = pick "macos" choices.macos.icons m "icons" (if dark then "clear" else "default");
      folders = pick "macos" choices.macos.folders m "folders" (if dark then "graphite" else "default");
      wallpaper = pick "macos" choices.macos.wallpaper m "wallpaper" (if dark then "black" else "default");
    };

  nvim = theme "neovim" nvimThemes { plugins = [ ]; lua = nvimFromColors + lualine; };
  emacs = theme "emacs" emacsThemes { packages = _: [ ]; elisp = emacsFromColors; };
}
