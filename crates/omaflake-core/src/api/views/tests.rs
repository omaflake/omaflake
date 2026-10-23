// SPDX-License-Identifier: AGPL-3.0-or-later
//! Tests of every page's model: built from the contract's fixtures (`api::contract`) – the same values the golden
//! files of the lists hold – and through a server on a temporary data folder.

use std::collections::BTreeMap;

use serde_json::{json, Value};

use super::activity::{ActivityEntry, Notification};
use super::text::Text;
use super::*;
use crate::api::contract as fx;
use crate::api::types::*;
use crate::models::VmStatus;

/// A context from the contract's fixtures: a running NixOS VM, a busy stopped macOS VM, two images, two recipes, the
/// catalogs looked into, the Doctor's report, the jobs of `jobs.list`, an update check, two activity entries
pub(crate) fn ctx() -> Ctx {
    let answer = fx::settings_answer();
    Ctx {
        paths: Paths::new("/data", None),
        vms: vec![fx::vm_full(), fx::vm_mac()],
        unreadable_vms: vec![Unreadable {
            name: "old".into(),
            reason: "written by a newer omaflake (version 9)".into(),
        }],
        images: vec![fx::image(false), fx::image_old()],
        unreadable_images: vec![],
        recipes: fx::recipe_list(),
        jobs: fx::jobs(),
        doctor: Some(fx::doctor()),
        checking: false,
        available: Some(fx::available()),
        fetching: false,
        sources: vec![fx::source_official(), fx::source_private()],
        settings: answer.settings.clone(),
        settings_answer: Some(answer),
        default_data_folder: "/home/.omaflake".into(),
        mac_max: 2,
        activity: vec![
            ActivityEntry {
                id: 2,
                time: fx::date("2026-10-23T09:42:00Z"),
                level: Level::Info,
                text: Text::fmt("%@ is ready", vec![Arg::from("research")]),
                notification: Some(Notification {
                    title: Text::new("VM ready"),
                    body: Text::fmt("%@ is running and answers SSH.", vec![Arg::from("research")]),
                }),
            },
            ActivityEntry {
                id: 1,
                time: fx::date("2026-10-23T09:41:00Z"),
                level: Level::Error,
                text: Text::verbatim("work: running – stop it first"),
                notification: None,
            },
        ],
        update: UpdateRecord {
            state: Some(UpdateState::UpToDate),
            release: None,
            checked: Some(fx::date("2026-10-23T09:00:00Z")),
            error: None,
            checking: false,
        },
        omaf_version: "0.1.0".into(),
        ssh: BTreeMap::from([("research".to_string(), "'/usr/bin/ssh' '-p' '2201' 'alex@127.0.0.1'".to_string())]),
        host_user: "alex".into(),
        host_full_name: "Alex Example".into(),
        host: HostOs::Macos,
        guides: None,
    }
}

fn get(ctx: &Ctx, page: &str, params: Value) -> Value {
    build(ctx, &PageRef { page: page.into(), params }).unwrap()
}

fn ids(actions: &Value) -> Vec<String> {
    actions.as_array().unwrap().iter().map(|a| a["id"].as_str().unwrap().to_string()).collect()
}

fn action<'a>(actions: &'a Value, id: &str) -> &'a Value {
    actions.as_array().unwrap().iter().find(|a| a["id"] == id).unwrap_or_else(|| panic!("no action {id} in {actions}"))
}

// MARK: VMs

#[test]
fn vm_rows_say_what_the_app_said() {
    let c = ctx();
    let page = get(&c, "vms", json!({}));
    assert_eq!(page["title"]["key"], "Virtual Machines");
    assert_eq!(page["count"], 2);
    assert!(page.get("of").is_none() && page.get("empty").is_none());
    assert_eq!(page["unreadable"][0]["text"]["text"], "old: written by a newer omaflake (version 9)");
    let research = &page["rows"][0];
    assert_eq!(research["lamp"]["state"], "on");
    assert_eq!(research["subtitle"]["text"], "dev (velvet) · 4 CPU · 8 GB · SSH 2201");
    let badges: Vec<&str> = research["badges"].as_array().unwrap().iter().map(|b| b["id"].as_str().unwrap()).collect();
    assert_eq!(badges, ["external", "clipboard", "sound", "upgradable"]);
    // Running: share folder, terminal, stop
    assert_eq!(ids(&research["actions"]), ["showShare", "terminal", "stop"]);
    assert_eq!(action(&research["actions"], "terminal")["then"], "terminal");
    assert_eq!(action(&research["actions"], "showShare")["then"], "reveal");
    // Its details while it runs: everything that changes it waits for the shutdown
    let d = &research["detail"];
    assert_eq!(d["switchesNote"]["key"], "Changes take effect at the next start.");
    assert_eq!(ids(&d["actions"]), ["export", "forget", "upgrade", "reset", "remove"]);
    for a in d["actions"].as_array().unwrap() {
        assert_eq!(a["enabled"], false);
        assert_eq!(a["reason"]["key"], "Shut the VM down first");
    }
    let remove = action(&d["actions"], "remove");
    assert_eq!(remove["confirm"]["title"]["text"], "Remove “research”?");
    assert_eq!(
        remove["confirm"]["text"]["key"],
        "The VM runs from %@ – it is deleted there and cannot be restored. Its share folder is kept if it holds files. To keep it there, choose Stop Listing instead."
    );
    assert_eq!(remove["confirm"]["destructive"], true);
    assert_eq!(
        action(&d["actions"], "upgrade")["params"],
        json!({"name": "research", "image": "omaflake.nixos.dev@mint-river-cloud"})
    );
    assert_eq!(action(&d["actions"], "export")["ask"][0]["param"], "folder");
    let info: Vec<&str> = d["info"].as_array().unwrap().iter().map(|l| l["label"]["key"].as_str().unwrap()).collect();
    assert_eq!(
        info,
        [
            "Image",
            "omaflake ID",
            "Recipe",
            "Resources",
            "Network",
            "SSH",
            "Share folder",
            "Created",
            "Runs from",
            "Agent",
            "Guest"
        ]
    );
    assert_eq!(d["info"][1]["value"]["text"], "vm:research:amber");
    assert_eq!(d["info"][0]["note"]["key"], "newer image available");
    // The microphone switch turns sound on with it
    assert_eq!(
        action(&d["switches"], "microphone")["params"],
        json!({"name": "research", "microphone": true, "audio": true})
    );
    assert_eq!(action(&d["switches"], "sound")["on"], true);

    // The macOS VM is busy: the lamp works, no buttons; its camera is not available
    let work = &page["rows"][1];
    assert_eq!(work["lamp"]["state"], "working");
    assert_eq!(work["actions"], json!([]));
    let camera = action(&work["detail"]["switches"], "camera");
    assert_eq!((camera["enabled"].clone(), camera["on"].clone()), (json!(false), json!(false)));
    let badges: Vec<&str> = work["badges"].as_array().unwrap().iter().map(|b| b["icon"].as_str().unwrap()).collect();
    assert_eq!(badges, ["microphone", "ephemeral", "network-isolated"]);
    assert_eq!(work["detail"]["info"][5]["value"]["key"], "address while running");
}

