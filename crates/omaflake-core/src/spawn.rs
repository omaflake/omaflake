// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Spawn.swift: detached child processes (QEMU, omaf-runner) – own session, outlive the CLI/app (plan §3).

use std::fs::OpenOptions;
use std::os::unix::process::CommandExt;
use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use crate::error::{os_text, Result};
use crate::vm_err;

/// A process ID (`pid_t`)
pub type Pid = i32;

/// Starts `exe` in its own session (setsid: no SIGHUP when the terminal closes), stdin from /dev/null, stdout and
/// stderr appended to `log`. `environment` is added to this process's environment. The child is not reaped here –
/// callers poll it with `waitpid(pid, WNOHANG)` while it starts (as in Swift).
///
/// Swift used posix_spawn with POSIX_SPAWN_SETSID | POSIX_SPAWN_CLOEXEC_DEFAULT; Rust's `Command` opens every
/// descriptor with O_CLOEXEC already, so only stdin/stdout/stderr reach the child (plan B104).
pub fn spawn_detached<S: AsRef<str>>(exe: &str, args: &[S], log: &Path, environment: &[(&str, &str)]) -> Result<Pid> {
    let name = Path::new(exe).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| exe.to_string());
    let out = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|e| vm_err!("Start of {name} failed: {}", os_text(&e)))?;
    let err = out.try_clone().map_err(|e| vm_err!("Start of {name} failed: {}", os_text(&e)))?;
    let mut cmd = Command::new(exe);
    cmd.args(args.iter().map(|a| a.as_ref()))
        .envs(environment.iter().copied())
        .stdin(Stdio::null())
        .stdout(Stdio::from(out))
        .stderr(Stdio::from(err));
    // SAFETY: setsid is async-signal-safe and touches no memory of the parent
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn().map_err(|e| vm_err!("Start of {name} failed: {}", os_text(&e)))?;
    // Not waited for here: dropping `Child` neither kills nor reaps it
    Ok(child.id() as Pid)
}

/// True once the process is gone (polls every 0.2 s).
pub fn wait_for_exit(pid: Pid, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        // A child of this process (e.g. QEMU started by `omaf serve`) stays a zombie – and `kill(pid, 0)` keeps
        // succeeding – until it is reaped; other processes are not affected
        if reap_if_exited(pid) || !is_alive(pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    false
}

/// `kill(pid, 0)` says the process exists (ESRCH: gone; EPERM: exists, another user's)
pub fn is_alive(pid: Pid) -> bool {
    // SAFETY: signal 0 only checks for existence
    let rc = unsafe { libc::kill(pid, 0) };
    rc == 0 || std::io::Error::last_os_error().raw_os_error() != Some(libc::ESRCH)
}

/// Sends `signal` to `pid` (SIGTERM, SIGKILL, …); false when it failed
pub fn send_signal(pid: Pid, signal: i32) -> bool {
    // SAFETY: plain libc call
    unsafe { libc::kill(pid, signal) == 0 }
}

/// Reaps `pid` if it has exited, without blocking (`waitpid(pid, WNOHANG)`): true when it was our child and is gone
pub fn reap_if_exited(pid: Pid) -> bool {
    let mut status = 0;
    // SAFETY: plain libc call with a valid out-parameter
    unsafe { libc::waitpid(pid, &mut status, libc::WNOHANG) == pid }
}

/// Executable path of a running process (to rule out PID reuse), None if it does not exist.
pub fn executable_path(pid: Pid) -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let mut buf = vec![0u8; libc::PROC_PIDPATHINFO_MAXSIZE as usize];
        // SAFETY: buf is valid for its length
        let n = unsafe { libc::proc_pidpath(pid, buf.as_mut_ptr().cast(), buf.len() as u32) };
        if n <= 0 {
            return None;
        }
        buf.truncate(n as usize);
        Some(String::from_utf8_lossy(&buf).into_owned())
    }
    #[cfg(not(target_os = "macos"))]
    {
        std::fs::read_link(format!("/proc/{pid}/exe")).ok().map(|p| p.to_string_lossy().into_owned())
    }
}

/// PID from a pid file if that process is alive and its executable path contains `executable`.
pub fn live_pid(file: &Path, executable: &str) -> Option<Pid> {
    let text = std::fs::read_to_string(file).ok()?;
    let pid: Pid = text.trim().parse().ok()?;
    if pid <= 0 {
        return None;
    }
    // SAFETY: signal 0 only checks for existence
    let alive =
        unsafe { libc::kill(pid, 0) } == 0 || std::io::Error::last_os_error().raw_os_error() == Some(libc::EPERM);
    if !alive {
        return None;
    }
    executable_path(pid).filter(|p| p.contains(executable)).map(|_| pid)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detached_child_gets_its_own_session_and_log() {
        let dir = tempfile::tempdir().unwrap();
        let log = dir.path().join("x.log");
        let pid = spawn_detached(
            "/bin/sh",
            &["-c", "echo started; ps -o sess= -p $$ >/dev/null; echo $OMAFLAKE_X"],
            &log,
            &[("OMAFLAKE_X", "y")],
        )
        .unwrap();
        assert!(pid > 0);
        let deadline = Instant::now() + Duration::from_secs(5);
        while !reap_if_exited(pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(20));
        }
        let text = std::fs::read_to_string(&log).unwrap();
        assert!(text.starts_with("started\n") && text.ends_with("y\n"));
        assert!(spawn_detached("/nonexistent/omaflake", &[] as &[&str], &log, &[])
            .unwrap_err()
            .message()
            .starts_with("Start of omaflake failed: "));
    }

    #[test]
    fn live_pid_checks_the_executable() {
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("pid");
        std::fs::write(&file, format!("{}\n", std::process::id())).unwrap();
        let me = executable_path(std::process::id() as Pid).unwrap();
        let name = Path::new(&me).file_name().unwrap().to_string_lossy().into_owned();
        assert_eq!(live_pid(&file, &name), Some(std::process::id() as Pid));
        assert_eq!(live_pid(&file, "qemu-system-omaflake"), None);
        std::fs::write(&file, "garbage").unwrap();
        assert_eq!(live_pid(&file, &name), None);
        assert!(!wait_for_exit(std::process::id() as Pid, Duration::from_millis(1)));
    }
}
