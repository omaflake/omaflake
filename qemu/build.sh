#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Custom QEMU build with 3D acceleration (spec M2, option A):
#   ANGLE (prebuilt, startergo) → libepoxy (source + EGL 1.5 patch) → virglrenderer 1.3.0
#   (source + startergo patches + native OpenGL patch) → QEMU 11.1.2 (source + patches from
#   try-omarchy) → signed ad hoc with the hypervisor entitlement.
#
# Usage: qemu/build.sh            → ~/.omaflake/qemu/<tag>/, symlink ~/.omaflake/qemu/current
# Then:  omaf config --qemu ~/.omaflake/qemu/current/bin/qemu-system-aarch64
# The build is relocatable (qemu/relocate.sh): its own libraries are found via @rpath.
#
# Runtime libraries (glib, pixman, libslirp, dtc) come from Homebrew – for local use only,
# not for redistribution. All downloads are pinned by sha256.
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)
# QEMU_BUILD_BASE: different target, e.g. for a verification build next to the ~/.omaflake/qemu in use
base=${QEMU_BUILD_BASE:-${VM_HOME:-$HOME/.omaflake}/qemu}
downloads=$base/src/downloads
jobs=${QEMU_BUILD_JOBS:-$(sysctl -n hw.ncpu)}

qemu_version=11.1.2
virgl_version=1.3.0
angle_version=1.0.16
epoxy_commit=1b6d7db184bb1a0d9af0e200e06a0331028eaaae
patchset=4   # bump when the patch selection changes (new folder, the old one stays usable)
tag="qemu-$qemu_version-virgl$virgl_version-angle$angle_version-p$patchset"
prefix=$base/$tag   # complete only once $prefix/.complete exists
brew_prefix=$(brew --prefix)

# Name  URL  sha256  (sha256 of ANGLE, virgl and the taps identical to the pins in try-omarchy
# or the startergo formulae; QEMU and libepoxy recorded on first download, 2026-10-16)
sources=$(cat <<EOF
qemu-$qemu_version.tar.xz	https://download.qemu.org/qemu-$qemu_version.tar.xz	731b5681e4bb18be313231579b8efd0296c5b015fa36dc533874b639ba838016
virglrenderer-$virgl_version.tar.gz	https://gitlab.freedesktop.org/virgl/virglrenderer/-/archive/$virgl_version/virglrenderer-$virgl_version.tar.gz	065bc56e89e6f631f96101cd62eba0748e48eb888b434edc86e89d05395e76f3
homebrew-virglrenderer-1.0.42.tar.gz	https://codeload.github.com/startergo/homebrew-virglrenderer/tar.gz/refs/tags/v1.0.42	950273fbba46905b6112ee2bd0598c1da706c25319a7347058cbc52f04ba96dd
libepoxy-${epoxy_commit:0:7}.tar.gz	https://github.com/anholt/libepoxy/archive/$epoxy_commit.tar.gz	15f769a8f24c361c8de28d72625daf07d0107b683fa0a5118d0aa8b0c5fc9eab
homebrew-libepoxy-1.0.5.tar.gz	https://github.com/startergo/homebrew-libepoxy/archive/refs/tags/v1.0.5.tar.gz	24a1d98c0c4900f385bbfe8fc70d725ba8c994c5fac1544aa4e464a083481d59
angle-$angle_version.arm64_sequoia.bottle.tar.gz	https://github.com/startergo/homebrew-angle/releases/download/v$angle_version/angle-$angle_version.arm64_sequoia.bottle.tar.gz	29fe2175b157a65f12879f9a12b5c8f94d0a76fafdf41ff009a2fdb4e9df525c
EOF
)

