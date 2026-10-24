// SPDX-License-Identifier: AGPL-3.0-or-later
//! The host's clipboard for the guest (spec §11.9, type `0x0100`), as `ClipboardBridge` in Bridge.swift does it with
//! the Mac's pasteboard: text (`text/plain;charset=utf-8`) and PNG (`image/png`) both ways, at most 32 MiB; each side
//! remembers what it last wrote and sent (fingerprints), so the change that writing causes is not sent back; nothing
//! is sent until something is copied after the bridge started.
//!
//! The clipboard itself is a `ClipboardBackend`, chosen once by `detect`:
//! - **Wayland** (`$WAYLAND_DISPLAY`): `wl-paste --watch` reports each change, `wl-paste`/`wl-copy` read and write
//!   (wl-clipboard, what the guest agent's session uses too). The watch needs the compositor's data-control protocol
//!   (wlroots' `zwlr_data_control` or `ext_data_control`: Hyprland, Sway, KDE Plasma, …; GNOME's Mutter has neither –
//!   OPEN whether a GNOME host gets a working watch).
//! - **X11** (`$DISPLAY`, and Wayland sessions without wl-clipboard through Xwayland): `xclip`; X11 has no change
//!   event without XFixes, so the selection's TIMESTAMP is polled every 0.5 s and the content read when it changed.
//!
//! Both run as the user, without root. `OMAFLAKE_CLIPBOARD=wayland|x11|none` forces a choice.

