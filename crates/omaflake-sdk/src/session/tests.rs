// SPDX-License-Identifier: AGPL-3.0-or-later
//! The session against a core that is played by hand: requests are taken from the effects and answered when and in
//! the order a test wants.

use serde_json::{Value, json};

use super::*;

/// A session with the requests it sent that are not answered yet
#[derive(Default)]
struct Play {
    session: Session,
    sent: Vec<Request>,
    /// Everything else it asked for, in order
    effects: Vec<Effect>,
}

impl Play {
    /// Connected, with the jobs' events subscribed
    fn connected() -> Play {
        let mut p = Play::default();
        let fx = p.session.connected(&json!({"apiVersion": 1}));
        p.take(fx);
        p.answer("events.subscribe", Value::Null);
        p
    }

    fn take(&mut self, effects: Vec<Effect>) {
        for e in effects {
            match e {
                Effect::Send(r) => self.sent.push(r),
                other => self.effects.push(other),
            }
        }
    }

    fn show(&mut self, pages: &[(&str, Value)]) {
        let fx = self.session.show(pages.iter().map(|(p, params)| PageRef::new(p, params.clone())).collect());
        self.take(fx);
    }

    fn event(&mut self, method: &str, params: Value) {
        let fx = self.session.event(method, &params);
        self.take(fx);
    }

    fn model(&mut self, page: &str, params: Value, model: Value) {
        self.event("view.changed", json!({"page": page, "params": params, "model": model}));
    }

    /// The methods sent and not answered, in the order sent
    fn waiting(&self) -> Vec<&str> {
        self.sent.iter().map(|r| r.method.as_str()).collect()
    }

    /// Takes the oldest unanswered request of `method`
    fn request(&mut self, method: &str) -> Request {
        let at = self.sent.iter().position(|r| r.method == method).unwrap_or_else(|| {
            panic!("no {method} waiting: {:?}", self.waiting());
        });
        self.sent.remove(at)
    }

    /// Answers the oldest unanswered request of `method`; its params
    fn answer(&mut self, method: &str, result: Value) -> Value {
        let r = self.request(method);
        let fx = self.session.answer(r.id, Ok(result));
        self.take(fx);
        r.params
    }

    fn fail(&mut self, method: &str, message: &str) {
        let r = self.request(method);
        let fx = self.session.answer(r.id, Err(RpcError::server(None, message)));
        self.take(fx);
    }

    fn start(&mut self, action: Value) {
        let fx = self.session.start(&action);
        self.take(fx);
    }

    fn flow(&self) -> FlowId {
        self.session.question().expect("a question is open").flow()
    }

    fn reply(&mut self, flow: FlowId, values: &[&str]) {
        let fx = self.session.reply(flow, values.iter().map(|v| v.to_string()).collect());
        self.take(fx);
    }

    fn drain(&mut self) -> Vec<Effect> {
        std::mem::take(&mut self.effects)
    }

    fn notices(&mut self) -> Vec<Notice> {
        self.drain()
            .into_iter()
            .filter_map(|e| match e {
                Effect::Notice(n) => Some(n),
                _ => None,
            })
            .collect()
    }
}

fn pages_of(params: &Value) -> Vec<String> {
    params["pages"].as_array().unwrap().iter().map(|p| format!("{} {}", p["page"], p["params"])).collect()
}

fn action(method: &str, params: Value) -> Value {
    json!({"id": "a", "label": {"text": "A"}, "method": method, "params": params, "enabled": true})
}

fn asking(method: &str, check: bool) -> Value {
    let mut a = action(method, json!({"expect": "vm"}));
    a["ask"] = json!([{"kind": "package", "label": {"text": "Package"}, "param": "path"}]);
    if check {
        a["ask"][0]["check"] = json!({"method": "packages.inspect", "params": {"expect": "vm"}, "param": "path"});
    }
    a
}

