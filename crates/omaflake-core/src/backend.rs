// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Backend.swift.
//!
//! Swift's `protocol VMBackend` is the object-safe trait `VmBackend`: `QemuBackend` (here, F2) and `VzBackend`
//! (vz_backend.rs, F4) implement it independently, and `Backends::backend` hands out `&dyn VmBackend` – so the two
//! work packages never touch each other's files.

use std::time::Duration;

use crate::agent_protocol::AgentState;
use crate::error::{Error, Result};
use crate::image_store::Image;
use crate::instance_store::InstanceStore;
use crate::models::{
    DisplayMode, GlBackend, GuestKind, InstanceConfig, NetworkMode, RecipeDefaults, StopResult, VmStatus,
};
use crate::paths::Paths;
use crate::qemu_runtime::QemuRuntime;
use crate::spawn::Pid;
use crate::ssh_command::SshCommand;
use crate::vz_backend::VzBackend;

/// Options for `start`; `gl` only matters for QEMU windows with 3D.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StartOptions {
    pub display: DisplayMode,
    pub gl: GlBackend,
}

impl StartOptions {
    /// Swift default `gl: .on`
    pub fn new(display: DisplayMode) -> Self {
        StartOptions { display, gl: GlBackend::On }
    }
}

/// How a VM showed it is up
#[derive(Debug, Clone, PartialEq)]
pub enum Readiness {
    Agent(Box<AgentState>),
    Ssh,
}

/// Operations on the instances of one guest kind (plan §7.3). The CLI and the app talk only to this
/// trait; NixOS is `QemuBackend`, macOS is `VzBackend` on `omaf-runner` (milestone M10).
pub trait VmBackend: Send + Sync {
    fn kind(&self) -> GuestKind;
    /// Create the instance from an image (NixOS: qcow2 overlay; macOS: clonefile + new machine id).
    fn create(&self, name: &str, recipe: &str, image: &Image, settings: &RecipeDefaults) -> Result<InstanceConfig>;
    fn start(&self, c: &InstanceConfig, options: StartOptions) -> Result<Pid>;
    fn stop(&self, c: &InstanceConfig, timeout: Duration) -> Result<StopResult>;
    fn status(&self, c: &InstanceConfig) -> VmStatus;
    /// Deletes the old state, fresh state from the same image.
    fn reset(&self, c: &InstanceConfig) -> Result<()>;
    /// Deletes the old state, fresh state from another image.
    fn upgrade(&self, c: &InstanceConfig, image: &Image) -> Result<()>;
    /// Deletes the instance folder (the share folder stays).
    fn remove(&self, c: &InstanceConfig) -> Result<()>;
    fn wait_for_ssh(&self, c: &InstanceConfig, timeout: Duration) -> bool;
    /// Waits until the VM is up: its agent's hello (spec §11.9) or SSH; None if it stopped or the time ran out.
    /// Default: SSH when the VM has a network.
    fn wait_for_ready(&self, c: &InstanceConfig, timeout: Duration) -> Option<Readiness> {
        (c.network != NetworkMode::None && self.wait_for_ssh(c, timeout)).then_some(Readiness::Ssh)
    }
    /// What the VM's agent last reported; None without one (macOS guests: not yet, spec §11.9)
    fn agent_state(&self, _c: &InstanceConfig) -> Option<AgentState> {
        None
    }
    /// ssh invocation including `/usr/bin/ssh` in first position.
    fn ssh_arguments(&self, c: &InstanceConfig, command: &[String]) -> Vec<String>;
}

/// NixOS guests: QEMU with HVF, qcow2 overlays (spec §6.3).
#[derive(Debug, Clone)]
pub struct QemuBackend {
    pub runtime: QemuRuntime,
    pub store: InstanceStore,
}

impl QemuBackend {
    pub fn new(runtime: QemuRuntime, store: InstanceStore) -> Self {
        QemuBackend { runtime, store }
    }

    /// QEMU from config.json, VM_QEMU or PATH (see `QemuRuntime::locate`).
    pub fn locate(paths: &Paths) -> Result<QemuBackend> {
        let (rt, qemu_img) = QemuRuntime::locate(paths)?;
        Ok(QemuBackend::new(rt, InstanceStore::new(paths.clone(), &qemu_img)))
    }
}

impl VmBackend for QemuBackend {
    fn kind(&self) -> GuestKind {
        GuestKind::Nixos
    }

    fn create(&self, name: &str, recipe: &str, image: &Image, settings: &RecipeDefaults) -> Result<InstanceConfig> {
        self.store.create(name, recipe, image, settings, None)
    }

    /// Checks the system (`InstanceStore::check_config_runs_here`), then `QemuRuntime::start`
    fn start(&self, c: &InstanceConfig, options: StartOptions) -> Result<Pid> {
        InstanceStore::check_config_runs_here(c)?;
        self.runtime.start(c, options.display, options.gl)
    }

