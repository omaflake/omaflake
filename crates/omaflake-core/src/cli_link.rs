// SPDX-License-Identifier: AGPL-3.0-or-later
//! The `omaf` command on the user's PATH (plan B122): the Doctor's `cli` check, and Install/Uninstall – a symlink to
//! this omaflake's `omaf`: `/usr/local/bin/omaf` on macOS (root: the administrator dialog in the app, sudo in the CLI),
//! `~/.local/bin/omaf` on Linux (no root).
//!
//! The PATH is the one a new terminal gets: the user's login shell, started interactively with a clean environment
//! (`$SHELL -l -i -c env`, a few seconds at most – on macOS /etc/paths and /etc/paths.d through path_helper, then the
//! user's own files). `omaf` is the first executable `omaf` in it, as `command -v omaf` finds it (aliases and
//! functions are not programs other tools can run). When that shell does not answer, the usual folders are searched.
//! Nothing here runs in a test against the real system: tests build a `CliLink` with fake folders and a fake shell.

use crate::vm_err;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crate::builder_maintenance::shell_quote;
use crate::error::{IoContext, Result, codes};
use crate::platform::{home_dir, is_executable_file, login_shell, user_name};
use crate::prerequisites::{Prerequisite, PrerequisiteItem, PrerequisiteStatus};

/// How long the login shell may take to print its PATH
const SHELL_TIMEOUT: Duration = Duration::from_secs(4);
/// How long `omaf --version` may take
const VERSION_TIMEOUT: Duration = Duration::from_secs(3);
/// What the login shell prints around its environment (its own files may print more)
const START: &str = "__omaf_path__";
const END: &str = "__omaf_end__";

/// Where the link goes and what it points to
#[derive(Debug, Clone)]
pub struct CliLink {
    /// This omaflake's `omaf` (canonical): the bundled one in omaflake.app, else the running executable; None when
    /// the core does not run as `omaf` (nothing to link to)
    pub ours: Option<PathBuf>,
    /// `/usr/local/bin/omaf` (macOS) or `~/.local/bin/omaf` (Linux)
    pub link: PathBuf,
    /// macOS: the link needs root
    pub mac: bool,
    pub home: PathBuf,
    /// The user's login shell
    pub shell: PathBuf,
}

/// What is at the link's place
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkState {
    Absent,
    /// A symlink; `omaflake`: it points to an `omaf` (this omaflake's or another copy's) – Uninstall removes it
    Symlink {
        target: PathBuf,
        omaflake: bool,
    },
    /// A file or folder; `omaf`: a program that answers `--version` with "omaf …" (Install replaces it), else Install
    /// refuses
    Other {
        omaf: bool,
    },
}

impl LinkState {
    /// omaflake's link is there (Uninstall)
    pub fn installed(&self) -> bool {
        matches!(self, LinkState::Symlink { omaflake: true, .. })
    }

    /// Something that is not omaflake's is in the way (Install refuses)
    pub fn blocked(&self) -> bool {
        matches!(self, LinkState::Other { omaf: false })
    }
}

/// The check's result: the Doctor's item and what Install/Uninstall would find
#[derive(Debug, Clone)]
pub struct CliCheck {
    pub item: Prerequisite,
    pub state: LinkState,
    /// The link's folder is on the login PATH
    pub on_path: bool,
}

impl CliLink {
    /// For this process: the running omaflake's `omaf`, the platform's link, the user's login shell
    pub fn current() -> Self {
        let home = home_dir();
        let mac = cfg!(target_os = "macos");
        CliLink { ours: Self::running_omaf(), link: Self::link_path(mac, &home), mac, home, shell: login_shell() }
    }

    /// `/usr/local/bin/omaf` on macOS, `~/.local/bin/omaf` elsewhere
    pub fn link_path(mac: bool, home: &Path) -> PathBuf {
        if mac { PathBuf::from("/usr/local/bin/omaf") } else { home.join(".local/bin/omaf") }
    }

