// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Platform.swift, plus the host facts Foundation gave the Swift code for free (`NSUserName()`,
//! `NSFullUserName()`, the home folder, `~` expansion).

use crate::vm_err;
use std::ffi::CStr;
use std::path::{Path, PathBuf};

use crate::error::Error;
use crate::models::GuestKind;

/// Architectures, named like Nix names systems (plan B114): what a recipe can be built for (`systems` in recipe.toml),
/// what an image was built for and what a VM runs as (`system` in image.json and instance.json).
pub enum Platform {}

impl Platform {
    /// The systems omaflake knows per guest kind
    pub fn systems(kind: GuestKind) -> &'static [&'static str] {
        match kind {
            GuestKind::Nixos => &["aarch64-linux", "x86_64-linux"],
            GuestKind::Macos => &["aarch64-darwin"],
        }
    }

    /// The system this host builds and runs guests of `kind` as; None = that kind does not run here
    pub fn host_system(kind: GuestKind) -> Option<&'static str> {
        #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
        {
            Some(match kind {
                GuestKind::Nixos => "aarch64-linux",
                GuestKind::Macos => "aarch64-darwin",
            })
        }
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            (kind == GuestKind::Nixos).then_some("x86_64-linux")
        }
        #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
        {
            (kind == GuestKind::Nixos).then_some("aarch64-linux")
        }
        #[cfg(not(any(
            all(target_os = "macos", target_arch = "aarch64"),
            all(target_os = "linux", target_arch = "x86_64"),
            all(target_os = "linux", target_arch = "aarch64")
        )))]
        {
            let _ = kind;
            None
        }
    }

    /// The system of a file or recipe from before B114, which has none: everything then ran on a Mac with Apple silicon
    pub fn legacy_system(kind: GuestKind) -> &'static str {
        if kind == GuestKind::Macos { "aarch64-darwin" } else { "aarch64-linux" }
    }

    /// A guest of `system` runs on this host
    pub fn runs_here(system: &str) -> bool {
        GuestKind::ALL.iter().any(|k| Self::host_system(*k) == Some(system))
    }

    /// "aarch64-linux" → "aarch64" (the part shown next to a recipe or image)
    pub fn architecture(system: &str) -> &str {
        system.split('-').find(|s| !s.is_empty()).unwrap_or(system)
    }

    /// The error for a guest that does not run here
    pub fn not_here(what: &str, system: &str) -> Error {
        vm_err!(
            crate::error::codes::UNSUPPORTED_SYSTEM,
            "{what} is built for {system} – this computer runs {}",
            Self::host_description()
        )
    }

    /// What runs here, for messages ("aarch64-linux, aarch64-darwin")
    pub fn host_description() -> String {
        let systems: Vec<&str> = GuestKind::ALL.iter().filter_map(|k| Self::host_system(*k)).collect();
        if systems.is_empty() { "no guests".into() } else { systems.join(", ") }
    }
}

/// The passwd entry of the user running omaflake: (name, full name, home, login shell)
fn passwd() -> Option<(String, String, String, String)> {
    let mut buf = vec![0 as libc::c_char; 16384];
    // SAFETY: zeroed passwd is a valid out-parameter; getpwuid_r writes into it and `buf`, which outlive the reads below
    unsafe {
        let mut pw: libc::passwd = std::mem::zeroed();
        let mut result: *mut libc::passwd = std::ptr::null_mut();
        let rc = libc::getpwuid_r(libc::getuid(), &mut pw, buf.as_mut_ptr(), buf.len(), &mut result);
        if rc != 0 || result.is_null() {
            return None;
        }
        let s = |p: *const libc::c_char| {
            if p.is_null() { String::new() } else { CStr::from_ptr(p).to_string_lossy().into_owned() }
        };
        Some((s(pw.pw_name), s(pw.pw_gecos), s(pw.pw_dir), s(pw.pw_shell)))
    }
}

/// The login name of the user running omaflake (Swift `NSUserName()`)
pub fn user_name() -> String {
    passwd()
        .map(|p| p.0)
        .filter(|n| !n.is_empty())
        .or_else(|| std::env::var("USER").ok())
        .unwrap_or_else(|| "user".into())
}

/// The full name of the user running omaflake (Swift `NSFullUserName()`): the passwd entry's GECOS name.
/// OPEN: macOS' `NSFullUserName` reads the directory service's RealName, which GECOS mirrors for local users.
pub fn full_user_name() -> String {
    passwd()
        .map(|p| p.1.split(',').next().unwrap_or("").to_string())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(user_name)
}

/// The user's home folder (Swift `FileManager.homeDirectoryForCurrentUser`): `$HOME`, else the passwd entry
pub fn home_dir() -> PathBuf {
    if let Some(h) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
        return PathBuf::from(h);
    }
    PathBuf::from(passwd().map(|p| p.2).unwrap_or_else(|| "/".into()))
}

/// The user's login shell: `$SHELL`, else the passwd entry (a launchd agent may have no `SHELL`), else /bin/sh
pub fn login_shell() -> PathBuf {
    std::env::var_os("SHELL")
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .or_else(|| passwd().map(|p| PathBuf::from(p.3)).filter(|p| p.is_absolute()))
        .unwrap_or_else(|| PathBuf::from("/bin/sh"))
}

/// `~` and `~/…` → the home folder (Swift `NSString.expandingTildeInPath`; `~user` is left as it is)
pub fn expand_tilde(path: &str) -> String {
    if path == "~" {
        return home_dir().to_string_lossy().into_owned();
    }
    if let Some(rest) = path.strip_prefix("~/") {
        return home_dir().join(rest).to_string_lossy().into_owned();
    }
    path.to_string()
}

