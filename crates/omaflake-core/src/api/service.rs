// SPDX-License-Identifier: AGPL-3.0-or-later
//! The core service (plan B120, docs/api.md §1): `omaf serve --socket` – one server per user on a Unix socket, so
//! every interface (the macOS app, the Linux app, the TUI, the CLI) shares one build queue, one job list and the
//! events. Each connection is a session of its own (`initialize`, `events.subscribe`, `shutdown` ends only it) on
//! the same `Server`.
//!
//! - The socket is `<data folder>/run/omaf.sock`; a path longer than a Unix socket takes falls back to
//!   `$XDG_RUNTIME_DIR` (Linux) or `$TMPDIR` (macOS) with a hash of the data folder, last `/tmp` (`socket_path`).
//! - Mode 0600, and a connection from another user is closed at once (peer credentials).
//! - `<socket>.lock` (flock) refuses a second service on the same socket; a socket file without a service behind it
//!   is replaced.
//! - It ends when asked (`service.restart`, once no job runs), when the data folder moved (its socket moved along),
//!   and with `idle_exit` when no client was connected and no job ran for that long.

use std::collections::HashMap;
use std::io::BufReader;
use std::os::fd::AsRawFd;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::Value;

use super::protocol;
use super::server::{lock, Server};
use super::stdio::{self, Writer};
use crate::error::{codes, Error};
use crate::paths::Paths;
use crate::qemu_runtime::FileLock;
use crate::sockets::MAX_SOCKET_PATH;

/// The socket's name in `<data folder>/run`
pub const SOCKET_NAME: &str = "omaf.sock";
/// A client that does not take a line for this long is disconnected (the others must not wait for it)
const WRITE_TIMEOUT: Duration = Duration::from_secs(10);
/// How often the service looks at its end conditions while no client connects
const LOOK: Duration = Duration::from_millis(200);

/// Where the service of the data folder `paths` listens (docs/api.md §1)
pub fn socket_path(paths: &Paths) -> PathBuf {
    let preferred = paths.run().join(SOCKET_NAME);
    if fits(&preferred) {
        return preferred;
    }
    let base = if cfg!(target_os = "linux") { "XDG_RUNTIME_DIR" } else { "TMPDIR" };
    fallback_socket(&paths.root, std::env::var_os(base).map(PathBuf::from))
}

fn fits(path: &Path) -> bool {
    path.as_os_str().len() <= MAX_SOCKET_PATH
}

/// `<base>/omaflake-<hash>.sock`, else `/tmp/omaflake-<uid>-<hash>.sock` – the hash of the data folder keeps two
/// data folders (`VM_HOME`) apart
fn fallback_socket(root: &Path, base: Option<PathBuf>) -> PathBuf {
    use sha2::{Digest, Sha256};
    let hash = hex::encode(Sha256::digest(root.as_os_str().as_encoded_bytes()));
    let hash = &hash[..16];
    if let Some(base) = base.filter(|b| !b.as_os_str().is_empty()) {
        let path = base.join(format!("omaflake-{hash}.sock"));
        if fits(&path) {
            return path;
        }
    }
    // SAFETY: getuid cannot fail
    let uid = unsafe { libc::getuid() };
    PathBuf::from(format!("/tmp/omaflake-{uid}-{hash}.sock"))
}

/// The lock file next to a socket
pub fn lock_path(socket: &Path) -> PathBuf {
    let mut name = socket.as_os_str().to_owned();
    name.push(".lock");
    PathBuf::from(name)
}

/// Is `version` newer than `other` (`major.minor.patch`, a pre-release such as `-alpha.1` older than the release)?
/// Versions that do not parse are never newer.
pub fn is_newer(version: &str, other: &str) -> bool {
    fn parse(v: &str) -> Option<(u64, u64, u64, bool)> {
        let (core, pre) = match v.trim().trim_start_matches('v').split_once('-') {
            Some((c, _)) => (c, true),
            None => (v.trim().trim_start_matches('v'), false),
        };
        let mut parts = core.split('.').map(|p| p.parse::<u64>().ok());
        let (a, b, c) = (parts.next()??, parts.next().unwrap_or(Some(0))?, parts.next().unwrap_or(Some(0))?);
        Some((a, b, c, !pre))
    }
    match (parse(version), parse(other)) {
        (Some(a), Some(b)) => a > b,
        _ => false,
    }
}

