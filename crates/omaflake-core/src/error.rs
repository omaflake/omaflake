// SPDX-License-Identifier: AGPL-3.0-or-later
//! The crate's one error type – the port of Swift's `VMError` (Models.swift).
//!
//! An error is a message for people (`Display`) and, where the API needs to tell errors apart, a machine-readable
//! `code` – the `data.kind` of docs/api.md §3 (kebab-case). Swift's `String(localized:)` texts are plain English here;
//! localization moves to the UIs, which can key their own texts on `code` (OPEN: most errors still have none – each
//! work package gives its errors the kinds api.md lists; a new kind goes into api.md and `codes` together).

use std::fmt;

/// Error codes (`Error::code`) = the error kinds of docs/api.md §3. Keep them stable: UIs and the JSON-RPC API rely
/// on them.
pub mod codes {
    /// A VM, image, recipe or file that is not there
    pub const NOT_FOUND: &str = "not-found";
    /// Something of that name is there already
    pub const EXISTS: &str = "exists";
    /// The VM runs – stop it first
    pub const VM_RUNNING: &str = "vm-running";
    /// The VM is stopped – start it first
    pub const VM_STOPPED: &str = "vm-stopped";
    /// Used by something else: a lock someone holds (`FileLock`, `InstanceLock`), an image VMs are built on, a port
    pub const IN_USE: &str = "in-use";
    /// At most two macOS VMs at a time (`VzBackend::MAX_RUNNING`, plan BM16)
    pub const MAC_LIMIT: &str = "mac-limit";
    /// Built for another system (`Platform::not_here`), or a feature this host does not have (yet)
    pub const UNSUPPORTED_SYSTEM: &str = "unsupported-system";
    /// A file this omaflake cannot read: too old (below `FileFormat::minimum`) or damaged
    pub const UNREADABLE_FILE: &str = "unreadable-file";
    /// A file of a newer omaflake (above `FileFormat::current`)
    pub const NEWER_FILE: &str = "newer-file";
    /// The work was cancelled (`Cancellation::check`, a build, a download, macOS' administrator dialog)
    pub const CANCELLED: &str = "cancelled";
    /// Something the Mac lacks (Prerequisites)
    pub const PREREQUISITE: &str = "prerequisite";
    /// A download or a source could not be reached
    pub const NETWORK: &str = "network";
    /// A token or password was refused
    pub const AUTH: &str = "auth";
    /// A bug or an unexpected state
    pub const INTERNAL: &str = "internal";
}

/// omaflake's error: a message (Swift `VMError.description`) and an optional code for the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    message: String,
    code: Option<&'static str>,
}

/// `Result` with the crate's `Error`
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// An error with a message (Swift `VMError("…")`)
    pub fn new(message: impl Into<String>) -> Self {
        Error { message: message.into(), code: None }
    }

    /// An error with a message and a machine-readable code (see `codes`)
    pub fn with_code(code: &'static str, message: impl Into<String>) -> Self {
        Error { message: message.into(), code: Some(code) }
    }

    /// "Cancelled" (Swift `Cancellation.check`)
    pub fn cancelled() -> Self {
        Self::with_code(codes::CANCELLED, "Cancelled")
    }

    /// A feature that needs an API this host does not have (macOS-only code on Linux)
    pub fn unsupported(what: &str) -> Self {
        Self::with_code(codes::UNSUPPORTED_SYSTEM, format!("{what} is not supported on this host yet – OPEN"))
    }

    pub fn message(&self) -> &str {
        &self.message
    }

    pub fn code(&self) -> Option<&'static str> {
        self.code
    }

    pub fn is_cancelled(&self) -> bool {
        self.code == Some(codes::CANCELLED)
    }

    /// The same error with `prefix` in front of its message (keeps the code)
    pub fn context(self, prefix: impl fmt::Display) -> Self {
        Error { message: format!("{prefix}{}", self.message), code: self.code }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for Error {}

/// `vm_err!("Instance \"{}\" does not exist", name)` – an `Error` from a format string (Swift `VMError("…")`)
#[macro_export]
macro_rules! vm_err {
    ($($arg:tt)*) => {
        $crate::error::Error::new(format!($($arg)*))
    };
}

/// The text of an OS error without Rust's " (os error N)" suffix – what Swift got from `strerror(errno)`
pub fn os_text(e: &std::io::Error) -> String {
    match e.raw_os_error() {
        Some(code) => strerror(code),
        None => e.to_string(),
    }
}

/// `strerror(code)` as a String
pub fn strerror(code: i32) -> String {
    let text = std::io::Error::from_raw_os_error(code).to_string();
    match text.rfind(" (os error ") {
        Some(i) => text[..i].to_string(),
        None => text,
    }
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::new(os_text(&e))
    }
}

impl From<serde_json::Error> for Error {
    fn from(e: serde_json::Error) -> Self {
        Error::new(format!("JSON: {e}"))
    }
}

impl From<toml::de::Error> for Error {
    fn from(e: toml::de::Error) -> Self {
        Error::new(format!("TOML: {e}"))
    }
}

impl From<ureq::Error> for Error {
    fn from(e: ureq::Error) -> Self {
        Error::new(format!("HTTP: {e}"))
    }
}

/// Adds the path to I/O errors: `fs::read(&p).with_path(&p)?` → "<path>: No such file or directory"
pub trait IoContext<T> {
    fn with_path(self, path: &std::path::Path) -> Result<T>;
}

impl<T> IoContext<T> for std::io::Result<T> {
    fn with_path(self, path: &std::path::Path) -> Result<T> {
        self.map_err(|e| Error::new(format!("{}: {}", path.display(), os_text(&e))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn display_is_the_message_and_codes_stay() {
        let e = vm_err!("Instance \"{}\" does not exist", "a");
        assert_eq!(e.to_string(), "Instance \"a\" does not exist");
        assert_eq!(e.code(), None);
        assert!(Error::cancelled().is_cancelled());
        assert_eq!(Error::cancelled().context("x: ").code(), Some(codes::CANCELLED));
        let io: Error = std::io::Error::from_raw_os_error(libc::ENOENT).into();
        assert_eq!(io.message(), "No such file or directory");
    }
}
