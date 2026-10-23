// SPDX-License-Identifier: AGPL-3.0-or-later
//! The `omaf` binary where it does not reach the core: help, version, usage errors and their exit codes.

use std::process::{Command, Output};

fn omaf(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_omaf"))
        .args(args)
        // Nothing here reads the data folder; should it ever, it is not the user's
        .env("VM_HOME", std::env::temp_dir().join("omaf-cli-tests-unused"))
        .output()
        .expect("omaf runs")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[test]
fn version_and_help() {
    let out = omaf(&["--version"]);
    assert_eq!(out.status.code(), Some(0));
    assert_eq!(text(&out.stdout).trim(), format!("omaf {}", env!("CARGO_PKG_VERSION")));

    let out = omaf(&["--help"]);
    assert_eq!(out.status.code(), Some(0));
    let help = text(&out.stdout);
    assert!(help.starts_with("Clean NixOS and macOS VMs from Nix recipes."), "{help}");
    for command in [
        "recipes", "sources", "build", "macos", "images", "new", "set", "start", "stop", "reset", "upgrade", "rm",
        "list", "export", "import", "external", "ssh", "wait", "agent", "log", "config", "doctor", "builder", "serve",
        "call",
    ] {
        assert!(help.contains(&format!("\n  {command} ")), "{command} missing in\n{help}");
    }
}

#[test]
fn a_group_without_its_command_shows_its_help() {
    for group in [&[][..], &["builder"], &["export"], &["macos"]] {
        let out = omaf(group);
        assert_eq!(out.status.code(), Some(0), "{group:?}");
        assert!(text(&out.stdout).contains("Usage: omaf"), "{group:?}");
    }
}

#[test]
fn usage_errors_exit_64() {
    let out = omaf(&["bogus"]);
    assert_eq!(out.status.code(), Some(64));
    let out = omaf(&["stop"]);
    assert_eq!(out.status.code(), Some(64));
    let out = omaf(&["new", "x"]);
    assert_eq!(out.status.code(), Some(64));
    assert_eq!(
        text(&out.stderr),
        "Error: Missing expected argument '--recipe <recipe>'\nUsage: omaf new [OPTIONS] <name>\n  See 'omaf new \
         --help' for more information.\n"
    );
}

#[test]
fn call_refuses_params_that_are_not_json() {
    let out = omaf(&["call", "vm.list", "{name"]);
    assert_eq!(out.status.code(), Some(1));
    assert!(text(&out.stderr).starts_with("Error: The params are not valid JSON: "));
}