#[test]
fn vms_filter_and_start_rules() {
    let mut c = ctx();
    let page = get(&c, "vms", json!({"filter": {"status": true}}));
    assert_eq!((page["count"].clone(), page["of"].clone()), (json!(1), json!(2)));
    assert_eq!(page["filter"]["active"], true);
    assert_eq!(page["filter"]["groups"][0]["value"], true);
    let none = get(&c, "vms", json!({"filter": {"kind": "macos", "status": true}}));
    assert_eq!(none["empty"]["text"]["key"], "Nothing matches the filter.");
    assert_eq!(none["empty"]["action"]["method"], "ui.filter");

    // Stopped NixOS VM with QEMU missing: the Doctor's warning instead of the start buttons
    c.vms[0].status = VmStatus::Stopped;
    let row = &get(&c, "vms", json!({}))["rows"][0];
    assert_eq!(ids(&row["actions"]), ["showShare", "doctor"]);
    assert_eq!(row["actions"][1]["help"]["text"], "Open the Doctor first: QEMU (NixOS guests)");
    assert_eq!(row["detail"]["actions"][0]["enabled"], true);

    // Without blockers it starts; two macOS VMs run: a third waits
    c.doctor.as_mut().unwrap().start_blockers.nixos.clear();
    let row = &get(&c, "vms", json!({}))["rows"][0];
    assert_eq!(ids(&row["actions"]), ["showShare", "startHeadless", "startWindow"]);
    assert_eq!(action(&row["actions"], "startWindow")["params"], json!({"name": "research", "display": "auto"}));
    c.vms[1].busy = false;
    for n in ["m1", "m2"] {
        let mut m = fx::vm_mac();
        m.name = n.into();
        m.status = VmStatus::Running;
        m.busy = false;
        c.vms.push(m);
    }
    let page = get(&c, "vms", json!({}));
    let work = page["rows"].as_array().unwrap().iter().find(|r| r["name"] == "work").unwrap();
    let start = action(&work["actions"], "startHeadless");
    assert_eq!(start["enabled"], false);
    assert_eq!(start["reason"]["text"], "macOS allows at most 2 macOS VMs at the same time – running: m1, m2");

    c.vms.clear();
    assert_eq!(get(&c, "vms", json!({}))["empty"]["text"]["key"], "No VMs yet – create one from an image.");
    let e = build(&c, &PageRef { page: "vm".into(), params: json!({"name": "nope"}) }).unwrap_err();
    assert_eq!(e.kind, Some(crate::error::codes::NOT_FOUND));
}

#[test]
fn one_vm_is_its_row() {
    let c = ctx();
    let page = get(&c, "vm", json!({"name": "work"}));
    assert_eq!(page["row"]["name"], "work");
    assert_eq!(page["row"]["vm"]["kind"], "macos");
}

// MARK: Images

#[test]
fn image_rows_and_builds() {
    let c = ctx();
    let page = get(&c, "images", json!({}));
    // Builds that wait or run on top (finished ones are images), images below
    let jobs: Vec<&str> = page["jobs"].as_array().unwrap().iter().map(|j| j["id"].as_str().unwrap()).collect();
    assert_eq!(jobs, ["j1", "j2"]);
    assert_eq!(page["jobs"][0]["title"]["text"], "3/5 Copying the image");
    assert_eq!(page["jobs"][0]["count"]["text"], "120/400");
    assert_eq!(
        page["jobs"][0]["open"]["params"],
        json!({"page": "build", "params": {"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"}})
    );
    assert_eq!(page["jobs"][1]["title"]["key"], "Waiting to build");
    assert_eq!(page["jobs"][1]["actions"][0]["label"]["key"], "Remove from the queue");
    let used = &page["rows"][0];
    assert_eq!(used["lamp"]["state"], "on");
    assert_eq!(used["subtitle"]["args"][2]["key"], "1 VM");
    assert_eq!(ids(&used["actions"]), ["reveal", "doctor"], "QEMU missing: no new VM from a NixOS image");
    let remove = action(&used["detail"]["actions"], "remove");
    assert_eq!(
        (remove["enabled"].clone(), remove["reason"]["key"].clone()),
        (json!(false), json!("Used by VMs – remove or upgrade them first"))
    );
    let mac = &page["rows"][1];
    assert_eq!(mac["lamp"]["state"], "off");
    assert_eq!(
        action(&mac["actions"], "newVm")["params"],
        json!({"page": "newVm", "params": {"image": "omaflake.macos.dev@stone-lamp"}})
    );
    assert_eq!(mac["detail"]["info"][3]["value"]["key"], "no git revision");
    // The recipe filter offers the images' recipes; status "in use" hides the builds
    // "In use" as the app counts it: an image a VM names (both fixture VMs do)
    let filtered = get(&c, "images", json!({"filter": {"status": true}}));
    assert_eq!((filtered["count"].clone(), filtered["jobs"].clone()), (json!(2), json!([])));
    let unused = get(&c, "images", json!({"filter": {"status": false, "kind": "nixos"}}));
    assert_eq!((unused["count"].clone(), unused["jobs"].as_array().unwrap().len()), (json!(0), 1));
    let groups = page["filter"]["groups"].as_array().unwrap();
    assert_eq!(groups.last().unwrap()["id"], "recipe");
    assert_eq!(groups.last().unwrap()["options"].as_array().unwrap().len(), 3);
    let one = get(&c, "image", json!({"name": "omaflake.nixos.dev@cedar-mango-lift"}));
    assert_eq!(one["row"]["image"]["instances"], json!(["research"]));
}

