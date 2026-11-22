# SPDX-License-Identifier: AGPL-3.0-or-later
# omaflake's settings for the Linux builder VM (nixpkgs' darwin.linux-builder): used by omaflake's own builder
# (flake output linuxBuilders, run as the user, plan B160) and by the nix-darwin host (hosts/linux-builder.nix).
# The default (1 core, 3 GiB, 20 GiB) ran out of disk on the first image build. A changed builder configuration is
# built by the running default builder. The disk (nixos.qcow2 in the builder's folder) is only created if it is
# missing – omaflake replaces it when it switches from the default builder to this one.
{
  virtualisation.cores = 6;
  virtualisation.darwin-builder.memorySize = 12 * 1024; # MiB
  virtualisation.darwin-builder.diskSize = 100 * 1024; # MiB
  # Every image build leaves several GB behind. When less than min-free is left during a build, Nix collects
  # garbage until max-free is free. The defaults (1 and 3 GiB) are too small for a ~9 GiB disk image (plan B38).
  virtualisation.darwin-builder.min-free = 20 * 1024 * 1024 * 1024; # bytes
  virtualisation.darwin-builder.max-free = 50 * 1024 * 1024 * 1024;
}
