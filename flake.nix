# SPDX-License-Identifier: AGPL-3.0-or-later
{
  description = "omaflake – clean NixOS and macOS VMs on Apple silicon";

  # nixpkgs only for omaflake's QEMU package (pinned to the same revision as nix/flake.lock).
  # The flake that builds images from recipes is in nix/ (it brings your local settings, see nix/README.md); the
  # recipes themselves come from recipe sources (omaflake/recipes by default). nix/flake.nix uses the guest module
  # file directly.
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }: {
    # Guest hardware for NixOS recipes run by omaflake (UEFI, virtio, labels, 9p share)
    nixosModules.vm-guest = import ./nix/modules/vm-guest.nix;
    nixosModules.default = self.nixosModules.vm-guest;

    # QEMU with 3D for NixOS guests (spec §11.2): nix profile install github:omaflake/omaflake#qemu
    packages.aarch64-darwin = let pkgs = nixpkgs.legacyPackages.aarch64-darwin; in {
      qemu = pkgs.callPackage ./qemu/package.nix { };
      default = self.packages.aarch64-darwin.qemu;
    };
  };
}
