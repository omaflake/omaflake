// SPDX-License-Identifier: AGPL-3.0-or-later
//! JSON-RPC 2.0 errors as omaflake's service answers them (docs/api.md §1, §3).

use serde_json::{Map, Value, json};

use crate::codes;

/// Invalid JSON
pub const PARSE_ERROR: i64 = -32700;
/// Not a JSON-RPC request object
pub const INVALID_REQUEST: i64 = -32600;
/// No such method
pub const METHOD_NOT_FOUND: i64 = -32601;
/// Params missing, of the wrong type or out of range
pub const INVALID_PARAMS: i64 = -32602;
/// A request before `initialize`
pub const NOT_INITIALIZED: i64 = -32002;
/// Everything the core refuses or fails at (`data.kind` tells which)
pub const SERVER_ERROR: i64 = -32000;

/// An error answer: the JSON-RPC `code`, a message for people and – for `-32000` – the error kind (`data.kind`) plus
/// extra data some methods give (e.g. `networkVolume` for `vms.useExternal`).
#[derive(Debug, Clone, PartialEq)]
pub struct RpcError {
    pub code: i64,
    pub message: String,
    pub kind: Option<&'static str>,
    pub data: Map<String, Value>,
}

impl RpcError {
    pub fn new(code: i64, message: impl Into<String>) -> Self {
        RpcError { code, message: message.into(), kind: None, data: Map::new() }
    }

    pub fn parse(message: impl Into<String>) -> Self {
        Self::new(PARSE_ERROR, message)
    }

    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self::new(INVALID_REQUEST, message)
    }

    pub fn method_not_found(method: &str) -> Self {
        Self::new(METHOD_NOT_FOUND, format!("Unknown method \"{method}\""))
    }

    pub fn invalid_params(message: impl Into<String>) -> Self {
        Self::new(INVALID_PARAMS, message)
    }

    pub fn not_initialized() -> Self {
        Self::new(NOT_INITIALIZED, "Not initialized – send initialize first")
    }

    /// A `-32000` error of a kind (docs/api.md §3)
    pub fn server(kind: Option<&'static str>, message: impl Into<String>) -> Self {
        RpcError { code: SERVER_ERROR, message: message.into(), kind, data: Map::new() }
    }

    /// A panic in the core (a bug): `-32000` of kind `internal`
    pub fn internal(message: impl Into<String>) -> Self {
        Self::server(Some(codes::INTERNAL), message)
    }

    /// Adds a field to `data` (next to `kind`)
    pub fn with_data(mut self, key: &str, value: Value) -> Self {
        self.data.insert(key.into(), value);
        self
    }

    /// The `error` object of a response
    pub fn to_value(&self) -> Value {
        let mut o = Map::new();
        o.insert("code".into(), json!(self.code));
        o.insert("message".into(), json!(self.message));
        let mut data = self.data.clone();
        if let Some(kind) = self.kind {
            data.insert("kind".into(), json!(kind));
        }
        if !data.is_empty() {
            o.insert("data".into(), Value::Object(data));
        }
        Value::Object(o)
    }
}

impl std::fmt::Display for RpcError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl RpcError {
    /// The message as a text (`data.text`, docs/api.md §3): a UI shows it in its language; None for an error without
    /// a key (its `message` is shown as it is)
    pub fn text(&self) -> Option<crate::text::Text> {
        self.data.get("text").and_then(|t| serde_json::from_value(t.clone()).ok())
    }
}
