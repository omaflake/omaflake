#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Builds "omaflake Tools.app" (release) into <repo>/build/: omaflake's app in a macOS guest (/Applications), with
# omaflake's camera extension inside (Contents/Library/SystemExtensions; plan B184) and its FSKit module
# (Contents/Extensions; plan B190). scripts/bundle-app.sh calls
# this and puts the result into omaflake.app.
#
# Two builds until there is a Developer ID (plan B184): a camera extension is a system extension, and the app's
# entitlement to install one (com.apple.developer.system-extension.install) is restricted – ad hoc signed, macOS
# kills an app that carries it at once, except in a guest with System Integrity Protection off (checked
# 2026-11-26). So ad hoc this script builds
#   build/omaflake Tools.app             for every guest: without that entitlement and without the extension
#   build/unchained/omaflake Tools.app   for unchained guests: with both
# – the same app, the same identifiers. With SIGN_IDENTITY="Developer ID Application: …" and TEAM_ID=<team id> there
# is one build, with the extension, for every guest; whether that also needs a provisioning profile in the bundle:
# OPEN until there is a Developer ID (M12).
#
# The extension's app group is also its Mach service's name (CoreMediaIO looks it up by that); with a team it must
# start with the team id.
# Usage: scripts/bundle-tools.sh   (prints the folder with the bundle(s) as its last line: <repo>/build)
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
swift_paths=()
[[ -n ${SWIFT_SCRATCH_PATH:-} ]] && swift_paths=(--scratch-path "$SWIFT_SCRATCH_PATH")
swift build ${swift_paths[@]+"${swift_paths[@]}"} -c release --product omaflake-camera
swift build ${swift_paths[@]+"${swift_paths[@]}"} -c release --product omaflake-tools
bindir=$(swift build ${swift_paths[@]+"${swift_paths[@]}"} -c release --show-bin-path)

identity=${SIGN_IDENTITY:--}
release_flags=()
version=$(plutil -extract CFBundleShortVersionString raw "$root/App/Info.plist")
if [[ $identity == - ]]; then
  # A development build replaces the one before only when its version differs: the time
  build=$(date +%Y%m%d.%H%M%S)
  group=app.omaflake.guest.camera
else
  [[ -n ${TEAM_ID:-} ]] || { echo "SIGN_IDENTITY needs TEAM_ID (the app group's prefix)" >&2; exit 1; }
  release_flags=(--options runtime --timestamp)
  build=$version
  group=$TEAM_ID.app.omaflake.guest.camera