fn build_model(job: &str, from: usize, lines: usize) -> Value {
    json!({"title": {"text": "Build Image"}, "log": {"job": job, "from": from, "lines": lines}})
}

fn job(id: &str, state: &str) -> Value {
    json!({"job": {"id": id, "kind": "build", "title": format!("Build {id}"), "state": state, "cancellable": true,
                   "logLines": 0, "hiddenLines": 0}})
}

// MARK: Pages and their order

#[test]
fn connecting_subscribes_the_jobs_and_then_the_pages() {
    let mut p = Play::default();
    p.show(&[("home", json!({})), ("vms", Value::Null)]);
    assert!(p.sent.is_empty(), "nothing is sent without a connection");
    let fx = p.session.connected(&json!({"apiVersion": 1}));
    p.take(fx);
    // One at a time: the pages follow the answer
    assert_eq!(p.waiting(), ["events.subscribe"]);
    assert_eq!(p.answer("events.subscribe", Value::Null), json!({"topics": ["jobs"]}));
    assert_eq!(p.waiting(), ["views.subscribe"]);
    let pages = p.answer("views.subscribe", Value::Null);
    assert_eq!(pages, json!({"pages": [{"page": "home", "params": {}}, {"page": "vms", "params": {}}]}));
    // The same pages again: nothing to send
    p.show(&[("home", json!({})), ("vms", json!({}))]);
    assert!(p.sent.is_empty());
    assert!(p.session.is_connected() && p.session.init()["apiVersion"] == 1);
}

#[test]
fn page_sets_cannot_overtake_each_other() {
    let mut p = Play::connected();
    p.show(&[("vms", json!({}))]);
    // Two quick tab switches while the first subscription is on its way
    p.show(&[("images", json!({}))]);
    p.show(&[("activity", json!({}))]);
    assert_eq!(p.waiting(), ["views.subscribe"], "one subscription at a time");
    assert_eq!(pages_of(&p.answer("views.subscribe", Value::Null)), ["\"vms\" {}"]);
    // Only the last one wanted follows – in between nothing was sent that could arrive late
    assert_eq!(p.waiting(), ["views.subscribe"]);
    assert_eq!(pages_of(&p.answer("views.subscribe", Value::Null)), ["\"activity\" {}"]);
    assert!(p.sent.is_empty());
    // There and back while one is on its way: what the core will have is what is wanted
    p.show(&[("vms", json!({}))]);
    p.show(&[("images", json!({}))]);
    p.show(&[("vms", json!({}))]);
    p.answer("views.subscribe", Value::Null);
    assert!(p.sent.is_empty(), "{:?}", p.waiting());
}

#[test]
fn a_failed_subscription_is_told_and_tried_with_the_next_show() {
    let mut p = Play::connected();
    p.show(&[("nope", json!({}))]);
    p.fail("views.subscribe", "Unknown page");
    assert!(matches!(&p.notices()[..], [Notice::Failed { method, .. }] if method == "views.subscribe"));
    assert!(p.sent.is_empty(), "not sent again by itself");
    p.show(&[("vms", json!({}))]);
    assert_eq!(pages_of(&p.answer("views.subscribe", Value::Null)), ["\"vms\" {}"]);
}

#[test]
fn values_are_set_one_after_the_other() {
    let mut p = Play::connected();
    for n in 1..=3 {
        let fx = p.session.set("settings.set", json!({"defaultCpus": n}));
        p.take(fx);
    }
    for n in 1..=3 {
        assert_eq!(p.waiting(), ["settings.set"]);
        assert_eq!(p.answer("settings.set", Value::Null), json!({"defaultCpus": n}));
    }
    // A call of the interface's own does not wait for them
    let fx = p.session.set("settings.set", json!({"x": 1}));
    p.take(fx);
    let fx = p.session.call("settings.get", Value::Null);
    p.take(fx);
    assert_eq!(p.waiting(), ["settings.set", "settings.get"]);
    p.answer("settings.get", json!({"settings": {}}));
    assert_eq!(p.drain(), [Effect::Answered { method: "settings.get".into(), result: json!({"settings": {}}) }]);
}

