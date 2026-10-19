#!/bin/bash
# SPDX-License-Identifier: AGPL-3.0-or-later
# Headless boot probe (spec M1, succeeded 2026-10-16): overlay on top of an image, QEMU without display,
# SSH via port forwarding. Reference for QEMUCommand in the omaf CLI (M3).
# Usage: scripts/bootprobe.sh <image.qcow2> ~/.omaflake/instances/<name> [ssh-port]
# Keep DIR short: Unix socket paths are limited to 103 bytes (macOS).
set -euo pipefail
IMG="$1"; DIR="$2"; PORT="${3:-2222}"
FW=/opt/homebrew/opt/qemu/share/qemu/edk2-aarch64-code.fd
mkdir -p "$DIR"
qemu-img create -q -f qcow2 -b "$IMG" -F qcow2 "$DIR/disk.qcow2" 40G
dd if=/dev/zero of="$DIR/efivars.fd" bs=1m count=64 status=none
qemu-system-aarch64 \
  -machine virt,highmem=on -accel hvf -cpu host -smp 4 -m 4G \
  -drive if=pflash,format=raw,readonly=on,file="$FW" \
  -drive if=pflash,format=raw,file="$DIR/efivars.fd" \
  -drive if=virtio,format=qcow2,file="$DIR/disk.qcow2" \
  -device virtio-gpu-pci -display none \
  -netdev user,id=n0,hostfwd=tcp:127.0.0.1:$PORT-:22 -device virtio-net-pci,netdev=n0 \
  -qmp unix:"$DIR/qmp.sock",server=on,wait=off \
  -serial file:"$DIR/serial.log" \
  -pidfile "$DIR/qemu.pid" -daemonize
echo "started, PID $(cat "$DIR/qemu.pid")"
