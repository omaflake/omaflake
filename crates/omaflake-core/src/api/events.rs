// SPDX-License-Identifier: AGPL-3.0-or-later
//! Events (docs/api.md §6): the topics a client subscribed to and the one way notifications leave the server.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use serde::Serialize;
use serde_json::Value;

use super::types::{Level, Message};
use super::views::activity::{ActivityEntry, ActivityLog, Notification};
use super::views::{PageRef, Text};

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

/// The subscription and the sink. Without a subscription nothing is sent. Also the activity list (it is fed from
/// wherever events come from: jobs, the watcher, methods) and the pages subscribed with `views.subscribe`.
pub(crate) struct Events {
    sink: Arc<Notify>,
    topics: Mutex<HashSet<Topic>>,
    views: Mutex<Vec<PageRef>>,
    activity: Mutex<ActivityLog>,
    /// Wakes the watcher to rebuild the subscribed pages (set by the server)
    on_change: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
}

fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

impl Events {
    pub fn new(sink: Arc<Notify>) -> Self {
        Events {
            sink,
            topics: Mutex::new(HashSet::new()),
            views: Mutex::default(),
            activity: Mutex::default(),
            on_change: Mutex::default(),
        }
    }

    pub fn set_on_change(&self, f: Arc<dyn Fn() + Send + Sync>) {
        *lock(&self.on_change) = Some(f);
    }

    /// Something a page shows changed (a job, the activity): the subscribed pages are rebuilt soon
    pub fn changed(&self) {
        if lock(&self.views).is_empty() {
            return;
        }
        let f = lock(&self.on_change).clone();
        if let Some(f) = f {
            f();
        }
    }

    /// Replaces the pages subscribed with `views.subscribe`
    pub fn subscribe_views(&self, pages: Vec<PageRef>) {
        *lock(&self.views) = pages;
    }

    pub fn views(&self) -> Vec<PageRef> {
        lock(&self.views).clone()
    }

    /// Sends a notification that belongs to no topic (`view.changed` goes to whoever subscribed the page)
    pub fn send(&self, method: &str, params: Value) {
        (self.sink)(method, params)
    }

    /// A line of the activity list
    pub fn record(&self, level: Level, text: Text, notification: Option<Notification>) {
        lock(&self.activity).push(level, text, notification);
        self.changed();
    }

    pub fn activity(&self) -> Vec<ActivityEntry> {
        lock(&self.activity).list()
    }

    pub fn clear_activity(&self) {
        lock(&self.activity).clear();
        self.changed();
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

    /// Anything subscribed: topics or pages (the watcher runs while there is)
    pub fn any(&self) -> bool {
        !self.topics.lock().unwrap().is_empty() || !lock(&self.views).is_empty()
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

    /// Something the user should read in the activity list (`message`, and a line of the activity list)
    pub fn message(&self, level: Level, text: impl Into<String>) {
        self.message_text(level, Text::verbatim(text.into()));
    }

    /// `message` with a text a UI can translate by its key (the event carries the English text)
    pub fn message_text(&self, level: Level, text: Text) {
        self.emit(Topic::Messages, "message", &Message { text: text.text.clone(), level });
        self.record(level, text, None);
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
        events.message(Level::Info, "not yet");
        assert!(got.lock().unwrap().is_empty());
        let added = events.subscribe([Topic::Messages, Topic::Jobs].into());
        assert_eq!(added.len(), 2);
        events.message(Level::Info, "now");
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
