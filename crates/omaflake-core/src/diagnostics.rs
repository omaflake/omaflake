// SPDX-License-Identifier: AGPL-3.0-or-later
//! A running VM's diagnosis (plan B162; spec §11.9 "Diagnosis", api.md page `diagnostics`): every feature checked in
//! turn – the guest's half through the omaflake agent (`0x0600`/`0x0601`, relayed by whatever holds the VM's agent
//! link: omaf-runner's control socket for macOS guests, the bridge's `bridge.sock` for NixOS guests), this computer's
//! half here (the clipboard token, the share file, SSH, the agent's report).
//!
//! The checks run in the order of `CHECKS`; each logs one line `✓|✗|?|– <Title>: <detail>` and its details below it.
//! A switch that is off makes its check `skipped`. Everything outside this process goes through `Probe`, so the logic
//! is tested with a fake one.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use crate::agent_protocol::AgentState;
use crate::cancellation::Cancellation;
use crate::models::{GuestKind, InstanceConfig, NetworkMode};
use crate::paths::Paths;

/// The checks, in the order they run
pub const CHECKS: [&str; 10] =
    ["agent", "ssh", "clipboard", "share", "network", "sound", "microphone", "camera", "midi", "disk"];

/// A check's title (a key of the views' texts)
pub fn title(id: &str) -> &'static str {
    match id {
        "agent" => "Agent",
        "ssh" => "SSH",
        "clipboard" => "Clipboard",
        "share" => "Share folder",
        "network" => "Network",
        "sound" => "Sound",
        "microphone" => "Microphone",
        "camera" => "Camera",
        "midi" => "MIDI",
        "disk" => "Disk",
        _ => "?",
    }
}

/// The details this module writes itself (English; the views show them translated – keys with German), as opposed to
/// what a guest or a tool said (shown as it is)
pub const OWN_DETAILS: [&str; 10] = [
    SKIP_CLIPBOARD,
    SKIP_SHARE,
    SKIP_NETWORK,
    SKIP_SOUND,
    SKIP_MICROPHONE,
    SKIP_CAMERA,
    SKIP_MIDI,
    NO_DIAGNOSIS,
    NO_AGENT,
    PASS_BOTH_WAYS,
];
pub const SKIP_CLIPBOARD: &str = "Clipboard is off for this VM";
pub const SKIP_SHARE: &str = "The share folder is off for this VM";
pub const SKIP_NETWORK: &str = "No network for this VM";
pub const SKIP_SOUND: &str = "Sound is off for this VM";
pub const SKIP_MICROPHONE: &str = "The microphone is off for this VM";
pub const SKIP_CAMERA: &str = "The camera is off for this VM";
pub const SKIP_MIDI: &str = "MIDI is off for this VM";
pub const NO_DIAGNOSIS: &str = "The guest's agent has no diagnosis – an image built before it";
pub const NO_AGENT: &str = "Not checked – no word from the guest's agent";
pub const PASS_BOTH_WAYS: &str = "both ways";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum CheckStatus {
    Pending,
    Running,
    Pass,
    Fail,
    /// Only the user can tell (did you hear it?)
    Confirm,
    Skipped,
}

impl CheckStatus {
    /// The log's mark
    pub fn mark(self) -> &'static str {
        match self {
            CheckStatus::Pass => "✓",
            CheckStatus::Fail => "✗",
            CheckStatus::Confirm => "?",
            CheckStatus::Skipped => "–",
            CheckStatus::Pending | CheckStatus::Running => "·",
        }
    }

    pub fn is_done(self) -> bool {
        !matches!(self, CheckStatus::Pending | CheckStatus::Running)
    }
}

/// One check's outcome (the job's `result.checks[]`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CheckResult {
    pub id: String,
    pub status: CheckStatus,
    pub detail: String,
}

impl CheckResult {
    pub fn new(id: &str, status: CheckStatus, detail: impl Into<String>) -> CheckResult {
        CheckResult { id: id.into(), status, detail: detail.into() }
    }

    /// `✓ Clipboard: both ways`
    pub fn line(&self) -> String {
        if self.detail.is_empty() {
            format!("{} {}", self.status.mark(), title(&self.id))
        } else {
            format!("{} {}: {}", self.status.mark(), title(&self.id), self.detail)
        }
    }
}

// MARK: The relay

/// The socket of whatever holds the VM's agent link: omaf-runner's control socket (macOS guests) or the QEMU bridge's
/// (NixOS guests, `--control`)
pub fn relay_socket(paths: &Paths, c: &InstanceConfig) -> PathBuf {
    let ip = paths.instance(&c.name);
    match c.guest_kind() {
        GuestKind::Macos => ip.runner_socket(),
        GuestKind::Nixos => ip.bridge_socket(),
    }
}

/// One control request – a JSON object and a newline, as `RunnerClient` sends it – and the one JSON object that comes
/// back (within `timeout`)
pub fn control_request(socket: &Path, request: &Value, timeout: Duration) -> Result<Value, String> {
    use std::io::{Read, Write};
    let mut stream =
        crate::sockets::connect_unix(socket, Duration::from_secs(2)).map_err(|e| e.message().to_string())?;
    let _ = stream.set_read_timeout(Some(timeout));
    let mut line = serde_json::to_vec(request).map_err(|e| e.to_string())?;
    line.push(b'\n');
    stream.write_all(&line).map_err(|e| format!("{}: {e}", socket.display()))?;
    let mut data = Vec::new();
    let mut buf = [0u8; 16384];
    while !data.contains(&b'\n') && data.len() < 4 << 20 {
        match stream.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => data.extend_from_slice(&buf[..n]),
            Err(e) if data.is_empty() => return Err(format!("no answer from {}: {e}", socket.display())),
            Err(_) => break,
        }
    }
    serde_json::from_slice(&data).map_err(|_| format!("not an answer: {}", String::from_utf8_lossy(&data).trim()))
}

