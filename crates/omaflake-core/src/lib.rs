// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaflake's core in Rust – the port of `Sources/OmaflakeCore` (everything but Brand.swift and BrandAssets.swift,
//! which stay with the Swift app). One module per Swift file; conventions, status and the Swift → Rust map are in
//! PORTING.md next to this crate's Cargo.toml.
//!
//! The core is blocking like the Swift one (threads where Swift used tasks), portable (macOS-only APIs behind
//! `cfg(target_os = "macos")`), and talks to Virtualization.framework only through the `omaf-runner` subprocess.

pub mod error;
pub mod json;

pub mod agent_protocol;
pub mod api;
pub mod backend;
pub mod build_progress;
pub mod builder_maintenance;
pub mod cancellation;
pub mod cli_link;
pub mod credentials;
pub mod data_folder;
pub mod file_format;
pub mod guest_choices;
pub mod guest_settings;
pub mod host_bridge;
pub mod image_migration;
pub mod image_store;
pub mod instance_store;
pub mod ipsw_downloader;
pub mod locales;
pub mod looks;
pub mod mac_base_installer;
pub mod mac_image_builder;
pub mod mac_vm;
pub mod models;
pub mod nix_gl;
pub mod omaflake_id;
pub mod packages;
pub mod paths;
pub mod platform;
pub mod prerequisites;
pub mod qemu_command;
pub mod qemu_host;
pub mod qemu_runtime;
pub mod qmp_client;
pub mod recipe_sources;
pub mod shell;
pub mod sockets;
pub mod source_tokens;
pub mod spawn;
pub mod ssh_command;
pub mod transfer;
pub mod trash;
pub mod update_check;
pub mod vz_backend;
pub mod word_list;

/// omaflake's version (the workspace version, kept equal to App/Info.plist's CFBundleShortVersionString)
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// A log callback: one line of progress or output (Swift `@Sendable (String) -> Void`)
pub type Log<'a> = dyn Fn(&str) + Send + Sync + 'a;
/// A progress callback: (done, total) bytes or units (Swift `@Sendable (Int64, Int64) -> Void`)
pub type Progress<'a> = dyn Fn(i64, i64) + Send + Sync + 'a;

pub use error::{Error, Result};

pub use agent_protocol::{
    AgentDecoder, AgentFrame, AgentHello, AgentLink, AgentMessage, AgentProtocol, AgentReassembler, AgentState,
    AgentStatus, AgentWelcome,
};
pub use backend::{Backends, QemuBackend, Readiness, StartOptions, VmBackend};
pub use build_progress::{BuildLog, BuildProgress, BuildProgressTracker};
pub use builder_maintenance::{shell_quote, AdminShell, BuilderMaintenance};
pub use cancellation::Cancellation;
pub use data_folder::DataFolder;
pub use file_format::{FileFormat, UnreadableFile};
pub use guest_choices::GuestChoices;
pub use guest_settings::GuestSettings;
pub use image_store::{Image, ImageInfo, ImageStore};
pub use instance_store::InstanceStore;
pub use ipsw_downloader::{free_space, gigabytes, IpswDownloader, KeepAwake, MacOsRelease};
pub use mac_base_installer::MacBaseInstaller;
pub use mac_image_builder::MacImageBuilder;
pub use mac_vm::{DhcpLeases, MacAddress, MacVmBundle, MacVmConfig, RunnerClient};
pub use models::{
    DisplayMode, GlBackend, GuestKind, InstanceConfig, LinePlace, NetworkMode, QemuCapabilities, RecipeDefaults,
    RecipeId, StopResult, VmStatus,
};
pub use omaflake_id::{Lengths, OmaflakeId, OmaflakeIdKind};
pub use packages::{Manifest as PackageManifest, Package, PackageKind};
pub use paths::{InstancePaths, Paths, Settings};
pub use platform::Platform;
pub use prerequisites::{Prerequisite, PrerequisiteItem, PrerequisiteStatus, Prerequisites};
pub use qemu_command::QemuCommand;
pub use qemu_runtime::{tail, FileLock, InstanceLock, QemuRuntime};
pub use qmp_client::QmpClient;
pub use recipe_sources::{
    RecipeCatalog, RecipeLibrary, RecipeRef, RecipeSource, RecipeSourceKind, SourceClient, SourceTokens,
};
pub use shell::{RunOptions, Shell, ShellResult, StreamOptions};
pub use sockets::{connect_unix, Ports};
pub use spawn::Pid;
pub use ssh_command::SshCommand;
pub use transfer::Transfer;
pub use trash::Disposal;
pub use update_check::{UpdateCheck, UpdateFeed};
pub use vz_backend::VzBackend;