    /// This omaflake's `omaf` (canonical), from the running executable
    pub fn running_omaf() -> Option<PathBuf> {
        let exe = std::env::current_exe().ok()?;
        let exe = fs::canonicalize(&exe).unwrap_or(exe);
        Self::omaf_for(&exe).map(|p| fs::canonicalize(&p).unwrap_or(p))
    }

    /// Inside an app bundle the bundled `<app>/Contents/MacOS/omaf`; else the executable itself when it is `omaf`, or
    /// an `omaf` next to it (the Linux app `omaflake` ships it in the same folder)
    pub(crate) fn omaf_for(exe: &Path) -> Option<PathBuf> {
        if let Some(app) = exe.ancestors().find(|a| a.extension().is_some_and(|e| e == "app")) {
            let bundled = app.join("Contents/MacOS/omaf");
            if is_executable_file(&bundled) {
                return Some(bundled);
            }
        }
        if exe.file_name().is_some_and(|n| n == "omaf") {
            return Some(exe.to_path_buf());
        }
        let sibling = exe.parent()?.join("omaf");
        is_executable_file(&sibling).then_some(sibling)
    }

    /// The command that installs it from a terminal (this omaflake's `omaf`, by its full path)
    pub fn install_command(&self) -> Option<String> {
        self.ours.as_ref().map(|o| format!("{} doctor --install-cli", sh_word(&o.to_string_lossy())))
    }

    // MARK: Check

    pub fn check(&self) -> CliCheck {
        let path = self.login_path();
        let found = Self::find_in(&path);
        let version = found.as_deref().and_then(version_of);
        let state = self.state();
        let on_path = self.link.parent().is_some_and(|dir| path.iter().any(|d| same_dir(d, dir)));
        let item = self.evaluate(found.as_deref(), version.as_deref(), &state, on_path);
        CliCheck { item, state, on_path }
    }

    /// The Doctor's item from what was found: ok when the first `omaf` on the PATH is this omaflake's (same file, or
    /// the same `--version`; on Linux also one from the Nix store – the package or a profile); limited when another
    /// one comes first, or omaflake's link is there but its folder is not on the PATH; missing when there is none.
    pub(crate) fn evaluate(
        &self,
        found: Option<&Path>,
        version: Option<&str>,
        state: &LinkState,
        on_path: bool,
    ) -> Prerequisite {
        let Some(found) = found else {
            if state.installed() && !on_path {
                let dir = self.link.parent().map(|d| self.tilde(d)).unwrap_or_default();
                return Prerequisite::new(
                    PrerequisiteItem::Cli,
                    PrerequisiteStatus::Limited,
                    &format!("{} – {dir} is not on your PATH", self.tilde(&self.link)),
                    &format!(
                        "Add {dir} to your PATH in your shell's profile: export PATH=\"{}:$PATH\"",
                        self.dollar_home(&dir)
                    ),
                    None,
                );
            }
            return Prerequisite::new(
                PrerequisiteItem::Cli,
                PrerequisiteStatus::Missing,
                "not on your PATH",
                "Optional: install the omaf command to use omaflake in a terminal",
                self.install_command(),
            );
        };
        let canonical = fs::canonicalize(found).unwrap_or_else(|_| found.to_path_buf());
        let same = self.ours.as_deref() == Some(canonical.as_path());
        let from_nix = !self.mac && canonical.starts_with("/nix/store/");
        let ours_version = format!("omaf {}", crate::VERSION);
        let ok = same || from_nix || version == Some(ours_version.as_str());
        Prerequisite::new(
            PrerequisiteItem::Cli,
            if ok { PrerequisiteStatus::Ok } else { PrerequisiteStatus::Limited },
            &format!("{} ({})", self.tilde(found), version.unwrap_or("version unknown")),
            if ok {
                ""
            } else {
                "Another omaf comes first on your PATH – install this omaflake's, or remove the other one"
            },
            if ok { None } else { self.install_command() },
        )
    }

