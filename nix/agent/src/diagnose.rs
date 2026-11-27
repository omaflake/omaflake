// SPDX-License-Identifier: AGPL-3.0-or-later
//! The guest's half of the VM diagnosis (spec §11.9, types 0x0600/0x0601, plan B162): one check per request, one
//! answer `{id, check, status, detail, data?}` within 15 s, requests one at a time, unknown checks `skipped`.
//!
//! Where a check runs:
//! - `camera`, `midi` and unknown checks: at once in the agent's main loop (they only read its state);
//! - Linux: `clipboard-*`, `sound` and `microphone` in the user's session process (`omaflake-agent session`): the
//!   system agent runs as a sandboxed root service without the user's Wayland display, PipeWire socket or home
//!   (vm-guest.nix: ProtectHome, no capabilities). `share-*` there too when a session is connected, else in the agent;
//! - everything else (and every check on macOS, where the agent runs as the logged-in user) on a worker thread, so the
//!   main loop keeps reading the port (status, clipboard, MIDI go on).

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// The answer is due within this time (spec §11.9)
pub const DEADLINE: Duration = Duration::from_secs(15);
/// How long the agent waits for the session process before it answers `fail` itself – below DEADLINE, so the Mac
/// still gets an answer when the session hangs
pub const SESSION_WAIT: Duration = Duration::from_secs(DEADLINE.as_secs() - 2);
/// Largest text a check reads or writes (share files, clipboard text in an answer)
pub const MAX_TEXT: usize = 64 * 1024;
/// Requests waiting while one runs (the Mac sends one at a time; more are refused)
pub const MAX_QUEUE: usize = 8;

/// Where every guest sees its volumes, one folder each (plan B187)
pub const SHARE: &str = "/Volumes/My Shared Files";

/// The folder of the volume the request names (`"volume"`), else all of them
fn volume_dir(args: &Value) -> Result<std::path::PathBuf, String> {
    match args.get("volume").and_then(Value::as_str) {
        Some(v) if valid_name(v) => Ok(Path::new(SHARE).join(v)),
        Some(v) => Err(format!("\"{v}\" is not a plain file name")),
        None => Ok(Path::new(SHARE).into()),
    }
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Request {
    pub id: u32,
    #[serde(default)]
    pub check: String,
    #[serde(default, skip_serializing_if = "Value::is_null")]
    pub args: Value,
}

#[derive(Debug, Serialize, Deserialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Status {
    Pass,
    Fail,
    Confirm,
    Skipped,
}

#[derive(Debug, Serialize, Deserialize, Clone, PartialEq)]
pub struct Answer {
    pub id: u32,
    pub check: String,
    pub status: Status,
    pub detail: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub data: Option<Value>,
}

/// What a check reports, before it gets the request's id and name
pub struct Outcome {
    pub status: Status,
    pub detail: String,
    pub data: Option<Value>,
}

impl Outcome {
    pub fn new(status: Status, detail: impl Into<String>) -> Outcome {
        Outcome { status, detail: short(&detail.into()), data: None }
    }
    pub fn with(mut self, data: Value) -> Outcome {
        self.data = Some(data);
        self
    }
    pub fn answer(self, req: &Request) -> Answer {
        Answer { id: req.id, check: req.check.clone(), status: self.status, detail: self.detail, data: self.data }
    }
}

fn fail(detail: impl Into<String>) -> Outcome {
    Outcome::new(Status::Fail, detail)
}

/// A detail fits the Mac's limit (256 characters, spec §11.9) with room to spare
fn short(s: &str) -> String {
    let s = s.trim();
    match s.char_indices().nth(240) {
        Some((i, _)) => format!("{}…", &s[..i]),
        None => s.to_string(),
    }
}

/// A request from the Mac; `Err(Some(id))` when it is malformed but its id is readable (answered `fail`)
pub fn parse_request(payload: &[u8]) -> Result<Request, Option<u32>> {
    let value: Value = serde_json::from_slice(payload).map_err(|_| None)?;
    let id = value.get("id").and_then(Value::as_u64).and_then(|i| u32::try_from(i).ok());
    serde_json::from_value::<Request>(value).map_err(|_| id)
}

/// The answer to a request that could not be read
pub fn malformed(id: u32) -> Answer {
    Answer { id, check: String::new(), status: Status::Fail, detail: "Malformed diagnosis request".into(), data: None }
}

pub fn encode(answer: &Answer) -> Vec<u8> {
    serde_json::to_vec(answer).expect("serializable")
}

/// Where a check runs (see the module's comment)
#[derive(Debug, PartialEq, Eq)]
pub enum Place {
    /// In the main loop, from the agent's state (camera, midi, unknown checks)
    Agent,
    /// On a worker thread
    Worker,
    /// In the user's session process; without one: `fail` (`fallback` false) or on a worker thread
    Session { fallback: bool },
}

pub fn place(check: &str) -> Place {
    match check {
        "camera" | "midi" => Place::Agent,
        "info" | "network" => Place::Worker,
        "clipboard-read" | "clipboard-write" | "sound" | "microphone" if cfg!(target_os = "linux") => {
            Place::Session { fallback: false }
        }
        "share-read" | "share-write" if cfg!(target_os = "linux") => Place::Session { fallback: true },
        "clipboard-read" | "clipboard-write" | "sound" | "microphone" | "share-read" | "share-write" => Place::Worker,
        _ => Place::Agent,
    }
}

