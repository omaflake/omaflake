// SPDX-License-Identifier: AGPL-3.0-or-later
//! The building blocks every page uses (docs/api.md §10.3): actions with their confirmation and what they ask for,
//! status lamps, badges, info lines, hints, filters and form fields.

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};

use super::text::{tf, Text};
use crate::api::types::PrerequisiteDto;

/// What something says at a glance: its colour role (design tokens: normal = secondary label, accent = cyan,
/// success = green, warning = orange, danger = red)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Tone {
    Normal,
    Accent,
    Success,
    Warning,
    Danger,
}

/// The status lamp that opens a row (plan B99): green on/active/built, grey off, orange attention, cyan pulsing working
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LampState {
    On,
    Off,
    Attention,
    Working,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Lamp {
    pub state: LampState,
    pub help: Text,
}

impl Lamp {
    pub fn new(state: LampState, help: &str) -> Lamp {
        Lamp { state, help: Text::new(help) }
    }
}

/// A small symbol (or short label) next to a name – `icon` from the icon vocabulary (api.md §10.3)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Badge {
    pub id: String,
    pub icon: String,
    /// Tooltip / what a text-only UI writes
    pub help: Text,
    pub tone: Tone,
    /// A short label drawn instead of (or next to) the icon ("x86_64", "outdated")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<Text>,
}

impl Badge {
    pub fn new(id: &str, icon: &str, help: Text, tone: Tone) -> Badge {
        Badge { id: id.into(), icon: icon.into(), help, tone, label: None }
    }

    pub fn label(mut self, label: Text) -> Badge {
        self.label = Some(label);
        self
    }
}

/// "Label  value" in a row's details; `full` is the whole value where the UI shortens it (tooltip, selection)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InfoLine {
    pub label: Text,
    pub value: Text,
    /// Said after the value in its tone ("newer image available")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tone: Option<Tone>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub full: Option<String>,
}

impl InfoLine {
    pub fn new(label: &str, value: Text) -> InfoLine {
        InfoLine { label: Text::new(label), value, note: None, tone: None, full: None }
    }

    pub fn full(mut self, full: impl Into<String>) -> InfoLine {
        self.full = Some(full.into());
        self
    }

    pub fn note(mut self, note: Text, tone: Tone) -> InfoLine {
        self.note = Some(note);
        self.tone = Some(tone);
        self
    }

    pub fn tone(mut self, tone: Tone) -> InfoLine {
        self.tone = Some(tone);
        self
    }
}

/// A line of text with a symbol: an empty list, a filter that leaves nothing, a file that cannot be read, a warning
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Hint {
    pub icon: String,
    pub text: Text,
    pub tone: Tone,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    /// Only explains (plan B148): shown while the help switch is on, each in a box of its own. A warning, an error or
    /// what is missing is never help.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub help: bool,
    /// The keys it speaks of, as caps after its text (`keys::caps`: each key's chords, each a list of caps)
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keys: Vec<Vec<Vec<String>>>,
}

impl Hint {
    pub fn new(icon: &str, text: Text, tone: Tone) -> Hint {
        Hint { icon: icon.into(), text, tone, action: None, help: false, keys: Vec::new() }
    }

    /// A hint that only explains, in the help's tone
    pub fn help(icon: &str, text: Text) -> Hint {
        Hint { help: true, ..Hint::new(icon, text, Tone::Normal) }
    }

    pub fn keys(mut self, keys: Vec<Vec<Vec<String>>>) -> Hint {
        self.keys = keys;
        self
    }

    pub fn action(mut self, a: Action) -> Hint {
        self.action = Some(a);
        self
    }
}

/// The question an action asks before it is sent (every UI asks the same)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Confirm {
    pub title: Text,
    pub text: Text,
    /// The answering button ("Remove", "Continue")
    pub button: Text,
    /// The answer removes something: the button is red
    pub destructive: bool,
    /// A text the answer needs, edited in the question; it goes into the params at `param`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<ConfirmInput>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfirmInput {
    pub param: String,
    pub default: String,
}