/// What `omaf serve --socket` was given
#[derive(Debug, Clone, Default)]
pub struct Options {
    /// The socket (None: `socket_path` of the data folder)
    pub socket: Option<PathBuf>,
    /// End when no client was connected and no job ran for this long
    pub idle_exit: Option<Duration>,
}

/// Why the service ended
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    /// `service.restart`
    Restart,
    /// The data folder moved; its socket with it
    Moved,
    /// `idle_exit`
    Idle,
}

/// `omaf serve --socket`: serves the user's data folder (`Paths::standard()`) on the socket until it ends
pub fn serve_socket(options: Options) -> crate::Result<Ended> {
    let paths = Paths::standard();
    let socket = options.socket.clone().unwrap_or_else(|| socket_path(&paths));
    let server = Server::new(paths, Arc::new(|_: &str, _: Value| {}));
    let listening = Listening::bind(&socket)?;
    eprintln!("omaf serve: listening on {} (omaf {}, pid {})", socket.display(), crate::VERSION, std::process::id());
    let ended = listening.serve(server, options.idle_exit);
    eprintln!("omaf serve: ended ({ended:?})");
    Ok(ended)
}

/// A bound socket, its lock held
pub(crate) struct Listening {
    listener: UnixListener,
    path: PathBuf,
    _lock: FileLock,
}

impl Listening {
    /// Takes the lock, replaces a socket file nobody serves and listens (0600)
    pub fn bind(path: &Path) -> crate::Result<Listening> {
        if !fits(path) {
            return Err(Error::new(format!("Socket path too long (over {MAX_SOCKET_PATH} bytes): {}", path.display())));
        }
        let lock = FileLock::new(
            &lock_path(path),
            &format!("omaf serve already runs on {} – one service per data folder", path.display()),
            false,
        )?;
        // Holding the lock: a socket file there is a leftover of a service that ended without removing it
        if path.exists() {
            std::fs::remove_file(path).map_err(|e| Error::from(e).context(format!("{}: ", path.display())))?;
        }
        let listener = UnixListener::bind(path).map_err(|e| Error::from(e).context(format!("{}: ", path.display())))?;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
        listener.set_nonblocking(true)?;
        Ok(Listening { listener, path: path.to_path_buf(), _lock: lock })
    }

    /// Accepts clients until an end condition holds; then closes every connection, cancels what still runs (nothing,
    /// for a restart or the idle end) and removes the socket
    pub fn serve(self, server: Server, idle_exit: Option<Duration>) -> Ended {
        // The first connection of `Server::new` is not a client
        server.disconnect();
        let root = server.paths().root;
        let conns: Arc<Mutex<HashMap<u64, UnixStream>>> = Arc::default();
        let next = AtomicU64::new(0);
        let mut idle_since: Option<Instant> = None;
        let ended = loop {
            let mut pfd = libc::pollfd { fd: self.listener.as_raw_fd(), events: libc::POLLIN, revents: 0 };
            // SAFETY: one valid pollfd for the listener, which lives as long as this call
            unsafe { libc::poll(&mut pfd, 1, LOOK.as_millis() as libc::c_int) };
            while let Ok((stream, _)) = self.listener.accept() {
                // An accepted socket inherits the listener's non-blocking mode on macOS
                if stream.set_nonblocking(false).is_err() || !same_user(&stream) {
                    eprintln!("omaf serve: refused a connection of another user");
                    continue;
                }
                let id = next.fetch_add(1, Ordering::SeqCst);
                let Ok(handle) = stream.try_clone() else { continue };
                lock(&conns).insert(id, handle);
                let (server, conns) = (server.clone(), conns.clone());
                std::thread::spawn(move || {
                    connection(&server, stream);
                    lock(&conns).remove(&id);
                });
            }
            let busy = server.busy();
            if busy {
                idle_since = None;
                continue;
            }
            if server.restart_requested() {
                break Ended::Restart;
            }
            if server.paths().root != root {
                break Ended::Moved;
            }
            if let Some(limit) = idle_exit {
                if lock(&conns).is_empty() {
                    if idle_since.get_or_insert_with(Instant::now).elapsed() >= limit {
                        break Ended::Idle;
                    }
                } else {
                    idle_since = None;
                }
            }
        };
        // No new clients; those connected see their connection close and start a new service when they need one
        let _ = std::fs::remove_file(&self.path);
        for stream in lock(&conns).values() {
            let _ = stream.shutdown(std::net::Shutdown::Both);
        }
        server.close(stdio::JOBS_STOP);
        ended
    }
}

