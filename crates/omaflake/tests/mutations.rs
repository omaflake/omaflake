// SPDX-License-Identifier: AGPL-3.0-or-later
//! Commands that change state go through the core service (docs/cli.md): on a temporary data folder (`VM_HOME` –
//! never the user's) they start `omaf serve --socket` when it does not run, print what they always printed, and what
//! they changed is what the service answers (`omaf call --socket`) and what the list commands read. Only commands
//! that need neither an image nor Nix; the service is stopped at the end, also when a check fails.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use serde_json::{Value, json};

/// A scratch data folder whose service ends with the test
struct Home {
    _dir: tempfile::TempDir,
    data: PathBuf,
}

impl Home {
    fn new() -> Home {
        let dir = tempfile::Builder::new().prefix("omm").tempdir_in("/tmp").unwrap();
        let data = dir.path().canonicalize().unwrap().join("data");
        std::fs::create_dir_all(&data).unwrap();
        Home { _dir: dir, data }
    }

    fn omaf(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_omaf")).args(args).env("VM_HOME", &self.data).output().expect("omaf runs")
    }

    /// A command that succeeds: its stdout
    fn ok(&self, args: &[&str]) -> String {
        let out = self.omaf(args);
        assert!(out.status.success(), "omaf {args:?}: {}", String::from_utf8_lossy(&out.stderr));
        String::from_utf8_lossy(&out.stdout).into_owned()
    }

    /// A command that fails with exit code `code`: its stderr
    fn fails(&self, args: &[&str], code: i32) -> String {
        let out = self.omaf(args);
        assert_eq!(out.status.code(), Some(code), "omaf {args:?}: {}", String::from_utf8_lossy(&out.stdout));
        String::from_utf8_lossy(&out.stderr).into_owned()
    }

    /// What the service answers
    fn call(&self, method: &str) -> Value {
        let text = self.ok(&["call", "--socket", method]);
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("{method}: {e}: {text}"))
    }

    fn running(&self) -> bool {
        let status: Value = serde_json::from_str(&self.ok(&["service", "status", "--json"])).unwrap();
        status["running"] == json!(true)
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = self.omaf(&["service", "stop"]);
    }
}

fn names(list: &Value) -> Vec<&str> {
    list.as_array().into_iter().flatten().filter_map(|x| x["name"].as_str()).collect()
}