impl Confirm {
    /// The app's default answer: "Continue"
    pub fn new(title: Text, text: Text) -> Confirm {
        Confirm { title, text, button: Text::new("Continue"), destructive: false, input: None }
    }

    pub fn button(mut self, button: &str) -> Confirm {
        self.button = Text::new(button);
        self
    }

    pub fn destructive(mut self) -> Confirm {
        self.destructive = true;
        self
    }

    pub fn input(mut self, param: &str, default: impl Into<String>) -> Confirm {
        self.input = Some(ConfirmInput { param: param.into(), default: default.into() });
        self
    }
}

/// What a value is that an action asks for before it is sent
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AskKind {
    /// A line of text
    Text,
    /// A secret (never shown, never sent to `views.get`)
    Password,
    /// A folder (an open panel; a TUI: path input with completion)
    Folder,
    /// A file
    File,
    /// An omaflake package (`accept`: vm, image or recipe) – a recipe may also be a plain folder
    Package,
}

/// A value an action needs from the user before it is sent: it goes into the params at `param` (a dotted path,
/// e.g. `params.path`). A UI may ask it inline on the page (a form) or in a dialog.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ask {
    pub param: String,
    pub kind: AskKind,
    pub label: Text,
    /// The panel's message / a longer explanation
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Text>,
    /// The panel's button
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub button: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default: Option<String>,
    /// Package kind or file type: `vm`, `image`, `recipe`, `ipsw`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub accept: Option<String>,
    /// Several can be chosen: the action is done once per value
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub multiple: bool,
    /// A password asked twice ("Repeat"); the action is sent only when both agree
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub repeat: bool,
    /// Checks a chosen value before the panel closes: call `method` with `params` and the value at `param`; an answer
    /// with `valid: false` refuses it with its `reason` (`packages.inspect`)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub check: Option<Check>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub method: String,
    pub params: Value,
    pub param: String,
}

impl Ask {
    pub fn new(param: &str, kind: AskKind, label: Text) -> Ask {
        Ask {
            param: param.into(),
            kind,
            label,
            prompt: None,
            button: None,
            default: None,
            accept: None,
            multiple: false,
            repeat: false,
            check: None,
        }
    }

    pub fn prompt(mut self, prompt: Text) -> Ask {
        self.prompt = Some(prompt);
        self
    }

    pub fn button(mut self, button: &str) -> Ask {
        self.button = Some(Text::new(button));
        self
    }

    /// A folder to export to (the app's `chooseFolder`)
    pub fn export_folder(prompt: Text) -> Ask {
        Ask::new("folder", AskKind::Folder, Text::new("Folder")).prompt(prompt).button("Export Here")
    }

    /// Packages to import, checked by `packages.inspect` before the panel closes (plan B109)
    pub fn packages(kind: &str, external: bool, prompt: &str, button: &str) -> Ask {
        let mut a =
            Ask::new("params.path", AskKind::Package, Text::new("Package")).prompt(Text::new(prompt)).button(button);
        a.accept = Some(kind.into());
        a.multiple = !external;
        a.check = Some(Check {
            method: "packages.inspect".into(),
            params: json!({"expect": kind, "external": external}),
            param: "path".into(),
        });
        a
    }
}

/// What the UI does with a successful answer
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Then {
    /// Show `result.path` in the file manager
    Reveal,
    /// Run `result.shellLine` (`result.argv`) in a terminal
    Terminal,
    /// Leave the page (a form that is done)
    Back,
}

/// Something the user can do. A UI draws `label`/`icon`, greys it out with `reason` when not `enabled`, asks `ask`
/// and `confirm`, then sends `method` with `params` – a core method, or a `ui.*` verb it does itself (api.md §10.4).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Action {
    pub id: String,
    pub label: Text,
    /// What a narrow header or toolbar writes instead of `label` ("Doctor" for "Doctor – requirements and preflight");
    /// absent: `label`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub short_label: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
    pub method: String,
    pub params: Value,
    pub enabled: bool,
    /// Why it is not enabled (the tooltip of a greyed-out button)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Text>,
    /// Tooltip while enabled
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<Text>,
    /// It removes or stops something: drawn red
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub destructive: bool,
    /// A switch: its state now (the action sets the other one)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub on: Option<bool>,
    /// Its work is under way (a spinner instead of the button, or next to it)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub busy: bool,
    /// A count drawn on it (the Doctor's issues)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub badge: Option<usize>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub ask: Vec<Ask>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confirm: Option<Confirm>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub then: Option<Then>,
    /// The default action of its row or page: what Enter, a double click or a click on a build row does, drawn as the
    /// prominent button (at most one per row or page)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub primary: bool,
}