/// The checks known at all (others answer `skipped`)
pub fn known(check: &str) -> bool {
    matches!(
        check,
        "info"
            | "clipboard-read"
            | "clipboard-write"
            | "share-write"
            | "share-read"
            | "network"
            | "sound"
            | "microphone"
            | "camera"
            | "midi"
    )
}

pub fn unknown(req: &Request) -> Answer {
    Outcome::new(Status::Skipped, format!("This agent does not know the check \"{}\"", req.check)).answer(req)
}

/// A delegated check whose session is missing or did not answer
pub fn no_session(req: &Request) -> Answer {
    fail("No desktop session is connected to the agent – log in to the desktop and run the diagnosis again").answer(req)
}

pub fn session_silent(req: &Request) -> Answer {
    fail("The desktop session did not answer in time").answer(req)
}

/// The clipboard as a check sees it: the user's (session process, macOS pasteboard)
pub trait Clipboard {
    /// (mime, data), None when empty or not text/PNG
    fn read(&self) -> Option<(&'static str, Vec<u8>)>;
    fn write(&self, text: &str) -> Result<(), String>;
}

/// What the agent knew when the check was handed out
#[derive(Default, Clone, Copy)]
pub struct Context {
    /// Session processes connected to the agent (Linux: the users' desktop sessions)
    #[cfg_attr(target_os = "macos", allow(dead_code))]
    pub sessions: usize,
}

/// A check that runs outside the main loop (worker thread, session process)
pub fn run(req: &Request, ctx: Context, clipboard: Option<&dyn Clipboard>) -> Answer {
    let outcome = match req.check.as_str() {
        "info" => info(ctx),
        "network" => network(),
        "clipboard-read" => match clipboard {
            Some(c) => clipboard_read(c),
            None => fail("No clipboard here"),
        },
        "clipboard-write" => match (clipboard, text_arg(&req.args)) {
            (_, Err(e)) => fail(e),
            (Some(c), Ok(text)) => match c.write(text) {
                Ok(()) => {
                    Outcome::new(Status::Pass, format!("Put {} characters on the clipboard", text.chars().count()))
                }
                Err(e) => fail(format!("Could not write the clipboard: {e}")),
            },
            (None, Ok(_)) => fail("No clipboard here"),
        },
        "share-write" => match volume_dir(&req.args) {
            Ok(dir) => share_write(&dir, &req.args),
            Err(e) => fail(e),
        },
        "share-read" => match volume_dir(&req.args) {
            Ok(dir) => share_read(&dir, &req.args),
            Err(e) => fail(e),
        },
        "sound" => sound(),
        "microphone" => microphone(),
        _ => return unknown(req),
    };
    outcome.answer(req)
}

fn text_arg(args: &Value) -> Result<&str, String> {
    let text = args.get("text").and_then(Value::as_str).ok_or("The argument \"text\" is missing")?;
    if text.len() > MAX_TEXT {
        return Err("The text is longer than 64 KiB".into());
    }
    Ok(text)
}

/// A plain file name: no path separator, not `.` or `..`, no NUL, at most 255 bytes
pub fn valid_name(name: &str) -> bool {
    !name.is_empty() && name.len() <= 255 && name != "." && name != ".." && !name.contains(['/', '\0'])
}

fn name_arg(args: &Value) -> Result<&str, String> {
    let name = args.get("name").and_then(Value::as_str).ok_or("The argument \"name\" is missing")?;
    if !valid_name(name) {
        return Err(format!("\"{name}\" is not a plain file name"));
    }
    Ok(name)
}

fn share_folder(dir: &Path) -> Result<(), Outcome> {
    match std::fs::metadata(dir) {
        Ok(m) if m.is_dir() => Ok(()),
        Ok(_) => Err(fail(format!("{} is not a folder", dir.display()))),
        Err(e) => Err(fail(format!("The share folder {} is missing: {e}", dir.display()))),
    }
}

pub fn share_write(dir: &Path, args: &Value) -> Outcome {
    let (name, text) = match (name_arg(args), text_arg(args)) {
        (Ok(n), Ok(t)) => (n, t),
        (Err(e), _) | (_, Err(e)) => return fail(e),
    };
    if let Err(o) = share_folder(dir) {
        return o;
    }
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let path = dir.join(name);
    let written = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .and_then(|mut f| f.write_all(text.as_bytes()));
    match written {
        Ok(()) => Outcome::new(Status::Pass, format!("Wrote {name} in {}", dir.display())),
        Err(e) => fail(format!("Could not write {}: {e}", path.display())),
    }
}

pub fn share_read(dir: &Path, args: &Value) -> Outcome {
    let name = match name_arg(args) {
        Ok(n) => n,
        Err(e) => return fail(e),
    };
    if let Err(o) = share_folder(dir) {
        return o;
    }
    use std::os::unix::fs::OpenOptionsExt;
    let path = dir.join(name);
    let mut bytes = vec![];
    let read = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&path)
        .and_then(|f| f.take(MAX_TEXT as u64 + 1).read_to_end(&mut bytes));
    if let Err(e) = read {
        return fail(format!("Could not read {}: {e}", path.display()));
    }
    if bytes.len() > MAX_TEXT {
        return fail(format!("{name} is larger than 64 KiB"));
    }
    match String::from_utf8(bytes) {
        Ok(text) => {
            Outcome::new(Status::Pass, format!("Read {name} from {}", dir.display())).with(json!({ "text": text }))
        }
        Err(_) => fail(format!("{name} is not UTF-8 text")),
    }
}

