// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of VZBackend.swift – work package F4. Done: the type and constants. Skeleton: the bodies (`todo!`).
//!
//! macOS guests on Virtualization.framework (spec §6.4, plan M10): every instance is an APFS clone of the
//! image folder in `instances/<name>.omaflake-vm/vm/`, run by its own detached `omaf-runner` process.
//!
//! Network: `shared` = NAT (address from /var/db/dhcpd_leases, SSH on port 22), `none` = no network card.
//! `isolated` is not available: Virtualization.framework's NAT has no restricted mode, and a vmnet host-mode
//! network (`VZVmnetNetworkDeviceAttachment`, macOS 26) needs the restricted vmnet entitlement (plan BM27).
//! Ephemeral: a fresh clone at every start; the clone of the previous run is deleted (BM27).
//!
//! Not 1:1: `locate_runner` looked next to `Bundle.main.executableURL`; here it is `std::env::current_exe()`
//! (resolved), with the same two candidates (`../Helpers/omaf-runner.app/Contents/MacOS/omaf-runner`, `omaf-runner`).
#![allow(unused_variables, dead_code)]

use std::time::Duration;

use crate::agent_protocol::AgentState;
use crate::backend::{StartOptions, VmBackend};
use crate::error::Result;
use crate::image_store::Image;
use crate::instance_store::InstanceStore;
use crate::models::{GuestKind, InstanceConfig, NetworkMode, RecipeDefaults, StopResult, VmStatus};
use crate::paths::Paths;
use crate::spawn::Pid;

#[derive(Debug, Clone)]
pub struct VzBackend {
    pub paths: Paths,
    pub store: InstanceStore,
    /// Signed omaf-runner (scripts/sign-runner.sh)
    pub runner: String,
}

impl VzBackend {
    /// Virtualization.framework runs at most two macOS VMs per Mac (plan BM16).
    pub const MAX_RUNNING: usize = 2;

    pub fn new(paths: Paths, store: InstanceStore, runner: &str) -> Self {
        VzBackend { paths, store, runner: runner.into() }
    }

    /// omaf-runner from VM_RUNNER, config.json (`runner`), next to the running executable, or PATH.
    pub fn locate_runner(paths: &Paths) -> Result<String> {
        todo!("port from VZBackend.swift")
    }

    /// `omaf-runner clone` (APFS clone, new machine identifier and MAC) of the image inside the package or the one in
    /// the data folder, then CPU/RAM from the instance settings. The disk keeps the image's size: growing a macOS
    /// system disk needs an APFS resize in the guest (OPEN).
    pub(crate) fn clone_image(&self, c: &InstanceConfig) -> Result<()> {
        todo!("port from VZBackend.swift")
    }

    /// "Network mode \"isolated\" is not available for macOS guests (plan BM27) – use shared or none"
    pub(crate) fn check_network(mode: NetworkMode) -> Result<()> {
        todo!("port from VZBackend.swift")
    }

    /// The runner's PID from its pid file, if alive and really omaf-runner
    pub fn pid(&self, c: &InstanceConfig) -> Option<Pid> {
        todo!("port from VZBackend.swift")
    }

    /// Without the entitlement, Virtualization.framework refuses to start – say so before spawning
    /// (`codesign -d --entitlements - --xml <runner>` must mention com.apple.security.virtualization).
    pub fn check_entitlement(&self) -> Result<()> {
        todo!("port from VZBackend.swift")
    }

    /// NAT address of the running guest (from the DHCP leases, by the MAC address of its clone).
    pub fn address(&self, c: &InstanceConfig) -> Option<String> {
        todo!("port from VZBackend.swift")
    }

    /// Sets HostName, LocalHostName and ComputerName to the instance name. The image carries its recipe's name
    /// (nix-darwin sets it only on activation), so every clone would otherwise share it. Needs the sudo rule from
    /// darwin-guest.nix; with an older image it fails and the guest keeps the recipe's name.
    pub fn apply_host_name(&self, c: &InstanceConfig, host: &str) -> bool {
        todo!("port from VZBackend.swift")
    }

    /// Starts the guest's clipboard agent (darwin-guest.nix) – over SSH instead of a LaunchAgent, which macOS would
    /// announce as a background item in every clone (BM25). Images without it simply have no shared clipboard.
    pub fn start_clipboard_agent(&self, c: &InstanceConfig, host: &str) -> bool {
        todo!("port from VZBackend.swift")
    }

    /// `SshCommand::arguments` to `host` port 22; `batch` adds `-o BatchMode=yes -o ConnectTimeout=10` after ssh
    pub(crate) fn ssh_arguments_to(
        &self,
        c: &InstanceConfig,
        host: &str,
        command: &[String],
        batch: bool,
    ) -> Vec<String> {
        todo!("port from VZBackend.swift")
    }
}

impl VmBackend for VzBackend {
    fn kind(&self) -> GuestKind {
        GuestKind::Macos
    }

    /// Ephemeral instances get their clone at every start; a failed clone deletes the new package again
    fn create(&self, name: &str, recipe: &str, image: &Image, settings: &RecipeDefaults) -> Result<InstanceConfig> {
        todo!("port from VZBackend.swift")
    }

    /// `omaf-runner run <vm> --socket … --pid-file … [--window] [--no-network] [--audio] [--microphone] [--clipboard]
    /// --agent-state <file> [--camera [--camera-test-pattern]] [--share <dir>]`, detached; ready when the runner's
    /// status says "running" (30 s). At most `MAX_RUNNING` macOS VMs.
    fn start(&self, c: &InstanceConfig, options: StartOptions) -> Result<Pid> {
        todo!("port from VZBackend.swift")
    }

    /// Shutdown via SSH (`sudo -n /sbin/shutdown -h now`, allowed by darwin-guest.nix; a stop request from the
    /// runner hangs on the guest's confirmation dialog when a user is logged in, plan BM17) → runner stop request
    /// → after `timeout` the runner forces the VM off → SIGKILL.
    fn stop(&self, c: &InstanceConfig, timeout: Duration) -> Result<StopResult> {
        todo!("port from VZBackend.swift")
    }

    fn status(&self, c: &InstanceConfig) -> VmStatus {
        todo!("port from VZBackend.swift")
    }

    fn reset(&self, c: &InstanceConfig) -> Result<()> {
        todo!("port from VZBackend.swift")
    }

    fn upgrade(&self, c: &InstanceConfig, image: &Image) -> Result<()> {
        todo!("port from VZBackend.swift")
    }

    fn remove(&self, c: &InstanceConfig) -> Result<()> {
        todo!("port from VZBackend.swift")
    }

    /// Once SSH answers, the guest is named after the instance (plan BM28) and gets its clipboard agent (B48).
    fn wait_for_ssh(&self, c: &InstanceConfig, timeout: Duration) -> bool {
        todo!("port from VZBackend.swift")
    }

    /// What the guest's omaflake agent last reported (agent.json, written by omaf-runner) – None while stopped
    fn agent_state(&self, c: &InstanceConfig) -> Option<AgentState> {
        todo!("port from VZBackend.swift")
    }

    /// Without a lease yet, ssh fails with a clear "could not resolve" instead of a wrong host
    /// (`no-address-yet.<name>.invalid`)
    fn ssh_arguments(&self, c: &InstanceConfig, command: &[String]) -> Vec<String> {
        todo!("port from VZBackend.swift")
    }
}