// MARK: This computer's clipboard

/// This computer's clipboard as text
pub trait HostClipboard {
    fn read(&self) -> Option<String>;
    fn write(&self, text: &str) -> bool;
}

/// macOS: pbpaste / pbcopy
struct Pasteboard;

impl HostClipboard for Pasteboard {
    fn read(&self) -> Option<String> {
        let out = std::process::Command::new("/usr/bin/pbpaste").output().ok()?;
        out.status.success().then(|| String::from_utf8_lossy(&out.stdout).into_owned())
    }

    fn write(&self, text: &str) -> bool {
        crate::credentials::copy_to_clipboard(text).is_ok()
    }
}

/// Linux: the bridge's clipboard backend (Wayland or X11)
struct Backend(std::sync::Arc<dyn crate::host_bridge::clipboard::ClipboardBackend>);

impl HostClipboard for Backend {
    fn read(&self) -> Option<String> {
        self.0
            .read()
            .filter(|(mime, _)| mime == crate::host_bridge::clipboard::TEXT)
            .map(|(_, d)| String::from_utf8_lossy(&d).into_owned())
    }

    fn write(&self, text: &str) -> bool {
        self.0.write(crate::host_bridge::clipboard::TEXT, text.as_bytes())
    }
}

/// This computer's clipboard, or why there is none
pub fn host_clipboard() -> Result<Box<dyn HostClipboard>, String> {
    if cfg!(target_os = "macos") {
        Ok(Box::new(Pasteboard))
    } else {
        crate::host_bridge::clipboard::detect().map(|b| Box::new(Backend(b)) as Box<dyn HostClipboard>)
    }
}

/// Puts `text` on this computer's clipboard (`diagnostics.copy`)
pub fn copy_text(text: &str) -> bool {
    if cfg!(target_os = "macos") {
        return Pasteboard.write(text);
    }
    match crate::host_bridge::clipboard::detect() {
        Ok(b) => b.write(crate::host_bridge::clipboard::TEXT, text.as_bytes()),
        Err(_) => crate::credentials::copy_to_clipboard(text).is_ok(),
    }
}

// MARK: What the checks need from outside

/// Everything a diagnosis touches outside this process
pub trait Probe {
    /// The agent's report (agent.json)
    fn agent_state(&self) -> Option<AgentState>;
    /// A control request to the holder of the agent link (`{"command": "diagnose" | "midi", …}`)
    fn relay(&self, request: &Value) -> Result<Value, String>;
    /// `ssh -o BatchMode=yes … true`: None while there is no address; Err with ssh's stderr
    fn ssh(&self) -> Option<Result<(), String>>;
    fn clipboard(&self) -> Result<Box<dyn HostClipboard + '_>, String>;
    /// The VM's share folder on this computer
    fn share_dir(&self) -> PathBuf;
}

/// The real one: the data folder, the VM's sockets, ssh, the clipboard
pub struct LiveProbe {
    pub paths: Paths,
    pub config: InstanceConfig,
}

impl Probe for LiveProbe {
    fn agent_state(&self) -> Option<AgentState> {
        AgentState::load(&self.paths.instance(&self.config.name).agent_state())
    }

    fn relay(&self, request: &Value) -> Result<Value, String> {
        // The holder waits up to 15 s for the agent; a little more here
        control_request(
            &relay_socket(&self.paths, &self.config),
            request,
            crate::agent_protocol::AgentProtocol::DIAGNOSE_TIMEOUT + Duration::from_secs(5),
        )
    }

    fn ssh(&self) -> Option<Result<(), String>> {
        let c = &self.config;
        let mut args = match c.guest_kind() {
            GuestKind::Macos => {
                let backends = crate::backend::Backends::new(self.paths.clone()).ok()?;
                let vz = backends.vz().ok()?;
                let host = vz.address(c)?;
                vz.ssh_arguments_to(c, &host, &["true".into()], true)
            }
            GuestKind::Nixos => {
                let mut a = crate::ssh_command::SshCommand::arguments(
                    c,
                    &self.paths,
                    crate::ssh_command::SshCommand::DEFAULT_HOST,
                    None,
                    &["true".into()],
                );
                a.splice(1..1, ["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"].map(String::from));
                a
            }
        };
        let exe = args.remove(0);
        Some(match crate::shell::Shell::run(&exe, &args, &crate::shell::RunOptions::default()) {
            Ok(r) if r.status == 0 => Ok(()),
            Ok(r) => Err(if r.stderr.trim().is_empty() {
                format!("ssh exited with {}", r.status)
            } else {
                r.stderr.trim().to_string()
            }),
            Err(e) => Err(e.message().to_string()),
        })
    }

    fn clipboard(&self) -> Result<Box<dyn HostClipboard + '_>, String> {
        host_clipboard()
    }

    fn share_dir(&self) -> PathBuf {
        self.paths.share_dir(&self.config)
    }
}

// MARK: The diagnosis

