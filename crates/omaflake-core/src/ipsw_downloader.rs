// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of IPSWDownloader.swift: `omaf-runner latest`, the IPSW download, keeping the Mac awake, free space.
//!
//! Not 1:1 (no URLSession): Swift kept URLSession's opaque resume data in `<build>.ipsw.resume`. The Rust downloader
//! keeps the bytes received so far in `<build>.ipsw.partial` and continues with an HTTP `Range` request (ureq);
//! a Swift `.resume` file cannot be used and is ignored. Prerequisites asks `IpswDownloader::resume_file_for`.
//! A server that ignores the range (200) or refuses it (416, e.g. the file changed) starts the download over.
//! Cancelling is noticed between two reads (each read times out after 60 s, like URLSession's request timeout).
//! `KeepAwake` used `ProcessInfo.beginActivity`; here it is `caffeinate -i -w <our pid>` on macOS (a child that ends
//! with us, even when we crash), nothing elsewhere. `free_space` used `volumeAvailableCapacityForImportantUsage`
//! (counts purgeable space); here it is `statvfs` (f_bavail × f_frsize), the plain free space – smaller than what
//! macOS can make available (OPEN: purgeable space is not counted).

use std::fs::OpenOptions;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::cancellation::Cancellation;
use crate::error::{codes, Error, IoContext, Result};
use crate::shell::{RunOptions, Shell};

/// A macOS release as Apple offers it for this Mac (`omaf-runner latest`, VZMacOSRestoreImage.latestSupported).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacOsRelease {
    pub version: String,
    pub build: String,
    /// https URL of the IPSW
    pub url: String,
    pub bytes: Option<i64>,
}

impl MacOsRelease {
    /// Output of `omaf-runner latest`:
    /// ```text
    /// macOS 27.0.1 (26A434)
    /// URL:   https://updates.cdn-apple.com/…/UniversalMac_27.0.1_26A434_Restore.ipsw
    /// Size:  26.6 GB
    /// ```
    /// None unless the first line is "macOS <version> (<build>)" and the URL is https; Size "<x> GB" → x × 1e9 bytes.
    pub fn parse(text: &str) -> Option<MacOsRelease> {
        let lines: Vec<&str> =
            text.split('\n').filter(|l| !l.is_empty()).map(|l| l.trim_matches([' ', '\t'])).collect();
        let first = *lines.first()?;
        if !first.starts_with("macOS ") {
            return None;
        }
        let (open, close) = (first.rfind('(')?, first.rfind(')')?);
        if open >= close {
            return None;
        }
        let url = lines.iter().find(|l| l.starts_with("URL:"))?[4..].trim_matches([' ', '\t']);
        // An https URL with a host, as Swift's `URL(string:)` + `scheme == "https"`
        let host = url.strip_prefix("https://")?;
        if host.is_empty() || host.starts_with('/') || url.contains(char::is_whitespace) {
            return None;
        }
        let version: String = first["macOS ".len()..].chars().take_while(|c| *c != ' ').collect();
        let bytes = lines
            .iter()
            .find(|l| l.starts_with("Size:"))
            .and_then(|l| l[5..].trim_matches([' ', '\t']).replace(" GB", "").parse::<f64>().ok())
            .map(|gb| (gb * 1e9) as i64);
        Some(MacOsRelease { version, build: first[open + 1..close].to_string(), url: url.to_string(), bytes })
    }

    /// Asks Apple via the runner (`<runner> latest`, needs its entitlement); None when offline or the runner is
    /// unusable.
    pub fn latest(runner: &str) -> Option<MacOsRelease> {
        let r = Shell::run(runner, &["latest"], &RunOptions::default()).ok()?;
        if r.status != 0 {
            return None;
        }
        Self::parse(&r.stdout)
    }
}

/// Downloads an IPSW from Apple into `ipsw/<build>.ipsw`. An interrupted or cancelled download keeps what it has in
/// `ipsw/<build>.ipsw.partial` and continues from there next time.
#[derive(Debug, Clone)]
pub struct IpswDownloader {
    pub release: MacOsRelease,
    pub destination: PathBuf,
    cancellation: Cancellation,
}

impl IpswDownloader {
    pub fn new(release: MacOsRelease, directory: &Path) -> Self {
        let destination = Self::file(&release, directory);
        IpswDownloader { release, destination, cancellation: Cancellation::new() }
    }

    /// `<directory>/<build>.ipsw`
    pub fn file(release: &MacOsRelease, directory: &Path) -> PathBuf {
        directory.join(format!("{}.ipsw", release.build))
    }