// MARK: Recipes

#[test]
fn recipe_rows_by_line_with_offers_and_updates() {
    let c = ctx();
    let page = get(&c, "recipes", json!({}));
    // The lab recipe is for another architecture: hidden unless asked
    assert_eq!((page["count"].clone(), page["of"].clone()), (json!(1), json!(2)));
    let dev = &page["rows"][0];
    assert_eq!(dev["lamp"]["state"], "working", "j1 builds it");
    assert_eq!(dev["secondary"], json!(["velvet"]));
    assert_eq!(ids(&dev["actions"]), ["reveal", "doctor"], "the Linux builder does not answer");
    let d = &dev["detail"];
    assert_eq!(d["info"][3]["value"]["text"], "Clipboard · Sound · Share folder");
    assert_eq!(d["info"][4]["value"]["key"], "%lld built, the newest %@");
    assert_eq!(ids(&d["actions"]), ["update", "removeAll"]);
    assert_eq!(action(&d["actions"], "removeAll")["enabled"], true, "the older version has no image");
    let versions = d["versions"].as_array().unwrap();
    assert_eq!(versions[0]["words"], "velvet");
    assert_eq!(versions[0]["state"]["text"], "2026-10-20 08:00 UTC · newest · images: 1");
    assert_eq!(action(&versions[0]["actions"], "remove")["reason"]["key"], "Kept while images built from it are there");
    assert_eq!(action(&versions[1]["actions"], "remove")["enabled"], true);

    let all = get(&c, "recipes", json!({"filter": {"otherSystems": true}}));
    assert_eq!(all["count"], 2);
    let lab = &all["rows"][1];
    assert_eq!(lab["lamp"]["state"], "off");
    let badges: Vec<&str> = lab["badges"].as_array().unwrap().iter().map(|b| b["id"].as_str().unwrap()).collect();
    assert_eq!(badges, ["lab", "arch"]);
    assert_eq!(lab["actions"][1]["enabled"], false);
    assert_eq!(lab["actions"][1]["reason"]["text"], "Not for this Mac – it is built for aarch64-darwin");

    // Offered: not installed, from the official source
    let offered: Vec<&str> = page["offered"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap()).collect();
    assert_eq!(offered, ["omaflake.nixos.minimal", "lab.nixos.try"]);
    assert_eq!(page["offered"][0]["actions"][0]["label"]["text"], "Install from omaflake/recipes");
    assert!(page["offered"][0]["detail"].get("warning").is_none());
    assert_eq!(
        page["offered"][1]["detail"]["warning"]["key"],
        "Not reviewed by the omaflake team – its Nix code runs on this Mac when you build it."
    );
    assert_eq!(page["updates"]["rows"][0]["action"]["label"]["key"], "Update");
    assert!(page["updates"].get("all").is_none());
    assert_eq!(page["errors"][0]["text"]["text"], "example.org: HTTP 404");
    assert!(page.get("suggestion").is_none(), "there are recipes already");
    assert_eq!(page["catalogsLoaded"], true);
    let tags = get(&c, "recipes", json!({"filter": {"tag": "gui"}}));
    assert_eq!((tags["count"].clone(), tags["offered"].clone()), (json!(1), json!([])));
}

#[test]
fn recipes_at_the_first_start() {
    let mut c = ctx();
    c.recipes.recipes.clear();
    let page = get(&c, "recipes", json!({}));
    assert_eq!(page["suggestion"]["action"]["method"], "recipes.installSuggested");
    c.available = None;
    c.fetching = true;
    let page = get(&c, "recipes", json!({}));
    assert_eq!(page["empty"]["text"]["key"], "Looking into the recipe sources …");
    assert_eq!((page["catalogsLoaded"].clone(), page["loading"].clone()), (json!(false), json!(true)));
    assert_eq!(action(&page["actions"], "refresh")["busy"], true);
}

// MARK: New VM

