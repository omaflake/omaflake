// SPDX-License-Identifier: AGPL-3.0-or-later
//! The core service end to end (plan B120): `omaf service start` starts `omaf serve --socket` on its own on a
//! temporary data folder (`VM_HOME` – never the user's, and never the user agent: with `VM_HOME` set it is not used),
//! `omaf call --socket` talks to it, `omaf service stop` ends it.

use std::path::Path;
use std::process::{Command, Output};

fn omaf(home: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_omaf")).args(args).env("VM_HOME", home).output().expect("omaf runs")
}

fn json(out: &Output) -> serde_json::Value {
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap_or_else(|e| panic!("{e}: {}", String::from_utf8_lossy(&out.stdout)))
}

fn round_trip(home: &Path) {
    let first = json(&omaf(home, &["service", "start", "--json"]));
    assert_eq!(first["started"], serde_json::json!(true));
    assert_eq!(first["version"], serde_json::json!(env!("CARGO_PKG_VERSION")));
    let socket = first["socket"].as_str().unwrap().to_string();
    let again = json(&omaf(home, &["service", "start", "--json"]));
    assert_eq!((again["started"].as_bool(), again["socket"].as_str()), (Some(false), Some(socket.as_str())));

    let status = json(&omaf(home, &["service", "status", "--json"]));
    assert_eq!((status["running"].as_bool(), status["jobs"].as_u64()), (Some(true), Some(0)));
    assert_eq!(status["socket"].as_str(), Some(socket.as_str()));

    // The same server for every call: its data folder, its (empty) job list
    let init = json(&omaf(home, &["call", "--socket", "initialize", r#"{"apiVersion":1}"#]));
    assert_eq!(Path::new(init["dataFolder"].as_str().unwrap()), home);
    assert_eq!(json(&omaf(home, &["call", &format!("--socket={socket}"), "jobs.list"])), serde_json::json!([]));
    let e = omaf(home, &["call", "--socket", "jobs.cancel", r#"{"id":"j9"}"#]);
    assert_eq!(e.status.code(), Some(1));
    assert!(String::from_utf8_lossy(&e.stderr).contains("j9"));

    let out = omaf(home, &["service", "stop"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let status = json(&omaf(home, &["service", "status", "--json"]));
    assert_eq!(status["running"].as_bool(), Some(false));
    assert!(!Path::new(&socket).exists());
}

#[test]
fn start_call_stop() {
    let dir = tempfile::Builder::new().prefix("oms").tempdir_in("/tmp").unwrap();
    let home = dir.path().canonicalize().unwrap().join("data");
    std::fs::create_dir_all(&home).unwrap();
    round_trip(&home);
}

#[test]
fn a_long_data_folder_gets_a_short_socket() {
    let dir = tempfile::Builder::new().prefix("omsl").tempdir_in("/tmp").unwrap();
    let home = dir.path().canonicalize().unwrap().join("d".repeat(110));
    std::fs::create_dir_all(&home).unwrap();
    let status = json(&omaf(&home, &["service", "status", "--json"]));
    let socket = status["socket"].as_str().unwrap();
    assert!(!socket.starts_with(home.to_str().unwrap()) && socket.len() <= 103, "{socket}");
    round_trip(&home);
}
