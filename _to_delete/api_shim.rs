// SPDX-License-Identifier: AGPL-3.0-or-later
//! Stand-in for `omaflake_core::api` (docs/api.md), which is written in parallel and not part of this branch.
//! MERGE: delete this file, drop `mod api_shim;` in main.rs and import `omaflake_core::api` as `api_shim` there (or
//! rename the two calls in commands/mod.rs). The signatures below are what the CLI expects:
//! - `serve_stdio()` serves JSON-RPC on stdin/stdout until stdin closes or `shutdown`;
//! - `call_once(method, params)` runs one request without a session (`params` = `Value::Null` when none were given)
//!   and returns its `result`; an error carries the message (and `data.kind` as the error's code).

#![allow(unused_variables)]

use omaflake_core::Result;
use serde_json::Value;

/// `omaf serve`
pub fn serve_stdio() -> Result<()> {
    unimplemented!("replaced by omaflake_core::api::serve_stdio at the merge")
}

/// `omaf call <method> [<params>]`
pub fn call_once(method: &str, params: Value) -> Result<Value> {
    unimplemented!("replaced by omaflake_core::api::call_once at the merge")
}