fn clipboard_read(c: &dyn Clipboard) -> Outcome {
    match c.read() {
        None => Outcome::new(Status::Pass, "The clipboard is empty").with(json!({ "text": "" })),
        Some((mime, _)) if mime != crate::clip::TEXT => fail("The clipboard holds an image, not text"),
        Some((_, data)) => {
            let text = String::from_utf8_lossy(&data);
            let (text, cut) = cut(&text, MAX_TEXT);
            let detail = if cut { "Read the clipboard (its first 64 KiB)" } else { "Read the clipboard" };
            Outcome::new(Status::Pass, detail).with(json!({ "text": text }))
        }
    }
}

/// At most `max` bytes of `s`, cut at a character boundary; true when something was cut
fn cut(s: &str, max: usize) -> (&str, bool) {
    if s.len() <= max {
        return (s, false);
    }
    let mut end = max;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    (&s[..end], true)
}

// ---------------------------------------------------------------------------------------------------------------
// info

fn info(ctx: Context) -> Outcome {
    let (user, session, clipboard) = desktop(ctx);
    let os = if cfg!(target_os = "macos") { "macos" } else { "linux" };
    let detail = match &user {
        Some(u) if session => format!("{u} is logged in to the desktop"),
        _ => "Nobody is logged in to the desktop".to_string(),
    };
    let detail = if clipboard { detail } else { format!("{detail}; the clipboard helper does not run") };
    Outcome::new(Status::Pass, detail)
        .with(json!({ "os": os, "user": user, "session": session, "helpers": { "clipboard": clipboard } }))
}

/// (the console user, a user is logged in to the desktop, the clipboard helper runs)
#[cfg(target_os = "macos")]
fn desktop(_: Context) -> (Option<String>, bool, bool) {
    let user = console_user();
    let session = user.as_deref().is_some_and(|u| u != "root" && u != "loginwindow");
    let clipboard = Command::new("/usr/bin/pgrep")
        .args(["-f", "clipboard-agent-macos.py"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success());
    (user, session, clipboard)
}

/// The owner of /dev/console: the user at the Mac's screen (root at the login window)
#[cfg(target_os = "macos")]
fn console_user() -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let uid = std::fs::metadata("/dev/console").ok()?.uid();
    user_name(uid)
}

#[cfg(target_os = "macos")]
fn user_name(uid: u32) -> Option<String> {
    let mut pwd: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut result: *mut libc::passwd = std::ptr::null_mut();
    // SAFETY: pwd, buf and result outlive the call; buf's length is passed
    let rc = unsafe { libc::getpwuid_r(uid, &mut pwd, buf.as_mut_ptr(), buf.len(), &mut result) };
    if rc != 0 || result.is_null() {
        return None;
    }
    // SAFETY: getpwuid_r succeeded, pw_name points into buf, NUL-terminated
    Some(unsafe { std::ffi::CStr::from_ptr(pwd.pw_name) }.to_string_lossy().into_owned())
}

/// Linux: a graphical logind session (TYPE wayland/x11/mir, not closing), or a session process connected to the agent
#[cfg(not(target_os = "macos"))]
fn desktop(ctx: Context) -> (Option<String>, bool, bool) {
    let user = std::fs::read_dir("/run/systemd/sessions")
        .into_iter()
        .flatten()
        .filter_map(|e| std::fs::read_to_string(e.ok()?.path()).ok())
        .find_map(|text| graphical_session_user(&text));
    let session = user.is_some() || ctx.sessions > 0;
    (user, session, true)
}

/// The user of a logind session file (/run/systemd/sessions/<id>) when it is a graphical session that is not closing
#[cfg(any(not(target_os = "macos"), test))]
pub fn graphical_session_user(text: &str) -> Option<String> {
    let get = |key: &str| text.lines().find_map(|l| l.strip_prefix(key).and_then(|r| r.strip_prefix('=')));
    let graphical = matches!(get("TYPE"), Some("wayland" | "x11" | "mir"));
    let open = !matches!(get("STATE"), Some("closing"));
    (graphical && open).then(|| get("USER").map(str::to_string)).flatten()
}

// ---------------------------------------------------------------------------------------------------------------
// network

fn network() -> Outcome {
    let addresses = crate::status::status().addresses;
    // getaddrinfo can hang for the resolver's whole timeout: on its own thread, waited for 5 s
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::net::ToSocketAddrs;
        let _ = tx.send(("omaflake.app", 443).to_socket_addrs().map(|mut a| a.next().is_some()).unwrap_or(false));
    });
    let dns = rx.recv_timeout(Duration::from_secs(5)).unwrap_or(false);
    let target = std::net::SocketAddr::from(([1, 1, 1, 1], 443));
    let internet = std::net::TcpStream::connect_timeout(&target, Duration::from_secs(3)).is_ok();
    let status = if dns && internet { Status::Pass } else { Status::Fail };
    let detail = match (dns, internet) {
        (true, true) => "Names resolve and the internet is reachable",
        (false, true) => "The internet is reachable, but omaflake.app does not resolve (DNS)",
        (true, false) => "omaflake.app resolves, but 1.1.1.1:443 is not reachable within 3 s",
        (false, false) => "No DNS and no connection to the internet",
    };
    Outcome::new(status, detail).with(json!({ "addresses": addresses, "dns": dns, "internet": internet }))
}

