// SPDX-License-Identifier: AGPL-3.0-or-later
//! The methods of docs/api.md §7: params, the core calls (as the app's AppModel made them) and the answers.
//! Secrets (`password`, `token`) are never logged and kept only as long as the work needs them (api.md §8).

use crate::vm_err;
use std::collections::{BTreeMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::Ordering;
use std::time::Duration;

use serde::Deserialize;
use serde_json::{json, Value};

use super::events::Topic;
use super::jobs::{JobCtx, JobSpec};
use super::model::{self, catalog_dto, source_dto};
use super::protocol::{params, params_map, RpcError};
use super::server::{lock, Answer, Reply, Server};
use super::types::*;
use crate::backend::{Readiness, StartOptions};
use crate::build_progress::BuildLog;
use crate::builder_maintenance::{AdminShell, BuilderMaintenance, NixVolume};
use crate::cli_link::CliLink;
use crate::credentials::{CredentialChange, Credentials};
use crate::data_folder::DataFolder;
use crate::error::{codes, Error};
use crate::guest_choices::GuestChoices;
use crate::guest_settings::{BuildChoice, GuestSettings};
use crate::image_store::ImageStore;
use crate::ipsw_downloader::MacOsRelease;
use crate::mac_base_installer::{BaseOutcome, BaseRequest, MacBaseInstaller};
use crate::mac_image_builder::{BuildRequest, MacImageBuilder};
use crate::models::{DisplayMode, GlBackend, GuestKind, RecipeDefaults, RecipeId, VmStatus};
use crate::omaflake_id::OmaflakeId;
use crate::packages::{Package, PackageKind};
use crate::paths::{Paths, Settings};
use crate::platform::{self, Platform};
use crate::prerequisites::{PrerequisiteStatus, Prerequisites};
use crate::qemu_runtime::{tail, InstanceLock, QemuRuntime};
use crate::recipe_sources::{RecipeLibrary, RecipeSource, SourceClient, SourceTokens};
use crate::transfer::{Conflict, Transfer};
use crate::update_check::UpdateCheck;
use crate::vz_backend::VzBackend;

fn err(e: Error) -> RpcError {
    e.into()
}

#[derive(Deserialize)]
struct Name {
    name: String,
}

#[derive(Deserialize)]
struct Id {
    id: String,
}

impl Server {
    pub(crate) fn dispatch(&self, method: &str, p: Value) -> Reply {
        match method {
            // Session and settings
            "initialize" => self.initialize(params(p)?),
            "events.subscribe" => self.subscribe(p),
            // Ends this connection's session (the stdio server ends with it; the service goes on for the others)
            "shutdown" => {
                self.conn.ended.store(true, Ordering::SeqCst);
                Ok(Answer::Value(Value::Null))
            }
            "service.restart" => {
                self.inner.restart.store(true, Ordering::SeqCst);
                Answer::of(ServiceRestart {
                    jobs: self.inner.jobs.list().iter().filter(|j| j.state.is_active()).count(),
                })
            }
            "settings.get" => Answer::of(self.settings_answer()?),
            "settings.set" => self.settings_set(p),
            // Credentials (plan B137)
            "credentials.list" => Answer::of(credential_dtos(&self.paths())),
            "locales.list" => Answer::of(locale_dtos(&self.paths())),
            "profiles.list" => Answer::of(profile_dtos(&self.paths())),
            "profiles.add" | "profiles.update" => {
                let p: ProfileParamsIn = params(p)?;
                let profiles = crate::profiles::Profiles::new(self.paths());
                let change = crate::profiles::ProfileChange { name: p.name, cpus: p.cpus, memory_gb: p.memory_gb };
                let x = if method == "profiles.update" {
                    let id = p.id.ok_or_else(|| RpcError::invalid_params("id is needed"))?;
                    profiles.update(&id, change).map_err(err)?
                } else {
                    profiles.add(change).map_err(err)?
                };
                self.poke();
                Answer::of(json!({"id": x.id}))
            }
            "profiles.remove" => {
                crate::profiles::Profiles::new(self.paths()).remove(&params::<Id>(p)?.id).map_err(err)?;
                self.poke();
                Answer::of(Value::Null)
            }
            "profiles.setDefault" => {
                crate::profiles::Profiles::new(self.paths()).set_default(&params::<Id>(p)?.id).map_err(err)?;
                self.poke();
                Answer::of(Value::Null)
            }
            "locales.add" | "locales.update" => {
                let p: LocaleParamsIn = params(p)?;
                let locales = crate::locales::Locales::new(self.paths());
                let change = crate::locales::LocaleChange {
                    name: p.name,
                    locale: p.locale,
                    time_zone: p.time_zone,
                    keyboards: p.keyboards,
                };
                let l = if method == "locales.update" {
                    let id = p.id.ok_or_else(|| RpcError::invalid_params("id is needed"))?;
                    locales.check_change(&id, &change).map_err(err)?;
                    locales.update(&id, change).map_err(err)?
                } else {
                    locales.add(change).map_err(err)?
                };
                self.check_freshness();
                self.poke();
                Answer::of(json!({"id": l.id}))
            }
            "locales.remove" => {
                let id = params::<Id>(p)?.id;
                let locales = crate::locales::Locales::new(self.paths());
                if !locales.images_with(&id).is_empty() {
                    return Err(RpcError::from(vm_err!(
                        codes::IN_USE,
                        "Images are built with this locale – remove them first"
                    )));
                }
                let l = locales.remove(&id).map_err(err)?;
                self.events().message_text(Level::Info, super::views::text::tf!("Locale “%@” removed", l.name));
                self.poke();
                Answer::of(Value::Null)
            }
            "locales.setDefault" => {
                crate::locales::Locales::new(self.paths()).set_default(&params::<Id>(p)?.id).map_err(err)?;
                self.poke();
                Answer::of(Value::Null)
            }
            "credentials.add" => self.credentials_change(None, params(p)?),
            "credentials.update" => {
                let p: CredentialParams = params(p)?;
                let id = p.id.clone().ok_or_else(|| RpcError::invalid_params("id is needed"))?;
                self.credentials_change(Some(id), p)
            }
            "credentials.copyPassword" => self.credentials_copy_password(&params::<Id>(p)?.id),
            "credentials.remove" => {
                let id = params::<Id>(p)?.id;
                // Images and macOS bases carry its values: a credential in use stays (plan B137, B139)
                crate::credentials::Usage::of(&self.paths(), &id).check_remove().map_err(err)?;
                let c = Credentials::new(self.paths()).remove(&id).map_err(err)?;
                self.events().message_text(Level::Info, super::views::text::tf!("Credential “%@” removed", c.name));
                self.check_freshness();
                self.poke();
                Answer::of(Value::Null)
            }
            "dataFolder.move" => self.data_folder_move(params(p)?),
            "updates.check" => Answer::of(self.updates_check()?),
            // Views (plan B121, api.md §10)
            "views.get" => Answer::of(self.view(&params::<super::views::PageRef>(p)?)?),
            "views.subscribe" => {
                #[derive(Deserialize)]
                struct P {
                    pages: Vec<super::views::PageRef>,
                }
                self.views_subscribe(params::<P>(p)?.pages)?;
                Ok(Answer::Value(Value::Null))
            }
            "activity.clear" => {
                self.events().clear_activity();
                Ok(Answer::Value(Value::Null))
            }
            // VMs
            "vms.list" => Answer::of(self.state().map_err(err)?.view().vms()),
            "vms.get" => Answer::of(self.vm(&params::<Name>(p)?.name)?),
            "vms.create" => self.vms_create(params(p)?),
            "vms.start" => self.vms_start(params(p)?),
            "vms.stop" => self.vms_stop(params(p)?),
            "vms.reset" => self.vms_reset_remove(&params::<Name>(p)?.name, false),
            "vms.remove" => self.vms_reset_remove(&params::<Name>(p)?.name, true),
            "vms.upgrade" => self.vms_upgrade(params(p)?),
            "vms.set" => self.vms_set(params(p)?),
            "vms.ssh" => self.vms_ssh(params(p)?),
            "vms.agent" => self.vms_agent(params(p)?),
            "vms.log" => self.vms_log(params(p)?),
            "vms.wait" => self.vms_wait(params(p)?),
            "vms.share" => self.vms_share(&params::<Name>(p)?.name),
            "vms.nextName" => self.vms_next_name(params(p)?),
            "vms.forget" => self.vms_forget(&params::<Name>(p)?.name),
            "vms.useExternal" => self.vms_use_external(params(p)?),
            "vms.limits" => {
                let state = self.state().map_err(err)?;
                Answer::of(Limits { mac_max: VzBackend::MAX_RUNNING, mac_running: state.snap.running_macs() })
            }
            // Images
            "images.list" => Answer::of(self.state().map_err(err)?.view().images()),
            "images.get" => self.images_get(&params::<Name>(p)?.name),
            "images.build" => self.images_build(params(p)?),
            "images.remove" => self.images_remove(&params::<Name>(p)?.name),
            // macOS bases (plan B137)
            "bases.list" => Answer::of(base_dtos(&self.paths())),
            "bases.remove" => self.bases_remove(&params::<Name>(p)?.name),
            "bases.assign" => self.bases_assign(params(p)?),
            // Recipes and sources
            "recipes.list" => self.recipes_list(params(p)?),
            "recipes.available" => Answer::of(self.recipes_available(true)?),
            "recipes.install" => self.recipes_install(params(p)?),
            "recipes.installParts" => self.recipes_install_parts(&params::<Id>(p)?.id),
            "recipes.save" => {
                let p: SaveRecipeParams = params(p)?;
                let previous = p.previous.as_deref().and_then(|b| {
                    let mut parts = b.split('.');
                    let (_, kind, name) = (parts.next()?, parts.next()?, parts.next()?);
                    Some((GuestKind::parse(kind)?, name.to_string()))
                });
                let draft = crate::my_recipes::RecipeDraft {
                    name: p.name,
                    platform: GuestKind::parse(&p.platform),
                    description: p.description,
                    modules: p.modules.iter().map(|m| crate::composition::PartRef::new(m)).collect(),
                    look: p.look.filter(|l| !l.is_empty()).map(|l| crate::composition::PartRef::new(&l)),
                    keymaps: p
                        .keymaps
                        .iter()
                        .filter(|k| !k.is_empty())
                        .map(|k| crate::composition::PartRef::new(k))
                        .collect(),
                    requirements: p.requirements,
                    vm: p.vm,
                    locales: Default::default(),
                };
                let base = crate::my_recipes::MyRecipes::new(self.paths()).save(&draft, previous).map_err(err)?;
                lock(&self.inner.catalogs).fetched = false;
                self.reload_recipes().map_err(err)?;
                Answer::of(json!({"base": base}))
            }
            "keymaps.save" => {
                let p: SaveKeymapParams = params(p)?;
                let draft = crate::my_recipes::KeymapDraft {
                    name: p.name,
                    title: p.title,
                    description: p.description,
                    keys: p.keys,
                };
                let reference =
                    crate::my_recipes::MyRecipes::new(self.paths()).save_keymap(&draft, p.previous).map_err(err)?;
                self.reload_recipes().map_err(err)?;
                Answer::of(json!({"reference": reference}))
            }
            "keymaps.delete" => {
                let name = p.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                crate::my_recipes::MyRecipes::new(self.paths()).delete_keymap(&name).map_err(err)?;
                self.reload_recipes().map_err(err)?;
                Answer::of(Value::Null)
            }
            "recipes.delete" => {
                let base = params::<BaseParams>(p)?.base;
                let mut parts = base.split('.');
                let (kind, name) = match (parts.next(), parts.next().and_then(GuestKind::parse), parts.next()) {
                    (Some(crate::my_recipes::MyRecipes::AUTHOR), Some(k), Some(n)) => (k, n.to_string()),
                    _ => return Err(RpcError::invalid_params("base is one of your recipes: my.<platform>.<name>")),
                };
                crate::my_recipes::MyRecipes::new(self.paths()).delete(kind, &name).map_err(err)?;
                self.reload_recipes().map_err(err)?;
                Answer::of(Value::Null)
            }
            "looks.list" => {
                let fetch = p.get("fetch").and_then(Value::as_bool).unwrap_or(false);
                if fetch || !lock(&self.inner.catalogs).fetched {
                    self.fetch_catalogs();
                }
                Answer::of(self.looks_answer(self.language().as_deref()))
            }
            "looks.install" => self.looks_install(params(p)?),
            "modules.list" => {
                let fetch = p.get("fetch").and_then(Value::as_bool).unwrap_or(false);
                if fetch || !lock(&self.inner.catalogs).fetched {
                    self.fetch_catalogs();
                }
                Answer::of(self.modules_answer(self.language().as_deref()))
            }
            "modules.install" => self.modules_install(params(p)?),
            "modules.installAll" => self.modules_install_all(params(p)?),
            "looks.installAll" => self.looks_install_all(params(p)?),
            "recipes.installAll" => self.recipes_install_all(params(p)?),
            "modules.remove" => self.modules_remove(&params::<Id>(p)?.id),
            "looks.installRecommended" => self.looks_install_recommended(),
            "looks.remove" => self.looks_remove(&params::<Id>(p)?.id),
            "looks.setDefault" => {
                let id = params::<Id>(p)?.id;
                crate::looks::LookStore::new(self.paths()).set_default(&id).map_err(err)?;
                self.check_freshness();
                self.check_doctor_later();
                self.poke();
                Answer::of(Value::Null)
            }
            "recipes.installSuggested" => self.recipes_install_suggested(),
            "recipes.installUpdates" => self.recipes_install_updates(),
            "recipes.remove" => self.recipes_remove(&params::<Id>(p)?.id),
            "recipes.removeLine" => self.recipes_remove_line(params(p)?),
            "recipes.pruneVersions" => {
                let removed = RecipeLibrary::new(self.paths()).prune_versions();
                self.reload_recipes_later();
                Answer::of(Pruned { removed })
            }
            "sources.list" => Answer::of(self.sources()),
            "sources.add" => self.sources_add(params(p)?),
            "sources.remove" => self.sources_remove(&params::<Id>(p)?.id),
            "sources.setEnabled" => self.sources_set_enabled(params(p)?),
            "sources.move" => self.sources_move(params(p)?),
            "sources.check" => self.sources_check(),
            // Packages
            "packages.inspect" => Answer::of(self.packages_inspect(params(p)?)?),
            "packages.import" => self.packages_import(params(p)?),
            "packages.export" => self.packages_export(params(p)?),
            // Doctor, macOS base, Linux builder
            "doctor.check" => Answer::of(model::doctor_report(&self.check_doctor())),
            "doctor.setPath" => self.doctor_set_path(params(p)?),
            "doctor.installCli" => self.cli_job(true),
            "doctor.uninstallCli" => self.cli_job(false),
            "macos.downloadIpsw" => self.macos_download_ipsw(),
            "macos.installBase" => self.macos_install_base(params(p)?),
            "builder.installNix" => self.builder_job(JobKind::BuilderInstallNix, false),
            "builder.mountNix" => self.builder_job(JobKind::BuilderMountNix, false),
            "builder.setup" => self.builder_job(JobKind::BuilderSetup, false),
            "builder.apply" => {
                #[derive(Deserialize)]
                struct P {
                    #[serde(default)]
                    force: bool,
                }
                self.builder_job(JobKind::BuilderApply, params::<P>(p)?.force)
            }
            "builder.cleanUp" => self.builder_job(JobKind::BuilderCleanUp, false),
            // Jobs
            "jobs.list" => Answer::of(self.inner.jobs.list()),
            "jobs.log" => {
                #[derive(Deserialize)]
                struct P {
                    id: String,
                    #[serde(default)]
                    from: usize,
                }
                let p: P = params(p)?;
                Answer::of(self.inner.jobs.find(&p.id).map_err(err)?.log(p.from))
            }
            "jobs.cancel" => Answer::of(self.inner.jobs.cancel(&params::<Id>(p)?.id).map_err(err)?),
            "jobs.dismiss" => {
                self.inner.jobs.dismiss(&params::<Id>(p)?.id).map_err(err)?;
                Ok(Answer::Value(Value::Null))
            }
            _ => Err(RpcError::method_not_found(method)),
        }
    }

    // MARK: Session

    fn initialize(&self, p: InitializeParams) -> Reply {
        if let Some(v) = p.api_version {
            if v != API_VERSION {
                return Err(RpcError::invalid_params(format!(
                    "API version {v} is not served – this omaf speaks version {API_VERSION}"
                )));
            }
        }
        {
            let mut s = lock(&self.conn.session);
            s.language = p.language.map(|l| l.chars().take(2).collect());
            s.update_feed = p.update_feed.filter(|f| !f.is_empty());
            s.client_version = p.client_version.clone().filter(|v| !v.is_empty());
            self.inner.events.set_language(self.conn.id, s.language.clone());
            self.inner.events.set_client_version(self.conn.id, s.client_version.clone());
        }
        let paths = self.paths();
        if self.inner.migrate_at_start && !self.inner.migrated.swap(true, Ordering::SeqCst) {
            // Older layouts (before image.json, before packages – plan B108); first, so the lists see the result
            let _ = super::jobs::guarded(|| {
                ImageStore::new(paths.clone()).migrate_legacy();
                Ok(())
            });
        }
        self.conn.initialized.store(true, Ordering::SeqCst);
        Answer::of(InitializeResult {
            api_version: API_VERSION,
            omaf_version: crate::VERSION.into(),
            host: host(),
            data_folder: paths.root.to_string_lossy().into_owned(),
            config_file: paths.config_file.to_string_lossy().into_owned(),
        })
    }

    fn subscribe(&self, p: Value) -> Reply {
        #[derive(Deserialize)]
        struct P {
            topics: Vec<String>,
        }
        let p: P = params(p)?;
        // Unknown topics are left out (a newer client may know more)
        let topics: HashSet<Topic> = p.topics.iter().filter_map(|t| Topic::parse(t)).collect();
        let fresh = self.events().subscribe(self.conn.id, topics);
        if !fresh.is_empty() {
            self.start_watcher(&fresh);
        }
        if fresh.contains(&Topic::Doctor) {
            // The checks run at start (AppModel.init), then after actions and before builds
            let s = self.clone();
            std::thread::spawn(move || {
                let _ = super::jobs::guarded(|| {
                    s.check_doctor();
                    Ok(())
                });
            });
        }
        if fresh.contains(&Topic::Jobs) {
            // Only to this connection – the others have them
            for job in self.inner.jobs.list() {
                self.events().emit_to(self.conn.id, Topic::Jobs, "jobs.changed", &JobChanged { job });
            }
        }
        Ok(Answer::Value(Value::Null))
    }

    // MARK: Settings

    pub(crate) fn settings_answer(&self) -> Result<SettingsAnswer, RpcError> {
        let paths = self.paths();
        let mac = GuestSettings::mac();
        Ok(SettingsAnswer {
            settings: settings_dto(&paths),
            guest_defaults: GuestValues {
                user: Some(mac.user),
                full_name: Some(mac.full_name),
                email: mac.email,
                time_zone: Some(mac.time_zone),
                locale: Some(mac.locale),
                keyboard: Some(mac.keyboard),
                look: None,
            },
            choices: Choices {
                time_zones: GuestChoices::time_zones(),
                locales: GuestChoices::locales(),
                keyboards: GuestChoices::KEYBOARDS
                    .iter()
                    .map(|(code, locale)| Keyboard { code: code.to_string(), locale: locale.to_string() })
                    .collect(),
                // The installed looks (plan B141): their ids and titles
                looks: crate::looks::LookStore::new(paths.clone())
                    .list(&Settings::load(&paths))
                    .into_iter()
                    .map(|l| Look { name: l.id, title: l.title })
                    .collect(),
            },
            is_default_data_folder: is_default_data_folder(&paths),
        })
    }

    /// Changes settings: only the keys given; `null` = back to the default (`AppModel`'s property observers)
    fn settings_set(&self, p: Value) -> Reply {
        let p = params_map(p)?;
        let paths = self.paths();
        let mut s = Settings::load(&paths);
        let before = s.clone();
        let flag = |key: &str| -> Result<Option<Option<bool>>, RpcError> {
            match p.get(key) {
                None => Ok(None),
                Some(Value::Null) => Ok(Some(None)),
                Some(Value::Bool(b)) => Ok(Some(Some(*b))),
                Some(_) => Err(RpcError::invalid_params(format!("{key} is true, false or null"))),
            }
        };
        let text = |key: &str| -> Result<Option<Option<String>>, RpcError> {
            match p.get(key) {
                None => Ok(None),
                Some(Value::Null) => Ok(Some(None)),
                Some(Value::String(v)) if v.trim().is_empty() => Ok(Some(None)),
                Some(Value::String(v)) => Ok(Some(Some(v.clone()))),
                Some(_) => Err(RpcError::invalid_params(format!("{key} is a string or null"))),
            }
        };
        // Stored only where they differ from the default (as the app wrote them)
        if let Some(v) = flag("developerMode")? {
            s.developer_mode = v.unwrap_or(false).then_some(true);
        }
        if let Some(v) = flag("keepRecipeVersions")? {
            s.keep_recipe_versions = v.unwrap_or(false).then_some(true);
        }
        if let Some(v) = flag("checkForUpdates")? {
            s.check_for_updates = (!v.unwrap_or(true)).then_some(false);
        }
        if let Some(v) = flag("notifications")? {
            s.notifications = (!v.unwrap_or(true)).then_some(false);
        }
        if let Some(v) = flag("help")? {
            s.help = v.unwrap_or(false).then_some(true);
        }
        for key in ["flake", "qemu", "nix", "runner"] {
            if let Some(v) = text(key)? {
                match key {
                    "flake" => s.flake = v,
                    "qemu" => s.qemu = v,
                    "nix" => s.nix = v,
                    _ => s.runner = v,
                }
            }
        }
        // This computer's keyboard (plan B141): the physical one, preset for new locales; null: as the system says
        if let Some(v) = text("hostKeyboard")? {
            if let Some(k) = &v {
                if !GuestChoices::KEYBOARDS.iter().any(|(code, _)| code == k) {
                    return Err(RpcError::invalid_params(format!("hostKeyboard: no keyboard \"{k}\"")));
                }
            }
            s.host_keyboard = v;
        }
        match p.get("defaultCredential") {
            None => {}
            // There is always a default while there are credentials (plan B139)
            Some(Value::Null) => return Err(RpcError::invalid_params("defaultCredential: a credential's id")),
            Some(Value::String(id)) => {
                if !s.credentials.as_ref().is_some_and(|all| all.iter().any(|c| &c.id == id)) {
                    return Err(RpcError::from(vm_err!(codes::NOT_FOUND, "No credential \"{id}\"")));
                }
                s.default_credential = Some(id.clone());
            }
            Some(_) => return Err(RpcError::invalid_params("defaultCredential is a credential's id or null")),
        }
        if s != before {
            s.save(&paths).map_err(err)?;
        }
        if before.keep_recipe_versions == Some(true) && s.keep_recipe_versions.is_none() {
            // Off: older versions go unless an image uses them (plan B111)
            let removed = RecipeLibrary::new(paths.clone()).prune_versions();
            if !removed.is_empty() {
                self.events().message_text(
                    Level::Info,
                    super::views::text::tf!("Older recipe versions removed: %lld", removed.len()),
                );
            }
        }
        let paths_changed =
            before.flake != s.flake || before.qemu != s.qemu || before.nix != s.nix || before.runner != s.runner;
        if paths_changed {
            self.check_doctor_later();
        }
        if before.host_keyboard != s.host_keyboard {
            // Images without a locale take this computer's keyboard: the lists say "outdated" now (plan B135)
            self.check_freshness();
        }
        if paths_changed
            || before.developer_mode != s.developer_mode
            || before.keep_recipe_versions != s.keep_recipe_versions
        {
            self.reload_recipes_later();
        }
        Answer::of(settings_dto(&paths))
    }

    fn data_folder_move(&self, p: TargetParams) -> Reply {
        if self.inner.jobs.any_active(&[JobKind::DataMove]).is_some() {
            return Err(RpcError::from(crate::vm_err!(codes::IN_USE, "The data folder is being moved already")));
        }
        let state = self.state().map_err(err)?;
        if state.snap.rows.iter().any(|r| r.status == VmStatus::Running) {
            return Err(RpcError::from(crate::vm_err!(
                codes::VM_RUNNING,
                "Stop all VMs before moving the data folder."
            )));
        }
        let target = PathBuf::from(platform::expand_tilde(&p.target));
        let s = self.clone();
        let mut spec = JobSpec::new(
            JobKind::DataMove,
            format!("Move the data folder to {}", target.display()),
            Box::new(move |ctx: &JobCtx| {
                let old = s.paths();
                let log = ctx.log_sink();
                let new = DataFolder::move_to(&old, &target, &old.config_file, &*log)?;
                s.set_paths(new.clone());
                result(DataMoveResult { data_folder: new.root.to_string_lossy().into_owned() })
            }),
        );
        spec.cancellable = false;
        // Next to config.json, which stays where it is: logs/ moves along with the data
        spec.log_file = Some(self.paths().config_file.with_file_name("data-move.log"));
        let s = self.clone();
        spec.on_end = Some(Box::new(move |_| {
            s.reload_recipes_later();
            s.check_doctor_later();
        }));
        Ok(Answer::Job(self.inner.jobs.submit(spec)))
    }

    // MARK: VMs

    pub(crate) fn vm(&self, name: &str) -> Result<Vm, RpcError> {
        let state = self.state().map_err(err)?;
        let row = state
            .snap
            .row(name)
            .ok_or_else(|| RpcError::from(crate::vm_err!(codes::NOT_FOUND, "Instance \"{name}\" does not exist")))?;
        Ok(state.view().vm(row))
    }

    fn qemu_gl(&self) -> bool {
        let r = lock(&self.inner.recipes);
        if r.loaded {
            return r.gl;
        }
        drop(r);
        super::jobs::guarded(|| {
            Ok(QemuRuntime::locate(&self.paths()).map(|(q, _)| q.capabilities().gl).unwrap_or(false))
        })
        .unwrap_or(false)
    }

    fn mac_limit(&self) -> Result<Option<String>, RpcError> {
        Ok(model::mac_limit_reason(&self.state().map_err(err)?.snap.running_macs()))
    }

    fn vms_create(&self, p: CreateParams) -> Reply {
        let settings = recipe_defaults(&p.settings);
        let kind = settings.kind;
        let mut start = p.start;
        if start && kind == GuestKind::Macos {
            if let Some(reason) = self.mac_limit()? {
                self.events().warning(format!("{}: {reason}", p.name));
                start = false;
            }
        }
        let gl = kind == GuestKind::Nixos && self.qemu_gl();
        // No lock yet: the instance folder (and its lock file) is created by `create`
        self.act(&p.name, false, |backends, paths| {
            let backend = backends.backend(kind)?;
            let image = ImageStore::new(paths.clone()).image(&p.image)?;
            if image.kind() != kind {
                return Err(vm_err!("{} is a {} image, the settings are for {}", image.name, image.kind(), kind));
            }
            let mut c = backend.create(&p.name, image.recipe(), &image, &settings)?;
            if settings.share {
                if let Some(share) = &p.share_path {
                    c.share_path = Some(share.clone());
                    backends.store.save(&c)?;
                }
            }
            if start {
                let _lock = InstanceLock::new(&paths.instance(&p.name))?;
                backend.start(&c, StartOptions { display: model::start_display(&c, gl, None), gl: GlBackend::On })?;
            }
            Ok(())
        })?;
        if start {
            self.when_reachable(&p.name);
        }
        self.check_doctor_later();
        Answer::of(self.vm(&p.name)?)
    }

    fn vms_start(&self, p: StartParams) -> Reply {
        let state = self.state().map_err(err)?;
        let row = state.snap.row(&p.name).ok_or_else(|| {
            RpcError::from(crate::vm_err!(codes::NOT_FOUND, "Instance \"{}\" does not exist", p.name))
        })?;
        let kind = row.config.kind;
        if kind == GuestKind::Macos && row.status != VmStatus::Running {
            let running = state.snap.running_macs();
            if model::mac_limit_reason(&running).is_some() {
                return Err(RpcError::from(crate::vm_err!(
                    codes::MAC_LIMIT,
                    "macOS allows at most {} macOS VMs at the same time – running: {}",
                    crate::vz_backend::VzBackend::MAX_RUNNING,
                    running.join(", ")
                )));
            }
        }
        if let Some(d) = lock(&self.inner.doctor).as_ref() {
            if let Some(b) = model::start_blockers(kind, &d.prerequisites).first() {
                return Err(RpcError::from(crate::vm_err!(
                    codes::PREREQUISITE,
                    "{} is missing – see the Doctor",
                    model::requirement_title(b.item)
                )));
            }
        }
        let display = match p.display.as_deref() {
            None | Some("auto") => None,
            Some("none") => Some(DisplayMode::None),
            Some("window") => Some(DisplayMode::Window),
            Some("gpu") => Some(DisplayMode::Gpu),
            Some(other) => {
                return Err(RpcError::invalid_params(format!("display \"{other}\": auto, none, window or gpu")))
            }
        };
        let gl = match p.gl.as_deref() {
            None | Some("on") => GlBackend::On,
            Some("es") => GlBackend::Es,
            Some(other) => return Err(RpcError::invalid_params(format!("gl \"{other}\": on or es"))),
        };
        let has_gl = self.qemu_gl();
        self.act(&p.name, true, |backends, _| {
            let c = backends.store.load(&p.name)?;
            let backend = backends.backend_for(&c)?;
            backend.start(&c, StartOptions { display: model::start_display(&c, has_gl, display), gl })?;
            Ok(())
        })?;
        self.when_reachable(&p.name);
        Answer::of(self.vm(&p.name)?)
    }

    fn vms_stop(&self, p: StopParams) -> Reply {
        lock(&self.inner.stop_requested).insert(p.name.clone());
        let timeout = Duration::from_secs_f64(p.timeout.unwrap_or(60.0).max(0.0));
        let result = self.act(&p.name, true, |backends, _| {
            let c = backends.store.load(&p.name)?;
            backends.backend_for(&c)?.stop(&c, timeout)
        })?;
        self.check_doctor_later();
        Answer::of(StopAnswer { result })
    }

    fn vms_reset_remove(&self, name: &str, remove: bool) -> Reply {
        self.act(name, true, |backends, _| {
            let c = backends.store.load(name)?;
            let backend = backends.backend_for(&c)?;
            if backend.status(&c) != VmStatus::Stopped {
                return Err(vm_err!(codes::VM_RUNNING, "running – stop it first"));
            }
            if remove {
                backend.remove(&c)
            } else {
                backend.reset(&c)
            }
        })?;
        self.check_doctor_later();
        Ok(Answer::Value(Value::Null))
    }

    fn vms_upgrade(&self, p: UpgradeParams) -> Reply {
        let target = match p.image {
            Some(i) => i,
            None => {
                let state = self.state().map_err(err)?;
                let row = state.snap.row(&p.name).ok_or_else(|| {
                    RpcError::from(crate::vm_err!(
                        crate::error::codes::NOT_FOUND,
                        "Instance \"{}\" does not exist",
                        p.name
                    ))
                })?;
                let latest = state.snap.current.get(&row.config.recipe).cloned();
                match latest {
                    Some(l) if l != row.config.image => l,
                    _ => {
                        return Err(RpcError::from(crate::vm_err!(
                            "{} uses the newest image of its recipe already",
                            p.name
                        )))
                    }
                }
            }
        };
        self.act(&p.name, true, |backends, paths| {
            let c = backends.store.load(&p.name)?;
            let backend = backends.backend_for(&c)?;
            if backend.status(&c) != VmStatus::Stopped {
                return Err(vm_err!(codes::VM_RUNNING, "running – stop it first"));
            }
            let image = ImageStore::new(paths.clone()).image(&target)?;
            backend.upgrade(&c, &image)
        })?;
        Answer::of(self.vm(&p.name)?)
    }

    fn vms_set(&self, p: SetParams) -> Reply {
        let running = self.act(&p.name, true, |backends, _| {
            let mut c = backends.store.load(&p.name)?;
            if let Some(v) = p.audio {
                c.audio = Some(v);
            }
            if let Some(v) = p.microphone {
                c.microphone = Some(v);
            }
            if let Some(v) = p.clipboard {
                c.clipboard = Some(v);
            }
            if let Some(v) = p.camera {
                c.camera = Some(v);
            }
            backends.store.save(&c)?;
            Ok(backends.backend_for(&c).is_ok_and(|b| b.status(&c) == VmStatus::Running))
        })?;
        Answer::of(VmSetAnswer { vm: self.vm(&p.name)?, takes_effect_at_next_start: running })
    }

    fn vms_ssh(&self, p: SshParams) -> Reply {
        let paths = self.paths();
        let backends = crate::backend::Backends::new(paths).map_err(err)?;
        let c = backends.store.load(&p.name).map_err(err)?;
        let argv = backends.backend_for(&c).map_err(err)?.ssh_arguments(&c, &p.command);
        let shell_line = crate::ssh_command::SshCommand::shell_line(&argv);
        Answer::of(SshAnswer { argv, shell_line })
    }

    fn vms_agent(&self, p: AgentParams) -> Reply {
        let backends = crate::backend::Backends::new(self.paths()).map_err(err)?;
        let c = backends.store.load(&p.name).map_err(err)?;
        let backend = backends.backend_for(&c).map_err(err)?;
        let deadline = std::time::Instant::now() + Duration::from_secs_f64(p.wait.unwrap_or(0.0).max(0.0));
        loop {
            let state = backend.agent_state(&c);
            if state.as_ref().is_some_and(|s| s.is_ready()) || std::time::Instant::now() >= deadline {
                return Ok(Answer::Value(state.and_then(|s| serde_json::to_value(s).ok()).unwrap_or(Value::Null)));
            }
            std::thread::sleep(Duration::from_millis(500));
        }
    }

    fn vms_log(&self, p: LogParams) -> Reply {
        let paths = self.paths();
        let c = self.store().load(&p.name).map_err(err)?;
        let ip = paths.instance(&p.name);
        let file = match c.kind {
            GuestKind::Macos => ip.runner_log(),
            GuestKind::Nixos if p.qemu => ip.qemu_log(),
            GuestKind::Nixos => ip.serial_log(),
        };
        let text = tail(&file, p.lines.unwrap_or(200));
        let lines: Vec<&str> = if text.is_empty() { vec![] } else { text.split('\n').collect() };
        Answer::of(LogLines { lines: lines.into_iter().map(str::to_string).collect() })
    }

    fn vms_wait(&self, p: WaitParams) -> Reply {
        let backends = crate::backend::Backends::new(self.paths()).map_err(err)?;
        let c = backends.store.load(&p.name).map_err(err)?;
        let backend = backends.backend_for(&c).map_err(err)?;
        if backend.status(&c) != VmStatus::Running {
            return Err(RpcError::from(crate::vm_err!(codes::VM_STOPPED, "{} is not running", p.name)));
        }
        let timeout = Duration::from_secs_f64(p.timeout.unwrap_or(300.0).max(0.0));
        match backend.wait_for_ready(&c, timeout) {
            Some(Readiness::Ssh) => Answer::of(WaitAnswer { via: Via::Ssh }),
            Some(Readiness::Agent(_)) => Answer::of(WaitAnswer { via: Via::Agent }),
            None => Err(RpcError::from(crate::vm_err!("{} did not answer within {} s", p.name, timeout.as_secs()))),
        }
    }

    fn vms_share(&self, name: &str) -> Reply {
        let paths = self.paths();
        let c = self.store().load(name).map_err(err)?;
        let dir = paths.prepare_share(&c, None).map_err(err)?;
        Answer::of(PathAnswer { path: dir.to_string_lossy().into_owned() })
    }

    fn vms_next_name(&self, p: RecipeParam) -> Reply {
        let recipes = self.recipes().unwrap_or_default();
        let taken: HashSet<String> = self.store().list().into_iter().map(|c| c.name).collect();
        Answer::of(NameAnswer { name: model::next_free_name(&p.recipe, recipes.defaults.get(&p.recipe), &taken) })
    }

    fn vms_forget(&self, name: &str) -> Reply {
        self.act(name, false, |backends, _| backends.store.forget(name))?;
        Ok(Answer::Value(Value::Null))
    }

    fn vms_use_external(&self, p: ExternalParams) -> Reply {
        let path = PathBuf::from(platform::expand_tilde(&p.path));
        if !p.confirm_network && Transfer::is_on_network_volume(&path) {
            return Err(RpcError::from(crate::vm_err!("{} is on a network volume. It runs from there, but slower – and it breaks when the connection does. Importing it (a copy into the data folder) avoids that.",
                    file_name(&path)))
            .with_data("networkVolume", json!(true)));
        }
        let name =
            self.act(&file_name(&path), false, |backends, _| Transfer::from_backends(backends).use_external(&path))?;
        Answer::of(self.vm(&name)?)
    }

    // MARK: Images

    fn images_get(&self, name: &str) -> Reply {
        let state = self.state().map_err(err)?;
        let image = state
            .snap
            .images
            .values()
            .flatten()
            .find(|i| i.name == name)
            .ok_or_else(|| RpcError::from(crate::vm_err!(codes::NOT_FOUND, "Image \"{name}\" does not exist")))?;
        Answer::of(state.view().image(image, true))
    }

    /// Deletes a macOS base for real (after the UI's confirmation) – not while a base is installed or a macOS image
    /// built: both use the bases. Images and VMs made from it stay: they are copies (APFS clones).
    fn bases_remove(&self, name: &str) -> Reply {
        let paths = self.paths();
        let busy = self.inner.jobs.list().into_iter().any(|j| {
            j.state.is_active()
                && (j.kind == JobKind::MacosInstall
                    || (j.kind == JobKind::Build && j.guest_kind == Some(GuestKind::Macos)))
        });
        if busy {
            return Err(RpcError::from(vm_err!(
                codes::IN_USE,
                "A macOS base is being installed or a macOS image built – wait until it ends"
            )));
        }
        let Some(base) = MacBaseInstaller::list(&paths).into_iter().find(|b| b.name == name) else {
            return Err(RpcError::from(vm_err!(codes::NOT_FOUND, "No macOS base \"{}\"", name)));
        };
        crate::trash::Disposal::delete(&[&base.path]).map_err(err)?;
        self.events().message_text(Level::Info, super::views::text::tf!("macOS base %@ removed", name));
        self.check_doctor_later();
        self.poke();
        Answer::of(Value::Null)
    }

    /// Assigns a credential to a macOS base created before bases had one (plan B139)
    fn bases_assign(&self, p: AssignParams) -> Reply {
        let paths = self.paths();
        let Some(base) = MacBaseInstaller::list(&paths).into_iter().find(|b| b.name == p.name) else {
            return Err(RpcError::from(vm_err!(codes::NOT_FOUND, "No macOS base \"{}\"", p.name)));
        };
        let record = Credentials::new(paths).assign_to_base(&base.path, &p.credential).map_err(err)?;
        self.events().message_text(
            Level::Info,
            super::views::text::tf!("%@ uses the credential “%@”", p.name, record.credential_name),
        );
        self.check_freshness();
        self.poke();
        Answer::of(Value::Null)
    }

    fn images_remove(&self, name: &str) -> Reply {
        let paths = self.paths();
        let backends = crate::backend::Backends::new(paths.clone()).map_err(err)?;
        ImageStore::new(paths).remove(name, &backends.store).map_err(err)?;
        // Its recipe version may have gone (plan B111)
        self.reload_recipes_later();
        self.poke();
        Ok(Answer::Value(Value::Null))
    }

    /// Queues a build (`AppModel.build`): one per guest kind at a time; a macOS recipe needs the guest user's password
    /// once (sudo in the build VM) – kept in the job's closure until the build uses it
    fn images_build(&self, p: BuildParams) -> Reply {
        if let Some(job) = self.inner.jobs.active(JobKind::Build, Some(&p.recipe)) {
            return Ok(Answer::Job(job));
        }
        let recipes = self.recipes().map_err(err)?;
        let d = recipes.defaults.get(&p.recipe).ok_or_else(|| {
            RpcError::from(crate::vm_err!(
                crate::error::codes::NOT_FOUND,
                "Recipe \"{}\" is not in your recipes",
                p.recipe
            ))
        })?;
        if !d.runs_here() {
            return Err(err(Platform::not_here(&RecipeId::qualified(&p.recipe), &d.systems.join(", "))));
        }
        // What it is made of must be here: the image flake would not find a missing module (plan B142)
        let parts = recipes.parts.get(&p.recipe);
        let missing: Vec<String> = parts
            .map(|x| {
                x.issues
                    .iter()
                    .filter_map(|i| match i {
                        crate::composition::Issue::MissingModule { reference, .. }
                        | crate::composition::Issue::MissingKeymap { reference, .. } => Some(reference.clone()),
                        _ => None,
                    })
                    .collect()
            })
            .unwrap_or_default();
        if !missing.is_empty() {
            return Err(RpcError::from(vm_err!(
                codes::PREREQUISITE,
                "{} is made of parts that are not here: {} – install them first (the recipe's \"Install Missing Parts\", omaf recipes show {} --install)",
                RecipeId::qualified(&p.recipe),
                missing.join(", "),
                RecipeId::qualified(&p.recipe)
            )));
        }
        let kind = d.kind;
        let paths = self.paths();
        // The look: the one chosen, else the recipe's when it is here, else the default (plan B142)
        let look = p.look.clone().filter(|l| !l.is_empty()).or_else(|| parts.and_then(|x| x.look_id.clone()));
        let mut choice = build_choice(&paths, p.credential.as_deref(), look.as_deref(), p.locale.as_deref())?;
        // macOS: the base's credential and its stored password – nobody types one (plan B139)
        let mut password = None;
        if kind == GuestKind::Macos {
            let base = p
                .base
                .clone()
                .filter(|b| !b.is_empty())
                .map(PathBuf::from)
                .or_else(|| MacImageBuilder::newest_base_in(&paths))
                .ok_or_else(|| {
                    RpcError::from(vm_err!(
                        codes::PREREQUISITE,
                        "No base found (expected {}/mac-base-<os-build>) – create one with: omaf macos install",
                        paths.bases().display()
                    ))
                })?;
            let login = Credentials::new(paths.clone()).login_of_base(&base).map_err(err)?;
            choice.credential = Some(login.credential);
            password = Some(login.password);
        }
        let name = RecipeId::display_name(&p.recipe, Some(d));
        self.inner.jobs.drop_ended(JobKind::Build, &p.recipe);
        let recipe = p.recipe.clone();
        let s = self.clone();
        let mac = MacOptions { base: p.base.filter(|b| !b.is_empty()).map(PathBuf::from), keep_failed: p.keep_failed };
        let mut spec = JobSpec::new(
            JobKind::Build,
            format!("Build {name}"),
            Box::new(move |ctx: &JobCtx| s.run_build(ctx, &recipe, kind, &choice, password, mac)),
        );
        spec.recipe = Some(p.recipe.clone());
        spec.guest_kind = Some(kind);
        spec.queue = Some(kind);
        spec.log_file = Some(paths.logs().join(format!("image-build-{}.log", p.recipe)));
        spec.tracker = Some(kind);
        spec.noise = Some(BuildLog::is_noise);
        // Right before the build: e.g. the Linux builder may have stopped since the last check
        let s = self.clone();
        spec.check = Some(Box::new(move |_| {
            let d = s.check_doctor();
            match model::build_blockers(kind, d.all()).first() {
                Some(b) => Err(vm_err!(
                    codes::PREREQUISITE,
                    "Doctor: {} – fix it there first",
                    model::requirement_title(b.item)
                )),
                None => Ok(()),
            }
        }));
        let s = self.clone();
        spec.on_end = Some(Box::new(move |_| {
            // A kept version, older ones gone (B111)
            s.reload_recipes_later();
            s.check_doctor_later();
            s.poke();
        }));
        Ok(Answer::Job(self.inner.jobs.submit(spec)))
    }

    fn run_build(
        &self,
        ctx: &JobCtx,
        recipe: &str,
        kind: GuestKind,
        choice: &BuildChoice,
        password: Option<String>,
        mac: MacOptions,
    ) -> crate::Result<Value> {
        let paths = self.paths();
        let cancel = ctx.cancellation();
        let log = ctx.log_sink();
        let flake = Settings::load(&paths).flake_path()?;
        let image = match kind {
            GuestKind::Macos => {
                ctx.log(&format!("omaf build {recipe}"));
                // The flake as it is now (the recipe list may be older, e.g. after a settings change)
                let user = ImageStore::new(paths.clone())
                    .recipe_defaults()
                    .get(recipe)
                    .and_then(|d| d.user.clone())
                    .unwrap_or_else(platform::user_name);
                let Some(password) = password else {
                    return Err(vm_err!("Password of {user} needed (sudo, once)"));
                };
                let vz = crate::backend::Backends::new(paths.clone())?.vz()?.clone();
                let request = BuildRequest {
                    recipe: recipe.into(),
                    flake: flake.clone(),
                    user,
                    password,
                    base: mac.base,
                    keep_failed: mac.keep_failed,
                    choice: choice.clone(),
                };
                MacImageBuilder::new(paths.clone(), vz).build(&request, cancel, &*log)?
            }
            GuestKind::Nixos => {
                ctx.log(&format!("nix build .#images.{recipe}"));
                ImageStore::new(paths.clone()).build(recipe, &flake, choice, None, cancel, &*log)?
            }
        };
        ctx.log(&format!("✓ Image {} ready", OmaflakeId::image(&image, 3)));
        // Retention: current + one before + those in use (plan B52)
        let used: HashSet<String> = self.store().list().into_iter().map(|c| c.image).collect();
        let removed = ImageStore::new(paths).prune(recipe, &used);
        for name in &removed {
            ctx.log(&format!("Removed older image {name}"));
        }
        result(BuildResult { image: image.name, removed })
    }

    /// `credentials.add` (id None) and `credentials.update`: a field left out stays, "" clears it; `password` "" removes
    /// the stored one
    fn credentials_change(&self, id: Option<String>, p: CredentialParams) -> Reply {
        let change = CredentialChange {
            name: p.name,
            user: p.user,
            full_name: p.full_name,
            email: p.email,
            password: p.password,
        };
        let credentials = Credentials::new(self.paths());
        if let Some(id) = &id {
            // Images and macOS bases carry its values: while used, only the name can change (plan B137, B139)
            let current = credentials.get(id).unwrap_or_default();
            crate::credentials::Usage::of(&self.paths(), id).check_change(&current, &change).map_err(err)?;
        }
        let (c, place) = match id {
            None => credentials.add(change),
            Some(id) => credentials.update(&id, change),
        }
        .map_err(err)?;
        // Another user or name gives other images: the lists say "outdated" at once
        self.check_freshness();
        self.poke();
        let dto = credential_dtos(&self.paths())
            .into_iter()
            .find(|d| d.id == c.id)
            .ok_or_else(|| RpcError::from(vm_err!(codes::NOT_FOUND, "No credential \"{}\"", c.id)))?;
        let note = match place {
            Some(crate::source_tokens::TokenPlace::File(path)) => {
                Some(format!("The password is stored unencrypted in {} – no keyring", path.display()))
            }
            _ => None,
        };
        if let Some(n) = &note {
            self.events().message_text(Level::Warning, super::views::text::Text::verbatim(n));
        }
        Answer::of(CredentialAnswer { credential: dto, note })
    }

    /// Copies a credential's password to the clipboard – after Touch ID or the login password (plan B137); the
    /// clipboard is cleared after 90 s when it still holds it
    fn credentials_copy_password(&self, id: &str) -> Reply {
        let credentials = Credentials::new(self.paths());
        let c =
            credentials.get(id).ok_or_else(|| RpcError::from(vm_err!(codes::NOT_FOUND, "No credential \"{}\"", id)))?;
        let password = credentials
            .password(id)
            .ok_or_else(|| RpcError::from(vm_err!(codes::NOT_FOUND, "No password stored for \"{}\"", c.name)))?;
        if !crate::credentials::confirm_owner(&format!("copy the password of “{}”", c.name)).map_err(err)? {
            return Err(RpcError::from(vm_err!(codes::CANCELLED, "Not copied – not confirmed")));
        }
        crate::credentials::copy_to_clipboard(&password).map_err(err)?;
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(90));
            let paste = if cfg!(target_os = "macos") { "/usr/bin/pbpaste" } else { "wl-paste" };
            let now = std::process::Command::new(paste).output().map(|o| o.stdout).unwrap_or_default();
            if String::from_utf8_lossy(&now).trim_end_matches('\n') == password {
                let _ = crate::credentials::copy_to_clipboard("");
            }
        });
        self.events().message_text(
            Level::Info,
            super::views::text::tf!("Password of “%@” copied – the clipboard is cleared in 90 s", &c.name),
        );
        Answer::of(Value::Null)
    }

    // MARK: Recipes

    fn recipes_list(&self, p: ListParams) -> Reply {
        self.reload_recipes_if_changed();
        self.recipes().map_err(err)?;
        let state = self.state().map_err(err)?;
        Answer::of(state.view().recipes(p.all))
    }

    pub(crate) fn sources(&self) -> Vec<SourceDto> {
        Settings::load(&self.paths()).recipe_sources().iter().map(source_dto).collect()
    }

    /// Fetches the enabled remote sources' catalogs (never in the background – only when asked)
    fn fetch_catalogs(&self) {
        lock(&self.inner.catalogs).fetching = true;
        self.poke();
        let sources = Settings::load(&self.paths()).recipe_sources();
        let remote: Vec<RecipeSource> = sources
            .into_iter()
            .filter(|s| s.is_enabled() && s.kind != crate::recipe_sources::RecipeSourceKind::Folder)
            .collect();
        let results: Vec<(String, crate::Result<crate::recipe_sources::RecipeCatalog>)> = std::thread::scope(|scope| {
            let handles: Vec<_> = remote
                .iter()
                .map(|s| {
                    scope.spawn(move || (s.id(), super::jobs::guarded(|| SourceClient::new(s.clone(), None).catalog())))
                })
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        let mut c = lock(&self.inner.catalogs);
        c.catalogs.clear();
        c.errors.clear();
        for (id, r) in results {
            match r {
                Ok(catalog) => {
                    c.catalogs.insert(id, catalog);
                }
                Err(e) => {
                    c.errors.insert(id, e.message().to_string());
                }
            }
        }
        c.fetched = true;
        c.fetching = false;
        drop(c);
        self.poke();
    }

    fn recipes_available(&self, fetch: bool) -> Result<Available, RpcError> {
        if fetch || !lock(&self.inner.catalogs).fetched {
            self.fetch_catalogs();
        }
        Ok(self.available_from_catalogs())
    }

    /// What the catalogs fetched last offer – None while they were not fetched in this session (the views never fetch)
    pub(crate) fn available_cached(&self) -> Option<Available> {
        // The guard must be gone before `available_from_catalogs` locks the catalogs again (it is not reentrant)
        let fetched = lock(&self.inner.catalogs).fetched;
        fetched.then(|| self.available_from_catalogs())
    }

    fn available_from_catalogs(&self) -> Available {
        let paths = self.paths();
        let settings = Settings::load(&paths);
        let sources = settings.recipe_sources();
        let recipes = self.recipes().unwrap_or_default();
        let language = self.language();
        let c = lock(&self.inner.catalogs);
        let hash = |r: &crate::recipe_sources::CatalogRecipe| r.content_hash();
        let offers = model::offered(&sources, &c.catalogs, &recipes, settings.developer_mode.unwrap_or(false), &hash);
        let suggested = model::suggested(&offers, &|r| r.runs_here());
        let updates = model::recipe_updates(&recipes.installed, &sources, &c.catalogs, &hash);
        let lang = language.as_deref();
        Available {
            offered: offers.iter().map(|(r, s)| catalog_dto(r, s, lang)).collect(),
            updates: updates
                .iter()
                .map(|(id, (r, s))| RecipeUpdate { id: id.clone(), recipe: catalog_dto(r, s, lang) })
                .collect(),
            suggested: suggested.iter().map(|(r, s)| catalog_dto(r, s, lang)).collect(),
            errors: c.errors.clone(),
        }
    }

    /// Installed looks (with their usage and updates) and what the catalogs fetched last offer besides
    pub(crate) fn looks_answer(&self, language: Option<&str>) -> LooksAnswer {
        let paths = self.paths();
        let settings = Settings::load(&paths);
        let store = crate::looks::LookStore::new(paths.clone());
        let (looks, unreadable) = store.list_with_errors(&settings);
        let default = store.default_look(&settings).map(|l| l.id);
        let installed = store.installed();
        let sources = settings.recipe_sources();
        let source_name = |id: &str| {
            sources.iter().find(|s| s.id() == id).map(RecipeSource::display_name).unwrap_or_else(|| id.to_string())
        };
        let official =
            |id: &str, author: &str| author == "omaflake" && sources.iter().any(|s| s.id() == id && s.is_official());
        let images = ImageStore::new(paths.clone()).list();
        let c = lock(&self.inner.catalogs);
        let offered_hash = |l: &crate::looks::Look| {
            c.catalogs.get(&l.source).and_then(|cat| cat.looks.iter().find(|x| x.id == l.id)).map(|x| x.hash.clone())
        };
        let dtos: Vec<LookDto> = looks
            .iter()
            .map(|l| LookDto {
                reference: l.reference(),
                author: l.author.clone(),
                name: l.name.clone(),
                title: l.title.clone(),
                description: l.description_in(language).to_string(),
                tags: l.tags.clone(),
                appearance: l.appearance.clone(),
                plain: l.plain,
                swatches: l.swatches.clone(),
                source_name: source_name(&l.source),
                official: official(&l.source, &l.author),
                installed: installed.contains_key(&l.id),
                update: l.hash.is_some() && offered_hash(l).is_some_and(|h| Some(h) != l.hash),
                is_default: default.as_deref() == Some(l.id.as_str()),
                images: images
                    .iter()
                    .filter(|i| i.info.look.as_deref() == Some(l.id.as_str()))
                    .map(|i| i.name.clone())
                    .collect(),
                source: l.source.clone(),
                id: l.id.clone(),
            })
            .collect();
        let mut offered = Vec::new();
        for s in sources.iter().filter(|s| s.is_enabled()) {
            let Some(cat) = c.catalogs.get(&s.id()) else { continue };
            for l in &cat.looks {
                if dtos.iter().any(|d| d.id == l.id) || offered.iter().any(|o: &OfferedLookDto| o.id == l.id) {
                    continue;
                }
                offered.push(OfferedLookDto {
                    id: l.id.clone(),
                    reference: format!("{}/{}", l.author, l.name),
                    author: l.author.clone(),
                    name: l.name.clone(),
                    title: l.title.clone(),
                    description: l.description_in(language).to_string(),
                    tags: l.tags.clone().unwrap_or_default(),
                    appearance: l.appearance.clone(),
                    plain: l.plain,
                    swatches: l.swatches.clone(),
                    source: s.id(),
                    source_name: s.display_name(),
                    official: l.author == "omaflake" && s.is_official(),
                });
            }
        }
        LooksAnswer { looks: dtos, offered, unreadable, fetched: c.fetched }
    }

    /// The modules and key binding sets here (with the recipes made of them and updates) and what the catalogs fetched
    /// last offer besides (plan B142)
    pub(crate) fn modules_answer(&self, language: Option<&str>) -> ModulesAnswer {
        let paths = self.paths();
        let settings = Settings::load(&paths);
        let sources = settings.recipe_sources();
        let source_name = |id: &str| {
            sources.iter().find(|s| s.id() == id).map(RecipeSource::display_name).unwrap_or_else(|| id.to_string())
        };
        let official =
            |id: &str, author: &str| author == "omaflake" && sources.iter().any(|s| s.id() == id && s.is_official());
        let (modules, mut unreadable) = crate::modules::ModuleStore::new(paths.clone()).list_with_errors(&settings);
        let (keymaps, more) = crate::keymaps::KeymapStore::new(paths.clone()).list_with_errors(&settings);
        unreadable.extend(more);
        let installed_modules = crate::modules::ModuleStore::new(paths.clone()).installed();
        let installed_keymaps = crate::keymaps::KeymapStore::new(paths.clone()).installed();
        // The recipes here made of each (by their names), from what the recipe list read
        let recipes = self.recipes().unwrap_or_default();
        let users = |kind: &str, reference: &str, platform: Option<GuestKind>| -> Vec<String> {
            let mut out: Vec<String> = recipes
                .parts
                .iter()
                .filter(|(id, p)| {
                    let kind_ok = platform.is_none_or(|k| recipes.defaults.get(*id).is_some_and(|d| d.kind == k));
                    let refs: Vec<&String> = if kind == "module" {
                        p.modules.iter().map(|m| &m.reference).collect()
                    } else {
                        p.keymaps.iter().collect()
                    };
                    kind_ok && refs.iter().any(|r| r.split(' ').next() == Some(reference))
                })
                .map(|(id, _)| RecipeId::display_name(id, recipes.defaults.get(id)))
                .collect();
            out.sort();
            out.dedup();
            out
        };
        let c = lock(&self.inner.catalogs);
        let mut parts: Vec<PartDto> = Vec::new();
        for m in &modules {
            let hash =
                c.catalogs.get(&m.source).and_then(|cat| cat.modules.iter().find(|x| x.id == m.id)).map(|x| &x.hash);
            parts.push(PartDto {
                id: m.id.clone(),
                reference: m.reference(),
                kind: "module".into(),
                platform: Some(m.platform),
                title: m.title.clone(),
                description: m.description_in(language).to_string(),
                version: m.version.clone(),
                tags: m.tags.clone(),
                provides: m.provides.clone(),
                needs: m.needs.clone(),
                enhances: m.enhances.clone(),
                keys: m.actions.iter().map(|(a, x)| (a.clone(), x.key.join(" / "))).collect(),
                unfree: m.unfree,
                source: m.source.clone(),
                source_name: source_name(&m.source),
                official: official(&m.source, &m.author),
                installed: installed_modules.contains_key(&m.id),
                update: m.hash.is_some() && hash.is_some_and(|h| Some(h) != m.hash.as_ref()),
                recipes: users("module", &m.reference(), Some(m.platform)),
            });
        }
        for k in &keymaps {
            let hash =
                c.catalogs.get(&k.source).and_then(|cat| cat.keymaps.iter().find(|x| x.id == k.id)).map(|x| &x.hash);
            parts.push(PartDto {
                id: k.id.clone(),
                reference: k.reference(),
                kind: "keymap".into(),
                platform: None,
                title: k.title.clone(),
                description: k.description_in(language).to_string(),
                version: k.version.clone(),
                tags: vec![],
                provides: vec![],
                needs: vec![],
                enhances: vec![],
                keys: keys_of(&k.keys),
                unfree: false,
                source: k.source.clone(),
                source_name: source_name(&k.source),
                official: official(&k.source, &k.author),
                installed: installed_keymaps.contains_key(&k.id),
                update: k.hash.is_some() && hash.is_some_and(|h| Some(h) != k.hash.as_ref()),
                recipes: users("keymap", &k.reference(), None),
            });
        }
        let mut offered: Vec<PartDto> = Vec::new();
        for s in &sources {
            let Some(cat) = c.catalogs.get(&s.id()) else { continue };
            for m in &cat.modules {
                if parts.iter().chain(offered.iter()).any(|p| p.id == m.id) {
                    continue;
                }
                offered.push(PartDto {
                    id: m.id.clone(),
                    reference: m.reference(),
                    kind: "module".into(),
                    platform: Some(m.platform),
                    title: m.title.clone(),
                    description: m.description_in(language).to_string(),
                    version: m.version.clone(),
                    tags: m.tags.clone(),
                    provides: m.provides.clone(),
                    needs: m.needs.clone(),
                    enhances: m.enhances.clone(),
                    keys: BTreeMap::new(),
                    unfree: m.unfree,
                    source: s.id(),
                    source_name: s.display_name(),
                    official: m.author == "omaflake" && s.is_official(),
                    installed: false,
                    update: false,
                    recipes: vec![],
                });
            }
            for k in &cat.keymaps {
                if parts.iter().chain(offered.iter()).any(|p| p.id == k.id) {
                    continue;
                }
                offered.push(PartDto {
                    id: k.id.clone(),
                    reference: format!("{}/{}", k.author, k.name),
                    kind: "keymap".into(),
                    platform: None,
                    title: k.title.clone(),
                    description: k.description_in(language).to_string(),
                    version: k.version.clone(),
                    tags: vec![],
                    provides: vec![],
                    needs: vec![],
                    enhances: vec![],
                    keys: keys_of(&k.keys),
                    unfree: false,
                    source: s.id(),
                    source_name: s.display_name(),
                    official: k.author == "omaflake" && s.is_official(),
                    installed: false,
                    update: false,
                    recipes: vec![],
                });
            }
        }
        ModulesAnswer { parts, offered, unreadable, fetched: c.fetched }
    }

    fn modules_install(&self, p: InstallParams) -> Reply {
        if let Some(job) = self.inner.jobs.active(JobKind::ModuleInstall, Some(&p.id)) {
            return Ok(Answer::Job(job));
        }
        let source =
            Settings::load(&self.paths()).recipe_sources().into_iter().find(|s| s.id() == p.source).ok_or_else(
                || RpcError::from(vm_err!(codes::NOT_FOUND, "Source \"{}\" is not one of yours", p.source)),
            )?;
        let s = self.clone();
        let id = p.id.clone();
        let mut spec = JobSpec::new(
            JobKind::ModuleInstall,
            format!("Install {}", p.id),
            Box::new(move |ctx: &JobCtx| {
                let client = SourceClient::new(source.clone(), None);
                let cached = lock(&s.inner.catalogs).catalogs.get(&source.id()).cloned();
                let catalog = match cached {
                    Some(c) => c,
                    None => client.catalog()?,
                };
                let title = if crate::parts::PartKind::Keymap.parse_id(&id).is_some() {
                    crate::keymaps::KeymapStore::new(s.paths()).install(&id, &catalog, &client)?.title
                } else {
                    crate::modules::ModuleStore::new(s.paths()).install(&id, &catalog, &client)?.title
                };
                ctx.log(&format!("Installed {title} ({id})"));
                result(InstallResult { installed: vec![id.clone()] })
            }),
        );
        spec.recipe = Some(p.id.clone());
        spec.cancellable = false;
        spec.log_file = Some(self.paths().logs().join("module-install.log"));
        let s = self.clone();
        spec.on_end = Some(Box::new(move |_| {
            s.reload_recipes_later();
            s.poke();
        }));
        Ok(Answer::Job(self.inner.jobs.submit(spec)))
    }

    /// The modules and key binding sets the Modules tab offers, one after the other in one job ("Install All")
    fn modules_install_all(&self, p: InstallAllParams) -> Reply {
        if let Some(job) = self.inner.jobs.active(JobKind::ModuleInstall, None) {
            return Ok(Answer::Job(job));
        }
        let sources = Settings::load(&self.paths()).recipe_sources();
        let items: Vec<(String, RecipeSource)> = p
            .items
            .into_iter()
            .filter_map(|i| sources.iter().find(|s| s.id() == i.source).map(|s| (i.id, s.clone())))
            .collect();
        let s = self.clone();
        let mut spec = JobSpec::new(
            JobKind::ModuleInstall,
            "Install modules".to_string(),
            Box::new(move |ctx: &JobCtx| {
                // Each on its own: one that fails does not keep the others out
                let mut installed = Vec::new();
                let mut first_error = None;
                let mut catalogs: BTreeMap<String, crate::recipe_sources::RecipeCatalog> = BTreeMap::new();
                for (id, source) in &items {
                    let client = SourceClient::new(source.clone(), None);
                    let done = (|| -> crate::Result<String> {
                        let catalog = match catalogs.entry(source.id()) {
                            std::collections::btree_map::Entry::Occupied(e) => e.into_mut(),
                            std::collections::btree_map::Entry::Vacant(e) => {
                                let cached = lock(&s.inner.catalogs).catalogs.get(&source.id()).cloned();
                                e.insert(match cached {
                                    Some(c) => c,
                                    None => client.catalog()?,
                                })
                            }
                        };
                        let catalog = &*catalog;
                        Ok(if crate::parts::PartKind::Keymap.parse_id(id).is_some() {
                            crate::keymaps::KeymapStore::new(s.paths()).install(id, catalog, &client)?.title
                        } else {
                            crate::modules::ModuleStore::new(s.paths()).install(id, catalog, &client)?.title
                        })
                    })();
                    match done {
                        Ok(title) => {
                            ctx.log(&format!("Installed {title} ({id})"));
                            installed.push(id.clone());
                        }
                        Err(e) => {
                            ctx.log(&format!("{id}: {}", e.message()));
                            first_error.get_or_insert(e);
                        }
                    }
                }
                match first_error {
                    Some(e) if installed.is_empty() => Err(e),
                    _ => result(InstallResult { installed }),
                }
            }),
        );
        spec.cancellable = false;
        spec.log_file = Some(self.paths().logs().join("module-install.log"));
        let s = self.clone();
        spec.on_end = Some(Box::new(move |_| {
            s.reload_recipes_later();
            s.poke();
        }));
        Ok(Answer::Job(self.inner.jobs.submit(spec)))
    }

    /// Removes an installed module or key binding set; recipes made of it cannot be built until it is back
    fn modules_remove(&self, id: &str) -> Reply {
        if crate::parts::PartKind::Keymap.parse_id(id).is_some() {
            crate::keymaps::KeymapStore::new(self.paths()).remove(id).map_err(err)?;
        } else {
            crate::modules::ModuleStore::new(self.paths()).remove(id).map_err(err)?;
        }
        self.events().message_text(Level::Info, super::views::text::tf!("%@ removed", id));
        self.reload_recipes_later();
        self.poke();
        Answer::of(Value::Null)
    }

    fn looks_install(&self, p: InstallParams) -> Reply {
        let source =
            Settings::load(&self.paths()).recipe_sources().into_iter().find(|s| s.id() == p.source).ok_or_else(
                || RpcError::from(vm_err!(codes::NOT_FOUND, "Source \"{}\" is not one of yours", p.source)),
            )?;
        Ok(Answer::Job(self.inner.jobs.submit(self.look_install_spec(vec![p.id], source))))
    }

    /// The looks the Looks tab offers ("Install All"): one job per source
    fn looks_install_all(&self, p: InstallAllParams) -> Reply {
        if let Some(job) = self.inner.jobs.active(JobKind::LookInstall, None) {
            return Ok(Answer::Job(job));
        }
        let mut by_source: Vec<(RecipeSource, Vec<String>)> = Vec::new();
        for source in Settings::load(&self.paths()).recipe_sources() {
            let ids: Vec<String> = p.items.iter().filter(|i| i.source == source.id()).map(|i| i.id.clone()).collect();
            if !ids.is_empty() {
                by_source.push((source, ids));
            }
        }
        let jobs =
            by_source.into_iter().map(|(s, ids)| self.inner.jobs.submit(self.look_install_spec(ids, s))).collect();
        Ok(Answer::Jobs(jobs))
    }

    /// What the Doctor offers without a look: omaflake's own and Catppuccin from the official catalog (plan B141)
    fn looks_install_recommended(&self) -> Reply {
        if let Some(job) = self.inner.jobs.active(JobKind::LookInstall, None) {
            return Ok(Answer::Job(job));
        }
        let ids = crate::looks::LookStore::RECOMMENDED.iter().map(|s| s.to_string()).collect();
        Ok(Answer::Job(self.inner.jobs.submit(self.look_install_spec(ids, RecipeSource::official()))))
    }

    fn look_install_spec(&self, ids: Vec<String>, source: RecipeSource) -> JobSpec {
        let s = self.clone();
        let title = match ids.as_slice() {
            [one] => format!("Install {one}"),
            _ => "Install looks".to_string(),
        };
        let mut spec = JobSpec::new(
            JobKind::LookInstall,
            title,
            Box::new(move |ctx: &JobCtx| {
                let client = SourceClient::new(source.clone(), None);
                let cached = lock(&s.inner.catalogs).catalogs.get(&source.id()).cloned();
                let catalog = match cached {
                    Some(c) => c,
                    None => client.catalog()?,
                };
                let store = crate::looks::LookStore::new(s.paths());
                // Each on its own: one that is missing in the catalog does not keep the others out
                let mut installed = Vec::new();
                let mut first_error = None;
                for id in &ids {
                    match store.install(id, &catalog, &client) {
                        Ok(look) => {
                            ctx.log(&format!("Installed the look {}", look.title));
                            installed.push(look.id);
                        }
                        Err(e) => {
                            ctx.log(&format!("{id}: {}", e.message()));
                            first_error.get_or_insert(e);
                        }
                    }
                }
                match first_error {
                    Some(e) if installed.is_empty() => Err(e),
                    _ => result(InstallResult { installed }),
                }
            }),
        );
        spec.cancellable = false;
        spec.log_file = Some(self.paths().logs().join("look-install.log"));
        let s = self.clone();
        spec.on_end = Some(Box::new(move |_| {
            s.check_freshness();
            s.check_doctor_later();
            s.poke();
        }));
        spec
    }

    /// Removes an installed look – not while images are built with it (they could not be built again)
    fn looks_remove(&self, id: &str) -> Reply {
        let used =
            ImageStore::new(self.paths()).list().into_iter().filter(|i| i.info.look.as_deref() == Some(id)).count();
        if used > 0 {
            return Err(RpcError::from(vm_err!(codes::IN_USE, "Images are built with this look – remove them first")));
        }
        crate::looks::LookStore::new(self.paths()).remove(id).map_err(err)?;
        self.events().message_text(Level::Info, super::views::text::tf!("Look %@ removed", id));
        self.check_doctor_later();
        self.poke();
        Answer::of(Value::Null)
    }

    fn recipes_install(&self, p: InstallParams) -> Reply {
        if let Some(job) = self.inner.jobs.active(JobKind::RecipeInstall, Some(&p.id)) {
            return Ok(Answer::Job(job));
        }
        let source =
            Settings::load(&self.paths()).recipe_sources().into_iter().find(|s| s.id() == p.source).ok_or_else(
                || {
                    RpcError::from(crate::vm_err!(
                        crate::error::codes::NOT_FOUND,
                        "Source \"{}\" is not one of yours",
                        p.source
                    ))
                },
            )?;
        Ok(Answer::Job(self.inner.jobs.submit(self.install_spec(p.id, source))))
    }

    fn install_spec(&self, id: String, source: RecipeSource) -> JobSpec {
        let s = self.clone();
        let recipe = id.clone();
        let mut spec = JobSpec::new(
            JobKind::RecipeInstall,
            format!("Install {}", RecipeId::qualified(&id)),
            Box::new(move |ctx: &JobCtx| {
                let client = SourceClient::new(source.clone(), None);
                let cached = lock(&s.inner.catalogs).catalogs.get(&source.id()).cloned();
                let catalog = match cached {
                    Some(c) => c,
                    None => client.catalog()?,
                };
                let library = RecipeLibrary::new(s.paths());
                let written = library.install(&recipe, &catalog, &client)?;
                for w in &written {
                    ctx.log(&format!("Installed {}", RecipeId::qualified(w)));
                }
                // What it is made of comes along: its modules, key bindings and look (plan B142)
                let folder = written.first().and_then(|w| library.folder(w)).or_else(|| {
                    let settings = Settings::load(&s.paths());
                    let base = RecipeId::base(&recipe).to_string();
                    library.available(&settings).into_iter().find(|a| RecipeId::base(&a.id) == base).map(|a| a.folder)
                });
                if let Some(spec) = folder.and_then(|f| crate::composition::RecipeSpec::read(&f).ok()) {
                    s.install_parts(&spec, ctx)?;
                }
                result(InstallResult { installed: written })
            }),
        );
        spec.recipe = Some(id.clone());
        spec.cancellable = false;
        spec.log_file = Some(self.paths().logs().join(format!("recipe-install-{}.log", file_safe(&id))));
        let s = self.clone();
        spec.on_end = Some(Box::new(move |_| s.reload_recipes_later()));
        spec
    }

    /// Installs what a recipe here is made of and is missing – its modules, key bindings and look (the build page's
    /// "Install Missing Parts")
    fn recipes_install_parts(&self, id: &str) -> Reply {
        if let Some(job) = self.inner.jobs.active(JobKind::RecipeInstall, Some(id)) {
            return Ok(Answer::Job(job));
        }
        let folder = self
            .recipes()
            .ok()
            .and_then(|r| r.folders.get(id).cloned())
            .ok_or_else(|| RpcError::from(vm_err!(codes::NOT_FOUND, "{} is not here", RecipeId::qualified(id))))?;
        let spec = crate::composition::RecipeSpec::read(&folder).map_err(err)?;
        let s = self.clone();
        let mut spec_job = JobSpec::new(
            JobKind::RecipeInstall,
            format!("Install what {} is made of", RecipeId::qualified(id)),
            Box::new(move |ctx: &JobCtx| result(InstallResult { installed: s.install_parts(&spec, ctx)? })),
        );
        spec_job.recipe = Some(id.to_string());
        spec_job.cancellable = false;
        spec_job.log_file = Some(self.paths().logs().join(format!("recipe-install-{}.log", file_safe(id))));
        let s = self.clone();
        spec_job.on_end = Some(Box::new(move |_| {
            s.check_doctor_later();
            s.reload_recipes_later();
        }));
        Ok(Answer::Job(self.inner.jobs.submit(spec_job)))
    }

    /// The remote sources with their catalogs for installing parts, in their order: the catalog fetched last, or fetched
    /// now; the official catalog last when it is not one of them – a reference always falls back to it (plan B141)
    fn part_catalogs(&self) -> Vec<(RecipeSource, crate::recipe_sources::RecipeCatalog)> {
        let mut sources: Vec<RecipeSource> = Settings::load(&self.paths())
            .recipe_sources()
            .into_iter()
            .filter(|s| s.is_enabled() && s.kind != crate::recipe_sources::RecipeSourceKind::Folder)
            .collect();
        if !sources.iter().any(RecipeSource::is_official) {
            sources.push(RecipeSource::official());
        }
        sources
            .into_iter()
            .filter_map(|source| {
                let cached = lock(&self.inner.catalogs).catalogs.get(&source.id()).cloned();
                let catalog = match cached {
                    Some(c) => c,
                    None => SourceClient::new(source.clone(), None).catalog().ok()?,
                };
                Some((source, catalog))
            })
            .collect()
    }

    /// Installs the parts `spec` needs that are not here from the sources' catalogs (`parts::install_recipe_parts`)
    pub(crate) fn install_parts(
        &self,
        spec: &crate::composition::RecipeSpec,
        ctx: &JobCtx,
    ) -> crate::Result<Vec<String>> {
        crate::parts::install_recipe_parts(&self.paths(), spec, &self.part_catalogs(), &mut |line| ctx.log(line))
    }

    fn recipes_install_suggested(&self) -> Reply {
        let available = self.recipes_available(false)?;
        let sources = Settings::load(&self.paths()).recipe_sources();
        let mut jobs = Vec::new();
        for r in available.suggested {
            if let Some(job) = self.inner.jobs.active(JobKind::RecipeInstall, Some(&r.id)) {
                jobs.push(job);
            } else if let Some(source) = sources.iter().find(|s| s.id() == r.source) {
                jobs.push(self.inner.jobs.submit(self.install_spec(r.id.clone(), source.clone())));
            }
        }
        Ok(Answer::Jobs(jobs))
    }

    /// The recipes the Recipes tab offers ("Install All"), each with what it is made of
    fn recipes_install_all(&self, p: InstallAllParams) -> Reply {
        let sources = Settings::load(&self.paths()).recipe_sources();
        let mut jobs = Vec::new();
        for i in p.items {
            if let Some(job) = self.inner.jobs.active(JobKind::RecipeInstall, Some(&i.id)) {
                jobs.push(job);
            } else if let Some(source) = sources.iter().find(|s| s.id() == i.source) {
                jobs.push(self.inner.jobs.submit(self.install_spec(i.id, source.clone())));
            }
        }
        Ok(Answer::Jobs(jobs))
    }

    /// Every newer version the sources have of downloaded recipes (the Recipes tab's "Update All")
    fn recipes_install_updates(&self) -> Reply {
        let available = self.recipes_available(false)?;
        let sources = Settings::load(&self.paths()).recipe_sources();
        let mut jobs = Vec::new();
        for u in available.updates {
            if let Some(job) = self.inner.jobs.active(JobKind::RecipeInstall, Some(&u.recipe.id)) {
                jobs.push(job);
            } else if let Some(source) = sources.iter().find(|s| s.id() == u.recipe.source) {
                jobs.push(self.inner.jobs.submit(self.install_spec(u.recipe.id.clone(), source.clone())));
            }
        }
        Ok(Answer::Jobs(jobs))
    }

    fn recipes_remove(&self, id: &str) -> Reply {
        RecipeLibrary::new(self.paths()).remove(id).map_err(err)?;
        self.reload_recipes().map_err(err)?;
        Ok(Answer::Value(Value::Null))
    }

    fn recipes_remove_line(&self, p: LineParams) -> Reply {
        let (removed, kept) = RecipeLibrary::new(self.paths()).remove_line(&p.line);
        let _ = self.reload_recipes();
        Answer::of(LineRemoval { removed, kept })
    }

    // MARK: Sources (changed in Settings; the recipe list follows)

    fn save_sources(&self, list: Vec<RecipeSource>) -> Result<(), RpcError> {
        let paths = self.paths();
        let mut s = Settings::load(&paths);
        s.sources = Some(list);
        s.save(&paths).map_err(err)?;
        lock(&self.inner.catalogs).fetched = false;
        self.reload_recipes_later();
        Ok(())
    }

    fn source_list(&self) -> Vec<RecipeSource> {
        Settings::load(&self.paths()).recipe_sources()
    }

    fn sources_add(&self, p: AddSourceParams) -> Reply {
        let mut s = RecipeSource::parse(&p.input, p.private).map_err(err)?;
        if let Some(branch) = p.branch.filter(|b| !b.is_empty()) {
            s.branch = Some(branch);
        }
        let mut list = self.source_list();
        if list.iter().any(|x| x.id() == s.id()) {
            return Err(RpcError::from(crate::vm_err!(codes::EXISTS, "{} is already a source", s.display_name())));
        }
        let token = if s.is_private_repository() {
            Some(
                p.token
                    .filter(|t| !t.is_empty())
                    .ok_or_else(|| RpcError::invalid_params("A private repository needs a token"))?,
            )
        } else {
            None
        };
        let mut note = SourceClient::new(s.clone(), token.clone()).verify().map_err(err)?;
        if let Some(token) = &token {
            // Without a keyring (Linux server, SSH session) the token is in a 0600 file – the note says so
            if let Some(stored) = SourceTokens::set(Some(token), &s).map_err(err)?.and_then(|p| p.note()) {
                note = format!("{note} – {stored}");
            }
        }
        if p.first {
            list.insert(0, s.clone());
        } else {
            list.push(s.clone());
        }
        self.save_sources(list)?;
        Answer::of(SourceAdded { note: format!("{}: {note}", s.display_name()), source: source_dto(&s) })
    }

    fn find_source(&self, id: &str) -> Result<(Vec<RecipeSource>, usize), RpcError> {
        let list = self.source_list();
        let i = list
            .iter()
            .position(|s| s.id() == id)
            .ok_or_else(|| RpcError::from(crate::vm_err!(codes::NOT_FOUND, "Source \"{id}\" is not one of yours")))?;
        Ok((list, i))
    }

    fn sources_remove(&self, id: &str) -> Reply {
        let (mut list, i) = self.find_source(id)?;
        let _ = SourceTokens::set(None, &list[i]);
        list.remove(i);
        self.save_sources(list)?;
        Ok(Answer::Value(Value::Null))
    }

    fn sources_set_enabled(&self, p: EnableParams) -> Reply {
        let (mut list, i) = self.find_source(&p.id)?;
        list[i].enabled = if p.enabled { None } else { Some(false) };
        self.save_sources(list)?;
        Answer::of(self.sources())
    }

    fn sources_move(&self, p: MoveParams) -> Reply {
        let (mut list, i) = self.find_source(&p.id)?;
        let j = if p.up { i.checked_sub(1) } else { Some(i + 1) };
        if let Some(j) = j.filter(|j| *j < list.len()) {
            list.swap(i, j);
            self.save_sources(list)?;
        }
        Answer::of(self.sources())
    }

    fn sources_check(&self) -> Reply {
        let list = self.source_list();
        let results: BTreeMap<String, SourceCheck> = std::thread::scope(|scope| {
            let handles: Vec<_> = list
                .iter()
                .map(|s| {
                    scope.spawn(move || (s.id(), super::jobs::guarded(|| SourceClient::new(s.clone(), None).verify())))
                })
                .collect();
            handles
                .into_iter()
                .filter_map(|h| h.join().ok())
                .map(|(id, r)| match r {
                    Ok(note) => (id, SourceCheck { ok: true, note }),
                    Err(e) => (id, SourceCheck { ok: false, note: e.message().to_string() }),
                })
                .collect()
        });
        Answer::of(SourceChecks { results })
    }

    // MARK: Packages (plan B108, B109)

    /// What an open panel checks before it closes, and what an import would ask about (`PackageChecker`,
    /// `AppModel.importPackage`)
    fn packages_inspect(&self, p: InspectParams) -> Result<Inspection, RpcError> {
        self.packages_inspect_for(&p.path, p.expect, p.external)
    }

    pub(crate) fn packages_inspect_for(
        &self,
        path: &str,
        expect: Option<PackageKindDto>,
        external: bool,
    ) -> Result<Inspection, RpcError> {
        let p = InspectParams { path: path.to_string(), expect, external };
        let path = PathBuf::from(platform::expand_tilde(&p.path));
        let kind = package_kind(Transfer::kind_of(&path));
        let expect = p.expect.unwrap_or(kind);
        let paths = self.paths();
        let check = || -> crate::Result<()> {
            match expect {
                PackageKindDto::Vm => {
                    if Package::kind_of(&path) != Some(PackageKind::Vm) {
                        return Err(vm_err!("Not a VM package (.omaflake-vm)"));
                    }
                    let c = Transfer::read_config(&path)?;
                    let disk = path.join(Transfer::EMBEDDED_IMAGE_NAME).join("disk.qcow2");
                    if p.external && c.kind == GuestKind::Nixos && !disk.exists() {
                        return Err(vm_err!(
                            "{} does not carry its image – export it again to run it from there",
                            c.name
                        ));
                    }
                    Ok(())
                }
                PackageKindDto::Image => ImageStore::new(paths.clone()).image_at(&path).map(|_| ()),
                PackageKindDto::Recipe => RecipeLibrary::new(paths.clone()).recipes_in(&path).map(|_| ()),
            }
        };
        let reason = super::jobs::guarded(check).err().map(|e| super::views::Text::verbatim(e.message()));
        let conflict = if reason.is_none() && !p.external {
            let c =
                super::jobs::guarded(|| Transfer::new(paths.clone(), "qemu-img", None).conflict(&path)).map_err(err)?;
            match c {
                Conflict::None => None,
                Conflict::VmName(existing) => {
                    let taken: HashSet<String> = self.store().list().into_iter().map(|c| c.name).collect();
                    Some(ConflictDto {
                        suggested_name: Some(model::next_numbered(&existing, &taken)),
                        vm_name: Some(existing),
                        image: None,
                    })
                }
                Conflict::Image(existing) => {
                    Some(ConflictDto { vm_name: None, suggested_name: None, image: Some(existing) })
                }
            }
        } else {
            None
        };
        Ok(Inspection {
            kind,
            valid: reason.is_none(),
            reason,
            conflict,
            network_volume: Transfer::is_on_network_volume(&path),
        })
    }

    fn packages_import(&self, p: ImportParams) -> Reply {
        let path = PathBuf::from(platform::expand_tilde(&p.path));
        let kind = package_kind(Transfer::kind_of(&path));
        let s = self.clone();
        let name = p.name.clone().map(|n| n.trim().to_string()).filter(|n| !n.is_empty());
        let replace = p.replace;
        let what = file_name(&path);
        let log_name = what.clone();
        let mut spec = JobSpec::new(
            JobKind::Import,
            format!("Import {what}"),
            Box::new(move |ctx: &JobCtx| {
                let log = ctx.log_sink();
                s.act(&what, false, |backends, _| {
                    let r = Transfer::from_backends(backends).import_package(&path, name.as_deref(), replace, &*log)?;
                    result(ImportResult { kind: package_kind(r.kind), names: r.names, notes: r.notes })
                })
                .map_err(Error::from)
            }),
        );
        spec.cancellable = false;
        spec.log_file = Some(self.paths().logs().join(format!("import-{}.log", file_safe(&log_name))));
        let s = self.clone();
        spec.on_end = Some(Box::new(move |job| {
            if job.state == JobState::Finished && kind == PackageKindDto::Recipe {
                s.reload_recipes_later();
            }
            s.poke();
        }));
        Ok(Answer::Job(self.inner.jobs.submit(spec)))
    }

    fn packages_export(&self, p: ExportParams) -> Reply {
        let folder = PathBuf::from(platform::expand_tilde(&p.folder));
        let s = self.clone();
        let (kind, name, with_image) = (p.kind, p.name.clone(), p.with_image);
        let mut spec = JobSpec::new(
            JobKind::Export,
            format!("Export {}", p.name),
            Box::new(move |ctx: &JobCtx| {
                let log = ctx.log_sink();
                let dest = match kind {
                    PackageKindDto::Vm => s.act(&name, true, |backends, _| {
                        let c = backends.store.load(&name)?;
                        let with = with_image.unwrap_or_else(|| Transfer::carries_image(&c, true));
                        Transfer::from_backends(backends).export_vm(&name, &folder, with, &*log)
                    }),
                    PackageKindDto::Image => s.act(&name, false, |backends, _| {
                        Transfer::from_backends(backends).export_image(&name, &folder, &*log)
                    }),
                    PackageKindDto::Recipe => s.act(&name, false, |_, paths| {
                        RecipeLibrary::new(paths.clone()).export_recipe(&name, &folder, &Settings::load(paths))
                    }),
                }
                .map_err(Error::from)?;
                result(ExportResult { path: dest.to_string_lossy().into_owned() })
            }),
        );
        spec.cancellable = false;
        spec.log_file = Some(self.paths().logs().join(format!("export-{}.log", file_safe(&p.name))));
        Ok(Answer::Job(self.inner.jobs.submit(spec)))
    }

    // MARK: Doctor, macOS base, Linux builder

    fn doctor_set_path(&self, p: SetPathParams) -> Reply {
        let paths = self.paths();
        let mut s = Settings::load(&paths);
        let value = Some(platform::expand_tilde(&p.path));
        match p.item.as_str() {
            "nix" => s.nix = value,
            "qemu" => s.qemu = value,
            "flake" => s.flake = value,
            other => return Err(RpcError::invalid_params(format!("item \"{other}\": nix, qemu or flake"))),
        }
        s.save(&paths).map_err(err)?;
        Answer::of(model::doctor_report(&self.check_doctor()))
    }

    fn macos_download_ipsw(&self) -> Reply {
        if let Some(job) = self.inner.jobs.any_active(&[JobKind::IpswDownload, JobKind::MacosInstall]) {
            return Ok(Answer::Job(job));
        }
        let s = self.clone();
        let mut spec = JobSpec::new(
            JobKind::IpswDownload,
            "Download macOS",
            Box::new(move |ctx: &JobCtx| {
                let paths = s.paths();
                let vz = crate::backend::Backends::new(paths.clone())?.vz()?.clone();
                let installer = MacBaseInstaller::new(paths, vz);
                let cached = lock(&s.inner.doctor).as_ref().and_then(|d| d.latest.clone());
                let release: MacOsRelease = match cached.or_else(|| installer.latest_release()) {
                    Some(r) => r,
                    None => return Err(vm_err!(codes::NETWORK, "No macOS release found")),
                };
                ctx.log(&format!("macOS {} ({})", release.version, release.build));
                let progress = ctx.progress_sink(&format!("Downloading macOS {}", release.version));
                let log = ctx.log_sink();
                let job = ctx.job.clone();
                // Cancel pauses: the download continues from there next time
                let file = installer.download_ipsw(
                    &release,
                    MacBaseInstaller::DOWNLOAD_RESERVE,
                    &|d| {
                        let d = d.clone();
                        JobCtx { job: job.clone() }.on_cancel(move || d.cancel());
                    },
                    &*progress,
                    &*log,
                )?;
                result(IpswResult { file: file.to_string_lossy().into_owned(), version: release.version })
            }),
        );
        spec.guest_kind = Some(GuestKind::Macos);
        let s = self.clone();
        spec.log_file = Some(self.paths().logs().join("ipsw-download.log"));
        spec.on_end = Some(Box::new(move |_| s.check_doctor_later()));
        Ok(Answer::Job(self.inner.jobs.submit(spec)))
    }

    /// `omaf macos install` as a job: download/install, clone, first boot (blocks until it is shut down)
    fn macos_install_base(&self, p: InstallBaseParams) -> Reply {
        if let Some(job) = self.inner.jobs.any_active(&[JobKind::MacosInstall, JobKind::IpswDownload]) {
            return Err(RpcError::from(crate::vm_err!(codes::IN_USE, "{} runs already", job.title)));
        }
        let s = self.clone();
        // The guest user comes from a credential, its password from the secret store (plan B139); user, full name and
        // password given directly still work (API version 1)
        let login = Credentials::new(self.paths()).base_login(p.credential.as_deref()).map_err(err)?;
        let record = Some(crate::mac_base_installer::BaseRecord {
            credential: login.credential.id.clone(),
            credential_name: login.credential.name.clone(),
            user: login.user.clone(),
        });
        let (user, full_name, password) = (login.user, login.full_name, login.password);
        let ipsw = p.ipsw;
        let mut spec = JobSpec::new(
            JobKind::MacosInstall,
            format!("Create the macOS base (user {user})"),
            Box::new(move |ctx: &JobCtx| {
                ctx.log(&format!("omaf macos install (user {user})"));
                let paths = s.paths();
                let vz = crate::backend::Backends::new(paths.clone())?.vz()?.clone();
                let installer = MacBaseInstaller::new(paths, vz);
                let mut request = BaseRequest::new(&user, &full_name, Some(password));
                request.ipsw = ipsw.map(|i| PathBuf::from(platform::expand_tilde(&i)));
                let progress = ctx.progress_sink("Downloading macOS");
                let log = ctx.log_sink();
                let job = ctx.job.clone();
                let outcome = installer.install(
                    &request,
                    &|d| {
                        let d = d.clone();
                        JobCtx { job: job.clone() }.on_cancel(move || d.cancel());
                    },
                    &*progress,
                    &*log,
                )?;
                result(match outcome {
                    BaseOutcome::Existing(dir) => {
                        ctx.log(&format!("✓ {} exists already", file_name(&dir)));
                        BaseResult { base: dir.to_string_lossy().into_owned(), existing: true, ready: true }
                    }
                    BaseOutcome::Created { dir, ready } => {
                        if let (Some(r), true) = (&record, ready) {
                            r.write(&dir)?;
                        }
                        ctx.log(&format!("✓ {} is ready – build a macOS image next.", file_name(&dir)));
                        BaseResult { base: dir.to_string_lossy().into_owned(), existing: false, ready }
                    }
                })
            }),
        );
        spec.guest_kind = Some(GuestKind::Macos);
        let s = self.clone();
        spec.log_file = Some(self.paths().logs().join("macos-install.log"));
        spec.on_end = Some(Box::new(move |_| {
            s.poke();
            s.check_doctor_later();
        }));
        Ok(Answer::Job(self.inner.jobs.submit(spec)))
    }

    /// The Linux builder's maintenance (`AppModel.installNix`, `setUpNixDarwin`, `applyBuilderSettings`,
    /// `cleanUpBuilder`): each asks for an administrator through the system's own dialog; one at a time
    fn builder_job(&self, kind: JobKind, force: bool) -> Reply {
        const KINDS: [JobKind; 5] = [
            JobKind::BuilderInstallNix,
            JobKind::BuilderMountNix,
            JobKind::BuilderSetup,
            JobKind::BuilderApply,
            JobKind::BuilderCleanUp,
        ];
        if let Some(job) = self.inner.jobs.any_active(&KINDS) {
            return Err(RpcError::from(crate::vm_err!(codes::IN_USE, "{} runs already", job.title)));
        }
        let maintenance = BuilderMaintenance::new(self.paths());
        if kind == JobKind::BuilderApply && !force && !maintenance.system_is_omaflakes() {
            return Err(RpcError::from(crate::vm_err!(codes::PREREQUISITE, "This Mac's nix-darwin system does not come from omaflake's image flake yet. Applying replaces your current nix-darwin configuration with omaflake's Linux builder host (nix/hosts/linux-builder.nix and your local/darwin.nix). Only continue if you set up nix-darwin for omaflake."))
            .with_data("replacesConfiguration", json!(true)));
        }
        let title = match kind {
            JobKind::BuilderInstallNix => "Install Nix",
            JobKind::BuilderMountNix => "Mount the Nix Store",
            JobKind::BuilderSetup => "Set up nix-darwin and the Linux builder",
            JobKind::BuilderApply => "Apply the Linux builder settings",
            _ => "Clean up the Linux builder",
        };
        let s = self.clone();
        let mut spec = JobSpec::new(
            kind,
            title,
            Box::new(move |ctx: &JobCtx| {
                let m = BuilderMaintenance::new(s.paths());
                let log = ctx.log_sink();
                let admin = |command: String, prompt: &str| -> crate::Result<String> {
                    AdminShell::run(&m.root_script(&command)?, prompt)
                };
                let apply = |ctx: &JobCtx| -> crate::Result<()> {
                    // Build as the user (root may not read the flake in ~/Documents), activate as root
                    ctx.log("Building Linux builder settings …");
                    let system = m.build_system("linux-builder", &*log)?;
                    ctx.log("Applying Linux builder settings …");
                    admin(
                        BuilderMaintenance::activate_command(&system),
                        "omaflake wants to apply the Linux builder settings (darwin-rebuild).",
                    )?;
                    s.wait_for_builder(ctx);
                    Ok(())
                };
                let done = match kind {
                    JobKind::BuilderInstallNix => {
                        ctx.log("Installing Nix …");
                        admin(BuilderMaintenance::install_nix_command(), "omaflake wants to install Nix.")?;
                        "Nix installed".to_string()
                    }
                    JobKind::BuilderMountNix => {
                        let Some(volume) = NixVolume::find() else {
                            return Err(crate::vm_err!(codes::NOT_FOUND, "No Nix Store volume on this Mac"));
                        };
                        ctx.log(&format!("Mounting the Nix Store at /nix ({}) …", volume.state()));
                        admin(volume.repair_command(), "omaflake wants to mount the Nix Store at /nix.")?;
                        "Nix Store mounted at /nix".to_string()
                    }
                    JobKind::BuilderSetup => {
                        ctx.log("Building nix-darwin (stage 1 of 2) …");
                        let first = m.build_system("linux-builder-bootstrap", &*log)?;
                        ctx.log("Activating nix-darwin (stage 1 of 2) …");
                        admin(
                            BuilderMaintenance::first_activation_command(&first),
                            "omaflake wants to set up nix-darwin and the Linux builder.",
                        )?;
                        s.wait_for_builder(ctx);
                        ctx.log("nix-darwin set up – applying omaflake's builder settings next");
                        apply(ctx)?;
                        "Linux builder settings applied".to_string()
                    }
                    JobKind::BuilderApply => {
                        apply(ctx)?;
                        "Linux builder settings applied".to_string()
                    }
                    _ => {
                        ctx.log("Cleaning up the Linux builder …");
                        let output = admin(
                            BuilderMaintenance::CLEAN_UP_COMMAND.to_string(),
                            "omaflake wants to clean up the Linux builder's disk.",
                        )?;
                        for line in output.lines() {
                            ctx.log(line);
                        }
                        cleanup_summary(&output)
                    }
                };
                ctx.log(&done);
                result(MessageResult { message: done })
            }),
        );
        spec.cancellable = false;
        let s = self.clone();
        // The admin scripts write logs/linux-builder.log themselves; the job's own lines go next to it
        spec.log_file = Some(self.paths().logs().join(format!("{}.log", job_kind_name(kind))));
        spec.on_end = Some(Box::new(move |_| s.check_doctor_later()));
        Ok(Answer::Job(self.inner.jobs.submit(spec)))
    }

    /// Install or uninstall the `omaf` command (plan B122): the link to this omaflake's `omaf`; on macOS through the
    /// administrator dialog (a dismissed dialog cancels the job), on Linux in ~/.local/bin without root
    fn cli_job(&self, install: bool) -> Reply {
        if let Some(job) = self.inner.jobs.any_active(&[JobKind::CliInstall]) {
            return Err(RpcError::from(crate::vm_err!(codes::IN_USE, "{} runs already", job.title)));
        }
        let title = if install { "Install the omaf command" } else { "Uninstall the omaf command" };
        let mut spec = JobSpec::new(
            JobKind::CliInstall,
            title,
            Box::new(move |ctx: &JobCtx| {
                let link = CliLink::current();
                let prompt = if install {
                    "omaflake wants to install the omaf command in /usr/local/bin."
                } else {
                    "omaflake wants to remove the omaf command from /usr/local/bin."
                };
                let as_root = |script: &str| AdminShell::run(script, prompt).map(|_| ());
                let message = if install { link.install(&as_root)? } else { link.uninstall(&as_root)? };
                ctx.log(&message);
                result(CliResult {
                    link: link.link.to_string_lossy().into_owned(),
                    installed: link.state().installed(),
                    message,
                })
            }),
        );
        spec.cancellable = false;
        let s = self.clone();
        spec.log_file = Some(self.paths().logs().join("cli-install.log"));
        spec.on_end = Some(Box::new(move |_| s.check_doctor_later()));
        Ok(Answer::Job(self.inner.jobs.submit(spec)))
    }

    /// Activation restarts the builder VM: up to two minutes until it answers
    fn wait_for_builder(&self, ctx: &JobCtx) {
        ctx.log("Waiting for the Linux builder …");
        let pre = Prerequisites::new(self.paths());
        for _ in 0..60 {
            if pre.builder_running().status == PrerequisiteStatus::Ok || self.is_closing() {
                return;
            }
            std::thread::sleep(Duration::from_secs(2));
        }
    }
}

// MARK: Helpers

impl Server {
    /// `updates.check`, kept for the Doctor page (and "omaflake x is available" in the activity list)
    fn updates_check(&self) -> Result<UpdateAnswer, RpcError> {
        lock(&self.inner.update).checking = true;
        self.poke();
        let answer = check_update(self);
        {
            let mut u = lock(&self.inner.update);
            *u = super::views::UpdateRecord {
                state: answer.as_ref().ok().map(|a| a.state),
                release: answer.as_ref().ok().and_then(|a| a.release.clone()),
                checked: Some(crate::json::now()),
                error: answer.as_ref().err().map(|e| e.message.clone()),
                checking: false,
            };
        }
        if let Ok(UpdateAnswer { state: UpdateState::Available, release: Some(r) }) = &answer {
            self.events().record(Level::Info, super::views::text::tf!("omaflake %@ is available", &r.version), None);
        }
        self.poke();
        answer
    }
}

/// The update check (`AppModel.checkForUpdate`) – only when a client asks (`updates.check`)
pub(crate) fn check_update(server: &Server) -> Result<UpdateAnswer, RpcError> {
    let paths = server.paths();
    if Settings::load(&paths).check_for_updates == Some(false) {
        return Ok(UpdateAnswer { state: UpdateState::Off, release: None });
    }
    let feed = lock(&server.conn.session).update_feed.clone();
    let Some(feed) = UpdateCheck::feed_url(feed.as_deref()) else {
        return Ok(UpdateAnswer { state: UpdateState::NoFeed, release: None });
    };
    let newer = UpdateCheck::newer(crate::VERSION, &feed)
        .map_err(|e| RpcError::from(Error::from_text(Some(e.code().unwrap_or(codes::NETWORK)), e.text().clone())))?;
    Ok(match newer {
        Some(r) => UpdateAnswer {
            state: UpdateState::Available,
            release: Some(ReleaseDto {
                version: r.version,
                url: r.url,
                sha256: r.sha256,
                notes: r.notes,
                page: r.page,
            }),
        },
        None => UpdateAnswer { state: UpdateState::UpToDate, release: None },
    })
}

pub(crate) fn settings_dto(paths: &Paths) -> SettingsDto {
    let s = Settings::load(paths);
    SettingsDto {
        developer_mode: s.developer_mode.unwrap_or(false),
        check_for_updates: s.check_for_updates.unwrap_or(true),
        notifications: s.notifications.unwrap_or(true),
        keep_recipe_versions: s.keep_recipe_versions.unwrap_or(false),
        help: s.help.unwrap_or(false),
        default_credential: Credentials::new(paths.clone()).default_credential().map(|c| c.id),
        host_keyboard: s.host_keyboard.clone(),
        default_locale: crate::locales::Locales::new(paths.clone()).default_locale().map(|l| l.id),
        default_look: crate::looks::LookStore::new(paths.clone()).default_look(&s).map(|l| l.id),
        flake_in_use: s.flake_path().ok(),
        flake: s.flake,
        qemu: s.qemu,
        nix: s.nix,
        runner: s.runner,
        data_folder: paths.root.to_string_lossy().into_owned(),
    }
}

fn is_default_data_folder(paths: &Paths) -> bool {
    platform::standardize(&paths.root) == platform::standardize(&Paths::default_home())
}

fn recipe_defaults(s: &RecipeSettings) -> RecipeDefaults {
    let mut d = RecipeDefaults::new(
        s.kind,
        None,
        s.cpus,
        s.memory_gb,
        s.disk_gb,
        s.network,
        s.share,
        s.ephemeral,
        s.user.clone(),
    );
    d.audio = s.audio;
    d.microphone = s.microphone;
    d.clipboard = s.clipboard;
    d.camera = s.camera;
    d.display = s.display;
    d
}

fn package_kind(k: PackageKind) -> PackageKindDto {
    match k {
        PackageKind::Vm => PackageKindDto::Vm,
        PackageKind::Image => PackageKindDto::Image,
        PackageKind::Recipe => PackageKindDto::Recipe,
    }
}

fn file_name(path: &Path) -> String {
    path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_else(|| path.to_string_lossy().into_owned())
}

/// A job's result as JSON
fn result<T: serde::Serialize>(value: T) -> crate::Result<Value> {
    serde_json::to_value(value).map_err(|e| vm_err!("Cannot encode the result: {e}"))
}

/// A name as a part of a file name: anything but letters, digits, `.`, `-` and `_` becomes `_`
fn file_safe(name: &str) -> String {
    name.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') { c } else { '_' }).collect()
}

/// The kebab-case name of a job kind (its JSON value)
fn job_kind_name(kind: JobKind) -> String {
    serde_json::to_value(kind).ok().and_then(|v| v.as_str().map(str::to_string)).unwrap_or_else(|| "job".into())
}

/// The clean-up's "before" and "after" lines of df (size, used, avail, …) → "Linux builder: 40G → 52G free"
pub(crate) fn cleanup_summary(output: &str) -> String {
    let free: Vec<&str> =
        output.lines().filter(|l| l.starts_with("/dev/")).map(|l| l.split_whitespace().nth(3).unwrap_or("?")).collect();
    if free.len() == 2 {
        format!("Linux builder: {} → {} free", free[0], free[1])
    } else {
        "Linux builder cleaned up".to_string()
    }
}

/// The host as `initialize` names it
fn host() -> Host {
    let arch = match platform::machine().as_str() {
        "arm64" | "aarch64" => "aarch64".to_string(),
        "x86_64" | "amd64" => "x86_64".to_string(),
        other => other.to_string(),
    };
    let (os_name, os) = if cfg!(target_os = "macos") {
        let version = std::process::Command::new("/usr/bin/sw_vers")
            .arg("-productVersion")
            .output()
            .ok()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        ("darwin", format!("macOS {version}").trim().to_string())
    } else {
        let pretty = std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|t| {
                t.lines().find_map(|l| l.strip_prefix("PRETTY_NAME=").map(|v| v.trim_matches('"').to_string()))
            })
            .unwrap_or_else(|| "Linux".into());
        ("linux", pretty)
    };
    Host { system: format!("{arch}-{os_name}"), os }
}

