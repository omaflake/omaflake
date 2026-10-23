// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Shell.swift: runs external tools (nix, qemu-img, git) synchronously.
//!
//! Differences to Swift (on purpose): stdout and stderr are read at the same time (Swift read stdout first and could
//! block on a full stderr pipe), and `input` is written from its own thread. A cancelled token (`RunOptions::cancel`)
//! terminates the running process – Swift found the token as a task-local value.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::process::ExitStatusExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};

use crate::cancellation::Cancellation;
use crate::error::{os_text, Error, Result};
use crate::paths::{Paths, Settings};
use crate::platform::{expand_tilde, home_dir, is_executable_file, user_name};
use crate::spawn::Pid;
use crate::vm_err;

/// What a run gave (Swift `Shell.Result`). `status` is the exit code, or the signal number when a signal ended the
/// process (Swift's `terminationStatus`: 15 after SIGTERM – `ImageStore::build` takes that as "cancelled").
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ShellResult {
    pub status: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Options of `Shell::run` and `Shell::check` (Swift's defaulted parameters)
#[derive(Default, Clone, Copy)]
pub struct RunOptions<'a> {
    pub cwd: Option<&'a Path>,
    /// Shows stderr live (e.g. the nix build log) instead of collecting it
    pub passthrough_stderr: bool,
    /// Goes to stdin (e.g. a password, never as an argument)
    pub input: Option<&'a [u8]>,
    /// Added to this process's environment
    pub environment: &'a [(&'a str, &'a str)],
    /// Checked before the start; cancelling terminates the process
    pub cancel: Option<&'a Cancellation>,
}

/// Options of `Shell::stream`
#[derive(Default, Clone, Copy)]
pub struct StreamOptions<'a> {
    pub cwd: Option<&'a Path>,
    pub environment: &'a [(&'a str, &'a str)],
    pub input: Option<&'a [u8]>,
    /// Sends stdout to `on_line` too (programs that report progress on stdout, like omaf-runner)
    pub merge_stdout: bool,
    /// Receives the process so the caller can stop it (Swift handed over the `Process`; here its PID – send SIGTERM)
    pub started: Option<&'a dyn Fn(Pid)>,
    pub cancel: Option<&'a Cancellation>,
}

/// Swift `enum Shell` (namespace)
pub enum Shell {}

impl Shell {
    /// Runs `executable` and collects output. `passthrough_stderr` shows stderr live (e.g. the nix build log).
    /// `input` goes to stdin (e.g. a password, never as an argument); `environment` is added to this process's.
    pub fn run<S: AsRef<str>>(executable: &str, args: &[S], opts: &RunOptions) -> Result<ShellResult> {
        let exe = Self::which(executable)?;
        if let Some(c) = opts.cancel {
            c.check()?;
        }
        let mut cmd = Command::new(&exe);
        cmd.args(args.iter().map(|a| a.as_ref()));
        if let Some(cwd) = opts.cwd {
            cmd.current_dir(cwd);
        }
        cmd.envs(opts.environment.iter().copied());
        if opts.input.is_some() {
            cmd.stdin(Stdio::piped());
        }
        cmd.stdout(Stdio::piped());
        cmd.stderr(if opts.passthrough_stderr { Stdio::inherit() } else { Stdio::piped() });
        let mut child = cmd.spawn().map_err(|e| spawn_error(&exe, &e))?;
        let pid = child.id() as Pid;
        if let Some(c) = opts.cancel {
            c.track(pid);
        }
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (out, err) = std::thread::scope(|s| {
            if let (Some(mut w), Some(input)) = (stdin, opts.input) {
                s.spawn(move || {
                    let _ = w.write_all(input);
                    // dropping `w` closes stdin
                });
            }
            let err = stderr.map(|mut e| {
                s.spawn(move || {
                    let mut buf = Vec::new();
                    let _ = e.read_to_end(&mut buf);
                    buf
                })
            });
            let mut out = Vec::new();
            if let Some(mut o) = stdout {
                let _ = o.read_to_end(&mut out);
            }
            (out, err.map(|h| h.join().unwrap_or_default()).unwrap_or_default())
        });
        let status = wait_untracked(&mut child, opts.cancel)?;
        Ok(ShellResult {
            status: status_code(status),
            stdout: String::from_utf8_lossy(&out).into_owned(),
            stderr: String::from_utf8_lossy(&err).into_owned(),
        })
    }

