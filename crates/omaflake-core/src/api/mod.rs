// SPDX-License-Identifier: AGPL-3.0-or-later
//! The core API (docs/api.md): JSON-RPC 2.0 over stdio (NDJSON) for every UI – the macOS app, later the Linux UIs,
//! scripts and tests. The logic the Swift app held in its `AppModel` lives here so all of them share it: the build
//! queue, the Doctor's blockers, the recipe list with its freshness, the VM list with "ended" and "ready", the
//! macOS limit, free names, import conflicts, the clipboard agent of macOS guests.
//!
//! - `serve_stdio()` is `omaf serve`: a session over stdin/stdout until stdin closes or `shutdown`.
//! - `service::serve_socket()` is `omaf serve --socket`: the core service, one per user, every client a session on
//!   the same server (plan B120); `client::Client` talks to it.
//! - `call_once(method, params)` is `omaf call`: one request without a session (no events); a method that starts a
//!   job waits for it and returns the ended job.
//! - `Server` is the same dispatcher in-process (a Rust UI): `Server::new(paths, notify)`, then `handle`/`call`.
//!
//! Files: `protocol` (JSON-RPC framing, errors), `types` (the API's types – the contract), `events` (topics, the
//! sink), `jobs` (the job queue), `model` (AppModel's derived state and the lists), `server` (state, watcher),
//! `methods` (the methods), `stdio` (the transport), `service` (the socket service), `client` (its client),
//! `contract` (golden JSON files shared with the Swift tests).

pub mod client;
#[cfg(test)]
mod contract;
pub mod events;
mod jobs;
mod methods;
mod model;
pub mod protocol;
mod server;
pub mod service;
mod stdio;
pub mod types;

use std::sync::Arc;
use std::time::Duration;

use serde_json::{json, Value};

pub use events::{Notify, Topic};
pub use protocol::RpcError;
pub use server::Server;

use crate::paths::Paths;

/// `omaf serve`: serves the user's data folder (`Paths::standard()`) over stdin/stdout until stdin closes or a
/// `shutdown` was answered. Jobs still running are cancelled then; VMs keep running.
pub fn serve_stdio() -> crate::Result<()> {
    stdio::serve(Paths::standard(), std::io::BufReader::new(std::io::stdin()), std::io::stdout());
    Ok(())
}

/// `omaf call <method> [params]`: one request without a session (it initializes itself, sends no events). A method
/// that answers with a job waits for it and returns the ended job (`state` tells whether it finished); several jobs
/// (`recipes.installSuggested`) come as their list.
pub fn call_once(method: &str, params: Value) -> crate::Result<Value> {
    let server = Server::new(Paths::standard(), Arc::new(|_: &str, _: Value| {}));
    if method != "initialize" {
        server.call("initialize", json!({"client": "omaf call", "clientVersion": crate::VERSION, "apiVersion": 1}))?;
    }
    let result = server.call_and_wait(method, params).map_err(crate::Error::from);
    server.close(Duration::from_secs(30));
    result
}

#[cfg(test)]
mod tests {
    use super::events::tests::recorder;
    use super::jobs::{JobCtx, JobSpec};
    use super::types::JobKind;
    use super::*;
    use crate::paths::tests::temp_paths;
    use crate::paths::Settings;

    fn request(id: i64, method: &str, params: Value) -> Value {
        json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
    }

    fn started() -> (tempfile::TempDir, Server, super::events::tests::Recorded) {
        let (dir, paths) = temp_paths();
        let (sink, got) = recorder();
        let server = Server::for_tests(paths, sink);
        let answer = server.handle(request(1, "initialize", json!({"apiVersion": 1, "language": "de"}))).unwrap();
        assert_eq!(answer["result"]["apiVersion"], json!(1), "{answer}");
        (dir, server, got)
    }

    #[test]
    fn requests_before_initialize_are_refused() {
        let (_dir, paths) = temp_paths();
        let (sink, _) = recorder();
        let server = Server::for_tests(paths, sink);
        let answer = server.handle(request(1, "vms.list", Value::Null)).unwrap();
        assert_eq!(answer["error"]["code"], json!(-32002));
        assert_eq!(answer["id"], json!(1));
        // A notification gets no answer
        assert!(server.handle(json!({"jsonrpc": "2.0", "method": "vms.list"})).is_none());
    }

    #[test]
    fn unknown_methods_and_bad_params() {
        let (_dir, server, _) = started();
        let a = server.handle(request(2, "vms.fly", Value::Null)).unwrap();
        assert_eq!(a["error"]["code"], json!(-32601));
        let a = server.handle(request(3, "vms.get", json!({"nom": "x"}))).unwrap();
        assert_eq!(a["error"]["code"], json!(-32602));
        let a = server.handle(request(4, "jobs.cancel", json!({"id": "j1"}))).unwrap();
        assert_eq!(a["error"]["data"]["kind"], json!("not-found"));
        let a = server.handle(request(5, "shutdown", Value::Null)).unwrap();
        assert_eq!(a["result"], Value::Null);
        assert!(server.is_shut_down());
    }