/// One client: its own connection to the server, its lines in and out
fn connection(server: &Server, stream: UnixStream) {
    let (Ok(out), Ok(closer), Ok(ender)) = (stream.try_clone(), stream.try_clone(), stream.try_clone()) else {
        return;
    };
    let _ = out.set_write_timeout(Some(WRITE_TIMEOUT));
    let writer = Writer::with_on_broken(out, move || {
        // A client that does not read: its connection ends, the reader with it
        let _ = closer.shutdown(std::net::Shutdown::Both);
    });
    let w = writer.clone();
    let conn =
        server.connect(Arc::new(move |method: &str, params: Value| w.line(&protocol::notification(method, params))));
    stdio::session(&conn, writer, BufReader::new(stream));
    conn.disconnect();
    // After `shutdown` the client waits for the end of the stream
    let _ = ender.shutdown(std::net::Shutdown::Both);
}

/// The peer runs as this user (the socket's 0600 already says so; this also holds where a folder's mode does not)
fn same_user(stream: &UnixStream) -> bool {
    let fd = stream.as_raw_fd();
    // SAFETY: geteuid cannot fail
    let me = unsafe { libc::geteuid() };
    #[cfg(target_os = "linux")]
    {
        // SAFETY: an all-zero ucred is valid; getsockopt fills at most its size
        let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
        let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
        // SAFETY: fd is the accepted socket, cred and len are valid for the call
        let rc = unsafe {
            libc::getsockopt(fd, libc::SOL_SOCKET, libc::SO_PEERCRED, (&mut cred as *mut libc::ucred).cast(), &mut len)
        };
        rc == 0 && cred.uid == me
    }
    #[cfg(not(target_os = "linux"))]
    {
        let (mut uid, mut gid) = (0, 0);
        // SAFETY: fd is the accepted socket, uid and gid are valid for the call
        let rc = unsafe { libc::getpeereid(fd, &mut uid, &mut gid) };
        rc == 0 && uid == me
    }
}

/// Is the service of this socket running (does it accept a connection)?
pub fn is_running(socket: &Path) -> bool {
    UnixStream::connect(socket).is_ok()
}

