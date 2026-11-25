#!/bin/sh
# SPDX-License-Identifier: AGPL-3.0-or-later
#
# Checks whether the Linux builder accepts this Mac's builds and, if it does not, gives it a new disk (plan B175).
#
# A builder whose disk has a damaged Nix store can stop trusting the Mac: every NixOS image build then fails with
# "cannot add path … because it lacks a signature by a trusted key". A new disk starts the builder with a fresh
# store; the old disk is moved aside (<disk>.broken-<time>), not deleted.
#
# Works for nix-darwin's builder (launchd org.nixos.linux-builder, asks for sudo) and for omaflake's own builder
# (LaunchAgent app.omaflake.linux-builder, as you). The app's Doctor does the same (Linux builder → Repair …).
#
#   scripts/repair-linux-builder.sh            check; repair after asking when the builder refuses
#   scripts/repair-linux-builder.sh --check    only check
#   scripts/repair-linux-builder.sh --force    repair even when the check passes
set -eu

mode=repair
case "${1:-}" in
  --check) mode=check ;;
  --force) mode=force ;;
  "") ;;
  *) echo "usage: $0 [--check|--force]" >&2; exit 2 ;;
esac

PORT=31022
HOST_KEY=c3NoLWVkMjU1MTkgQUFBQUMzTnphQzFsWkRJMU5URTVBQUFBSUpCV2N4Yi9CbGFxdDFhdU90RStGOFFVV3JVb3RpQzVxQkorVXVFV2RWQ2Igcm9vdEBuaXhvcwo=
DARWIN_PLIST=/Library/LaunchDaemons/org.nixos.linux-builder.plist
OWN_LABEL=app.omaflake.linux-builder
OWN_PLIST="$HOME/Library/LaunchAgents/$OWN_LABEL.plist"

# omaflake's data folder: VM_HOME, else "home" in ~/.omaflake/config.json, else ~/.omaflake
data="${VM_HOME:-}"
if [ -z "$data" ] && [ -f "$HOME/.omaflake/config.json" ]; then
  data=$(/usr/bin/plutil -extract home raw -o - "$HOME/.omaflake/config.json" 2>/dev/null || true)
fi
[ -n "$data" ] || data="$HOME/.omaflake"

if [ -f "$DARWIN_PLIST" ] && grep -qs linux-builder /etc/nix/machines; then
  kind=nix-darwin
  dir=$(/usr/libexec/PlistBuddy -c 'Print :WorkingDirectory' "$DARWIN_PLIST" 2>/dev/null || echo /var/lib/linux-builder)
  builders=""
elif [ -f "$OWN_PLIST" ] && [ -f "$data/builder/builder_ed25519" ]; then
  kind=omaflake
  dir="$data/builder"
  jobs=6
  [ "$(cat "$dir/stage" 2>/dev/null)" = bootstrap ] && jobs=1
  builders="ssh-ng://builder@localhost:$PORT aarch64-linux $dir/builder_ed25519 $jobs 1 kvm,benchmark,big-parallel - $HOST_KEY"
else
  echo "No Linux builder found (neither nix-darwin's nor omaflake's own)." >&2
  exit 1
fi
disk="$dir/nixos.qcow2"
echo "Linux builder: $kind (disk $disk)"

# Builds a tiny Linux derivation whose input was just built on this Mac – unsigned, so the builder takes it only when
# it trusts the Mac. A new nonce every time: the input must not be on the builder yet.
probe() {
  # shellcheck disable=SC2016 # the expression is Nix, not shell
  out=$(OMAFLAKE_PROBE_NONCE="$(date +%s)-$$" nix build --no-link --impure \
    --extra-experimental-features 'nix-command' \
    ${builders:+--builders "$builders"} \
    --expr '
      let
        nonce = builtins.getEnv "OMAFLAKE_PROBE_NONCE";
        input = derivation {
          name = "omaflake-builder-probe-input"; system = "aarch64-darwin"; builder = "/bin/sh";
          args = [ "-c" "echo ${nonce} > $out" ];
        };
      in derivation {
        name = "omaflake-builder-probe"; system = "aarch64-linux"; builder = "/bin/sh";
        args = [ "-c" "read x < ${input}; echo $x > $out" ];
      }' 2>&1) && return 0
  case "$out" in
    *"lacks a signature by a trusted key"*) return 1 ;;
    *) printf '%s\n' "$out" | tail -5 >&2; return 2 ;;
  esac
}

wait_for_builder() {
  i=0
  while ! nc -z -G 2 localhost "$PORT" 2>/dev/null; do
    i=$((i + 1))
    if [ "$i" -gt 90 ]; then echo "The builder does not answer after 3 minutes." >&2; return 1; fi
    sleep 2
  done
  sleep 5
}

echo "Checking whether the builder accepts this Mac's builds …"
if probe; then status=ok; else status=$?; fi
case "$status" in
  ok) echo "✓ The builder accepts this Mac's builds." ;;
  1) echo "✗ The builder refuses this Mac's builds (it does not trust them)." ;;
  *) echo "✗ The check failed for another reason (lines above) – does the builder run?" ;;
esac

[ "$mode" = check ] && { [ "$status" = ok ] && exit 0 || exit 1; }
if [ "$status" = ok ] && [ "$mode" != force ]; then exit 0; fi
if [ "$status" = 2 ] && [ "$mode" != force ]; then
  echo "Not repairing: a new disk only helps when the builder refuses the builds. Use --force to do it anyway." >&2
  exit 1
fi

stamp=$(date +%Y%m%d-%H%M%S)
printf 'Give the builder a new disk? The old one is moved to %s.broken-%s (delete it later). [y/N] ' "$disk" "$stamp"
read -r answer
case "$answer" in y|Y|j|J) ;; *) echo "Nothing changed."; exit 1 ;; esac

if [ "$kind" = nix-darwin ]; then
  sudo launchctl bootout system/org.nixos.linux-builder 2>/dev/null || true
  sudo sh -c '[ ! -e "$1" ] || mv "$1" "$1.broken-$2"' sh "$disk" "$stamp"
  sudo launchctl bootstrap system "$DARWIN_PLIST"
else
  launchctl bootout "gui/$(id -u)/$OWN_LABEL" 2>/dev/null || true
  [ ! -e "$disk" ] || mv "$disk" "$disk.broken-$stamp"
  launchctl bootstrap "gui/$(id -u)" "$OWN_PLIST"
fi

echo "Waiting for the builder (its first start with a new disk takes a minute or two) …"
wait_for_builder
echo "Checking again …"
if probe; then
  echo "✓ Repaired: the builder accepts this Mac's builds. Build the image again."
else
  echo "✗ The builder still refuses the builds – the cause is elsewhere (OPEN). Its old disk: $disk.broken-$stamp" >&2
  exit 1
fi