#[test]
fn changes_go_through_the_service() {
    let home = Home::new();
    assert!(!home.running(), "a fresh data folder has no service");

    // Networks: the first command that changes something starts the service
    assert_eq!(
        home.ok(&["networks", "create", "dev"]),
        "Network dev created: 10.231.1.0/24 (the switch is 10.231.1.1).\n"
    );
    assert!(home.running(), "the command started the service");
    assert_eq!(names(&home.call("networks.list")), ["dev"]);
    assert!(home.ok(&["networks"]).starts_with("dev "));
    // The service's refusals are the command's errors
    assert_eq!(home.fails(&["networks", "create", "dev"], 1), "Error: Network \"dev\" exists already\n");
    assert_eq!(home.fails(&["networks", "join", "dev", "nope"], 1), "Error: Instance \"nope\" does not exist\n");
    assert_eq!(
        home.ok(&["networks", "rm", "dev"]),
        "Network dev removed – its members lose that card at their next start.\n"
    );
    assert_eq!(home.call("networks.list"), json!([]));
    assert_eq!(home.fails(&["networks", "rm", "dev"], 1), "Error: No network \"dev\"\n");

    // Volumes
    let folder = home.data.join("share/docs");
    assert_eq!(home.ok(&["volumes", "create", "docs"]), format!("Volume docs created: {}\n", folder.display()));
    assert!(folder.is_dir());
    let volumes = home.call("volumes.list");
    assert_eq!(names(&volumes), ["docs"]);
    assert_eq!(Path::new(volumes[0]["folder"].as_str().unwrap()), folder);
    assert!(home.ok(&["volumes"]).starts_with("docs "));
    assert_eq!(
        home.ok(&["volumes", "rm", "docs"]),
        "Volume docs removed – out of its VMs from their next start; its empty folder is gone.\n"
    );
    assert_eq!(home.call("volumes.list"), json!([]));

    // Profiles: found by their name here, changed by their id there
    assert_eq!(
        home.ok(&["profiles", "add", "Small", "--cpus", "2", "--memory", "4"]),
        "Profile Small added (2 CPU, 4 GB RAM).\n"
    );
    assert_eq!(home.ok(&["profiles", "edit", "Small", "--cpus", "3"]), "Profile Small changed (3 CPU, 4 GB RAM).\n");
    assert_eq!(home.ok(&["profiles", "default", "Small"]), "New VMs get the profile Small.\n");
    let profiles = home.call("profiles.list");
    assert_eq!(names(&profiles), ["Small"]);
    assert_eq!((&profiles[0]["cpus"], &profiles[0]["memoryGB"]), (&json!(3), &json!(4)));
    assert_eq!(profiles[0]["isDefault"], json!(true));
    assert_eq!(home.ok(&["profiles"]), "* Small           3 CPU · 4 GB RAM\n");
    assert_eq!(home.ok(&["profiles", "rm", "Small"]), "Profile Small removed.\n");
    assert_eq!(home.call("profiles.list"), json!([]));
    // A name nobody has is this command's usage error, as before
    assert!(home.fails(&["profiles", "rm", "Small"], 64).starts_with("Error: No profile Small – see omaf profiles\n"));

    // Locales
    assert_eq!(
        home.ok(&[
            "locales",
            "add",
            "Home",
            "--locale",
            "de_CH.UTF-8",
            "--time-zone",
            "Europe/Zurich",
            "--keyboards",
            "ch"
        ]),
        "Locale Home added (de_CH.UTF-8, Europe/Zurich, ch).\n"
    );
    let locales = home.call("locales.list");
    assert_eq!(names(&locales), ["Home"]);
    assert_eq!(locales[0]["keyboards"], json!(["ch"]));
    assert_eq!(home.ok(&["locales", "rm", "Home"]), "Locale Home removed.\n");
    assert_eq!(home.call("locales.list"), json!([]));

    // Sources: a local folder with a recipe, checked by the service before it is kept
    let source = home.data.parent().unwrap().join("recipes-here");
    let recipe = source.join("recipes/me/nixos/test");
    std::fs::create_dir_all(&recipe).unwrap();
    std::fs::write(recipe.join("recipe.toml"), "# only there\n").unwrap();
    let location = source.to_str().unwrap();
    let added = home.ok(&["sources", "add", location]);
    assert!(added.ends_with("Added. Recipes: omaf recipes available\n"), "{added}");
    let locations = |list: &Value| -> Vec<String> {
        list.as_array().into_iter().flatten().filter_map(|s| s["location"].as_str().map(str::to_string)).collect()
    };
    let sources = home.call("sources.list");
    assert_eq!(sources.as_array().map(Vec::len), Some(2), "{sources}");
    assert!(locations(&sources).contains(&location.to_string()), "{sources}");
    assert!(home.fails(&["sources", "add", location], 64).contains("is already a source"));
    assert!(home.ok(&["sources", "disable", "2"]).ends_with(": off\n"));
    assert_eq!(home.call("sources.list")[1]["enabled"], json!(false));
    assert!(home.ok(&["sources", "remove", location]).starts_with("Removed "));
    assert!(!locations(&home.call("sources.list")).contains(&location.to_string()));

    // Settings
    assert!(home.ok(&["config", "--keep-recipe-versions", "on"]).contains("recipe versions: older ones are kept\n"));
    assert_eq!(home.call("settings.get")["settings"]["keepRecipeVersions"], json!(true));
    assert!(
        home.ok(&["config", "--keep-recipe-versions", "off"])
            .contains("recipe versions: the newest and those images use\n")
    );
    assert_eq!(home.call("settings.get")["settings"]["keepRecipeVersions"], json!(false));

    // VMs that are not there: said in the store's words, exit 1
    for args in [&["rm", "nope", "-y"][..], &["reset", "nope", "-y"], &["stop", "nope"], &["set", "nope", "--audio"]] {
        assert_eq!(home.fails(args, 1), "Error: Instance \"nope\" does not exist\n", "{args:?}");
    }

    // Everything above ran in one service, which ends here
    assert_eq!(home.ok(&["service", "stop"]), "Stopped\n");
    assert!(!home.running());
}
