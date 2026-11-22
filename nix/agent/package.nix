# SPDX-License-Identifier: AGPL-3.0-or-later
# The omaflake guest agent (spec §11.9), built from this folder. The tests (frame vectors shared with the Mac side)
# run in the build. On Linux the agent links libasound (MIDI through the ALSA sequencer, midi_alsa.rs; its UMP functions
# are looked up at run time, so the alsa-lib of this nixpkgs only needs the base sequencer API).
{ lib, stdenv, rustPlatform, alsa-lib }:

rustPlatform.buildRustPackage {
  pname = "omaflake-agent";
  version = (lib.importTOML ./Cargo.toml).package.version;
  src = lib.fileset.toSource {
    root = ./.;
    fileset = lib.fileset.unions [ ./Cargo.toml ./Cargo.lock ./src ./testdata ];
  };
  cargoLock.lockFile = ./Cargo.lock;
  buildInputs = lib.optionals stdenv.hostPlatform.isLinux [ alsa-lib ];
  meta = {
    description = "omaflake guest agent: status, clipboard, camera and MIDI with omaflake on the Mac over a virtio port";
    license = lib.licenses.agpl3Plus;
    mainProgram = "omaflake-agent";
  };
}
