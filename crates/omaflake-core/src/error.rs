// SPDX-License-Identifier: AGPL-3.0-or-later
//! The crate's one error type – the port of Swift's `VMError` (Models.swift).
//!
//! An error is a message for people (`Display`) and, where the API needs to tell errors apart, a machine-readable
//! `code` – the `data.kind` of docs/api.md §3 (kebab-case). The message is English and a **text** (docs/api.md §10.2,
//! plan B125): an error made with `vm_err!` keeps its format string as the key (`{}` → `%@`) and the values it was
//! filled with as args, so the interfaces show it in their language (`data.text` of api.md §3). `Error::new` with a
//! ready string has no key (shown as it is). A new kind goes into api.md and `codes` together.

use std::fmt;

use crate::api::views::text::{Arg, Text};

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

    /// Every kind above
    pub const ALL: [&str; 14] = [
        NOT_FOUND,
        EXISTS,
        VM_RUNNING,
        VM_STOPPED,
        IN_USE,
        MAC_LIMIT,
        UNSUPPORTED_SYSTEM,
        UNREADABLE_FILE,
        NEWER_FILE,
        CANCELLED,
        PREREQUISITE,
        NETWORK,
        AUTH,
        INTERNAL,
    ];

    /// The kind of this name (an API client reading `data.kind`); None for one this omaflake does not know
    pub fn known(kind: &str) -> Option<&'static str> {
        ALL.into_iter().find(|k| *k == kind)
    }
}

/// omaflake's error: a message (Swift `VMError.description`) as a text, and an optional code for the API.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Error {
    text: Text,
    code: Option<&'static str>,
}

/// Between the values of `vm_err!`'s joined string (omaflake-macros' `SEPARATOR`)
const SEPARATOR: &str = "\u{1f}\u{1e}\u{1f}";

/// `Result` with the crate's `Error`
pub type Result<T> = std::result::Result<T, Error>;

impl Error {
    /// An error with a message (Swift `VMError("…")`)
    pub fn new(message: impl Into<String>) -> Self {
        Error { text: Text::verbatim(message), code: None }
    }

    /// An error with a message and a machine-readable code (see `codes`)
    pub fn with_code(code: &'static str, message: impl Into<String>) -> Self {
        Error { text: Text::verbatim(message), code: Some(code) }
    }

    /// An error of a text (its key translated by the interfaces) and an optional code
    pub fn from_text(code: Option<&'static str>, text: Text) -> Self {
        Error { text, code }
    }

    /// What `vm_err!` expands to: the format string's key, how many values it has and the values formatted as the
    /// string says, joined by `SEPARATOR`. The message is the key filled in – the same as `format!` would give.
    #[doc(hidden)]
    pub fn __keyed(code: Option<&'static str>, key: &'static str, count: usize, joined: String) -> Self {
        let values: Vec<String> =
            if count == 0 { Vec::new() } else { joined.split(SEPARATOR).map(str::to_string).collect() };
        if values.len() != count {
            // A value with the separator in it (never in practice): the message without a key
            return Error { text: Text::verbatim(joined.replace(SEPARATOR, "")), code };
        }
        let text = Text::fmt(key, values.into_iter().map(Arg::Str).collect());
        Error { text, code }
    }

    /// "Cancelled" (Swift `Cancellation.check`)
    pub fn cancelled() -> Self {
        crate::vm_err!(codes::CANCELLED, "Cancelled")
    }

    /// A feature that needs an API this host does not have (macOS-only code on Linux)
    pub fn unsupported(what: &str) -> Self {
        crate::vm_err!(codes::UNSUPPORTED_SYSTEM, "{what} is not supported on this host yet – OPEN")
    }

    pub fn message(&self) -> &str {
        &self.text.text
    }

    /// The message as a text: its key (None: shown as it is), the English text and the args (docs/api.md §3)
    pub fn text(&self) -> &Text {
        &self.text
    }

    /// The key of the message (`vm_err!`'s format string, `{}` as `%@`)
    pub fn key(&self) -> Option<&str> {
        self.text.key.as_deref()
    }

    pub fn code(&self) -> Option<&'static str> {
        self.code
    }

    pub fn is_cancelled(&self) -> bool {
        self.code == Some(codes::CANCELLED)
    }

    /// The same error with `prefix` (data, e.g. a path) in front of its message (keeps the code and the key)
    pub fn context(self, prefix: impl fmt::Display) -> Self {
        let text = match self.text.key {
            Some(_) => Text::fmt("%@%@", vec![Arg::Str(prefix.to_string()), Arg::from(self.text)]),
            None => Text::verbatim(format!("{prefix}{}", self.text.text)),
        };
        Error { text, code: self.code }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.text.text)
    }
}

impl std::error::Error for Error {}

/// `vm_err!("Instance \"{}\" does not exist", name)` – an `Error` from a format string (Swift `VMError("…")`);
/// `vm_err!(codes::NOT_FOUND, "No such folder: {path}")` with a code. The format string is the message's key
/// (`{…}` → `%@`, translated in `de.lproj/Localizable.strings`), the values its args (omaflake-macros).
#[macro_export]
macro_rules! vm_err {
    ($($arg:tt)*) => {
        $crate::error::__keyed_error!(($crate) $($arg)*)
    };
}

#[doc(hidden)]
pub use omaflake_macros::keyed_error as __keyed_error;

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
    /// The OS's text is the key ("No such file or directory"): the common ones are translated
    fn from(e: std::io::Error) -> Self {
        Error { text: Text::new(&os_text(&e)), code: None }
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
        self.map_err(|e| {
            let os = Text::new(&os_text(&e));
            Error::from_text(None, Text::fmt("%@: %@", vec![Arg::Str(path.display().to_string()), Arg::from(os)]))
        })
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
        assert_eq!(io.key(), Some("No such file or directory"));
    }

    #[test]
    fn keys_and_args_of_format_strings() {
        let name = "research";
        let e = vm_err!(codes::NOT_FOUND, "{name}: {:?} – {{x}} at {}%", "b", 5);
        assert_eq!(e.message(), "research: \"b\" – {x} at 5%");
        assert_eq!(e.key(), Some("%@: %@ – {x} at %@%%"));
        assert_eq!(e.text().args, vec![Arg::from("research"), Arg::from("\"b\""), Arg::from("5")]);
        assert_eq!(e.code(), Some(codes::NOT_FOUND));
        let plain = vm_err!("Cancelled");
        assert_eq!((plain.message(), plain.key(), plain.text().args.len()), ("Cancelled", Some("Cancelled"), 0));
        let width = 6;
        assert_eq!(vm_err!("[{:>width$}]", "ab").message(), "[    ab]");
        assert_eq!(vm_err!("{0}-{0}", 1).message(), "1-1");
        assert_eq!(vm_err!("{a}", a = 2,).message(), "2");
        // Context keeps the key: the prefix is data
        let c = Error::cancelled().context("x: ");
        assert_eq!((c.message(), c.key()), ("x: Cancelled", Some("%@%@")));
        let p = std::fs::read("/nonexistent/x").with_path(std::path::Path::new("/nonexistent/x")).unwrap_err();
        assert_eq!((p.message(), p.key()), ("/nonexistent/x: No such file or directory", Some("%@: %@")));
    }
}
