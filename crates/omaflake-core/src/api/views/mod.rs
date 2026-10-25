// SPDX-License-Identifier: AGPL-3.0-or-later
//! The presentation model (plan B121, docs/api.md §10): what each page of a UI shows – rows with their labels, lamps
//! and badges, the actions with whether they can be done now and why not, the confirmations they ask, filters and
//! sorting, form fields with their defaults and what is wrong with them – so the SwiftUI app (macOS), the iced app
//! (Linux) and the terminal interface (`omaf ui`) only draw it.
//!
//! `views.get {page, params}` answers a page's model; `views.subscribe {pages}` sends `view.changed {page, params,
//! model}` whenever a subscribed page's model changes (the watcher rebuilds them after every look at the data folder,
//! after actions and job changes). Actions are the API's methods (`vms.start`, …) or a few `ui.*` verbs a UI does
//! itself; the model names the method and its params, so UIs draw and send them generically.
//!
//! Texts are `{key, text, args}` (`text.rs`): the macOS app's English source strings as keys, so its translations
//! keep working, and English for every UI without a table.
//!
//! One module per page: `home`, `vms` (and `vm`), `images` (and `image`), `recipes`, `new_vm`, `build`, `doctor`
//! (and `macBase`), `settings`, `activity`, `import`. The builders are pure functions of `Ctx` – the server gathers it
//! (`Server::views_ctx`), the tests build it from fixtures.

pub mod activity;
pub mod build;
pub mod common;
pub mod credentials;
pub mod doctor;
pub mod home;
pub mod images;
pub mod import;
pub mod locales;
pub mod looks;
pub mod new_vm;
pub mod recipes;
pub mod settings;
pub mod text;
pub mod vms;

#[cfg(test)]
pub(crate) mod tests;

use std::collections::{BTreeMap, HashSet};

use chrono::{DateTime, Utc};
use serde::de::DeserializeOwned;
use serde::Serialize;
use serde_json::{json, Value};

use super::model;
use super::protocol::{params, RpcError};
use super::server::{lock, Server};
use super::types::*;
use crate::models::{GuestKind, RecipeId};
use crate::paths::Paths;

pub use activity::ActivityEntry;
pub use text::{Arg, Text};

/// Every page `views.get` knows
pub const PAGES: [&str; 19] = [
    "home",
    "vms",
    "vm",
    "images",
    "image",
    "recipes",
    "newVm",
    "build",
    "doctor",
    "macBase",
    "settings",
    "activity",
    "import",
    "credentials",
    "credential",
    "looks",
    "locales",
    "locale",
    "baseCredential",
];

/// The computer the core runs on (`initialize`'s `host`): texts that name the Mac, Finder or the Keychain have a
/// host-neutral variant for the others
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HostOs {
    Macos,
    Linux,
}

impl HostOs {
    pub fn this() -> HostOs {
        if cfg!(target_os = "macos") {
            HostOs::Macos
        } else {
            HostOs::Linux
        }
    }
}

/// The last `updates.check` (the Doctor's "Updates" line)
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct UpdateRecord {
    pub state: Option<UpdateState>,
    pub release: Option<ReleaseDto>,
    pub checked: Option<DateTime<Utc>>,
    pub error: Option<String>,
    pub checking: bool,
}

