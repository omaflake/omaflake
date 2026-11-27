#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Starts the development build of omaflake (build/omaflake.app) with its own data folder ~/.omaflake-dev: its own core
# service and socket, the checkout's image flake (nix/) and the catalog checkout next to this one (../catalog) as the
# first recipe source. The installed omaflake and ~/.omaflake stay untouched. Build it first with:
#   VM_HOME=~/.omaflake-dev scripts/bundle-app.sh
# Works on any machine that sees the checkout (the Mac or a VM with it as a volume): on the first start (no
# ~/.omaflake-dev/config.json yet) it sets this machine's paths of nix/ and ../catalog; the remote omaflake/catalog
# stays in the list, switched off.
set -euo pipefail
repo=$(cd "$(dirname "$0")/.." && pwd)
dev="$HOME/.omaflake-dev"
app="$repo/build/omaflake.app"
omaf="$app/Contents/MacOS/omaf"
catalog="$repo/../catalog"
[[ -x $omaf ]] || { echo "No development build at $app – build it first (see above)." >&2; exit 1; }
if [[ ! -f $dev/config.json ]]; then
  [[ -d $catalog ]] || { echo "No catalog checkout at $catalog – clone omaflake/catalog there first." >&2; exit 1; }
  echo "First start: setting up $dev"
  VM_HOME="$dev" "$omaf" config --flake "$repo/nix" >/dev/null
  VM_HOME="$dev" "$omaf" sources add "$(cd "$catalog" && pwd)" --first
  # The official remote source, when the list has it, stays but is switched off
  if VM_HOME="$dev" "$omaf" sources list | grep -q 'github omaflake/catalog'; then
    n=$(VM_HOME="$dev" "$omaf" sources list | awk '/github omaflake\/catalog/ {print $1; exit}')
    VM_HOME="$dev" "$omaf" sources disable "$n" >/dev/null
  fi
  VM_HOME="$dev" "$omaf" sources list
fi
open -n --env VM_HOME="$dev" "$app"
