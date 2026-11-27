// SPDX-License-Identifier: AGPL-3.0-or-later
//! The building blocks every page uses (docs/api.md §10.3): actions with their confirmation and what they ask for,
//! status lamps, badges, info lines, hints, filters and form fields.

use serde_json::json;

use super::text::{Text, tf};
use crate::api::types::PrerequisiteDto;

// The page's model (plan B192: in omaflake-sdk); this file builds it
pub use omaflake_sdk::views::common::*;

/// The rows of a catalog tab shown at once (plan B148)
pub const BROWSE_PAGE: usize = 10;

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

/// "Update All": every update of the tab ({id, source} each, on every page), as one call of `method` (its install-all)
pub fn update_all(method: &str, items: Vec<serde_json::Value>, busy: bool) -> Option<Action> {
    (!items.is_empty()).then(|| {
        Action::call("updateAll", Text::new("Update All"), Some("download"), method, json!({"items": items}))
            .help(tf!("Update all %lld", items.len()))
            .enabled_if(!busy, None)
    })
}

/// Parts here that cannot be read and no source offers anew; before the sources were looked into, a way to do it –
/// one of them may have a version this omaflake reads
pub fn unreadable_hints<'a>(
    items: impl IntoIterator<Item = &'a (String, String)>,
    fetched: bool,
    refresh: &Action,
) -> Vec<Hint> {
    items
        .into_iter()
        .map(|(id, e)| {
            let hint = Hint::new("warning", tf!("%@ cannot be read: %@", id, e), Tone::Warning);
            if fetched {
                hint
            } else {
                let mut look = refresh.clone();
                look.label = Text::new("Look for an Update");
                hint.action(look)
            }
        })
        .collect()
}

/// The lamp of a part here that cannot be read: its source's version replaces it
pub fn broken_lamp() -> Lamp {
    Lamp::new(LampState::Attention, "Cannot be read – Update replaces it with its source's version")
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

/// The Doctor's blockers as a page's box, None when there are none – in the core, not in omaflake-sdk with
/// `Blockers`: the hints are the Doctor's (plan B192)
pub fn blockers_of(blockers: &[PrerequisiteDto], host: super::HostOs) -> Option<Blockers> {
    (!blockers.is_empty()).then(|| Blockers {
        title: Text::new("Not possible yet – the Doctor reports:"),
        items: blockers
            .iter()
            .map(|p| BlockerLine { title: Text::new(&p.title), hint: super::doctor::hint_on(p, host) })
            .collect(),
        action: Action::show("doctor", Text::new("Open Doctor …"), Some("doctor"), "doctor", serde_json::json!({})),
    })
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
