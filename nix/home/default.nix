# SPDX-License-Identifier: AGPL-3.0-or-later
# home-manager configuration of the guest user on NixOS, the platform's part: who the user is and git's identity
# from the credential. Everything else – desktop, terminal, shell, editors – comes from the recipe's modules
# (omaflake/catalog `modules/`), which add to `home-manager.users.<user>`.
{ lib, settings, ... }:

{
  home.username = settings.user;
  home.homeDirectory = "/home/${settings.user}";
  home.stateVersion = "26.05";

  programs.git = {
    enable = true;
    settings.user = {
      name = settings.fullName;
    } // lib.optionalAttrs (settings.email != null) { inherit (settings) email; };
  };

  programs.home-manager.enable = true;
}
