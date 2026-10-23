# SPDX-License-Identifier: AGPL-3.0-or-later
# The emacs line (omaflake.editor = "emacs", modules/editor.nix): Emacs, preconfigured – pure GTK, native on Wayland,
# in the desktop's colours. Vertico/Orderless/Marginalia/Consult for finding things, Corfu for completion, Magit,
# Eglot (built in) and tree-sitter grammars from Nix. Cmd+E opens it; it runs a server, so $EDITOR (emacs-edit,
# modules/editor.nix) opens files from the terminal in it. No multiplexer: Emacs has its own windows, tabs and shells.
{ pkgs, lib, osConfig, ... }:

lib.mkIf (osConfig.omaflake.editor == "emacs") {
  wayland.windowManager.hyprland.extraConfig = lib.mkAfter ''
    hl.bind(mod .. " + E", hl.dsp.exec_cmd("emacs"))
  '';

  # ---------- Editor: Emacs (Catppuccin Mocha, preconfigured) ----------
  programs.emacs = {
    enable = true;
    package = pkgs.emacs-pgtk;
    extraPackages = epkgs: with epkgs; [
      catppuccin-theme
      doom-modeline
      nerd-icons
      vertico
      orderless
      marginalia
      consult
      corfu
      magit
      diff-hl
      nix-ts-mode
      treesit-grammars.with-all-grammars
    ];
    extraConfig = ''
      ;; Basics
      (setq inhibit-startup-screen t
            initial-scratch-message ";; omaflake – C-x C-f opens a file, C-x g is Magit, M-x anything else\n\n"
            ring-bell-function #'ignore
            use-short-answers t
            make-backup-files nil
            auto-save-default nil
            select-enable-clipboard t        ; the Wayland clipboard
            custom-file (locate-user-emacs-file "custom.el"))
      (setq-default indent-tabs-mode nil tab-width 2 fill-column 100)
      (menu-bar-mode -1)
      (tool-bar-mode -1)
      (scroll-bar-mode -1)
      (global-display-line-numbers-mode 1)
      (global-hl-line-mode 1)
      (electric-pair-mode 1)
      (savehist-mode 1)
      (recentf-mode 1)
      (save-place-mode 1)
      (global-auto-revert-mode 1)
      (pixel-scroll-precision-mode 1)
      (which-key-mode 1)

      ;; The first Emacs window serves the terminal's $EDITOR (emacs-edit): C-x # when done
      (require 'server)
      (unless (server-running-p) (server-start))

      ;; Look
      (set-face-attribute 'default nil :family "JetBrainsMono Nerd Font" :height 120)
      (setq catppuccin-flavor 'mocha)
      (load-theme 'catppuccin t)
      (setq doom-modeline-icon t)
      (doom-modeline-mode 1)

      ;; Finding things: C-x b, M-s r (ripgrep), M-s l (lines), M-g i (imenu)
      (setq completion-styles '(orderless basic)
            completion-category-overrides '((file (styles partial-completion))))
      (vertico-mode 1)
      (marginalia-mode 1)
      (global-set-key (kbd "C-x b") #'consult-buffer)
      (global-set-key (kbd "M-s r") #'consult-ripgrep)
      (global-set-key (kbd "M-s l") #'consult-line)
      (global-set-key (kbd "M-g i") #'consult-imenu)

      ;; Completion in the buffer
      (setq corfu-auto t corfu-cycle t)
      (global-corfu-mode 1)

      ;; Git
      (global-set-key (kbd "C-x g") #'magit-status)
      (global-diff-hl-mode 1)
      (add-hook 'magit-post-refresh-hook #'diff-hl-magit-post-refresh)

      ;; Tree-sitter modes (grammars from Nix); Eglot: M-x eglot in any file whose language server is installed
      (setq major-mode-remap-alist
            '((python-mode . python-ts-mode) (js-mode . js-ts-mode) (sh-mode . bash-ts-mode)
              (c-mode . c-ts-mode) (c++-mode . c++-ts-mode) (css-mode . css-ts-mode)
              (json-mode . json-ts-mode) (js-json-mode . json-ts-mode) (conf-toml-mode . toml-ts-mode)))
      (dolist (m '(("\\.nix\\'" . nix-ts-mode) ("\\.rs\\'" . rust-ts-mode) ("\\.go\\'" . go-ts-mode)
                   ("\\.ts\\'" . typescript-ts-mode) ("\\.tsx\\'" . tsx-ts-mode)
                   ("\\.ya?ml\\'" . yaml-ts-mode) ("\\.lua\\'" . lua-ts-mode)))
        (add-to-list 'auto-mode-alist m))
      (with-eval-after-load 'eglot
        (add-to-list 'eglot-server-programs '(nix-ts-mode . ("nil"))))
      (add-hook 'nix-ts-mode-hook #'eglot-ensure)
    '';
  };
}
