#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Notarizes and staples a Developer ID signed build/omaflake.app (scripts/bundle-app.sh with SIGN_IDENTITY), then packs
# build/omaflake-<version>.zip and its .sha256 for a release. Needs Xcode's notarytool and an App Store Connect API key:
#   NOTARY_KEY_PATH (the .p8 file), NOTARY_KEY_ID, NOTARY_ISSUER_ID
# Untested until the Developer ID account exists (plan B67, spec §11.4).
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
app=$root/build/omaflake.app
version=$(plutil -extract CFBundleShortVersionString raw "$app/Contents/Info.plist")
zip=$root/build/omaflake-$version.zip
: "${NOTARY_KEY_PATH:?}" "${NOTARY_KEY_ID:?}" "${NOTARY_ISSUER_ID:?}"
codesign --verify --strict --deep "$app"
ditto -c -k --keepParent "$app" "$zip"
xcrun notarytool submit "$zip" --key "$NOTARY_KEY_PATH" --key-id "$NOTARY_KEY_ID" --issuer "$NOTARY_ISSUER_ID" --wait
xcrun stapler staple "$app"
# Zip again, now with the stapled ticket inside
mv "$zip" "$root/build/.notarized-upload.zip"
ditto -c -k --keepParent "$app" "$zip"
(cd "$root/build" && shasum -a 256 "$(basename "$zip")" > "$(basename "$zip").sha256")
echo "Notarized: $zip"
