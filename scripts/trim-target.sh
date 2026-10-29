#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Trims target/ (and nix/agent/target/) without a full `cargo clean`: an interrupted rustc leaves its
# codegen objects (*.o) in deps/, and every changed configuration adds incremental sessions cargo never removes.
# Removes those objects and incremental sessions untouched for DAYS days (default 3); the next build only
# recompiles what it needs. Build caches only – nothing here goes to _to_delete.
# Usage: scripts/trim-target.sh [DAYS]   (skipped while cargo or rustc runs)
set -euo pipefail
root=$(cd "$(dirname "$0")/.." && pwd)
days=${1:-3}
if pgrep -x cargo >/dev/null || pgrep -x rustc >/dev/null; then
  echo "cargo/rustc is running, not trimming" >&2
  exit 0
fi
for target in "$root/target" "$root/nix/agent/target"; do
  [[ -d $target ]] || continue
  before=$(du -sk "$target" | cut -f1)
  for profile in "$target"/*/; do
    [[ -d $profile/deps ]] && find "$profile/deps" -maxdepth 1 -name '*.o' -type f -delete
    [[ -d $profile/incremental ]] && find "$profile/incremental" -mindepth 1 -maxdepth 1 -type d -mtime +"$days" \
      -exec rm -rf {} +
  done
  after=$(du -sk "$target" | cut -f1)
  echo "${target#"$root"/}: $((before / 1024)) MB -> $((after / 1024)) MB"
done