/// Everything the pages are built from – the API's own types, so a page shows exactly what the lists say
#[derive(Debug, Clone)]
pub(crate) struct Ctx {
    pub paths: Paths,
    pub vms: Vec<Vm>,
    pub unreadable_vms: Vec<Unreadable>,
    pub images: Vec<ImageDto>,
    pub unreadable_images: Vec<Unreadable>,
    /// As `recipes.list` gives it (lab recipes only in developer mode)
    pub recipes: RecipeList,
    pub jobs: Vec<JobDto>,
    pub doctor: Option<DoctorReport>,
    /// The Doctor's checks run now
    pub checking: bool,
    /// What the remote sources offer – None while they were not looked into in this session
    pub available: Option<Available>,
    pub fetching: bool,
    pub sources: Vec<SourceDto>,
    /// The credentials (plan B137)
    pub credentials: Vec<CredentialDto>,
    /// The locales (plan B141)
    pub locales: Vec<crate::api::types::LocaleDto>,
    /// The looks here and what the catalogs fetched last offer (plan B141)
    pub looks: crate::api::types::LooksAnswer,
    /// The macOS bases (plan B137)
    pub bases: Vec<BaseDto>,
    pub settings: SettingsDto,
    /// Settings' choices and the computer's guest values (only for the settings page: they read system lists)
    pub settings_answer: Option<SettingsAnswer>,
    pub default_data_folder: String,
    pub mac_max: usize,
    pub activity: Vec<ActivityEntry>,
    pub update: UpdateRecord,
    pub omaf_version: String,
    /// The connected UI's own version (`initialize`), for the Doctor
    pub client_version: Option<String>,
    /// VM → its SSH command line, where there is one (NixOS always; macOS while it has an address)
    pub ssh: BTreeMap<String, String>,
    pub host_user: String,
    pub host_full_name: String,
    pub host: HostOs,
    /// The guides the app bundles (omaflake.app's `Contents/Resources/Guides`), when there is such a folder
    pub guides: Option<std::path::PathBuf>,
}

impl Ctx {
    /// The macOS app's text on a Mac, the host-neutral one elsewhere (both are keys)
    pub fn on_host<'a>(&self, mac: &'a str, other: &'a str) -> &'a str {
        match self.host {
            HostOs::Macos => mac,
            HostOs::Linux => other,
        }
    }

    /// "Guide" for a doc of `docs/` (its file name and permalink on omaflake.dev)
    pub fn guide(&self, file: &str, permalink: &str, title: &str) -> common::Action {
        let bundled = self
            .guides
            .as_ref()
            .map(|d| d.join(format!("{file}.md")))
            .filter(|p| p.exists())
            .map(|p| p.to_string_lossy().into_owned());
        common::Action::guide(file, permalink, Text::new(title), bundled)
    }

    pub fn start_blockers(&self, kind: GuestKind) -> Vec<PrerequisiteDto> {
        self.doctor.as_ref().map(|d| per_kind(&d.start_blockers, kind).clone()).unwrap_or_default()
    }

    pub fn build_blockers(&self, kind: GuestKind) -> Vec<PrerequisiteDto> {
        self.doctor.as_ref().map(|d| per_kind(&d.build_blockers, kind).clone()).unwrap_or_default()
    }

    pub fn builds(&self) -> impl Iterator<Item = &JobDto> {
        self.jobs.iter().filter(|j| j.kind == JobKind::Build)
    }

    /// The recipe's latest build (also an ended one, for its log)
    pub fn latest_build(&self, recipe: &str) -> Option<&JobDto> {
        self.builds().filter(|j| j.recipe.as_deref() == Some(recipe)).last()
    }

    /// The recipe's waiting or running build
    pub fn active_build(&self, recipe: &str) -> Option<&JobDto> {
        self.latest_build(recipe).filter(|j| j.state.is_active())
    }

    pub fn active_job(&self, kind: JobKind) -> Option<&JobDto> {
        self.jobs.iter().rev().find(|j| j.kind == kind && j.state.is_active())
    }

    /// A recipe being downloaded
    pub fn installing(&self, id: &str) -> bool {
        self.jobs
            .iter()
            .any(|j| j.kind == JobKind::RecipeInstall && j.state.is_active() && j.recipe.as_deref() == Some(id))
    }

    pub fn source_name(&self, id: &str) -> String {
        self.sources.iter().find(|s| s.id == id).map(|s| s.display_name.clone()).unwrap_or_else(|| id.to_string())
    }

    /// The first free VM name for a recipe (`vms.nextName`)
    pub fn next_free_name(&self, recipe: &str) -> String {
        let taken: HashSet<String> = self.vms.iter().map(|v| v.name.clone()).collect();
        let name = self
            .recipes
            .recipes
            .iter()
            .find(|r| r.id == recipe)
            .map(|r| r.display_name.clone())
            .unwrap_or_else(|| RecipeId::name_of_id(recipe));
        if taken.contains(&name) {
            model::next_numbered(&name, &taken)
        } else {
            name
        }
    }

