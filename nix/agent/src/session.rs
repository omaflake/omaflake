// SPDX-License-Identifier: AGPL-3.0-or-later
//! Session mode (`omaflake-agent session`): the clipboard of the user's session. One process per session; it talks
//! to the system agent – the one owner of the port – over the Unix socket SOCKET, with the same frames (spec §11.9).
//! Linux (a systemd user service, replaces clipboard-agent.py): wl-paste watches the Wayland clipboard, wl-copy
//! writes it. macOS: the pasteboard's change count is polled, pbpaste/pbcopy read and write text (PNG: OPEN).
//! Each side remembers what it wrote last, so its echo is not sent back.

use crate::clip;
use crate::proto::*;
#[cfg(target_os = "linux")]
use std::io::{BufRead, BufReader};
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub const SOCKET: &str = crate::concat_runtime!("session.sock");
pub const RUNTIME: &str = if cfg!(target_os = "macos") { "/var/run/omaflake-agent" } else { "/run/omaflake-agent" };

fn log(msg: &str) {
    eprintln!("omaflake-agent session: {msg}");
}

#[derive(Default)]
struct Seen {
    written: Option<u64>,
    sent: Option<u64>,
}

/// (mime, data) of the Wayland clipboard: a PNG if offered, else text; None if empty, something else or too large
#[cfg(target_os = "linux")]
fn current_clipboard() -> Option<(&'static str, Vec<u8>)> {
    let types = Command::new("wl-paste").arg("--list-types").output().ok()?.stdout;
    let types = String::from_utf8_lossy(&types);
    let (mime, args): (&str, &[&str]) = if types.lines().any(|t| t == clip::PNG) {
        (clip::PNG, &["--type", "image/png"])
    } else if types.lines().any(|t| t.starts_with("text/plain") || ["UTF8_STRING", "STRING", "TEXT"].contains(&t)) {
        (clip::TEXT, &["--no-newline", "--type", "text"])
    } else {
        return None;
    };
    let data = Command::new("wl-paste").args(args).output().ok()?.stdout;
    (!data.is_empty() && data.len() <= MAX_CLIPBOARD).then_some((mime, data))
}

pub fn run() {
    let stream = loop {
        match UnixStream::connect(SOCKET) {
            Ok(s) => break s,
            Err(_) => std::thread::sleep(Duration::from_secs(2)), // the system agent is not up yet
        }
    };
    log("connected to the omaflake agent");
    let seen = Arc::new(Mutex::new(Seen::default()));

    // Mac → Wayland: clipboard messages the system agent passes on
    let mut reader = stream.try_clone().expect("socket");
    let seen_in = seen.clone();
    std::thread::spawn(move || {
        let mut decoder = Decoder::default();
        let mut parts = Reassembler::default();
        let mut chunk = vec![0u8; 65536];
        loop {
            let n = match reader.read(&mut chunk) {
                Ok(0) | Err(_) => {
                    log("the omaflake agent went away");
                    std::process::exit(0); // systemd starts this again
                }
                Ok(n) => n,
            };
            decoder.feed(&chunk[..n]);
            while let Some(frame) = decoder.next_frame() {
                if frame.kind != MSG_CLIPBOARD {
                    continue;
                }
                let Some(payload) = parts.push(frame, clip::MAX_MESSAGE) else { continue };
                let Some((mime, data)) = clip::decode(&payload) else { continue };
                seen_in.lock().unwrap().written = Some(clip::fingerprint(&mime, &data));
                write_clipboard(&mime, &data);
            }
        }
    });

    // guest → Mac: every change of the clipboard, except the echo of what came from the Mac
    let mut writer = stream;
    watch_clipboard(|| {
        let Some((mime, data)) = current_clipboard() else { return true };
        let fp = clip::fingerprint(mime, &data);
        {
            let mut s = seen.lock().unwrap();
            if s.written == Some(fp) || s.sent == Some(fp) {
                return true;
            }
            s.sent = Some(fp);
        }
        if writer.write_all(&encode_message(MSG_CLIPBOARD, &clip::encode(mime, &data))).is_err() {
            log("the omaflake agent went away");
            return false;
        }
        true
    });
}