#[test]
fn models_are_kept_while_wanted_and_the_newest_per_page() {
    let mut p = Play::connected();
    p.show(&[("home", json!({})), ("newVm", json!({"image": "i"}))]);
    p.model("home", json!({}), json!({"title": {"text": "omaflake"}}));
    p.model("newVm", json!({"image": "i"}), json!({"n": 0}));
    assert_eq!(p.drain().len(), 2);
    // A form: every keystroke is another page (its values are params)
    for n in 1..=50 {
        let params = json!({"image": "i", "values": {"name": "x".repeat(n)}});
        p.show(&[("home", json!({})), ("newVm", params.clone())]);
        // Until its model comes, the one before is shown
        let shown = p.session.model_or_latest(&PageRef::new("newVm", params.clone())).unwrap();
        assert_eq!(shown["n"], n - 1);
        p.model("newVm", params.clone(), json!({"n": n}));
        assert_eq!(p.session.model(&PageRef::new("newVm", params)).unwrap()["n"], n);
    }
    assert_eq!(p.session.held().count(), 2, "home and the newest form");
    // A page left: its newest model stays for the way back, older ones of its name do not come back
    p.show(&[("home", json!({}))]);
    assert_eq!(p.session.held().count(), 2);
    assert_eq!(p.session.latest("newVm").unwrap().params["values"]["name"], "x".repeat(50));
    assert!(!p.session.holds(&PageRef::new("newVm", json!({"image": "i"}))));
}

// MARK: A new connection

#[test]
fn a_new_connection_forgets_jobs_logs_and_requests() {
    let mut p = Play::connected();
    p.show(&[("build", json!({"recipe": "r"}))]);
    p.answer("views.subscribe", Value::Null);
    p.event("jobs.changed", job("j1", "running"));
    p.model("build", json!({"recipe": "r"}), build_model("j1", 0, 2));
    p.answer("jobs.log", json!({"lines": ["old 1", "old 2"], "next": 2}));
    assert_eq!(p.session.log("j1").unwrap(), ["old 1", "old 2"]);
    assert_eq!(p.session.jobs().count(), 1);
    // An action whose answer is still out, and a check that runs
    p.start(action("vms.start", json!({"name": "a"})));
    p.start(asking("packages.import", true));
    let flow = p.flow();
    p.reply(flow, &["/x"]);
    assert!(matches!(p.session.question(), Some(Question::Ask { checking: true, .. })));
    let (old_start, old_check) = (p.request("vms.start"), p.request("packages.inspect"));
    p.drain();

    // The service ends and a new one starts: its job ids begin at j1 again
    let fx = p.session.disconnected();
    p.take(fx);
    assert_eq!(p.session.jobs().count(), 0);
    assert!(p.drain().contains(&Effect::Jobs));
    assert!(matches!(p.session.question(), Some(Question::Ask { checking: false, .. })), "no check runs any more");
    let fx = p.session.connected(&json!({"apiVersion": 1}));
    p.take(fx);
    assert!(p.session.log("j1").is_none(), "the old service's log is gone");
    // Answers of the old connection change nothing
    let fx = p.session.answer(old_start.id, Ok(json!({})));
    p.take(fx);
    let fx = p.session.answer(old_check.id, Ok(json!({"valid": true})));
    p.take(fx);
    assert!(p.drain().is_empty());
    assert!(matches!(p.session.question(), Some(Question::Ask { checking: false, .. })));
    // Jobs' events and the pages again
    p.answer("events.subscribe", Value::Null);
    assert_eq!(pages_of(&p.answer("views.subscribe", Value::Null)), ["\"build\" {\"recipe\":\"r\"}"]);
    // The new service's j1 is another job: its log is read from its start
    p.model("build", json!({"recipe": "r"}), build_model("j1", 0, 1));
    assert_eq!(p.answer("jobs.log", json!({"lines": ["new 1"], "next": 1})), json!({"id": "j1", "from": 0}));
    assert_eq!(p.session.log("j1").unwrap(), ["new 1"]);
    // The question survived; its answer is checked on the new connection
    p.reply(flow, &["/x"]);
    p.answer("packages.inspect", json!({"valid": true}));
    assert_eq!(p.answer("packages.import", Value::Null), json!({"expect": "vm", "path": "/x"}));
}