fi
work=$(mktemp -d)
# omaflake's FSKit module (plan B190): swiftc, not SwiftPM – entry point NSExtensionMain and no `main` of its own, as in
# Apple's modules; a SwiftPM executable of the same source trapped in ExtensionFoundation (Package.swift)
xcrun swiftc -target arm64-apple-macos27.0 -parse-as-library -O -module-name omaflake_volumes \
  "$root"/Sources/omaflake-volumes/*.swift -o "$work/omaflake-volumes" -Xlinker -e -Xlinker _NSExtensionMain
cp "$root/App/omaflake-camera.entitlements" "$work/extension.entitlements"
plutil -insert com\\.apple\\.security\\.application-groups -json "[\"$group\"]" "$work/extension.entitlements"

# bundle <folder> <with the camera extension: yes|no>
bundle() {
  local app="$1/omaflake Tools.app" extension=$2
  local ext="$app/Contents/Library/SystemExtensions/app.omaflake.guest.camera.systemextension"
  local volumes="$app/Contents/Extensions/omaflake-volumes.appex"
  if [[ -e $app ]]; then
    local trash
    trash=${VM_HOME:-$HOME/.omaflake}/_to_delete/$(date +%Y%m%d-%H%M%S)-$RANDOM-omaflake-Tools.app
    mkdir -p "$(dirname "$trash")" && mv "$app" "$trash"
  fi
  mkdir -p "$app/Contents/MacOS"
  cp "$bindir/omaflake-tools" "$app/Contents/MacOS/omaflake-tools"
  cp "$root/App/omaflake-tools-Info.plist" "$app/Contents/Info.plist"
  # omaflake's icon (a copy of omaflake/brand's, like the app's)
  mkdir -p "$app/Contents/Resources"
  cp "$root/brand/macos/omaflake.icns" "$app/Contents/Resources/omaflake.icns"
  plutil -insert CFBundleShortVersionString -string "$version" "$app/Contents/Info.plist"
  plutil -insert CFBundleVersion -string "$build" "$app/Contents/Info.plist"
  "$root/scripts/compile-icon.sh" "$app"   # Liquid Glass on macOS 26+ (Assets.car, CFBundleIconName; plan B196)
  if [[ $extension == yes ]]; then
    mkdir -p "$ext/Contents/MacOS"
    cp "$bindir/omaflake-camera" "$ext/Contents/MacOS/omaflake-camera"
    cp "$root/App/omaflake-camera-Info.plist" "$ext/Contents/Info.plist"
    plutil -insert CFBundleShortVersionString -string "$version" "$ext/Contents/Info.plist"
    plutil -insert CFBundleVersion -string "$build" "$ext/Contents/Info.plist"
    plutil -insert CMIOExtension -json "{\"CMIOExtensionMachServiceName\": \"$group\"}" "$ext/Contents/Info.plist"
    # omaflake's FSKit module (plan B190) – its entitlement is restricted like the camera's, so it goes with it
    mkdir -p "$volumes/Contents/MacOS"
    cp "$work/omaflake-volumes" "$volumes/Contents/MacOS/omaflake-volumes"
    cp "$root/App/omaflake-volumes-Info.plist" "$volumes/Contents/Info.plist"
    plutil -insert CFBundleShortVersionString -string "$version" "$volumes/Contents/Info.plist"
    plutil -insert CFBundleVersion -string "$build" "$volumes/Contents/Info.plist"
    # Inside out: the extensions first, then the app
    codesign --force --sign "$identity" ${release_flags[@]+"${release_flags[@]}"} --entitlements "$work/extension.entitlements" "$ext"
    codesign --force --sign "$identity" ${release_flags[@]+"${release_flags[@]}"} --entitlements "$root/App/omaflake-volumes.entitlements" "$volumes"
    codesign --force --sign "$identity" ${release_flags[@]+"${release_flags[@]}"} --entitlements "$root/App/omaflake-tools.entitlements" "$app"
    codesign -d --entitlements - "$ext" 2>/dev/null | grep -q "$group" \
      || { echo "the camera extension lacks its app group" >&2; exit 1; }
    codesign -d --entitlements - "$volumes" 2>/dev/null | grep -q com.apple.developer.fskit.fsmodule \
      || { echo "the FSKit module lacks its entitlement" >&2; exit 1; }
    codesign -d --entitlements - "$app" 2>/dev/null | grep -q com.apple.developer.system-extension.install \
      || { echo "omaflake Tools.app lacks its entitlement" >&2; exit 1; }
  else
    codesign --force --sign "$identity" ${release_flags[@]+"${release_flags[@]}"} "$app"
    ! codesign -d --entitlements - "$app" 2>/dev/null | grep -q com.apple.developer.system-extension.install \
      || { echo "the plain omaflake Tools.app must not carry the restricted entitlement" >&2; exit 1; }
  fi
  codesign --verify --strict "$app"
  echo "Built: $app ($version, build $build$([[ $extension == yes ]] && echo ", with the camera extension and the FSKit module"))" >&2
}

if [[ $identity == - ]]; then
  bundle "$root/build" no
  bundle "$root/build/unchained" yes
else
  # One signed build for every guest; a leftover ad hoc one for unchained guests must not ship beside it
  if [[ -e $root/build/unchained ]]; then
    trash=${VM_HOME:-$HOME/.omaflake}/_to_delete/$(date +%Y%m%d-%H%M%S)-unchained-Tools
    mkdir -p "$(dirname "$trash")" && mv "$root/build/unchained" "$trash"
  fi
  bundle "$root/build" yes
fi
echo "$root/build"