/// The home folder as `~` (Swift `NSString.abbreviatingWithTildeInPath`)
pub fn abbreviate_tilde(path: &str) -> String {
    let home = home_dir();
    let home = home.to_string_lossy();
    if path == home {
        return "~".into();
    }
    match path.strip_prefix(&format!("{home}/")) {
        Some(rest) => format!("~/{rest}"),
        None => path.to_string(),
    }
}

/// `path` with `.` and `..` removed lexically (Swift `URL.standardizedFileURL` for absolute paths)
pub fn standardize(path: &Path) -> PathBuf {
    use std::path::Component;
    let mut out = PathBuf::new();
    for c in path.components() {
        match c {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() {
                    out.push("..");
                }
            }
            other => out.push(other.as_os_str()),
        }
    }
    out
}

/// Symbolic links resolved as far as the path exists, the rest kept (Swift `URL.resolvingSymlinksInPath()`), then
/// standardized. Like Swift, a leading `/private` is dropped when the path without it exists (macOS' /tmp, /var).
/// OPEN: Swift decides that on the whole path; here on the part that exists (consistent for files not made yet).
pub fn resolve_symlinks(path: &Path) -> PathBuf {
    let path = standardize(path);
    let mut existing = path.clone();
    let mut rest: Vec<std::ffi::OsString> = Vec::new();
    let resolved = loop {
        if let Ok(c) = std::fs::canonicalize(&existing) {
            // Like Swift, a leading `/private` goes when the path without it exists (macOS' /tmp, /var) – decided on
            // the part that exists, so a path and a file below it that does not exist yet resolve alike
            if let Ok(stripped) = c.strip_prefix("/private") {
                let candidate = Path::new("/").join(stripped);
                if candidate.exists() {
                    break candidate;
                }
            }
            break c;
        }
        match (existing.file_name().map(|n| n.to_os_string()), existing.parent().map(Path::to_path_buf)) {
            (Some(name), Some(parent)) => {
                rest.push(name);
                existing = parent;
            }
            _ => return path,
        }
    };
    let mut out = resolved;
    for name in rest.into_iter().rev() {
        out.push(name);
    }
    out
}

/// `access(path, X_OK)` and not a folder (Swift `FileManager.isExecutableFile(atPath:)`)
pub fn is_executable_file(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(meta) = std::fs::metadata(path) else { return false };
    if meta.is_dir() {
        return false;
    }
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: plain libc call with a valid NUL-terminated path
    unsafe { libc::access(c.as_ptr(), libc::X_OK) == 0 }
}

/// `access(path, W_OK)` (Swift `FileManager.isWritableFile(atPath:)`)
pub fn is_writable(path: &Path) -> bool {
    use std::os::unix::ffi::OsStrExt;
    let Ok(c) = std::ffi::CString::new(path.as_os_str().as_bytes()) else { return false };
    // SAFETY: plain libc call with a valid NUL-terminated path
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// `uname -m` ("arm64" on Apple silicon, "x86_64", "aarch64" on Linux) – Swift `Prerequisites.machine()`
pub fn machine() -> String {
    // SAFETY: zeroed utsname is a valid out-parameter for uname
    unsafe {
        let mut u: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut u) != 0 {
            return String::new();
        }
        CStr::from_ptr(u.machine.as_ptr()).to_string_lossy().into_owned()
    }
}

/// `uname -r` (the kernel release, e.g. "6.12.1" on Linux)
pub fn kernel_release() -> String {
    // SAFETY: zeroed utsname is a valid out-parameter for uname
    unsafe {
        let mut u: libc::utsname = std::mem::zeroed();
        if libc::uname(&mut u) != 0 {
            return String::new();
        }
        CStr::from_ptr(u.release.as_ptr()).to_string_lossy().into_owned()
    }
}

/// A system tool: on macOS its fixed path (`/usr/bin/ssh` – not whatever comes first in PATH, as in Swift); elsewhere
/// its name, found in PATH – NixOS has no /usr/bin/ssh, /bin/chmod or /usr/bin/sudo (`/run/wrappers/bin`, profiles)
pub fn host_tool(macos_path: &str) -> String {
    if cfg!(target_os = "macos") {
        macos_path.to_string()
    } else {
        Path::new(macos_path).file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn architectures() {
        assert_eq!(Platform::architecture("x86_64-linux"), "x86_64");
        assert_eq!(Platform::architecture("aarch64-darwin"), "aarch64");
        assert_eq!(Platform::legacy_system(GuestKind::Macos), "aarch64-darwin");
        assert_eq!(Platform::legacy_system(GuestKind::Nixos), "aarch64-linux");
        let foreign = if Platform::host_system(GuestKind::Nixos) == Some("x86_64-linux") {
            "aarch64-linux"
        } else {
            "x86_64-linux"
        };
        assert!(!Platform::runs_here(foreign));
        if let Some(own) = Platform::host_system(GuestKind::Nixos) {
            assert!(Platform::runs_here(own));
        }
        assert!(Platform::not_here("x", "y").message().starts_with("x is built for y – this computer runs "));
    }

    #[test]
    fn user_and_paths() {
        assert!(!user_name().is_empty());
        assert_eq!(expand_tilde("~/a"), home_dir().join("a").to_string_lossy());
        assert_eq!(expand_tilde("/a/~"), "/a/~");
        assert_eq!(abbreviate_tilde(&home_dir().join("x").to_string_lossy()), "~/x");
        assert_eq!(standardize(Path::new("/a/./b/../c")), PathBuf::from("/a/c"));
        assert_eq!(
            resolve_symlinks(Path::new("/nonexistent-omaflake/x/../y")),
            PathBuf::from("/nonexistent-omaflake/y")
        );
        assert!(is_executable_file(Path::new("/bin/sh")));
        assert!(!is_executable_file(Path::new("/bin")));
    }
}
