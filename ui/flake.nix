# SPDX-License-Identifier: AGPL-3.0-or-later
{
  description = "omaflake Tauri 2 UI development environment";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/55bfb4d7437bd4e09ab7cba58a43bd68eb310044";

  outputs = { nixpkgs, ... }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
    in
    {
      devShells.${system}.default = pkgs.mkShell {
        packages = with pkgs; [
          nodejs_22
          pnpm
          rustc
          cargo
          rust-analyzer
          rustfmt
          clippy
          pkg-config
          openssl
          gtk3
          glib
          webkitgtk_4_1
          libsoup_3
          cairo
          gdk-pixbuf
          pango
          atk
          librsvg
          patchelf
        ];

        shellHook = ''
          export CARGO_TARGET_DIR="''${XDG_CACHE_HOME:-$HOME/.cache}/omaflake/ui-target"
          echo "omaflake UI development shell (x86_64-linux)"
          echo "Run 'pnpm install' once, then 'pnpm tauri dev'."
        '';
      };
    };
}
