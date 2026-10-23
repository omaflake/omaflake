// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of QEMUCommand.swift – work package F2. Done: the type and constants. Skeleton: the argument list (`todo!`).
//! Port the QEMUCommandTests of Tests/OmaflakeCoreTests/OmaflakeCoreTests.swift with it.
//!
//! Builds the QEMU argument list from an instance (pure function, covered by unit tests).
//! Reference: scripts/bootprobe.sh (M1) and spec §6.3.
#![allow(unused_variables, dead_code)]

use crate::models::{DisplayMode, GlBackend, InstanceConfig, QemuCapabilities};
use crate::paths::InstancePaths;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QemuCommand {
    pub firmware: String,
    pub paths: InstancePaths,
    pub share_path: Option<String>,
    pub display: DisplayMode,
    pub gl: GlBackend,
    pub capabilities: QemuCapabilities,
    /// Initial resolution of the virtual GPU in pixels (the window opens to match; afterwards the guest follows the
    /// window)
    pub initial_resolution: Option<(u32, u32)>,
}

/// PCI slots per device, the same with and without a window (Swift `QEMUCommand.Slot`)
pub mod slot {
    pub const DISK: &str = "0x2";
    pub const RNG: &str = "0x3";
    pub const NET: &str = "0x4";
    pub const GPU: &str = "0x5";
    pub const USB: &str = "0x6";
    pub const SHARE: &str = "0x7";
    pub const SERIAL: &str = "0x8";
    pub const SOUND: &str = "0x9";
}

impl QemuCommand {
    /// UID/GID of the guest user (NixOS default for the first normal user and the "users" group)
    pub const GUEST_UID: u32 = 1000;
    pub const GUEST_GID: u32 = 100;

    /// USB keyboard and tablet on an xHCI controller (windows only)
    pub const INPUT: [&'static str; 6] =
        ["-device", "qemu-xhci,addr=0x6", "-device", "usb-kbd", "-device", "usb-tablet"];

    /// Swift defaults: `gl` .on, capabilities without GL and guest owner, no initial resolution
    pub fn new(firmware: &str, paths: InstancePaths, share_path: Option<String>, display: DisplayMode) -> Self {
        QemuCommand {
            firmware: firmware.into(),
            paths,
            share_path,
            display,
            gl: GlBackend::On,
            capabilities: QemuCapabilities::default(),
            initial_resolution: None,
        }
    }

    /// 75 % of the screen area in 16:9, in pixels, even width/height (Retina scale 2 stays integral).
    /// Swift `initialResolution(screenPoints:scale:)`.
    pub fn initial_resolution_for(screen_points: (f64, f64), scale: f64) -> (u32, u32) {
        todo!("port from QEMUCommand.swift")
    }

    /// "<name>,xres=<w>,yres=<h>" with an initial resolution, else the name
    pub(crate) fn gpu_device(&self, name: &str) -> String {
        todo!("port from QEMUCommand.swift")
    }

    /// The whole argument list (without the binary) – see QEMUCommand.swift for the order, which the tests check
    pub fn arguments(&self, c: &InstanceConfig) -> Vec<String> {
        todo!("port from QEMUCommand.swift")
    }
}
