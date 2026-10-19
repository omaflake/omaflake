#!/usr/bin/env bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Build omaf-runner and sign it ad hoc with com.apple.security.virtualization (plan §7.4).
# Usage: scripts/sign-runner.sh [debug|release]   (last output line: path of the signed binary)
set -euo pipefail
root="$(cd "$(dirname "$0")/.." && pwd)"
conf="${1:-debug}"
swift build --package-path "$root" -c "$conf" --product omaf-runner
bin="$(swift build --package-path "$root" -c "$conf" --show-bin-path)/omaf-runner"
codesign --force --sign - --entitlements "$root/App/omaf-runner.entitlements" "$bin"
codesign -d --entitlements - "$bin" 2>/dev/null | grep -q com.apple.security.virtualization \
  || { echo "Entitlement missing after signing" >&2; exit 1; }
echo "$bin"