#[test]
fn a_stale_model_of_the_old_connection_reads_no_log() {
    let mut p = Play::connected();
    p.show(&[("build", json!({"recipe": "r"}))]);
    p.answer("views.subscribe", Value::Null);
    p.model("build", json!({"recipe": "r"}), build_model("j3", 0, 0));
    p.answer("jobs.log", json!({"lines": [], "next": 0}));
    let fx = p.session.disconnected();
    p.take(fx);
    let fx = p.session.connected(&Value::Null);
    p.take(fx);
    // Its page is still shown, with the model of before – which names a job of the service that is gone
    assert!(p.session.holds(&PageRef::new("build", json!({"recipe": "r"}))));
    p.event("job.log", json!({"id": "j3", "lines": ["of another job"]}));
    assert_eq!(p.waiting(), ["events.subscribe"]);
}

#[test]
fn without_a_connection_requests_are_told_not_sent() {
    let mut p = Play::default();
    p.start(action("vms.start", json!({"name": "a"})));
    let fx = p.session.set("settings.set", json!({}));
    p.take(fx);
    assert!(p.sent.is_empty());
    assert_eq!(
        p.notices(),
        [Notice::NotConnected { method: "vms.start".into() }, Notice::NotConnected { method: "settings.set".into() }]
    );
}

// MARK: Actions

#[test]
fn an_action_asks_confirms_and_is_sent() {
    let mut p = Play::connected();
    let mut a = asking("packages.export", false);
    a["confirm"] = json!({"title": {"text": "Export?"}, "text": {"text": "…"}, "button": {"text": "Export"},
                          "destructive": false, "input": {"param": "name", "default": "x"}});
    a["then"] = json!("reveal");
    p.start(a.clone());
    assert_eq!(p.drain(), [Effect::Question]);
    let Some(Question::Ask { flow, ask, index: 0, checking: false, .. }) = p.session.question() else { panic!() };
    assert_eq!(ask["param"], "path");
    p.reply(flow, &["/exports"]);
    let Some(Question::Confirm { params, confirm, .. }) = p.session.question() else { panic!("no confirmation") };
    assert_eq!((params, &confirm["button"]["text"]), (json!({"expect": "vm", "path": "/exports"}), &json!("Export")));
    assert!(p.sent.is_empty(), "nothing is sent before the yes");
    let fx = p.session.confirm(flow, Some("y".into()));
    p.take(fx);
    assert!(p.session.question().is_none());
    assert_eq!(
        p.answer("packages.export", json!({"path": "/exports/y"})),
        json!({"expect": "vm", "path": "/exports", "name": "y"})
    );
    assert_eq!(
        p.drain(),
        [Effect::Question, Effect::Question, Effect::Done { action: a, result: json!({"path": "/exports/y"}) }]
    );
}

#[test]
fn what_cannot_be_done_or_fails_is_told() {
    let mut p = Play::connected();
    let mut a = action("vms.reset", json!({}));
    a["enabled"] = json!(false);
    a["reason"] = json!({"text": "Shut the VM down first"});
    p.start(a);
    assert_eq!(p.notices(), [Notice::Disabled { reason: Some(json!({"text": "Shut the VM down first"})) }]);
    p.start(action("vms.start", json!({"name": "a"})));
    p.fail("vms.start", "No such VM");
    assert_eq!(
        p.notices(),
        [Notice::Failed { method: "vms.start".into(), error: RpcError::server(None, "No such VM") }]
    );
    // The interface's own verbs come back to it
    p.start(action("ui.show", json!({"page": "doctor", "params": {}})));
    assert_eq!(p.drain(), [Effect::Ui { verb: "show".into(), params: json!({"page": "doctor", "params": {}}) }]);
}