#[test]
fn new_vm_defaults_and_validation() {
    let c = ctx();
    let page = get(&c, "newVm", json!({"image": "omaflake.nixos.dev@cedar-mango-lift"}));
    let create = action(&page["actions"], "create");
    assert_eq!(create["enabled"], true);
    assert_eq!(create["then"], "back");
    assert_eq!(create["params"]["name"], "dev", "the next free name");
    assert_eq!(create["params"]["settings"]["cpus"], 4);
    assert_eq!(create["params"]["settings"]["display"], "gpu");
    assert_eq!(page["subtitle"]["text"], "recipe dev (velvet)");
    assert_eq!(page["blockers"]["items"][0]["title"]["key"], "QEMU (NixOS guests)");
    let fields: Vec<&str> = page["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["fields"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()))
        .collect();
    assert_eq!(
        fields,
        [
            "name",
            "ephemeral",
            "cpus",
            "memoryGB",
            "diskGB",
            "audio",
            "microphone",
            "clipboard",
            "camera",
            "network",
            "share",
            "sharePath",
            "start"
        ]
    );
    let share = &page["sections"][2]["fields"][1];
    assert_eq!(share["placeholder"]["text"], "/data/share/dev");
    // Values the UI sends back: kept, checked
    let page = get(
        &c,
        "newVm",
        json!({"image": "omaflake.nixos.dev@cedar-mango-lift", "values": {"name": "research", "audio": false, "microphone": true, "cpus": 2}}),
    );
    let name = &page["sections"][0]["fields"][0];
    assert_eq!(name["error"]["text"], "“research” is here already");
    assert_eq!(page["valid"], false);
    let create = action(&page["actions"], "create");
    assert_eq!(create["enabled"], false);
    assert_eq!(create["params"]["settings"]["microphone"], false, "no microphone without sound");
    assert_eq!(page["sections"][1]["fields"][0]["label"]["text"], "CPU cores: 2");
    let bad = get(&c, "newVm", json!({"image": "omaflake.nixos.dev@cedar-mango-lift", "values": {"name": "Bad Name"}}));
    assert!(bad["sections"][0]["fields"][0]["error"]["text"].as_str().unwrap().starts_with("Invalid name"));

    // macOS: no isolated network, the disk as the image, no camera
    let mac = get(&c, "newVm", json!({"image": "omaflake.macos.dev@stone-lamp", "values": {"network": "isolated"}}));
    let resources = &mac["sections"][1]["fields"];
    let network = action(resources, "network");
    assert_eq!(network["value"], "shared");
    assert_eq!(network["choices"].as_array().unwrap().len(), 2);
    assert_eq!(action(resources, "diskGB")["kind"], "info");
    assert!(resources.as_array().unwrap().iter().all(|f| f["id"] != "camera"));
    let e = build(&c, &PageRef { page: "newVm".into(), params: json!({"image": "nope"}) }).unwrap_err();
    assert_eq!(e.kind, Some(crate::error::codes::NOT_FOUND));
}

// MARK: Build

#[test]
fn build_page_follows_the_job() {
    let mut c = ctx();
    let page = get(&c, "build", json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"}));
    assert_eq!(page["state"], "running");
    assert_eq!(ids(&page["actions"]), ["cancel"]);
    assert_eq!(page["progress"]["step"]["text"], "3/5");
    assert_eq!(page["progress"]["title"]["key"], "Copying the image");
    assert_eq!(page["log"]["job"], "j1");
    assert!(page.get("blockers").is_none(), "not while it runs");
    // Queued (macOS): Remove from Queue
    let page = get(&c, "build", json!({"recipe": "omaflake.macos.dev@ocean-maple"}));
    assert_eq!(
        (page["state"].clone(), page["status"]["key"].clone()),
        (json!("queued"), json!("Waiting in the queue"))
    );
    assert_eq!(page["actions"][0]["label"]["key"], "Remove from Queue");
    // Ended: the outcome and Build again – blocked by the Doctor
    c.jobs[0].state = JobState::Failed;
    c.jobs[0].error = Some("nix build failed".into());
    c.jobs[0].hidden_lines = 3;
    let page = get(&c, "build", json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"}));
    assert_eq!(page["outcome"]["title"]["key"], "Build failed");
    assert_eq!(page["outcome"]["detail"]["text"], "nix build failed");
    let build = action(&page["actions"], "build");
    assert_eq!(build["enabled"], false);
    assert_eq!(build["reason"]["text"], "Open the Doctor first: Linux builder answers");
    assert_eq!(
        page["blockers"]["items"][0]["hint"]["key"],
        "The Linux builder VM does not answer – it is started by nix-darwin (launchd: org.nixos.linux-builder)."
    );
    assert!(page["log"]["note"]["text"].as_str().unwrap().starts_with("(3 routine lines"));
    // Finished: New VM from the image
    c.jobs[0].state = JobState::Finished;
    c.jobs[0].result = Some(json!({"image": "omaflake.nixos.dev@cedar-mango-lift", "removed": []}));
    let page = get(&c, "build", json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"}));
    assert_eq!(page["outcome"]["action"]["params"]["params"]["image"], "omaflake.nixos.dev@cedar-mango-lift");
    // A macOS recipe asks the password of the base's user; queued behind another macOS build
    c.recipes.recipes[0].runs_here = true;
    c.jobs.retain(|j| j.id != "j2");
    c.doctor.as_mut().unwrap().build_blockers.macos.clear();
    let page = get(&c, "build", json!({"recipe": "lab.macos.base@ocean-maple"}));
    let build = action(&page["actions"], "build");
    assert_eq!(build["ask"][0]["kind"], "password");
    assert_eq!(build["ask"][0]["label"]["text"], "Password of alex");
    assert_eq!(build["enabled"], true);
    assert_eq!(page["state"], "idle");
}

// MARK: Doctor

#[test]
fn doctor_items_hints_and_actions() {
    let c = ctx();
    let page = get(&c, "doctor", json!({}));
    let sections: Vec<&str> = page["sections"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(sections, ["requirements", "nixos", "macos"]);
    assert_eq!(page["issues"], 3);
    let req = &page["sections"][0]["items"];
    let nix = &req[0];
    assert_eq!(nix["status"], "ok");
    assert_eq!(ids(&nix["actions"]), ["install", "setPath"]);
    assert_eq!(action(&nix["actions"], "install")["method"], "ui.terminal");
    assert_eq!(nix["expanded"], false);
    let qemu = &req[1];
    assert_eq!(qemu["hint"]["key"], "Needed for NixOS guests: install omaflake's QEMU, or enter a path.");
    assert!(qemu.get("detail").is_none(), "a missing item says it all in its hint");
    assert_eq!((qemu["expandable"].clone(), qemu["expanded"].clone()), (json!(true), json!(true)));
    let set = action(&qemu["actions"], "setPath");
    assert_eq!((set["params"].clone(), set["ask"][0]["kind"].clone()), (json!({"item": "qemu"}), json!("file")));
    let mac = &page["sections"][2]["items"];
    assert_eq!(mac[0]["item"], "ipsw");
    assert_eq!(mac[0]["busy"], true, "j4 downloads");
    assert_eq!(mac[0]["download"]["text"]["text"], "1.0 GB / 18.0 GB");
    assert_eq!(action(&mac[0]["actions"], "pause")["params"], json!({"id": "j4"}));
    assert!(mac[1]["hint"]["text"].as_str().unwrap().starts_with("The base's user alex differs from macUser sam"));
    assert_eq!(mac[2]["detail"]["key"], "%@ free, about %@ needed");
    assert_eq!(page["update"]["text"]["key"], "Up to date – checked %@");
    assert_eq!(ids(&page["update"]["actions"]), ["checkNow"]);
    assert_eq!(ids(&page["actions"]), ["check"]);
}

#[test]
fn doctor_linux_builder_tasks() {
    use crate::prerequisites::{PrerequisiteItem as I, PrerequisiteStatus as S};
    let mut c = ctx();
    let builder = fx::prerequisite(I::LinuxBuilder, S::Ok);
    c.doctor.as_mut().unwrap().prerequisites.push(builder);
    let page = get(&c, "doctor", json!({}));
    let item = page["sections"][0]["items"].as_array().unwrap().iter().find(|i| i["item"] == "linuxBuilder").unwrap();
    // j13 (apply) runs: the tasks wait, the row says which runs
    assert_eq!(item["busy"], true);
    assert_eq!(item["task"]["text"], "BuilderApply");
    assert_eq!(ids(&item["actions"]), ["cleanUp", "apply", "guide"]);
    assert_eq!(action(&item["actions"], "apply")["enabled"], false);
    c.jobs.retain(|j| j.id != "j13");
    let page = get(&c, "doctor", json!({}));
    let item = page["sections"][0]["items"].as_array().unwrap().iter().find(|i| i["item"] == "linuxBuilder").unwrap();
    let apply = action(&item["actions"], "apply");
    // This Mac's nix-darwin system is not omaflake's: applying replaces it
    assert_eq!(apply["params"], json!({"force": true}));
    assert_eq!(apply["confirm"]["button"]["key"], "Replace and Apply");
}

#[test]
fn mac_base_page() {
    let c = ctx();
    let page = get(&c, "macBase", json!({}));
    assert_eq!(page["fields"][0]["value"], "alex");
    assert_eq!(page["fields"][1]["value"], "Alex Example");
    assert_eq!(page["base"]["tone"], "warning");
    let create = action(&page["actions"], "create");
    assert_eq!(create["ask"][0]["repeat"], true);
    assert_eq!(create["enabled"], false, "the IPSW downloads");
    assert_eq!(ids(&page["ipsw"]["actions"]), ["pause"]);
    let mut c = ctx();
    c.jobs.retain(|j| j.id != "j4");
    let page = get(&c, "macBase", json!({"values": {"ipsw": "/x/UniversalMac.ipsw", "user": "sam"}}));
    assert_eq!(page["ipsw"]["text"]["text"], "UniversalMac.ipsw");
    assert_eq!(ids(&page["ipsw"]["actions"]), ["useApple", "chooseIpsw"]);
    let create = action(&page["actions"], "create");
    assert_eq!(create["params"], json!({"user": "sam", "fullName": "Alex Example", "ipsw": "/x/UniversalMac.ipsw"}));
    assert_eq!(create["enabled"], true);
    assert_eq!(page["log"]["job"], "j6", "the base's creation is a job like any other");
    assert_eq!(page["log"]["from"], 0);
    // The IPSW is shown once: in its line, set with ui.field on this page
    assert!(page["fields"].as_array().unwrap().iter().all(|f| f["id"] != "ipsw"));
    let choose = action(&page["ipsw"]["actions"], "chooseIpsw");
    assert_eq!(choose["params"], json!({"page": "macBase", "params": {}, "field": "ipsw", "valueParam": "ipsw"}));
    assert_eq!(action(&page["ipsw"]["actions"], "useApple")["params"]["value"], Value::Null);
    let user = action(&page["fields"], "user");
    assert_eq!((user["param"].clone(), user["valueParam"].clone()), (json!("user"), json!("user")));
    assert_eq!(create["primary"], true);
}

// MARK: Settings

#[test]
fn settings_fields_store_themselves() {
    let c = ctx();
    let page = get(&c, "settings", json!({}));
    let rows = &page["sources"]["rows"];
    assert_eq!(rows[0]["actions"][0]["enabled"], false, "the first source cannot move up");
    assert_eq!(rows[1]["badges"][0]["help"]["key"], "Private – token in the Keychain");
    assert_eq!(
        action(&rows[1]["actions"], "enabled")["params"],
        json!({"id": "url:https://example.org/catalog.json", "enabled": true})
    );
    let dev = action(&page["general"]["fields"], "developerMode");
    assert_eq!(dev["set"], json!({"method": "settings.set", "params": {}, "param": "developerMode"}));
    assert_eq!(dev["value"], true);
    let locale = action(&page["guest"]["fields"], "locale");
    assert_eq!(locale["set"]["param"], "guest.locale");
    assert_eq!(locale["set"]["params"], json!({"guest": {"user": "alex", "locale": "de_CH.UTF-8", "keyboard": "ch"}}));
    assert_eq!(locale["choices"][0]["label"]["text"], "Like this Mac – de_CH.UTF-8");
    assert_eq!(action(&page["guest"]["fields"], "user")["placeholder"]["text"], "mac");
    // A VM runs: the data folder stays
    let df = &page["dataFolder"];
    assert_eq!(action(&df["actions"], "move")["reason"]["key"], "Stop all VMs before moving the data folder.");
    assert_eq!(action(&df["actions"], "move")["confirm"]["title"]["args"][0], json!({"param": "target"}));
    // The new source's form: private only for GitHub, the token is asked when sent
    assert_eq!(page["sources"]["addAction"]["enabled"], false);
    let page = get(&c, "settings", json!({"values": {"input": "acme/recipes", "private": true}}));
    let add = &page["sources"]["addAction"];
    assert_eq!(add["params"], json!({"input": "acme/recipes", "private": true}));
    assert_eq!(add["ask"][0]["param"], "token");
    assert_eq!(page["sources"]["add"]["fields"].as_array().unwrap().len(), 2);
    let page = get(&c, "settings", json!({"values": {"input": "https://example.org/c.json", "private": true}}));
    assert_eq!(page["sources"]["addAction"]["params"]["private"], false);
    assert!(settings::looks_like_github("https://github.com/a/b") && !settings::looks_like_github("~/recipes"));
}

// MARK: Home, activity, import

#[test]
fn home_tabs_and_header() {
    let c = ctx();
    let page = get(&c, "home", json!({}));
    assert_eq!(page["actions"][0]["badge"], 3);
    assert_eq!(action(&page["actions"], "quit")["confirm"]["text"]["args"][0], "research");
    assert_eq!(
        page["tabs"][0]["badge"],
        json!({"kind": "count", "count": 1, "tone": "success", "help": {"key": "Running", "text": "Running"}})
    );
    assert_eq!(page["tabs"][1]["badge"]["kind"], "working");
    assert_eq!(page["tabs"][2]["badge"]["kind"], "dot");
    assert!(page["tabs"][3].get("badge").is_none(), "unseen activity is the UI's");
    assert_eq!(page["firstTab"], "vms");
}

#[test]
fn activity_page_and_job_wording() {
    let c = ctx();
    let page = get(&c, "activity", json!({}));
    assert_eq!(page["entries"][0]["notification"]["title"]["key"], "VM ready");
    assert_eq!(action(&page["actions"], "clear")["method"], "activity.clear");
    assert_eq!(page["notifications"], json!(c.settings.notifications));
    for (p, params, title) in [
        ("doctor", json!({}), "Doctor"),
        ("settings", json!({}), "Settings"),
        ("newVm", json!({"image": "omaflake.nixos.dev@cedar-mango-lift"}), "New VM"),
        ("build", json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"}), "Build Image"),
        ("macBase", json!({}), "Create macOS Base"),
    ] {
        assert_eq!(get(&c, p, params)["title"]["key"], title);
    }
    let empty = activity::page(&[], true);
    assert_eq!(empty.head.empty.unwrap().text.key.as_deref(), Some("Nothing happened yet in this session."));
    let jobs = fx::jobs();
    let worded: Vec<Option<String>> = jobs.iter().map(|j| activity::job_ended(j).map(|(_, t, _)| t.text)).collect();
    assert_eq!(worded[2].as_deref(), Some("Image Build ready"), "{worded:?}");
    assert_eq!(worded[4].as_deref(), Some("macOS 27.0.1 downloaded"));
    assert_eq!(worded[7].as_deref(), Some("Export exported to /Volumes/Disk/research.omaflake-vm"));
    assert_eq!(worded[9].as_deref(), Some("Installed omaflake/nixos/minimal"));
    assert_eq!(worded[10].as_deref(), Some("Failed – see /data/logs/j11.log\nit went wrong"));
    assert_eq!(worded[0], None, "running");
    let (_, _, n) = activity::job_ended(&JobDto { state: JobState::Failed, ..jobs[0].clone() }).unwrap();
    assert_eq!(n.unwrap().body.text, "dev – the log is in the Images tab.");
}

#[test]
fn import_step_asks_about_what_is_here() {
    let vm = Inspection {
        kind: PackageKindDto::Vm,
        valid: true,
        reason: None,
        conflict: Some(ConflictDto {
            vm_name: Some("research".into()),
            suggested_name: Some("research-2".into()),
            image: None,
        }),
        network_volume: false,
    };
    let p = import::ImportParams { path: "/x/research.omaflake-vm".into(), expect: None, external: false };
    let page = serde_json::to_value(import::page(&p, vm.clone())).unwrap();
    assert_eq!(page["action"]["confirm"]["input"], json!({"param": "name", "default": "research-2"}));
    assert_eq!(page["action"]["confirm"]["title"]["text"], "“research” is here already");
    let image = Inspection {
        conflict: Some(ConflictDto { vm_name: None, suggested_name: None, image: Some("i".into()) }),
        ..vm.clone()
    };
    let page = serde_json::to_value(import::page(&p, image)).unwrap();
    assert_eq!(page["action"]["params"], json!({"path": "/x/research.omaflake-vm", "replace": true}));
    let plain = serde_json::to_value(import::page(&p, Inspection { conflict: None, ..vm.clone() })).unwrap();
    assert!(plain["action"].get("confirm").is_none());
    let ext = import::ImportParams { external: true, ..p };
    let page =
        serde_json::to_value(import::page(&ext, Inspection { network_volume: true, conflict: None, ..vm.clone() }))
            .unwrap();
    assert_eq!(page["action"]["method"], "vms.useExternal");
    assert_eq!(page["action"]["confirm"]["title"]["key"], "Run the VM over the network?");
    let bad = Inspection {
        valid: false,
        reason: Some(Text::verbatim("Not a VM package (.omaflake-vm)")),
        conflict: None,
        ..vm
    };
    let page = serde_json::to_value(import::page(&ext, bad)).unwrap();
    assert_eq!(page["reason"]["text"], "research.omaflake-vm: Not a VM package (.omaflake-vm)");
    assert!(page.get("action").is_none());
}

// MARK: What the interfaces needed (plan B121)

#[test]
fn primary_actions_and_short_labels() {
    let mut c = ctx();
    let primaries = |actions: &Value| -> Vec<String> {
        actions.as_array().unwrap().iter().filter(|a| a["primary"] == true).map(|a| s(a, "id")).collect()
    };
    fn s(a: &Value, k: &str) -> String {
        a[k].as_str().unwrap().to_string()
    }
    let vms = get(&c, "vms", json!({}));
    assert_eq!(primaries(&vms["rows"][0]["actions"]), ["terminal"], "running: its terminal");
    let images = get(&c, "images", json!({}));
    assert_eq!(images["jobs"][0]["open"]["primary"], true, "a build row opens its page");
    assert_eq!(primaries(&images["rows"][0]["actions"]), ["doctor"], "blocked: the way to the Doctor");
    assert_eq!(primaries(&images["rows"][1]["actions"]), ["newVm"]);
    let recipes = get(&c, "recipes", json!({}));
    assert_eq!(primaries(&recipes["offered"][0]["actions"]), ["install"]);
    assert_eq!(recipes["updates"]["rows"][0]["action"]["primary"], true);
    let new_vm = get(&c, "newVm", json!({"image": "omaflake.nixos.dev@cedar-mango-lift"}));
    assert_eq!(primaries(&new_vm["actions"]), ["create"]);
    let settings = get(&c, "settings", json!({}));
    assert_eq!(settings["sources"]["addAction"]["primary"], true);
    assert!(settings["sources"]["rows"][0]["actions"].as_array().unwrap().iter().all(|a| a.get("primary").is_none()));
    c.vms[0].status = VmStatus::Stopped;
    c.doctor.as_mut().unwrap().start_blockers.nixos.clear();
    assert_eq!(primaries(&get(&c, "vms", json!({}))["rows"][0]["actions"]), ["startWindow"]);

    let home = get(&c, "home", json!({}));
    let short: Vec<Option<&str>> =
        home["actions"].as_array().unwrap().iter().map(|a| a["shortLabel"]["key"].as_str()).collect();
    assert_eq!(short, [Some("Doctor"), None, Some("Data Folder"), Some("Quit")]);
}

#[test]
fn form_fields_name_their_value_and_every_page_has_a_title() {
    let c = ctx();
    let new_vm = get(&c, "newVm", json!({"image": "omaflake.nixos.dev@cedar-mango-lift"}));
    for f in new_vm["sections"].as_array().unwrap().iter().flat_map(|s| s["fields"].as_array().unwrap()) {
        assert_eq!(f["valueParam"], f["id"], "{f}");
    }
    assert_eq!(action(&new_vm["sections"][1]["fields"], "memoryGB")["param"], "settings.memoryGB");
    let mac = get(&c, "newVm", json!({"image": "omaflake.macos.dev@stone-lamp"}));
    assert!(action(&mac["sections"][1]["fields"], "diskGB").get("valueParam").is_none(), "shown, not edited");
    let settings = get(&c, "settings", json!({"values": {"input": "acme/recipes"}}));
    let add: Vec<&str> = settings["sources"]["add"]["fields"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["valueParam"].as_str().unwrap())
        .collect();
    assert_eq!(add, ["input", "private"]);
    assert!(settings["general"]["fields"][0].get("valueParam").is_none(), "stored at once");

    for (p, params, title) in [
        ("home", json!({}), "omaflake"),
        ("vms", json!({}), "Virtual Machines"),
        ("vm", json!({"name": "research"}), "research"),
        ("images", json!({}), "Images"),
        ("image", json!({"name": "omaflake.macos.dev@stone-lamp"}), "omaflake.macos.dev@stone-lamp"),
        ("recipes", json!({}), "Recipes"),
        ("activity", json!({}), "Activity"),
    ] {
        assert_eq!(get(&c, p, params)["title"]["text"], title);
    }
}

#[test]
fn recipes_say_what_they_show() {
    let mut c = ctx();
    let page = get(&c, "recipes", json!({}));
    assert_eq!(page["offeredTitle"]["key"], "More recipes in your sources");
    assert!(page.get("loadingText").is_none() && page.get("labHint").is_none());
    c.fetching = true;
    c.recipes.lab_hidden = true;
    let page = get(&c, "recipes", json!({}));
    assert_eq!(page["loadingText"]["key"], "Looking into the recipe sources …");
    assert_eq!(page["labHint"]["text"]["key"], "Lab recipes are hidden – Developer Mode in Settings shows them.");
    assert_eq!(page["labHint"]["action"]["params"], json!({"page": "settings", "params": {}}));
    c.available.as_mut().unwrap().offered.clear();
    assert!(get(&c, "recipes", json!({})).get("offeredTitle").is_none());
}

#[test]
fn guides_and_logs_say_where_they_are() {
    let mut c = ctx();
    let builder = fx::prerequisite(
        crate::prerequisites::PrerequisiteItem::LinuxBuilder,
        crate::prerequisites::PrerequisiteStatus::Ok,
    );
    c.doctor.as_mut().unwrap().prerequisites.push(builder);
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("linux-builder.md"), "# Linux builder").unwrap();
    let guide_of = |c: &Ctx| {
        let page = get(c, "doctor", json!({}));
        let item = page["sections"][0]["items"]
            .as_array()
            .unwrap()
            .iter()
            .find(|i| i["item"] == "linuxBuilder")
            .unwrap()
            .clone();
        action(&item["actions"], "guide").clone()
    };
    let guide = guide_of(&c);
    assert_eq!(guide["method"], "ui.show");
    assert_eq!(guide["params"]["page"], "guide");
    assert_eq!(guide["params"]["params"]["url"], "https://omaflake.dev/linux-builder/");
    assert_eq!(guide["params"]["params"]["title"]["key"], "Linux Builder Guide");
    assert!(guide["params"]["params"].get("path").is_none(), "no app bundle here");
    c.guides = Some(dir.path().to_path_buf());
    let path = dir.path().join("linux-builder.md").to_string_lossy().into_owned();
    assert_eq!(guide_of(&c)["params"]["params"]["path"], path);

    // The build page names its job and where its log starts
    c.jobs[0].log_lines = 5012;
    let page = get(&c, "build", json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"}));
    assert_eq!((page["log"]["job"].clone(), page["log"]["from"].clone()), (json!("j1"), json!(12)));
}

#[test]
fn linux_hosts_get_their_own_words() {
    let mut c = ctx();
    c.host = HostOs::Linux;
    let home = get(&c, "home", json!({}));
    assert_eq!(action(&home["actions"], "dataFolder")["label"]["key"], "Show Data Folder");
    let new_vm = get(&c, "newVm", json!({"image": "omaflake.nixos.dev@cedar-mango-lift"}));
    assert_eq!(action(&new_vm["sections"][2]["fields"], "share")["label"]["key"], "Share a folder with the VM");
    let doctor = get(&c, "doctor", json!({}));
    assert_eq!(
        doctor["intro"]["key"],
        "omaflake uses what this computer already has. Set up missing parts here or enter their path."
    );
    let settings = get(&c, "settings", json!({}));
    let locale = action(&settings["guest"]["fields"], "locale");
    assert_eq!(locale["choices"][0]["label"]["key"], "Like this computer – %@");
    assert_eq!(settings["sources"]["rows"][1]["badges"][0]["help"]["key"], "Private – token in the keyring");
    let images = get(&c, "images", json!({}));
    assert_eq!(images["rows"][0]["actions"][0]["label"]["key"], "Show in File Manager");
    let recipes = get(&c, "recipes", json!({"filter": {"otherSystems": true}}));
    assert_eq!(recipes["rows"][1]["actions"][1]["reason"]["key"], "Not for this computer – it is built for %@");
    // Every text that names the Mac, Finder or the Keychain has its other variant
    for page in [home, new_vm, doctor, settings, images, recipes] {
        let text = page.to_string();
        for word in ["Finder", "Keychain", "this Mac", "your Mac", "a Mac folder", "Like this Mac"] {
            assert!(!text.contains(word), "{word} in {text}");
        }
    }
}

#[test]
fn inspection_reasons_are_texts() {
    let bad = Inspection {
        kind: PackageKindDto::Image,
        valid: false,
        reason: Some(Text::verbatim("Not an image package (.omaflake-image)")),
        conflict: None,
        network_volume: false,
    };
    let v = serde_json::to_value(&bad).unwrap();
    assert_eq!(v["reason"], json!({"text": "Not an image package (.omaflake-image)"}));
    let p = import::ImportParams { path: "/x/a.omaflake-image".into(), expect: None, external: false };
    let page = serde_json::to_value(import::page(&p, bad)).unwrap();
    assert_eq!(page["reason"]["args"][1], json!({"text": "Not an image package (.omaflake-image)"}));
    assert_eq!(page["title"]["key"], "Import");
}

#[test]
fn every_page_reads_back_as_its_type() {
    // What the server sends, a Rust UI decodes with the same types
    let c = ctx();
    fn back<T: serde::de::DeserializeOwned + Serialize>(v: &Value) {
        let t: T = serde_json::from_value(v.clone()).unwrap();
        assert_eq!(&serde_json::to_value(t).unwrap(), v);
    }
    back::<home::HomePage>(&get(&c, "home", json!({})));
    back::<vms::VmsPage>(&get(&c, "vms", json!({})));
    back::<images::ImagesPage>(&get(&c, "images", json!({})));
    back::<recipes::RecipesPage>(&get(&c, "recipes", json!({})));
    back::<new_vm::NewVmPage>(&get(&c, "newVm", json!({"image": "omaflake.nixos.dev@cedar-mango-lift"})));
    back::<build::BuildPage>(&get(&c, "build", json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"})));
    back::<doctor::DoctorPage>(&get(&c, "doctor", json!({})));
    back::<doctor::MacBasePage>(&get(&c, "macBase", json!({})));
    back::<settings::SettingsPage>(&get(&c, "settings", json!({})));
    back::<activity::ActivityPage>(&get(&c, "activity", json!({})));
    back::<vms::VmPage>(&get(&c, "vm", json!({"name": "research"})));
    back::<images::ImagePage>(&get(&c, "image", json!({"name": "omaflake.macos.dev@stone-lamp"})));
    let e = build(&c, &PageRef { page: "nope".into(), params: json!({}) }).unwrap_err();
    assert_eq!(e.code, crate::api::protocol::INVALID_PARAMS);
}

// MARK: Through the server

mod server {
    use super::*;
    use crate::api::events::tests::recorder;
    use crate::api::server::lock;
    use crate::api::Server;
    use crate::paths::tests::temp_paths;

    #[test]
    fn views_get_subscribe_and_activity() {
        let (_dir, paths) = temp_paths();
        let (sink, got) = recorder();
        let server = Server::for_tests(paths, sink);
        server.call("initialize", json!({"apiVersion": 1})).unwrap();
        let vms = server.call("views.get", json!({"page": "vms"})).unwrap();
        assert_eq!(vms["empty"]["text"]["key"], "No VMs yet – create one from an image.");
        let home = server.call("views.get", json!({"page": "home", "params": {}})).unwrap();
        assert_eq!(home["firstTab"], "recipes");
        let e = server.call("views.get", json!({"page": "nope"})).unwrap_err();
        assert_eq!(e.code, crate::api::protocol::INVALID_PARAMS);
        let e = server.call("views.get", json!({"page": "vm", "params": {"name": "x"}})).unwrap_err();
        assert_eq!(e.kind, Some(crate::error::codes::NOT_FOUND));

        // Once the catalogs were looked into, the pages use them (this deadlocked: the catalogs' lock taken twice)
        lock(&server.inner.catalogs).fetched = true;
        let recipes = server.call("views.get", json!({"page": "recipes"})).unwrap();
        assert_eq!(recipes["catalogsLoaded"], true);

        // A refused action lands in the activity list, worded for every UI
        assert!(server.call("vms.remove", json!({"name": "ghost"})).is_err());
        let activity = server.call("views.get", json!({"page": "activity"})).unwrap();
        assert!(activity["entries"][0]["text"]["text"].as_str().unwrap().starts_with("ghost: "), "{activity}");
        assert_eq!(activity["entries"][0]["level"], "error");

        // Subscribed pages come at once and again when they change
        server
            .call(
                "views.subscribe",
                json!({"pages": [{"page": "activity"}, {"page": "vms", "params": {"filter": {"status": true}}}]}),
            )
            .unwrap();
        let wait = |n: usize| {
            for _ in 0..50 {
                if got.lock().unwrap().iter().filter(|(m, _)| m == "view.changed").count() >= n {
                    return;
                }
                std::thread::sleep(std::time::Duration::from_millis(100));
            }
            panic!("no view.changed: {:?}", got.lock().unwrap());
        };
        wait(2);
        server.call("activity.clear", json!({})).unwrap();
        wait(3);
        let sent = got.lock().unwrap().clone();
        let last = sent.iter().rev().find(|(m, _)| m == "view.changed").unwrap();
        assert_eq!(last.1["page"], "activity");
        assert_eq!(last.1["model"]["entries"], json!([]));
        let vms = sent.iter().find(|(m, p)| m == "view.changed" && p["page"] == "vms").unwrap();
        assert_eq!(vms.1["params"], json!({"filter": {"status": true}}));

        // Subscribing again: a page that stays (same params) is not sent again, a new one comes at once
        let count =
            |page: &str| got.lock().unwrap().iter().filter(|(m, p)| m == "view.changed" && p["page"] == page).count();
        let activity_before = count("activity");
        server.call("views.subscribe", json!({"pages": [{"page": "activity"}, {"page": "home"}]})).unwrap();
        for _ in 0..50 {
            if count("home") > 0 {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        assert_eq!(count("home"), 1);
        assert_eq!(count("activity"), activity_before);
        server.call("views.subscribe", json!({"pages": []})).unwrap();
        server.close(std::time::Duration::from_secs(5));
    }
}