use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};
use std::io::{BufRead, BufReader, Read, Write};
use std::process::{Child, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use base64::engine::general_purpose::STANDARD as BASE64;
use base64::Engine;

use crate::agent_protocol::{AgentMessage, AgentProtocol, AgentSender};
use crate::Log;

pub const TEXT: &str = "text/plain;charset=utf-8";
pub const PNG: &str = "image/png";

/// Reads, writes and watches one clipboard
pub trait ClipboardBackend: Send + Sync {
    /// "Wayland (wl-clipboard)", "X11 (xclip)" – for the log
    fn describe(&self) -> String;
    /// Blocks until the clipboard may have changed; false when watching ended (the bridge then stops sending)
    fn wait_change(&self) -> bool;
    /// The current content: PNG if offered, else text; None when empty, something else or larger than 32 MiB
    fn read(&self) -> Option<(String, Vec<u8>)>;
    /// Puts content on the clipboard; false when it failed
    fn write(&self, mime: &str, data: &[u8]) -> bool;
}

fn fingerprint(mime: &str, data: &[u8]) -> u64 {
    let mut h = DefaultHasher::new();
    mime.hash(&mut h);
    data.hash(&mut h);
    h.finish()
}

#[derive(Default)]
struct Seen {
    /// What came from the guest and was written here
    written: Option<u64>,
    /// What was last sent to the guest (or was there when the bridge started)
    sent: Option<u64>,
    agent: Option<AgentSender>,
}

pub struct ClipboardBridge {
    backend: Arc<dyn ClipboardBackend>,
    seen: Mutex<Seen>,
    log: Arc<Log<'static>>,
}

impl ClipboardBridge {
    pub fn new(backend: Arc<dyn ClipboardBackend>, log: Arc<Log<'static>>) -> Arc<ClipboardBridge> {
        Arc::new(ClipboardBridge { backend, seen: Mutex::new(Seen::default()), log })
    }

    /// Watches the host's clipboard on a thread of its own; what is there now only counts as already sent
    pub fn start(self: &Arc<Self>) {
        let me = self.clone();
        std::thread::spawn(move || {
            if let Some((mime, data)) = me.backend.read() {
                me.seen.lock().unwrap_or_else(|e| e.into_inner()).sent = Some(fingerprint(&mime, &data));
            }
            while me.backend.wait_change() {
                me.changed();
            }
            (me.log)("clipboard: the host's clipboard can no longer be watched");
        });
    }

    /// The guest's agent carries the clipboard (its welcome enabled it)
    pub fn use_agent(&self, sender: AgentSender) {
        self.seen.lock().unwrap_or_else(|e| e.into_inner()).agent = Some(sender);
    }

    /// Host → guest: the clipboard changed – send it unless it is what came from the guest or was sent already
    pub fn changed(&self) {
        let Some((mime, data)) = self.backend.read() else { return };
        if data.len() > AgentProtocol::MAX_CLIPBOARD {
            return;
        }
        let fp = fingerprint(&mime, &data);
        let sender = {
            let mut s = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            if s.written == Some(fp) || s.sent == Some(fp) {
                return;
            }
            let Some(sender) = s.agent.clone() else { return };
            s.sent = Some(fp);
            sender
        };
        let json = serde_json::json!({ "mime": mime, "data": BASE64.encode(&data) });
        if sender.send(AgentMessage::Clipboard, json.to_string().as_bytes()) && debug() {
            (self.log)(&format!("clipboard: sent {mime} {} bytes", data.len()));
        }
    }

    /// Guest → host: put the content on the clipboard (checked by `AgentLink`: text or PNG within the limit)
    pub fn apply(&self, mime: &str, data: &[u8]) {
        let fp = fingerprint(mime, data);
        {
            let mut s = self.seen.lock().unwrap_or_else(|e| e.into_inner());
            s.written = Some(fp);
            s.sent = Some(fp); // the guest has it already
        }
        if !self.backend.write(mime, data) {
            (self.log)(&format!("clipboard: could not write {mime} ({})", self.backend.describe()));
        } else if debug() {
            (self.log)(&format!("clipboard: received {mime} {} bytes", data.len()));
        }
    }
}

fn debug() -> bool {
    std::env::var_os("OMAFLAKE_DEBUG").is_some()
}

// MARK: Choosing a backend

/// The clipboard of this session, or why there is none (for the log)
pub fn detect() -> Result<Arc<dyn ClipboardBackend>, String> {
    let forced = std::env::var("OMAFLAKE_CLIPBOARD").ok();
    let wayland = std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty());
    let x11 = std::env::var_os("DISPLAY").is_some_and(|v| !v.is_empty());
    let has = |tool: &str| crate::shell::Shell::which(tool).is_ok();
    match forced.as_deref() {
        Some("none") => return Err("off (OMAFLAKE_CLIPBOARD=none)".into()),
        Some("wayland") => return Wayland::start().map(|b| Arc::new(b) as _),
        Some("x11") => return Ok(Arc::new(X11::new())),
        _ => {}
    }
    if wayland && has("wl-paste") && has("wl-copy") {
        return Wayland::start().map(|b| Arc::new(b) as _);
    }
    if x11 && has("xclip") {
        return Ok(Arc::new(X11::new()));
    }
    Err(match (wayland, x11) {
        (false, false) => "no desktop session (neither WAYLAND_DISPLAY nor DISPLAY is set)".into(),
        (true, _) => "wl-paste/wl-copy not found (install wl-clipboard)".into(),
        (false, true) => "xclip not found (install xclip)".into(),
    })
}

// MARK: Running the tools

/// Runs a clipboard tool with a time limit (an owner that never answers must not hang the bridge); its stdout when
/// it exited with 0
fn output(cmd: &mut Command, limit: Duration) -> Option<Vec<u8>> {
    let mut child = cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let reader = std::thread::spawn(move || {
        let mut out = Vec::new();
        let _ = stdout.by_ref().take(AgentProtocol::MAX_CLIPBOARD as u64 + 1).read_to_end(&mut out);
        out
    });
    let deadline = Instant::now() + limit;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = reader.join().ok()?;
    status.success().then_some(out)
}

/// Runs a clipboard tool that takes `input` on stdin and forks to serve it (wl-copy, xclip -i): its output goes
/// nowhere – the forked server keeps whatever it inherited open – and only the first process is waited for
fn feed(cmd: &mut Command, input: &[u8], limit: Duration) -> bool {
    let Ok(mut child) = cmd.stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn() else {
        return false;
    };
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input);
    }
    let deadline = Instant::now() + limit;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status.success(),
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(10)),
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return false;
            }
        }
    }
}

/// The text-like types a clipboard offers for plain text
fn is_text_type(t: &str) -> bool {
    t.starts_with("text/plain") || ["UTF8_STRING", "STRING", "TEXT"].contains(&t)
}