    /// Like `run`, but stderr goes line by line to `on_line` (live log, e.g. nix build -L). `started` receives the
    /// process ID so the caller can cancel; `merge_stdout` sends stdout to `on_line` too (programs that report
    /// progress on stdout, like omaf-runner); `input` goes to stdin. The result's `stderr` is empty (it went to
    /// `on_line`), as in Swift.
    pub fn stream<S: AsRef<str>>(
        executable: &str,
        args: &[S],
        opts: &StreamOptions,
        on_line: &(dyn Fn(&str) + Send + Sync),
    ) -> Result<ShellResult> {
        let exe = Self::which(executable)?;
        let (reader, writer) = std::io::pipe().map_err(Error::from)?;
        let mut child = {
            let mut cmd = Command::new(&exe);
            cmd.args(args.iter().map(|a| a.as_ref()));
            if let Some(cwd) = opts.cwd {
                cmd.current_dir(cwd);
            }
            cmd.envs(opts.environment.iter().copied());
            if opts.input.is_some() {
                cmd.stdin(Stdio::piped());
            }
            if opts.merge_stdout {
                cmd.stdout(Stdio::from(writer.try_clone().map_err(Error::from)?));
            } else {
                cmd.stdout(Stdio::piped());
            }
            cmd.stderr(Stdio::from(writer));
            if let Some(c) = opts.cancel {
                c.check()?;
            }
            cmd.spawn().map_err(|e| spawn_error(&exe, &e))?
            // `cmd` is dropped here, closing this process's write ends: the reader sees EOF when the child is done
        };
        let pid = child.id() as Pid;
        if let Some(c) = opts.cancel {
            c.track(pid);
        }
        if let Some(started) = opts.started {
            started(pid);
        }
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let out = std::thread::scope(|s| {
            if let (Some(mut w), Some(input)) = (stdin, opts.input) {
                s.spawn(move || {
                    let _ = w.write_all(input);
                });
            }
            let lines = s.spawn(move || split_lines(reader, on_line));
            let mut out = Vec::new();
            if let Some(mut o) = stdout {
                let _ = o.read_to_end(&mut out);
            }
            let _ = lines.join();
            out
        });
        let status = wait_untracked(&mut child, opts.cancel)?;
        Ok(ShellResult {
            status: status_code(status),
            stdout: String::from_utf8_lossy(&out).into_owned(),
            stderr: String::new(),
        })
    }

    /// Like `run`, but errs on exit code ≠ 0: "<executable> <args> failed (<status>):\n<stderr>"
    pub fn check<S: AsRef<str>>(executable: &str, args: &[S], opts: &RunOptions) -> Result<String> {
        let r = Self::run(executable, args, opts)?;
        if r.status != 0 {
            let msg = r.stderr.trim();
            let joined = args.iter().map(|a| a.as_ref()).collect::<Vec<_>>().join(" ");
            let detail = if msg.is_empty() { String::new() } else { format!(":\n{msg}") };
            return Err(vm_err!("{executable} {joined} failed ({}){detail}", r.status));
        }
        Ok(r.stdout)
    }

    /// Finds the absolute path of a program: `nix` from `Settings.nix` if set, otherwise PATH, then common locations.
    pub fn which(name: &str) -> Result<String> {
        if name.starts_with('/') {
            return Ok(name.to_string());
        }
        if name == "nix" {
            if let Some(custom) = Settings::load(&Paths::standard()).nix {
                return Ok(expand_tilde(&custom));
            }
        }
        for d in Self::search_directories() {
            let path = PathBuf::from(&d).join(name);
            if is_executable_file(&path) {
                return Ok(path.to_string_lossy().into_owned());
            }
        }
        Err(vm_err!("Program not found: {name}"))
    }

    /// PATH, then where Nix, nix-darwin and Homebrew put programs (an app started from Finder has a minimal PATH).
    pub fn search_directories() -> Vec<String> {
        let home = home_dir().to_string_lossy().into_owned();
        let mut dirs: Vec<String> = std::env::var("PATH")
            .unwrap_or_default()
            .split(':')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        dirs.extend([
            "/run/current-system/sw/bin".to_string(),
            format!("/etc/profiles/per-user/{}/bin", user_name()),
            format!("{home}/.nix-profile/bin"),
            "/nix/var/nix/profiles/default/bin".to_string(),
            "/opt/homebrew/bin".to_string(),
            "/usr/bin".to_string(),
            "/bin".to_string(),
        ]);
        let mut seen = std::collections::HashSet::new();
        dirs.retain(|d| seen.insert(d.clone()));
        dirs
    }
}

fn spawn_error(exe: &str, e: &std::io::Error) -> Error {
    vm_err!("Cannot run {exe}: {}", os_text(e))
}

