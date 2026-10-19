#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Builds omaflake.app (release) into <repo>/build/ and signs it ad hoc.
# No App Sandbox: the app launches QEMU/nix/ssh and works in ~/.omaflake (plan §1.3 item 8).
# omaf-runner is embedded as its own helper app, Contents/Helpers/omaf-runner.app (found there by
# VZBackend.locateRunner) – a runner with a VM window inside Contents/MacOS made macOS take it for omaflake.app
# (plan B65). It is signed first, with its own entitlement com.apple.security.virtualization; the app itself needs none.
# Usage: scripts/bundle-app.sh [--install]   (--install: copy to ~/Applications)
# Signing: ad hoc by default; SIGN_IDENTITY="Developer ID Application: …" signs with that identity, the hardened
# runtime and a secure timestamp (release builds, .github/workflows/release.yml; notarize with scripts/notarize.sh).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
cd "$root"
swift build -c release --product OmaflakeApp
swift build -c release --product omaf-runner
bindir=$(swift build -c release --show-bin-path)
bin=$bindir/OmaflakeApp
app=$root/build/omaflake.app
# Do not delete the old bundle, move it to the trash folder
if [[ -e $app ]]; then
  trash=${VM_HOME:-$HOME/.omaflake}/_to_delete/$(date +%Y%m%d-%H%M%S)-omaflake.app
  mkdir -p "$(dirname "$trash")" && mv "$app" "$trash"
fi
mkdir -p "$app/Contents/MacOS" "$app/Contents/Resources"
cp "$bin" "$app/Contents/MacOS/omaflake"
helper=$app/Contents/Helpers/omaf-runner.app
mkdir -p "$helper/Contents/MacOS"
cp "$bindir/omaf-runner" "$helper/Contents/MacOS/omaf-runner"
cp "$root/App/omaf-runner-Info.plist" "$helper/Contents/Info.plist"
plutil -insert CFBundleExecutable -string omaf-runner "$helper/Contents/Info.plist"
plutil -insert CFBundlePackageType -string APPL "$helper/Contents/Info.plist"
plutil -replace CFBundleName -string "omaflake VM" "$helper/Contents/Info.plist"   # name in the Dock and menu bar
plutil -insert CFBundleShortVersionString -string "$(plutil -extract CFBundleShortVersionString raw "$root/App/Info.plist")" "$helper/Contents/Info.plist"
cp "$root/App/Info.plist" "$app/Contents/Info.plist"
cp "$root/brand/macos/omaflake.icns" "$app/Contents/Resources/omaflake.icns"
cp -R "$root"/App/Resources/*.lproj "$app/Contents/Resources/"   # UI languages
# The image flake (nix/): the app's default when no flake path is set – recipes come from sources (plan B107).
# Without local/ (each developer's own), build output and Finder files.
rsync -a --exclude local/ --exclude agent/target/ --exclude 'result*' --exclude .DS_Store "$root/nix/" "$app/Contents/Resources/nix/"
mkdir -p "$app/Contents/Resources/Guides"                          # guides shown in the app (offline)
cp "$root/docs/linux-builder.md" "$app/Contents/Resources/Guides/"
# Inside out: the nested runner first, then the bundle (without --deep, which would drop the runner's entitlement)
identity=${SIGN_IDENTITY:--}
release_flags=()
[[ $identity == - ]] || release_flags=(--options runtime --timestamp)
codesign --force --sign "$identity" ${release_flags[@]+"${release_flags[@]}"} --entitlements "$root/App/omaf-runner.entitlements" "$helper"
codesign --force --sign "$identity" ${release_flags[@]+"${release_flags[@]}"} "$app"
codesign -d --entitlements - "$helper/Contents/MacOS/omaf-runner" 2>/dev/null | grep -q com.apple.security.virtualization \
  || { echo "omaf-runner in the bundle lacks its entitlement" >&2; exit 1; }
codesign --verify --strict "$app"
echo "Built: $app"
if [[ ${1:-} == --install ]]; then
  dest=$HOME/Applications/omaflake.app
  if [[ -e $dest ]]; then
    trash=${VM_HOME:-$HOME/.omaflake}/_to_delete/$(date +%Y%m%d-%H%M%S)-omaflake.app-installed
    mkdir -p "$(dirname "$trash")" && mv "$dest" "$trash"
  fi
  mkdir -p "$HOME/Applications" && cp -R "$app" "$dest"
  echo "Installed: $dest"
fi