/// What the guest's agent answered (`0x0601`)
#[derive(Debug, Clone)]
struct Answer {
    status: CheckStatus,
    detail: String,
    data: Value,
}

/// One diagnosis of one VM
pub struct Diagnosis<'a> {
    pub config: &'a InstanceConfig,
    pub probe: &'a dyn Probe,
    pub log: &'a dyn Fn(&str),
    /// Each check as it starts (`running`) and when it is done
    pub on_check: &'a dyn Fn(&CheckResult),
    pub cancel: Option<&'a Cancellation>,
    /// How long the clipboard and the share file are waited for, and how often looked at (5 s, 0.3 s)
    pub wait: Duration,
    pub poll: Duration,
    /// Set by the agent check: why guest checks cannot run (None: they can)
    unavailable: Option<String>,
    info: Value,
}

impl<'a> Diagnosis<'a> {
    pub fn new(
        config: &'a InstanceConfig,
        probe: &'a dyn Probe,
        log: &'a dyn Fn(&str),
        on_check: &'a dyn Fn(&CheckResult),
    ) -> Diagnosis<'a> {
        Diagnosis {
            config,
            probe,
            log,
            on_check,
            cancel: None,
            wait: Duration::from_secs(5),
            poll: Duration::from_millis(300),
            unavailable: None,
            info: Value::Null,
        }
    }

    /// Runs every check in order; stops early only when cancelled
    pub fn run(&mut self) -> Vec<CheckResult> {
        let c = self.config;
        (self.log)(&format!(
            "Diagnosis of {} ({}, image {}) – omaflake {} on {}",
            c.name,
            match c.guest_kind() {
                GuestKind::Macos => "macOS guest",
                GuestKind::Nixos => "NixOS guest",
            },
            c.image,
            crate::VERSION,
            std::env::consts::OS
        ));
        let mut results = Vec::new();
        for id in CHECKS {
            if self.cancel.is_some_and(Cancellation::is_cancelled) {
                break;
            }
            (self.on_check)(&CheckResult::new(id, CheckStatus::Running, ""));
            let r = self.check(id);
            (self.log)(&r.line());
            (self.on_check)(&r);
            results.push(r);
        }
        let failed: Vec<&str> =
            results.iter().filter(|r| r.status == CheckStatus::Fail).map(|r| title(&r.id)).collect();
        (self.log)(&if failed.is_empty() {
            "Done – nothing failed".to_string()
        } else {
            format!("Done – failed: {}", failed.join(", "))
        });
        results
    }

    fn check(&mut self, id: &str) -> CheckResult {
        let c = self.config;
        let r = |status, detail: String| CheckResult::new(id, status, detail);
        match id {
            "agent" => self.agent(),
            "ssh" => self.ssh(),
            "clipboard" if !c.clipboard_on() => r(CheckStatus::Skipped, SKIP_CLIPBOARD.into()),
            "clipboard" => self.clipboard(),
            "share" if !c.share => r(CheckStatus::Skipped, SKIP_SHARE.into()),
            "share" => self.share(),
            "network" if c.network == NetworkMode::None => r(CheckStatus::Skipped, SKIP_NETWORK.into()),
            "network" => self.network(),
            "sound" if !c.sound_on() => r(CheckStatus::Skipped, SKIP_SOUND.into()),
            "microphone" if !c.microphone_on() => r(CheckStatus::Skipped, SKIP_MICROPHONE.into()),
            "camera" if !c.camera_on() => r(CheckStatus::Skipped, SKIP_CAMERA.into()),
            // macOS too since 0.7.5 (plan B169): the guest's agent checks the stream, OBS and its camera extension
            "sound" | "microphone" | "camera" => self.passed_through(id),
            "midi" if !c.midi_on() => r(CheckStatus::Skipped, SKIP_MIDI.into()),
            "midi" => self.midi(),
            "disk" => self.disk(),
            _ => r(CheckStatus::Skipped, String::new()),
        }
    }

    /// Asks the guest's agent one check; Err when it cannot answer (with why)
    fn guest(&self, check: &str, args: Option<Value>) -> Result<Answer, String> {
        if let Some(why) = &self.unavailable {
            return Err(why.clone());
        }
        let mut request = json!({"command": "diagnose", "check": check});
        if let Some(a) = args {
            request["args"] = a;
        }
        let raw = self.probe.relay(&request)?;
        if let Some(e) = raw.get("error").and_then(Value::as_str) {
            return Err(e.to_string());
        }
        let status = match raw.get("status").and_then(Value::as_str) {
            Some("pass") => CheckStatus::Pass,
            Some("fail") => CheckStatus::Fail,
            Some("confirm") => CheckStatus::Confirm,
            Some("skipped") => CheckStatus::Skipped,
            other => return Err(format!("an answer without a known status ({other:?})")),
        };
        let detail = raw.get("detail").and_then(Value::as_str).unwrap_or("").to_string();
        let data = raw.get("data").cloned().unwrap_or(Value::Null);
        (self.log)(&format!("    guest {check}: {raw}"));
        Ok(Answer { status, detail, data })
    }

    /// A guest check that cannot run: skipped when the agent cannot be asked at all, else a failure
    fn not_asked(&self, id: &str, why: String) -> CheckResult {
        if self.unavailable.is_some() {
            CheckResult::new(id, CheckStatus::Skipped, why)
        } else {
            CheckResult::new(id, CheckStatus::Fail, why)
        }
    }

    fn agent(&mut self) -> CheckResult {
        let id = "agent";
        let Some(state) = self.probe.agent_state().filter(AgentState::is_ready) else {
            self.unavailable = Some(NO_AGENT.into());
            let why = if self.config.guest_kind() == GuestKind::Macos {
                "no word from the guest's agent – macOS guests start it over SSH (see SSH); images built before 0.7.4 have none"
            } else {
                "no word from the guest's agent – it starts with the guest; an image built before it has none"
            };
            return CheckResult::new(id, CheckStatus::Fail, why);
        };
        if let Some(problem) = &state.problem {
            self.unavailable = Some(NO_AGENT.into());
            return CheckResult::new(id, CheckStatus::Fail, problem.clone());
        }
        // Ready means it said hello
        let Some(hello) = state.hello.clone() else { return CheckResult::new(id, CheckStatus::Fail, NO_AGENT) };
        let os = hello.os_name.clone().or(hello.os.clone()).unwrap_or_default();
        let age = state.status_at.or(state.hello_at).map(|t| (crate::json::now() - t).num_seconds());
        (self.log)(&format!(
            "    agent {} · {os} · features {} · last report {} s ago",
            hello.agent,
            state.features.clone().unwrap_or_default().join(", "),
            age.map(|a| a.to_string()).unwrap_or_else(|| "?".into())
        ));
        if age.is_some_and(|a| a > 30) {
            self.unavailable = Some(NO_AGENT.into());
            return CheckResult::new(
                id,
                CheckStatus::Fail,
                format!("the agent's last report is {} s old", age.unwrap_or_default()),
            );
        }
        if state.features.as_ref().is_some_and(|f| !f.iter().any(|x| x == "diagnose")) {
            self.unavailable = Some(NO_DIAGNOSIS.into());
            return CheckResult::new(id, CheckStatus::Pass, format!("{} · {os} – {NO_DIAGNOSIS}", hello.agent));
        }
        let t0 = Instant::now();
        match self.guest("info", None) {
            Ok(a) => {
                self.info = a.data.clone();
                let ms = t0.elapsed().as_millis();
                let status = if a.status == CheckStatus::Fail { CheckStatus::Fail } else { CheckStatus::Pass };
                CheckResult::new(id, status, format!("{} · {os} · round trip {ms} ms", hello.agent))
            }
            Err(why) => {
                self.unavailable = Some(format!("Not checked – the agent did not answer: {why}"));
                CheckResult::new(id, CheckStatus::Fail, format!("the agent did not answer the diagnosis: {why}"))
            }
        }
    }

    fn ssh(&self) -> CheckResult {
        let id = "ssh";
        if self.config.guest_kind() == GuestKind::Nixos && self.config.network == NetworkMode::None {
            return CheckResult::new(id, CheckStatus::Skipped, SKIP_NETWORK);
        }
        match self.probe.ssh() {
            None => CheckResult::new(id, CheckStatus::Fail, "no address yet – the guest has not asked for one"),
            Some(Ok(())) => CheckResult::new(id, CheckStatus::Pass, "omaflake's key is accepted"),
            Some(Err(why)) => {
                for line in why.lines() {
                    (self.log)(&format!("    ssh: {line}"));
                }
                let last = why.lines().last().unwrap_or("").to_string();
                CheckResult::new(id, CheckStatus::Fail, last)
            }
        }
    }

    /// Polls `f` every `poll` until it gives Some or `wait` ran out
    fn until<T>(&self, mut f: impl FnMut() -> Option<T>) -> Option<T> {
        let deadline = Instant::now() + self.wait;
        loop {
            if let Some(v) = f() {
                return Some(v);
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(self.poll);
        }
    }

    fn clipboard(&self) -> CheckResult {
        let id = "clipboard";
        if let Some(why) = &self.unavailable {
            return CheckResult::new(id, CheckStatus::Skipped, why.clone());
        }
        if self.info.pointer("/helpers/clipboard") == Some(&Value::Bool(false)) {
            return CheckResult::new(
                id,
                CheckStatus::Fail,
                "the guest's clipboard helper does not run – macOS guests start it over SSH (see SSH)",
            );
        }
        let host = match self.probe.clipboard() {
            Ok(h) => h,
            Err(why) => return CheckResult::new(id, CheckStatus::Fail, format!("no clipboard here: {why}")),
        };
        let saved = host.read();
        let to_guest = token();
        let mut last_seen = String::new();
        let there = host.write(&to_guest)
            && self
                .until(|| match self.guest("clipboard-read", None) {
                    Ok(a) => {
                        let text = a.data.get("text").and_then(Value::as_str).unwrap_or("").to_string();
                        if text == to_guest {
                            Some(())
                        } else {
                            last_seen = text;
                            None
                        }
                    }
                    Err(_) => None,
                })
                .is_some();
        let from_guest = token();
        let back = match self.guest("clipboard-write", Some(json!({"text": from_guest}))) {
            Ok(a) if a.status != CheckStatus::Fail => {
                self.until(|| (host.read().as_deref() == Some(from_guest.as_str())).then_some(())).is_some()
            }
            Ok(a) => {
                (self.log)(&format!("    guest clipboard-write: {}", a.detail));
                false
            }
            Err(why) => {
                (self.log)(&format!("    guest clipboard-write: {why}"));
                false
            }
        };
        host.write(saved.as_deref().unwrap_or(""));
        (self.log)("    this computer's clipboard is back as it was");
        match (there, back) {
            (true, true) => CheckResult::new(id, CheckStatus::Pass, PASS_BOTH_WAYS),
            (false, true) => CheckResult::new(
                id,
                CheckStatus::Fail,
                format!("only guest → this computer; the guest's clipboard held {:?}", shorten(&last_seen)),
            ),
            (true, false) => CheckResult::new(id, CheckStatus::Fail, "only this computer → guest"),
            (false, false) => CheckResult::new(id, CheckStatus::Fail, "neither way"),
        }
    }

    fn share(&self) -> CheckResult {
        let id = "share";
        if let Some(why) = &self.unavailable {
            return CheckResult::new(id, CheckStatus::Skipped, why.clone());
        }
        let dir = self.probe.share_dir();
        if !dir.is_dir() {
            return CheckResult::new(id, CheckStatus::Fail, format!("no share folder here: {}", dir.display()));
        }
        let (to_guest, from_guest) = (token(), token());
        let (name_in, name_out) = (format!(".{to_guest}.txt"), format!(".{from_guest}.txt"));
        let file_in = dir.join(&name_in);
        let file_out = dir.join(&name_out);
        if let Err(e) = std::fs::write(&file_in, &to_guest) {
            return CheckResult::new(id, CheckStatus::Fail, format!("cannot write {}: {e}", file_in.display()));
        }
        let there = match self.guest("share-read", Some(json!({"name": name_in}))) {
            Ok(a) => {
                let ok = a.status == CheckStatus::Pass && a.data.get("text").and_then(Value::as_str) == Some(&to_guest);
                if !ok {
                    (self.log)(&format!("    guest share-read: {}", a.detail));
                }
                ok
            }
            Err(why) => {
                // The files are omaflake's own temporary ones
                let _ = std::fs::remove_file(&file_in);
                return self.not_asked(id, why);
            }
        };
        let back = match self.guest("share-write", Some(json!({"name": name_out, "text": from_guest}))) {
            Ok(a) if a.status == CheckStatus::Pass => self
                .until(|| {
                    (std::fs::read_to_string(&file_out).ok().as_deref() == Some(from_guest.as_str())).then_some(())
                })
                .is_some(),
            Ok(a) => {
                (self.log)(&format!("    guest share-write: {}", a.detail));
                false
            }
            Err(_) => false,
        };
        // omaflake's own temporary files
        let _ = std::fs::remove_file(&file_in);
        let _ = std::fs::remove_file(&file_out);
        match (there, back) {
            (true, true) => CheckResult::new(id, CheckStatus::Pass, PASS_BOTH_WAYS),
            (false, true) => CheckResult::new(id, CheckStatus::Fail, "the guest does not see this computer's file"),
            (true, false) => CheckResult::new(id, CheckStatus::Fail, "this computer does not see the guest's file"),
            (false, false) => CheckResult::new(id, CheckStatus::Fail, "neither way"),
        }
    }

    fn network(&self) -> CheckResult {
        let id = "network";
        let a = match self.guest("network", None) {
            Ok(a) => a,
            Err(why) => return self.not_asked(id, why),
        };
        let addresses = a.data.get("addresses").and_then(Value::as_array).map(Vec::len).unwrap_or(0);
        let dns = a.data.get("dns").and_then(Value::as_bool).unwrap_or(false);
        let internet = a.data.get("internet").and_then(Value::as_bool).unwrap_or(false);
        let summary = format!(
            "{addresses} address(es) · DNS {} · internet {}",
            if dns { "yes" } else { "no" },
            if internet { "yes" } else { "no" }
        );
        if self.config.network == NetworkMode::Isolated {
            // No internet is what isolated means
            let status = if addresses > 0 { CheckStatus::Pass } else { CheckStatus::Fail };
            return CheckResult::new(id, status, format!("{summary} (isolated)"));
        }
        let status = if a.status == CheckStatus::Pass || (dns && internet) { a.status } else { CheckStatus::Fail };
        let detail = if a.detail.is_empty() { summary } else { format!("{summary} – {}", a.detail) };
        CheckResult::new(id, if status == CheckStatus::Skipped { CheckStatus::Fail } else { status }, detail)
    }

    /// Sound, microphone, camera: the guest's answer as it is
    fn passed_through(&self, id: &str) -> CheckResult {
        match self.guest(id, None) {
            Ok(a) => {
                let mut detail = a.detail.clone();
                if let Some(level) = a.data.get("level").and_then(Value::as_f64) {
                    detail = format!("level {level:.3}{}{detail}", if detail.is_empty() { "" } else { " – " });
                }
                CheckResult::new(id, a.status, detail)
            }
            Err(why) => self.not_asked(id, why),
        }
    }

    fn midi(&self) -> CheckResult {
        let id = "midi";
        let a = match self.guest("midi", None) {
            Ok(a) => a,
            Err(why) => return self.not_asked(id, why),
        };
        let guest = a.data.get("endpoints").and_then(number);
        let failed: Vec<String> = a
            .data
            .get("failed")
            .and_then(Value::as_array)
            .map(|f| f.iter().filter_map(Value::as_str).map(str::to_string).collect())
            .unwrap_or_default();
        let host = self.probe.relay(&json!({"command": "midi"}));
        (self.log)(&format!("    host midi: {}", host.as_ref().map(Value::to_string).unwrap_or_else(|e| e.clone())));
        let host_count = host.as_ref().ok().and_then(|h| h.get("endpoints")).and_then(number);
        if let Ok(h) = &host
            && let Some(e) = h.get("error").and_then(Value::as_str)
        {
            return CheckResult::new(id, CheckStatus::Fail, format!("this computer's side: {e}"));
        }
        let shown = |n: Option<u64>| n.map(|n| n.to_string()).unwrap_or_else(|| "?".into());
        let mut detail = format!("{} endpoint(s) in the guest, {} here", shown(guest), shown(host_count));
        if !failed.is_empty() {
            detail.push_str(&format!(" – failed: {}", failed.join(", ")));
        }
        // OPEN (no guest here): whether every host's count equals the guest's exactly (the VM's own pair is in both
        // lists, spec §11.9)
        let ok = a.status != CheckStatus::Fail && failed.is_empty() && guest.is_some() && guest == host_count;
        CheckResult::new(id, if ok { CheckStatus::Pass } else { CheckStatus::Fail }, detail)
    }

    fn disk(&self) -> CheckResult {
        let id = "disk";
        let Some(disk) = self.probe.agent_state().and_then(|s| s.status).and_then(|s| s.disk) else {
            return CheckResult::new(id, CheckStatus::Skipped, NO_AGENT);
        };
        let detail = format!("{} of {} free", bytes(disk.free), bytes(disk.total));
        let low = disk.free < 2 * 1024 * 1024 * 1024 || (disk.total > 0 && disk.free * 20 < disk.total);
        CheckResult::new(id, if low { CheckStatus::Fail } else { CheckStatus::Pass }, detail)
    }
}