impl Action {
    /// A core method
    pub fn call(id: &str, label: Text, icon: Option<&str>, method: &str, params: Value) -> Action {
        Action {
            id: id.into(),
            label,
            short_label: None,
            icon: icon.map(str::to_string),
            method: method.into(),
            params,
            enabled: true,
            reason: None,
            help: None,
            destructive: false,
            on: None,
            busy: false,
            badge: None,
            ask: Vec::new(),
            confirm: None,
            then: None,
            primary: false,
        }
    }

    /// `ui.show`: open a page (one level deeper)
    pub fn show(id: &str, label: Text, icon: Option<&str>, page: &str, params: Value) -> Action {
        Action::call(id, label, icon, "ui.show", json!({"page": page, "params": params}))
    }

    /// `ui.field`: set the form field `field` of the page `page` (with `params`, without its `values`) – its value goes
    /// to `values.<value_param>`; `value` absent: the UI asks it (`ask`), `null`: back to the default
    pub fn field(
        id: &str,
        label: Text,
        icon: Option<&str>,
        page: &str,
        params: Value,
        field: &str,
        value_param: &str,
    ) -> Action {
        Action::call(
            id,
            label,
            icon,
            "ui.field",
            json!({"page": page, "params": params, "field": field, "valueParam": value_param}),
        )
    }

    /// `ui.show guide`: a guide of the docs – its page on omaflake.dev and, where the app bundles it, the file
    pub fn guide(file: &str, permalink: &str, title: Text, bundled: Option<String>) -> Action {
        let mut params = json!({"file": file, "url": format!("https://omaflake.dev/{permalink}/"), "title": title});
        if let Some(path) = bundled {
            params["path"] = json!(path);
        }
        Action::show("guide", Text::new("Guide"), Some("guide"), "guide", params)
    }

    /// The Doctor's warning sign in place of an action it blocks (the row's primary action: it is the way on)
    pub fn doctor_warning(blockers: &[PrerequisiteDto]) -> Action {
        Action::show("doctor", Text::new("Doctor"), Some("warning"), "doctor", json!({}))
            .help(tf!("Open the Doctor first: %@", blocker_titles(blockers)))
            .primary()
    }

    /// Greyed out while the Doctor reports `blockers` against it: the warning sign on it (`badge`), what to fix in its
    /// tooltip – the action stays where it is (a recipe's build)
    pub fn blocked_by(self, blockers: &[PrerequisiteDto]) -> Action {
        let mut a = self.disabled(tf!("Open the Doctor first: %@", blocker_titles(blockers)));
        a.badge = Some(blockers.len());
        a
    }

    pub fn primary(mut self) -> Action {
        self.primary = true;
        self
    }

    pub fn short(mut self, label: &str) -> Action {
        self.short_label = Some(Text::new(label));
        self
    }

    pub fn enabled_if(mut self, enabled: bool, reason: Option<Text>) -> Action {
        if !enabled {
            self.enabled = false;
            if self.reason.is_none() {
                self.reason = reason;
            }
        }
        self
    }

    pub fn disabled(self, reason: Text) -> Action {
        self.enabled_if(false, Some(reason))
    }

    pub fn help(mut self, help: Text) -> Action {
        self.help = Some(help);
        self
    }

    pub fn destructive(mut self) -> Action {
        self.destructive = true;
        self
    }

    pub fn confirm(mut self, c: Confirm) -> Action {
        self.confirm = Some(c);
        self
    }