/// The error when the service refuses to start: `in-use` means another one runs
pub fn already_running(e: &Error) -> bool {
    e.code() == Some(codes::IN_USE)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::client::Client;
    use crate::api::jobs::{JobCtx, JobSpec};
    use crate::api::types::JobKind;
    use crate::paths::tests::temp_paths;
    use serde_json::json;
    use std::thread::JoinHandle;

    /// A service on a temporary data folder and a short socket path, in a thread
    fn service(idle: Option<Duration>) -> (tempfile::TempDir, tempfile::TempDir, Server, PathBuf, JoinHandle<Ended>) {
        let (dir, paths) = temp_paths();
        let sockets = tempfile::Builder::new().prefix("oms").tempdir_in("/tmp").unwrap();
        let socket = sockets.path().join("s.sock");
        let server = Server::for_tests(paths, Arc::new(|_: &str, _: Value| {}));
        let listening = Listening::bind(&socket).unwrap();
        let s = server.clone();
        let thread = std::thread::spawn(move || listening.serve(s, idle));
        (dir, sockets, server, socket, thread)
    }

    fn client(socket: &Path) -> Client {
        let c = Client::connect(socket).unwrap();
        c.initialize("test").unwrap();
        c
    }

    /// Events of `c` until one named `method` came (or a few seconds passed)
    fn wait_for(c: &Client, method: &str) -> Option<Value> {
        wait_until(c, method, |_| true)
    }

    fn wait_until(c: &Client, method: &str, pick: impl Fn(&Value) -> bool) -> Option<Value> {
        let end = Instant::now() + Duration::from_secs(5);
        while Instant::now() < end {
            if let Some((m, p)) = c.next_event(Duration::from_millis(100)) {
                if m == method && pick(&p) {
                    return Some(p);
                }
            }
        }
        None
    }

    #[test]
    fn two_connections_share_jobs_and_events() {
        let (_d, _s, server, socket, thread) = service(None);
        let a = client(&socket);
        let b = client(&socket);
        a.call("events.subscribe", json!({"topics": ["jobs", "messages"]})).unwrap();
        b.call("events.subscribe", json!({"topics": ["jobs"]})).unwrap();
        let job = server.inner.jobs.submit(JobSpec::new(
            JobKind::Export,
            "Export x",
            Box::new(|ctx: &JobCtx| {
                ctx.log("copying");
                Ok(json!({"path": "/tmp/x"}))
            }),
        ));
        job.wait();
        // Both see the job, in the list and as events
        for c in [&a, &b] {
            let list = c.call("jobs.list", Value::Null).unwrap();
            assert_eq!(list[0]["id"], json!(job.id));
            assert!(wait_for(c, "jobs.changed").is_some());
        }
        // One's log read is the other's too; a message goes only to who subscribed to messages
        assert_eq!(b.call("jobs.log", json!({"id": job.id})).unwrap()["lines"], json!(["copying"]));
        server.events().message(crate::api::types::Level::Info, "hello");
        // (other messages may come first: the temporary data folder has no flake)
        assert!(wait_until(&a, "message", |p| p["text"] == json!("hello")).is_some());
        // shutdown ends one session, not the service
        assert_eq!(b.call("shutdown", Value::Null).unwrap(), Value::Null);
        assert!(a.call("jobs.list", Value::Null).is_ok());
        // A later client gets the jobs at subscribe time
        let c = client(&socket);
        c.call("events.subscribe", json!({"topics": ["jobs"]})).unwrap();
        assert_eq!(wait_for(&c, "jobs.changed").unwrap()["job"]["id"], json!(job.id));
        assert!(!thread.is_finished());
        // Restart: no job runs, so it ends at once and closes the connections
        assert_eq!(a.call("service.restart", Value::Null).unwrap(), json!({"jobs": 0}));
        assert_eq!(thread.join().unwrap(), Ended::Restart);
        assert!(!socket.exists());
        assert!(a.call("jobs.list", Value::Null).is_err());
    }

    #[test]
    fn views_are_per_connection_and_the_activity_is_shared() {
        let (_d, _s, server, socket, _thread) = service(None);
        let a = client(&socket);
        let b = client(&socket);
        // views.get over the socket
        assert_eq!(a.call("views.get", json!({"page": "vms"})).unwrap()["title"]["key"], json!("Virtual Machines"));
        // Each connection gets its own pages
        a.call("views.subscribe", json!({"pages": [{"page": "activity"}]})).unwrap();
        b.call("views.subscribe", json!({"pages": [{"page": "home"}, {"page": "activity"}]})).unwrap();
        assert!(wait_until(&b, "view.changed", |p| p["page"] == json!("home")).is_some());
        assert!(wait_until(&a, "view.changed", |p| p["page"] == json!("home")).is_none(), "a did not subscribe home");
        // One activity list: a refusal on a is seen by b
        assert!(a.call("vms.remove", json!({"name": "ghost"})).is_err());
        let seen = |p: &Value| {
            p["page"] == json!("activity")
                && p["model"]["entries"].as_array().is_some_and(|e| {
                    e.iter().any(|x| x["text"]["text"].as_str().is_some_and(|t| t.starts_with("ghost: ")))
                })
        };
        assert!(wait_until(&b, "view.changed", seen).is_some());
        assert!(wait_until(&a, "view.changed", seen).is_some());
        server.close(Duration::from_secs(5));
    }

    #[test]
    fn requests_before_initialize_are_refused_per_connection() {
        let (_d, _s, server, socket, thread) = service(None);
        let _a = client(&socket);
        let b = Client::connect(&socket).unwrap();
        assert_eq!(b.call("jobs.list", Value::Null).unwrap_err().code, protocol::NOT_INITIALIZED);
        server.inner.restart.store(true, Ordering::SeqCst);
        assert_eq!(thread.join().unwrap(), Ended::Restart);
    }

    #[test]
    fn a_restart_waits_for_running_jobs() {
        let (_d, _s, server, socket, thread) = service(None);
        let a = client(&socket);
        let (tx, rx) = std::sync::mpsc::channel::<()>();
        let rx = Mutex::new(rx);
        server.inner.jobs.submit(JobSpec::new(
            JobKind::Export,
            "Export slow",
            Box::new(move |_: &JobCtx| {
                let _ = lock(&rx).recv();
                Ok(Value::Null)
            }),
        ));
        assert_eq!(a.call("service.restart", Value::Null).unwrap(), json!({"jobs": 1}));
        std::thread::sleep(Duration::from_millis(600));
        assert!(!thread.is_finished() && a.call("jobs.list", Value::Null).is_ok());
        tx.send(()).unwrap();
        assert_eq!(thread.join().unwrap(), Ended::Restart);
    }

    #[test]
    fn a_second_service_is_refused() {
        let (_d, _s, server, socket, thread) = service(None);
        let e = Listening::bind(&socket).err().unwrap();
        assert!(already_running(&e), "{e}");
        assert!(is_running(&socket));
        server.inner.restart.store(true, Ordering::SeqCst);
        thread.join().unwrap();
        assert!(!is_running(&socket));
        // A leftover socket file without a service is replaced
        std::os::unix::net::UnixListener::bind(&socket).unwrap();
        // Other tests fork processes meanwhile: a child holds a copy of the lock's descriptor from fork until its exec
        // closes it (O_CLOEXEC), so the lock can stay taken for a moment after the service ended
        let deadline = Instant::now() + Duration::from_secs(2);
        let again = loop {
            match Listening::bind(&socket) {
                Ok(l) => break l,
                Err(e) if already_running(&e) && Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(20))
                }
                Err(e) => panic!("{e}"),
            }
        };
        drop(again);
    }

    #[test]
    fn idle_exit_after_the_last_client() {
        let (_d, _s, _server, socket, thread) = service(Some(Duration::from_millis(400)));
        let a = client(&socket);
        std::thread::sleep(Duration::from_millis(900));
        assert!(!thread.is_finished(), "a connected client keeps it");
        drop(a);
        let start = Instant::now();
        assert_eq!(thread.join().unwrap(), Ended::Idle);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn the_socket_is_private() {
        let (_d, _s, server, socket, thread) = service(None);
        let mode = std::fs::metadata(&socket).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        server.inner.restart.store(true, Ordering::SeqCst);
        thread.join().unwrap();
    }

    #[test]
    fn socket_paths_fall_back_when_too_long() {
        let short = Paths::new("/x/.omaflake", None);
        assert_eq!(socket_path(&short), PathBuf::from("/x/.omaflake/run/omaf.sock"));
        let long = Paths::new(format!("/x/{}", "d".repeat(120)), None);
        let fallback = fallback_socket(&long.root, Some(PathBuf::from("/run/user/1000")));
        let name = fallback.file_name().unwrap().to_string_lossy().into_owned();
        assert!(fallback.starts_with("/run/user/1000") && name.starts_with("omaflake-") && name.ends_with(".sock"));
        // Another data folder, another socket; the same one, the same
        let other = fallback_socket(Path::new("/y"), Some(PathBuf::from("/run/user/1000")));
        assert_ne!(fallback, other);
        assert_eq!(fallback, fallback_socket(&long.root, Some(PathBuf::from("/run/user/1000"))));
        // A base too long itself, or none: /tmp with the user id
        let tmp = fallback_socket(&long.root, Some(PathBuf::from(format!("/{}", "t".repeat(100)))));
        assert!(tmp.starts_with("/tmp") && fits(&tmp));
        assert!(fallback_socket(&long.root, None).starts_with("/tmp"));
        assert!(fits(&socket_path(&long)));
        assert_eq!(lock_path(Path::new("/a/omaf.sock")), PathBuf::from("/a/omaf.sock.lock"));
    }

    #[test]
    fn versions_compare() {
        assert!(is_newer("0.2.0", "0.1.9"));
        assert!(is_newer("1.0.0", "1.0.0-alpha.2"));
        assert!(is_newer("0.10.0", "0.9.3"));
        assert!(!is_newer("0.2.0", "0.2.0"));
        assert!(!is_newer("0.1.0", "0.2.0"));
        assert!(!is_newer("x", "0.1.0") && !is_newer("0.2.0", "dev"));
    }
}
