// SPDX-License-Identifier: AGPL-3.0-or-later
//! The TUI on the golden page models of the core (`crates/omaflake-core/testdata/api/views/`): drawn to ratatui's
//! `TestBackend`, and keys through actions to the requests they send – against a core that records them.

use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use omaflake_tui::app::{App, Command, Effect, Modal, Msg};
use omaflake_tui::core::{Core, CoreError};
use omaflake_tui::icons::IconSet;
use omaflake_tui::theme::{ColorMode, Theme};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use serde_json::{json, Value};

type Answerer = dyn Fn(&str, &Value) -> Value + Send + Sync;

/// Records every request; answers with `answer`
struct Fake {
    calls: Mutex<Vec<(String, Value)>>,
    answer: Box<Answerer>,
}

impl Core for Fake {
    fn call(&self, method: &str, params: Value) -> Result<Value, CoreError> {
        let a = (self.answer)(method, &params);
        self.calls.lock().unwrap().push((method.to_string(), params));
        Ok(a)
    }
}

fn fixture(name: &str) -> Value {
    let path = format!("{}/../omaflake-core/testdata/api/views/{name}.json", env!("CARGO_MANIFEST_DIR"));
    serde_json::from_str(&std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("{path}: {e}"))).unwrap()
}

struct Harness {
    app: App,
    rx: Receiver<Msg>,
    fake: Arc<Fake>,
}

impl Harness {
    fn new(answer: impl Fn(&str, &Value) -> Value + Send + Sync + 'static) -> Harness {
        let fake = Arc::new(Fake { calls: Mutex::default(), answer: Box::new(answer) });
        let (tx, rx) = mpsc::channel();
        // The expected texts are English, whatever the locale of the test run
        omaflake_tui::text::set_language(omaflake_tui::text::Language::English);
        let mut app = App::new(fake.clone(), tx, Theme::new(ColorMode::None), IconSet::Plain, "test");
        app.start();
        let mut h = Harness { app, rx, fake };
        h.settle();
        h.model("home", json!({}), fixture("home"));
        h
    }

    /// A page's model arrives
    fn model(&mut self, page: &str, params: Value, model: Value) {
        self.app.handle(Msg::Event("view.changed".into(), json!({"page": page, "params": params, "model": model})));
    }

    /// Answers of the requests sent so far are handled
    fn settle(&mut self) {
        let end = Instant::now() + Duration::from_millis(300);
        while Instant::now() < end {
            match self.rx.recv_timeout(Duration::from_millis(50)) {
                Ok(m) => self.app.handle(m),
                Err(_) if self.app_waits() => continue,
                Err(_) => break,
            }
        }
    }

    fn app_waits(&self) -> bool {
        false
    }

    fn key(&mut self, code: KeyCode) {
        self.app.key(KeyEvent::new(code, KeyModifiers::NONE));
        self.settle();
    }

    fn type_text(&mut self, text: &str) {
        for c in text.chars() {
            self.app.key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
        }
    }

    fn calls(&self, method: &str) -> Vec<Value> {
        self.fake.calls.lock().unwrap().iter().filter(|(m, _)| m == method).map(|(_, p)| p.clone()).collect()
    }

    fn screen(&mut self) -> String {
        self.screen_sized(140, 45)
    }