/// Exit code, or the signal number (Swift `terminationStatus`)
fn status_code(status: ExitStatus) -> i32 {
    status.code().or_else(|| status.signal()).unwrap_or(-1)
}

/// Waits for the child without reaping it (`WNOWAIT`), untracks it from the token, then reaps it – a PID is never
/// signalled by `cancel` after it could have been reused
fn wait_untracked(child: &mut Child, cancel: Option<&Cancellation>) -> Result<ExitStatus> {
    let pid = child.id() as Pid;
    if let Some(c) = cancel {
        loop {
            // SAFETY: zeroed siginfo_t is a valid out-parameter; waitid only inspects our own child
            let rc = unsafe {
                let mut info: libc::siginfo_t = std::mem::zeroed();
                libc::waitid(libc::P_PID, pid as libc::id_t, &mut info, libc::WEXITED | libc::WNOWAIT)
            };
            if rc == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::EINTR) {
                break;
            }
        }
        c.untrack(pid);
    }
    child.wait().map_err(Error::from)
}

/// Splits incoming bytes into lines (`\n`, decoded leniently as UTF-8) and hands each to `on_line`; a last line
/// without `\n` is handed over too (Swift `LineSplitter`)
pub(crate) fn split_lines(reader: impl Read, on_line: &(dyn Fn(&str) + Send + Sync)) {
    let mut r = BufReader::new(reader);
    let mut buf = Vec::new();
    loop {
        buf.clear();
        match r.read_until(b'\n', &mut buf) {
            Ok(0) => break,
            Ok(_) => {
                if buf.last() == Some(&b'\n') {
                    buf.pop();
                }
                on_line(&String::from_utf8_lossy(&buf));
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;
    use std::time::{Duration, Instant};

    #[test]
    fn runs_and_collects() {
        let r = Shell::run("/bin/sh", &["-c", "echo out; echo err >&2; exit 3"], &RunOptions::default()).unwrap();
        assert_eq!((r.status, r.stdout.as_str(), r.stderr.as_str()), (3, "out\n", "err\n"));
        let r = Shell::run("/bin/cat", &[] as &[&str], &RunOptions { input: Some(b"secret"), ..Default::default() })
            .unwrap();
        assert_eq!(r.stdout, "secret");
        let env = [("OMAFLAKE_TEST", "x")];
        let r = Shell::run(
            "/bin/sh",
            &["-c", "printf %s \"$OMAFLAKE_TEST\""],
            &RunOptions { environment: &env, ..Default::default() },
        )
        .unwrap();
        assert_eq!(r.stdout, "x");
        let e = Shell::check("/bin/sh", &["-c", "echo bad >&2; exit 1"], &RunOptions::default()).unwrap_err();
        assert_eq!(e.message(), "/bin/sh -c echo bad >&2; exit 1 failed (1):\nbad");
        assert!(Shell::which("no-such-program-omaflake").is_err());
        assert_eq!(Shell::which("sh").unwrap().rsplit('/').next(), Some("sh"));
    }

    #[test]
    fn streams_lines() {
        let lines = Mutex::new(Vec::new());
        let collect = |l: &str| lines.lock().unwrap().push(l.to_string());
        let r = Shell::stream(
            "/bin/sh",
            &["-c", "echo a >&2; echo out; printf b >&2"],
            &StreamOptions::default(),
            &collect,
        )
        .unwrap();
        assert_eq!(r.stdout, "out\n");
        assert_eq!(*lines.lock().unwrap(), vec!["a", "b"]);
        lines.lock().unwrap().clear();
        let opts = StreamOptions { merge_stdout: true, ..Default::default() };
        let r = Shell::stream("/bin/sh", &["-c", "echo one; echo two >&2"], &opts, &collect).unwrap();
        assert_eq!(r.stdout, "");
        assert_eq!(*lines.lock().unwrap(), vec!["one", "two"]);
    }

    #[test]
    fn cancel_stops_the_running_shell_process() {
        let token = Cancellation::new();
        let t = token.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(500));
            t.cancel();
        });
        let t0 = Instant::now();
        let r = Shell::run("/bin/sleep", &["10"], &RunOptions { cancel: Some(&token), ..Default::default() }).unwrap();
        assert!(r.status != 0 && t0.elapsed() < Duration::from_secs(5));
        assert_eq!(r.status, 15); // SIGTERM, as Swift's terminationStatus says
        assert!(Shell::run("/bin/echo", &["x"], &RunOptions { cancel: Some(&token), ..Default::default() }).is_err());
    }
}