    /// What is at the link's place now
    pub fn state(&self) -> LinkState {
        match fs::symlink_metadata(&self.link) {
            Err(_) => LinkState::Absent,
            Ok(m) if m.file_type().is_symlink() => {
                let target = fs::read_link(&self.link).unwrap_or_default();
                let omaflake = target.file_name().is_some_and(|n| n == "omaf");
                LinkState::Symlink { target, omaflake }
            }
            Ok(m) => {
                LinkState::Other { omaf: m.is_file() && version_of(&self.link).is_some_and(|v| v.starts_with("omaf ")) }
            }
        }
    }

    /// The folders of the login shell's PATH; the usual folders when the shell does not answer
    pub fn login_path(&self) -> Vec<PathBuf> {
        self.shell_path()
            .unwrap_or_else(|| Self::usual_dirs(Path::new("/etc/paths"), Path::new("/etc/paths.d"), &self.home))
    }

    /// The login shell's PATH: started interactively as a login shell with a clean environment, it prints `env`
    /// between two markers
    fn shell_path(&self) -> Option<Vec<PathBuf>> {
        let user = user_name();
        let mut cmd = Command::new(&self.shell);
        cmd.args(["-l", "-i", "-c", &format!("echo {START}; env; echo {END}")])
            .env_clear()
            .env("HOME", &self.home)
            .env("USER", &user)
            .env("LOGNAME", &user)
            .env("SHELL", &self.shell)
            .env("TERM", "dumb")
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .current_dir(&self.home);
        let (_, out) = run_with_timeout(cmd, SHELL_TIMEOUT)?;
        parse_env_path(&out)
    }

    /// /etc/paths, /etc/paths.d/* (macOS' path_helper), then where omaf usually is
    pub(crate) fn usual_dirs(etc_paths: &Path, etc_paths_d: &Path, home: &Path) -> Vec<PathBuf> {
        let mut files = vec![etc_paths.to_path_buf()];
        let mut extra: Vec<PathBuf> =
            fs::read_dir(etc_paths_d).map(|e| e.flatten().map(|e| e.path()).collect()).unwrap_or_default();
        extra.sort();
        files.extend(extra);
        let mut dirs: Vec<PathBuf> = files
            .iter()
            .filter_map(|f| fs::read_to_string(f).ok())
            .flat_map(|t| {
                t.lines().map(str::trim).filter(|l| l.starts_with('/')).map(PathBuf::from).collect::<Vec<_>>()
            })
            .collect();
        dirs.extend([
            PathBuf::from("/usr/local/bin"),
            PathBuf::from("/opt/homebrew/bin"),
            home.join(".local/bin"),
            home.join(".nix-profile/bin"),
            PathBuf::from("/run/current-system/sw/bin"),
        ]);
        dedupe(dirs)
    }

    /// The first executable `omaf` in these folders
    pub(crate) fn find_in(dirs: &[PathBuf]) -> Option<PathBuf> {
        dirs.iter().map(|d| d.join("omaf")).find(|p| is_executable_file(p))
    }

    // MARK: Install and uninstall

    /// macOS, as root: the folder, then the link (`ln -sfn` replaces a link or omaf file there)
    pub fn install_script(&self, ours: &Path) -> String {
        let dir = self.link.parent().unwrap_or(Path::new("/"));
        format!(
            "set -e; mkdir -p {}; ln -sfn {} {}",
            shell_quote(&dir.to_string_lossy()),
            shell_quote(&ours.to_string_lossy()),
            shell_quote(&self.link.to_string_lossy())
        )
    }

    /// macOS, as root: the link only – never a file
    pub fn uninstall_script(&self) -> String {
        let link = shell_quote(&self.link.to_string_lossy());
        format!("if [ -L {link} ]; then rm -f {link}; fi")
    }

