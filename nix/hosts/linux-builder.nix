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

    # The default (1 core, 3 GiB, 20 GiB) ran out of disk on the first image build.
    # A changed builder configuration is built by the running default builder. The disk
    # (workingDirectory/nixos.qcow2) is only created if it is missing – see docs/linux-builder.md.
    config = {
      virtualisation.cores = 6;
      virtualisation.darwin-builder.memorySize = 12 * 1024; # MiB
      virtualisation.darwin-builder.diskSize = 100 * 1024;  # MiB
      # Every image build leaves several GB behind. When less than min-free is left during a build, Nix collects
      # garbage until max-free is free. The defaults (1 and 3 GiB) are too small for a ~9 GiB disk image (plan B38).
      virtualisation.darwin-builder.min-free = 20 * 1024 * 1024 * 1024; # bytes
      virtualisation.darwin-builder.max-free = 50 * 1024 * 1024 * 1024;
    };
  };

  # Tells omaflake that this nix-darwin system is its builder host (the app's "Apply Settings" warns otherwise)
  environment.etc."omaflake/linux-builder".text = "nix-darwin system from omaflake's image flake (#linux-builder)\n";

  system.stateVersion = 7;
}