/// Content up to the limit and not empty
fn content(mime: &str, data: Vec<u8>) -> Option<(String, Vec<u8>)> {
    (!data.is_empty() && data.len() <= AgentProtocol::MAX_CLIPBOARD).then(|| (mime.to_string(), data))
}

/// A child that dies with the bridge (Linux: PR_SET_PDEATHSIG; the bridge exits without unwinding when the VM ends)
fn tied_to_parent(cmd: &mut Command) -> &mut Command {
    #[cfg(target_os = "linux")]
    {
        use std::os::unix::process::CommandExt;
        // SAFETY: prctl is async-signal-safe and touches no memory of the parent
        unsafe {
            cmd.pre_exec(|| {
                libc::prctl(libc::PR_SET_PDEATHSIG, libc::SIGTERM);
                Ok(())
            });
        }
    }
    cmd
}

// MARK: Wayland

/// wl-clipboard: `wl-paste --watch echo changed` prints a line per change
pub struct Wayland {
    watch: Mutex<(Child, BufReader<ChildStdout>)>,
}

impl Wayland {
    pub fn start() -> Result<Wayland, String> {
        let mut child = tied_to_parent(Command::new("wl-paste").args(["--watch", "echo", "changed"]))
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| format!("wl-paste: {e}"))?;
        let stdout = child.stdout.take().ok_or("wl-paste: no output")?;
        Ok(Wayland { watch: Mutex::new((child, BufReader::new(stdout))) })
    }
}

impl ClipboardBackend for Wayland {
    fn describe(&self) -> String {
        "Wayland (wl-clipboard)".into()
    }

    fn wait_change(&self) -> bool {
        let mut w = self.watch.lock().unwrap_or_else(|e| e.into_inner());
        let mut line = String::new();
        // EOF: wl-paste ended (no data-control protocol, the compositor went away)
        matches!(w.1.read_line(&mut line), Ok(n) if n > 0)
    }

    fn read(&self) -> Option<(String, Vec<u8>)> {
        let limit = Duration::from_secs(5);
        let types = output(Command::new("wl-paste").arg("--list-types"), limit)?;
        let types = String::from_utf8_lossy(&types);
        if types.lines().any(|t| t == PNG) {
            return content(PNG, output(Command::new("wl-paste").args(["--type", PNG]), limit)?);
        }
        if types.lines().any(is_text_type) {
            let data = output(Command::new("wl-paste").args(["--no-newline", "--type", "text"]), limit)?;
            return content(TEXT, data);
        }
        None
    }

    fn write(&self, mime: &str, data: &[u8]) -> bool {
        // wl-copy forks to serve the content and returns once it has read it
        feed(Command::new("wl-copy").args(["--type", mime]), data, Duration::from_secs(5))
    }
}

// MARK: X11

/// xclip on the CLIPBOARD selection; changes by polling the selection's TIMESTAMP
pub struct X11 {
    last_stamp: Mutex<Option<Vec<u8>>>,
}

impl X11 {
    pub const POLL: Duration = Duration::from_millis(500);

    pub fn new() -> X11 {
        X11 { last_stamp: Mutex::new(None) }
    }

    fn xclip_out(target: &str) -> Option<Vec<u8>> {
        output(Command::new("xclip").args(["-selection", "clipboard", "-o", "-t", target]), Duration::from_secs(5))
    }
}

impl Default for X11 {
    fn default() -> Self {
        Self::new()
    }
}

impl ClipboardBackend for X11 {
    fn describe(&self) -> String {
        "X11 (xclip)".into()
    }

    fn wait_change(&self) -> bool {
        loop {
            std::thread::sleep(Self::POLL);
            // The owner's time of taking the selection (ICCCM); an owner without TIMESTAMP: read and compare instead
            let Some(stamp) = Self::xclip_out("TIMESTAMP").filter(|s| !s.is_empty()) else { return true };
            let mut last = self.last_stamp.lock().unwrap_or_else(|e| e.into_inner());
            if last.as_ref() != Some(&stamp) {
                *last = Some(stamp);
                return true;
            }
        }
    }

