#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Gives an app bundle omaflake's Liquid Glass icon (macOS 26+; plan B196): compiles brand/macos/omaflake.icon – an Icon
# Composer document, a copy of omaflake/brand's – with actool into <app>/Contents/Resources/Assets.car and names it in
# the bundle's Info.plist (CFBundleIconName, as actool's partial Info.plist gives it). The bundle's omaflake.icns
# (CFBundleIconFile) stays as it is – omaflake/brand's own, with the small glyph in the 16/32 px slots – for whatever
# reads only that; actool writes no loose icon beside the catalog (--standalone-icon-behavior none).
# Needs Xcode's actool (checked with Xcode 27.2; the release and CI images have Xcode 27). With the Command Line Tools
# alone there is none: then the bundle keeps only its .icns, with a warning – the build goes on.
# Call it before signing the bundle. Usage: scripts/compile-icon.sh <app bundle>
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
app=$1
work=$(mktemp -d)
minimum=$(plutil -extract LSMinimumSystemVersion raw "$root/App/Info.plist")
if xcrun actool "$root/brand/macos/omaflake.icon" --compile "$work" --platform macosx \
     --minimum-deployment-target "$minimum" --app-icon omaflake --standalone-icon-behavior none \
     --output-partial-info-plist "$work/partial.plist" --errors --warnings >"$work/actool.log" 2>&1 \
   && [[ -s $work/Assets.car ]]; then
  mkdir -p "$app/Contents/Resources"
  cp "$work/Assets.car" "$app/Contents/Resources/Assets.car"
  plutil -replace CFBundleIconName -string "$(plutil -extract CFBundleIconName raw "$work/partial.plist")" \
    "$app/Contents/Info.plist"
else
  echo "warning: $(basename "$app") gets no Liquid Glass icon, only omaflake.icns – actool (Xcode) could not compile" \
    "brand/macos/omaflake.icon:" >&2
  sed 's/^/  /' "$work/actool.log" >&2
fi