    /// Creates the link. macOS: `as_root` runs `install_script` (the app: the administrator dialog, the CLI: sudo);
    /// elsewhere the link is made directly. Refuses a file that is not omaflake's. Returns what it did, in English.
    pub fn install(&self, as_root: &dyn Fn(&str) -> Result<()>) -> Result<String> {
        let ours = self.ours.as_deref().ok_or_else(|| {
            vm_err!(codes::PREREQUISITE, "This omaflake runs without its omaf – there is nothing to link to")
        })?;
        if self.state().blocked() {
            return Err(vm_err!(
                codes::EXISTS,
                "{} is there and is not omaflake's – remove or rename it first",
                self.link.to_string_lossy()
            ));
        }
        if self.mac {
            as_root(&self.install_script(ours))?;
        } else {
            self.link_directly(ours)?;
        }
        Ok(format!("omaf installed: {} → {}", self.link.to_string_lossy(), ours.to_string_lossy()))
    }

    /// Removes omaflake's link (only a link to an `omaf`)
    pub fn uninstall(&self, as_root: &dyn Fn(&str) -> Result<()>) -> Result<String> {
        if !self.state().installed() {
            return Err(vm_err!(codes::NOT_FOUND, "{} is not omaflake's link", self.link.to_string_lossy()));
        }
        if self.mac {
            as_root(&self.uninstall_script())?;
        } else {
            fs::remove_file(&self.link).with_path(&self.link)?;
        }
        Ok(format!("omaf removed: {}", self.link.to_string_lossy()))
    }

    /// Linux: `~/.local/bin` (created), a link or omaf file there replaced
    fn link_directly(&self, ours: &Path) -> Result<()> {
        if let Some(dir) = self.link.parent() {
            fs::create_dir_all(dir).with_path(dir)?;
        }
        if fs::symlink_metadata(&self.link).is_ok() {
            fs::remove_file(&self.link).with_path(&self.link)?;
        }
        std::os::unix::fs::symlink(ours, &self.link).with_path(&self.link)
    }

    // MARK: Paths as shown

    /// `~/…` for paths in the home folder
    fn tilde(&self, path: &Path) -> String {
        match path.strip_prefix(&self.home) {
            Ok(rest) if !self.home.as_os_str().is_empty() && self.home != Path::new("/") => {
                format!("~/{}", rest.to_string_lossy())
            }
            _ => path.to_string_lossy().into_owned(),
        }
    }

    /// `~/x` → `$HOME/x` (for a line in a shell profile)
    fn dollar_home(&self, shown: &str) -> String {
        match shown.strip_prefix("~/") {
            Some(rest) => format!("$HOME/{rest}"),
            None => shown.to_string(),
        }
    }
}

/// `<path> --version`'s first line when it ends well (a few seconds at most)
pub fn version_of(path: &Path) -> Option<String> {
    let mut cmd = Command::new(path);
    cmd.arg("--version");
    let (status, out) = run_with_timeout(cmd, VERSION_TIMEOUT)?;
    let line = out.lines().next().unwrap_or("").trim().to_string();
    (status == 0 && !line.is_empty()).then_some(line)
}

/// The PATH from `env`'s output between the markers (the last pair): its absolute folders, each once
pub(crate) fn parse_env_path(out: &str) -> Option<Vec<PathBuf>> {
    let lines: Vec<&str> = out.lines().collect();
    let start = lines.iter().rposition(|l| l.trim() == START)?;
    let end = start + lines[start..].iter().position(|l| l.trim() == END)?;
    let path = lines[start + 1..end].iter().find_map(|l| l.strip_prefix("PATH="))?;
    Some(dedupe(path.split(':').filter(|d| d.starts_with('/')).map(PathBuf::from).collect()))
}