// ---------------------------------------------------------------------------------------------------------------
// sound and microphone

pub const RATE: u32 = 44_100;

/// A 16-bit mono PCM WAV with a sine tone (amplitude 0.3, 10 ms fade in and out against clicks)
pub fn tone_wav(freq: f64, secs: f64, rate: u32) -> Vec<u8> {
    let n = (secs * rate as f64) as usize;
    let fade = (rate as usize / 100).max(1);
    let samples = (0..n).map(|i| {
        let ramp = (i.min(n - 1 - i) as f64 / fade as f64).min(1.0);
        let v = (2.0 * std::f64::consts::PI * freq * i as f64 / rate as f64).sin() * 0.3 * ramp;
        (v * i16::MAX as f64) as i16
    });
    wav(samples, rate, n)
}

fn wav(samples: impl Iterator<Item = i16>, rate: u32, n: usize) -> Vec<u8> {
    let data_len = (n * 2) as u32;
    let mut out = Vec::with_capacity(44 + n * 2);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes()); // PCM
    out.extend_from_slice(&1u16.to_le_bytes()); // mono
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * 2).to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for s in samples {
        out.extend_from_slice(&s.to_le_bytes());
    }
    out
}

/// The 16-bit samples of a PCM WAV (all channels interleaved). A data chunk whose size is 0 or too large – a
/// recorder stopped before it fixed the header – counts up to the end of the file.
#[cfg(any(not(target_os = "macos"), test))]
pub fn wav_samples(bytes: &[u8]) -> Result<Vec<i16>, String> {
    if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err("not a WAV file".into());
    }
    let mut at = 12;
    let mut bits = None;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size = u32::from_le_bytes(bytes[at + 4..at + 8].try_into().unwrap()) as usize;
        let body = at + 8;
        if id == b"fmt " && body + 16 <= bytes.len() {
            let format = u16::from_le_bytes([bytes[body], bytes[body + 1]]);
            if format != 1 && format != 0xFFFE {
                return Err(format!("WAV format {format}, not PCM"));
            }
            bits = Some(u16::from_le_bytes([bytes[body + 14], bytes[body + 15]]));
        } else if id == b"data" {
            if bits != Some(16) {
                return Err("the WAV is not 16-bit PCM".into());
            }
            let end = if size == 0 || body + size > bytes.len() { bytes.len() } else { body + size };
            return Ok(bytes[body..end].as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)).collect());
        }
        at = body + size + (size & 1);
    }
    Err("the WAV has no data".into())
}

/// (RMS, peak) of 16-bit samples, both 0…1
pub fn levels(samples: &[i16]) -> (f64, f64) {
    if samples.is_empty() {
        return (0.0, 0.0);
    }
    let mut sum = 0.0;
    let mut peak = 0.0f64;
    for &s in samples {
        let v = s as f64 / 32768.0;
        sum += v * v;
        peak = peak.max(v.abs());
    }
    ((sum / samples.len() as f64).sqrt(), peak)
}

/// What the microphone check says about a recording
pub fn judge(samples: &[i16]) -> Outcome {
    let (rms, peak) = levels(samples);
    let data = json!({ "level": (rms * 1e4).round() / 1e4, "peak": (peak * 1e4).round() / 1e4 });
    if rms > 0.01 {
        Outcome::new(Status::Pass, format!("The microphone hears sound (level {rms:.3})")).with(data)
    } else {
        Outcome::new(Status::Confirm, "No sound reached the microphone – say something and run again").with(data)
    }
}

/// A file in the temp folder, moved away (removed) when dropped – the product removes its own temp files
struct TempFile(PathBuf);

impl TempFile {
    fn new(stem: &str) -> TempFile {
        let nanos =
            std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
        TempFile(std::env::temp_dir().join(format!("omaflake-{stem}-{}-{nanos}.wav", std::process::id())))
    }
}