#[test]
fn a_cancelled_action_never_goes_on() {
    let mut p = Play::connected();
    p.start(asking("packages.import", true));
    let flow = p.flow();
    p.reply(flow, &["/a.omaflake-vm"]);
    assert!(matches!(p.session.question(), Some(Question::Ask { checking: true, .. })));
    // A second answer while the check runs does nothing
    p.reply(flow, &["/b.omaflake-vm"]);
    assert_eq!(p.waiting(), ["packages.inspect"]);
    // Esc – and then the check says yes
    let fx = p.session.cancel(flow);
    p.take(fx);
    assert!(p.session.question().is_none());
    p.answer("packages.inspect", json!({"valid": true}));
    assert!(p.sent.is_empty(), "the cancelled action was sent: {:?}", p.waiting());
    // Nor does anything meant for it reach the next question
    p.start(asking("packages.import", true));
    let next = p.flow();
    assert_ne!(flow, next);
    p.reply(flow, &["/a.omaflake-vm"]);
    let fx = p.session.confirm(flow, None);
    p.take(fx);
    assert!(p.sent.is_empty());
    assert!(matches!(p.session.question(), Some(Question::Ask { checking: false, .. })));
}

#[test]
fn a_check_answers_only_the_value_it_was_asked_for() {
    let mut p = Play::connected();
    p.start(asking("packages.import", true));
    let flow = p.flow();
    p.reply(flow, &["/first"]);
    let first = p.request("packages.inspect");
    // The user types on: the check of the old value no longer counts
    let fx = p.session.retract(flow);
    p.take(fx);
    assert!(matches!(p.session.question(), Some(Question::Ask { checking: false, .. })));
    p.reply(flow, &["/second"]);
    let fx = p.session.answer(first.id, Ok(json!({"valid": true})));
    p.take(fx);
    assert!(matches!(p.session.question(), Some(Question::Ask { checking: true, .. })), "the old verdict was taken");
    // Refused: asked again, with the reason
    p.answer("packages.inspect", json!({"valid": false, "reason": {"text": "Not a VM"}}));
    let Some(Question::Ask { checking: false, refused, .. }) = p.session.question() else { panic!() };
    assert_eq!(refused, [Refusal { value: "/second".into(), reason: Some(json!({"text": "Not a VM"})), error: None }]);
    // A check that fails refuses too; the next answer clears it
    p.reply(flow, &["/third"]);
    p.fail("packages.inspect", "cannot read");
    let Some(Question::Ask { refused, .. }) = p.session.question() else { panic!() };
    assert_eq!(refused[0].error.as_ref().unwrap().message, "cannot read");
    p.reply(flow, &["/fourth"]);
    assert_eq!(p.answer("packages.inspect", json!({"valid": true})), json!({"expect": "vm", "path": "/fourth"}));
    assert_eq!(p.answer("packages.import", Value::Null), json!({"expect": "vm", "path": "/fourth"}));
    assert!(p.notices().is_empty());
}