    #[test]
    fn settings_change_only_what_is_given() {
        let (_dir, server, _) = started();
        let a = server
            .call("settings.set", json!({"developerMode": true, "checkForUpdates": false, "flake": "/x/nix"}))
            .unwrap();
        assert_eq!(a["developerMode"], json!(true));
        assert_eq!(a["checkForUpdates"], json!(false));
        assert_eq!(a["flake"], json!("/x/nix"));
        let s = Settings::load(&server.paths());
        assert_eq!((s.developer_mode, s.check_for_updates), (Some(true), Some(false)));
        // null = back to the default (not stored); untouched keys stay
        let a = server
            .call("settings.set", json!({"checkForUpdates": null, "guest": {"user": " ", "locale": "de_CH"}}))
            .unwrap();
        assert_eq!(a["checkForUpdates"], json!(true));
        assert_eq!(a["developerMode"], json!(true));
        assert_eq!(a["guest"], json!({"locale": "de_CH"}));
        let s = Settings::load(&server.paths());
        assert_eq!(s.check_for_updates, None);
        assert_eq!(s.guest.unwrap().user, None);
        let e = server.call("settings.set", json!({"notifications": "yes"})).unwrap_err();
        assert_eq!(e.code, protocol::INVALID_PARAMS);
        // Updates switched off: no feed is asked
        server.call("settings.set", json!({"checkForUpdates": false})).unwrap();
        assert_eq!(server.call("updates.check", Value::Null).unwrap(), json!({"state": "off"}));
    }

    #[test]
    fn jobs_are_listed_logged_and_sent() {
        let (_dir, server, got) = started();
        server.call("events.subscribe", json!({"topics": ["jobs", "unknown-topic"]})).unwrap();
        let job = server.inner.jobs.submit(JobSpec::new(
            JobKind::Export,
            "Export x",
            Box::new(|ctx: &JobCtx| {
                ctx.log("copying");
                Ok(json!({"path": "/tmp/x"}))
            }),
        ));
        job.wait();
        let list = server.call("jobs.list", Value::Null).unwrap();
        assert_eq!(list[0]["state"], json!("finished"));
        assert_eq!(list[0]["result"], json!({"path": "/tmp/x"}));
        let log = server.call("jobs.log", json!({"id": job.id})).unwrap();
        assert_eq!(log, json!({"lines": ["copying"], "next": 1}));
        let methods: Vec<String> = got.lock().unwrap().iter().map(|(m, _)| m.clone()).collect();
        assert!(methods.contains(&"job.log".to_string()) && methods.contains(&"jobs.changed".to_string()));
        assert_eq!(server.call("jobs.dismiss", json!({"id": job.id})).unwrap(), Value::Null);
        assert_eq!(server.call("jobs.list", Value::Null).unwrap(), json!([]));
    }

    #[test]
    fn panics_of_unported_parts_are_internal_errors() {
        let (_dir, server, _) = started();
        // Some core functions are still `todo!()` in this worktree; whatever a method hits, the answer is an error
        // and the server keeps serving. After the merge these return real answers.
        for (method, params) in [("sources.list", json!({})), ("vms.nextName", json!({"recipe": "omaflake.nixos.dev"}))]
        {
            match server.call(method, params) {
                Ok(_) => {}
                Err(e) => assert!(e.code == protocol::SERVER_ERROR, "{method}: {e:?}"),
            }
        }
        assert!(server.call("jobs.list", Value::Null).is_ok());
    }

    #[test]
    fn call_once_shape() {
        // call_once uses the user's data folder – only the error path without touching it is checked here
        let e = Server::for_tests(temp_paths().1, Arc::new(|_: &str, _: Value| {}))
            .call_and_wait("vms.list", Value::Null)
            .unwrap_err();
        assert_eq!(e.code, protocol::NOT_INITIALIZED);
    }
}

/// Integration tests: they need the merged core (no `todo!()` left), a temporary data folder and, for some, Nix and
/// QEMU. Run with `cargo test -p omaflake-core api::integration -- --ignored` after the merge.
#[cfg(test)]
mod integration {
    use super::*;
    use crate::api::events::tests::recorder;
    use crate::paths::tests::temp_paths;

    fn server() -> (tempfile::TempDir, Server, super::events::tests::Recorded) {
        let (dir, paths) = temp_paths();
        let (sink, got) = recorder();
        let server = Server::new(paths, sink);
        server.call("initialize", json!({"apiVersion": 1, "client": "test"})).unwrap();
        (dir, server, got)
    }

    #[test]
    #[ignore = "needs the merged core"]
    fn empty_data_folder_lists_nothing() {
        let (_dir, server, _) = server();
        assert_eq!(server.call("vms.list", Value::Null).unwrap(), json!({"vms": [], "unreadable": []}));
        assert_eq!(server.call("images.list", Value::Null).unwrap(), json!({"images": [], "unreadable": []}));
        assert_eq!(server.call("vms.limits", Value::Null).unwrap()["macRunning"], json!([]));
        let e = server.call("vms.get", json!({"name": "nope"})).unwrap_err();
        assert_eq!(e.kind, Some(crate::error::codes::NOT_FOUND));
        let sources = server.call("sources.list", Value::Null).unwrap();
        assert_eq!(sources[0]["official"], json!(true));
        assert!(server.call("settings.get", Value::Null).unwrap()["choices"]["timeZones"].is_array());
    }

    #[test]
    #[ignore = "needs the merged core"]
    fn subscribing_sends_the_lists() {
        let (_dir, server, got) = server();
        server.call("events.subscribe", json!({"topics": ["vms", "images"]})).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(500));
        let methods: Vec<String> = got.lock().unwrap().iter().map(|(m, _)| m.clone()).collect();
        assert!(methods.contains(&"vms.changed".to_string()));
        assert!(methods.contains(&"images.changed".to_string()));
        server.close(std::time::Duration::from_secs(5));
    }

    #[test]
    #[ignore = "needs the merged core, Nix and a recipe source"]
    fn recipes_and_the_doctor_answer() {
        let (_dir, server, _) = server();
        let report = server.call("doctor.check", Value::Null).unwrap();
        assert!(report["prerequisites"].is_array());
        assert!(report["buildBlockers"]["nixos"].is_array());
        // Without a flake the recipe list is an error, not a panic
        let _ = server.call("recipes.list", Value::Null);
    }
}