impl Drop for TempFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Runs a command for at most `limit`; `interrupt` sends SIGINT after that time (recorders that run until stopped).
/// Returns its exit status (None when interrupted) and the end of its error output.
fn run_for(
    mut cmd: Command,
    limit: Duration,
    interrupt: Option<Duration>,
) -> Result<(Option<ExitStatus>, String), String> {
    cmd.stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::piped());
    let program = cmd.get_program().to_string_lossy().into_owned();
    let mut child = cmd.spawn().map_err(|e| format!("{program}: {e}"))?;
    let mut stderr = child.stderr.take();
    let reader = std::thread::spawn(move || {
        let mut bytes = vec![];
        if let Some(e) = stderr.as_mut() {
            let _ = e.read_to_end(&mut bytes);
        }
        let text = String::from_utf8_lossy(&bytes).trim().to_string();
        let last: Vec<&str> = text.lines().rev().take(3).collect();
        last.into_iter().rev().collect::<Vec<_>>().join(" ")
    });
    let start = Instant::now();
    let mut interrupted = false;
    loop {
        if let Some(status) = child.try_wait().map_err(|e| e.to_string())? {
            let err = reader.join().unwrap_or_default();
            return Ok(((!interrupted).then_some(status), err));
        }
        let elapsed = start.elapsed();
        if let Some(after) = interrupt
            && !interrupted
            && elapsed >= after
        {
            // SAFETY: the child's pid; it has not been waited for, so it is still ours
            unsafe { libc::kill(child.id() as libc::pid_t, libc::SIGINT) };
            interrupted = true;
        }
        if elapsed >= limit {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!("{program} did not finish within {} s", limit.as_secs()));
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

fn exit_error(program: &str, status: ExitStatus, err: &str) -> String {
    if err.is_empty() { format!("{program} failed ({status})") } else { format!("{program}: {err}") }
}

fn sound() -> Outcome {
    let file = TempFile::new("tone");
    if let Err(e) = std::fs::write(&file.0, tone_wav(440.0, 1.0, RATE)) {
        return fail(format!("Could not write the tone: {e}"));
    }
    let Some((name, program)) = player() else {
        return fail("No audio player found (pw-play, paplay or aplay)");
    };
    let mut cmd = Command::new(&program);
    cmd.arg(&file.0);
    match run_for(cmd, Duration::from_secs(10), None) {
        Ok((Some(s), _)) if s.success() => Outcome::new(Status::Confirm, "Did you hear a tone?"),
        Ok((Some(s), err)) => fail(exit_error(name, s, &err)),
        Ok((None, _)) => fail(format!("{name} was interrupted")),
        Err(e) => fail(e),
    }
}

#[cfg(target_os = "macos")]
fn player() -> Option<(&'static str, PathBuf)> {
    Some(("afplay", PathBuf::from("/usr/bin/afplay")))
}

#[cfg(not(target_os = "macos"))]
fn player() -> Option<(&'static str, PathBuf)> {
    ["pw-play", "paplay", "aplay"].into_iter().find_map(|n| find_tool(n).map(|p| (n, p)))
}

/// A program in PATH or in NixOS's system and per-user profiles (a systemd user unit has a short PATH)
#[cfg(not(target_os = "macos"))]
fn find_tool(name: &str) -> Option<PathBuf> {
    use std::os::unix::fs::PermissionsExt;
    let mut dirs: Vec<PathBuf> =
        std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    dirs.push("/run/current-system/sw/bin".into());
    if let Ok(user) = std::env::var("USER") {
        dirs.push(format!("/etc/profiles/per-user/{user}/bin").into());
    }
    dirs.extend(["/usr/bin".into(), "/bin".into()]);
    dirs.into_iter()
        .map(|d| d.join(name))
        .find(|p| std::fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0))
}

/// Linux: 2 s from the default input with pw-record, parecord or arecord into a temp WAV
#[cfg(not(target_os = "macos"))]
fn microphone() -> Outcome {
    let file = TempFile::new("mic");
    let recorders: [(&str, &[&str], bool); 3] = [
        ("pw-record", &["--rate", "44100", "--channels", "1", "--format", "s16"], true),
        ("parecord", &["--rate=44100", "--channels=1", "--format=s16le", "--file-format=wav"], true),
        ("arecord", &["-q", "-d", "2", "-f", "S16_LE", "-r", "44100", "-c", "1", "-t", "wav"], false),
    ];
    let Some((name, program, args, until_stopped)) =
        recorders.into_iter().find_map(|(n, a, s)| find_tool(n).map(|p| (n, p, a, s)))
    else {
        return fail("No recorder found (pw-record, parecord or arecord)");
    };
    let mut cmd = Command::new(&program);
    cmd.args(args).arg(&file.0);
    let interrupt = until_stopped.then_some(Duration::from_secs(2));
    match run_for(cmd, Duration::from_secs(8), interrupt) {
        Ok((Some(s), err)) if !s.success() => return fail(exit_error(name, s, &err)),
        Ok(_) => {}
        Err(e) => return fail(e),
    }
    match std::fs::read(&file.0).map_err(|e| e.to_string()).and_then(|b| wav_samples(&b)) {
        Ok(samples) if samples.is_empty() => fail(format!("{name} recorded nothing")),
        Ok(samples) => judge(&samples),
        Err(e) => fail(format!("{name}: {e}")),
    }
}

/// macOS: 2 s from the default input through an AudioToolbox input queue (16-bit mono, 44.1 kHz).
/// OPEN: whether TCC lets the agent – started from an SSH session, not an app – use the microphone; denied access may
/// come as an error or as silence (then the check says `confirm`).
#[cfg(target_os = "macos")]
fn microphone() -> Outcome {
    // AudioToolbox may block (seen in a VM without audio devices: no answer for more than 30 s) – the recording runs on
    // its own thread and the check gives up after 8 s; a thread that hangs stays behind
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(mac_audio::record(Duration::from_secs(2)));
    });
    let recorded = rx.recv_timeout(Duration::from_secs(8)).unwrap_or_else(|_| {
        Err("The microphone did not answer within 8 s (no input device, or macOS is asking for permission)".into())
    });
    match recorded {
        Ok(samples) if samples.is_empty() => {
            fail("The microphone gave no samples (no input device, or no permission to use the microphone)")
        }
        Ok(samples) => judge(&samples),
        Err(e) => fail(e),
    }
}

