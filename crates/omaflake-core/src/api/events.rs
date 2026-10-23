// SPDX-License-Identifier: AGPL-3.0-or-later
//! Events (docs/api.md §6): the topics each connected client subscribed to and the one way notifications leave the
//! server – to every connection that wants them.

use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;

use super::types::{Level, Message};

/// Receives the server's notifications: (method, params). The stdio transport writes them as JSON-RPC
/// notifications; a Rust UI linking the server in-process gets them directly.
pub type Notify = dyn Fn(&str, Value) + Send + Sync;

/// What a client can subscribe to (`events.subscribe {topics}`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Topic {
    /// `vms.changed`, `vm.ended`, `vm.ready`
    Vms,
    /// `images.changed`
    Images,
    /// `recipes.changed`
    Recipes,
    /// `jobs.changed`, `job.log`
    Jobs,
    /// `doctor.changed`
    Doctor,
    /// `message`
    Messages,
}

impl Topic {
    pub const ALL: [Topic; 6] =
        [Topic::Vms, Topic::Images, Topic::Recipes, Topic::Jobs, Topic::Doctor, Topic::Messages];

    pub fn as_str(self) -> &'static str {
        match self {
            Topic::Vms => "vms",
            Topic::Images => "images",
            Topic::Recipes => "recipes",
            Topic::Jobs => "jobs",
            Topic::Doctor => "doctor",
            Topic::Messages => "messages",
        }
    }

    pub fn parse(s: &str) -> Option<Topic> {
        Self::ALL.into_iter().find(|t| t.as_str() == s)
    }
}

/// One connection's id (`Server::connect`); the first one of a server is 0
pub(crate) type ConnId = u64;

/// A connection: where its notifications go, what it subscribed to, its language (`initialize`)
struct Subscriber {
    sink: Arc<Notify>,
    topics: HashSet<Topic>,
    language: Option<String>,
}

/// The subscriptions and sinks of every connection (B120: one service, several clients). Without a subscription
/// nothing is sent; an event goes to every connection subscribed to its topic.
pub(crate) struct Events {
    subscribers: Mutex<HashMap<ConnId, Subscriber>>,
}