#[test]
fn several_values_send_the_action_once_each() {
    let mut p = Play::connected();
    let mut a = asking("ui.step", true);
    a["params"] = json!({"page": "import", "params": {"expect": "vm"}});
    a["ask"][0]["param"] = json!("params.path");
    a["ask"][0]["multiple"] = json!(true);
    p.start(a);
    let flow = p.flow();
    p.reply(flow, &["/a", "/b", "/c"]);
    assert_eq!(p.waiting(), ["packages.inspect"; 3]);
    // Answered in another order; /b is refused
    let (a, b, c) = (p.request("packages.inspect"), p.request("packages.inspect"), p.request("packages.inspect"));
    for (r, valid) in [(c, true), (a, true), (b, false)] {
        let fx = p.session.answer(r.id, Ok(json!({"valid": valid})));
        p.take(fx);
    }
    assert!(matches!(&p.notices()[..], [Notice::Skipped { refused }] if refused[0].value == "/b"));
    // The step of each: its page, whose action runs – one question at a time
    assert_eq!(p.waiting(), ["views.get", "views.get"]);
    let ask_name = |name: &str| {
        let mut import = action("packages.import", json!({"path": name}));
        import["confirm"] = json!({"title": {"text": "Import?"}, "text": {"text": ""}, "button": {"text": "Import"},
                                   "destructive": false});
        json!({"name": name, "valid": true, "action": import})
    };
    assert_eq!(
        p.answer("views.get", ask_name("/a")),
        json!({"page": "import", "params": {"expect": "vm", "path": "/a"}})
    );
    p.answer("views.get", ask_name("/c"));
    let Some(Question::Confirm { flow: first, params, .. }) = p.session.question() else { panic!() };
    assert_eq!(params, json!({"path": "/a"}));
    let fx = p.session.confirm(first, None);
    p.take(fx);
    assert_eq!(p.waiting(), ["packages.import"]);
    // The second one's question follows; cancelled, nothing of it is sent
    let Some(Question::Confirm { flow: second, params, .. }) = p.session.question() else { panic!() };
    assert_eq!(params, json!({"path": "/c"}));
    let fx = p.session.cancel(second);
    p.take(fx);
    assert!(p.session.question().is_none());
    assert_eq!(p.waiting(), ["packages.import"]);
    // A step its page refuses says why
    p.start(action("ui.step", json!({"page": "import", "params": {"path": "/x"}})));
    p.answer("views.get", json!({"name": "x", "valid": false, "reason": {"text": "Not a package"}}));
    assert_eq!(p.notices(), [Notice::Refused { reason: Some(json!({"text": "Not a package"})), name: "x".into() }]);
}

#[test]
fn an_action_without_questions_leaves_the_open_one_alone() {
    let mut p = Play::connected();
    p.start(asking("packages.import", false));
    let flow = p.flow();
    // (a tab that looks into its catalogs when it opens)
    p.start(action("recipes.refresh", json!({})));
    assert_eq!(p.waiting(), ["recipes.refresh"]);
    assert_eq!(p.flow(), flow);
    // One with a question takes its place; the old one is cancelled
    p.start(asking("packages.export", false));
    assert_ne!(p.flow(), flow);
    p.reply(flow, &["/old"]);
    assert_eq!(p.waiting(), ["recipes.refresh"]);
    // No value chosen (a file dialog closed): cancelled
    let flow = p.flow();
    p.reply(flow, &[]);
    assert!(p.session.question().is_none());
}

// MARK: Jobs and logs