    /// Where an interrupted download waits: `<destination>.partial`
    pub fn resume_file(&self) -> PathBuf {
        Self::resume_file_for(&self.release, self.destination.parent().unwrap_or(Path::new(".")))
    }

    /// `<directory>/<build>.ipsw.partial`
    pub fn resume_file_for(release: &MacOsRelease, directory: &Path) -> PathBuf {
        directory.join(format!("{}.ipsw.partial", release.build))
    }

    /// Blocks until the file is complete (returns it) or the download failed/was cancelled (what was received is
    /// kept). `progress` gets (received, expected) bytes. Errors as in Swift: "Download paused – it continues from here
    /// next time" (cancelled, code `cancelled`), "Download interrupted (<reason>) – it continues from here next time".
    /// `progress` may borrow (`&Progress` is `'static`-bounded as a type alias; this one is not).
    pub fn run(&self, progress: &(dyn Fn(i64, i64) + Send + Sync + '_)) -> Result<PathBuf> {
        if self.destination.exists() {
            return Ok(self.destination.clone());
        }
        if let Some(dir) = self.destination.parent() {
            std::fs::create_dir_all(dir).with_path(dir)?;
        }
        let partial = self.resume_file();
        // Once more from the start when the server does not continue what we have
        for _ in 0..2 {
            match self.attempt(&partial, progress)? {
                Attempt::Complete => {
                    std::fs::rename(&partial, &self.destination).with_path(&self.destination)?;
                    return Ok(self.destination.clone());
                }
                Attempt::StartOver => {
                    std::fs::File::create(&partial).with_path(&partial)?;
                }
            }
        }
        Err(interrupted("the server does not continue the download"))
    }

    /// One request: from the end of `partial` (Range) or from the start
    fn attempt(&self, partial: &Path, progress: &(dyn Fn(i64, i64) + Send + Sync + '_)) -> Result<Attempt> {
        self.check_cancelled()?;
        let mut offset = std::fs::metadata(partial).map(|m| m.len()).unwrap_or(0);
        let agent: ureq::Agent = ureq::Agent::config_builder()
            .http_status_as_error(false)
            .timeout_connect(Some(Duration::from_secs(30)))
            .timeout_recv_response(Some(Duration::from_secs(60)))
            .timeout_per_read(Some(Duration::from_secs(60)))
            .build()
            .into();
        // The file as it is on the server: no compression, which would make the byte ranges meaningless
        let mut request = agent.get(&self.release.url).header("Accept-Encoding", "identity");
        if offset > 0 {
            request = request.header("Range", format!("bytes={offset}-"));
        }
        let mut response = request.call().map_err(|e| interrupted(&e.to_string()))?;
        let status = response.status().as_u16();
        let range = response.headers().get("content-range").and_then(|v| v.to_str().ok()).and_then(parse_content_range);
        let mut file = match status {
            206 => {
                if range.and_then(|(start, _)| start) != Some(offset) {
                    return Ok(Attempt::StartOver);
                }
                OpenOptions::new().append(true).open(partial).with_path(partial)?
            }
            200 => {
                // No range (a first request, or the server ignored it): from the start
                offset = 0;
                std::fs::File::create(partial).with_path(partial)?
            }
            416 if offset > 0 => {
                // Nothing left to send: complete if what we have is the whole file, otherwise start over
                let total = range.and_then(|(_, t)| t);
                return Ok(if total == Some(offset) { Attempt::Complete } else { Attempt::StartOver });
            }
            _ => return Err(interrupted(&format!("HTTP {status}"))),
        };
        let total: i64 = match (status, range.and_then(|(_, t)| t)) {
            (206, Some(t)) => t as i64,
            _ => response.body().content_length().map(|n| (n + offset) as i64).unwrap_or(-1),
        };
        let mut received = offset;
        let mut reader = response.body_mut().as_reader();
        let mut buf = vec![0u8; 1024 * 1024];
        loop {
            self.check_cancelled()?;
            let n = match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => n,
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) => return Err(interrupted(&crate::error::os_text(&e))),
            };
            file.write_all(&buf[..n]).map_err(|e| interrupted(&crate::error::os_text(&e)))?;
            received += n as u64;
            progress(received as i64, total);
        }
        if total >= 0 && received as i64 != total {
            return Err(interrupted(&format!("{received} of {total} bytes")));
        }
        Ok(Attempt::Complete)
    }

    fn check_cancelled(&self) -> Result<()> {
        if self.cancellation.is_cancelled() {
            return Err(Error::with_code(codes::CANCELLED, "Download paused – it continues from here next time"));
        }
        Ok(())
    }