impl Events {
    pub fn new() -> Self {
        Events { subscribers: Mutex::new(HashMap::new()) }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, HashMap<ConnId, Subscriber>> {
        self.subscribers.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// A connection's sink; it gets nothing until it subscribes
    pub fn attach(&self, conn: ConnId, sink: Arc<Notify>) {
        self.lock().insert(conn, Subscriber { sink, topics: HashSet::new(), language: None });
    }

    /// The connection is gone: nothing more goes to it
    pub fn detach(&self, conn: ConnId) {
        self.lock().remove(&conn);
    }

    pub fn set_language(&self, conn: ConnId, language: Option<String>) {
        if let Some(s) = self.lock().get_mut(&conn) {
            s.language = language;
        }
    }

    /// Replaces a connection's subscription; returns the topics that are new in it
    pub fn subscribe(&self, conn: ConnId, topics: HashSet<Topic>) -> HashSet<Topic> {
        let mut subs = self.lock();
        let Some(s) = subs.get_mut(&conn) else { return HashSet::new() };
        let added = topics.difference(&s.topics).copied().collect();
        s.topics = topics;
        added
    }

    /// Someone is subscribed to `topic`
    pub fn is_subscribed(&self, topic: Topic) -> bool {
        self.lock().values().any(|s| s.topics.contains(&topic))
    }

    /// Someone is subscribed to anything
    pub fn any(&self) -> bool {
        self.lock().values().any(|s| !s.topics.is_empty())
    }

    /// The languages of the connections subscribed to `topic` (each once)
    pub fn languages(&self, topic: Topic) -> Vec<Option<String>> {
        let mut out: Vec<Option<String>> = Vec::new();
        for s in self.lock().values().filter(|s| s.topics.contains(&topic)) {
            if !out.contains(&s.language) {
                out.push(s.language.clone());
            }
        }
        out
    }

    /// The sinks of the subscribers of `topic` that `pick` accepts – called outside the lock (a sink may block on
    /// a slow client; the others must not wait for the list)
    fn send<T: Serialize>(&self, topic: Topic, method: &str, params: &T, pick: impl Fn(ConnId, &Subscriber) -> bool) {
        let sinks: Vec<Arc<Notify>> = self
            .lock()
            .iter()
            .filter(|(id, s)| s.topics.contains(&topic) && pick(**id, s))
            .map(|(_, s)| s.sink.clone())
            .collect();
        if sinks.is_empty() {
            return;
        }
        match serde_json::to_value(params) {
            Ok(v) => {
                for sink in sinks {
                    sink(method, v.clone());
                }
            }
            Err(e) => eprintln!("omaf serve: cannot encode {method}: {e}"),
        }
    }

    /// Sends `method` with `params` to every connection subscribed to `topic`
    pub fn emit<T: Serialize>(&self, topic: Topic, method: &str, params: &T) {
        self.send(topic, method, params, |_, _| true)
    }

    /// Sends to one connection only (if it is subscribed to `topic`)
    pub fn emit_to<T: Serialize>(&self, conn: ConnId, topic: Topic, method: &str, params: &T) {
        self.send(topic, method, params, |id, _| id == conn)
    }

    /// Sends to the connections of one language (lists with translated texts)
    pub fn emit_in<T: Serialize>(&self, language: &Option<String>, topic: Topic, method: &str, params: &T) {
        self.send(topic, method, params, |_, s| &s.language == language)
    }

    /// Something the user should read in the activity list
    pub fn message(&self, level: Level, text: impl Into<String>) {
        self.emit(Topic::Messages, "message", &Message { text: text.into(), level });
    }

    pub fn info(&self, text: impl Into<String>) {
        self.message(Level::Info, text)
    }

    pub fn warning(&self, text: impl Into<String>) {
        self.message(Level::Warning, text)
    }

    pub fn error(&self, text: impl Into<String>) {
        self.message(Level::Error, text)
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// What a recording sink got: (method, params)
    pub type Recorded = Arc<Mutex<Vec<(String, Value)>>>;

    /// A sink that keeps what it gets
    pub fn recorder() -> (Arc<Notify>, Recorded) {
        let got: Arc<Mutex<Vec<(String, Value)>>> = Arc::default();
        let g = got.clone();
        let sink: Arc<Notify> = Arc::new(move |m: &str, v: Value| g.lock().unwrap().push((m.to_string(), v)));
        (sink, got)
    }

    #[test]
    fn only_subscribed_topics_are_sent() {
        let (sink, got) = recorder();
        let events = Events::new();
        events.attach(0, sink);
        events.info("not yet");
        assert!(got.lock().unwrap().is_empty());
        let added = events.subscribe(0, [Topic::Messages, Topic::Jobs].into());
        assert_eq!(added.len(), 2);
        events.info("now");
        events.emit(Topic::Vms, "vms.changed", &serde_json::json!({}));
        let got = got.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "message");
        assert_eq!(got[0].1, serde_json::json!({"text": "now", "level": "info"}));
        assert_eq!(events.subscribe(0, [Topic::Jobs, Topic::Vms].into()), [Topic::Vms].into());
        assert_eq!(Topic::parse("doctor"), Some(Topic::Doctor));
        assert_eq!(Topic::parse("nope"), None);
    }

    #[test]
    fn every_subscribed_connection_gets_an_event() {
        let (a, got_a) = recorder();
        let (b, got_b) = recorder();
        let events = Events::new();
        events.attach(1, a);
        events.attach(2, b);
        events.subscribe(1, [Topic::Jobs, Topic::Recipes].into());
        events.subscribe(2, [Topic::Jobs].into());
        events.set_language(1, Some("de".into()));
        events.emit(Topic::Jobs, "jobs.changed", &serde_json::json!({}));
        events.emit_to(2, Topic::Jobs, "job.log", &serde_json::json!({}));
        events.emit_in(&Some("de".into()), Topic::Recipes, "recipes.changed", &serde_json::json!({}));
        assert_eq!(events.languages(Topic::Jobs).len(), 2);
        events.detach(2);
        events.emit(Topic::Jobs, "jobs.changed", &serde_json::json!({}));
        let a: Vec<String> = got_a.lock().unwrap().iter().map(|(m, _)| m.clone()).collect();
        let b: Vec<String> = got_b.lock().unwrap().iter().map(|(m, _)| m.clone()).collect();
        assert_eq!(a, ["jobs.changed", "recipes.changed", "jobs.changed"]);
        assert_eq!(b, ["jobs.changed", "job.log"]);
        assert!(events.is_subscribed(Topic::Recipes) && !events.is_subscribed(Topic::Vms));
    }
}