/// A number or a string of digits (omaf-runner answers strings)
fn number(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str().and_then(|s| s.trim().parse().ok()))
}

/// A token nobody else has on a clipboard: "omaflake-diagnosis-<random>"
fn token() -> String {
    use std::hash::{BuildHasher, Hasher};
    let mut h = std::collections::hash_map::RandomState::new().build_hasher();
    h.write_u128(std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_nanos());
    format!("omaflake-diagnosis-{:016x}", h.finish())
}

fn shorten(s: &str) -> String {
    let t: String = s.chars().take(60).collect();
    if t.len() < s.len() { format!("{t}…") } else { t }
}

/// "12.3 GB" (decimal units, as Finder)
pub fn bytes(n: u64) -> String {
    let n = n as f64;
    if n >= 1e9 {
        format!("{:.1} GB", n / 1e9)
    } else if n >= 1e6 {
        format!("{:.0} MB", n / 1e6)
    } else {
        format!("{:.0} kB", n / 1e3)
    }
}

/// `<data>/logs/diagnostics-<vm>-<time>.log`, a new file per run
pub fn log_file(paths: &Paths, vm: &str) -> PathBuf {
    let stamp = chrono::Local::now().format("%Y%m%d-%H%M%S");
    let base = paths.logs().join(format!("diagnostics-{vm}-{stamp}.log"));
    if !base.exists() {
        return base;
    }
    (2..).map(|i| paths.logs().join(format!("diagnostics-{vm}-{stamp}-{i}.log"))).find(|p| !p.exists()).unwrap_or(base)
}