# QEMU patches from try-omarchy (MIT, patches/try-omarchy/LICENSE), in their order
qemu_patches=(
  qemu-texture-borrowing-11.1   # GL textures from virgl straight to Cocoa (core of 3D output)
  qemu-gpu-spike-resolution-fix # Cocoa: resolution jumps
  qemu-9p-guest-owner           # 9p: show Mac files as owned by the guest user (plan B18)
  qemu-hvf-free-page-reclaim    # HVF: return freed guest RAM
  qemu-hvf-mapped-sections      # HVF: memory mapping
  qemu-darwin-gpu-fence-poll    # virtio-gpu: poll fences on macOS
  qemu-cocoa-dynamic-display    # window size/Retina → guest resolution (EDID DPI), 16:9 initial window (p2)
)
# Own patches (patches/omaflake/), applied after those from try-omarchy
own_patches=(
  qemu-cocoa-initial-size       # zoom-to-fit: window follows guest resolution until the user resizes it (p3)
  qemu-cocoa-dock-icon          # Dock icon from OMAFLAKE_DOCK_ICON (set by omaf/the app for window starts) (p4)
)
# virglrenderer patches from startergo/homebrew-virglrenderer v1.0.42 (list as in their formula)
virgl_patches=(
  virglrenderer-debug-init-logging virglrenderer-default-debug-log virglrenderer-macos-unified
  virglrenderer-venus-metal-func-ptrs virglrenderer-gallium-endian virglrenderer-macos-a8-swizzle
  virglrenderer-corefoundation-link virglrenderer-a8-shader-swizzle virglrenderer-a8-shader-swizzle-texture
  virglrenderer-a8-unpack-alignment virglrenderer-bgra-upload-swizzle-core virglrenderer-msaa-assertion-fix
  virglrenderer-ignore-surface0-clear virglrenderer-venus-errno-debug virglrenderer-macos-profile-forcing
  virglrenderer-macos-egl-profile virglrenderer-texture-swizzle-core virglrenderer-bgra-unified
  virglrenderer-core-profile-frag-datalocation virglrenderer-macos-core-profile-fixes
  virglrenderer-gles-dual-source-output
)

log() { printf '\n[qemu-build] %s\n' "$*"; }
die() { printf '[qemu-build] ERROR: %s\n' "$*" >&2; exit 1; }

for tool in curl shasum meson ninja pkg-config python3 codesign install_name_tool patch; do
  command -v "$tool" >/dev/null || die "$tool missing (brew install meson ninja pkgconf)"
done
for f in glib pixman libslirp dtc; do
  brew list --versions "$f" >/dev/null 2>&1 || die "Homebrew package $f missing (brew install $f)"
done
[[ ! -e $prefix/.complete ]] || die "$prefix already exists – move it to ~/.omaflake/_to_delete to rebuild"
if [[ -e $prefix ]]; then
  # Leftover of an aborted run: do not delete, move it to the trash folder
  trash=${VM_HOME:-$HOME/.omaflake}/_to_delete/$(date +%Y%m%d-%H%M%S)-$tag-aborted
  log "Moving leftover of an aborted build to $trash"
  mkdir -p "$(dirname "$trash")" && mv "$prefix" "$trash"
fi

# --- Downloads -------------------------------------------------------------
mkdir -p "$downloads"
while IFS=$'\t' read -r name url sha; do
  file=$downloads/$name
  [[ -f $file ]] || { log "Downloading $name"; curl -fsSL -o "$file.part" "$url"; mv "$file.part" "$file"; }
  echo "$sha  $file" | shasum -a 256 -c --quiet - || die "Checksum mismatch for $name"
done <<<"$sources"

work=$base/src/build-$(date +%Y%m%d-%H%M%S)
mkdir -p "$work" "$prefix/lib/pkgconfig"
log "Work dir $work, target $prefix"
cd "$work"
tar -xf "$downloads/qemu-$qemu_version.tar.xz"
tar -xzf "$downloads/virglrenderer-$virgl_version.tar.gz"
tar -xzf "$downloads/homebrew-virglrenderer-1.0.42.tar.gz"
tar -xzf "$downloads/libepoxy-${epoxy_commit:0:7}.tar.gz"
tar -xzf "$downloads/homebrew-libepoxy-1.0.5.tar.gz"
tar -xzf "$downloads/angle-$angle_version.arm64_sequoia.bottle.tar.gz"

# pyyaml for virglrenderer (gallium), in its own venv
python3 -m venv "$work/venv"
"$work/venv/bin/pip" install --quiet pyyaml==6.0.3
export PATH="$work/venv/bin:$PATH"
export PKG_CONFIG_PATH="$prefix/lib/pkgconfig:$brew_prefix/lib/pkgconfig"
export LDFLAGS="-Wl,-headerpad_max_install_names -Wl,-rpath,$prefix/lib"

