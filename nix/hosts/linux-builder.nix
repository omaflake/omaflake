# SPDX-License-Identifier: AGPL-3.0-or-later
# nix-darwin host with a Linux builder for aarch64-linux images (docs/linux-builder.md).
# Activate: sudo darwin-rebuild switch --flake path:$PWD#linux-builder   (in recipes/)
# Machine-specific changes (cores, memory, disk): local/darwin.nix
{ ... }:

{
  nixpkgs.hostPlatform = "aarch64-darwin";

  # An existing multi-user installation (nixbld GID 350) is adopted by nix-darwin
  nix.enable = true;
  nix.settings.experimental-features = [ "nix-command" "flakes" ];

  # Linux builder: aarch64-linux VM under macOS, registered as a remote builder
  # (nix-darwin modules/nix/linux-builder.nix)
  nix.linux-builder = {
    enable = true;
    ephemeral = false; # keep the builder's Nix store → fewer rebuilds

    # cores, memory, disk, min-free: shared with omaflake's own builder (plan B160)
    config = import ./builder-vm.nix;
  };

  # Tells omaflake that this nix-darwin system is its builder host (the app's "Apply Settings" warns otherwise)
  environment.etc."omaflake/linux-builder".text = "nix-darwin system from omaflake's image flake (#linux-builder)\n";

  system.stateVersion = 7;
}