    /// The instance store's rule for names, with its English reason
    pub fn validate_name(&self, name: &str) -> Result<(), String> {
        crate::instance_store::InstanceStore::new(self.paths.clone(), "qemu-img")
            .validate(name)
            .map_err(|e| e.message().to_string())
    }
}

fn per_kind<T>(p: &PerKind<T>, kind: GuestKind) -> &T {
    match kind {
        GuestKind::Nixos => &p.nixos,
        GuestKind::Macos => &p.macos,
    }
}

/// A page and its params, as `views.get` and `views.subscribe` take them
#[derive(Debug, Clone, PartialEq, serde::Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PageRef {
    pub page: String,
    #[serde(default = "empty_object")]
    pub params: Value,
}

fn empty_object() -> Value {
    json!({})
}

impl PageRef {
    /// The key a subscription is known by
    pub fn key(&self) -> String {
        format!("{} {}", self.page, self.params)
    }
}

fn to_value<T: Serialize>(v: T) -> Result<Value, RpcError> {
    serde_json::to_value(v).map_err(|e| RpcError::internal(e.to_string()))
}

fn page_params<T: DeserializeOwned>(p: &Value) -> Result<T, RpcError> {
    params(if p.is_null() { json!({}) } else { p.clone() })
}

/// A page built from `ctx` (every page but `import`, which inspects a file)
pub(crate) fn build(ctx: &Ctx, page: &PageRef) -> Result<Value, RpcError> {
    let p = &page.params;
    match page.page.as_str() {
        "home" => to_value(home::page(ctx)),
        "vms" => to_value(vms::page(ctx, &page_params(p)?)),
        "vm" => {
            let p: vms::VmParams = page_params(p)?;
            to_value(vms::vm_page(ctx, &p).ok_or_else(|| {
                RpcError::from(crate::vm_err!(crate::error::codes::NOT_FOUND, "Instance \"{}\" does not exist", p.name))
            })?)
        }
        "images" => to_value(images::page(ctx, &page_params(p)?)),
        "image" => {
            let p: images::ImageParams = page_params(p)?;
            to_value(images::image_page(ctx, &p).ok_or_else(|| {
                RpcError::from(crate::vm_err!(crate::error::codes::NOT_FOUND, "Image \"{}\" does not exist", p.name))
            })?)
        }
        "recipes" => to_value(recipes::page(ctx, &page_params(p)?)),
        "newVm" => {
            let p: new_vm::NewVmParams = page_params(p)?;
            to_value(new_vm::page(ctx, &p).ok_or_else(|| {
                RpcError::from(crate::vm_err!(crate::error::codes::NOT_FOUND, "Image \"{}\" does not exist", p.image))
            })?)
        }
        "build" => to_value(build::page(ctx, &page_params(p)?)),
        "doctor" => to_value(doctor::page(ctx)),
        "macBase" => to_value(doctor::mac_base_page(ctx, &page_params(p)?)),
        "credentials" => to_value(credentials::page(ctx)),
        "looks" => to_value(looks::page(ctx)),
        "locales" => to_value(locales::page(ctx)),
        "locale" => {
            let answer = ctx.settings_answer.as_ref().ok_or_else(|| RpcError::internal("settings not gathered"))?;
            let p: locales::LocaleParams = page_params(p)?;
            to_value(locales::locale_page(ctx, answer, &p).ok_or_else(|| {
                RpcError::from(crate::vm_err!(
                    crate::error::codes::NOT_FOUND,
                    "No locale \"{}\"",
                    p.id.unwrap_or_default()
                ))
            })?)
        }
        "credential" => {
            let p: credentials::CredentialParams = page_params(p)?;
            to_value(credentials::credential_page(ctx, &p).ok_or_else(|| {
                RpcError::from(crate::vm_err!(
                    crate::error::codes::NOT_FOUND,
                    "No credential \"{}\"",
                    p.id.unwrap_or_default()
                ))
            })?)
        }
        "baseCredential" => {
            let p: credentials::BaseCredentialParams = page_params(p)?;
            to_value(credentials::base_credential_page(ctx, &p).ok_or_else(|| {
                RpcError::from(crate::vm_err!(crate::error::codes::NOT_FOUND, "No macOS base \"{}\"", p.name))
            })?)
        }
        "settings" => {
            let answer = ctx.settings_answer.as_ref().ok_or_else(|| RpcError::internal("settings not gathered"))?;
            to_value(settings::page(ctx, answer, &page_params(p)?))
        }
        "activity" => to_value(activity::page(&ctx.activity, ctx.settings.notifications)),
        other => Err(RpcError::invalid_params(format!("page \"{other}\": {}", PAGES.join(", ")))),
    }
}