// MARK: Params

#[derive(Deserialize)]
struct TargetParams {
    target: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CreateParams {
    name: String,
    image: String,
    settings: RecipeSettings,
    #[serde(default)]
    share_path: Option<String>,
    #[serde(default)]
    start: bool,
}

#[derive(Deserialize)]
struct StartParams {
    name: String,
    #[serde(default)]
    display: Option<String>,
    #[serde(default)]
    gl: Option<String>,
}

#[derive(Deserialize)]
struct StopParams {
    name: String,
    /// Seconds
    #[serde(default)]
    timeout: Option<f64>,
}

#[derive(Deserialize)]
struct UpgradeParams {
    name: String,
    #[serde(default)]
    image: Option<String>,
}

#[derive(Deserialize)]
struct SetParams {
    name: String,
    #[serde(default)]
    audio: Option<bool>,
    #[serde(default)]
    microphone: Option<bool>,
    #[serde(default)]
    clipboard: Option<bool>,
    #[serde(default)]
    camera: Option<bool>,
}

#[derive(Deserialize)]
struct SshParams {
    name: String,
    #[serde(default)]
    command: Vec<String>,
}

#[derive(Deserialize)]
struct AgentParams {
    name: String,
    /// Seconds to wait for a report that says the agent is up
    #[serde(default)]
    wait: Option<f64>,
}

#[derive(Deserialize)]
struct LogParams {
    name: String,
    #[serde(default)]
    lines: Option<usize>,
    #[serde(default)]
    qemu: bool,
}

#[derive(Deserialize)]
struct WaitParams {
    name: String,
    #[serde(default)]
    timeout: Option<f64>,
}

#[derive(Deserialize)]
struct RecipeParam {
    recipe: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExternalParams {
    path: String,
    #[serde(default)]
    confirm_network: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct BuildParams {
    recipe: String,
    /// The credential's ID (plan B137); left out or "" = the default credential
    #[serde(default)]
    credential: Option<String>,
    /// An installed look; left out or "" = the default look (plan B141)
    #[serde(default)]
    look: Option<String>,
    /// A locale's ID or name; left out or "" = the default locale (plan B141)
    #[serde(default)]
    locale: Option<String>,
    /// macOS: the base folder (default: the newest base)
    #[serde(default)]
    base: Option<String>,
    /// macOS: keep a failed build folder for inspection
    #[serde(default)]
    keep_failed: bool,
}

/// The macOS bases (none on a host without macOS guests)
pub(crate) fn base_dtos(paths: &Paths) -> Vec<BaseDto> {
    MacBaseInstaller::list(paths)
        .into_iter()
        .map(|b| BaseDto {
            path: b.path.to_string_lossy().into_owned(),
            name: b.name,
            os_build: b.os_build,
            user: b.user,
            ready: b.ready,
            newest: b.newest,
            created: b.created,
            size_bytes: b.size_bytes,
            credential: b.record.as_ref().map(|r| r.credential.clone()),
            credential_name: b.record.map(|r| r.credential_name),
        })
        .collect()
}

/// Every locale with the images built with it and the default (plan B141)
pub(crate) fn locale_dtos(paths: &Paths) -> Vec<LocaleDto> {
    let locales = crate::locales::Locales::new(paths.clone());
    let default = locales.default_locale().map(|l| l.id);
    let images = ImageStore::new(paths.clone()).list();
    locales
        .list()
        .into_iter()
        .map(|l| LocaleDto {
            is_default: default.as_deref() == Some(l.id.as_str()),
            images: images
                .iter()
                .filter(|i| i.info.locale.as_deref() == Some(l.id.as_str()))
                .map(|i| i.name.clone())
                .collect(),
            id: l.id,
            name: l.name,
            locale: l.locale,
            time_zone: l.time_zone,
            keyboards: l.keyboards,
        })
        .collect()
}

pub(crate) fn profile_dtos(paths: &Paths) -> Vec<ProfileDto> {
    let profiles = crate::profiles::Profiles::new(paths.clone());
    let default = profiles.default_profile().map(|p| p.id);
    profiles
        .list()
        .into_iter()
        .map(|p| ProfileDto {
            is_default: default.as_deref() == Some(p.id.as_str()),
            id: p.id,
            name: p.name,
            cpus: p.cpus,
            memory_gb: p.memory_gb,
        })
        .collect()
}

/// `keymaps.save` (plan B147)
#[derive(Deserialize)]
struct SaveKeymapParams {
    name: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    keys: crate::keymaps::Keys,
    /// The set edited ("vim"): renamed, its old folder goes
    #[serde(default)]
    previous: Option<String>,
}

#[derive(Deserialize)]
struct SaveRecipeParams {
    name: String,
    platform: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    modules: Vec<String>,
    #[serde(default)]
    look: Option<String>,
    #[serde(default)]
    keymaps: Vec<String>,
    #[serde(default)]
    requirements: crate::composition::Requirements,
    #[serde(default)]
    vm: BTreeMap<String, Value>,
    /// The recipe edited ("my.nixos.work"): renamed, its old folder goes
    #[serde(default)]
    previous: Option<String>,
}

#[derive(Deserialize)]
struct BaseParams {
    base: String,
}

#[derive(Deserialize)]
struct ProfileParamsIn {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    cpus: Option<u32>,
    #[serde(default, rename = "memoryGB")]
    memory_gb: Option<u32>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LocaleParamsIn {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    locale: Option<String>,
    #[serde(default)]
    time_zone: Option<String>,
    #[serde(default)]
    keyboards: Option<Vec<String>>,
}

/// Every credential with whether a password is stored (read without the secret: no prompt) and the default
pub(crate) fn credential_dtos(paths: &Paths) -> Vec<CredentialDto> {
    let credentials = Credentials::new(paths.clone());
    let default = credentials.default_credential().map(|c| c.id);
    credentials
        .list()
        .into_iter()
        .map(|c| CredentialDto {
            has_password: credentials.has_password(&c.id),
            is_default: default.as_deref() == Some(c.id.as_str()),
            id: c.id,
            name: c.name,
            user: c.user,
            full_name: c.full_name,
            email: c.email,
        })
        .collect()
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct CredentialParams {
    #[serde(default)]
    id: Option<String>,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    user: Option<String>,
    #[serde(default)]
    full_name: Option<String>,
    #[serde(default)]
    email: Option<String>,
    #[serde(default)]
    password: Option<String>,
}

/// The credential, look and locale of a build (plan B137, B141): `credential` None or "" = the default; `look` None or
/// "" = the default look, else an installed look (id, "<author>/<name>", name or title); `locale` None or "" = the
/// default locale (none: this computer's values), else a locale's ID or name
pub(crate) fn build_choice(
    paths: &Paths,
    credential: Option<&str>,
    look: Option<&str>,
    locale: Option<&str>,
) -> Result<BuildChoice, RpcError> {
    let defaults = BuildChoice::defaults(paths);
    // Every image is built with a credential – left out (or "") the default one (plan B139)
    let credential = match credential.filter(|c| !c.is_empty()) {
        None => defaults.credential.ok_or_else(|| {
            RpcError::from(vm_err!(
                codes::PREREQUISITE,
                "No credential yet – create one first (Credentials, or omaf credentials add)"
            ))
        })?,
        Some(id) => Credentials::new(paths.clone())
            .get(id)
            .ok_or_else(|| RpcError::from(vm_err!(codes::NOT_FOUND, "No credential \"{id}\"")))?,
    };
    let credential = Some(credential);
    let look = match look.filter(|l| !l.is_empty()) {
        None => defaults.look,
        Some(l) => Some(
            crate::looks::LookStore::new(paths.clone())
                .find(&Settings::load(paths), l)
                .ok_or_else(|| RpcError::from(vm_err!(codes::NOT_FOUND, "No look \"{l}\" – install it first (Looks)")))?
                .id,
        ),
    };
    let locale = match locale.filter(|l| !l.is_empty()) {
        None => defaults.locale,
        Some(l) => Some(
            crate::locales::Locales::new(paths.clone())
                .find(l)
                .ok_or_else(|| RpcError::from(vm_err!(codes::NOT_FOUND, "No locale \"{l}\"")))?,
        ),
    };
    Ok(BuildChoice { credential, look, locale })
}

/// What `images.build` takes for a macOS build besides the password
struct MacOptions {
    base: Option<PathBuf>,
    keep_failed: bool,
}

#[derive(Deserialize)]
struct ListParams {
    /// Also lab recipes when developer mode is off
    #[serde(default)]
    all: bool,
}

#[derive(Deserialize)]
struct InstallParams {
    id: String,
    source: String,
}

/// What a list offers and its "Install All" brings: the rows it shows (its filter applied)
#[derive(Deserialize)]
struct InstallAllParams {
    items: Vec<InstallParams>,
}

#[derive(Deserialize)]
struct LineParams {
    line: String,
}

#[derive(Deserialize)]
struct AddSourceParams {
    input: String,
    #[serde(default)]
    private: bool,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    branch: Option<String>,
    #[serde(default)]
    first: bool,
}

#[derive(Deserialize)]
struct EnableParams {
    id: String,
    enabled: bool,
}

#[derive(Deserialize)]
struct MoveParams {
    id: String,
    up: bool,
}

#[derive(Deserialize)]
struct InspectParams {
    path: String,
    #[serde(default)]
    expect: Option<PackageKindDto>,
    #[serde(default)]
    external: bool,
}

#[derive(Deserialize)]
struct ImportParams {
    path: String,
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    replace: bool,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct ExportParams {
    kind: PackageKindDto,
    name: String,
    folder: String,
    #[serde(default)]
    with_image: Option<bool>,
}

#[derive(Deserialize)]
struct SetPathParams {
    item: String,
    path: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct InstallBaseParams {
    /// The credential the guest user is created from (ID or name; default: the default credential) – plan B139
    #[serde(default)]
    credential: Option<String>,
    #[serde(default)]
    ipsw: Option<String>,
}

#[derive(Deserialize)]
struct AssignParams {
    /// The base ("mac-base-26A434")
    name: String,
    /// The credential's ID or name
    credential: String,
}

/// A key binding set's keys as one list: "<module> <action>" → key
fn keys_of(keys: &crate::keymaps::Keys) -> BTreeMap<String, String> {
    keys.iter()
        .flat_map(|(m, a)| a.iter().map(move |(action, key)| (format!("{m} {action}"), key.describe())))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cleanup_summary_reads_df() {
        let out = "/dev/vda 50G 30G 18G 63% /\nfreed 1 GiB\n/dev/vda 50G 20G 28G 42% /\n";
        assert_eq!(cleanup_summary(out), "Linux builder: 18G → 28G free");
        assert_eq!(cleanup_summary("nothing"), "Linux builder cleaned up");
    }

    #[test]
    fn host_names_a_system() {
        let h = host();
        assert!(h.system.ends_with("-darwin") || h.system.ends_with("-linux"));
        assert!(!h.os.is_empty());
    }

    #[test]
    fn create_settings_become_recipe_defaults() {
        let s: RecipeSettings = serde_json::from_value(json!({"kind": "nixos", "cpus": 2, "memoryGB": 4, "diskGB": 20,
            "network": "shared", "share": true, "ephemeral": false, "audio": false, "display": "window"}))
        .unwrap();
        let d = recipe_defaults(&s);
        assert_eq!((d.cpus, d.memory_gb, d.disk_gb), (2, 4, 20));
        assert_eq!(d.audio, Some(false));
        assert_eq!(d.display, Some(DisplayMode::Window));
    }
}
