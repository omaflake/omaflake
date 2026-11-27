// SPDX-License-Identifier: AGPL-3.0-or-later
//! Error kinds (`data.kind` of a `-32000` error, docs/api.md §3) – the core's `Error::code`. Keep them stable: UIs
//! and every client rely on them.

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
