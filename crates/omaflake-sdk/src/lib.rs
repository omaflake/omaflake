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
//! Stage one of the split out of `omaflake-core`: the session, jobs and events, errors, texts and the client. The other
//! types of docs/api.md and the page models (§10) follow (OPEN); until then they are `omaflake-core`'s.

#![forbid(unsafe_code)]

pub mod client;
pub mod codes;
pub mod models;
pub mod protocol;
pub mod text;
pub mod time;
pub mod types;

/// This SDK's version – omaflake's
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
