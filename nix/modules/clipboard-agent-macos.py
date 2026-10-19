# SPDX-License-Identifier: AGPL-3.0-or-later
"""omaflake clipboard agent (macOS guest).

The counterpart of clipboard-agent.py for macOS guests: bridges the guest's pasteboard and the Mac's over the virtio
console port org.omaflake.clipboard (/dev/cu.org.omaflake.clipboard, omaf-runner --clipboard). Same protocol: one
JSON object per line, {"mime": ..., "data": <base64>}, text and PNG, fingerprints against echo.
Runs as a LaunchAgent in the user's session (the pasteboard belongs to the GUI session).
"""

import base64
import ctypes
import hashlib
import json
import os
import subprocess
import sys
import tempfile
import termios
import threading
import time
import tty

PORT = "/dev/cu.org.omaflake.clipboard"
TEXT = "text/plain;charset=utf-8"
PNG = "image/png"
MAX_BYTES = 32 * 1024 * 1024
ENV = dict(os.environ, LANG="en_US.UTF-8", LC_ALL="en_US.UTF-8")  # pbcopy/pbpaste use the locale's encoding

lock = threading.Lock()
last_written = None  # fingerprint of what came from the Mac
last_sent = None
written_count = -1   # pasteboard change count right after writing it


def log(msg):
    print(f"omaflake-clipboard: {msg}", file=sys.stderr, flush=True)


def fingerprint(mime, data):
    return hashlib.sha256(mime.encode() + b"\0" + data).hexdigest()


# NSPasteboard.generalPasteboard.changeCount through the Objective-C runtime – cheap enough to poll
objc = ctypes.cdll.LoadLibrary("/usr/lib/libobjc.A.dylib")
ctypes.cdll.LoadLibrary("/System/Library/Frameworks/AppKit.framework/AppKit")
objc.objc_getClass.restype = ctypes.c_void_p
objc.objc_getClass.argtypes = [ctypes.c_char_p]
objc.sel_registerName.restype = ctypes.c_void_p
objc.sel_registerName.argtypes = [ctypes.c_char_p]
send_id = ctypes.CFUNCTYPE(ctypes.c_void_p, ctypes.c_void_p, ctypes.c_void_p)(("objc_msgSend", objc))
send_long = ctypes.CFUNCTYPE(ctypes.c_long, ctypes.c_void_p, ctypes.c_void_p)(("objc_msgSend", objc))


def change_count():
    pb = send_id(objc.objc_getClass(b"NSPasteboard"), objc.sel_registerName(b"generalPasteboard"))
    return send_long(pb, objc.sel_registerName(b"changeCount"))


def osascript(script):
    return subprocess.run(["osascript", "-e", script], capture_output=True, env=ENV)


def read_pasteboard():
    """(mime, data): a PNG if there is one, else text; None if empty or something else."""
    with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as f:
        path = f.name
    try:
        r = osascript(f'set f to open for access (POSIX file "{path}") with write permission\n'
                      f'try\nwrite (the clipboard as «class PNGf») to f\nend try\nclose access f')
        if r.returncode == 0 and os.path.getsize(path) > 0:
            with open(path, "rb") as f:
                return PNG, f.read()
    finally:
        os.unlink(path)
    text = subprocess.run(["pbpaste", "-Prefer", "txt"], capture_output=True, env=ENV).stdout
    return (TEXT, text) if text else None


def write_pasteboard(mime, data):
    if mime == TEXT:
        subprocess.run(["pbcopy"], input=data, env=ENV)
    elif mime == PNG:
        with tempfile.NamedTemporaryFile(suffix=".png", delete=False) as f:
            f.write(data)
            path = f.name
        try:
            osascript(f'set the clipboard to (read (POSIX file "{path}") as «class PNGf»)')
        finally:
            os.unlink(path)


def from_mac(fd):
    global last_written, written_count
    buf = b""
    while True:
        chunk = os.read(fd, 65536)
        if not chunk:
            time.sleep(0.5)   # nothing connected on the Mac side
            continue
        buf += chunk
        while b"\n" in buf:
            line, buf = buf.split(b"\n", 1)
            try:
                msg = json.loads(line)
                mime, data = msg["mime"], base64.b64decode(msg["data"])
            except (ValueError, KeyError) as e:
                log(f"ignored a malformed message: {e}")
                continue
            if mime not in (TEXT, PNG) or len(data) > MAX_BYTES:
                continue
            with lock:
                last_written = fingerprint(mime, data)
                write_pasteboard(mime, data)
                written_count = change_count()
        if len(buf) > 2 * MAX_BYTES:
            buf = b""


def to_mac(fd):
    global last_sent
    seen = change_count()   # only what is copied from now on
    while True:
        time.sleep(0.4)
        count = change_count()
        if count == seen:
            continue
        seen = count
        with lock:
            if count == written_count:
                continue
        current = read_pasteboard()
        if current is None or len(current[1]) > MAX_BYTES:
            continue
        mime, data = current
        fp = fingerprint(mime, data)
        with lock:
            if fp in (last_written, last_sent):
                continue
            last_sent = fp
        line = (json.dumps({"mime": mime, "data": base64.b64encode(data).decode()}) + "\n").encode()
        while line:
            line = line[os.write(fd, line):]


def main():
    if not os.path.exists(PORT):
        log(f"{PORT} not found – the clipboard is off for this VM")
        return 0
    fd = os.open(PORT, os.O_RDWR | os.O_NOCTTY)
    tty.setraw(fd)   # a serial device: no line editing, no echo
    attrs = termios.tcgetattr(fd)
    attrs[3] &= ~termios.ECHO
    termios.tcsetattr(fd, termios.TCSANOW, attrs)
    threading.Thread(target=from_mac, args=(fd,), daemon=True).start()
    to_mac(fd)


if __name__ == "__main__":
    sys.exit(main())