#[test]
fn log_lines_come_once_and_none_is_lost() {
    let mut p = Play::connected();
    p.show(&[("build", json!({"recipe": "r"}))]);
    p.answer("views.subscribe", Value::Null);
    // The model shows a log the core has from line 40 on (older ones are gone)
    p.model("build", json!({"recipe": "r"}), build_model("j1", 40, 42));
    assert!(p.session.log("j1").is_none(), "not read yet");
    // Lines that come as events while the log is being read are in its answer already – or not
    p.event("job.log", json!({"id": "j1", "lines": ["line 42"]}));
    p.event("job.log", json!({"id": "j1", "lines": ["line 43"]}));
    assert_eq!(p.waiting(), ["jobs.log"], "one request at a time");
    assert_eq!(
        p.answer("jobs.log", json!({"lines": ["line 40", "line 41", "line 42"], "next": 43})),
        json!({"id": "j1", "from": 40})
    );
    assert_eq!(p.session.log("j1").unwrap(), ["line 40", "line 41", "line 42"]);
    assert!(p.drain().contains(&Effect::Log("j1".into())));
    // More was announced meanwhile: read on from `next`
    assert_eq!(p.answer("jobs.log", json!({"lines": ["line 43"], "next": 44})), json!({"id": "j1", "from": 43}));
    assert_eq!(p.session.log("j1").unwrap(), ["line 40", "line 41", "line 42", "line 43"]);
    assert!(p.sent.is_empty());
    // An event: its lines are read, not taken from it
    p.event("job.log", json!({"id": "j1", "lines": ["line 44", "line 45"]}));
    assert_eq!(p.session.log("j1").unwrap().len(), 4);
    assert_eq!(
        p.answer("jobs.log", json!({"lines": ["line 44", "line 45"], "next": 46})),
        json!({"id": "j1", "from": 44})
    );
    // The model's line count says there is more, too (a client without the jobs' events)
    p.model("build", json!({"recipe": "r"}), build_model("j1", 40, 46));
    assert!(p.sent.is_empty(), "nothing new: {:?}", p.waiting());
    p.model("build", json!({"recipe": "r"}), build_model("j1", 40, 47));
    // A core that answers from further back than asked: what is here is not taken again
    assert_eq!(
        p.answer("jobs.log", json!({"lines": ["line 45", "line 46"], "next": 47})),
        json!({"id": "j1", "from": 46})
    );
    let log = p.session.log("j1").unwrap();
    assert_eq!(log.len(), 7);
    assert_eq!(log.last().unwrap(), "line 46");
    // Events of a job no page shows are not the session's
    p.event("job.log", json!({"id": "j9", "lines": ["x"]}));
    assert!(p.sent.is_empty());
}

#[test]
fn a_log_keeps_the_newest_lines_and_is_read_on_when_shown_again() {
    let mut p = Play::connected();
    let build = ("build", json!({"recipe": "r"}));
    p.show(std::slice::from_ref(&build));
    p.answer("views.subscribe", Value::Null);
    p.model("build", json!({"recipe": "r"}), build_model("j1", 0, LOG_LINES));
    let lines: Vec<String> = (0..LOG_LINES).map(|n| format!("line {n}")).collect();
    p.answer("jobs.log", json!({"lines": lines, "next": LOG_LINES}));
    // The page is left: events of its job are noted, not read
    p.show(&[("vms", json!({}))]);
    p.answer("views.subscribe", Value::Null);
    p.event("job.log", json!({"id": "j1", "lines": ["more"]}));
    assert!(p.sent.is_empty());
    // Back on it: read on from where it was
    p.show(&[build]);
    p.answer("views.subscribe", Value::Null);
    p.model("build", json!({"recipe": "r"}), build_model("j1", 0, LOG_LINES));
    let from = p.answer("jobs.log", json!({"lines": ["more", "and more"], "next": LOG_LINES + 2}));
    assert_eq!(from, json!({"id": "j1", "from": LOG_LINES}));
    let log = p.session.log("j1").unwrap();
    assert_eq!((log.len(), log[0].as_str(), log.last().unwrap().as_str()), (LOG_LINES, "line 2", "and more"));
    // A log that cannot be read is told once, not asked at every model
    p.model("build", json!({"recipe": "r"}), build_model("j1", 0, LOG_LINES + 3));
    p.fail("jobs.log", "No job j1");
    assert_eq!(p.notices().len(), 1);
    p.model("build", json!({"recipe": "r"}), build_model("j1", 0, LOG_LINES + 4));
    assert!(p.sent.is_empty());
}

