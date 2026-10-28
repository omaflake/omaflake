# SPDX-License-Identifier: AGPL-3.0-or-later
# omaflake's QEMU as a Nix package (spec §11.2): the same build as qemu/build.sh – QEMU with 3D for NixOS guests
# (virtio-gpu-gl via virglrenderer, ANGLE from nixpkgs and libepoxy) – but from nixpkgs' derivations, without Homebrew:
#   nix profile install github:omaflake/omaflake#qemu
# Sources are pinned by hash; versions and patch lists match qemu/build.sh (patchset 5).
# Binary cache so that users don't compile it: OPEN.
{
  lib,
  stdenv,
  fetchurl,
  buildPackages,
  qemu,
  virglrenderer,
  libepoxy,
  vulkan-headers,
  vulkan-loader,
  # ANGLE (OpenGL ES over Metal) from nixpkgs – built from source, BSD-3-Clause, in the binary cache; replaced the
  # prebuilt bottle without a license file (plan B55)
  angle,
}:

let
  # libepoxy at the commit qemu/build.sh uses, with EGL 1.5 (akihikodaki), EGL from ANGLE
  epoxy = libepoxy.overrideAttrs (old: {
    version = "1.5.10-1b6d7db";
    src = fetchurl {
      url = "https://github.com/anholt/libepoxy/archive/1b6d7db184bb1a0d9af0e200e06a0331028eaaae.tar.gz";
      sha256 = "15f769a8f24c361c8de28d72625daf07d0107b683fa0a5118d0aa8b0c5fc9eab";
    };
    # The EGL 1.5 patch from startergo's tap (v1.0.5), kept here: the tap's tag was moved later (plan B116)
    patches = [ ./patches/startergo/libepoxy-akihikodaki-egl15.patch ];
    # epoxy opens ANGLE by bare name ("libEGL.dylib"), which dyld does not find outside Homebrew's prefix –
    # QEMU aborted with --gl es (plan B55): load ANGLE's libraries by their store paths
    postPatch = ''
      substituteInPlace src/dispatch_common.c \
        --replace-fail '"libEGL.dylib"' '"${angle}/lib/libEGL.dylib"' \
        --replace-fail '"libGLESv2.dylib"' '"${angle}/lib/libGLESv2.dylib"' \
        --replace-fail '"libGLESv1_CM.dylib"' '"${angle}/lib/libGLESv1_CM.dylib"'
    '';
    # epoxy's EGL headers include ANGLE's: everything that uses epoxy (virglrenderer, QEMU's Cocoa UI) needs them
    propagatedBuildInputs = (old.propagatedBuildInputs or [ ]) ++ [ angle ];
    mesonFlags = [ "-Degl=yes" "-Dx11=false" "-Dtests=false" ];
    env = (old.env or { }) // { NIX_CFLAGS_COMPILE = "-I${angle}/include"; };
    doCheck = false;
  });

  # virglrenderer 1.3.0 with startergo's macOS patches (one clean patch, see its header) and the native OpenGL
  # patch of try-omarchy
  virgl = (virglrenderer.override { libepoxy = epoxy; }).overrideAttrs (old: {
    # The release archive qemu/build.sh uses (the patches do not apply to nixpkgs' git checkout)
    src = fetchurl {
      url = "https://gitlab.freedesktop.org/virgl/virglrenderer/-/archive/1.3.0/virglrenderer-1.3.0.tar.gz";
      sha256 = "065bc56e89e6f631f96101cd62eba0748e48eb888b434edc86e89d05395e76f3";
    };
    patches = [ ./patches/startergo/virglrenderer-1.3.0-macos.patch ./patches/try-omarchy/virgl-native-opengl.patch ];
    buildInputs = [ epoxy angle vulkan-headers vulkan-loader ];
    mesonBuildType = "debugoptimized";
    mesonFlags = [ "-Db_ndebug=false" "-Ddrm-renderers=[]" "-Dvenus=true" "-Dtests=false" "-Dvideo=false" "-Dtracing=none" ];
    env = (old.env or { }) // { NIX_CFLAGS_COMPILE = "-I${angle}/include"; };
    separateDebugInfo = false;
  });

  # QEMU patches in the order of qemu/build.sh: try-omarchy (MIT), then omaflake's own
  qemuPatches = map (p: ./patches/try-omarchy/${p}.patch) [
    "qemu-texture-borrowing-11.1" "qemu-gpu-spike-resolution-fix" "qemu-9p-guest-owner" "qemu-hvf-free-page-reclaim"
    "qemu-hvf-mapped-sections" "qemu-darwin-gpu-fence-poll" "qemu-cocoa-dynamic-display"
  ] ++ map (p: ./patches/omaflake/${p}.patch) [ "qemu-cocoa-initial-size" "qemu-cocoa-dock-icon" "qemu-virgl-2d-flush" ];
in
(qemu.override {
  hostCpuTargets = [ "aarch64-softmmu" ];
  openGLSupport = true;
  virglSupport = true;
  # nixpkgs turns rutabaga on with OpenGL; omaflake uses virglrenderer only (and rutabaga's dylib name points into
  # its build directory on darwin)
  rutabagaSupport = false;
  libepoxy = epoxy;
  virglrenderer = virgl;
  # Only what omaflake uses: HVF, Cocoa, virtio, 9p, slirp
  guestAgentSupport = false;
  spiceSupport = false;
  smartcardSupport = false;
  vncSupport = false;
  capstoneSupport = false;
  tpmSupport = false;
  libiscsiSupport = false;
  enableDocs = false;
}).overrideAttrs (old: {
  pname = "qemu-omaflake";
  patches = (old.patches or [ ]) ++ qemuPatches;
  # qemu/build.sh builds --without-default-features; the D-Bus display that nixpkgs enables does not follow the
  # texture-borrowing patch's interface
  configureFlags = (old.configureFlags or [ ]) ++ [ "--disable-dbus-display" ];
  # openGLSupport adds libgbm and libdrm, which exist on Linux only; on macOS QEMU needs just libepoxy
  buildInputs = lib.filter (d: d != null && !(lib.elem (lib.getName d) [ "libdrm" "libgbm" "mesa-libgbm" ])) old.buildInputs;
  passthru = (old.passthru or { }) // { inherit angle epoxy virgl; };
  meta = (old.meta or { }) // {
    description = "QEMU with 3D (virglrenderer + ANGLE) for omaflake's NixOS guests";
    platforms = [ "aarch64-darwin" ];
    mainProgram = "qemu-system-aarch64";
  };
})
