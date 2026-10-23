# SPDX-License-Identifier: AGPL-3.0-or-later
# omaflake's Linux app (crates/omaflake-gui, binary `omaflake`) with the CLI `omaf` it starts the core service with
# (plan B119, docs/linux.md). iced draws with wgpu (Vulkan, else OpenGL) and winit opens its window on Wayland or X11
# – all of them load their libraries at run time, so the wrapper puts them on LD_LIBRARY_PATH.
{ lib, rustPlatform, makeWrapper, wayland, libxkbcommon, vulkan-loader, libGL, libx11, libxcursor, libxi
, libxrandr, libxcb, fontconfig }:

let
  runtimeLibs = [ wayland libxkbcommon vulkan-loader libGL libx11 libxcursor libxi libxrandr libxcb fontconfig ];
  cargo = builtins.fromTOML (builtins.readFile ../Cargo.toml);
in
rustPlatform.buildRustPackage {
  pname = "omaflake-gui";
  version = cargo.workspace.package.version;

  src = lib.fileset.toSource {
    root = ../.;
    fileset = lib.fileset.unions [
      ../Cargo.toml
      ../Cargo.lock
      ../rustfmt.toml
      ../crates
      ../brand/png
      ../brand/svg
      ../brand/tokens.toml
      ./omaflake.desktop
    ];
  };
  cargoLock.lockFile = ../Cargo.lock;

  cargoBuildFlags = [ "-p" "omaflake-gui" "-p" "omaf" ];
  # The tests run with `cargo test` in development (some need a data folder, Nix or QEMU)
  doCheck = false;

  nativeBuildInputs = [ makeWrapper ];

  postInstall = ''
    install -Dm644 linux/omaflake.desktop $out/share/applications/omaflake.desktop
    for size in 16 32 64 128 256 512; do
      install -Dm644 brand/png/icon/omaflake-icon-$size.png $out/share/icons/hicolor/''${size}x''${size}/apps/omaflake.png
    done
    install -Dm644 brand/svg/omaflake-icon.svg $out/share/icons/hicolor/scalable/apps/omaflake.svg
    wrapProgram $out/bin/omaflake --prefix LD_LIBRARY_PATH : ${lib.makeLibraryPath runtimeLibs}
  '';

  meta = {
    description = "Clean, disposable NixOS VMs – omaflake's Linux app and CLI";
    homepage = "https://omaflake.dev";
    license = lib.licenses.agpl3Plus;
    mainProgram = "omaflake";
    platforms = [ "aarch64-linux" "x86_64-linux" ];
  };
}
