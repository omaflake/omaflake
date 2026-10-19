# SPDX-License-Identifier: AGPL-3.0-or-later
# The omaflake guest agent (spec §11.9), built from this folder. The tests (frame vectors shared with the Mac side)
# run in the build.
{ lib, rustPlatform }:

rustPlatform.buildRustPackage {
  pname = "omaflake-agent";
  version = (lib.importTOML ./Cargo.toml).package.version;
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./src ./testdata ];
  };
  cargoLock.lockFile = ./Cargo.lock;
  meta = {
    description = "omaflake guest agent: status, clipboard and camera with omaflake on the Mac over a virtio port";
    license = lib.licenses.agpl3Plus;
    mainProgram = "omaflake-agent";
  };
}