#[cfg(target_os = "macos")]
mod mac_audio {
    use std::ffi::c_void;
    use std::sync::Mutex;
    use std::time::Duration;

    type AudioQueueRef = *mut c_void;
    type OSStatus = i32;

    /// AudioQueue.h, AudioQueueBuffer
    #[repr(C)]
    struct AudioQueueBuffer {
        audio_data_bytes_capacity: u32,
        audio_data: *mut c_void,
        audio_data_byte_size: u32,
        user_data: *mut c_void,
        packet_description_capacity: u32,
        packet_descriptions: *mut c_void,
        packet_description_count: u32,
    }

    /// CoreAudioBaseTypes.h, AudioStreamBasicDescription
    #[repr(C)]
    struct AudioStreamBasicDescription {
        sample_rate: f64,
        format_id: u32,
        format_flags: u32,
        bytes_per_packet: u32,
        frames_per_packet: u32,
        bytes_per_frame: u32,
        channels_per_frame: u32,
        bits_per_channel: u32,
        reserved: u32,
    }

    type InputCallback =
        extern "C" fn(*mut c_void, AudioQueueRef, *mut AudioQueueBuffer, *const c_void, u32, *const c_void);

    #[link(name = "AudioToolbox", kind = "framework")]
    unsafe extern "C" {
        fn AudioQueueNewInput(
            format: *const AudioStreamBasicDescription,
            callback: InputCallback,
            user_data: *mut c_void,
            run_loop: *const c_void,
            run_loop_mode: *const c_void,
            flags: u32,
            out: *mut AudioQueueRef,
        ) -> OSStatus;
        fn AudioQueueAllocateBuffer(aq: AudioQueueRef, size: u32, out: *mut *mut AudioQueueBuffer) -> OSStatus;
        fn AudioQueueEnqueueBuffer(
            aq: AudioQueueRef,
            buf: *mut AudioQueueBuffer,
            n: u32,
            descs: *const c_void,
        ) -> OSStatus;
        fn AudioQueueStart(aq: AudioQueueRef, start: *const c_void) -> OSStatus;
        fn AudioQueueStop(aq: AudioQueueRef, immediate: u8) -> OSStatus;
        fn AudioQueueDispose(aq: AudioQueueRef, immediate: u8) -> OSStatus;
    }

    const LPCM: u32 = u32::from_be_bytes(*b"lpcm");
    const SIGNED_INTEGER: u32 = 1 << 2;
    const PACKED: u32 = 1 << 3;

    /// The samples, filled on the queue's own thread (no run loop given)
    extern "C" fn on_input(
        user: *mut c_void,
        aq: AudioQueueRef,
        buf: *mut AudioQueueBuffer,
        _: *const c_void,
        _: u32,
        _: *const c_void,
    ) {
        // SAFETY: `user` is the Mutex `record` keeps alive until the queue is disposed; `buf` is a buffer of this
        // queue, its data valid for `audio_data_byte_size` bytes during the callback
        unsafe {
            let samples = &*(user as *const Mutex<Vec<i16>>);
            let b = &*buf;
            let data = std::slice::from_raw_parts(b.audio_data as *const u8, b.audio_data_byte_size as usize);
            if let Ok(mut s) = samples.lock() {
                s.extend(data.as_chunks::<2>().0.iter().map(|c| i16::from_le_bytes(*c)));
            }
            AudioQueueEnqueueBuffer(aq, buf, 0, std::ptr::null()); // fails harmlessly once the queue stops
        }
    }

