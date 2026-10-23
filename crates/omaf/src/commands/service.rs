// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf serve --socket` and `omaf service …` (plan B120, docs/api.md §1, docs/cli.md): the core service as a user
//! agent (macOS: launchd, `~/Library/LaunchAgents/app.omaflake.serve.plist`; Linux: systemd, `~/.config/systemd/user/
//! omaflake.service`), and how a client finds it – started on demand when it does not run.
//!
//! On demand: the installed agent is started (`launchctl kickstart`, `systemctl --user start`); without one – or with
//! `VM_HOME` set, which the agent does not serve – `omaf serve --socket --idle-exit 300` is started on its own (its own
//! session, setsid), and it ends five minutes after its last client and job. Updating: a client newer than the
//! service asks it to restart (`service.restart`, it ends once no job runs) and starts the new one, when that start
//! would run this omaf (no agent, or an agent of this omaf).

use std::io::Write;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use omaflake_core::api::client::Client;
use omaflake_core::api::service::{self, is_newer, socket_path, Options};
use omaflake_core::api::types::JobDto;
use omaflake_core::platform::home_dir;
use omaflake_core::{Error, Paths, Result};
use serde_json::{json, Value};

use crate::cli::{Serve, ServiceCommand};
use crate::tty::Interrupts;
use crate::util::Outcome;

/// launchd's label of the agent
pub const LABEL: &str = "app.omaflake.serve";
/// systemd's name of the unit
pub const UNIT: &str = "omaflake.service";
/// A service started on demand ends this long after its last client and job
pub const IDLE_EXIT: u64 = 300;
/// How long a client waits for a service it started
const START_WAIT: Duration = Duration::from_secs(10);
/// How long a restart (`service.restart` without jobs) may take
const STOP_WAIT: Duration = Duration::from_secs(10);

