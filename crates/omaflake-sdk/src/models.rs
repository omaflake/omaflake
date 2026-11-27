// SPDX-License-Identifier: AGPL-3.0-or-later
//! The kinds of the API that the core's own models are made of too.

use serde::{Deserialize, Serialize};

/// Guest kind of a recipe or instance (spec §3): NixOS on QEMU or macOS on Virtualization.framework.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum GuestKind {
    Nixos,
    Macos,
}

impl GuestKind {
    /// Swift `GuestKind.allCases`, in their order
    pub const ALL: [GuestKind; 2] = [GuestKind::Nixos, GuestKind::Macos];

    /// Swift `rawValue`: "nixos", "macos"
    pub fn as_str(self) -> &'static str {
        match self {
            GuestKind::Nixos => "nixos",
            GuestKind::Macos => "macos",
        }
    }

    /// Swift `GuestKind(rawValue:)`
    pub fn parse(s: &str) -> Option<GuestKind> {
        match s {
            "nixos" => Some(GuestKind::Nixos),
            "macos" => Some(GuestKind::Macos),
            _ => None,
        }
    }
}

impl std::fmt::Display for GuestKind {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}
