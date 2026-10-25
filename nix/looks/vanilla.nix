# SPDX-License-Identifier: AGPL-3.0-or-later
# Vanilla (plan B135): every program in its own colours – fonts, layout and keys stay omaflake's
{ pkgs, lib }:

{
  desktop = null;
  wezterm = "{}";
  zellij = { theme = "default"; kdl = null; };
  nvim = { plugins = [ ]; lua = ''return "auto"''; };
  emacs = { packages = _: [ ]; elisp = ""; };
}