    fn screen_sized(&mut self, width: u16, height: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(width, height)).unwrap();
        term.draw(|f| omaflake_tui::ui::draw(f, &mut self.app)).unwrap();
        let buf = term.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buf.area.height {
            for x in 0..buf.area.width {
                out.push_str(buf[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    /// Shows `page` on top (as `ui.show`) with its model
    fn show(&mut self, page: &str, params: Value, model: Value) {
        self.app.run_action(&json!({"id": "show", "label": {"text": "x"}, "method": "ui.show", "enabled": true,
                                    "params": {"page": page, "params": params}}));
        self.model(page, params, model);
    }
}

fn none(_: &str, _: &Value) -> Value {
    Value::Null
}

#[test]
fn subscribes_home_activity_and_the_first_tab() {
    let h = Harness::new(none);
    let subs = h.calls("views.subscribe");
    let pages: Vec<&str> = subs[0]["pages"].as_array().unwrap().iter().map(|p| p["page"].as_str().unwrap()).collect();
    assert_eq!(pages, ["home", "activity", "vms"]);
    assert_eq!(h.calls("events.subscribe")[0], json!({"topics": ["jobs"]}));
}

/// Tab goes on from the last tab of the running group into the workshop, whose tabs the line shows then (plan B147)
#[test]
fn tab_goes_on_into_the_workshop() {
    let mut h = Harness::new(none);
    h.model("vms", json!({}), fixture("vms"));
    for _ in 0..4 {
        h.key(KeyCode::Tab);
    }
    assert_eq!(h.app.tab, "recipeStudio", "the workshop's first tab");
    h.key(KeyCode::Tab);
    assert_eq!(h.app.tab, "modules");
    h.model("modules", json!({}), fixture("modules"));
    let s = h.screen();
    // The running group needs attention (a build runs): its name has the dot
    assert!(s.contains("Recipe Studio") && s.contains("Modules") && s.contains("│ Run • ›"), "{s}");
    assert!(!s.contains("Images"), "the running group's tabs are hidden: {s}");
}

#[test]
fn draws_the_vms_tab() {
    let mut h = Harness::new(none);
    h.model("vms", json!({}), fixture("vms"));
    let s = h.screen();
    for want in [
        "VMs 1",
        "Images",
        "Recipes",
        "Activity",
        "Virtual Machines",
        "research",
        "work",
        "dev (velvet) · Work · omaflake · 4 CPU · 8 GB · SSH 2201",
        " omaflake ",
        // home's short labels; the letters come from what is written
        "D Doctor 3",
        "S Settings  A Data Folder  q Quit",
        // the other group by its name (plan B147)
        "│ Workshop ›",
        "/ Filter",
        "i ⇩ Import",
        "r ↗ Run from There",
        "1 ⌨ Keys …",
        "2 ▤ Show Share Folder",
        "3 ›_ Open Terminal with SSH",
        "4 ■ Shut down",
    ] {
        assert!(s.contains(want), "missing {want:?} in\n{s}");
    }
    // Details open with Space: switches, info lines, detail actions
    h.key(KeyCode::Char(' '));
    let s = h.screen();
    for want in [
        "[x] Clipboard",
        "Changes take effect at the next start.",
        "omaflake.nixos.dev@cedar-mango-lift",
        "newer image available",
        "Runs from",
        "9 ⇧ Export …",
        "Remove …",
    ] {
        assert!(s.contains(want), "missing {want:?} in\n{s}");
    }
}

#[test]
fn draws_every_page() {
    // The titles are the model's (`‹ Esc  <title>`)
    let pages = [
        (
            "images",
            "images",
            vec!["‹ Esc  Images", "dev (velvet)", "Copying the image", "120/400", "62 %", "cedar-mango-lift"],
        ),
        (
            "recipes",
            "recipes",
            vec![
                "‹ Esc  Recipes",
                "Newer recipe versions in your sources",
                "omaflake/nixos/dev",
                "minimal",
                "example.org: HTTP 404",
                "More recipes in your sources",
            ],
        ),
        (
            "recipes",
            "recipes.loading",
            vec![
                "Looking into the recipe sources …",
                "More recipes in your sources",
                "Lab recipes are hidden – Developer Mode in Settings shows them.",
            ],
        ),
        ("activity", "activity", vec!["research is ready", "work: running – stop it first", "c ✕ Clear History"]),
        // A recipe's keys (plan B147): by module and mode, the way there, where a key leads
        (
            "keys",
            "keys",
            vec!["Hyprland", "Zellij", "Locked", "⌃+g › p", "h / ←", "New pane", "→ then Pane", "↩ then Locked"],
        ),
        (
            "build",
            "build",
            vec!["‹ Esc  Build Image", "dev (velvet)", "Copying the image", "62 %", "Full Log", "Cancel"],
        ),
        ("build", "build.failed", vec!["‹ Esc  Build Image", "Build failed"]),
        // The credential and look of a new image (plan B137)
        ("build", "build.macos", vec!["‹ Esc  Build Image", "Credential", "Work – alex", "Look", "Rosé Pine Dawn"]),
        (
            "credentials",
            "credentials",
            vec![
                "‹ Esc  Credentials",
                "Who you are in a guest",
                "Work",
                "alex · 1 image",
                "default",
                "Copy Password",
                "New Credential …",
                "login keychain",
            ],
        ),
        (
            "credential",
            "credential.edit",
            vec!["‹ Esc  Edit Credential", "only its name can change", "Name", "User", "Cancel", "Save"],
        ),
        ("images", "images", vec!["macOS Bases", "mac-base-26A434", "macOS 26A434 · Work (alex)", "newest"]),
        ("doctor", "doctor", vec!["‹ Esc  Doctor", "omaf ui", "Core", "Up to date", "Nix", "Check Again"]),
        (
            "settings",
            "settings",
            vec![
                "‹ Esc  Settings",
                "Recipe Sources",
                "omaflake/recipes",
                "General",
                "[x] Check for updates",
                "Data Folder",
            ],
        ),
        ("settings", "settings.linux", vec!["‹ Esc  Settings", "Recipe Sources", "on this computer"]),
        ("newVm", "newVm", vec!["‹ Esc  New VM", "CPU cores: 2", "‹ 4 ›", "Create", "Cancel"]),
        ("macBase", "macBase", vec!["‹ Esc  Create macOS Base", "Create Base", "differs from macUser", "Full Log"]),
        ("vm", "vm", vec!["‹ Esc  research", "Runs from", "Remove …"]),
        ("image", "image", vec!["‹ Esc  omaflake.macos.dev@stone-lamp", "Remove …"]),
        // Plan B142: modules in the looks' shape, profiles and the recipe editor in the credentials'
        ("modules", "modules", vec!["Modules", "Hyprland", "omaflake/hyprland · 1.0.0 · NixOS module", "VS Code"]),
        ("profiles", "profiles", vec!["Profiles", "Small", "2 CPU · 4 GB RAM", "New Profile …"]),
        ("profile", "profile", vec!["New Profile", "CPU cores: 8", "Create"]),
        (
            "recipeStudio",
            "recipeStudio",
            vec![
                "Copy to My Recipes",
                "Your recipe – 2 modules",
                "Hyprland",
                "Catalog",
                "Every module is in the recipe",
                "Key bindings 1",
                "Save",
            ],
        ),
    ];
    for (page, file, wants) in pages {
        let mut h = Harness::new(none);
        let params = json!({"name": "x"});
        h.show(page, params, fixture(file));
        let s = h.screen();
        // OMAF_TUI_DUMP=1 cargo test -p omaflake-tui -- --nocapture: every page as drawn
        if std::env::var_os("OMAF_TUI_DUMP").is_some() {
            println!(
                "== {file}\n{}",
                s.lines().map(str::trim_end).filter(|l| !l.is_empty()).collect::<Vec<_>>().join("\n")
            );
        }
        for want in wants {
            assert!(s.contains(want), "{file}: missing {want:?} in\n{s}");
        }
    }
}

#[test]
fn build_log_comes_from_jobs_log_and_job_log() {
    let mut h = Harness::new(|m, _| match m {
        "jobs.log" => json!({"lines": ["building '/nix/store/abc'", "copying path"], "next": 2}),
        _ => Value::Null,
    });
    h.show("build", json!({"recipe": "r"}), fixture("build"));
    h.settle();
    assert_eq!(h.calls("jobs.log"), [json!({"id": "j1", "from": 0})]);
    h.app.handle(Msg::Event("job.log".into(), json!({"id": "j1", "lines": ["[3/5] Copying the image"]})));
    let s = h.screen();
    assert!(s.contains("building '/nix/store/abc'") && s.contains("[3/5] Copying the image"), "{s}");
}

#[test]
fn mac_base_log_is_a_log_ref_and_the_ipsw_is_shown_once() {
    let mut h = Harness::new(|m, _| match m {
        "jobs.log" => json!({"lines": ["Installing macOS"], "next": 41}),
        _ => Value::Null,
    });
    let mut model = fixture("macBase");
    model["log"]["from"] = json!(40);
    h.show("macBase", json!({}), model);
    h.settle();
    assert_eq!(h.calls("jobs.log"), [json!({"id": "j6", "from": 40})]);
    let s = h.screen();
    assert!(s.contains("Installing macOS"), "{s}");
    assert_eq!(s.matches("macOS from Apple (IPSW)").count(), 1, "{s}");
}

#[test]
fn ui_field_sets_the_value_of_its_page_by_value_param() {
    let mut h = Harness::new(none);
    h.show("macBase", json!({}), fixture("macBase"));
    let field = |value: Option<Value>| {
        let mut a = json!({"id": "chooseIpsw", "label": {"text": "Choose IPSW …"}, "method": "ui.field",
            "enabled": true, "params": {"page": "macBase", "params": {}, "field": "ipsw", "valueParam": "ipsw"}});
        match value {
            Some(v) => a["params"]["value"] = v,
            None => {
                a["ask"] = json!([{"kind": "text", "label": {"text": "Choose IPSW …"}, "param": "value"}]);
            }
        }
        a
    };
    // No value: asked with its ask
    h.app.run_action(&field(None));
    h.type_text("/tmp/a.ipsw");
    h.key(KeyCode::Enter);
    assert_eq!(h.app.stack.last().unwrap().params, json!({"values": {"ipsw": "/tmp/a.ipsw"}}));
    // The credential of the base is a field of the page too (plan B139)
    let page = h.app.page();
    assert!(page.items.iter().any(|i| i.key == "macBase:field:credential"));
    // value null: back to the default (Apple's)
    h.app.run_action(&field(Some(Value::Null)));
    h.settle();
    assert_eq!(h.app.stack.last().unwrap().params, json!({"values": {}}));
    let last = h.calls("views.subscribe").pop().unwrap();
    assert_eq!(last["pages"][3], json!({"page": "macBase", "params": {"values": {}}}));
}

#[test]
fn enter_does_the_primary_action_only() {
    let mut h = Harness::new(none);
    // A build job row: its primary `open` shows the build page
    h.model("images", json!({}), fixture("images"));
    h.key(KeyCode::Tab);
    h.model("images", json!({}), fixture("images"));
    h.key(KeyCode::Enter);
    assert_eq!(h.app.stack.last().map(|p| p.page.as_str()), Some("build"));
    h.key(KeyCode::Esc);
    // A row whose primary is the Doctor's warning
    let images = fixture("images");
    let page = h.app.page();
    let row = page.items.iter().find(|i| i.key.starts_with("image:")).unwrap();
    assert_eq!(row.default.map(|d| row.actions[d]["id"].clone()), Some(images["rows"][0]["actions"][2]["id"].clone()));
    // A hint without a primary (the lab hint) has no default: Enter opens its actions
    let mut h = Harness::new(none);
    h.key(KeyCode::Tab);
    h.key(KeyCode::Tab);
    h.model("recipes", json!({}), fixture("recipes.loading"));
    let page = h.app.page();
    let lab = page.items.iter().find(|i| i.key == "labHint").unwrap();
    assert_eq!(lab.default, None);
    let offered = page.items.iter().find(|i| i.key.starts_with("offered:")).unwrap();
    assert_eq!(offered.default.map(|d| offered.actions[d]["id"].clone()), Some(json!("install")));
}

#[test]
fn enter_runs_the_default_action_and_then_terminal() {
    let mut h = Harness::new(|m, _| match m {
        "vms.ssh" => json!({"argv": ["/usr/bin/ssh", "-p", "2201", "alex@127.0.0.1"], "shellLine": "ssh …"}),
        _ => Value::Null,
    });
    h.model("vms", json!({}), fixture("vms"));
    h.key(KeyCode::Enter);
    assert_eq!(h.calls("vms.ssh"), [json!({"name": "research"})]);
    assert_eq!(
        h.app.effects.pop_front(),
        Some(Effect::Terminal(Command::Argv(
            ["/usr/bin/ssh", "-p", "2201", "alex@127.0.0.1"].map(String::from).to_vec()
        )))
    );
    // 1 = its keys (plan B147), 2 = show the share folder: vms.share, then reveal its path
    h.key(KeyCode::Char('2'));
    assert_eq!(h.calls("vms.share").len(), 1);
}

#[test]
fn a_destructive_action_asks_first() {
    let mut h = Harness::new(none);
    h.model("vms", json!({}), fixture("vms"));
    // Selected: research; reset is one of its detail actions – disabled while it runs: the reason goes to the
    // status line, nothing is sent
    h.key(KeyCode::Char('a'));
    let Some(Modal::Menu { actions, .. }) = &h.app.modal else { panic!("no menu") };
    let reset = actions.iter().position(|a| a["id"] == "reset").unwrap();
    for _ in 0..reset {
        h.key(KeyCode::Down);
    }
    assert!(h.screen().contains("Shut the VM down first"));
    h.key(KeyCode::Enter);
    assert!(h.calls("vms.reset").is_empty());
    assert_eq!(h.app.status.as_ref().unwrap().0, "Shut the VM down first");
    // An enabled one with a question: stop has none, so take the quit of home
    h.key(KeyCode::Char('q'));
    assert!(matches!(h.app.modal, Some(Modal::Confirm { .. })));
    let s = h.screen();
    assert!(s.contains("Quit omaflake?") && s.contains("These VMs keep running without the app: research"), "{s}");
    h.key(KeyCode::Esc);
    assert!(!h.app.quit && h.app.modal.is_none());
    h.key(KeyCode::Char('q'));
    h.key(KeyCode::Enter);
    assert!(h.app.quit);
}

#[test]
fn the_action_menu_scrolls_to_its_selection_in_a_small_terminal() {
    let mut h = Harness::new(none);
    h.model("vms", json!({}), fixture("vms"));
    h.key(KeyCode::Char('a'));
    let Some(Modal::Menu { actions, .. }) = &h.app.modal else { panic!("no menu") };
    let last = actions.len() - 1;
    assert_eq!(actions[last]["id"], "remove");
    // 14 rows: the menu of eight actions (most with a second line) does not fit – the top is shown, more below
    let s = h.screen_sized(100, 14);
    assert!(s.contains("Show Share Folder") && !s.contains("Remove …") && s.contains(" ↓ "), "{s}");
    for _ in 0..last {
        h.key(KeyCode::Char('j'));
    }
    let s = h.screen_sized(100, 14);
    assert!(s.contains("Remove …") && !s.contains("Show Share Folder") && s.contains(" ↑ "), "{s}");
    // Back to the first: the top again
    for _ in 0..last {
        h.key(KeyCode::Char('k'));
    }
    let s = h.screen_sized(100, 14);
    assert!(s.contains("Show Share Folder") && !s.contains("Remove …"), "{s}");
    // A terminal it fits into: everything, no arrows
    let s = h.screen();
    assert!(s.contains("Show Share Folder") && s.contains("Remove …") && !s.contains(" ↓ "), "{s}");
}

#[test]
fn export_asks_a_folder_with_completion() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::create_dir(dir.path().join("exports")).unwrap();
    let mut h = Harness::new(none);
    // An image's export (a running VM's is greyed out)
    let images = fixture("images");
    let export = images["rows"][0]["detail"]["actions"][0].clone();
    assert_eq!(export["id"], "export");
    h.app.run_action(&export);
    let Some(Modal::Prompt(p)) = &h.app.modal else { panic!("no prompt") };
    assert_eq!(p.kind, "folder");
    h.app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    h.type_text(&format!("{}/exp", dir.path().display()));
    h.key(KeyCode::Tab);
    let Some(Modal::Prompt(p)) = &h.app.modal else { panic!("no prompt") };
    assert_eq!(p.input, format!("{}/exports/", dir.path().display()));
    h.key(KeyCode::Enter);
    let sent = h.calls("packages.export");
    let mut want = export["params"].clone();
    want["folder"] = json!(format!("{}/exports", dir.path().display()));
    assert_eq!(sent, [want]);
}

#[test]
fn import_steps_through_the_check_and_the_question() {
    let dir = tempfile::tempdir().unwrap();
    let pkg = dir.path().join("research.omaflake-vm");
    std::fs::create_dir(&pkg).unwrap();
    let mut h = Harness::new(|m, _| match m {
        "packages.inspect" => json!({"kind": "vm", "valid": true}),
        "views.get" => fixture("import"),
        _ => Value::Null,
    });
    h.model("vms", json!({}), fixture("vms"));
    h.key(KeyCode::Char('i'));
    h.app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    h.type_text(&pkg.display().to_string());
    h.key(KeyCode::Enter);
    let path = pkg.display().to_string();
    assert_eq!(h.calls("packages.inspect"), [json!({"expect": "vm", "external": false, "path": path})]);
    assert_eq!(h.calls("views.get"), [json!({"page": "import", "params": {"expect": "vm", "path": path}})]);
    // The VM is here already: the question with the name to import under
    let Some(Modal::Confirm { input, .. }) = &h.app.modal else { panic!("no question") };
    assert_eq!(input.as_deref(), Some("research-2"));
    h.key(KeyCode::Backspace);
    h.type_text("3");
    h.key(KeyCode::Enter);
    assert_eq!(
        h.calls("packages.import"),
        [json!({"path": "/Volumes/Disk/research.omaflake-vm", "name": "research-3"})]
    );
}

#[test]
fn a_package_the_check_refuses_stays_in_the_prompt() {
    let dir = tempfile::tempdir().unwrap();
    let mut h = Harness::new(|m, _| match m {
        "packages.inspect" => json!({"kind": "image", "valid": false, "reason": {"text": "Not a VM"}}),
        _ => Value::Null,
    });
    h.model("vms", json!({}), fixture("vms"));
    h.key(KeyCode::Char('i'));
    h.app.key(KeyEvent::new(KeyCode::Char('u'), KeyModifiers::CONTROL));
    h.type_text(&dir.path().display().to_string());
    h.key(KeyCode::Enter);
    let Some(Modal::Prompt(p)) = &h.app.modal else { panic!("prompt closed") };
    assert_eq!(p.error.as_deref(), Some("Not a VM"));
    assert!(h.calls("views.get").is_empty());
}

#[test]
fn filter_and_tabs_subscribe_again() {
    let mut h = Harness::new(none);
    h.model("vms", json!({}), fixture("vms"));
    h.key(KeyCode::Char('/'));
    h.key(KeyCode::Right);
    h.key(KeyCode::Enter);
    let last = h.calls("views.subscribe").pop().unwrap();
    assert_eq!(last["pages"][2], json!({"page": "vms", "params": {"filter": {"status": true}}}));
    h.key(KeyCode::Tab);
    let last = h.calls("views.subscribe").pop().unwrap();
    assert_eq!(last["pages"][2], json!({"page": "images", "params": {}}));
    // The reset of the filter (ui.filter with null) on the vms page
    h.key(KeyCode::BackTab);
    h.app.run_action(&json!({"id": "resetFilter", "label": {"text": "Reset Filter"}, "method": "ui.filter",
                             "enabled": true, "params": {"page": "vms", "filter": null}}));
    h.settle();
    let last = h.calls("views.subscribe").pop().unwrap();
    assert_eq!(last["pages"][2], json!({"page": "vms", "params": {}}));
}

#[test]
fn form_fields_go_into_the_page_values_or_are_set() {
    let mut h = Harness::new(none);
    h.show("newVm", json!({"image": "img"}), fixture("newVm"));
    // To the CPU field and one up
    let page = h.app.page();
    // (CPU cores and RAM come from the default profile: the disk is a field of its own)
    let cpus = page.items.iter().position(|i| i.key == "newVm:field:diskGB").unwrap();
    let first = h.app.selected_index(&page).unwrap();
    let steps = page.items[first..cpus].iter().filter(|i| i.selectable).count();
    for _ in 0..steps {
        h.key(KeyCode::Down);
    }
    h.key(KeyCode::Right);
    let last = h.calls("views.subscribe").pop().unwrap();
    assert_eq!(last["pages"][3], json!({"page": "newVm", "params": {"image": "img", "values": {"diskGB": 74}}}));
    // Settings store at once
    let mut h = Harness::new(none);
    h.show("settings", json!({}), fixture("settings"));
    let page = h.app.page();
    let at = page.items.iter().position(|i| i.key == "general:field:developerMode").unwrap();
    let first = h.app.selected_index(&page).unwrap();
    for _ in 0..page.items[first..at].iter().filter(|i| i.selectable).count() {
        h.key(KeyCode::Down);
    }
    h.key(KeyCode::Enter);
    assert_eq!(h.calls("settings.set"), [json!({"developerMode": false})]);
    // Esc leaves the page
    h.key(KeyCode::Esc);
    assert!(h.app.stack.is_empty());
}

#[test]
fn passwords_are_hidden_and_asked_twice() {
    let mut h = Harness::new(none);
    h.app.run_action(&json!({"id": "create", "label": {"text": "Create Base"}, "method": "macos.installBase",
        "enabled": true, "params": {"user": "alex"},
        "ask": [{"kind": "password", "label": {"text": "Password"}, "param": "password", "repeat": true}]}));
    h.type_text("secret");
    assert!(!h.screen().contains("secret"));
    h.key(KeyCode::Enter);
    h.type_text("other");
    h.key(KeyCode::Enter);
    let Some(Modal::Prompt(p)) = &h.app.modal else { panic!() };
    assert!(p.error.is_some());
    h.type_text("secret");
    h.key(KeyCode::Enter);
    h.type_text("secret");
    h.key(KeyCode::Enter);
    assert_eq!(h.calls("macos.installBase"), [json!({"user": "alex", "password": "secret"})]);
}

#[test]
fn help_lists_the_rows_actions_and_activity_reaches_the_status_line() {
    let mut h = Harness::new(none);
    h.model("vms", json!({}), fixture("vms"));
    h.key(KeyCode::Char('?'));
    let s = h.screen();
    assert!(s.contains("This row") && s.contains("Open Terminal") && s.contains("Shut the VM down first"), "{s}");
    h.key(KeyCode::Esc);
    let mut act = fixture("activity");
    h.model("activity", json!({}), act.clone());
    assert!(h.app.status.is_none());
    act["entries"].as_array_mut().unwrap().insert(
        0,
        json!({"id": 3, "level": "info", "time": "2026-10-23T09:43:00Z", "text": {"text": "Image dev ready"},
               "notification": {"title": {"text": "Image ready"}, "body": {"text": "dev can be used now."}}}),
    );
    h.model("activity", json!({}), act);
    assert_eq!(h.app.status.as_ref().unwrap().0, "Image ready: dev can be used now.");
    // A new server: its ids start at 1 again – lower than the mark, which is forgotten: its entry is unseen
    h.app.status = None;
    h.model(
        "activity",
        json!({}),
        json!({"title": {"text": "Activity"}, "entries": [
        {"id": 1, "level": "info", "time": "2026-10-23T10:00:00Z", "text": {"text": "Service started again"}}]}),
    );
    assert_eq!(h.app.status.as_ref().unwrap().0, "Service started again");
    // Cleared: the mark stays, nothing new
    h.app.status = None;
    h.model("activity", json!({}), json!({"title": {"text": "Activity"}, "entries": []}));
    h.model(
        "activity",
        json!({}),
        json!({"title": {"text": "Activity"}, "entries": [
        {"id": 1, "level": "info", "time": "2026-10-23T10:00:00Z", "text": {"text": "Service started again"}}]}),
    );
    assert!(h.app.status.is_none());
}

#[test]
fn ui_verbs_open_reveal_and_show_the_guide() {
    let mut h = Harness::new(none);
    let verb =
        |m: &str, p: Value| json!({"id": "x", "label": {"text": "x"}, "method": m, "enabled": true, "params": p});
    h.app.run_action(&verb("ui.open", json!({"path": "/data/logs/j1.log"})));
    h.app.run_action(&verb("ui.reveal", json!({"path": "/data/share/research"})));
    // The Doctor's guide action as the model has it: its `url` (no bundled `path` in the tests)
    let doctor = fixture("doctor");
    let guide = &doctor["sections"][1]["items"][0]["actions"][0];
    assert_eq!(guide["params"]["page"], "guide");
    h.app.run_action(guide);
    h.app.run_action(&verb("ui.terminal", json!({"line": "echo hi"})));
    assert_eq!(
        h.app.effects.drain(..).collect::<Vec<_>>(),
        [
            Effect::Open("/data/logs/j1.log".into()),
            Effect::Reveal("/data/share/research".into()),
            Effect::Open("https://omaflake.dev/linux-builder/".into()),
            Effect::Terminal(Command::Shell("echo hi".into())),
        ]
    );
}

#[test]
fn linux_home_and_an_import_refused_with_a_text_reason() {
    let mut h = Harness::new(|m, _| match m {
        "views.get" => {
            let mut import = fixture("import");
            import["valid"] = json!(false);
            import["reason"] = json!({"text": "Not an image package (.omaflake-image)"});
            import
        }
        _ => Value::Null,
    });
    h.model("home", json!({}), fixture("home.linux"));
    h.model("vms", json!({}), fixture("vms"));
    let s = h.screen();
    assert!(s.contains(" omaflake ") && s.contains("S Settings") && s.contains("D Doctor"), "{s}");
    h.app.run_action(&json!({"id": "x", "label": {"text": "x"}, "method": "ui.step", "enabled": true,
                             "params": {"page": "import", "params": {"path": "/x"}}}));
    h.settle();
    assert_eq!(h.app.status.as_ref().unwrap().0, "Not an image package (.omaflake-image)");
}