    pub fn ask(mut self, a: Ask) -> Action {
        self.ask.push(a);
        self
    }

    pub fn then(mut self, t: Then) -> Action {
        self.then = Some(t);
        self
    }

    pub fn switch(mut self, on: bool) -> Action {
        self.on = Some(on);
        self
    }

    pub fn busy(mut self, busy: bool) -> Action {
        self.busy = busy;
        self
    }
}

/// "Nix, QEMU (NixOS guests)" – the blockers' titles, each translated
pub fn blocker_titles(blockers: &[PrerequisiteDto]) -> Text {
    let parts: Vec<Text> = blockers.iter().map(|b| Text::new(&b.title)).collect();
    let key = vec!["%@"; parts.len()].join(", ");
    Text::fmt(&key, parts.into_iter().map(super::text::Arg::from).collect())
}

/// What the Doctor reports against an action, with a way to it (the app's `DoctorBanner`)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Blockers {
    pub title: Text,
    /// "• Title: hint" per blocker
    pub items: Vec<BlockerLine>,
    pub action: Action,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BlockerLine {
    pub title: Text,
    pub hint: Text,
}

impl Blockers {
    /// None when nothing blocks
    pub fn of(blockers: &[PrerequisiteDto], host: super::HostOs) -> Option<Blockers> {
        (!blockers.is_empty()).then(|| Blockers {
            title: Text::new("Not possible yet – the Doctor reports:"),
            items: blockers
                .iter()
                .map(|p| BlockerLine { title: Text::new(&p.title), hint: super::doctor::hint_on(p, host) })
                .collect(),
            action: Action::show("doctor", Text::new("Open Doctor …"), Some("doctor"), "doctor", json!({})),
        })
    }
}

/// One option of a choice (a filter, a picker)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Choice {
    pub value: Value,
    pub label: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub icon: Option<String>,
}

impl Choice {
    pub fn new(value: Value, label: Text) -> Choice {
        Choice { value, label, icon: None }
    }

    pub fn icon(mut self, icon: &str) -> Choice {
        self.icon = Some(icon.into());
        self
    }
}