/// The VM's newest diagnosis log (by name: the time is in it)
pub fn newest_log(paths: &Paths, vm: &str) -> Option<PathBuf> {
    let prefix = format!("diagnostics-{vm}-");
    let mut logs: Vec<PathBuf> = std::fs::read_dir(paths.logs())
        .ok()?
        .flatten()
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
                // "diagnostics-<vm>-<8 digits>-…": not another VM whose name starts with this one's
                n.strip_prefix(&prefix)
                    .is_some_and(|rest| rest.len() > 8 && rest[..8].chars().all(|c| c.is_ascii_digit()))
                    && n.ends_with(".log")
            })
        })
        .collect();
    // "<stamp>.log" before "<stamp>-2.log" (a second run in the same second)
    let key = |p: &PathBuf| {
        let rest = p.file_name().and_then(|n| n.to_str()).and_then(|n| n.strip_prefix(&prefix)).unwrap_or("");
        let rest = rest.trim_end_matches(".log");
        let (stamp, n) = match rest.get(15..).and_then(|s| s.strip_prefix('-')) {
            Some(n) => (&rest[..15], n.parse::<u32>().unwrap_or(1)),
            None => (rest, 1),
        };
        (stamp.to_string(), n)
    };
    logs.sort_by_key(key);
    logs.pop()
}