# --- ANGLE (prebuilt) ------------------------------------------------------
log "Installing ANGLE $angle_version"
angle=$work/angle/$angle_version
cp -R "$angle/lib/." "$prefix/lib/"
mkdir -p "$prefix/include" && cp -R "$angle/include/." "$prefix/include/"
chmod -R u+w "$prefix"   # bottle files are read-only (sed, xattr, codesign need write access)
sed -i '' "s|^prefix=.*|prefix=$prefix|" "$prefix"/lib/pkgconfig/{egl,glesv1_cm,glesv2}.pc

# --- libepoxy ----------------------------------------------------------------
log "Building libepoxy (${epoxy_commit:0:7})"
epoxy=$work/libepoxy-$epoxy_commit
patch -d "$epoxy" -p1 -f -s -i "$work/homebrew-libepoxy-1.0.5/patches/libepoxy-akihikodaki-egl15.patch"
meson setup "$epoxy/build" "$epoxy" --prefix="$prefix" --libdir=lib --buildtype=release \
  -Dc_args="-I$prefix/include" -Degl=yes -Dx11=false -Dtests=false
ninja -j "$jobs" -C "$epoxy/build" && meson install -C "$epoxy/build" --quiet

# --- virglrenderer -----------------------------------------------------------
log "Building virglrenderer $virgl_version"
virgl=$work/virglrenderer-$virgl_version
for p in "${virgl_patches[@]}"; do
  patch -d "$virgl" -p1 -f -s -i "$work/homebrew-virglrenderer-1.0.42/patches/$p.patch"
done
patch -d "$virgl" -p1 -f -s -i "$here/patches/try-omarchy/virgl-native-opengl.patch"
meson setup "$virgl/build" "$virgl" --prefix="$prefix" --libdir=lib --buildtype=debugoptimized -Db_ndebug=false \
  -Dc_args="-I$prefix/include" -Dcpp_args="-I$prefix/include" \
  -Ddrm-renderers=[] -Dvenus=true -Dtests=false -Dvideo=false -Dtracing=none
ninja -j "$jobs" -C "$virgl/build" && meson install -C "$virgl/build" --quiet

# --- QEMU --------------------------------------------------------------------
log "Building QEMU $qemu_version"
qemu=$work/qemu-$qemu_version
for p in "${qemu_patches[@]}"; do
  patch -d "$qemu" -p1 -f -s -i "$here/patches/try-omarchy/$p.patch"
done
for p in "${own_patches[@]}"; do
  patch -d "$qemu" -p1 -f -s -i "$here/patches/omaflake/$p.patch"
done
mkdir "$qemu/build"
(
  cd "$qemu/build"
  ../configure --prefix="$prefix" --target-list=aarch64-softmmu --without-default-features \
    --enable-system --enable-tools --enable-hvf --disable-tcg \
    --enable-cocoa --enable-opengl --enable-virglrenderer --enable-pixman \
    --enable-slirp --enable-fdt=system --enable-virtfs --enable-bzip2 \
    --disable-debug-info --disable-werror \
    --extra-cflags="-I$prefix/include -I$brew_prefix/include" --extra-ldflags="$LDFLAGS -L$brew_prefix/lib"
)
ninja -j "$jobs" -C "$qemu/build" && ninja -C "$qemu/build" install >/dev/null

# --- Sign & verify -----------------------------------------------------------
log "Making it relocatable and signing (ad hoc, hypervisor entitlement)"
"$here/relocate.sh" "$prefix"

"$prefix/bin/qemu-system-aarch64" -device help | grep -q '"virtio-gpu-gl-pci"' || die "virtio-gpu-gl-pci missing from the build"
[[ -f $prefix/share/qemu/edk2-aarch64-code.fd ]] || die "UEFI firmware missing from the build"

touch "$prefix/.complete"
ln -sfn "$tag" "$base/current"
log "Done: $prefix ($("$prefix/bin/qemu-system-aarch64" --version | head -1))"
echo "Activate: omaf config --qemu $base/current/bin/qemu-system-aarch64"
echo "Work dir $work can be moved to ~/.omaflake/_to_delete."