/// A list's filter (plan B101): groups the UI offers; the chosen values come back in `params.filter`
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Filter {
    /// Something is chosen (the filter symbol is filled)
    pub active: bool,
    pub groups: Vec<FilterGroup>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FilterKind {
    /// One of `options` (`null` = all)
    Choice,
    /// On or off
    Toggle,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterGroup {
    /// The key in `params.filter`: `status`, `kind`, `tag`, `recipe`, `otherSystems`
    pub id: String,
    pub label: Text,
    pub kind: FilterKind,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub options: Vec<Choice>,
}

/// The filter values a list was asked with (`params.filter`)
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct FilterParams {
    #[serde(default)]
    pub status: Option<bool>,
    #[serde(default)]
    pub kind: Option<crate::models::GuestKind>,
    #[serde(default)]
    pub tag: Option<String>,
    #[serde(default)]
    pub recipe: Option<String>,
    #[serde(default)]
    pub other_systems: bool,
    /// The catalog tabs' search (plan B148)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub search: Option<String>,
    /// The catalog tabs' chips that are on: rows with all of them
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    /// The catalog tabs' page, from 0
    #[serde(default, skip_serializing_if = "is_zero")]
    pub page: usize,
}

fn is_zero(n: &usize) -> bool {
    *n == 0
}

impl FilterParams {
    pub fn is_active(&self) -> bool {
        self.status.is_some()
            || self.kind.is_some()
            || self.tag.is_some()
            || self.recipe.is_some()
            || self.other_systems
    }

    /// The status group ("All" / `on` / `off`) and the guest kinds – every list has them
    pub fn base_groups(&self, on: &str, off: &str) -> Vec<FilterGroup> {
        vec![
            FilterGroup {
                id: "status".into(),
                label: Text::new("Status"),
                kind: FilterKind::Choice,
                value: json!(self.status),
                options: vec![
                    Choice::new(Value::Null, Text::new("All")),
                    Choice::new(json!(true), Text::new(on)),
                    Choice::new(json!(false), Text::new(off)),
                ],
            },
            FilterGroup {
                id: "kind".into(),
                label: Text::new("System"),
                kind: FilterKind::Choice,
                value: json!(self.kind),
                options: vec![
                    Choice::new(Value::Null, Text::new("All")),
                    Choice::new(json!("nixos"), Text::verbatim("NixOS")).icon("nixos"),
                    Choice::new(json!("macos"), Text::verbatim("macOS")).icon("macos"),
                ],
            },
        ]
    }
}

/// The parts every list page has: its title, the count ("3/6" while filtered), its filter and tools
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ListHead {
    pub title: Text,
    pub count: usize,
    /// All rows before the filter, when the filter leaves some out
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub of: Option<usize>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filter: Option<Filter>,
    /// Tools on the right (import, refresh, clear)
    pub actions: Vec<Action>,
    /// Shown instead of rows: the list is empty, or the filter leaves nothing (then with "Reset Filter")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub empty: Option<Hint>,
    /// The catalog tabs (plan B148): a search, chips, a page at a time
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub browse: Option<Browse>,
}

impl ListHead {
    pub fn new(title: &str, count: usize, all: usize) -> ListHead {
        ListHead {
            title: Text::new(title),
            count,
            of: (count != all).then_some(all),
            filter: None,
            actions: vec![],
            empty: None,
            browse: None,
        }
    }
}

/// The rows of a catalog tab shown at once (plan B148)
pub const BROWSE_PAGE: usize = 10;

/// A catalog tab's search, chips and pages (plan B148). The search is `params.filter.search` (a UI sets it while
/// typing), the chips' and pages' actions are `ui.filter` with the whole new filter.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Browse {
    /// What is searched for now
    pub search: String,
    /// The search field's placeholder
    pub placeholder: Text,
    /// The same as a field whose `set` is `ui.filter` with the search – for a UI that edits fields one at a time
    pub field: Field,
    /// Every tag of the rows, the most common first
    pub tags: Vec<BrowseTag>,
    pub pages: Pager,
    /// Clears the search, the chips and the page
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reset: Option<Action>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct BrowseTag {
    pub id: String,
    pub label: Text,
    /// Rows with it
    pub count: usize,
    pub on: bool,
    pub action: Action,
}

/// Where a list is, a page at a time
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Pager {
    /// From 0
    pub index: usize,
    pub count: usize,
    /// "11–20 of 34"; empty while nothing is shown
    pub text: Text,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub previous: Option<Action>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub next: Option<Action>,
}

/// One row to browse: the row, what the search looks into, its tags
pub struct Browsable<T> {
    pub row: T,
    pub haystack: String,
    pub tags: Vec<String>,
}

/// Searches `items`, keeps those with every chip that is on, and cuts out the page asked for (a page that is gone is
/// the last one). Returns the page's rows, how many matched, and the head's `browse`.
pub fn browse<T>(page: &str, f: &FilterParams, items: Vec<Browsable<T>>, placeholder: &str) -> (Vec<T>, usize, Browse) {
    let search = f.search.clone().unwrap_or_default();
    let words: Vec<String> = search.to_lowercase().split_whitespace().map(str::to_string).collect();
    let mut counts: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
    for i in &items {
        for t in &i.tags {
            *counts.entry(t.clone()).or_default() += 1;
        }
    }
    let matched: Vec<Browsable<T>> = items
        .into_iter()
        .filter(|i| {
            let hay = i.haystack.to_lowercase();
            words.iter().all(|w| hay.contains(w)) && f.tags.iter().all(|t| i.tags.contains(t))
        })
        .collect();
    let total = matched.len();
    let pages = total.div_ceil(BROWSE_PAGE).max(1);
    let index = f.page.min(pages - 1);
    let with = |g: FilterParams| {
        Action::call("filter", Text::new("Filter"), None, "ui.filter", json!({"page": page, "filter": g}))
    };
    let go = |id: &str, label: &str, icon: &str, to: usize| {
        let mut a = with(FilterParams { page: to, ..f.clone() });
        a.id = id.into();
        a.label = Text::new(label);
        a.icon = Some(icon.into());
        a
    };
    let rows: Vec<T> = matched.into_iter().skip(index * BROWSE_PAGE).take(BROWSE_PAGE).map(|i| i.row).collect();
    let first = index * BROWSE_PAGE + 1;
    let mut chips: Vec<(String, usize)> = counts.into_iter().collect();
    chips.sort_by(|a, b| b.1.cmp(&a.1).then(a.0.cmp(&b.0)));
    let tags = chips
        .into_iter()
        .map(|(id, count)| {
            let on = f.tags.contains(&id);
            let next: Vec<String> = if on {
                f.tags.iter().filter(|t| **t != id).cloned().collect()
            } else {
                [f.tags.clone(), vec![id.clone()]].concat()
            };
            let mut a = with(FilterParams { tags: next, page: 0, ..f.clone() });
            a.id = "tag".into();
            a.label = Text::verbatim(&id);
            BrowseTag { label: Text::verbatim(&id), id, count, on, action: a.switch(on) }
        })
        .collect();
    let browsing = !search.trim().is_empty() || !f.tags.is_empty();
    let mut field = Field::new("search", FieldKind::Text, Text::new("Search"), json!(search));
    field.placeholder = Some(Text::new(placeholder));
    field.set = Some(Setter {
        method: "ui.filter".into(),
        params: json!({"page": page, "filter": FilterParams { search: None, page: 0, ..f.clone() }}),
        param: "filter.search".into(),
    });
    let browse = Browse {
        search,
        placeholder: Text::new(placeholder),
        field,
        tags,
        pages: Pager {
            index,
            count: pages,
            text: if rows.is_empty() {
                Text::verbatim("")
            } else {
                tf!("%lld–%lld of %lld", first, first + rows.len() - 1, total)
            },
            previous: (index > 0).then(|| go("previous", "Previous", "chevron-left", index - 1)),
            next: (index + 1 < pages).then(|| go("next", "Next", "chevron-right", index + 1)),
        },
        reset: browsing.then(|| {
            let mut a = with(FilterParams { search: None, tags: vec![], page: 0, ..f.clone() });
            a.id = "reset".into();
            a.label = Text::new("Show all");
            a.icon = Some("refresh".into());
            a
        }),
    };
    (rows, total, browse)
}

/// "Install All" in a list's head (Recipes, Modules, Looks): the offered rows it shows – each as its own Install
/// action has it (`{id, source}`), the filter applied – to `method` (`<kind>.installAll`). Asks first when one is not
/// official (code from a source the omaflake team did not review); None while nothing is offered.
pub fn install_all<'a>(
    method: &str,
    offered: impl IntoIterator<Item = (&'a [Action], bool)>,
    busy: bool,
) -> Option<Action> {
    let mut items = Vec::new();
    let mut unreviewed = false;
    for (actions, official) in offered {
        if let Some(install) = actions.iter().find(|a| a.id == "install" && a.enabled) {
            items.push(install.params.clone());
            unreviewed |= !official;
        }
    }
    if items.is_empty() {
        return None;
    }
    let n = items.len();
    let action =
        Action::call("installAll", Text::new("Install All"), Some("download"), method, json!({"items": items}))
            .help(tf!("Install all %lld shown here", n))
            .enabled_if(!busy, None);
    Some(if unreviewed {
        action.confirm(
            Confirm::new(
                tf!("Install all %lld?", n),
                Text::new("Some are not reviewed by the omaflake team – their Nix code runs on this computer when an image is built with them."),
            )
            .button("Install All"),
        )
    } else {
        action
    })
}

/// "Nothing matches the filter." with a way back
pub fn filtered_out(page: &str) -> Hint {
    Hint::new("filter", Text::new("Nothing matches the filter."), Tone::Normal).action(Action::call(
        "resetFilter",
        Text::new("Reset Filter"),
        None,
        "ui.filter",
        json!({"page": page, "filter": null}),
    ))
}

/// What a form field is
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldKind {
    Toggle,
    Text,
    Number,
    Choice,
    /// A folder or file, chosen with a panel (`ask`)
    Path,
    /// Shown, not edited
    Info,
}

/// How a field's change is stored at once (Settings): send `method` with `params`, the new value put at `param`
/// (a dotted path; `null` = back to the default)
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Setter {
    pub method: String,
    pub params: Value,
    pub param: String,
}

/// A form field. On a form page (New VM, macOS base, a new source) its value goes into the page's action params at
/// `param`; in Settings `set` stores it at once.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Field {
    pub id: String,
    pub kind: FieldKind,
    pub label: Text,
    pub value: Value,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub help: Option<Text>,
    /// Shown while empty (the value used then)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<Text>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub choices: Vec<Choice>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step: Option<i64>,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Text>,
    /// What is wrong with the value (validation)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<Text>,
    /// Where the value goes in the page action's params (dotted path)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub param: Option<String>,
    /// On a form page: the key in the page's `params.values` the UI keeps the user's value under (the page is asked
    /// again with it)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_param: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub set: Option<Setter>,
    /// Path fields: how to choose one
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ask: Option<Ask>,
}

impl Field {
    pub fn new(id: &str, kind: FieldKind, label: Text, value: Value) -> Field {
        Field {
            id: id.into(),
            kind,
            label,
            value,
            help: None,
            placeholder: None,
            choices: vec![],
            min: None,
            max: None,
            step: None,
            enabled: true,
            reason: None,
            error: None,
            param: None,
            value_param: None,
            set: None,
            ask: None,
        }
    }

    pub fn param(mut self, p: &str) -> Field {
        self.param = Some(p.into());
        self
    }

    /// A form field: the user's value is kept under `values.<key>` of the page's params
    pub fn value_param(mut self, key: &str) -> Field {
        self.value_param = Some(key.into());
        self
    }

    pub fn help(mut self, t: Text) -> Field {
        self.help = Some(t);
        self
    }

    pub fn range(mut self, min: i64, max: i64, step: i64) -> Field {
        self.min = Some(min);
        self.max = Some(max);
        self.step = Some(step);
        self
    }

    pub fn enabled_if(mut self, enabled: bool, reason: Option<Text>) -> Field {
        self.enabled = enabled;
        if !enabled {
            self.reason = reason;
        }
        self
    }

    pub fn settings(mut self, key: &str, base: Value) -> Field {
        self.set = Some(Setter { method: "settings.set".into(), params: base, param: key.into() });
        self
    }
}

/// Sets `value` at the dotted `path` in `params` (an object), creating objects on the way
pub fn set_param(params: &mut Value, path: &str, value: Value) {
    let mut at = params;
    let parts: Vec<&str> = path.split('.').collect();
    for (i, p) in parts.iter().enumerate() {
        if !at.is_object() {
            *at = Value::Object(Map::new());
        }
        let map = at.as_object_mut().unwrap();
        if i == parts.len() - 1 {
            map.insert(p.to_string(), value);
            return;
        }
        at = map.entry(p.to_string()).or_insert_with(|| Value::Object(Map::new()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn actions_leave_out_what_is_unset() {
        let a = Action::call("stop", Text::new("Shut down"), Some("stop"), "vms.stop", json!({"name": "a"}));
        assert_eq!(
            serde_json::to_value(&a).unwrap(),
            json!({"id": "stop", "label": {"key": "Shut down", "text": "Shut down"}, "icon": "stop",
                   "method": "vms.stop", "params": {"name": "a"}, "enabled": true})
        );
        let b = a.disabled(Text::new("x")).disabled(Text::new("y"));
        assert_eq!(b.reason, Some(Text::new("x")), "the first reason stays");
    }

    #[test]
    fn params_are_set_by_path() {
        let mut p = json!({"guest": {"user": "a"}});
        set_param(&mut p, "guest.locale", json!("de_CH"));
        set_param(&mut p, "params.path", json!("/x"));
        assert_eq!(p, json!({"guest": {"user": "a", "locale": "de_CH"}, "params": {"path": "/x"}}));
    }
}
