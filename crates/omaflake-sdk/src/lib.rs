// SPDX-License-Identifier: AGPL-3.0-or-later
//! omaflake's SDK (plan B192): what every interface and every tool uses to talk to omaflake – the client of the
//! core service's socket (`omaf serve`, docs/api.md), the API's types, its errors and texts. It needs a running
//! omaflake: the app, or `omaf service start`.
//!
//! ```no_run
//! use omaflake_sdk::client::Client;
//!
//! let socket = std::path::Path::new("/Users/me/.omaflake/run/omaf.sock");
//! let omaf = Client::connect(socket)?;
//! let session = omaf.initialize("my-tool")?;
//! println!("omaflake {} on {}", session.omaf_version, session.host.os);
//! let vms = omaf.call("vms.list", serde_json::Value::Null)?;
//! # Ok::<(), omaflake_sdk::protocol::RpcError>(())
//! ```
//!
//! - [`client`]: the connection to the service – calls, events, following a job
//! - [`types`]: the API's types (docs/api.md §2–§7), [`models`], [`networks`], [`diagnostics`]: the kinds they use
//! - [`views`]: the page models (§10) – what each page shows; an interface only draws them
//! - [`protocol`], [`codes`]: errors and their kinds; [`text`]: texts with their translation keys; [`time`]: dates
//! - [`brand`]: omaflake's design tokens – colours, spacing, type – for interfaces of their own
//!
//! The core (`omaflake-core`) builds and serves all of it; it changes only with the API's version (`apiVersion`).

#![forbid(unsafe_code)]

pub mod brand;
pub mod client;
pub mod codes;
pub mod diagnostics;
pub mod models;
pub mod networks;
pub mod protocol;
pub mod text;
pub mod time;
pub mod types;
pub mod views;

/// This SDK's version – omaflake's
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
