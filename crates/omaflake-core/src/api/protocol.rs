// SPDX-License-Identifier: AGPL-3.0-or-later
//! JSON-RPC 2.0 framing (docs/api.md §1, §3): parsing a line into a request, building responses, notifications and
//! errors. Nothing here knows about omaflake – the dispatcher (server.rs) does.

use serde::de::DeserializeOwned;
use serde_json::{Map, Value, json};

use crate::error::codes;

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
    pub fn text(&self) -> Option<super::views::text::Text> {
        self.data.get("text").and_then(|t| serde_json::from_value(t.clone()).ok())
    }
}

impl From<crate::Error> for RpcError {
    /// `-32000` with the kind and – for a message with a key – `data.text`
    fn from(e: crate::Error) -> Self {
        let rpc = Self::server(e.code(), e.message());
        match e.key() {
            Some(_) => rpc.with_data("text", serde_json::to_value(e.text()).unwrap_or_default()),
            None => rpc,
        }
    }
}

impl From<RpcError> for crate::Error {
    fn from(e: RpcError) -> Self {
        let text = e.text().unwrap_or_else(|| super::views::text::Text::verbatim(&e.message));
        crate::Error::from_text(e.kind, text)
    }
}

/// A line from the client, classified
#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Incoming {
    /// Expects an answer
    Request { id: Value, method: String, params: Value },
    /// No `id`: run, never answered (api.md: clients send none, but JSON-RPC allows them)
    Notification { method: String, params: Value },
    /// Answered with this error at once
    Invalid { id: Value, error: RpcError },
}

/// Classifies one NDJSON line (docs/api.md §1). Batches (arrays) are not supported – `-32600`.
pub(crate) fn parse_line(line: &str) -> Incoming {
    let value: Value = match serde_json::from_str(line) {
        Ok(v) => v,
        Err(e) => return Incoming::Invalid { id: Value::Null, error: RpcError::parse(format!("Parse error: {e}")) },
    };
    parse_value(value)
}

/// Classifies a request object (the in-process entry: `Server::handle`)
pub(crate) fn parse_value(value: Value) -> Incoming {
    let Value::Object(mut o) = value else {
        return Incoming::Invalid {
            id: Value::Null,
            error: RpcError::invalid_request("A request is a JSON object (batches are not supported)"),
        };
    };
    let id = o.remove("id");
    let valid_id = matches!(id, None | Some(Value::Null | Value::Number(_) | Value::String(_)));
    let answer_id = if valid_id { id.clone().unwrap_or(Value::Null) } else { Value::Null };
    if o.get("jsonrpc").and_then(Value::as_str) != Some("2.0") || !valid_id {
        return Incoming::Invalid { id: answer_id, error: RpcError::invalid_request("Not a JSON-RPC 2.0 request") };
    }
    let Some(Value::String(method)) = o.remove("method") else {
        return Incoming::Invalid { id: answer_id, error: RpcError::invalid_request("The request has no method") };
    };
    let params = o.remove("params").unwrap_or(Value::Null);
    if !matches!(params, Value::Null | Value::Object(_) | Value::Array(_)) {
        return Incoming::Invalid { id: answer_id, error: RpcError::invalid_params("params is an object") };
    }
    match id {
        None => Incoming::Notification { method, params },
        Some(id) => Incoming::Request { id, method, params },
    }
}

pub(crate) fn response_ok(id: Value, result: Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": result})
}

pub(crate) fn response_err(id: Value, error: &RpcError) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "error": error.to_value()})
}

pub(crate) fn notification(method: &str, params: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": method, "params": params})
}

/// Decodes `params` (absent or `null` = `{}`); a mismatch is `-32602` with serde's text
pub(crate) fn params<T: DeserializeOwned>(params: Value) -> Result<T, RpcError> {
    let params = if params.is_null() { Value::Object(Map::new()) } else { params };
    serde_json::from_value(params).map_err(|e| RpcError::invalid_params(format!("Invalid params: {e}")))
}