    pub fn record(length: Duration) -> Result<Vec<i16>, String> {
        let format = AudioStreamBasicDescription {
            sample_rate: super::RATE as f64,
            format_id: LPCM,
            format_flags: SIGNED_INTEGER | PACKED,
            bytes_per_packet: 2,
            frames_per_packet: 1,
            bytes_per_frame: 2,
            channels_per_frame: 1,
            bits_per_channel: 16,
            reserved: 0,
        };
        let samples: Box<Mutex<Vec<i16>>> = Box::new(Mutex::new(Vec::with_capacity(super::RATE as usize * 3)));
        let user = &*samples as *const Mutex<Vec<i16>> as *mut c_void;
        let mut aq: AudioQueueRef = std::ptr::null_mut();
        // SAFETY: format and aq are valid; `samples` outlives the queue (disposed below before it drops)
        let rc = unsafe { AudioQueueNewInput(&format, on_input, user, std::ptr::null(), std::ptr::null(), 0, &mut aq) };
        if rc != 0 {
            return Err(format!("The microphone could not be opened (AudioQueueNewInput: OSStatus {rc})"));
        }
        let result = (|| {
            for _ in 0..3 {
                let mut buf: *mut AudioQueueBuffer = std::ptr::null_mut();
                // SAFETY: aq is a live queue; buf is written on success
                let rc = unsafe { AudioQueueAllocateBuffer(aq, super::RATE / 10 * 2, &mut buf) };
                if rc != 0 {
                    return Err(format!("AudioQueueAllocateBuffer: OSStatus {rc}"));
                }
                // SAFETY: buf belongs to aq
                let rc = unsafe { AudioQueueEnqueueBuffer(aq, buf, 0, std::ptr::null()) };
                if rc != 0 {
                    return Err(format!("AudioQueueEnqueueBuffer: OSStatus {rc}"));
                }
            }
            // SAFETY: aq is a live queue
            let rc = unsafe { AudioQueueStart(aq, std::ptr::null()) };
            if rc != 0 {
                return Err(format!(
                    "The microphone could not be started (AudioQueueStart: OSStatus {rc}) – no input device or no permission"
                ));
            }
            std::thread::sleep(length);
            Ok(())
        })();
        // SAFETY: aq is a live queue; immediate stop and dispose, so no callback runs after this
        unsafe {
            AudioQueueStop(aq, 1);
            AudioQueueDispose(aq, 1);
        }
        result?;
        let samples = samples.lock().map(|s| s.clone()).unwrap_or_default();
        Ok(samples)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn request_and_answer_json() {
        let r = parse_request(br#"{"id":7,"check":"share-read","args":{"name":"a.txt"}}"#).unwrap();
        assert_eq!((r.id, r.check.as_str()), (7, "share-read"));
        assert_eq!(r.args["name"], "a.txt");
        let r = parse_request(br#"{"id":8,"check":"info"}"#).unwrap();
        assert!(r.args.is_null());
        assert_eq!(parse_request(br#"{"id":9,"check":5}"#).unwrap_err(), Some(9));
        assert_eq!(parse_request(br#"{"id":-1,"check":"info"}"#).unwrap_err(), None);
        assert_eq!(parse_request(b"nope").unwrap_err(), None);

        let a = Outcome::new(Status::Confirm, "Did you hear a tone?").answer(&r);
        let v: Value = serde_json::from_slice(&encode(&a)).unwrap();
        assert_eq!(v, json!({ "id": 8, "check": "info", "status": "confirm", "detail": "Did you hear a tone?" }));
        let with = Outcome::new(Status::Pass, "x").with(json!({ "text": "t" })).answer(&r);
        let back: Answer = serde_json::from_slice(&encode(&with)).unwrap();
        assert_eq!(back, with);
        assert_eq!(serde_json::to_value(malformed(3)).unwrap()["status"], "fail");
    }

    #[test]
    fn unknown_checks_are_skipped() {
        let r = Request { id: 1, check: "teleport".into(), args: Value::Null };
        assert!(!known(&r.check));
        assert_eq!(place(&r.check), Place::Agent);
        let a = run(&r, Context::default(), None);
        assert_eq!((a.id, a.status, a.check.as_str()), (1, Status::Skipped, "teleport"));
        assert_eq!(unknown(&r).status, Status::Skipped);
    }

    #[test]
    fn places() {
        assert_eq!(place("midi"), Place::Agent);
        assert_eq!(place("network"), Place::Worker);
        if cfg!(target_os = "linux") {
            assert_eq!(place("sound"), Place::Session { fallback: false });
            assert_eq!(place("share-read"), Place::Session { fallback: true });
        } else {
            assert_eq!(place("sound"), Place::Worker);
            assert_eq!(place("share-read"), Place::Worker);
        }
    }

    #[test]
    fn names() {
        for ok in ["a.txt", ".omaflake-diagnose", "with space", "ü"] {
            assert!(valid_name(ok), "{ok}");
        }
        for bad in ["", ".", "..", "a/b", "/etc/passwd", "../x", "a\0b", &"x".repeat(256)] {
            assert!(!valid_name(bad), "{bad}");
        }
    }

    #[test]
    fn volume_folders() {
        assert_eq!(volume_dir(&json!({})).unwrap(), Path::new(SHARE));
        assert_eq!(volume_dir(&json!({"volume": "projects"})).unwrap(), Path::new(SHARE).join("projects"));
        assert!(volume_dir(&json!({"volume": "../etc"})).is_err());
    }

    #[test]
    fn share_round_trip_and_errors() {
        let dir = std::env::temp_dir().join(format!("omaflake-diag-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let w = share_write(&dir, &json!({ "name": "t.txt", "text": "hällo" }));
        assert_eq!(w.status, Status::Pass, "{}", w.detail);
        let r = share_read(&dir, &json!({ "name": "t.txt" }));
        assert_eq!(r.status, Status::Pass);
        assert_eq!(r.data.unwrap()["text"], "hällo");
        assert_eq!(share_write(&dir, &json!({ "name": "../t.txt", "text": "x" })).status, Status::Fail);
        assert_eq!(share_write(&dir, &json!({ "name": "t.txt" })).status, Status::Fail);
        let long = "x".repeat(MAX_TEXT + 1);
        assert_eq!(share_write(&dir, &json!({ "name": "t.txt", "text": long })).status, Status::Fail);
        assert_eq!(share_read(&dir, &json!({ "name": "missing" })).status, Status::Fail);
        let gone = share_read(&dir.join("nope"), &json!({ "name": "t.txt" }));
        assert_eq!(gone.status, Status::Fail);
        assert!(gone.detail.contains("missing"), "{}", gone.detail);
        let _ = std::fs::remove_file(dir.join("t.txt"));
        let _ = std::fs::remove_dir(&dir);
    }

    #[test]
    fn tone_is_a_valid_wav() {
        let w = tone_wav(440.0, 1.0, RATE);
        assert_eq!(w.len(), 44 + RATE as usize * 2);
        let s = wav_samples(&w).unwrap();
        assert_eq!(s.len(), RATE as usize);
        let (rms, peak) = levels(&s);
        // a sine of amplitude 0.3 has an RMS of 0.3/√2 (a little less with the fades)
        assert!((rms - 0.3 / 2f64.sqrt()).abs() < 0.005, "{rms}");
        assert!((peak - 0.3).abs() < 0.001, "{peak}");
        // 440 Hz: 880 zero crossings in a second (± the fades)
        let crossings = s.windows(2).filter(|p| (p[0] < 0) != (p[1] < 0)).count();
        assert!((870..=890).contains(&crossings), "{crossings}");
    }

    #[test]
    fn wav_with_open_data_size_and_errors() {
        let mut w = tone_wav(440.0, 0.01, 8000);
        w[40..44].copy_from_slice(&0u32.to_le_bytes()); // a recorder that was stopped
        assert_eq!(wav_samples(&w).unwrap().len(), 80);
        assert!(wav_samples(b"RIFF....WAVE").is_err());
        assert!(wav_samples(b"hello").is_err());
    }

    #[test]
    fn levels_and_judgement() {
        assert_eq!(levels(&[]), (0.0, 0.0));
        let (rms, peak) = levels(&[16384, -16384]);
        assert!((rms - 0.5).abs() < 1e-9 && (peak - 0.5).abs() < 1e-9);
        assert_eq!(judge(&[0; 100]).status, Status::Confirm);
        assert_eq!(judge(&[3000, -3000]).status, Status::Pass);
        assert_eq!(judge(&[100, -100]).data.unwrap()["level"], 0.0031);
    }

    #[test]
    fn logind_session_files() {
        let wayland = "UID=1000\nUSER=alice\nACTIVE=1\nSTATE=active\nTYPE=wayland\nCLASS=user\n";
        assert_eq!(graphical_session_user(wayland).as_deref(), Some("alice"));
        assert_eq!(graphical_session_user("USER=bob\nTYPE=tty\nSTATE=online\n"), None);
        assert_eq!(graphical_session_user("USER=bob\nTYPE=x11\nSTATE=closing\n"), None);
    }

    #[test]
    fn details_and_text_are_cut() {
        assert!(short(&"x".repeat(1000)).chars().count() <= 241);
        assert_eq!(cut("äöü", 3), ("ä", true));
        assert_eq!(cut("abc", 3), ("abc", false));
    }

    struct Memory(std::cell::RefCell<Option<String>>);
    impl Clipboard for Memory {
        fn read(&self) -> Option<(&'static str, Vec<u8>)> {
            self.0.borrow().as_ref().map(|t| (crate::clip::TEXT, t.as_bytes().to_vec()))
        }
        fn write(&self, text: &str) -> Result<(), String> {
            *self.0.borrow_mut() = Some(text.to_string());
            Ok(())
        }
    }

    #[test]
    fn clipboard_checks() {
        let c = Memory(Default::default());
        let read = Request { id: 2, check: "clipboard-read".into(), args: Value::Null };
        let a = run(&read, Context::default(), Some(&c));
        assert_eq!((a.status, a.data.unwrap()["text"].as_str()), (Status::Pass, Some("")));
        let write = Request { id: 3, check: "clipboard-write".into(), args: json!({ "text": "token-1" }) };
        assert_eq!(run(&write, Context::default(), Some(&c)).status, Status::Pass);
        assert_eq!(run(&read, Context::default(), Some(&c)).data.unwrap()["text"], "token-1");
        let bad = Request { id: 4, check: "clipboard-write".into(), args: json!({}) };
        assert_eq!(run(&bad, Context::default(), Some(&c)).status, Status::Fail);
        assert_eq!(run(&read, Context::default(), None).status, Status::Fail);
    }

    /// Runs the real checks of this machine once (sound plays a tone, the clipboard is restored afterwards):
    /// `cargo test -- --ignored --nocapture live_checks`
    #[test]
    #[ignore]
    fn live_checks() {
        struct Pb;
        impl Clipboard for Pb {
            fn read(&self) -> Option<(&'static str, Vec<u8>)> {
                crate::session::current_clipboard()
            }
            fn write(&self, text: &str) -> Result<(), String> {
                crate::session::write_clipboard(crate::clip::TEXT, text.as_bytes())
            }
        }
        let check = |i: u32, check: &str, args: Value| {
            let r = Request { id: i, check: check.into(), args };
            let start = Instant::now();
            let a = run(&r, Context::default(), Some(&Pb));
            println!("{check} ({:.1} s): {}", start.elapsed().as_secs_f64(), serde_json::to_string(&a).unwrap());
        };
        // The clipboard first, restored right away (before the checks that may hang)
        let before = Pb.read();
        check(0, "clipboard-write", json!({ "text": "omaflake-diagnose-test" }));
        check(1, "clipboard-read", Value::Null);
        if let Some((mime, data)) = before {
            let _ = crate::session::write_clipboard(mime, &data);
        }
        check(2, "info", Value::Null);
        check(3, "network", Value::Null);
        check(4, "sound", Value::Null);
        check(5, "microphone", Value::Null);
    }
}