// MARK: The server's side

impl Server {
    /// Gathers what `pages` need (the settings' system lists and an image's size only when asked for), recipe texts
    /// in `language`
    pub(crate) fn views_ctx(
        &self,
        language: Option<String>,
        client_version: Option<String>,
        pages: &[PageRef],
    ) -> Result<Ctx, RpcError> {
        let language_for_looks = language.clone();
        let state = self.state_in(language).map_err(RpcError::from)?;
        let view = state.view();
        let vms = view.vms();
        let mut images = view.images();
        for p in pages.iter().filter(|p| p.page == "image") {
            if let Some(name) = p.params.get("name").and_then(Value::as_str) {
                if let Some(i) = state.snap.images.values().flatten().find(|i| i.name == name) {
                    let sized = view.image(i, true);
                    if let Some(slot) = images.images.iter_mut().find(|x| x.name == name) {
                        *slot = sized;
                    }
                }
            }
        }
        let recipes = view.recipes(false);
        let ssh = state
            .snap
            .rows
            .iter()
            .filter_map(|r| {
                let c = &r.config;
                let argv = match c.kind {
                    GuestKind::Nixos => crate::ssh_command::SshCommand::arguments(
                        c,
                        &state.paths,
                        crate::ssh_command::SshCommand::DEFAULT_HOST,
                        None,
                        &[],
                    ),
                    GuestKind::Macos => {
                        crate::ssh_command::SshCommand::arguments(c, &state.paths, r.address.as_deref()?, Some(22), &[])
                    }
                };
                Some((c.name.clone(), crate::ssh_command::SshCommand::shell_line(&argv)))
            })
            .collect();
        let settings_answer = if pages.iter().any(|p| p.page == "settings" || p.page == "locale") {
            Some(self.settings_answer()?)
        } else {
            None
        };
        let catalogs = lock(&self.inner.catalogs).fetching;
        Ok(Ctx {
            paths: state.paths.clone(),
            vms: vms.vms,
            unreadable_vms: vms.unreadable,
            images: images.images,
            unreadable_images: images.unreadable,
            recipes,
            jobs: self.inner.jobs.list(),
            doctor: lock(&self.inner.doctor).as_ref().map(model::doctor_report),
            checking: self.inner.checking.load(std::sync::atomic::Ordering::SeqCst) > 0,
            available: self.available_cached(),
            fetching: catalogs,
            sources: self.sources(),
            credentials: super::methods::credential_dtos(&state.paths),
            looks: self.looks_answer(language_for_looks.as_deref()),
            locales: super::methods::locale_dtos(&state.paths),
            bases: super::methods::base_dtos(&state.paths),
            settings: super::methods::settings_dto(&state.paths),
            settings_answer,
            default_data_folder: Paths::default_home().to_string_lossy().into_owned(),
            mac_max: crate::vz_backend::VzBackend::MAX_RUNNING,
            activity: self.events().activity(),
            update: lock(&self.inner.update).clone(),
            omaf_version: crate::VERSION.into(),
            client_version,
            ssh,
            host_user: crate::platform::user_name(),
            host_full_name: crate::platform::full_user_name(),
            host: HostOs::this(),
            guides: crate::paths::Settings::bundled_guides(),
        })
    }

