# SPDX-License-Identifier: AGPL-3.0-or-later
# The editor line of a recipe (plan B118): the base has no editor; a recipe picks neovim or emacs. The user's
# configuration of each line is home/neovim.nix or home/emacs.nix; here is what the system needs for it.
{ config, lib, pkgs, ... }:

let
  # $EDITOR for the emacs line: the running Emacs (its server, C-x # when done), else the user's configured Emacs in
  # the terminal; root, who has neither, gets plain Emacs (the same package, already in the closure)
  emacs = pkgs.emacs-pgtk;
  emacsEdit = pkgs.writeShellScriptBin "emacs-edit" ''
    if ${emacs}/bin/emacsclient --eval t >/dev/null 2>&1; then exec ${emacs}/bin/emacsclient "$@"; fi
    if command -v emacs >/dev/null; then exec emacs -nw "$@"; fi
    exec ${emacs}/bin/emacs -nw "$@"
  '';
in
{
  options.omaflake.editor = lib.mkOption {
    type = lib.types.enum [ "neovim" "emacs" ];
    default = "neovim";
    description = "The recipe's editor line: neovim (with Zellij) or emacs.";
  };

  config = lib.mkMerge [
    (lib.mkIf (config.omaflake.editor == "neovim") {
      environment.systemPackages = [ pkgs.neovim ]; # plain nvim for root; the user gets the configured one
      environment.variables.EDITOR = "nvim";
    })
    (lib.mkIf (config.omaflake.editor == "emacs") {
      environment.systemPackages = [ emacsEdit pkgs.nil ]; # nil: Nix language server for Eglot
      environment.variables.EDITOR = "emacs-edit";
      environment.variables.VISUAL = "emacs-edit";
    })
  ];
}