/// What identifies a VM's current run: its process' pid file (runner.pid, qemu.pid) – its content and time
pub fn run_id(paths: &Paths, c: &InstanceConfig) -> Option<String> {
    let ip = paths.instance(&c.name);
    let file = match c.guest_kind() {
        GuestKind::Macos => ip.runner_pid(),
        GuestKind::Nixos => ip.pid_file(),
    };
    let text = std::fs::read_to_string(&file).ok()?;
    let modified = std::fs::metadata(&file).and_then(|m| m.modified()).ok()?;
    let at = modified.duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis();
    Some(format!("{}@{at}", text.trim()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::HashMap;

    /// A fake clipboard shared by "this computer" and the fake guest
    #[derive(Default)]
    struct Clip(RefCell<String>);

    struct HostSide<'a>(&'a Clip);

    impl HostClipboard for HostSide<'_> {
        fn read(&self) -> Option<String> {
            Some(self.0.0.borrow().clone())
        }
        fn write(&self, text: &str) -> bool {
            *self.0.0.borrow_mut() = text.to_string();
            true
        }
    }

    struct Fake {
        state: Option<AgentState>,
        clip: Clip,
        share: PathBuf,
        /// check → (status, detail, data) the guest answers
        answers: HashMap<&'static str, (&'static str, &'static str, Value)>,
        ssh: Option<Result<(), String>>,
        asked: RefCell<Vec<String>>,
    }

    impl Probe for Fake {
        fn agent_state(&self) -> Option<AgentState> {
            self.state.clone()
        }
        fn relay(&self, request: &Value) -> Result<Value, String> {
            if request["command"] == "midi" {
                return Ok(json!({"endpoints": "4", "agent": "connected"}));
            }
            let check = request["check"].as_str().unwrap();
            self.asked.borrow_mut().push(check.to_string());
            let args = &request["args"];
            // The guest's clipboard and share folder are the same as the host's here
            let (status, detail, data) = if let Some(a) = self.answers.get(check) {
                a.clone()
            } else {
                match check {
                    "clipboard-read" => ("pass", "", json!({"text": self.clip.0.borrow().clone()})),
                    "clipboard-write" => {
                        *self.clip.0.borrow_mut() = args["text"].as_str().unwrap().into();
                        ("pass", "", Value::Null)
                    }
                    "share-read" => {
                        let text = std::fs::read_to_string(self.share.join(args["name"].as_str().unwrap())).unwrap();
                        ("pass", "", json!({"text": text}))
                    }
                    "share-write" => {
                        std::fs::write(self.share.join(args["name"].as_str().unwrap()), args["text"].as_str().unwrap())
                            .unwrap();
                        ("pass", "", Value::Null)
                    }
                    _ => ("skipped", "unknown check", Value::Null),
                }
            };
            Ok(json!({"id": 1, "check": check, "status": status, "detail": detail, "data": data}))
        }
        fn ssh(&self) -> Option<Result<(), String>> {
            self.ssh.clone()
        }
        fn clipboard(&self) -> Result<Box<dyn HostClipboard + '_>, String> {
            Ok(Box::new(HostSide(&self.clip)))
        }
        fn share_dir(&self) -> PathBuf {
            self.share.clone()
        }
    }

    fn ready_state(features: &[&str]) -> AgentState {
        let hello: crate::agent_protocol::AgentHello = serde_json::from_value(json!({
            "agent": "0.4.0", "protocol": {"min": 1, "max": 1}, "os": "linux", "osName": "NixOS",
            "features": features,
        }))
        .unwrap();
        AgentState {
            hello: Some(hello),
            hello_at: Some(crate::json::now()),
            protocol_version: Some(1),
            status: Some(
                serde_json::from_value(
                    json!({"disk": {"path": "/", "free": 20_000_000_000u64, "total": 64_000_000_000u64}}),
                )
                .unwrap(),
            ),
            status_at: Some(crate::json::now()),
            features: Some(features.iter().map(|s| s.to_string()).collect()),
            ..Default::default()
        }
    }

    fn fake(dir: &Path) -> Fake {
        let answers = HashMap::from([
            ("info", ("pass", "", json!({"os": "linux", "session": true, "helpers": {"clipboard": true}}))),
            ("network", ("pass", "", json!({"addresses": ["10.0.2.15"], "dns": true, "internet": true}))),
            ("sound", ("confirm", "played 440 Hz on the default output", Value::Null)),
            ("midi", ("pass", "", json!({"endpoints": 4, "failed": []}))),
        ]);
        Fake {
            state: Some(ready_state(&["status", "diagnose", "clipboard"])),
            clip: Clip(RefCell::new("before".into())),
            share: dir.to_path_buf(),
            answers,
            ssh: Some(Ok(())),
            asked: RefCell::default(),
        }
    }

    fn run(c: &InstanceConfig, probe: &dyn Probe) -> (Vec<CheckResult>, Vec<String>, usize) {
        let lines = RefCell::new(Vec::new());
        let seen = RefCell::new(0usize);
        let log = |l: &str| lines.borrow_mut().push(l.to_string());
        let on = |_: &CheckResult| *seen.borrow_mut() += 1;
        let mut d = Diagnosis::new(c, probe, &log, &on);
        d.wait = Duration::from_millis(200);
        d.poll = Duration::from_millis(10);
        let r = d.run();
        (r, lines.into_inner(), seen.into_inner())
    }

    fn status(r: &[CheckResult], id: &str) -> (CheckStatus, String) {
        let x = r.iter().find(|x| x.id == id).unwrap();
        (x.status, x.detail.clone())
    }

    #[test]
    fn every_check_in_order_with_switches() {
        let dir = tempfile::tempdir().unwrap();
        let f = fake(dir.path());
        let mut c = crate::models::tests::config("t1");
        c.clipboard = Some(true);
        c.share = true;
        c.audio = Some(true);
        c.midi = Some(true);
        c.microphone = Some(false);
        c.camera = Some(false);
        let (r, lines, seen) = run(&c, &f);
        assert_eq!(r.iter().map(|x| x.id.as_str()).collect::<Vec<_>>(), CHECKS);
        assert_eq!(seen, 2 * CHECKS.len(), "running, then done");
        assert_eq!(status(&r, "agent").0, CheckStatus::Pass);
        assert_eq!(status(&r, "ssh").0, CheckStatus::Pass);
        assert_eq!(status(&r, "clipboard"), (CheckStatus::Pass, PASS_BOTH_WAYS.into()));
        assert_eq!(*f.clip.0.borrow(), "before", "this computer's clipboard is restored");
        assert_eq!(status(&r, "share"), (CheckStatus::Pass, PASS_BOTH_WAYS.into()));
        assert_eq!(std::fs::read_dir(dir.path()).unwrap().count(), 0, "the share files are removed");
        assert_eq!(status(&r, "network").0, CheckStatus::Pass);
        assert_eq!(status(&r, "sound").0, CheckStatus::Confirm);
        assert_eq!(status(&r, "microphone"), (CheckStatus::Skipped, SKIP_MICROPHONE.into()));
        assert_eq!(status(&r, "camera"), (CheckStatus::Skipped, SKIP_CAMERA.into()));
        assert_eq!(status(&r, "midi").0, CheckStatus::Pass);
        assert_eq!(status(&r, "disk"), (CheckStatus::Pass, "20.0 GB of 64.0 GB free".into()));
        assert!(lines.iter().any(|l| l == "✓ Clipboard: both ways"), "{lines:?}");
        assert!(lines.iter().any(|l| l == "– Microphone: The microphone is off for this VM"));
        assert!(lines.iter().any(|l| l == "? Sound: played 440 Hz on the default output"));
        assert_eq!(lines.last().unwrap(), "Done – nothing failed");
    }

    #[test]
    fn without_the_agent_guest_checks_are_skipped_and_ssh_says_why() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = fake(dir.path());
        f.state = None;
        f.ssh = Some(Err("debug1: …\nuser@10.0.0.2: Permission denied (publickey).".into()));
        let mut c = crate::vz_backend::tests::mac_config("m1", NetworkMode::Shared);
        c.clipboard = Some(true);
        let (r, lines, _) = run(&c, &f);
        assert_eq!(status(&r, "agent").0, CheckStatus::Fail);
        assert_eq!(status(&r, "ssh"), (CheckStatus::Fail, "user@10.0.0.2: Permission denied (publickey).".into()));
        assert_eq!(status(&r, "clipboard"), (CheckStatus::Skipped, NO_AGENT.into()));
        assert_eq!(status(&r, "disk"), (CheckStatus::Skipped, NO_AGENT.into()));
        assert!(f.asked.borrow().is_empty(), "the guest was not asked");
        assert!(lines.last().unwrap().contains("failed: Agent, SSH"));
    }

    #[test]
    fn an_agent_without_the_diagnosis() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = fake(dir.path());
        f.state = Some(ready_state(&["status"]));
        let mut c = crate::models::tests::config("t1");
        c.audio = Some(true);
        let (r, _, _) = run(&c, &f);
        assert_eq!(status(&r, "agent").0, CheckStatus::Pass);
        assert_eq!(status(&r, "sound"), (CheckStatus::Skipped, NO_DIAGNOSIS.into()));
    }

    #[test]
    fn a_clipboard_one_way_fails() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = fake(dir.path());
        f.answers.insert("clipboard-read", ("pass", "", json!({"text": "stale"})));
        let mut c = crate::models::tests::config("t1");
        c.clipboard = Some(true);
        let (r, _, _) = run(&c, &f);
        let (s, d) = status(&r, "clipboard");
        assert_eq!(s, CheckStatus::Fail);
        assert!(d.starts_with("only guest → this computer; the guest's clipboard held \"stale\""), "{d}");
        assert_eq!(*f.clip.0.borrow(), "before", "restored also when it failed");
    }

    #[test]
    fn log_files_and_run_ids() {
        let dir = tempfile::tempdir().unwrap();
        let paths = Paths::new(dir.path(), None);
        std::fs::create_dir_all(paths.logs()).unwrap();
        let a = log_file(&paths, "dev");
        std::fs::write(&a, "x").unwrap();
        let b = log_file(&paths, "dev");
        assert_ne!(a, b, "unique per run");
        std::fs::write(&b, "y").unwrap();
        std::fs::write(paths.logs().join("diagnostics-dev-2-20990101-000000.log"), "other VM").unwrap();
        assert_eq!(newest_log(&paths, "dev"), Some(b));
        let c = crate::models::tests::config("dev");
        assert_eq!(run_id(&paths, &c), None);
        let ip = paths.instance("dev");
        std::fs::create_dir_all(&ip.run).unwrap();
        std::fs::write(ip.pid_file(), "123\n").unwrap();
        assert!(run_id(&paths, &c).unwrap().starts_with("123@"));
    }

    #[test]
    fn control_requests_are_one_line_each_way() {
        use std::io::{BufRead, Write};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("runner.sock");
        let listener = std::os::unix::net::UnixListener::bind(&path).unwrap();
        let server = std::thread::spawn(move || {
            let (s, _) = listener.accept().unwrap();
            let mut line = String::new();
            std::io::BufReader::new(&s).read_line(&mut line).unwrap();
            (&s).write_all(b"{\"id\":3,\"status\":\"pass\",\"data\":{\"text\":\"x\"}}\n").unwrap();
            line
        });
        let a =
            control_request(&path, &json!({"command": "diagnose", "check": "info"}), Duration::from_secs(2)).unwrap();
        assert_eq!(a["data"]["text"], "x");
        let sent = server.join().unwrap();
        assert!(sent.ends_with('\n'));
        assert_eq!(serde_json::from_str::<Value>(&sent).unwrap(), json!({"command": "diagnose", "check": "info"}));
    }
}