/// `omaf serve [--socket[=<path>]] [--idle-exit <secs>]`
pub fn serve(a: &Serve) -> Outcome {
    let Some(socket) = &a.socket else {
        return Ok(omaflake_core::api::serve_stdio()?);
    };
    let options = Options { socket: socket.clone(), idle_exit: a.idle_exit.map(Duration::from_secs) };
    match service::serve_socket(options) {
        Ok(_) => Ok(()),
        // Another service has the socket: nothing to do (exit 0 – launchd/systemd must not try again and again)
        Err(e) if service::already_running(&e) => {
            eprintln!("omaf serve: {e}");
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

pub fn run(command: &ServiceCommand) -> Outcome {
    let paths = Paths::standard();
    match command {
        ServiceCommand::Install => install(&paths),
        ServiceCommand::Uninstall => uninstall(),
        ServiceCommand::Status { json } => status(&paths, *json),
        ServiceCommand::Start { json } => {
            let running = ensure_running(&paths)?;
            if *json {
                println!("{}", running.json());
            } else if running.started {
                println!("Service started on {} (omaf {})", running.socket.display(), running.version);
            } else {
                println!("Service runs on {} (omaf {})", running.socket.display(), running.version);
            }
            Ok(())
        }
        ServiceCommand::Stop => stop(&paths),
    }
}

// MARK: Finding and starting the service

/// A service that answers
pub struct Running {
    pub client: Client,
    pub socket: PathBuf,
    pub version: String,
    /// This call started it
    pub started: bool,
}

impl Running {
    fn json(&self) -> Value {
        json!({"socket": self.socket, "version": self.version, "started": self.started})
    }
}

/// Connects and initializes when the service runs; None when it does not
pub fn connect(socket: &Path) -> Option<(Client, String)> {
    let client = Client::connect(socket).ok()?;
    let init = client.initialize("omaf").ok()?;
    Some((client, init.omaf_version))
}

/// The service of `paths`, started when it does not run, restarted once when it is older than this omaf (and the
/// restart would bring this omaf)
pub fn ensure_running(paths: &Paths) -> Result<Running> {
    let socket = socket_path(paths);
    if let Some((client, version)) = connect(&socket) {
        if !(is_newer(omaflake_core::VERSION, &version) && restart_brings_this_omaf()) {
            return Ok(Running { client, socket, version, started: false });
        }
        let jobs = client.call("service.restart", Value::Null).ok().and_then(|v| v["jobs"].as_u64()).unwrap_or(1);
        if jobs > 0 {
            // It ends after its jobs; until then it is the one to use
            eprintln!(
                "omaf: the service (omaf {version}) restarts with omaf {} once its jobs are done",
                omaflake_core::VERSION
            );
            return Ok(Running { client, socket, version, started: false });
        }
        drop(client);
        let end = Instant::now() + STOP_WAIT;
        while service::is_running(&socket) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    start(paths)?;
    let end = Instant::now() + START_WAIT;
    loop {
        if let Some((client, version)) = connect(&socket) {
            return Ok(Running { client, socket, version, started: true });
        }
        if Instant::now() >= end {
            return Err(Error::new(format!(
                "The service did not start on {} – see {}",
                socket.display(),
                log_hint(paths).display()
            )));
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// The installed agent serves the standard data folder – not one of `VM_HOME`
fn agent_usable() -> bool {
    std::env::var_os("VM_HOME").is_none_or(|v| v.is_empty()) && agent_file().exists()
}

/// A restart starts this omaf again: no agent (this omaf starts it), or the agent runs this omaf
fn restart_brings_this_omaf() -> bool {
    if !agent_usable() {
        return true;
    }
    let installed = std::fs::read_to_string(agent_file()).ok().and_then(|text| installed_program(&text));
    installed.is_some_and(|p| Some(p) == this_omaf().ok())
}

/// Starts the service: the agent if installed (and usable), else `omaf serve --socket --idle-exit 300` on its own
fn start(paths: &Paths) -> Result<()> {
    if agent_usable() {
        let ok = if cfg!(target_os = "macos") {
            quiet("launchctl", &["kickstart", &format!("gui/{}/{LABEL}", uid())])
        } else {
            quiet("systemctl", &["--user", "start", UNIT])
        };
        if ok {
            return Ok(());
        }
        eprintln!("omaf: the installed service did not start – starting one on its own");
    }
    let log = paths.logs().join("service.log");
    let _ = std::fs::create_dir_all(paths.logs());
    let err = std::fs::OpenOptions::new().create(true).append(true).open(&log)?;
    let mut cmd = Command::new(this_omaf()?);
    cmd.args(["serve", "--socket", "--idle-exit", &IDLE_EXIT.to_string()])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(err);
    // SAFETY: setsid is async-signal-safe; the child leaves this terminal's session (Ctrl-C here does not reach it)
    unsafe {
        cmd.pre_exec(|| {
            libc::setsid();
            Ok(())
        });
    }
    cmd.spawn().map_err(|e| Error::from(e).context("omaf serve --socket: "))?;
    Ok(())
}

fn log_hint(paths: &Paths) -> PathBuf {
    if agent_usable() && cfg!(target_os = "macos") {
        launchd_log()
    } else {
        paths.logs().join("service.log")
    }
}

/// This omaf (the agent and a service started on demand run it)
fn this_omaf() -> Result<PathBuf> {
    let exe = std::env::current_exe()?;
    Ok(exe.canonicalize().unwrap_or(exe))
}

fn uid() -> u32 {
    // SAFETY: getuid cannot fail
    unsafe { libc::getuid() }
}

/// Runs a tool, its output thrown away; true when it succeeded
fn quiet(tool: &str, args: &[&str]) -> bool {
    Command::new(tool)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|s| s.success())
}

/// Runs a tool; its error output is the error
fn checked(tool: &str, args: &[&str]) -> Result<()> {
    let out = Command::new(tool).args(args).stdin(Stdio::null()).output()?;
    if out.status.success() {
        return Ok(());
    }
    Err(Error::new(format!("{tool} {}: {}", args.join(" "), String::from_utf8_lossy(&out.stderr).trim())))
}

/// Follows a job of the service until it ended: its log lines go to `on_lines`; Ctrl-C cancels it (`jobs.cancel`)
/// and waits for its end
pub fn follow(client: &Client, job: &Value, on_lines: &mut dyn FnMut(&[String])) -> Result<JobDto> {
    let id = job["id"].as_str().ok_or_else(|| Error::new("The service answered without a job"))?.to_string();
    let interrupted = Arc::new(AtomicBool::new(false));
    let interrupts = {
        let flag = interrupted.clone();
        // The job's log says "Cancelling …" itself
        Interrupts::handle(move || flag.store(true, Ordering::SeqCst))?
    };
    let flag = interrupted.clone();
    let ended = client.follow_job(&id, 0, on_lines, &move || flag.load(Ordering::SeqCst));
    drop(interrupts);
    Ok(ended?)
}

/// Is this answer a job (the methods of api.md §5 answer with one)?
pub fn is_job(v: &Value) -> bool {
    v["id"].is_string() && v["kind"].is_string() && v["state"].is_string() && v.get("logLines").is_some()
}

// MARK: The agent

/// The agent's file: the launchd plist (macOS) or the systemd unit (Linux)
pub fn agent_file() -> PathBuf {
    if cfg!(target_os = "macos") {
        home_dir().join("Library/LaunchAgents").join(format!("{LABEL}.plist"))
    } else {
        let config = std::env::var_os("XDG_CONFIG_HOME")
            .filter(|c| !c.is_empty())
            .map(PathBuf::from)
            .unwrap_or_else(|| home_dir().join(".config"));
        config.join("systemd/user").join(UNIT)
    }
}

/// Where launchd writes the agent's diagnostics (outside the data folder, which may move)
fn launchd_log() -> PathBuf {
    home_dir().join("Library/Logs/omaflake/service.log")
}

fn xml(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// The launchd agent: started at login, again after a crash (not after a clean end – `service.restart`, `omaf
/// service stop`; then the next client starts it)
pub fn plist(omaf: &Path, log: &Path) -> String {
    let omaf = xml(&omaf.to_string_lossy());
    let log = xml(&log.to_string_lossy());
    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
	<key>Label</key>
	<string>{LABEL}</string>
	<key>ProgramArguments</key>
	<array>
		<string>{omaf}</string>
		<string>serve</string>
		<string>--socket</string>
	</array>
	<key>RunAtLoad</key>
	<true/>
	<key>KeepAlive</key>
	<dict>
		<key>SuccessfulExit</key>
		<false/>
	</dict>
	<key>ProcessType</key>
	<string>Standard</string>
	<key>StandardErrorPath</key>
	<string>{log}</string>
</dict>
</plist>
"#
    )
}

/// systemd's quoting of one word of ExecStart (`%` and `$` are specifiers there)
fn systemd_word(s: &str) -> String {
    let s = s.replace('\\', "\\\\").replace('"', "\\\"").replace('%', "%%").replace('$', "$$");
    format!("\"{s}\"")
}

/// The systemd user unit: started at login (default.target), again after a crash
pub fn unit(omaf: &Path) -> String {
    format!(
        "# Written by omaf service install – the omaflake core service (docs/api.md)\n\
         [Unit]\n\
         Description=omaflake core service (omaf serve --socket)\n\
         \n\
         [Service]\n\
         Type=simple\n\
         ExecStart={} serve --socket\n\
         Restart=on-failure\n\
         \n\
         [Install]\n\
         WantedBy=default.target\n",
        systemd_word(&omaf.to_string_lossy())
    )
}

/// The omaf an agent file runs (its first program argument)
fn installed_program(text: &str) -> Option<PathBuf> {
    if let Some(rest) = text.split("<key>ProgramArguments</key>").nth(1) {
        let start = rest.find("<string>")? + "<string>".len();
        let end = rest[start..].find("</string>")? + start;
        let raw = &rest[start..end];
        let path = raw.replace("&quot;", "\"").replace("&lt;", "<").replace("&gt;", ">").replace("&amp;", "&");
        return Some(PathBuf::from(path));
    }
    let line = text.lines().find_map(|l| l.strip_prefix("ExecStart="))?;
    let word = line.strip_prefix('"')?;
    let mut out = String::new();
    let mut chars = word.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => out.push(chars.next()?),
            '"' => break,
            _ => out.push(c),
        }
    }
    Some(PathBuf::from(out.replace("%%", "%").replace("$$", "$")))
}

fn install(paths: &Paths) -> Outcome {
    if std::env::var_os("VM_HOME").is_some_and(|v| !v.is_empty()) {
        return Err(
            Error::new("VM_HOME is set – the user agent serves the standard data folder; unset it to install").into()
        );
    }
    let omaf = this_omaf()?;
    let file = agent_file();
    if let Some(dir) = file.parent() {
        std::fs::create_dir_all(dir).map_err(Error::from)?;
    }
    // A service started on demand holds the socket: it hands over once its jobs are done
    let socket = socket_path(paths);
    if let Some((client, _)) = connect(&socket) {
        let jobs = client.call("service.restart", Value::Null).ok().and_then(|v| v["jobs"].as_u64()).unwrap_or(0);
        if jobs > 0 {
            println!(
                "The running service ends after its {jobs} job(s); the agent takes over then (at the next start)."
            );
        }
        drop(client);
        let end = Instant::now() + STOP_WAIT;
        while jobs == 0 && service::is_running(&socket) && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    if cfg!(target_os = "macos") {
        std::fs::create_dir_all(launchd_log().parent().expect("a folder")).map_err(Error::from)?;
        write_file(&file, &plist(&omaf, &launchd_log()))?;
        let domain = format!("gui/{}", uid());
        // Loaded already (an older install): out first
        quiet("launchctl", &["bootout", &format!("{domain}/{LABEL}")]);
        checked("launchctl", &["bootstrap", &domain, &file.to_string_lossy()])?;
    } else {
        write_file(&file, &unit(&omaf))?;
        checked("systemctl", &["--user", "daemon-reload"])?;
        checked("systemctl", &["--user", "enable", "--now", UNIT])?;
    }
    println!("Installed {} – the service starts at login and runs {}", file.display(), omaf.display());
    Ok(())
}

fn write_file(file: &Path, text: &str) -> Result<()> {
    let mut f = std::fs::File::create(file).map_err(|e| Error::from(e).context(format!("{}: ", file.display())))?;
    f.write_all(text.as_bytes())?;
    Ok(())
}

fn uninstall() -> Outcome {
    let file = agent_file();
    if !file.exists() {
        println!("Not installed ({} does not exist)", file.display());
        return Ok(());
    }
    if cfg!(target_os = "macos") {
        // Stops it too; jobs still running are cancelled
        quiet("launchctl", &["bootout", &format!("gui/{}/{LABEL}", uid())]);
    } else {
        quiet("systemctl", &["--user", "disable", "--now", UNIT]);
    }
    std::fs::remove_file(&file).map_err(|e| Error::from(e).context(format!("{}: ", file.display())))?;
    if !cfg!(target_os = "macos") {
        quiet("systemctl", &["--user", "daemon-reload"]);
    }
    println!("Removed {} – clients start the service on their own when they need it", file.display());
    Ok(())
}

fn status(paths: &Paths, as_json: bool) -> Outcome {
    let socket = socket_path(paths);
    let file = agent_file();
    let installed = file.exists();
    let running = connect(&socket);
    let jobs = running.as_ref().and_then(|(c, _)| c.call("jobs.list", Value::Null).ok()).map(|list| {
        list.as_array()
            .map(|a| {
                a.iter().filter(|j| matches!(j["state"].as_str(), Some("queued" | "checking" | "running"))).count()
            })
            .unwrap_or(0)
    });
    if as_json {
        let v = json!({
            "installed": installed,
            "agentFile": file,
            "socket": socket,
            "running": running.is_some(),
            "version": running.as_ref().map(|(_, v)| v.clone()),
            "jobs": jobs,
        });
        println!("{}", super::pretty(&v));
        return Ok(());
    }
    println!("Agent:   {}", if installed { format!("installed ({})", file.display()) } else { "not installed".into() });
    println!("Socket:  {}", socket.display());
    match running {
        Some((_, version)) => {
            println!("Running: yes – omaf {version}, {} job(s) waiting or running", jobs.unwrap_or(0))
        }
        None => println!("Running: no"),
    }
    Ok(())
}

fn stop(paths: &Paths) -> Outcome {
    let socket = socket_path(paths);
    let Some((client, _)) = connect(&socket) else {
        println!("The service does not run");
        return Ok(());
    };
    let jobs = client.call("service.restart", Value::Null).map_err(Error::from)?["jobs"].as_u64().unwrap_or(0);
    drop(client);
    if jobs > 0 {
        println!("The service ends once its {jobs} job(s) are done");
        return Ok(());
    }
    let end = Instant::now() + STOP_WAIT;
    while service::is_running(&socket) {
        if Instant::now() >= end {
            return Err(Error::new("The service did not end").into());
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    println!("Stopped");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_plist_runs_omaf_serve_on_the_socket() {
        let text = plist(Path::new("/Applications/omaflake & co.app/Contents/MacOS/omaf"), Path::new("/l/service.log"));
        assert!(text.contains("<string>app.omaflake.serve</string>"));
        assert!(text.contains("<string>/Applications/omaflake &amp; co.app/Contents/MacOS/omaf</string>"));
        assert!(text.contains("<string>serve</string>\n\t\t<string>--socket</string>"));
        assert!(text.contains("<key>SuccessfulExit</key>\n\t\t<false/>"));
        assert_eq!(
            installed_program(&text),
            Some(PathBuf::from("/Applications/omaflake & co.app/Contents/MacOS/omaf"))
        );
        // The system's own check, where there is one
        if Path::new("/usr/bin/plutil").exists() {
            let dir = tempfile::tempdir().unwrap();
            let file = dir.path().join("app.omaflake.serve.plist");
            std::fs::write(&file, &text).unwrap();
            let out = Command::new("/usr/bin/plutil").arg("-lint").arg(&file).output().unwrap();
            assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stdout));
        }
    }

    #[test]
    fn the_unit_runs_omaf_serve_on_the_socket() {
        let text = unit(Path::new("/opt/my omaf/100%/omaf"));
        assert!(text.contains("ExecStart=\"/opt/my omaf/100%%/omaf\" serve --socket\n"));
        assert!(text.contains("Restart=on-failure") && text.contains("WantedBy=default.target"));
        assert_eq!(installed_program(&text), Some(PathBuf::from("/opt/my omaf/100%/omaf")));
        assert_eq!(installed_program("nothing"), None);
    }

    #[test]
    fn the_agent_file_has_its_place() {
        let file = agent_file();
        if cfg!(target_os = "macos") {
            assert!(file.ends_with("Library/LaunchAgents/app.omaflake.serve.plist"));
        } else {
            assert!(file.ends_with("systemd/user/omaflake.service"));
        }
    }
}
