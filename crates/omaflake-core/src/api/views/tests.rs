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
/// Rosé Pine Dawn installed (the default, an image built with it), omaflake's look in a local folder, Catppuccin offered
pub(crate) fn looks_fixture() -> crate::api::types::LooksAnswer {
    use crate::api::types::{LookDto, LooksAnswer, OfferedLookDto};
    use crate::looks::Swatches;
    let swatches = |bg: &str, accent: &str| {
        Some(Swatches {
            background: bg.into(),
            surface: bg.into(),
            text: "#575279".into(),
            accent: accent.into(),
            urgent: "#b4637a".into(),
            terminal: vec!["#f2e9e1".into(); 16],
        })
    };
    let look =
        |name: &str, title: &str, appearance: &str, installed: bool, default: bool, images: Vec<String>| LookDto {
            id: format!("omaflake.look.{name}"),
            reference: format!("omaflake/{name}"),
            author: "omaflake".into(),
            name: name.into(),
            title: title.into(),
            description: format!("{title}."),
            tags: vec![appearance.into()],
            appearance: Some(appearance.into()),
            plain: false,
            swatches: swatches("#faf4ed", "#907aa9"),
            source: if installed { "github:omaflake/catalog@main".into() } else { "folder:/src/catalog".into() },
            source_name: if installed { "omaflake/catalog".into() } else { "/src/catalog".into() },
            official: installed,
            installed,
            update: false,
            is_default: default,
            images,
        };
    LooksAnswer {
        looks: vec![
            look(
                "rose-pine-dawn",
                "Rosé Pine Dawn",
                "light",
                true,
                true,
                vec!["omaflake.nixos.dev@cedar-mango-lift".into()],
            ),
            look("omaflake", "omaflake", "dark", false, false, vec![]),
        ],
        offered: vec![OfferedLookDto {
            id: "omaflake.look.catppuccin".into(),
            reference: "omaflake/catppuccin".into(),
            author: "omaflake".into(),
            name: "catppuccin".into(),
            title: "Catppuccin Mocha".into(),
            description: "Pastel.".into(),
            tags: vec!["dark".into()],
            appearance: Some("dark".into()),
            plain: false,
            swatches: swatches("#1e1e2e", "#cba6f7"),
            source: "github:omaflake/catalog@main".into(),
            source_name: "omaflake/catalog".into(),
            official: true,
        }],
        unreadable: vec![],
        fetched: true,
    }
}

/// A module here (used by the dev recipe), key bindings, and a module a catalog offers
pub(crate) fn modules_fixture() -> crate::api::types::ModulesAnswer {
    use crate::api::types::PartDto;
    let part = |id: &str, kind: &str, title: &str| PartDto {
        id: id.into(),
        reference: crate::parts::PartKind::Module.reference(id),
        kind: kind.into(),
        platform: (kind == "module").then_some(GuestKind::Nixos),
        title: title.into(),
        description: format!("{title}."),
        version: "1.0.0".into(),
        tags: vec![],
        provides: vec![],
        needs: vec![],
        enhances: vec![],
        keys: BTreeMap::new(),
        unfree: false,
        source: "github:omaflake/recipes@main".into(),
        source_name: "omaflake/recipes".into(),
        official: true,
        installed: true,
        update: false,
        recipes: vec![],
    };
    crate::api::types::ModulesAnswer {
        parts: vec![
            PartDto {
                provides: vec!["desktop".into()],
                needs: vec!["terminal".into()],
                keys: BTreeMap::from([("terminal".to_string(), "Super+Return".to_string())]),
                recipes: vec!["dev".into()],
                update: true,
                ..part("omaflake.nixos.hyprland", "module", "Hyprland")
            },
            PartDto {
                reference: "omaflake/default".into(),
                keys: BTreeMap::from([("omaflake/hyprland terminal".to_string(), "Super+Return".to_string())]),
                ..part("omaflake.keymap.default", "keymap", "omaflake")
            },
        ],
        offered: vec![
            PartDto {
                installed: false,
                unfree: true,
                platform: Some(GuestKind::Macos),
                ..part("omaflake.macos.vscode", "module", "VS Code")
            },
            PartDto {
                installed: false,
                provides: vec!["terminal".into()],
                tags: vec!["terminal".into()],
                ..part("omaflake.nixos.wezterm", "module", "WezTerm")
            },
        ],
        unreadable: vec![],
        fetched: true,
    }
}

