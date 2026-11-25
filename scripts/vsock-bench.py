#!/usr/bin/env python3
# SPDX-License-Identifier: AGPL-3.0-or-later
"""Plan B174, in a macOS guest: how fast vsock brings data from the Mac. Needs a runner started while
~/.omaflake/developer/vsock-bench exists on the Mac (the VM then has a virtio socket device, the runner listens on port
47100 and sends for 10 s). Prints MB/s and the raw 1280x720 NV12 camera frames a second that would be; the agent's
virtio console carries ~35 MB/s (~25 frames)."""
import ctypes
import ctypes.util
import os
import struct
import sys
import time

AF_VSOCK, SOCK_STREAM, CID_HOST, PORT = 40, 1, 2, 47100
FRAME = 1280 * 720 * 3 // 2

libc = ctypes.CDLL(ctypes.util.find_library("c"), use_errno=True)
fd = libc.socket(AF_VSOCK, SOCK_STREAM, 0)
if fd < 0:
    sys.exit(f"socket(AF_VSOCK): {os.strerror(ctypes.get_errno())} – no virtio socket device in this VM")
# struct sockaddr_vm (sys/vsock.h): len, family, reserved, port, cid
address = struct.pack("<BBHII", 12, AF_VSOCK, 0, PORT, CID_HOST)
if libc.connect(fd, address, len(address)) < 0:
    sys.exit(f"connect to the Mac's port {PORT}: {os.strerror(ctypes.get_errno())}")
total, start = 0, time.monotonic()
while chunk := os.read(fd, 1 << 20):
    total += len(chunk)
elapsed = time.monotonic() - start
print(f"{total / elapsed / 1e6:.1f} MB/s, {total / elapsed / FRAME:.1f} raw frames a second, {elapsed:.1f} s")
