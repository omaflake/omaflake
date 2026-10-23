# SPDX-License-Identifier: AGPL-3.0-or-later
# The neovim line (omaflake.editor = "neovim", modules/editor.nix): Neovim, preconfigured, and Zellij
{ pkgs, lib, osConfig, ... }:

lib.mkIf (osConfig.omaflake.editor == "neovim") {
  # ---------- Editor: Neovim (Catppuccin Mocha, preconfigured) ----------
  programs.neovim = {
    enable = true;
    defaultEditor = true;
    viAlias = true;
    vimAlias = true;
    withRuby = false;
    withPython3 = false;
    plugins = with pkgs.vimPlugins; [
      catppuccin-nvim
      nvim-web-devicons
      lualine-nvim
      bufferline-nvim
      # grammars via p.<name>: kept explicit, so no `with p;` can shadow names in scope
      (nvim-treesitter.withPlugins (p: map (n: p.${n}) [
        "bash" "c" "go" "javascript" "json" "lua" "markdown" "markdown_inline"
        "nix" "python" "rust" "toml" "typescript" "vim" "vimdoc" "yaml"
      ]))
      indent-blankline-nvim
      gitsigns-nvim
      which-key-nvim
      telescope-nvim
      telescope-fzf-native-nvim
      plenary-nvim
      neo-tree-nvim
      nui-nvim
      nvim-autopairs
      alpha-nvim
    ];
    initLua = ''
      -- Basics
      vim.g.mapleader = " "
      vim.g.maplocalleader = " "
      local o = vim.opt
      o.number = true
      o.relativenumber = true
      o.termguicolors = true
      o.mouse = "a"
      o.clipboard = "unnamedplus"   -- wl-clipboard
      o.cursorline = true
      o.signcolumn = "yes"
      o.scrolloff = 8
      o.expandtab = true
      o.shiftwidth = 2
      o.tabstop = 2
      o.ignorecase = true
      o.smartcase = true
      o.splitright = true
      o.splitbelow = true
      o.undofile = true
      o.showmode = false            -- shown in lualine
      o.laststatus = 3

      -- Look
      require("catppuccin").setup({
        flavour = "mocha",
        integrations = { gitsigns = true, neotree = true, telescope = { enabled = true },
                         which_key = true, indent_blankline = { enabled = true }, alpha = true },
      })
      vim.cmd.colorscheme("catppuccin")
      require("lualine").setup({ options = { theme = "catppuccin-mocha", globalstatus = true,
        component_separators = "", section_separators = { left = "", right = "" } } })
      require("bufferline").setup({ options = { diagnostics = false, separator_style = "slant",
        offsets = { { filetype = "neo-tree", text = "Files", separator = true } } } })
      require("ibl").setup({ indent = { char = "│" }, scope = { enabled = true } })
      require("gitsigns").setup()
      require("nvim-autopairs").setup()
      require("which-key").setup()
      require("neo-tree").setup({ close_if_last_window = true,
        filesystem = { follow_current_file = { enabled = true } } })

      -- Start screen
      local dashboard = require("alpha.themes.dashboard")
      dashboard.section.header.val = { "", "  N E O V I M", "" }
      dashboard.section.buttons.val = {
        dashboard.button("f", "  Find file", ":Telescope find_files<CR>"),
        dashboard.button("g", "  Find text", ":Telescope live_grep<CR>"),
        dashboard.button("r", "  Recent files", ":Telescope oldfiles<CR>"),
        dashboard.button("e", "  New file", ":ene <BAR> startinsert<CR>"),
        dashboard.button("q", "  Quit", ":qa<CR>"),
      }
      require("alpha").setup(dashboard.config)

      -- Search
      local telescope = require("telescope")
      telescope.setup({ defaults = { prompt_prefix = "   ", selection_caret = " " } })
      pcall(telescope.load_extension, "fzf")

      -- Treesitter (nvim-treesitter main: parsers from Nix, highlighting via the Neovim API)
      vim.api.nvim_create_autocmd("FileType", {
        callback = function(args)
          if pcall(vim.treesitter.start, args.buf) then
            vim.bo[args.buf].indentexpr = "v:lua.require'nvim-treesitter'.indentexpr()"
          end
        end,
      })

      -- Key bindings (leader = space)
      local map = vim.keymap.set
      local tb = require("telescope.builtin")
      map("n", "<leader>ff", tb.find_files, { desc = "Find files" })
      map("n", "<leader>fg", tb.live_grep,  { desc = "Find text" })
      map("n", "<leader>fb", tb.buffers,    { desc = "Buffers" })
      map("n", "<leader>fh", tb.help_tags,  { desc = "Help" })
      map("n", "<leader>e", "<cmd>Neotree toggle<CR>", { desc = "File tree" })
      map("n", "<S-l>", "<cmd>BufferLineCycleNext<CR>", { desc = "Next buffer" })
      map("n", "<S-h>", "<cmd>BufferLineCyclePrev<CR>", { desc = "Previous buffer" })
      map("n", "<leader>w", "<cmd>w<CR>", { desc = "Save" })
      map("n", "<leader>q", "<cmd>q<CR>", { desc = "Close" })
      map("n", "<Esc>", "<cmd>nohlsearch<CR>")
    '';
  };

  # ---------- Multiplexer: Zellij ----------
  programs.zellij = {
    enable = true;
    enableZshIntegration = false; # not in every shell – start `zellij` by hand
    settings = {
      theme = "catppuccin-mocha";
      default_layout = "compact";
      pane_frames = false;
      simplified_ui = true;
      show_startup_tips = false;
      show_release_notes = false;
    };
  };
}