fn dedupe(dirs: Vec<PathBuf>) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for d in dirs {
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

fn same_dir(a: &Path, b: &Path) -> bool {
    a == b || matches!((fs::canonicalize(a), fs::canonicalize(b)), (Ok(x), Ok(y)) if x == y)
}

/// A word for a shell line: as it is when it needs no quotes
fn sh_word(s: &str) -> String {
    if !s.is_empty() && s.chars().all(|c| c.is_ascii_alphanumeric() || "/._-+".contains(c)) {
        s.to_string()
    } else {
        shell_quote(s)
    }
}

/// Runs a command with no input and collects its output; None when it cannot start or takes longer than `timeout`
/// (then its process group is ended). Output a leftover child holds open is waited for briefly only.
fn run_with_timeout(mut cmd: Command, timeout: Duration) -> Option<(i32, String)> {
    use std::os::unix::process::CommandExt;
    cmd.stdin(Stdio::null()).stdout(Stdio::piped()).stderr(Stdio::null()).process_group(0);
    let mut child = cmd.spawn().ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    std::thread::spawn(move || {
        let mut bytes = Vec::new();
        let _ = stdout.read_to_end(&mut bytes);
        let _ = tx.send(String::from_utf8_lossy(&bytes).into_owned());
    });
    let deadline = Instant::now() + timeout;
    let pid = child.id() as libc::pid_t;
    let status = loop {
        match child.try_wait() {
            Ok(Some(s)) => break s,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            _ => {
                // SAFETY: signals the process group this command was started in (`process_group(0)`)
                unsafe {
                    libc::kill(-pid, libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                return None;
            }
        }
    };
    let out = rx.recv_timeout(Duration::from_millis(500)).ok()?;
    Some((status.code().unwrap_or(-1), out))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn script(path: &Path, body: &str) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, format!("#!/bin/sh\n{body}\n")).unwrap();
        fs::set_permissions(path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    /// A temporary home with `ours` (this version's fake omaf) and a fake shell that prints `path` as its PATH
    fn setup(mac: bool, path: &[&Path]) -> (tempfile::TempDir, CliLink) {
        let d = tempfile::tempdir().unwrap();
        let root = fs::canonicalize(d.path()).unwrap();
        let ours = root.join("omaflake.app/Contents/MacOS/omaf");
        script(&ours, &format!("echo 'omaf {}'", crate::VERSION));
        let shell = root.join("shell");
        let joined: Vec<String> = path.iter().map(|p| p.to_string_lossy().into_owned()).collect();
        script(
            &shell,
            &format!("echo 'noise from a profile'\necho {START}\necho 'PATH={}'\necho {END}", joined.join(":")),
        );
        let home = root.join("home");
        fs::create_dir_all(&home).unwrap();
        let link = if mac { root.join("usr/local/bin/omaf") } else { CliLink::link_path(false, &home) };
        (d, CliLink { ours: Some(ours), link, mac, home, shell })
    }

    fn root(d: &tempfile::TempDir) -> PathBuf {
        fs::canonicalize(d.path()).unwrap()
    }

    #[test]
    fn finds_ours_in_the_bundle_or_next_to_the_executable() {
        let d = tempfile::tempdir().unwrap();
        let r = root(&d);
        let bundled = r.join("omaflake.app/Contents/MacOS/omaf");
        script(&bundled, "true");
        assert_eq!(CliLink::omaf_for(&r.join("omaflake.app/Contents/MacOS/omaflake")), Some(bundled.clone()));
        assert_eq!(CliLink::omaf_for(&bundled), Some(bundled));
        let bin = r.join("store/bin");
        script(&bin.join("omaf"), "true");
        assert_eq!(CliLink::omaf_for(&bin.join("omaflake")), Some(bin.join("omaf")));
        assert_eq!(CliLink::omaf_for(&r.join("target/debug/deps/omaflake_core-1")), None);
        assert_eq!(CliLink::omaf_for(&r.join("x/omaf")), Some(r.join("x/omaf")));
    }

    #[test]
    fn link_paths() {
        assert_eq!(CliLink::link_path(true, Path::new("/h")), Path::new("/usr/local/bin/omaf"));
        assert_eq!(CliLink::link_path(false, Path::new("/home/u")), Path::new("/home/u/.local/bin/omaf"));
    }

    #[test]
    fn parses_the_login_shells_path() {
        let out = format!("hello\n{START}\nHOME=/h\nPATH=/a:rel:/b:/a\nX=1\n{END}\n");
        assert_eq!(parse_env_path(&out), Some(vec![PathBuf::from("/a"), PathBuf::from("/b")]));
        assert_eq!(parse_env_path("PATH=/a\n"), None);
        assert_eq!(parse_env_path(&format!("{START}\nX=1\n{END}")), None);
    }

    #[test]
    fn ok_when_the_first_omaf_is_ours() {
        let d = tempfile::tempdir().unwrap();
        let first = root(&d).join("first");
        let (_d, mut cli) = setup(true, &[&first]);
        fs::create_dir_all(&first).unwrap();
        std::os::unix::fs::symlink(cli.ours.as_ref().unwrap(), first.join("omaf")).unwrap();
        let c = cli.check();
        assert_eq!(c.item.status, PrerequisiteStatus::Ok, "{:?}", c.item);
        assert_eq!(c.item.detail, format!("{} (omaf {})", first.join("omaf").display(), crate::VERSION));
        assert_eq!((c.item.install.as_deref(), c.item.required()), (None, false));
        // another file with the same version counts too
        fs::remove_file(first.join("omaf")).unwrap();
        script(&first.join("omaf"), &format!("echo 'omaf {}'", crate::VERSION));
        assert_eq!(cli.check().item.status, PrerequisiteStatus::Ok);
        // even without knowing our own omaf
        cli.ours = None;
        assert_eq!(cli.check().item.status, PrerequisiteStatus::Ok);
    }

    #[test]
    fn limited_when_another_omaf_comes_first() {
        let d = tempfile::tempdir().unwrap();
        let (a, b) = (root(&d).join("a"), root(&d).join("b"));
        let (_d, cli) = setup(true, &[&a, &b]);
        script(&a.join("omaf"), "echo 'omaf 0.0.1'");
        script(&b.join("omaf"), &format!("echo 'omaf {}'", crate::VERSION));
        let c = cli.check();
        assert_eq!(c.item.status, PrerequisiteStatus::Limited);
        assert_eq!(c.item.detail, format!("{} (omaf 0.0.1)", a.join("omaf").display()));
        assert!(c.item.hint.starts_with("Another omaf comes first"));
        let ours = cli.ours.as_ref().unwrap().to_string_lossy().into_owned();
        assert_eq!(c.item.install, Some(format!("{ours} doctor --install-cli")));
        // a program that does not answer --version
        script(&a.join("omaf"), "exit 3");
        assert_eq!(cli.check().item.detail, format!("{} (version unknown)", a.join("omaf").display()));
    }

    #[test]
    fn missing_when_there_is_none() {
        let d = tempfile::tempdir().unwrap();
        let empty = root(&d).join("empty");
        let (_d, cli) = setup(true, &[&empty]);
        let c = cli.check();
        assert_eq!((c.item.status, c.item.detail.as_str()), (PrerequisiteStatus::Missing, "not on your PATH"));
        assert_eq!(c.state, LinkState::Absent);
        assert!(c.item.install.unwrap().ends_with("/omaflake.app/Contents/MacOS/omaf doctor --install-cli"));
        // a non-executable omaf does not count
        fs::create_dir_all(&empty).unwrap();
        fs::write(empty.join("omaf"), "").unwrap();
        assert_eq!(cli.check().item.status, PrerequisiteStatus::Missing);
    }

    #[test]
    fn falls_back_to_the_usual_folders() {
        let d = tempfile::tempdir().unwrap();
        let r = root(&d);
        fs::write(r.join("paths"), "/usr/bin\n/bin\n\n").unwrap();
        fs::create_dir_all(r.join("paths.d")).unwrap();
        fs::write(r.join("paths.d/b"), "/opt/b\n").unwrap();
        fs::write(r.join("paths.d/a"), "/opt/a\n/usr/bin\n").unwrap();
        let dirs = CliLink::usual_dirs(&r.join("paths"), &r.join("paths.d"), Path::new("/home/u"));
        let shown: Vec<String> = dirs.iter().map(|d| d.to_string_lossy().into_owned()).collect();
        assert_eq!(
            shown,
            [
                "/usr/bin",
                "/bin",
                "/opt/a",
                "/opt/b",
                "/usr/local/bin",
                "/opt/homebrew/bin",
                "/home/u/.local/bin",
                "/home/u/.nix-profile/bin",
                "/run/current-system/sw/bin"
            ]
        );
        // a shell that does not answer (or does not exist) leads there
        let (_d, mut cli) = setup(false, &[]);
        cli.shell = r.join("no-shell");
        assert!(cli.login_path().contains(&cli.home.join(".local/bin")));
    }

    #[test]
    fn a_hanging_shell_times_out() {
        let d = tempfile::tempdir().unwrap();
        let slow = root(&d).join("slow");
        script(&slow, "sleep 30");
        let start = Instant::now();
        assert_eq!(run_with_timeout(Command::new(&slow), Duration::from_millis(300)), None);
        assert!(start.elapsed() < Duration::from_secs(5));
    }

    #[test]
    fn linux_links_into_local_bin_and_names_a_missing_path() {
        let d = tempfile::tempdir().unwrap();
        let elsewhere = root(&d).join("elsewhere");
        let (_d, cli) = setup(false, &[&elsewhere]);
        assert!(cli.link.ends_with(".local/bin/omaf") && cli.link.starts_with(&cli.home));
        let never = |_: &str| -> Result<()> { panic!("no root on Linux") };
        let done = cli.install(&never).unwrap();
        assert!(done.starts_with("omaf installed: "));
        assert_eq!(fs::read_link(&cli.link).unwrap(), *cli.ours.as_ref().unwrap());
        // ~/.local/bin is not on the PATH: the detail says how to add it
        let c = cli.check();
        assert!(c.state.installed() && !c.on_path);
        assert_eq!(c.item.status, PrerequisiteStatus::Limited);
        assert_eq!(c.item.detail, "~/.local/bin/omaf – ~/.local/bin is not on your PATH");
        assert_eq!(
            c.item.hint,
            "Add ~/.local/bin to your PATH in your shell's profile: export PATH=\"$HOME/.local/bin:$PATH\""
        );
        // with it on the PATH: ok
        let local = cli.home.join(".local/bin");
        let (_e, mut on) = setup(false, &[&local]);
        on.home = cli.home.clone();
        on.link = cli.link.clone();
        on.ours = cli.ours.clone();
        let c = on.check();
        assert_eq!((c.item.status, c.on_path), (PrerequisiteStatus::Ok, true));
        // installing again replaces the link; uninstall removes only it
        cli.install(&never).unwrap();
        assert_eq!(cli.uninstall(&never).unwrap(), format!("omaf removed: {}", cli.link.display()));
        assert!(fs::symlink_metadata(&cli.link).is_err() && cli.ours.as_ref().unwrap().exists());
        assert_eq!(cli.uninstall(&never).unwrap_err().code(), Some(codes::NOT_FOUND));
    }

    #[test]
    fn linux_nix_store_omaf_is_ok() {
        let cli = CliLink {
            ours: Some("/x/omaf".into()),
            link: "/h/.local/bin/omaf".into(),
            mac: false,
            home: "/h".into(),
            shell: "/bin/sh".into(),
        };
        // NixOS with the package: /run/current-system/sw/bin/omaf → /nix/store/…/bin/omaf, any version
        let found = Path::new("/nix/store/0000-omaflake-0.0.1/bin/omaf");
        let item = cli.evaluate(Some(found), Some("omaf 0.0.1"), &LinkState::Absent, false);
        assert_eq!((item.status, item.install), (PrerequisiteStatus::Ok, None));
        let mac = CliLink { mac: true, ..cli.clone() };
        let item = mac.evaluate(Some(found), Some("omaf 0.0.1"), &LinkState::Absent, false);
        assert_eq!(item.status, PrerequisiteStatus::Limited);
    }

    #[test]
    fn refuses_a_file_that_is_not_omaflakes() {
        let (_d, cli) = setup(false, &[]);
        script(&cli.link, "echo 'something else 1.0'");
        assert_eq!(cli.state(), LinkState::Other { omaf: false });
        let never = |_: &str| -> Result<()> { panic!("not reached") };
        let e = cli.install(&never).unwrap_err();
        assert_eq!(e.code(), Some(codes::EXISTS));
        assert!(e.message().ends_with("is there and is not omaflake's – remove or rename it first"));
        assert!(cli.uninstall(&never).is_err());
        // an omaf file (an older install) is replaced
        script(&cli.link, "echo 'omaf 0.0.1'");
        assert_eq!(cli.state(), LinkState::Other { omaf: true });
        cli.install(&never).unwrap();
        assert!(cli.state().installed());
    }

    #[test]
    fn macos_runs_the_script_as_root() {
        let (_d, cli) = setup(true, &[]);
        let ours = cli.ours.clone().unwrap();
        let script = cli.install_script(&ours);
        let dir = cli.link.parent().unwrap().to_string_lossy().into_owned();
        assert_eq!(script, format!("set -e; mkdir -p '{dir}'; ln -sfn '{}' '{}'", ours.display(), cli.link.display()));
        let real = CliLink { link: CliLink::link_path(true, Path::new("/h")), ..cli.clone() };
        assert_eq!(
            real.install_script(Path::new("/Applications/omaflake.app/Contents/MacOS/omaf")),
            "set -e; mkdir -p '/usr/local/bin'; ln -sfn '/Applications/omaflake.app/Contents/MacOS/omaf' \
             '/usr/local/bin/omaf'"
        );
        assert_eq!(real.uninstall_script(), "if [ -L '/usr/local/bin/omaf' ]; then rm -f '/usr/local/bin/omaf'; fi");
        // the script goes to `as_root` (the administrator dialog / sudo); nothing is linked here
        let seen = std::sync::Mutex::new(Vec::new());
        let root = |s: &str| -> Result<()> {
            seen.lock().unwrap().push(s.to_string());
            Ok(())
        };
        cli.install(&root).unwrap();
        assert_eq!(*seen.lock().unwrap(), [script]);
        assert_eq!(cli.state(), LinkState::Absent);
        let cancelled = |_: &str| -> Result<()> { Err(crate::Error::cancelled()) };
        assert!(cli.install(&cancelled).unwrap_err().is_cancelled());
        // uninstall only with omaflake's link there
        assert!(cli.uninstall(&root).is_err());
        fs::create_dir_all(cli.link.parent().unwrap()).unwrap();
        std::os::unix::fs::symlink(&ours, &cli.link).unwrap();
        cli.uninstall(&root).unwrap();
        assert_eq!(seen.lock().unwrap().last().unwrap(), &cli.uninstall_script());
    }

    #[test]
    fn quotes_only_when_needed() {
        assert_eq!(
            sh_word("/Applications/omaflake.app/Contents/MacOS/omaf"),
            "/Applications/omaflake.app/Contents/MacOS/omaf"
        );
        assert_eq!(sh_word("/a b/omaf"), "'/a b/omaf'");
    }
}
