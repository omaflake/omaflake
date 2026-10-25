# SPDX-License-Identifier: AGPL-3.0-or-later
# Shared base for all recipes: user, locale, SSH, Nix – no editor (modules/editor.nix)
{ lib, pkgs, settings, ... }:

let
  # The Mac's keyboard layout: the same keys as on the Mac (\ = Option+Shift+7 on German, @ = Option+L, …).
  # XKB has a Mac variant for these (xkeyboard-config, checked 2026-10-17); es, pl and cz have none (plan B81)
  macVariants = {
    us = "mac"; gb = "mac"; de = "mac"; ch = "de_mac"; at = "mac"; fr = "mac"; it = "mac"; nl = "mac";
    se = "mac"; no = "mac"; dk = "mac"; fi = "mac"; pt = "mac";
  };
  # The locale's keyboards (plan B141): the first is the default, Super+Space switches (grp:win_space_toggle)
  keyboards = if (settings.keyboards or [ ]) != [ ] then settings.keyboards else [ settings.keyboard ];
in
{
  time.timeZone = settings.timeZone;
  i18n.defaultLocale = settings.locale;
  # One keyboard setting for desktop and console: the console follows XKB (any XKB name works, not only kbd's).
  # Both Option keys act as on the Mac (level 3 for special characters); the desktop's shortcuts use Cmd.
  services.xserver.xkb = {
    layout = lib.concatStringsSep "," keyboards;
    variant = lib.concatMapStringsSep "," (k: macVariants.${k} or "") keyboards;
    options = "lv3:alt_switch" + lib.optionalString (builtins.length keyboards > 1) ",grp:win_space_toggle";
  };
  console.useXkbConfig = true;

  networking.networkmanager.enable = true;

  users.users.${settings.user} = {
    isNormalUser = true;
    description = settings.fullName;
    extraGroups = [ "wheel" "networkmanager" "video" ];
    shell = pkgs.zsh;
    initialPassword = "changeme"; # change with `passwd` after the first login
    # Public keys from local/keys/ – for `omaf ssh` and the readiness check
    openssh.authorizedKeys.keyFiles = settings.authorizedKeyFiles;
    openssh.authorizedKeys.keys = settings.authorizedKeys;
  };
  programs.zsh.enable = true;

  # SSH – for omaflake and editors with remote support
  services.openssh.enable = true;

  nix.settings = {
    experimental-features = [ "nix-command" "flakes" ];
    trusted-users = [ "@wheel" ];
    auto-optimise-store = true;
  };
  nix.gc = {
    automatic = true;
    dates = "weekly";
    options = "--delete-older-than 14d";
  };

  environment.systemPackages = with pkgs; [ git curl wget htop ]; # no editor: the recipe's line brings it (editor.nix)

  system.stateVersion = "26.05";
}
