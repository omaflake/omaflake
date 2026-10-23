// SPDX-License-Identifier: AGPL-3.0-or-later
//! Events (docs/api.md §6): the topics a client subscribed to and the one way notifications leave the server.

use std::collections::HashSet;
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

/// The subscription and the sink. Without a subscription nothing is sent.
pub(crate) struct Events {
    sink: Arc<Notify>,
    topics: Mutex<HashSet<Topic>>,
}

impl Events {
    pub fn new(sink: Arc<Notify>) -> Self {
        Events { sink, topics: Mutex::new(HashSet::new()) }
    }

    /// Replaces the subscription; returns the topics that are new in it
    pub fn subscribe(&self, topics: HashSet<Topic>) -> HashSet<Topic> {
        let mut current = self.topics.lock().unwrap();
        let added = topics.difference(&current).copied().collect();
        *current = topics;
        added
    }

    pub fn is_subscribed(&self, topic: Topic) -> bool {
        self.topics.lock().unwrap().contains(&topic)
    }

    pub fn any(&self) -> bool {
        !self.topics.lock().unwrap().is_empty()
    }

    /// Sends `method` with `params` when `topic` is subscribed
    pub fn emit<T: Serialize>(&self, topic: Topic, method: &str, params: &T) {
        if !self.is_subscribed(topic) {
            return;
        }
        match serde_json::to_value(params) {
            Ok(v) => (self.sink)(method, v),
            Err(e) => eprintln!("omaf serve: cannot encode {method}: {e}"),
        }
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
        let events = Events::new(sink);
        events.info("not yet");
        assert!(got.lock().unwrap().is_empty());
        let added = events.subscribe([Topic::Messages, Topic::Jobs].into());
        assert_eq!(added.len(), 2);
        events.info("now");
        events.emit(Topic::Vms, "vms.changed", &serde_json::json!({}));
        let got = got.lock().unwrap();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "message");
        assert_eq!(got[0].1, serde_json::json!({"text": "now", "level": "info"}));
        assert_eq!(events.subscribe([Topic::Jobs, Topic::Vms].into()), [Topic::Vms].into());
        assert_eq!(Topic::parse("doctor"), Some(Topic::Doctor));
        assert_eq!(Topic::parse("nope"), None);
    }
}