    /// `views.get`: one page's model
    pub(crate) fn view(&self, page: &PageRef) -> Result<Value, RpcError> {
        if page.page == "import" {
            let p: import::ImportParams = page_params(&page.params)?;
            let inspection = self.packages_inspect_for(&p.path, p.expect, p.external)?;
            return to_value(import::page(&p, inspection));
        }
        if !PAGES.contains(&page.page.as_str()) {
            return Err(RpcError::invalid_params(format!("page \"{}\": {}", page.page, PAGES.join(", "))));
        }
        if page.page == "doctor" || page.page == "macBase" {
            self.check_doctor_if_never();
        }
        let client_version = lock(&self.conn.session).client_version.clone();
        build(&self.views_ctx(self.language(), client_version, std::slice::from_ref(page))?, page)
    }

    /// The Doctor's page shows its checks: run them once in the background when they never ran
    fn check_doctor_if_never(&self) {
        if lock(&self.inner.doctor).is_some() || self.inner.checking.load(std::sync::atomic::Ordering::SeqCst) > 0 {
            return;
        }
        let s = self.clone();
        std::thread::spawn(move || {
            let _ = super::jobs::guarded(|| {
                s.check_doctor();
                Ok(())
            });
        });
    }

    /// `views.subscribe`: replaces this connection's subscribed pages; each gets its model at once
    pub(crate) fn views_subscribe(&self, pages: Vec<PageRef>) -> Result<(), RpcError> {
        for p in &pages {
            if !PAGES.contains(&p.page.as_str()) {
                return Err(RpcError::invalid_params(format!("page \"{}\": {}", p.page, PAGES.join(", "))));
            }
        }
        if pages.iter().any(|p| p.page == "doctor" || p.page == "macBase") {
            self.check_doctor_if_never();
        }
        let conn = self.conn.id;
        // A page that stays subscribed with the same params is sent again only when it changes; new ones come at once
        let keep: HashSet<String> = pages.iter().map(PageRef::key).collect();
        self.events().subscribe_views(conn, pages);
        lock(&self.inner.views_last).retain(|(c, key), _| *c != conn || keep.contains(key));
        if self.events().any() {
            self.start_watcher(&HashSet::new());
        }
        Ok(())
    }

    /// Rebuilds every connection's subscribed pages and sends each connection those that changed (`view.changed`) –
    /// from the watcher. Pages are built once per language; the activity list is the same for every connection.
    pub(crate) fn send_views(&self) {
        let subs = self.events().view_subscriptions();
        {
            // Forget what was sent to connections that are gone or unsubscribed pages
            let live: HashSet<(u64, String)> =
                subs.iter().flat_map(|s| s.pages.iter().map(move |p| (s.conn, p.key()))).collect();
            lock(&self.inner.views_last).retain(|k, _| live.contains(k));
        }
        // One context per language and client version (the Doctor names the UI's version)
        type Key = (Option<String>, Option<String>);
        let key = |s: &super::events::ViewSubscription| -> Key { (s.language.clone(), s.client_version.clone()) };
        let mut contexts: Vec<(Key, Result<Ctx, RpcError>)> = Vec::new();
        for sub in &subs {
            let k = key(sub);
            if !contexts.iter().any(|(l, _)| *l == k) {
                let pages: Vec<PageRef> = subs.iter().filter(|s| key(s) == k).flat_map(|s| s.pages.clone()).collect();
                let ctx = self.views_ctx(k.0.clone(), k.1.clone(), &pages);
                contexts.push((k, ctx));
            }
        }
        for sub in subs {
            let k = key(&sub);
            let Some((_, ctx)) = contexts.iter().find(|(l, _)| *l == k) else { continue };
            let ctx = match ctx {
                Ok(c) => c,
                Err(e) => {
                    eprintln!("omaf serve: views: {}", e.message);
                    continue;
                }
            };
            for page in sub.pages {
                let model = if page.page == "import" { self.view(&page) } else { build(ctx, &page) };
                let model = match model {
                    Ok(m) => m,
                    Err(e) => json!({"error": {"message": e.message, "kind": e.kind, "text": e.text()}}),
                };
                let key = (sub.conn, page.key());
                {
                    let mut last = lock(&self.inner.views_last);
                    if last.get(&key) == Some(&model) {
                        continue;
                    }
                    last.insert(key, model.clone());
                }
                self.events().send_to(
                    sub.conn,
                    "view.changed",
                    json!({"page": page.page, "params": page.params, "model": model}),
                );
            }
        }
    }
}