    /// Stops the download (from another thread; clones share the token) and keeps what was received for the next `run`.
    pub fn cancel(&self) {
        self.cancellation.cancel()
    }
}

enum Attempt {
    /// The partial file is the whole IPSW
    Complete,
    /// What the partial file holds cannot be continued: empty it and ask again
    StartOver,
}

fn interrupted(reason: &str) -> Error {
    Error::with_code(codes::NETWORK, format!("Download interrupted ({reason}) – it continues from here next time"))
}

/// `Content-Range: bytes <start>-<end>/<total>` (or `bytes */<total>` with a 416) → (start, total)
fn parse_content_range(value: &str) -> Option<(Option<u64>, Option<u64>)> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let start = if range == "*" { None } else { Some(range.split_once('-')?.0.trim().parse().ok()?) };
    Some((start, total.trim().parse().ok()))
}

/// Keeps the Mac awake (no idle sleep) while long work runs: downloads, macOS installation, image builds.
pub enum KeepAwake {}

impl KeepAwake {
    pub fn during<T>(reason: &str, body: impl FnOnce() -> T) -> T {
        // The guard ends caffeinate also when `body` panics
        let _awake = Caffeinate::start(reason);
        body()
    }
}

/// `caffeinate -i -w <our pid>`: no idle sleep while it runs; it ends with us (or when dropped)
struct Caffeinate {
    child: Option<std::process::Child>,
}

impl Caffeinate {
    fn start(_reason: &str) -> Caffeinate {
        // caffeinate takes no reason; `pmset -g assertions` names it "caffeinate command-line tool"
        #[cfg(target_os = "macos")]
        {
            use std::process::{Command, Stdio};
            let child = Command::new("/usr/bin/caffeinate")
                .args(["-i", "-w", &std::process::id().to_string()])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .spawn()
                .ok();
            Caffeinate { child }
        }
        #[cfg(not(target_os = "macos"))]
        {
            Caffeinate { child: None }
        }
    }
}

impl Drop for Caffeinate {
    fn drop(&mut self) {
        if let Some(mut c) = self.child.take() {
            let _ = c.kill();
            let _ = c.wait();
        }
    }
}

/// statvfs: blocks available to unprivileged users × fragment size
fn statvfs_available(dir: &Path) -> Option<i64> {
    use std::os::unix::ffi::OsStrExt;
    let path = std::ffi::CString::new(dir.as_os_str().as_bytes()).ok()?;
    // SAFETY: a zeroed statvfs is a valid out-parameter; `path` is NUL-terminated and outlives the call
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(path.as_ptr(), &mut st) } != 0 {
        return None;
    }
    #[allow(clippy::unnecessary_cast)] // the field types differ between macOS and Linux
    let bytes = (st.f_bavail as u64).saturating_mul(st.f_frsize as u64);
    Some(bytes.min(i64::MAX as u64) as i64)
}

/// Free space for the volume of `path` (the nearest existing folder above it). Swift asked for "important usage"
/// (with purgeable space) and fell back to the plain value, which the read-only Nix store volume needed; `statvfs`
/// is the plain value everywhere.
pub fn free_space(path: &Path) -> Option<i64> {
    let mut dir = path.to_path_buf();
    while !dir.exists() && dir != Path::new("/") {
        if !dir.pop() {
            return None;
        }
    }
    statvfs_available(&dir)
}