#[test]
fn logs_go_with_the_pages_that_show_them_and_jobs_when_they_end() {
    let mut p = Play::connected();
    p.show(&[("build", json!({"recipe": "a"}))]);
    p.answer("views.subscribe", Value::Null);
    p.model("build", json!({"recipe": "a"}), build_model("j1", 0, 1));
    p.answer("jobs.log", json!({"lines": ["a"], "next": 1}));
    // Another recipe's build page: the first one's model is no longer the newest of its name – its log goes with it
    p.show(&[("build", json!({"recipe": "b"}))]);
    p.answer("views.subscribe", Value::Null);
    assert!(p.session.log("j1").is_some(), "kept until the new page's model is there");
    p.model("build", json!({"recipe": "b"}), build_model("j2", 0, 1));
    assert!(p.session.log("j1").is_none());
    p.answer("jobs.log", json!({"lines": ["b"], "next": 1}));
    assert_eq!(p.session.log("j2").unwrap(), ["b"]);
    p.drain();
    // Jobs: those that wait or run
    p.event("jobs.changed", job("j1", "queued"));
    p.event("jobs.changed", job("j2", "running"));
    p.event("jobs.changed", job("j1", "running"));
    assert_eq!(p.session.jobs().map(|j| j.id.as_str()).collect::<Vec<_>>(), ["j1", "j2"]);
    p.event("jobs.changed", job("j1", "finished"));
    p.event("jobs.changed", job("j2", "failed"));
    // (one that ended before this session knew it changes nothing)
    p.event("jobs.changed", job("j0", "cancelled"));
    assert_eq!(p.session.jobs().count(), 0);
    assert_eq!(p.drain(), [Effect::Jobs, Effect::Jobs, Effect::Jobs, Effect::Jobs, Effect::Jobs]);
}

// MARK: Activity and filters

#[test]
fn activity_tells_what_was_not_seen() {
    let mut p = Play::connected();
    let entry = |id: u64| json!({"id": id, "level": "info", "text": {"text": format!("entry {id}")}});
    let list = |ids: &[u64]| json!({"entries": ids.iter().map(|i| entry(*i)).collect::<Vec<_>>()});
    let news = |p: &mut Play| {
        p.drain().into_iter().find_map(|e| match e {
            Effect::Activity(n) => Some(n),
            _ => None,
        })
    };
    // The first list is no news
    p.model("activity", json!({}), list(&[2, 1]));
    assert_eq!(news(&mut p), Some(News { fresh: vec![], first: true, restarted: false, newest: Some(2) }));
    p.model("activity", json!({}), list(&[2, 1]));
    assert_eq!(news(&mut p), None);
    p.model("activity", json!({}), list(&[4, 3, 2, 1]));
    let n = news(&mut p).unwrap();
    assert_eq!((n.fresh, n.first, n.restarted, n.newest), (vec![entry(4), entry(3)], false, false, Some(4)));
    // Cleared: the mark stays
    p.model("activity", json!({}), list(&[]));
    p.model("activity", json!({}), list(&[4]));
    assert_eq!(news(&mut p), None);
    // A new server counts from 1 again
    p.model("activity", json!({}), list(&[1]));
    let n = news(&mut p).unwrap();
    assert_eq!((n.fresh, n.restarted), (vec![entry(1)], true));
}

#[test]
fn filters_by_page() {
    let mut f = Filters::default();
    f.set_group("recipes", "page", json!(3));
    f.set_group("recipes", "kind", json!("nixos"));
    assert_eq!(f.get("recipes"), Some(&json!({"page": 3, "kind": "nixos"})));
    // A new search starts on the first page
    f.set_group("recipes", "search", json!("dev"));
    assert_eq!(f.get("recipes"), Some(&json!({"kind": "nixos", "search": "dev"})));
    f.set_group("recipes", "search", json!(""));
    f.set_group("recipes", "kind", Value::Null);
    assert_eq!(f.get("recipes"), None);
    f.set("vms", json!({"status": true}));
    assert_eq!(f.get("vms"), Some(&json!({"status": true})));
    f.set("vms", json!({}));
    f.set("vms", Value::Null);
    assert_eq!(f.get("vms"), None);
}