    fn stop(&self, c: &InstanceConfig, timeout: Duration) -> Result<StopResult> {
        self.runtime.stop(&c.name, timeout)
    }

    fn status(&self, c: &InstanceConfig) -> VmStatus {
        self.runtime.status(&c.name)
    }

    fn reset(&self, c: &InstanceConfig) -> Result<()> {
        self.store.reset(&c.name)
    }

    fn upgrade(&self, c: &InstanceConfig, image: &Image) -> Result<()> {
        InstanceStore::check_runs_here(image)?;
        self.store.upgrade(&c.name, image)
    }

    fn remove(&self, c: &InstanceConfig) -> Result<()> {
        self.store.remove(&c.name)
    }

    fn wait_for_ssh(&self, c: &InstanceConfig, timeout: Duration) -> bool {
        self.runtime.wait_for_ssh(c, timeout)
    }

    fn wait_for_ready(&self, c: &InstanceConfig, timeout: Duration) -> Option<Readiness> {
        self.runtime.wait_for_ready(c, timeout)
    }

    fn agent_state(&self, c: &InstanceConfig) -> Option<AgentState> {
        self.runtime.agent_state(&c.name)
    }

    fn ssh_arguments(&self, c: &InstanceConfig, command: &[String]) -> Vec<String> {
        SshCommand::arguments(c, &self.store.paths, SshCommand::DEFAULT_HOST, None, command)
    }
}

/// Backend per guest kind. Instances are listed by `InstanceStore` regardless of kind. A missing QEMU only
/// blocks NixOS guests and a missing omaf-runner only macOS guests; the error comes with the first use.
#[derive(Debug, Clone)]
pub struct Backends {
    pub paths: Paths,
    pub store: InstanceStore,
    qemu_result: std::result::Result<QemuBackend, Error>,
    vz_result: std::result::Result<VzBackend, Error>,
}

impl Backends {
    /// Locates QEMU (`QemuBackend::locate`) and omaf-runner (`VzBackend::locate_runner`); the store uses QEMU's
    /// qemu-img, else "qemu-img". Never fails itself (Swift: `throws`, but nothing in it threw).
    pub fn new(paths: Paths) -> Result<Backends> {
        let qemu_result = QemuBackend::locate(&paths);
        let qemu_img = qemu_result.as_ref().map(|q| q.store.qemu_img.clone()).unwrap_or_else(|_| "qemu-img".into());
        let store = InstanceStore::new(paths.clone(), &qemu_img);
        let vz_result =
            VzBackend::locate_runner(&paths).map(|runner| VzBackend::new(paths.clone(), store.clone(), &runner));
        Ok(Backends { paths, store, qemu_result, vz_result })
    }

    pub fn qemu(&self) -> Result<&QemuBackend> {
        self.qemu_result.as_ref().map_err(Clone::clone)
    }

    pub fn vz(&self) -> Result<&VzBackend> {
        self.vz_result.as_ref().map_err(Clone::clone)
    }

    pub fn backend(&self, kind: GuestKind) -> Result<&dyn VmBackend> {
        match kind {
            GuestKind::Nixos => self.qemu().map(|b| b as &dyn VmBackend),
            GuestKind::Macos => self.vz().map(|b| b as &dyn VmBackend),
        }
    }

    pub fn backend_for(&self, c: &InstanceConfig) -> Result<&dyn VmBackend> {
        self.backend(c.guest_kind())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::tests::config;
    use crate::paths::tests::temp_paths;

    fn backend() -> (tempfile::TempDir, QemuBackend) {
        let (t, p) = temp_paths();
        let rt = QemuRuntime::new(p.clone(), "qemu-system-aarch64", "/fw.fd");
        (t, QemuBackend::new(rt, InstanceStore::new(p, "qemu-img")))
    }

    #[test]
    fn qemu_backend_basics() {
        let (_t, b) = backend();
        let c = config("t1");
        assert_eq!(b.kind(), GuestKind::Nixos);
        assert_eq!(b.status(&c), VmStatus::Stopped);
        assert_eq!(b.stop(&c, Duration::from_secs(1)).unwrap(), StopResult::AlreadyStopped);
        assert_eq!(b.agent_state(&c), None);
        let args = b.ssh_arguments(&c, &["uptime".to_string()]);
        assert_eq!(&args[..3], ["/usr/bin/ssh", "-p", "2230"]);
        assert!(args.last().unwrap() == "uptime" && args[args.len() - 2].ends_with("@127.0.0.1"));
    }

    /// A VM built for another architecture does not start here (plan B114)
    #[test]
    fn start_checks_the_system_first() {
        let (_t, b) = backend();
        let mut c = config("t1");
        c.system = "riscv64-linux".into();
        let e = b.start(&c, StartOptions::new(DisplayMode::None)).unwrap_err();
        assert_eq!(e.code(), Some(crate::error::codes::UNSUPPORTED_SYSTEM));
    }
}