pub(crate) fn ctx() -> Ctx {
    let answer = fx::settings_answer();
    Ctx {
        language: None,
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
        credentials: vec![crate::api::types::CredentialDto {
            id: "c-1".into(),
            name: "Work".into(),
            user: Some("alex".into()),
            full_name: Some("Alex Doe".into()),
            email: None,
            has_password: true,
            is_default: true,
        }],
        looks: looks_fixture(),
        modules: modules_fixture(),
        profiles: vec![crate::api::types::ProfileDto {
            id: "p-1".into(),
            name: "Small".into(),
            cpus: 2,
            memory_gb: 4,
            is_default: true,
        }],
        locales: vec![crate::api::types::LocaleDto {
            id: "l-1".into(),
            name: "German".into(),
            locale: "de_DE.UTF-8".into(),
            time_zone: "Europe/Berlin".into(),
            keyboards: vec!["de".into(), "ru".into()],
            is_default: true,
            images: vec!["omaflake.nixos.dev@cedar-mango-lift".into()],
        }],
        bases: vec![crate::api::types::BaseDto {
            name: "mac-base-26A434".into(),
            path: "/data/bases/mac-base-26A434".into(),
            os_build: Some("26A434".into()),
            user: Some("alex".into()),
            ready: true,
            newest: true,
            created: Some(fx::date("2026-10-17T10:00:00Z")),
            size_bytes: Some(41_000_000_000),
            credential: Some("c-1".into()),
            credential_name: Some("Work".into()),
        }],
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
        client_version: Some("0.1.2".into()),
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
    // The image's credential and look next to the recipe (plan B137)
    assert_eq!(research["subtitle"]["text"], "dev (velvet) · Work · omaflake · 4 CPU · 8 GB · SSH 2201");
    let badges: Vec<&str> = research["badges"].as_array().unwrap().iter().map(|b| b["id"].as_str().unwrap()).collect();
    assert_eq!(badges, ["external", "clipboard", "sound", "upgradable"]);
    // Running: its keys, share folder, terminal, stop
    assert_eq!(ids(&research["actions"]), ["keys", "showShare", "terminal", "stop"]);
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
            "Credential",
            "Look",
            "Locale",
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
    // The credential and the look come with the image (plan B135, B137)
    assert_eq!(d["info"][3]["value"]["text"], "Work (alex)");
    assert_eq!(d["info"][4]["value"]["text"], "omaflake");
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
    let info = work["detail"]["info"].as_array().unwrap();
    let line = |label: &str| info.iter().find(|l| l["label"]["key"] == label).unwrap().clone();
    assert_eq!(line("SSH")["value"]["key"], "address while running");
    // Its image is from before looks
    assert_eq!(line("Look")["value"]["key"], "unknown – built before looks");
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
    assert_eq!(ids(&row["actions"]), ["keys", "showShare", "doctor"]);
    assert_eq!(row["actions"][2]["help"]["text"], "Open the Doctor first: QEMU (NixOS guests)");
    assert_eq!(row["detail"]["actions"][0]["enabled"], true);

    // Without blockers it starts; two macOS VMs run: a third waits
    c.doctor.as_mut().unwrap().start_blockers.nixos.clear();
    let row = &get(&c, "vms", json!({}))["rows"][0];
    assert_eq!(ids(&row["actions"]), ["keys", "showShare", "startHeadless", "startWindow"]);
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

/// A running VM on a Linux host names its accelerator; TCG with a warning (plan B124); a Mac's HVF is not shown
#[test]
fn vm_details_name_the_accelerator() {
    let mut c = ctx();
    let line = |c: &Ctx| -> Option<Value> {
        let page = get(c, "vm", json!({"name": "research"}));
        page["row"]["detail"]["info"].as_array().unwrap().iter().find(|l| l["label"]["key"] == "Accelerator").cloned()
    };
    assert_eq!(line(&c), None);
    let vm = c.vms.iter_mut().find(|v| v.name == "research").unwrap();
    vm.accelerator = Some("tcg".into());
    let tcg = line(&c).unwrap();
    assert_eq!(tcg["value"]["text"], "TCG");
    assert_eq!(tcg["tone"], "warning");
    c.vms.iter_mut().find(|v| v.name == "research").unwrap().accelerator = Some("kvm".into());
    assert_eq!(line(&c).unwrap()["value"]["text"], "KVM");
    c.vms.iter_mut().find(|v| v.name == "research").unwrap().accelerator = Some("hvf".into());
    assert_eq!(line(&c), None);
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
    assert_eq!(ids(&used["actions"]), ["keys", "reveal", "doctor"], "QEMU missing: no new VM from a NixOS image");
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
    // The Linux builder does not answer: the build button stays, greyed out with the Doctor's sign
    assert_eq!(ids(&dev["actions"]), ["keys", "reveal", "build"]);
    assert_eq!(action(&dev["actions"], "keys")["params"]["page"], "keys");
    let build = &dev["actions"][2];
    assert_eq!(
        (build["enabled"].clone(), build["badge"].clone(), build["primary"].clone()),
        (json!(false), json!(1), json!(true))
    );
    assert_eq!(build["reason"]["key"], "Open the Doctor first: %@");
    let d = &dev["detail"];
    assert_eq!(d["info"][3]["value"]["text"], "Clipboard · Sound · Share folder");
    assert_eq!(d["info"][4]["value"]["key"], "%lld built, the newest %@");
    assert_eq!(ids(&d["actions"]), ["update", "copy", "removeAll"]);
    // What it is made of (plan B142): its modules, look, key bindings and what is wrong – its keys on a page of their own
    let info: Vec<&str> = d["info"].as_array().unwrap().iter().filter_map(|l| l["label"]["key"].as_str()).collect();
    assert!(info.contains(&"Modules") && info.contains(&"Look") && info.contains(&"Key bindings"), "{info:?}");
    assert_eq!(d["notes"][0]["action"]["method"], "recipes.installParts");
    assert_eq!(d["notes"][1]["text"]["key"], "%@ (%@): without a mode a key starts with Super or Ctrl");
    assert_eq!(d["notes"][1]["keys"], json!([[["t"]]]), "the key as caps after the text (plan B148)");
    assert!(d.get("keys").is_none(), "the keys page has them (plan B147)");
    assert_eq!(dev["lamp"]["state"], "working");
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

    // Offered: not installed, from the official source – the catalog's tab (plan B147)
    assert!(page["offered"].as_array().unwrap().is_empty(), "Build Images has the recipes here only");
    let catalog = get(&c, "recipeCatalog", json!({"filter": {"otherSystems": true}}));
    assert_eq!(catalog["title"]["key"], "Recipes in your sources");
    assert!(catalog["rows"].as_array().unwrap().is_empty());
    let page = catalog;
    let offered: Vec<&str> = page["offered"].as_array().unwrap().iter().map(|o| o["id"].as_str().unwrap()).collect();
    assert_eq!(offered, ["omaflake.nixos.minimal", "lab.nixos.try"]);
    assert_eq!(page["offered"][0]["actions"][0]["label"]["text"], "Install from omaflake/recipes");
    assert!(page["offered"][0]["detail"].get("warning").is_none());
    assert_eq!(
        page["offered"][1]["detail"]["warning"]["key"],
        "Not reviewed by the omaflake team – its Nix code runs on this Mac when you build it."
    );
    assert_eq!(page["errors"][0]["text"]["text"], "example.org: HTTP 404");
    assert_eq!(page["catalogsLoaded"], true);
    assert!(page.get("updates").is_none(), "newer versions of the recipes here: Build Images");
    let build = get(&c, "recipes", json!({}));
    assert_eq!(build["updates"]["rows"][0]["action"]["label"]["key"], "Update");
    assert!(build["updates"].get("all").is_none());
    assert!(build.get("suggestion").is_none(), "there are recipes already");
    assert!(build["errors"].as_array().unwrap().is_empty(), "the sources' errors are the catalog's");
    let tags = get(&c, "recipes", json!({"filter": {"tag": "gui"}}));
    assert_eq!((tags["count"].clone(), tags["offered"].clone()), (json!(1), json!([])));
}

#[test]
fn recipes_at_the_first_start() {
    let mut c = ctx();
    c.recipes.recipes.clear();
    let page = get(&c, "recipes", json!({}));
    assert_eq!(page["suggestion"]["action"]["method"], "recipes.installSuggested");
    assert_eq!(
        page["empty"]["text"]["key"],
        "No recipes here yet – install some from the catalog (Catalog → Recipes), or put one together in the Recipe Studio."
    );
    c.available = None;
    c.fetching = true;
    let page = get(&c, "recipeCatalog", json!({}));
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
    // The default profile (plan B142): its CPU cores and RAM, fixed; less than the recipe needs is said
    assert_eq!(
        (create["params"]["settings"]["cpus"].clone(), create["params"]["settings"]["memoryGB"].clone()),
        (json!(2), json!(4))
    );
    assert_eq!(
        page["warnings"][0]["key"].as_str().unwrap().split(' ').take(3).collect::<Vec<_>>(),
        ["Its", "recipe", "needs"]
    );
    let custom =
        get(&c, "newVm", json!({"image": "omaflake.nixos.dev@cedar-mango-lift", "values": {"profile": "", "cpus": 6}}));
    assert_eq!(action(&custom["actions"], "create")["params"]["settings"]["cpus"], 6, "custom values: the form's");
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
            "profile",
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
    assert_eq!(page["sections"][1]["fields"][0]["id"], "profile");
    assert_eq!(page["sections"][1]["fields"][1]["label"]["text"], "CPU cores: 2");
    assert_eq!(page["sections"][1]["fields"][1]["enabled"], false, "the profile's");
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
    // A macOS recipe: no password is ever asked – the base's credential has it (plan B139)
    c.recipes.recipes[0].runs_here = true;
    c.jobs.retain(|j| j.id != "j2");
    c.doctor.as_mut().unwrap().build_blockers.macos.clear();
    // The form: the base's credential (fixed) and the default look
    let page = get(&c, "build", json!({"recipe": "lab.macos.base@ocean-maple", "values": {"credential": "c-9"}}));
    let build = action(&page["actions"], "build");
    assert!(build.get("ask").is_none());
    assert_eq!(
        build["params"],
        json!({"recipe": "lab.macos.base@ocean-maple", "credential": "c-1", "look": "omaflake.look.rose-pine-dawn", "locale": "l-1"})
    );
    let fields: Vec<&str> = page["fields"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(fields, ["credential", "look", "locale"]);
    assert_eq!(page["fields"][0]["valueParam"], "credential");
    assert_eq!(page["fields"][0]["choices"][0]["label"]["text"], "Work – alex");
    assert_eq!(page["fields"][0]["enabled"], false, "the base's credential");
    // The installed looks (plan B141), the default preset; no "recipe's own" any more
    let looks: Vec<&str> =
        page["fields"][1]["choices"].as_array().unwrap().iter().map(|c| c["value"].as_str().unwrap()).collect();
    assert_eq!(looks, ["omaflake.look.rose-pine-dawn", "omaflake.look.omaflake"]);
    assert_eq!(build["enabled"], true);
    assert_eq!(page["state"], "idle");
    // A base from before credentials: assign one first – still no password asked
    c.bases[0].credential = None;
    let page = get(&c, "build", json!({"recipe": "lab.macos.base@ocean-maple"}));
    let build = action(&page["actions"], "build");
    assert!(build.get("ask").is_none());
    assert_eq!(build["enabled"], false);
    assert_eq!(
        page["macBase"]["action"]["params"],
        json!({"page": "baseCredential", "params": {"name": "mac-base-26A434"}})
    );
    // No credential at all: nothing is built (the Doctor says so too)
    c.credentials.clear();
    c.bases[0].credential = Some("c-1".into());
    let page = get(&c, "build", json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"}));
    assert_eq!(action(&page["actions"], "build")["enabled"], false);
}

// MARK: Keys

/// The keys page (plan B147): by module, the desktop's first; a mode with the way there and where each key leads
#[test]
fn keys_page_by_module_and_mode() {
    let mut c = ctx();
    c.language = Some("de".into());
    c.host = super::HostOs::Macos;
    let page = get(&c, "keys", json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"}));
    assert_eq!(page["title"]["key"], "Keys of %@");
    let modules: Vec<&str> =
        page["modules"].as_array().unwrap().iter().map(|m| m["reference"].as_str().unwrap()).collect();
    assert_eq!(modules, ["omaflake/hyprland", "omaflake/zellij"], "the desktop's first; WezTerm has no keys");
    let hyprland = &page["modules"][0]["groups"][0];
    assert_eq!(hyprland["title"]["key"], "Anywhere on the desktop");
    let terminal = hyprland["rows"].as_array().unwrap().iter().find(|r| r["action"] == "terminal").unwrap();
    assert_eq!(terminal["caps"][0], json!([["Cmd", "Enter"]]), "Super is Cmd on a Mac");
    let groups = page["modules"][1]["groups"].as_array().unwrap();
    let titles: Vec<&str> = groups.iter().map(|g| g["title"]["text"].as_str().unwrap()).collect();
    assert_eq!(titles, ["Gesperrt", "Unlocked", "Pane"], "the start mode first, then as they are reached");
    assert_eq!(groups[0]["start"], true);
    // Pane: Ctrl+g › p
    let way: Vec<&Value> = groups[2]["way"].as_array().unwrap().iter().map(|s| &s["caps"]).collect();
    assert_eq!(way, [&json!([["Ctrl", "g"]]), &json!([["p"]])]);
    assert_eq!(groups[2]["way"][1]["mode"]["text"], "Pane");
    let rows = groups[2]["rows"].as_array().unwrap();
    let new_pane = rows.iter().find(|r| r["action"] == "new-pane").unwrap();
    assert_eq!(new_pane["title"]["text"], "Neuer Bereich");
    assert_eq!(
        (new_pane["after"]["key"].clone(), new_pane["after"]["args"][0]["text"].clone()),
        (json!("then %@"), json!("Gesperrt"))
    );
    assert_eq!((new_pane["backToStart"].clone(), new_pane["switches"].clone()), (json!(true), json!(false)));
    assert_eq!((new_pane["does"].clone(), new_pane["from"].clone()), (json!("NewPane"), json!("omaflake/default")));
    let focus = rows.iter().find(|r| r["action"] == "focus-left").unwrap();
    assert_eq!(focus["caps"], json!([[["h"]], [["←"]]]));
    assert!(focus.get("after").is_none(), "it stays in the mode");
    // Quit works in two modes: listed in both
    assert!(groups[1]["rows"].as_array().unwrap().iter().any(|r| r["action"] == "quit"));
    assert!(rows.iter().any(|r| r["action"] == "quit"));
    // An image or a VM: through its recipe, with a note
    let vm = get(&c, "keys", json!({"vm": "research"}));
    assert_eq!(vm["subtitle"]["key"], "VM %@");
    assert_eq!(
        vm["notes"][0]["text"]["key"],
        "As its recipe has them now – a module or key binding set updated since the build may differ."
    );
    let gone = get(&c, "keys", json!({"recipe": "nope"}));
    assert_eq!(gone["notes"][0]["tone"], "warning");
    assert!(gone["modules"].as_array().unwrap().is_empty());
}

/// The key binding studio (plan B147): a module of a local folder source with modes, a set to start from; values
/// change a key and the mode after one; what overlaps is said; only differences are saved
#[test]
fn keymap_studio_changes_keys_and_modes() {
    use crate::recipe_sources::{RecipeSource, RecipeSourceKind};
    let (_t, paths) = crate::paths::tests::temp_paths();
    let src = tempfile::tempdir().unwrap();
    let module = src.path().join("modules/alice/nixos/mux");
    std::fs::create_dir_all(&module).unwrap();
    std::fs::write(module.join("default.nix"), "{ ... }: { }\n").unwrap();
    std::fs::write(
        module.join("module.toml"),
        r#""$schema" = "https://omaflake.dev/schema/module/v2.json"
version = "1.0.0"
title = "Mux"
[modes.locked]
title = "Locked"
title_de = "Gesperrt"
start = true
[modes.pane]
title = "Pane"
[actions.unlock]
title = "Unlock"
key = "Ctrl+g"
mode = "locked"
after = "pane"
[actions.new]
title = "New pane"
title_de = "Neuer Bereich"
key = "n"
mode = "pane"
does = "NewPane"
after = "locked"
[actions.left]
title = "Focus left"
key = ["h", "Left"]
mode = "pane"
[actions.copy]
title = "Copy"
key = "Ctrl+Shift+c"
own = true
"#,
    )
    .unwrap();
    let set = src.path().join("keymaps/alice/base");
    std::fs::create_dir_all(&set).unwrap();
    std::fs::write(
        set.join("keymap.toml"),
        "\"$schema\" = \"https://omaflake.dev/schema/keymap/v2.json\"\nversion = \"1.0.0\"\ntitle = \"Base\"\n[keys.\"alice/mux\"]\nunlock = \"Ctrl+a\"\n",
    )
    .unwrap();
    let mut s = crate::paths::Settings::load(&paths);
    s.sources =
        Some(vec![RecipeSource::new(RecipeSourceKind::Folder, &src.path().to_string_lossy(), None, None, None)]);
    s.save(&paths).unwrap();
    let mut c = ctx();
    c.paths = paths;
    c.language = Some("de".into());
    c.host = super::HostOs::Macos;
    // A copy of alice/base: its key, the module's otherwise; the set covers the modules the base names (plan B148)
    let page = get(&c, "keymapStudio", json!({"id": "alice.keymap.base", "values": {"name": "mine"}}));
    assert_eq!(page["title"]["key"], "Copy Key Bindings");
    let chips: Vec<(&str, bool)> = page["modules"]
        .as_array()
        .unwrap()
        .iter()
        .map(|c| (c["reference"].as_str().unwrap(), c["on"].as_bool().unwrap()))
        .collect();
    assert!(chips.contains(&("alice/mux", true)), "{chips:?}");
    let card = &page["cards"][0];
    assert_eq!((card["reference"].as_str(), card["open"].as_bool()), (Some("alice/mux"), Some(true)));
    let groups = card["groups"].as_array().unwrap();
    let titles: Vec<&str> = groups.iter().map(|g| g["title"]["text"].as_str().unwrap()).collect();
    assert_eq!(titles, ["In every mode", "Gesperrt", "Pane"]);
    assert_eq!(
        (groups[1]["open"].as_bool(), groups[2]["open"].as_bool()),
        (Some(true), Some(false)),
        "the start mode open"
    );
    assert_eq!(groups[2]["way"], json!([[["Ctrl", "a"]]]), "the way to Pane with the set's key");
    let row = |page: &Value, action: &str| -> Value {
        page["cards"][0]["groups"]
            .as_array()
            .unwrap()
            .iter()
            .flat_map(|g| g["rows"].as_array().unwrap().clone())
            .find(|r| r["action"] == action)
            .unwrap()
    };
    let unlock = row(&page, "unlock");
    assert_eq!((unlock["caps"].clone(), unlock["changed"].clone()), (json!([[["Ctrl", "a"]]]), json!(true)));
    let copy = row(&page, "copy");
    assert_eq!(copy["own"], true);
    assert!(copy.get("edit").is_none(), "the program's own key is not changed");
    let new = row(&page, "new");
    assert_eq!(new["title"]["text"], "Neuer Bereich");
    assert_eq!(new["after"]["args"][0]["text"], "Gesperrt");
    assert_eq!(new["edit"]["params"]["value"], "alice/mux:new");
    // The key editor: the keys as caps, the modifiers and keys to put one together, where it leads
    let editing = json!({"id": "alice.keymap.base", "values": {"name": "mine", "edit": "alice/mux:new"}});
    let ed = get(&c, "keymapStudio", editing)["editor"].clone();
    assert_eq!(ed["keys"], json!(["n"]));
    assert_eq!(ed["modifiers"][3], json!({"value": "Super", "label": "Cmd"}), "the Mac's name for Super");
    assert_eq!(ed["keyGroups"][0]["keys"][0]["value"], "a");
    assert_eq!(ed["after"]["value"], "locked");
    assert_eq!(ed["after"]["choices"][0]["label"]["key"], "stays in %@", "staying first");
    assert_eq!(ed["after"]["choices"][1]["label"]["key"], "back to %@", "then the start mode");
    assert_eq!(ed["apply"]["params"]["value"], json!(["n"]));
    // A draft: Alt+n staying in pane mode – Apply takes it, closing the editor
    let values = json!({"name": "mine", "edit": "alice/mux:new", "draft": ["Alt+n"], "draftAfter": "pane"});
    let page = get(&c, "keymapStudio", json!({"id": "alice.keymap.base", "values": values}));
    let ed = &page["editor"];
    assert_eq!(ed["caps"], json!([[["Opt", "n"]]]));
    let apply = &ed["apply"]["params"];
    assert_eq!((apply["valueParam"].as_str(), &apply["value"]), (Some("key:alice/mux:new"), &json!(["Alt+n"])));
    let also: Vec<(&str, &Value)> =
        apply["also"].as_array().unwrap().iter().map(|a| (a["valueParam"].as_str().unwrap(), &a["value"])).collect();
    assert_eq!(also[0], ("after:alice/mux:new", &json!("pane")));
    assert!(also.contains(&("edit", &Value::Null)) && also.contains(&("draft", &Value::Null)));
    assert_eq!(ed["default"]["params"]["value"], json!(["n"]), "back to the module's own");
    // Values: new stays in pane mode, left gets n too – they overlap
    let values = json!({"name": "mine", "after:alice/mux:new": "pane", "key:alice/mux:left": ["n", "Left"]});
    let page = get(&c, "keymapStudio", json!({"id": "alice.keymap.base", "values": values}));
    let left = row(&page, "left");
    assert_eq!(left["conflict"]["key"], "Overlaps %@");
    assert_eq!(page["cards"][0]["conflicts"], 2);
    assert_eq!(page["hints"][0]["text"]["key"], "Keys overlap: %@ and %@");
    assert_eq!(left["reset"]["params"]["also"][0]["valueParam"], "after:alice/mux:left");
    let save = action(&page["actions"], "save");
    assert_eq!(
        save["params"]["keys"],
        json!({"alice/mux": {"unlock": "Ctrl+a", "new": {"after": "pane"}, "left": ["n", "Left"]}})
    );
    assert_eq!(save["params"]["name"], "mine");
    assert!(save["params"].get("previous").is_none(), "a copy");
    // A module taken out of the set: its changes are not saved
    let values = json!({"name": "mine", "modules": [], "key:alice/mux:left": ["n"]});
    let page = get(&c, "keymapStudio", json!({"id": "alice.keymap.base", "values": values}));
    assert_eq!(action(&page["actions"], "save")["params"]["keys"], json!({}));
    assert!(page["cards"].as_array().unwrap().is_empty());
    // Typed keys (a terminal's editor): one that is not a key is said there, Apply waits
    let values = json!({"name": "mine", "edit": "alice/mux:left", "draftText": "Hyper+x"});
    let ed = get(&c, "keymapStudio", json!({"values": values}))["editor"].clone();
    assert!(ed["text"]["error"].is_object());
    assert_eq!(ed["apply"]["enabled"], false);
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

/// Nix installed, its volume not at /nix (a restart without /etc/fstab): Mount …, never Install …
#[test]
fn doctor_offers_to_mount_the_nix_store() {
    use crate::prerequisites::{PrerequisiteItem as I, PrerequisiteStatus as S, Prerequisites};
    let mut c = ctx();
    let mut nix = fx::prerequisite(I::Nix, S::Missing);
    nix.detail = format!("{} (mounted at /Volumes/Nix Store)", Prerequisites::NIX_NOT_MOUNTED);
    nix.install = Some("sudo sh -c '…'".into());
    c.doctor.as_mut().unwrap().prerequisites[0] = nix;
    let page = get(&c, "doctor", json!({}));
    let item = &page["sections"][0]["items"][0];
    assert_eq!(item["item"], "nix");
    let ids = ids(&item["actions"]);
    assert!(ids.contains(&"mountNix".to_string()) && !ids.contains(&"installNix".to_string()), "{ids:?}");
    let mount = action(&item["actions"], "mountNix");
    assert_eq!(mount["method"], "builder.mountNix");
    assert_eq!(mount["confirm"]["title"]["key"], "Mount the Nix Store?");
    assert!(item["hint"]["key"]
        .as_str()
        .unwrap()
        .starts_with("Nix is installed, but its volume is not mounted at /nix"));
}

/// Apple offers a newer macOS than the IPSW here; a download that failed at once says why at the row (plan B146)
#[test]
fn doctor_newer_ipsw_and_a_failed_download() {
    use crate::prerequisites::{PrerequisiteItem as I, PrerequisiteStatus as S, Prerequisites};
    let mut c = ctx();
    let mut ipsw = fx::prerequisite(I::Ipsw, S::Limited);
    ipsw.hint = Prerequisites::IPSW_NEWER.into();
    c.doctor.as_mut().unwrap().macos_preflight[0] = ipsw;
    c.jobs.retain(|j| j.id != "j4");
    let page = get(&c, "doctor", json!({}));
    let row = &page["sections"][2]["items"][0];
    assert!(row["hint"]["key"].as_str().unwrap().starts_with("A newer macOS is available from Apple."));
    assert_eq!(action(&row["actions"], "download")["label"]["key"], "Download");
    let j5 = c.jobs.iter_mut().find(|j| j.id == "j5").unwrap();
    j5.state = JobState::Failed;
    j5.error = Some("Not enough space for the download: 20.0 GB free, about 28.6 GB needed".into());
    let page = get(&c, "doctor", json!({}));
    let row = &page["sections"][2]["items"][0];
    assert_eq!(row["hint"]["key"], "Download failed: %@");
    assert_eq!(row["expanded"], true);
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

/// A Linux host's Doctor (plan B124): no macOS section, its own host and QEMU texts, KVM with the fix, Nix installed
/// from a terminal (no administrator dialog)
#[test]
fn doctor_on_a_linux_host() {
    use crate::prerequisites::{PrerequisiteItem as I, PrerequisiteStatus as S, Prerequisites};
    let mut c = ctx();
    c.host = HostOs::Linux;
    let dto = |p: &crate::prerequisites::Prerequisite| {
        let mut d = crate::api::model::prerequisite_dto(p);
        d.title = crate::api::model::requirement_title_on(p.item, HostOs::Linux).into();
        d
    };
    let report = c.doctor.as_mut().unwrap();
    report.prerequisites = vec![
        dto(&Prerequisites::linux_host_item("6.12.1", "x86_64")),
        dto(&Prerequisites::kvm_item(crate::qemu_host::KvmState::NoAccess, false)),
        fx::prerequisite(I::Nix, S::Missing),
        fx::prerequisite(I::Qemu, S::Limited),
    ];
    report.prerequisites[2].install = Some(Prerequisites::INSTALL_NIX_LINUX.into());
    let page = get(&c, "doctor", json!({}));
    let sections: Vec<&str> = page["sections"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(sections, ["requirements", "nixos"]);
    let item = |id: &str| page["sections"][0]["items"].as_array().unwrap().iter().find(|i| i["item"] == id).unwrap();
    assert_eq!(item("host")["title"]["key"], "Linux with x86_64 or aarch64");
    assert_eq!(item("host")["status"], "ok");
    let kvm = item("kvm");
    assert_eq!(
        (kvm["status"].as_str(), kvm["title"]["key"].as_str()),
        (Some("limited"), Some("KVM (hardware virtualization)"))
    );
    assert!(kvm["hint"]["key"].as_str().unwrap().starts_with("Add yourself to the group kvm, then log in again."));
    let fix = action(&kvm["actions"], "install");
    assert_eq!(fix["method"], "ui.terminal");
    assert_eq!(fix["params"]["line"], Prerequisites::INSTALL_KVM_GROUP);
    // Nix: the installer in a terminal, not the Mac's administrator dialog
    assert_eq!(action(&item("nix")["actions"], "install")["method"], "ui.terminal");
    assert!(item("qemu")["hint"]["key"].as_str().unwrap().contains("nixpkgs#qemu"));
    let set_path = action(&item("qemu")["actions"], "setPath");
    assert_eq!(set_path["ask"][0]["prompt"]["key"], "The qemu-system-* binary");
}

#[test]
fn doctor_omaf_command() {
    use crate::prerequisites::PrerequisiteStatus as S;
    let cli_item = |c: &Ctx| {
        let page = get(c, "doctor", json!({}));
        page["sections"][0]["items"].as_array().unwrap().iter().find(|i| i["item"] == "cli").unwrap().clone()
    };
    let mut c = ctx();
    // missing: Install … with what it creates and the administrator; optional, so no issue
    let item = cli_item(&c);
    assert_eq!(item["status"], "missing");
    assert_eq!(ids(&item["actions"]), ["installCli"]);
    let install = action(&item["actions"], "installCli");
    assert_eq!(install["method"], "doctor.installCli");
    assert_eq!(install["confirm"]["text"]["args"][0], "/usr/local/bin/omaf");
    assert!(install["confirm"]["text"]["key"].as_str().unwrap().ends_with("macOS asks for an administrator."));
    assert_eq!(get(&c, "doctor", json!({}))["issues"], 3);
    // a file that is not omaflake's is in the way: Install is disabled and the hint says so
    c.doctor.as_mut().unwrap().cli.as_mut().unwrap().blocked = true;
    let item = cli_item(&c);
    assert_eq!(action(&item["actions"], "installCli")["enabled"], false);
    assert_eq!(item["hint"]["key"], "%@ is there and is not omaflake's – remove or rename it, then install.");
    // installed and found: ok, only Uninstall …
    {
        let d = c.doctor.as_mut().unwrap();
        let cli = d.cli.as_mut().unwrap();
        (cli.blocked, cli.installed) = (false, true);
        d.prerequisites.last_mut().unwrap().status = S::Ok;
    }
    let item = cli_item(&c);
    assert_eq!((item["expandable"].clone(), item["expanded"].clone()), (json!(true), json!(false)));
    assert_eq!(ids(&item["actions"]), ["uninstallCli"]);
    let uninstall = action(&item["actions"], "uninstallCli");
    assert_eq!(
        (uninstall["method"].clone(), uninstall["confirm"]["destructive"].clone()),
        (json!("doctor.uninstallCli"), json!(true))
    );
    // Linux: the link in ~/.local/bin, its folder not on the PATH
    {
        let d = c.doctor.as_mut().unwrap();
        let cli = d.cli.as_mut().unwrap();
        (cli.link, cli.on_path, cli.admin) = ("/home/u/.local/bin/omaf".into(), false, false);
        d.prerequisites.last_mut().unwrap().status = S::Limited;
    }
    let item = cli_item(&c);
    assert_eq!(
        item["hint"]["text"],
        "Add /home/u/.local/bin to your PATH in your shell's profile (export PATH=\"/home/u/.local/bin:$PATH\")."
    );
    assert_eq!(ids(&item["actions"]), ["installCli", "uninstallCli"]);
    assert!(!action(&item["actions"], "installCli")["confirm"]["text"]["key"]
        .as_str()
        .unwrap()
        .contains("administrator"));
    // while it runs, both wait
    c.jobs.push(fx::job("j99", JobKind::CliInstall, JobState::Running, None));
    let item = cli_item(&c);
    assert_eq!(item["busy"], true);
    assert_eq!(action(&item["actions"], "uninstallCli")["enabled"], false);
}

#[test]
fn mac_base_page() {
    let c = ctx();
    let page = get(&c, "macBase", json!({}));
    // The guest user comes from a credential – the default one; no password is asked (plan B139)
    assert_eq!(page["fields"][0]["id"], "credential");
    assert_eq!(page["fields"][0]["value"], "c-1");
    assert_eq!(page["base"]["tone"], "warning");
    let create = action(&page["actions"], "create");
    assert!(create.get("ask").is_none());
    assert_eq!(create["enabled"], false, "the IPSW downloads");
    assert_eq!(ids(&page["ipsw"]["actions"]), ["pause"]);
    let mut c = ctx();
    c.jobs.retain(|j| j.id != "j4");
    let page = get(&c, "macBase", json!({"values": {"ipsw": "/x/UniversalMac.ipsw", "credential": "c-1"}}));
    assert_eq!(page["ipsw"]["text"]["text"], "UniversalMac.ipsw");
    assert_eq!(ids(&page["ipsw"]["actions"]), ["useApple", "chooseIpsw"]);
    let create = action(&page["actions"], "create");
    assert_eq!(create["params"], json!({"credential": "c-1", "ipsw": "/x/UniversalMac.ipsw"}));
    assert_eq!(create["enabled"], true);
    // Without a credential: create one first
    let mut none = c.clone();
    none.credentials.clear();
    none.settings.default_credential = None;
    let empty = get(&none, "macBase", json!({}));
    assert_eq!(ids(&empty["actions"]), ["newCredential", "create"]);
    assert_eq!(action(&empty["actions"], "create")["enabled"], false);
    assert_eq!(page["log"]["job"], "j6", "the base's creation is a job like any other");
    assert_eq!(page["log"]["from"], 0);
    // The IPSW is shown once: in its line, set with ui.field on this page
    assert!(page["fields"].as_array().unwrap().iter().all(|f| f["id"] != "ipsw"));
    let choose = action(&page["ipsw"]["actions"], "chooseIpsw");
    assert_eq!(choose["params"], json!({"page": "macBase", "params": {}, "field": "ipsw", "valueParam": "ipsw"}));
    assert_eq!(action(&page["ipsw"]["actions"], "useApple")["params"]["value"], Value::Null);
    let credential = action(&page["fields"], "credential");
    assert_eq!(
        (credential["param"].clone(), credential["valueParam"].clone()),
        (json!("credential"), json!("credential"))
    );
    assert_eq!(create["primary"], true);
}

// MARK: Settings

#[test]
fn settings_fields_store_themselves() {
    let c = ctx();
    // The sources are a tab of the catalog (plan B147)
    let sources = get(&c, "sources", json!({}));
    assert_eq!(sources["title"]["key"], "Recipe Sources");
    let rows = &sources["sources"]["rows"];
    assert_eq!(rows[0]["actions"][0]["enabled"], false, "the first source cannot move up");
    assert_eq!(rows[1]["badges"][0]["help"]["key"], "Private – token in the Keychain");
    assert_eq!(
        action(&rows[1]["actions"], "enabled")["params"],
        json!({"id": "url:https://example.org/catalog.json", "enabled": true})
    );
    let page = get(&c, "settings", json!({}));
    assert!(page.get("sources").is_none());
    let dev = action(&page["general"]["fields"], "developerMode");
    assert_eq!(dev["set"], json!({"method": "settings.set", "params": {}, "param": "developerMode"}));
    assert_eq!(dev["value"], true);
    // This Mac: only its keyboard – language, region, time zone and the guests' keyboards are a locale's, the
    // defaults are set in their own places (plan B141)
    let ids: Vec<&str> =
        page["guest"]["fields"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["hostKeyboard"]);
    let keyboard = action(&page["guest"]["fields"], "hostKeyboard");
    assert_eq!(keyboard["set"]["param"], "hostKeyboard");
    assert_eq!(keyboard["choices"][0]["label"]["text"], "As the Mac says – ch");
    // A VM runs: the data folder stays
    let df = &page["dataFolder"];
    assert_eq!(action(&df["actions"], "move")["reason"]["key"], "Stop all VMs before moving the data folder.");
    assert_eq!(action(&df["actions"], "move")["confirm"]["title"]["args"][0], json!({"param": "target"}));
    // The new source's form: private only for GitHub, the token is asked when sent
    assert_eq!(sources["sources"]["addAction"]["enabled"], false);
    let page = get(&c, "sources", json!({"values": {"input": "acme/recipes", "private": true}}));
    let add = &page["sources"]["addAction"];
    assert_eq!(add["params"], json!({"input": "acme/recipes", "private": true}));
    assert_eq!(add["ask"][0]["param"], "token");
    assert_eq!(page["sources"]["add"]["fields"].as_array().unwrap().len(), 2);
    let page = get(&c, "sources", json!({"values": {"input": "https://example.org/c.json", "private": true}}));
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
    let tab = |id: &str| page["tabs"].as_array().unwrap().iter().find(|t| t["id"] == id).unwrap().clone();
    assert_eq!(tab("images")["badge"]["kind"], "working");
    assert_eq!(tab("recipes")["badge"]["kind"], "dot");
    assert!(tab("activity").get("badge").is_none(), "unseen activity is the UI's");
    assert_eq!(page["firstTab"], "vms");
}

/// The recipe studio's catalog a page at a time (plan B148): five modules, Next and Previous, a page that is gone
#[test]
fn recipe_studio_pages_through_the_catalog() {
    let mut c = ctx();
    let wezterm = c.modules.offered.iter().chain(c.modules.parts.iter()).find(|p| p.reference == "omaflake/wezterm");
    let wezterm = wezterm.cloned().unwrap();
    for i in 0..10 {
        let mut m = wezterm.clone();
        m.reference = format!("zoe/term{i:02}");
        m.id = format!("zoe.module.term{i:02}");
        m.title = format!("Term {i:02}");
        c.modules.offered.push(m);
    }
    let page = |n: Value| get(&c, "recipeStudio", json!({"values": {"page": n}}));
    let first = page(json!(null));
    let cat = &first["catalog"];
    assert_eq!(cat["rows"].as_array().unwrap().len(), 5);
    assert_eq!((cat["pages"]["index"].as_u64(), cat["pages"]["count"].as_u64()), (Some(0), Some(3)));
    assert_eq!(cat["pages"]["text"]["text"], "1–5 of 12");
    assert!(cat["pages"].get("previous").is_none());
    assert_eq!(cat["pages"]["next"]["params"]["value"], 1);
    let last = page(json!(2));
    assert_eq!(last["catalog"]["rows"].as_array().unwrap().len(), 2);
    assert_eq!(last["catalog"]["pages"]["text"]["text"], "11–12 of 12");
    assert!(last["catalog"]["pages"].get("next").is_none());
    assert_eq!(page(json!(9))["catalog"]["pages"]["index"], 2, "a page that is gone is the last one");
    // Typing: six suggestions at most
    let typed = get(&c, "recipeStudio", json!({"values": {"search": "term"}}));
    assert_eq!(typed["catalog"]["suggest"].as_array().unwrap().len(), 6);
}

/// The catalog tabs a page at a time (plan B148): search, chips (tags and capabilities), ten a page across those here
/// and those offered
#[test]
fn catalog_tabs_search_chips_and_pages() {
    let mut c = ctx();
    let wezterm = c.modules.offered.iter().chain(c.modules.parts.iter()).find(|p| p.reference == "omaflake/wezterm");
    let wezterm = wezterm.cloned().unwrap();
    for i in 0..12 {
        let mut m = wezterm.clone();
        m.reference = format!("zoe/term{i:02}");
        m.id = format!("zoe.module.term{i:02}");
        m.title = format!("Term {i:02}");
        m.tags = if i % 2 == 0 { vec!["even".into()] } else { vec![] };
        c.modules.offered.push(m);
    }
    let modules = |filter: Value| get(&c, "modules", json!({"filter": filter}));
    let first = modules(json!({}));
    let b = &first["browse"];
    assert_eq!(first["rows"].as_array().unwrap().len() + first["offered"].as_array().unwrap().len(), 10);
    assert_eq!((b["pages"]["index"].as_u64(), b["pages"]["count"].as_u64()), (Some(0), Some(2)));
    let next = &b["pages"]["next"];
    assert_eq!((next["method"].as_str(), &next["params"]["filter"]["page"]), (Some("ui.filter"), &json!(1)));
    assert_eq!(next["params"]["page"], "modules");
    let even = b["tags"].as_array().unwrap().iter().find(|t| t["id"] == "even").unwrap();
    assert_eq!((even["count"].as_u64(), even["on"].as_bool()), (Some(6), Some(false)));
    assert_eq!(even["action"]["params"]["filter"]["tags"], json!(["even"]));
    // A chip on: six; the search on top of it
    let tagged = modules(json!({"tags": ["even"]}));
    assert_eq!(tagged["offered"].as_array().unwrap().len(), 6);
    assert_eq!(tagged["count"], 6);
    assert!(tagged["browse"]["reset"].is_object());
    let searched = modules(json!({"tags": ["even"], "search": "term 04"}));
    assert_eq!(searched["offered"][0]["name"], "Term 04");
    // Nothing matches: said, with the way back
    let none = modules(json!({"search": "zzz"}));
    assert_eq!(none["empty"]["action"]["id"], "resetFilter");
    // The looks and the recipes of the catalog browse the same way
    assert!(get(&c, "looks", json!({}))["browse"]["pages"].is_object());
}

/// The Nix Store at the Doctor (plan B149): what it holds, and Clean Up (nix-collect-garbage) after a question
#[test]
fn doctor_cleans_up_the_nix_store() {
    let mut c = ctx();
    let mut space = crate::api::contract::prerequisite(
        crate::prerequisites::PrerequisiteItem::NixSpace,
        crate::prerequisites::PrerequisiteStatus::Ok,
    );
    (space.free_bytes, space.needed_bytes) = (Some(441_000_000_000), Some(30_000_000_000));
    c.doctor.as_mut().unwrap().nixos_preflight.push(space);
    let page = get(&c, "doctor", json!({}));
    let item = page["sections"]
        .as_array()
        .unwrap()
        .iter()
        .flat_map(|s| s["items"].as_array().unwrap().clone())
        .find(|i| i["item"] == "nixSpace")
        .unwrap();
    assert_eq!(item["detail"]["key"], "%@ free, about %@ needed – the Nix Store holds %@");
    let clean = action(&item["actions"], "cleanUpNix");
    assert_eq!(clean["method"], "nix.cleanUp");
    assert!(clean["confirm"].is_object(), "asks first");
}

/// The look studio (plan B150): starts from omaflake's colours without a look here; colours are fields with the colour
/// kind; what was chosen goes into the look Save writes; hard-to-read colours are said
#[test]
fn look_studio_colours_preview_and_contrast() {
    let c = ctx();
    let page = get(&c, "lookStudio", json!({"values": {"name": "dusk"}}));
    assert_eq!(page["title"]["key"], "Look Studio");
    let ids: Vec<&str> = page["sections"].as_array().unwrap().iter().map(|s| s["id"].as_str().unwrap()).collect();
    assert_eq!(ids, ["colors", "terminal", "desktop", "editors", "macos"]);
    let bg = &page["sections"][0]["fields"][0];
    assert_eq!((bg["id"].as_str(), bg["kind"].as_str()), (Some("color.background"), Some("color")));
    assert_eq!(page["sections"][1]["fields"].as_array().unwrap().len(), 16);
    assert!(page["hints"].as_array().unwrap().is_empty(), "omaflake's colours read well");
    // A chosen colour: in the preview and in what Save writes; grey text on grey is said
    let values = json!({"name": "dusk", "color.text": "#555555", "color.background": "#444444", "neovim": "rose-pine:moon",
        "border.1": "#ff00ff", "macos.wallpaper": "black"});
    let page = get(&c, "lookStudio", json!({"values": values}));
    assert_eq!(page["preview"]["text"], "#555555");
    assert_eq!(page["hints"][0]["text"]["key"], "Text is hard to read on the background (contrast %@ – at least 4.5)");
    let look = &action(&page["actions"], "save")["params"]["look"];
    assert_eq!(look["colors"]["background"], "#444444");
    assert_eq!(look["neovim"], json!({"theme": "rose-pine", "variant": "moon"}));
    assert_eq!(look["desktop"]["border"], json!(["#3fd0e0", "#ff00ff"]));
    assert_eq!(look["macos"], json!({"wallpaper": "black"}));
    // A colour that is not one: said at the field, Save waits; plain: no colours at all
    let page = get(&c, "lookStudio", json!({"values": {"name": "dusk", "color.accent": "teal"}}));
    assert!(page["sections"][0]["fields"][6]["error"].is_object());
    assert_eq!(action(&page["actions"], "save")["enabled"], false);
    let plain = get(&c, "lookStudio", json!({"values": {"name": "dusk", "plain": true}}));
    assert!(plain["sections"].as_array().unwrap().is_empty() && plain.get("preview").is_none());
    let look = &action(&plain["actions"], "save")["params"]["look"];
    assert!(look.get("colors").is_none() && look["plain"] == true);
}

/// The help switch (plan B148): off by default, the home page's action turns it on; a hint that only explains says so
#[test]
fn help_switch_on_the_home_page() {
    let mut c = ctx();
    let help = &get(&c, "home", json!({}))["help"];
    assert_eq!((help["on"].as_bool(), help["method"].as_str()), (Some(false), Some("settings.set")));
    assert_eq!(help["params"], json!({"help": true}));
    c.settings.help = true;
    let help = &get(&c, "home", json!({}))["help"];
    assert_eq!((help["on"].as_bool(), &help["params"]), (Some(true), &json!({"help": false})));
    let keys = get(&c, "keys", json!({"vm": "research"}));
    assert_eq!(keys["notes"][0]["help"], true, "explains: drawn only with help on");
    let warning = super::common::Hint::new("warning", Text::new("x"), super::common::Tone::Warning);
    assert!(serde_json::to_value(warning).unwrap().get("help").is_none(), "a warning is never help");
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
    assert_eq!(primaries(&get(&c, "recipeCatalog", json!({}))["offered"][0]["actions"]), ["install"]);
    assert_eq!(recipes["updates"]["rows"][0]["action"]["primary"], true);
    let new_vm = get(&c, "newVm", json!({"image": "omaflake.nixos.dev@cedar-mango-lift"}));
    assert_eq!(primaries(&new_vm["actions"]), ["create"]);
    let settings = get(&c, "sources", json!({}));
    assert_eq!(settings["sources"]["addAction"]["primary"], true);
    assert!(settings["sources"]["rows"][0]["actions"].as_array().unwrap().iter().all(|a| a.get("primary").is_none()));
    c.vms[0].status = VmStatus::Stopped;
    c.doctor.as_mut().unwrap().start_blockers.nixos.clear();
    assert_eq!(primaries(&get(&c, "vms", json!({}))["rows"][0]["actions"]), ["startWindow"]);

    let home = get(&c, "home", json!({}));
    let short: Vec<Option<&str>> =
        home["actions"].as_array().unwrap().iter().map(|a| a["shortLabel"]["key"].as_str()).collect();
    assert_eq!(short, [Some("Doctor"), None, Some("Data Folder"), Some("Quit")]);
    // Credentials, locales and profiles are tabs of the workshop now (plan B147), not header buttons
    assert!(home["actions"].as_array().unwrap().iter().all(|a| a["params"]["page"] != "credentials"));
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
    let settings = get(&c, "sources", json!({"values": {"input": "acme/recipes"}}));
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
        ("recipes", json!({}), "Build Images"),
        ("recipeCatalog", json!({}), "Recipes in your sources"),
        ("keymaps", json!({}), "Key Bindings"),
        ("sources", json!({}), "Recipe Sources"),
        ("activity", json!({}), "Activity"),
    ] {
        assert_eq!(get(&c, p, params)["title"]["text"], title);
    }
}

#[test]
fn recipes_say_what_they_show() {
    let mut c = ctx();
    let page = get(&c, "recipeCatalog", json!({}));
    assert!(page.get("offeredTitle").is_none(), "the tab says it: Recipes in your sources");
    assert!(page.get("loadingText").is_none() && page.get("labHint").is_none());
    c.fetching = true;
    c.recipes.lab_hidden = true;
    let page = get(&c, "recipeCatalog", json!({}));
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
    let keyboard = action(&settings["guest"]["fields"], "hostKeyboard");
    assert_eq!(keyboard["choices"][0]["label"]["key"], "As the system says – %@");
    assert_eq!(
        get(&c, "sources", json!({}))["sources"]["rows"][1]["badges"][0]["help"]["key"],
        "Private – token stored (keyring or tokens.json)"
    );
    let images = get(&c, "images", json!({}));
    assert_eq!(action(&images["rows"][0]["actions"], "reveal")["label"]["key"], "Show in File Manager");
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

// MARK: Credentials and bases (plan B137)

#[test]
fn credentials_are_rows_and_a_used_one_keeps_its_values() {
    let c = ctx();
    let page = get(&c, "credentials", json!({}));
    let work = &page["rows"][0];
    // A row as the other lists: lamp (in use), name, subtitle, the quick action; the rest when opened
    assert_eq!(work["lamp"]["state"], "on");
    assert_eq!(work["subtitle"]["text"], "alex · 1 image · 1 macOS base");
    assert_eq!(work["usedBy"], json!(["omaflake.nixos.dev@cedar-mango-lift", "mac-base-26A434"]));
    assert_eq!(ids(&work["actions"]), ["copyPassword"]);
    assert_eq!(work["actions"][0]["enabled"], true);
    assert_eq!(ids(&work["detail"]["actions"]), ["edit", "password", "default", "remove"]);
    let remove = action(&work["detail"]["actions"], "remove");
    assert_eq!(remove["enabled"], false);
    assert_eq!(remove["reason"]["key"], "A macOS base was created from it – remove the base first");
    // A base's account keeps its password
    let password = action(&work["detail"]["actions"], "password");
    assert_eq!(password["enabled"], false);
    let edit = action(&work["detail"]["actions"], "edit");
    assert_eq!(edit["method"], "ui.show");
    assert_eq!(edit["params"], json!({"page": "credential", "params": {"id": "c-1"}}));
    // Its form: images use it – only the name can change, the other fields say why
    let form = get(&c, "credential", json!({"id": "c-1"}));
    assert!(form["locked"]["key"].as_str().unwrap().starts_with("Images or a macOS base use it"));
    let fields = form["fields"].as_array().unwrap();
    assert_eq!(fields[0]["enabled"], true);
    for f in &fields[1..] {
        assert_eq!(f["enabled"], false, "{}", f["id"]);
        assert_eq!(f["reason"]["key"], "In use – only the name can change");
    }
    let save = action(&form["actions"], "save");
    assert_eq!(save["params"], json!({"id": "c-1", "name": "Work"}));
    // A new one: every field, the password asked twice when it is created; a taken name is refused
    let form = get(&c, "credential", json!({"values": {"name": "work", "user": "Alex!"}}));
    let create = action(&form["actions"], "create");
    assert_eq!(create["enabled"], false);
    assert_eq!(form["fields"][0]["error"]["key"], "There is a credential \"%@\" already");
    assert_eq!(
        form["fields"][1]["error"]["key"],
        "Invalid user name \"%@\": a–z, 0–9, \"_\" and \"-\", starting with a letter"
    );
    let form = get(&c, "credential", json!({"values": {"name": "Private", "user": "al", "email": "a@example.org"}}));
    let create = action(&form["actions"], "create");
    assert_eq!(create["enabled"], true);
    assert_eq!(create["params"]["email"], "a@example.org");
    assert_eq!(
        (create["ask"][0]["kind"].clone(), create["ask"][0]["repeat"].clone()),
        (json!("password"), json!(true))
    );
    assert_eq!(create["then"], "back");
    // Only the first credential starts from this computer's user; later ones from nothing (plan B139)
    assert_eq!(get(&c, "credential", json!({}))["fields"][1]["value"], "");
    let mut first = c.clone();
    first.credentials.clear();
    let form = get(&first, "credential", json!({}));
    assert_eq!(
        (form["fields"][1]["value"].clone(), form["fields"][2]["value"].clone()),
        (json!("alex"), json!("Alex Example"))
    );
}

#[test]
fn macos_bases_are_a_section_of_the_images_on_a_mac() {
    let mut c = ctx();
    let page = get(&c, "images", json!({}));
    let bases = &page["bases"];
    assert_eq!(bases["title"]["key"], "macOS Bases");
    let row = &bases["rows"][0];
    assert_eq!(row["name"], "mac-base-26A434");
    assert_eq!(row["badges"][0]["id"], "newest");
    assert!(row["subtitle"]["text"].as_str().unwrap().starts_with("macOS 26A434 · Work (alex)"));
    assert!(row["actions"].as_array().unwrap().iter().all(|a| a["id"] != "assign"));
    // A macOS build runs: the base stays
    assert_eq!(action(&row["actions"], "remove")["enabled"], false);
    c.jobs.clear();
    let page = get(&c, "images", json!({}));
    let remove = action(&page["bases"]["rows"][0]["actions"], "remove");
    assert_eq!((remove["enabled"].clone(), remove["method"].clone()), (json!(true), json!("bases.remove")));
    // A base from before credentials: a warning and the way to assign one – the form takes the one with its user
    c.bases[0].credential = None;
    c.bases[0].credential_name = None;
    let page = get(&c, "images", json!({}));
    let row = &page["bases"]["rows"][0];
    assert!(row["badges"].as_array().unwrap().iter().any(|b| b["id"] == "no-credential"));
    let assign = action(&row["actions"], "assign");
    assert_eq!(assign["params"], json!({"page": "baseCredential", "params": {"name": "mac-base-26A434"}}));
    let form = get(&c, "baseCredential", json!({"name": "mac-base-26A434"}));
    assert_eq!(form["fields"][0]["value"], "c-1");
    let save = action(&form["actions"], "save");
    assert_eq!(save["params"], json!({"name": "mac-base-26A434", "credential": "c-1"}));
    assert_eq!((save["enabled"].clone(), save["then"].clone()), (json!(true), json!("back")));
    let mut other = c.clone();
    other.credentials[0].user = Some("sam".into());
    let form = get(&other, "baseCredential", json!({"name": "mac-base-26A434"}));
    assert_eq!(form["fields"][0]["error"]["key"], "The base's user is %@ – this credential has %@");
    assert_eq!(action(&form["actions"], "save")["enabled"], false);
    // None: the way to create one; Linux: no section
    c.bases.clear();
    assert_eq!(get(&c, "images", json!({}))["bases"]["empty"]["action"]["id"], "macBase");
    c.host = HostOs::Linux;
    assert!(get(&c, "images", json!({})).get("bases").is_none());
}

// MARK: Looks (plan B141)

#[test]
fn looks_are_a_tab_with_their_colours_usage_and_offers() {
    let mut c = ctx();
    let home = get(&c, "home", json!({}));
    let tabs: Vec<&str> = home["tabs"].as_array().unwrap().iter().map(|t| t["id"].as_str().unwrap()).collect();
    assert_eq!(
        tabs,
        [
            "vms",
            "images",
            "activity",
            "recipes",
            "recipeStudio",
            "keymapStudio",
            "lookStudio",
            "recipeCatalog",
            "modules",
            "looks",
            "keymaps",
            "sources",
            "credentials",
            "locales",
            "profiles"
        ]
    );
    // Four groups (plan B147): run, build, find, who and how big
    let groups: Vec<&str> = home["groups"].as_array().unwrap().iter().map(|g| g["id"].as_str().unwrap()).collect();
    assert_eq!(groups, ["run", "workshop", "catalog", "profiles"]);
    let of = |g: &str| -> Vec<String> {
        home["tabs"]
            .as_array()
            .unwrap()
            .iter()
            .filter(|t| t["group"] == g)
            .map(|t| t["id"].as_str().unwrap().to_string())
            .collect()
    };
    assert_eq!(of("workshop"), ["recipes", "recipeStudio", "keymapStudio", "lookStudio"]);
    assert_eq!(of("catalog"), ["recipeCatalog", "modules", "looks", "keymaps", "sources"]);
    assert_eq!(of("profiles"), ["credentials", "locales", "profiles"]);
    let page = get(&c, "looks", json!({}));
    assert_eq!(page["title"]["key"], "Looks");
    let rose = &page["rows"][0];
    assert_eq!(rose["name"], "Rosé Pine Dawn");
    assert_eq!(rose["subtitle"]["text"], "omaflake/rose-pine-dawn · light · 1 image");
    assert_eq!(ids(&rose["badges"]), ["default", "official"]);
    assert_eq!(rose["swatches"].as_array().unwrap().len(), 11);
    // The default needs no Make Default; a look of the catalog is copied in the look studio (plan B150)
    assert_eq!(ids(&rose["actions"]), ["studio"]);
    assert_eq!(action(&rose["actions"], "studio")["label"]["key"], "Copy to My Looks …");
    let remove = action(&rose["detail"]["actions"], "remove");
    assert_eq!(
        (remove["enabled"].clone(), remove["reason"]["key"].clone()),
        (json!(false), json!("Images are built with it – remove them first"))
    );
    // A local folder's look: Make Default as its quick action, removed there
    let local = &page["rows"][1];
    assert_eq!(ids(&local["actions"]), ["studio", "default"]);
    assert!(ids(&local["badges"]).iter().any(|b| b == "folder"));
    assert_eq!(action(&local["detail"]["actions"], "remove")["enabled"], false);
    // Offered: Install from its source
    assert_eq!(page["offeredTitle"]["key"], "In your sources");
    let install = action(&page["offered"][0]["actions"], "install");
    assert_eq!(install["params"], json!({"id": "omaflake.look.catppuccin", "source": "github:omaflake/catalog@main"}));
    // None installed: the tab says so, offers the recommended ones, and its badge warns
    c.looks.looks.clear();
    c.looks.offered.clear();
    let page = get(&c, "looks", json!({}));
    assert_eq!(page["empty"]["action"]["method"], "looks.installRecommended");
    let home = get(&c, "home", json!({}));
    let looks = home["tabs"].as_array().unwrap().iter().find(|t| t["id"] == "looks").unwrap();
    assert_eq!(looks["badge"]["tone"], "warning");
    assert_eq!(home["groups"][2]["attention"], true, "the catalog shows that a tab of it needs attention");
    // The build page can't build without a look
    c.jobs.clear();
    let build = get(&c, "build", json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"}));
    assert_eq!(action(&build["actions"], "build")["enabled"], false);
}

// MARK: Locales (plan B141)

#[test]
fn locales_are_rows_and_a_form_like_credentials() {
    let c = ctx();
    let page = get(&c, "locales", json!({}));
    let de = &page["rows"][0];
    assert_eq!(de["name"], "German");
    assert_eq!(de["subtitle"]["text"], "de_DE · de + ru · Europe/Berlin · 1 image");
    assert_eq!(ids(&de["detail"]["actions"]), ["edit", "default", "remove"]);
    assert_eq!(action(&de["detail"]["actions"], "remove")["enabled"], false);
    // Its form: images use it – only the name; a new one: three keyboard fields, the second and third optional
    let mut c = c;
    c.settings_answer = Some(crate::api::contract::settings_answer());
    let form = get(&c, "locale", json!({"id": "l-1"}));
    assert!(form["locked"].is_object());
    let fields: Vec<&str> = form["fields"].as_array().unwrap().iter().map(|f| f["id"].as_str().unwrap()).collect();
    assert_eq!(fields, ["name", "locale", "timeZone", "keyboard", "keyboard2", "keyboard3"]);
    let form = get(&c, "locale", json!({"values": {"name": "Russian", "keyboard": "ru", "keyboard2": "de"}}));
    let create = action(&form["actions"], "create");
    assert_eq!(create["params"]["keyboards"], json!(["ru", "de"]));
    assert_eq!(create["enabled"], true);
    let form = get(&c, "locale", json!({"values": {"name": "Twice", "keyboard": "de", "keyboard2": "de"}}));
    assert_eq!(action(&form["actions"], "create")["enabled"], false);
    // The build page offers the locales, the default preset
    c.jobs.clear();
    let build = get(&c, "build", json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"}));
    let locale = action(&build["fields"], "locale");
    assert_eq!(
        (locale["value"].clone(), locale["choices"][0]["label"]["text"].clone()),
        (json!("l-1"), json!("German – de + ru"))
    );
}

// MARK: Building blocks (plan B142)

#[test]
fn modules_tab_profiles_and_the_recipe_studio() {
    let c = ctx();
    // The Modules tab: the shape of the Looks tab
    let page = get(&c, "modules", json!({}));
    assert_eq!(page["title"]["key"], "Modules");
    let rows = page["rows"].as_array().unwrap();
    assert_eq!(rows.iter().map(|r| r["name"].as_str().unwrap()).collect::<Vec<_>>(), ["Hyprland"]);
    // Key bindings: a tab of their own (plan B147), each opening in the key binding studio
    let keymaps = get(&c, "keymaps", json!({}));
    assert_eq!(keymaps["title"]["key"], "Key Bindings");
    let set = &keymaps["rows"][0];
    assert_eq!(set["name"], "omaflake");
    assert_eq!(
        action(&set["actions"], "studio")["params"],
        json!({"page": "keymapStudio", "params": {"id": "omaflake.keymap.default"}})
    );
    assert_eq!(rows[0]["subtitle"]["text"], "omaflake/hyprland · 1.0.0 · NixOS module · 1 recipe");
    assert_eq!(ids(&rows[0]["actions"]), ["keys", "update"], "its keys as caps in the keys window (plan B148)");
    assert_eq!(
        action(&rows[0]["actions"], "keys")["params"]["params"],
        json!({"module": "omaflake/hyprland", "platform": "nixos"})
    );
    let remove = action(&rows[0]["detail"]["actions"], "remove");
    assert_eq!(remove["confirm"]["text"]["key"], "Recipes made of it cannot be built until it is back: %@");
    let vscode = page["offered"].as_array().unwrap().iter().find(|o| o["name"] == "VS Code").unwrap();
    assert_eq!(vscode["actions"][0]["method"], "modules.install");
    assert!(vscode["badges"].as_array().unwrap().iter().any(|b| b["id"] == "unfree"));
    // Profiles: the shape of the credentials; the form checks its name
    let profiles = get(&c, "profiles", json!({}));
    assert_eq!(profiles["rows"][0]["subtitle"]["text"], "2 CPU · 4 GB RAM");
    let form = get(&c, "profile", json!({"values": {"name": "small"}}));
    assert_eq!(action(&form["actions"], "create")["enabled"], false, "the name is taken (any case)");
    let form = get(&c, "profile", json!({"values": {"name": "Big", "cpus": 8, "memoryGB": 32}}));
    assert_eq!(action(&form["actions"], "create")["params"], json!({"name": "Big", "cpus": 8, "memoryGB": 32}));
    // The recipe studio (plan B147): a copy of dev – its modules that are here or offered are in the recipe
    let studio = get(&c, "recipeStudio", json!({"id": "omaflake.nixos.dev@velvet-tiger-orbit"}));
    assert_eq!(studio["title"]["key"], "Copy to My Recipes");
    let refs = |rows: &Value| -> Vec<String> {
        rows.as_array().unwrap().iter().map(|r| r["reference"].as_str().unwrap().to_string()).collect()
    };
    assert_eq!(refs(&studio["recipe"]["rows"]), ["omaflake/hyprland", "omaflake/wezterm"]);
    // The catalog shows them too, checked (plan B148); NixOS only: VS Code is macOS
    assert_eq!(refs(&studio["catalog"]["rows"]), ["omaflake/hyprland", "omaflake/wezterm"]);
    assert_eq!(studio["catalog"]["rows"][0]["inside"], true);
    assert_eq!(studio["catalog"]["count"]["text"], "2 modules");
    let save = action(&studio["actions"], "save");
    assert_eq!(save["params"]["modules"], json!(["omaflake/hyprland", "omaflake/wezterm"]));
    assert_eq!(save["params"]["keymaps"], json!(["omaflake/default"]));
    // Key binding sets as a list (plan B153): each taken out or moved; those not in it to add
    let km = &studio["keymaps"];
    assert_eq!(km["rows"][0]["reference"], "omaflake/default");
    assert_eq!(km["rows"][0]["remove"]["params"]["value"], json!([]));
    assert!(km["rows"][0].get("up").is_none() && km["add"].as_array().unwrap().is_empty());
    let none =
        get(&c, "recipeStudio", json!({"id": "omaflake.nixos.dev@velvet-tiger-orbit", "values": {"keymaps": []}}));
    assert_eq!(none["keymaps"]["empty"]["key"], "None – every module has its own keys");
    let add = &none["keymaps"]["add"][0];
    assert_eq!(
        (add["params"]["valueParam"].as_str(), &add["params"]["value"]),
        (Some("keymaps"), &json!(["omaflake/default"]))
    );
    assert_eq!(action(&none["actions"], "save")["params"]["keymaps"], json!([]));
    // Removing sets the whole list (ui.field on the page)
    let remove = &studio["recipe"]["rows"][1]["action"];
    assert_eq!(
        (remove["method"].clone(), remove["params"]["valueParam"].clone()),
        (json!("ui.field"), json!("modules"))
    );
    assert_eq!(remove["params"]["value"], json!(["omaflake/hyprland"]));
    assert_eq!(remove["params"]["params"], json!({"id": "omaflake.nixos.dev@velvet-tiger-orbit"}));
    // A new recipe with Hyprland only: it needs a terminal – said, with the one to add; WezTerm is in the catalog
    let new = get(&c, "recipeStudio", json!({"values": {"modules": ["omaflake/hyprland"]}}));
    assert_eq!(refs(&new["catalog"]["rows"]), ["omaflake/hyprland", "omaflake/wezterm"]);
    let wezterm = &new["catalog"]["rows"][1];
    assert_eq!(wezterm["inside"], false);
    let about: Vec<&str> =
        wezterm["details"].as_array().unwrap().iter().filter_map(|l| l["label"]["key"].as_str()).collect();
    assert!(about.contains(&"Provides") && about.contains(&"Source"), "{about:?}");
    assert!(wezterm["badges"].as_array().unwrap().iter().any(|b| b["id"] == "offered"));
    assert_eq!(wezterm["action"]["params"]["value"], json!(["omaflake/hyprland", "omaflake/wezterm"]));
    let need = &new["recipe"]["hints"][0];
    assert_eq!(need["text"]["key"], "%@ needs a %@");
    assert_eq!(need["action"]["label"]["text"], "Add WezTerm");
    assert_eq!(need["action"]["params"]["value"], json!(["omaflake/hyprland", "omaflake/wezterm"]));
    assert_eq!(need["action"]["params"]["params"], json!({}));
    // A search that matches nothing; Show all clears it and the chips
    let searched = get(&c, "recipeStudio", json!({"values": {"modules": ["omaflake/hyprland"], "search": "zzz"}}));
    assert_eq!(searched["catalog"]["empty"]["key"], "No module matches – change the search or the tags");
    assert_eq!(searched["catalog"]["reset"]["params"]["also"][0]["valueParam"], "tags");
    // Suggestions while typing: the title first; taking one adds it and ends the search
    let typed = get(&c, "recipeStudio", json!({"values": {"modules": ["omaflake/hyprland"], "search": "wez"}}));
    let first = &typed["catalog"]["suggest"][0];
    assert_eq!((first["reference"].as_str(), first["inside"].as_bool()), (Some("omaflake/wezterm"), Some(false)));
    assert_eq!(first["action"]["params"]["value"], json!(["omaflake/hyprland", "omaflake/wezterm"]));
    assert_eq!(first["action"]["params"]["also"][0]["valueParam"], "search");
    // Chips: tags and capabilities; one on narrows the list and goes back to the first page
    let filtered = get(&c, "recipeStudio", json!({"values": {"tags": ["desktop"]}}));
    assert_eq!(refs(&filtered["catalog"]["rows"]), ["omaflake/hyprland"]);
    let chip = filtered["catalog"]["tags"].as_array().unwrap().iter().find(|t| t["id"] == "desktop").unwrap();
    assert_eq!((chip["on"].as_bool(), &chip["action"]["params"]["value"]), (Some(true), &json!([])));
    assert_eq!(chip["action"]["params"]["also"][0]["valueParam"], "page");
    assert_eq!(filtered["title"]["key"], "Recipe Studio");
    let none = get(&c, "recipeStudio", json!({"values": {"name": "x"}}));
    assert_eq!(action(&none["actions"], "save")["reason"]["key"], "Choose at least one module");
    assert_eq!(none["recipe"]["empty"]["key"], "Add modules from the catalog – a desktop, a terminal, an editor …");
}