/// "26.6 GB" (decimal gigabytes, one digit)
pub fn gigabytes(bytes: i64) -> String {
    format!("{:.1} GB", bytes as f64 / 1e9)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;
    use std::io::{BufRead, BufReader};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    // PreflightTests.parsesRunnerLatest
    #[test]
    fn parses_runner_latest() {
        let out = "macOS 27.0.1 (26A434)\nURL:   https://updates.cdn-apple.com/x/UniversalMac_27.0.1_26A434_Restore.ipsw\nSize:  26.6 GB\n";
        let r = MacOsRelease::parse(out).unwrap();
        assert_eq!((r.version.as_str(), r.build.as_str(), r.bytes), ("27.0.1", "26A434", Some(26_600_000_000)));
        assert_eq!(r.url, "https://updates.cdn-apple.com/x/UniversalMac_27.0.1_26A434_Restore.ipsw");
        assert_eq!(MacOsRelease::parse("Error: offline"), None);
        // only https; an unknown size is no size
        assert_eq!(MacOsRelease::parse(&out.replace("https:", "http:")), None);
        let unknown = MacOsRelease::parse(&out.replace("26.6 GB", "unknown")).unwrap();
        assert_eq!(unknown.bytes, None);
        assert_eq!(IpswDownloader::file(&r, Path::new("/d")), Path::new("/d/26A434.ipsw"));
        assert_eq!(IpswDownloader::resume_file_for(&r, Path::new("/d")), Path::new("/d/26A434.ipsw.partial"));
    }

    #[test]
    fn latest_asks_the_runner() {
        let (_t, p) = temp_paths();
        std::fs::create_dir_all(&p.root).unwrap();
        let runner = p.root.join("omaf-runner");
        let out = "macOS 27.0.1 (26A434)\nURL:   https://example.invalid/x.ipsw\nSize:  unknown\n";
        std::fs::write(&runner, format!("#!/bin/sh\n[ \"$1\" = latest ] && printf '{out}'\n")).unwrap();
        std::fs::set_permissions(&runner, std::os::unix::fs::PermissionsExt::from_mode(0o755)).unwrap();
        assert_eq!(MacOsRelease::latest(runner.to_str().unwrap()).unwrap().build, "26A434");
        assert_eq!(MacOsRelease::latest("/usr/bin/false"), None);
    }

    #[test]
    fn gigabytes_and_free_space() {
        assert_eq!(gigabytes(26_600_000_000), "26.6 GB");
        let (_t, p) = temp_paths();
        // a path that does not exist yet counts on the volume of the nearest folder above it
        assert!(free_space(&p.root.join("ipsw/not/yet")).unwrap() > 0);
        // PreflightTests.freeSpaceFallsBackForTheNixStore
        assert!(free_space(Path::new("/nix/store")).unwrap_or(0) > 0 || !Path::new("/nix/store").exists());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn keep_awake_runs_caffeinate_while_the_body_runs() {
        let pid = KeepAwake::during("test", || {
            let c = Caffeinate::start("test");
            let pid = c.child.as_ref().map(|c| c.id() as crate::spawn::Pid).unwrap();
            assert!(crate::spawn::executable_path(pid).unwrap().ends_with("caffeinate"));
            drop(c);
            pid
        });
        assert!(
            !crate::spawn::is_alive(pid)
                || crate::spawn::executable_path(pid).is_none_or(|p| !p.ends_with("caffeinate"))
        );
        assert_eq!(KeepAwake::during("test", || 42), 42);
    }

    #[test]
    fn content_ranges() {
        assert_eq!(parse_content_range("bytes 100-299/300"), Some((Some(100), Some(300))));
        assert_eq!(parse_content_range("bytes */300"), Some((None, Some(300))));
        assert_eq!(parse_content_range("bytes 0-9/*"), Some((Some(0), None)));
        assert_eq!(parse_content_range("items 1-2/3"), None);
    }

    /// How the test server answers one request
    #[derive(Clone, Copy)]
    enum Serve {
        /// Honours Range; sends at most `limit` body bytes, then closes the connection
        Ranges { limit: usize },
        /// Ignores Range: always 200 with the whole file
        Whole,
    }

    /// A tiny HTTP/1.1 server on 127.0.0.1 answering one request per connection; returns the URL and the Range
    /// headers it saw
    fn server(data: Arc<Vec<u8>>, plan: Vec<Serve>) -> (String, Arc<Mutex<Vec<Option<String>>>>) {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://127.0.0.1:{}/x.ipsw", l.local_addr().unwrap().port());
        let seen = Arc::new(Mutex::new(Vec::new()));
        let s = seen.clone();
        std::thread::spawn(move || {
            for serve in plan {
                let Ok((mut c, _)) = l.accept() else { return };
                let mut r = BufReader::new(c.try_clone().unwrap());
                let mut range = None;
                loop {
                    let mut line = String::new();
                    if r.read_line(&mut line).unwrap_or(0) == 0 || line == "\r\n" {
                        break;
                    }
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("range: bytes=") {
                        range = Some(v.trim().trim_end_matches('-').parse::<usize>().unwrap());
                    }
                }
                s.lock().unwrap().push(range.map(|r| format!("bytes={r}-")));
                let n = data.len();
                let (head, body) = match (serve, range) {
                    (Serve::Ranges { .. }, Some(start)) if start >= n => {
                        (format!("HTTP/1.1 416 Range Not Satisfiable\r\nContent-Range: bytes */{n}\r\nContent-Length: 0\r\n"), &data[0..0])
                    }
                    (Serve::Ranges { .. }, Some(start)) => (
                        format!("HTTP/1.1 206 Partial Content\r\nContent-Range: bytes {start}-{}/{n}\r\nContent-Length: {}\r\n", n - 1, n - start),
                        &data[start..],
                    ),
                    _ => (format!("HTTP/1.1 200 OK\r\nContent-Length: {n}\r\n"), &data[..]),
                };
                let limit = match serve {
                    Serve::Ranges { limit } => limit,
                    Serve::Whole => usize::MAX,
                };
                let _ = c.write_all(format!("{head}Connection: close\r\n\r\n").as_bytes());
                let _ = c.write_all(&body[..body.len().min(limit)]);
                let _ = c.flush();
            }
        });
        (url, seen)
    }

    fn release(url: &str) -> MacOsRelease {
        MacOsRelease { version: "27.0.1".into(), build: "26A434".into(), url: url.into(), bytes: None }
    }

    fn data() -> Arc<Vec<u8>> {
        Arc::new((0..300_000u32).map(|i| (i % 251) as u8).collect())
    }

    #[test]
    fn interrupted_download_continues_with_a_range() {
        let (_t, p) = temp_paths();
        let data = data();
        let (url, seen) =
            server(data.clone(), vec![Serve::Ranges { limit: 100_000 }, Serve::Ranges { limit: usize::MAX }]);
        let d = IpswDownloader::new(release(&url), &p.ipsw());
        let last = Mutex::new((0, 0));
        let progress = |done: i64, total: i64| *last.lock().unwrap() = (done, total);
        let e = d.run(&progress).unwrap_err();
        assert_eq!(e.code(), Some(codes::NETWORK));
        assert!(
            e.message().starts_with("Download interrupted (")
                && e.message().ends_with(") – it continues from here next time")
        );
        assert_eq!(std::fs::metadata(d.resume_file()).unwrap().len(), 100_000);
        assert!(!d.destination.exists());

        assert_eq!(d.run(&progress).unwrap(), d.destination);
        assert_eq!(std::fs::read(&d.destination).unwrap(), *data);
        assert!(!d.resume_file().exists());
        assert_eq!(*last.lock().unwrap(), (300_000, 300_000));
        assert_eq!(*seen.lock().unwrap(), vec![None, Some("bytes=100000-".to_string())]);
        // a finished file is not downloaded again
        assert_eq!(d.run(&progress).unwrap(), d.destination);
    }

    #[test]
    fn a_server_without_ranges_starts_over() {
        let (_t, p) = temp_paths();
        let data = data();
        let (url, _) = server(data.clone(), vec![Serve::Whole]);
        let d = IpswDownloader::new(release(&url), &p.ipsw());
        std::fs::create_dir_all(p.ipsw()).unwrap();
        std::fs::write(d.resume_file(), b"0123456789").unwrap();
        d.run(&|_, _| {}).unwrap();
        assert_eq!(std::fs::read(&d.destination).unwrap(), *data);
    }

    #[test]
    fn a_complete_partial_file_is_taken() {
        let (_t, p) = temp_paths();
        let data = data();
        let (url, _) = server(data.clone(), vec![Serve::Ranges { limit: usize::MAX }]);
        let d = IpswDownloader::new(release(&url), &p.ipsw());
        std::fs::create_dir_all(p.ipsw()).unwrap();
        std::fs::write(d.resume_file(), &*data).unwrap();
        d.run(&|_, _| {}).unwrap();
        assert_eq!(std::fs::read(&d.destination).unwrap(), *data);
    }

    #[test]
    fn cancel_pauses_and_keeps_what_was_received() {
        let (_t, p) = temp_paths();
        let data = data();
        let (url, seen) = server(data.clone(), vec![Serve::Ranges { limit: usize::MAX }]);
        let d = IpswDownloader::new(release(&url), &p.ipsw());
        std::fs::create_dir_all(p.ipsw()).unwrap();
        std::fs::write(d.resume_file(), &data[..1000]).unwrap();
        // a clone (what `started` hands out) cancels the same download
        d.clone().cancel();
        let e = d.run(&|_, _| {}).unwrap_err();
        assert!(e.is_cancelled());
        assert_eq!(e.message(), "Download paused – it continues from here next time");
        assert_eq!(std::fs::read(d.resume_file()).unwrap(), data[..1000]);
        assert!(seen.lock().unwrap().is_empty());
    }

    #[test]
    fn unreachable_server_is_a_network_error() {
        let (_t, p) = temp_paths();
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        let d = IpswDownloader::new(release(&format!("http://127.0.0.1:{port}/x.ipsw")), &p.ipsw());
        assert_eq!(d.run(&|_, _| {}).unwrap_err().code(), Some(codes::NETWORK));
    }
}