/// The params object as a map (for methods where an absent key and `null` mean different things)
pub(crate) fn params_map(params: Value) -> Result<Map<String, Value>, RpcError> {
    match params {
        Value::Null => Ok(Map::new()),
        Value::Object(o) => Ok(o),
        _ => Err(RpcError::invalid_params("params is an object")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_requests_notifications_and_garbage() {
        let r = parse_line(r#"{"jsonrpc":"2.0","id":1,"method":"vms.list"}"#);
        assert_eq!(r, Incoming::Request { id: json!(1), method: "vms.list".into(), params: Value::Null });
        let r = parse_line(r#"{"jsonrpc":"2.0","id":"a","method":"vms.get","params":{"name":"x"}}"#);
        assert_eq!(r, Incoming::Request { id: json!("a"), method: "vms.get".into(), params: json!({"name": "x"}) });
        let r = parse_line(r#"{"jsonrpc":"2.0","method":"x"}"#);
        assert_eq!(r, Incoming::Notification { method: "x".into(), params: Value::Null });
        let Incoming::Invalid { id, error } = parse_line("{nope") else { panic!() };
        assert_eq!((id, error.code), (Value::Null, PARSE_ERROR));
        let Incoming::Invalid { id, error } = parse_line(r#"{"id":3,"method":"x"}"#) else { panic!() };
        assert_eq!((id, error.code), (json!(3), INVALID_REQUEST));
        let Incoming::Invalid { error, .. } = parse_line(r#"[{"jsonrpc":"2.0","id":1,"method":"x"}]"#) else {
            panic!()
        };
        assert_eq!(error.code, INVALID_REQUEST);
        let Incoming::Invalid { id, error } = parse_line(r#"{"jsonrpc":"2.0","id":4,"method":"x","params":3}"#) else {
            panic!()
        };
        assert_eq!((id, error.code), (json!(4), INVALID_PARAMS));
        let Incoming::Invalid { id, .. } = parse_line(r#"{"jsonrpc":"2.0","id":{"a":1},"method":"x"}"#) else {
            panic!()
        };
        assert_eq!(id, Value::Null);
    }

    #[test]
    fn errors_carry_kind_and_data() {
        let e: RpcError = crate::Error::with_code(codes::VM_RUNNING, "running – stop it first").into();
        assert_eq!(
            e.to_value(),
            json!({"code": -32000, "message": "running – stop it first", "data": {"kind": "vm-running"}})
        );
        let plain: RpcError = crate::Error::new("boom").into();
        assert_eq!(plain.to_value(), json!({"code": -32000, "message": "boom"}));
        let extra = RpcError::server(Some(codes::NOT_FOUND), "x").with_data("networkVolume", json!(true));
        assert_eq!(extra.to_value()["data"], json!({"kind": "not-found", "networkVolume": true}));
        let back: crate::Error = extra.into();
        assert_eq!(back.code(), Some(codes::NOT_FOUND));
        // A keyed message carries its text, and comes back with it
        let keyed: RpcError = crate::vm_err!(codes::NOT_FOUND, "Instance \"{}\" does not exist", "a").into();
        assert_eq!(
            keyed.to_value()["data"],
            json!({"kind": "not-found", "text": {"key": "Instance \"%@\" does not exist", "text": "Instance \"a\" does not exist", "args": ["a"]}})
        );
        let back: crate::Error = keyed.into();
        assert_eq!(
            (back.key(), back.message()),
            (Some("Instance \"%@\" does not exist"), "Instance \"a\" does not exist")
        );
        assert_eq!(response_err(json!(2), &RpcError::not_initialized())["error"]["code"], json!(-32002));
        assert_eq!(response_ok(json!(2), Value::Null), json!({"jsonrpc": "2.0", "id": 2, "result": null}));
        assert_eq!(notification("message", json!({})), json!({"jsonrpc": "2.0", "method": "message", "params": {}}));
    }

    #[test]
    fn params_default_to_an_empty_object() {
        #[derive(serde::Deserialize)]
        struct P {
            #[serde(default)]
            a: Option<u32>,
        }
        assert_eq!(params::<P>(Value::Null).unwrap().a, None);
        assert_eq!(params::<P>(json!({"a": 2, "unknown": 1})).unwrap().a, Some(2));
        assert_eq!(params::<P>(json!({"a": "x"})).err().unwrap().code, INVALID_PARAMS);
    }
}
