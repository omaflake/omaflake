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
        let mut term = Terminal::new(TestBackend::new(140, 45)).unwrap();
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
        "dev (velvet) · 4 CPU · 8 GB · SSH 2201",
        "D Doctor 3",
        "S Settings",
        "H ▤",
        "q Quit omaflake",
        "/ Filter",
        "i ⇩ Import",
        "r ↗ Run from There",
        "1 ▤ Show Share Folder",
        "2 ›_ Open Terminal with SSH",
        "3 ■ Shut down",
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
        "8 ⇧ Export …",
        "Remove …",
    ] {
        assert!(s.contains(want), "missing {want:?} in\n{s}");
    }
}

#[test]
fn draws_every_page() {
    let pages = [
        ("images", "images", vec!["dev (velvet)", "Copying the image", "120/400", "62 %", "cedar-mango-lift"]),
        (
            "recipes",
            "recipes",
            vec!["Newer recipe versions in your sources", "omaflake/nixos/dev", "minimal", "example.org: HTTP 404"],
        ),
        ("activity", "activity", vec!["research is ready", "work: running – stop it first", "c ✕ Clear History"]),
        ("build", "build", vec!["dev (velvet)", "Copying the image", "62 %", "Full Log", "Cancel"]),
        ("build", "build.failed", vec!["Build failed"]),
        ("doctor", "doctor", vec!["omaf ui", "Core", "Up to date", "Nix", "Check Again"]),
        (
            "settings",
            "settings",
            vec!["Recipe Sources", "omaflake/recipes", "General", "[x] Check for updates", "Data Folder"],
        ),
        ("newVm", "newVm", vec!["New VM", "CPU cores: 4", "‹ 4 ›", "Create", "Cancel"]),
        ("macBase", "macBase", vec!["macOS Base", "Create Base", "differs from macUser"]),
        ("vm", "vm", vec!["research", "Runs from", "Remove …"]),
        ("image", "image", vec!["stone-lamp", "Remove …"]),
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
    assert_eq!(h.calls("jobs.log"), [json!({"id": "j1"})]);
    h.app.handle(Msg::Event("job.log".into(), json!({"id": "j1", "lines": ["[3/5] Copying the image"]})));
    let s = h.screen();
    assert!(s.contains("building '/nix/store/abc'") && s.contains("[3/5] Copying the image"), "{s}");
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
    // 1 = show the share folder: vms.share, then reveal its path
    h.key(KeyCode::Char('1'));
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
        "packages.inspect" => json!({"kind": "image", "valid": false, "reason": "Not a VM"}),
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
    let cpus = page.items.iter().position(|i| i.key == "newVm:field:cpus").unwrap();
    let first = h.app.selected_index(&page).unwrap();
    let steps = page.items[first..cpus].iter().filter(|i| i.selectable).count();
    for _ in 0..steps {
        h.key(KeyCode::Down);
    }
    h.key(KeyCode::Right);
    let last = h.calls("views.subscribe").pop().unwrap();
    assert_eq!(last["pages"][3], json!({"page": "newVm", "params": {"image": "img", "values": {"cpus": 5}}}));
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
}

#[test]
fn ui_verbs_open_reveal_and_show_the_guide() {
    let mut h = Harness::new(none);
    let verb =
        |m: &str, p: Value| json!({"id": "x", "label": {"text": "x"}, "method": m, "enabled": true, "params": p});
    h.app.run_action(&verb("ui.open", json!({"path": "/data/logs/j1.log"})));
    h.app.run_action(&verb("ui.reveal", json!({"path": "/data/share/research"})));
    h.app.run_action(&verb("ui.show", json!({"page": "guide", "params": {"file": "linux-builder"}})));
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