    fn read(&self) -> Option<(String, Vec<u8>)> {
        let targets = Self::xclip_out("TARGETS")?;
        let targets = String::from_utf8_lossy(&targets);
        if targets.lines().any(|t| t == PNG) {
            return content(PNG, Self::xclip_out(PNG)?);
        }
        if targets.lines().any(is_text_type) {
            return content(TEXT, Self::xclip_out("UTF8_STRING")?);
        }
        None
    }

    fn write(&self, mime: &str, data: &[u8]) -> bool {
        let target = if mime == PNG { PNG } else { "UTF8_STRING" };
        // xclip forks to own the selection and returns once it has read the input
        feed(Command::new("xclip").args(["-selection", "clipboard", "-i", "-t", target]), data, Duration::from_secs(5))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::sync::mpsc::{self, Receiver, Sender};

    /// A clipboard in memory: `copy` changes it as a user would, `written` records what the bridge wrote
    pub(crate) struct FakeClipboard {
        pub content: Mutex<Option<(String, Vec<u8>)>>,
        pub written: Mutex<Vec<(String, Vec<u8>)>>,
        changes: Mutex<Receiver<()>>,
        notify: Mutex<Sender<()>>,
    }

    impl FakeClipboard {
        pub(crate) fn new() -> Arc<FakeClipboard> {
            let (tx, rx) = mpsc::channel();
            Arc::new(FakeClipboard {
                content: Mutex::new(None),
                written: Mutex::new(vec![]),
                changes: Mutex::new(rx),
                notify: Mutex::new(tx),
            })
        }

        pub(crate) fn copy(&self, mime: &str, data: &[u8]) {
            *self.content.lock().unwrap() = Some((mime.into(), data.into()));
            self.notify.lock().unwrap().send(()).unwrap();
        }
    }

    impl ClipboardBackend for FakeClipboard {
        fn describe(&self) -> String {
            "fake".into()
        }
        fn wait_change(&self) -> bool {
            self.changes.lock().unwrap().recv().is_ok()
        }
        fn read(&self) -> Option<(String, Vec<u8>)> {
            self.content.lock().unwrap().clone()
        }
        fn write(&self, mime: &str, data: &[u8]) -> bool {
            self.written.lock().unwrap().push((mime.into(), data.into()));
            *self.content.lock().unwrap() = Some((mime.into(), data.into()));
            // a real clipboard reports the change it was just given
            self.notify.lock().unwrap().send(()).unwrap();
            true
        }
    }

    #[test]
    fn text_types() {
        assert!(is_text_type("text/plain;charset=utf-8") && is_text_type("UTF8_STRING") && is_text_type("text/plain"));
        assert!(!is_text_type("text/html") && !is_text_type("image/png"));
        assert_eq!(content(TEXT, vec![]), None);
        assert_eq!(content(TEXT, b"x".to_vec()), Some((TEXT.to_string(), b"x".to_vec())));
    }

    #[test]
    fn tools_run_with_a_time_limit() {
        assert_eq!(output(Command::new("printf").arg("hi"), Duration::from_secs(5)), Some(b"hi".to_vec()));
        assert_eq!(output(Command::new("sh").args(["-c", "exit 1"]), Duration::from_secs(5)), None);
        let t = Instant::now();
        assert_eq!(output(Command::new("sleep").arg("10"), Duration::from_millis(300)), None);
        assert!(t.elapsed() < Duration::from_secs(5));
        assert_eq!(output(&mut Command::new("/nonexistent/tool"), Duration::from_secs(1)), None);
        // a tool that forks a server keeping stdout open (as wl-copy and xclip do) does not hold up `feed`
        assert!(feed(Command::new("sh").args(["-c", "cat >/dev/null; sleep 5 &"]), b"x", Duration::from_secs(3)));
        assert!(!feed(Command::new("sh").args(["-c", "exit 2"]), b"x", Duration::from_secs(3)));
    }

    #[test]
    fn forced_off() {
        // no other test reads OMAFLAKE_CLIPBOARD
        std::env::set_var("OMAFLAKE_CLIPBOARD", "none");
        assert!(detect().is_err());
        std::env::remove_var("OMAFLAKE_CLIPBOARD");
    }
}
