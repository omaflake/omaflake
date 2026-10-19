# SPDX-License-Identifier: AGPL-3.0-or-later
# Desktop: Hyprland, audio, fonts (user configuration in home/)
{ pkgs, settings, ... }:

{
  # Hyprland (compositor, portal, session)
  programs.hyprland.enable = true;
  hardware.graphics.enable = true;

  # Auto-login on TTY1; Hyprland is started from zsh (see home/default.nix)
  services.getty.autologinUser = settings.user;

  # Audio
  security.rtkit.enable = true;
  services.pipewire = {
    enable = true;
    alsa.enable = true;
    pulse.enable = true;
  };

  # Electron/Chromium apps natively on Wayland
  environment.sessionVariables.NIXOS_OZONE_WL = "1";

  fonts.packages = with pkgs; [
    nerd-fonts.jetbrains-mono
    noto-fonts
  ];

  environment.systemPackages = with pkgs; [
    swaybg        # background
    wl-clipboard  # wl-copy / wl-paste
    grim slurp    # screenshots
    brightnessctl
    pavucontrol
  ];
}