#[cfg(target_os = "linux")]
fn write_clipboard(mime: &str, data: &[u8]) {
    let child = Command::new("wl-copy").args(["--type", mime]).stdin(Stdio::piped()).spawn();
    if let Ok(mut child) = child {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(data);
        }
        let _ = child.wait();
    }
}

/// Calls `changed` on every clipboard change until it returns false (Linux: wl-paste --watch)
#[cfg(target_os = "linux")]
fn watch_clipboard(mut changed: impl FnMut() -> bool) {
    let watch = Command::new("wl-paste").args(["--watch", "echo", "changed"]).stdout(Stdio::piped()).spawn();
    let Ok(mut watch) = watch else {
        log("wl-paste not found – no clipboard");
        return;
    };
    for line in BufReader::new(watch.stdout.take().unwrap()).lines() {
        if line.is_err() || !changed() {
            break;
        }
    }
}

// macOS: pbpaste/pbcopy in UTF-8, the change count through the Objective-C runtime (no crate needed)
#[cfg(target_os = "macos")]
fn pb_command(name: &str) -> Command {
    let mut c = Command::new(format!("/usr/bin/{name}"));
    c.env("LANG", "en_US.UTF-8").env("LC_ALL", "en_US.UTF-8");
    c
}

#[cfg(target_os = "macos")]
fn current_clipboard() -> Option<(&'static str, Vec<u8>)> {
    let data = pb_command("pbpaste").output().ok()?.stdout;
    (!data.is_empty() && data.len() <= MAX_CLIPBOARD).then_some((clip::TEXT, data))
}

#[cfg(target_os = "macos")]
fn write_clipboard(mime: &str, data: &[u8]) {
    if mime != clip::TEXT {
        return; // PNG into a macOS guest's pasteboard: OPEN
    }
    if let Ok(mut child) = pb_command("pbcopy").stdin(Stdio::piped()).spawn() {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(data);
        }
        let _ = child.wait();
    }
}

#[cfg(target_os = "macos")]
mod pasteboard {
    use std::ffi::{c_char, c_long, c_void};

    #[link(name = "objc")]
    unsafe extern "C" {
        fn objc_getClass(name: *const c_char) -> *mut c_void;
        fn sel_registerName(name: *const c_char) -> *mut c_void;
        fn objc_msgSend();
    }
    #[link(name = "AppKit", kind = "framework")]
    unsafe extern "C" {}

    /// NSPasteboard.generalPasteboard.changeCount – cheap enough to poll
    pub fn change_count() -> i64 {
        unsafe {
            let send_id: extern "C" fn(*mut c_void, *mut c_void) -> *mut c_void =
                std::mem::transmute(objc_msgSend as *const ());
            let send_long: extern "C" fn(*mut c_void, *mut c_void) -> c_long =
                std::mem::transmute(objc_msgSend as *const ());
            let pb = send_id(objc_getClass(c"NSPasteboard".as_ptr()), sel_registerName(c"generalPasteboard".as_ptr()));
            if pb.is_null() {
                return -1;
            }
            send_long(pb, sel_registerName(c"changeCount".as_ptr())) as i64
        }
    }
}

#[cfg(target_os = "macos")]
fn watch_clipboard(mut changed: impl FnMut() -> bool) {
    let mut last = pasteboard::change_count();
    loop {
        std::thread::sleep(Duration::from_millis(400));
        let now = pasteboard::change_count();
        if now != last {
            last = now;
            if !changed() {
                return;
            }
        }
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    /// The Objective-C call works (reads only the change count, never the pasteboard's content); without a
    /// pasteboard – the Nix build sandbox – it says -1 instead of crashing
    #[test]
    fn change_count_is_read() {
        assert!(super::pasteboard::change_count() >= -1);
    }
}
