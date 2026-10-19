#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Makes a QEMU build from qemu/build.sh relocatable and signs it again (ad hoc, hypervisor entitlement):
# references to the build's own libraries become @rpath/<lib>, with rpath @executable_path/../lib (binaries) and
# @loader_path (libraries). The folder can then be moved, e.g. from ~/VMs/qemu to ~/.omaflake/qemu.
# Homebrew and system libraries keep their absolute paths.
#
# Usage: qemu/relocate.sh <prefix>     (folder with bin/ and lib/; called by build.sh, safe to run again)
set -euo pipefail
here=$(cd "$(dirname "$0")" && pwd)
prefix=$(cd "${1:?usage: $(basename "$0") <prefix>}" && pwd)
lib=$prefix/lib
[[ -x $prefix/bin/qemu-system-aarch64 ]] || { echo "no QEMU build in $prefix" >&2; exit 1; }

own() { [[ $1 == /* && $1 != /usr/* && $1 != /System/* && $1 != "$(brew --prefix)"/* && -e $lib/$(basename "$1") ]]; }

fix() {   # <file> <rpath to add>
  local f=$1 rp=$2 dep path
  chmod u+w "$f"
  while read -r dep; do
    own "$dep" && install_name_tool -change "$dep" "@rpath/$(basename "$dep")" "$f" 2>/dev/null
  done < <(otool -L "$f" | tail -n +2 | awk '{print $1}')
  while read -r path; do
    [[ $path == /* ]] && install_name_tool -delete_rpath "$path" "$f" 2>/dev/null
  done < <(otool -l "$f" | awk '/LC_RPATH/{getline; getline; print $2}')
  otool -l "$f" | awk '/LC_RPATH/{getline; getline; print $2}' | grep -qx "$rp" || install_name_tool -add_rpath "$rp" "$f" 2>/dev/null
}

for f in "$lib"/*.dylib; do
  [[ -L $f ]] && continue
  fix "$f" @loader_path
  own "$(otool -D "$f" | tail -1)" && install_name_tool -id "@rpath/$(basename "$f")" "$f" 2>/dev/null
done
for f in "$prefix"/bin/*; do
  [[ -L $f ]] || ! file "$f" | grep -q Mach-O || fix "$f" @executable_path/../lib
done

# install_name_tool invalidates signatures: libraries first, then the binaries (QEMU with its entitlement)
xattr -cr "$prefix/bin" "$lib"
for f in "$lib"/*.dylib; do [[ -L $f ]] || codesign --force --sign - "$f"; done
for f in "$prefix"/bin/*; do
  [[ -L $f ]] || ! file "$f" | grep -q Mach-O || codesign --force --sign - "$f"
done
codesign --force --sign - --entitlements "$here/entitlements.plist" "$prefix/bin/qemu-system-aarch64"
codesign -d --entitlements - "$prefix/bin/qemu-system-aarch64" 2>/dev/null | grep -q com.apple.security.hypervisor \
  || { echo "Hypervisor entitlement missing after signing" >&2; exit 1; }
if otool -L "$prefix/bin/qemu-system-aarch64" | tail -n +2 | awk '{print $1}' | grep -q "^$prefix"; then
  echo "absolute references to $prefix remain" >&2; exit 1
fi
echo "Relocatable: $prefix"
