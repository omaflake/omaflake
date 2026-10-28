// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the app's `AppModel` (Sources/OmaflakeApp/AppModel.swift) knew beyond the core, moved here so every UI shares
//! it: the recipe list with origins, versions and freshness, the current image of each recipe line, the VM rows with
//! status, address and agent report, the Doctor's blockers, the macOS limit, free names and the catalog offers.
//!
//! The functions here read the data folder (`RecipeCache::load`, `Snapshot::read`) or are pure (everything else) and
//! build the API's types (`View`). The server (server.rs) keeps the results and decides when to read again.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use super::types::*;
use crate::agent_protocol::AgentState;
use crate::backend::Backends;
use crate::file_format::UnreadableFile;
use crate::image_store::{Image, ImageStore};
use crate::ipsw_downloader::MacOsRelease;
use crate::mac_image_builder::MacImageBuilder;
use crate::models::{DisplayMode, GuestKind, InstanceConfig, RecipeDefaults, RecipeId, VmStatus};
use crate::omaflake_id::{Lengths, OmaflakeId};
use crate::paths::{Paths, Settings};
use crate::prerequisites::{Prerequisite, PrerequisiteItem, PrerequisiteStatus};
use crate::qemu_runtime::QemuRuntime;
use crate::recipe_sources::{CatalogRecipe, Installed, RecipeCatalog, RecipeLibrary, RecipeSource, Version};
use crate::vz_backend::VzBackend;
use crate::Result;

// MARK: Recipes (AppModel.loadRecipes)

/// The recipes as last read (`loadRecipes`): their defaults from the image flake, where each comes from, its versions
#[derive(Debug, Clone, Default)]
pub(crate) struct RecipeCache {
    pub loaded: bool,
    pub defaults: HashMap<String, RecipeDefaults>,
    /// QEMU can do 3D (virgl) – `display: auto` starts NixOS guests with the GPU then
    pub gl: bool,
    /// Recipe → source id (first source wins)
    pub origins: HashMap<String, String>,
    pub folders: HashMap<String, PathBuf>,
    pub installed: BTreeMap<String, Installed>,
    pub distinctions: HashMap<String, String>,
    /// Line → versions
    pub versions: HashMap<String, Vec<Version>>,
    pub unreadable: Vec<UnreadableFile>,
    /// What each recipe is made of (plan B142)
    pub parts: HashMap<String, RecipePartsDto>,
}

impl RecipeCache {
    /// Reads everything `AppModel.loadRecipes` read – from the recipes' files, without Nix: a recipe that is here is
    /// listed and installed whether Nix works or not (only building needs it). Without QEMU, macOS guests still work
    /// (`gl` false).
    pub fn load(paths: &Paths) -> Result<RecipeCache> {
        let settings = Settings::load(paths);
        let gl = QemuRuntime::locate(paths).map(|(r, _)| r.capabilities().gl).unwrap_or(false);
        let library = RecipeLibrary::new(paths.clone());
        let available = library.available(&settings);
        let mut origins = HashMap::new();
        let mut folders = HashMap::new();
        for a in available {
            origins.entry(a.id.clone()).or_insert(a.source);
            folders.entry(a.id).or_insert(a.folder);
        }
        let parts = recipe_parts(paths, &settings, &folders);
        let (defaults, not_read) = library.defaults(&settings);
        let mut unreadable = library.unreadable(&settings);
        unreadable.extend(not_read);
        Ok(RecipeCache {
            loaded: true,
            defaults,
            gl,
            origins,
            folders,
            installed: library.installed(),
            distinctions: library.distinctions(&settings),
            versions: library.versions(&settings),
            unreadable,
            parts,
        })
    }
}

/// What each recipe (id → its folder) is made of, with the modules, key bindings and looks here (plan B142)
pub(crate) fn recipe_parts(
    paths: &Paths,
    settings: &Settings,
    folders: &HashMap<String, PathBuf>,
) -> HashMap<String, RecipePartsDto> {
    let modules = crate::modules::ModuleStore::new(paths.clone());
    let capabilities = modules.capabilities(settings);
    let modules = modules.list(settings);
    let keymaps = crate::keymaps::KeymapStore::new(paths.clone()).list(settings);
    let looks = crate::looks::LookStore::new(paths.clone()).list(settings);
    folders
        .iter()
        .filter_map(|(id, folder)| {
            let spec = crate::composition::RecipeSpec::read(folder).ok()?;
            Some((id.clone(), parts_dto(&spec, &modules, &keymaps, &looks, &capabilities)))
        })
        .collect()
}

pub(crate) fn parts_dto(
    spec: &crate::composition::RecipeSpec,
    modules: &[crate::modules::Module],
    keymaps: &[crate::keymaps::Keymap],
    looks: &[crate::looks::Look],
    capabilities: &BTreeMap<String, crate::modules::Capability>,
) -> RecipePartsDto {
    let c = crate::composition::Composition::of(spec, modules, keymaps, capabilities);
    let look_id = spec
        .look
        .as_ref()
        .and_then(|r| looks.iter().find(|l| l.reference() == r.id && r.allows(&l.version)))
        .map(|l| l.id.clone());
    RecipePartsDto {
        modules: spec
            .modules
            .iter()
            .map(|r| {
                let here = c.modules.iter().find(|m| m.reference() == r.id);
                ModuleUseDto {
                    reference: r.label(),
                    title: here.map(|m| m.title.clone()),
                    version: here.map(|m| m.version.clone()),
                    provides: here.map(|m| m.provides.clone()).unwrap_or_default(),
                    modes: here.map(|m| m.modes.clone()).unwrap_or_default(),
                }
            })
            .collect(),
        look: spec.look.as_ref().map(crate::composition::PartRef::label),
        look_id,
        keymaps: spec.keymaps.iter().map(crate::composition::PartRef::label).collect(),
        bindings: c.bindings,
        issues: c.issues,
        unfree: c.unfree,
        requirements: spec.requirements,
        locales: spec.locales.clone(),
    }
}

/// A recipe's line (plan B110): its source and base – images of a line replace each other (`AppModel.line(of:)`)
pub(crate) fn line_of(recipe: &str, origins: &HashMap<String, String>, images: &HashMap<String, Vec<Image>>) -> String {
    if let Some(source) = origins.get(recipe) {
        return RecipeId::line_key(recipe, source);
    }
    images
        .values()
        .flatten()
        .find(|i| i.recipe() == recipe)
        .map(|i| i.info.line())
        .unwrap_or_else(|| RecipeId::line_key(recipe, ""))
}

/// The newest change in these folders (recursively; hidden files and `result*` links of nix builds left out) – the
/// recipe list is read again when it moves (`AppModel.newestChange`, plan B73/B82)
pub(crate) fn newest_change(folders: &[PathBuf]) -> SystemTime {
    fn walk(dir: &Path, newest: &mut SystemTime) {
        let Ok(entries) = std::fs::read_dir(dir) else { return };
        for e in entries.flatten() {
            let name = e.file_name();
            let name = name.to_string_lossy();
            if name.starts_with('.') || name.starts_with("result") {
                continue;
            }
            let Ok(meta) = std::fs::symlink_metadata(e.path()) else { continue };
            if let Ok(m) = meta.modified() {
                if m > *newest {
                    *newest = m;
                }
            }
            if meta.is_dir() {
                walk(&e.path(), newest);
            }
        }
    }
    let mut newest = SystemTime::UNIX_EPOCH;
    for f in folders {
        walk(f, &mut newest);
    }
    newest
}

/// The folders whose changes reload the recipes: the flake, the guest values, the downloaded recipes and the enabled
/// local recipe folders (plan B107)
pub(crate) fn watched_folders(paths: &Paths) -> Vec<PathBuf> {
    let settings = Settings::load(paths);
    let mut folders = Vec::new();
    if let Ok(flake) = settings.flake_path() {
        folders.push(PathBuf::from(flake));
    }
    folders.push(paths.guest());
    folders.push(paths.recipes());
    // Parts a recipe is made of: an installed or removed module changes what its recipes have (plan B142)
    for kind in ["modules", "keymaps", "looks"] {
        folders.push(paths.root.join(kind));
    }
    for s in settings.recipe_sources() {
        if s.is_enabled() && s.kind == crate::recipe_sources::RecipeSourceKind::Folder {
            folders.push(PathBuf::from(&s.location));
        }
    }
    folders
}

// MARK: VMs and images (AppModel.refresh)

/// A VM as the list shows it
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct VmRow {
    pub config: InstanceConfig,
    pub status: VmStatus,
    pub address: Option<String>,
    pub agent: Option<AgentState>,
    /// NixOS while running: the accelerator QEMU was started with (plan B124)
    pub accelerator: Option<crate::qemu_command::Accel>,
    pub location: Option<String>,
}

/// One look at the data folder: VMs, images by line, current images, word counts
#[derive(Debug, Clone, Default)]
pub(crate) struct Snapshot {
    pub rows: Vec<VmRow>,
    pub unreadable_vms: Vec<UnreadableFile>,
    /// Line → images, newest first
    pub images: HashMap<String, Vec<Image>>,
    pub unreadable_images: Vec<UnreadableFile>,
    /// Recipe → the newest image of its line
    pub current: HashMap<String, String>,
    pub lengths: Lengths,
}

impl Snapshot {
    pub fn read(paths: &Paths, recipes: &RecipeCache) -> Result<Snapshot> {
        let backends = Backends::new(paths.clone())?;
        let store = &backends.store;
        let image_store = ImageStore::new(paths.clone());
        let images = by_line(image_store.list());
        let configs = store.list();
        let current = current_images(
            configs.iter().map(|c| c.recipe.clone()).chain(recipes.defaults.keys().cloned()),
            &recipes.origins,
            &images,
        );
        let vz = backends.vz().ok();
        let rows = configs
            .into_iter()
            .map(|c| {
                let backend = backends.backend_for(&c).ok();
                let status = backend.map(|b| b.status(&c)).unwrap_or(VmStatus::Stopped);
                let running = status == VmStatus::Running;
                let address = if c.kind == GuestKind::Macos && running { vz.and_then(|v| v.address(&c)) } else { None };
                let agent = if running { backend.and_then(|b| b.agent_state(&c)) } else { None };
                let accelerator = match (running, c.kind, backends.qemu()) {
                    (true, GuestKind::Nixos, Ok(q)) => q.runtime.accelerator(&c.name),
                    _ => None,
                };
                let location =
                    store.is_external(&c.name).then(|| paths.instance(&c.name).dir.to_string_lossy().into_owned());
                VmRow { config: c, status, address, agent, accelerator, location }
            })
            .collect::<Vec<_>>();
        let recipe_ids: Vec<String> = recipes.versions.values().flatten().map(|v| v.id.clone()).collect();
        let image_names: Vec<String> = images.values().flatten().map(|i| i.name.clone()).collect();
        let configs: Vec<InstanceConfig> = rows.iter().map(|r| r.config.clone()).collect();
        Ok(Snapshot {
            lengths: Lengths::new(&recipe_ids, &image_names, &configs),
            rows,
            unreadable_vms: store.unreadable(),
            images,
            unreadable_images: image_store.unreadable(),
            current,
        })
    }

    pub fn row(&self, name: &str) -> Option<&VmRow> {
        self.rows.iter().find(|r| r.config.name == name)
    }

    /// Running macOS guests (Virtualization.framework allows `VzBackend::MAX_RUNNING`, plan BM16)
    pub fn running_macs(&self) -> Vec<String> {
        self.rows
            .iter()
            .filter(|r| r.config.kind == GuestKind::Macos && r.status == VmStatus::Running)
            .map(|r| r.config.name.clone())
            .collect()
    }
}

/// Images grouped by line, newest first
pub(crate) fn by_line(images: Vec<Image>) -> HashMap<String, Vec<Image>> {
    let mut lines: HashMap<String, Vec<Image>> = HashMap::new();
    for i in images {
        lines.entry(i.info.line()).or_default().push(i);
    }
    for list in lines.values_mut() {
        list.sort_by(|a, b| b.info.created.cmp(&a.info.created).then_with(|| a.name.cmp(&b.name)));
    }
    lines
}

/// A recipe's image is the newest of its line: there is no default image to pick – a VM is created from the image
/// it is started from (plan B100); "upgrade" moves a VM to the newest image of its recipe's line (B110)
pub(crate) fn current_images(
    recipes: impl Iterator<Item = String>,
    origins: &HashMap<String, String>,
    images: &HashMap<String, Vec<Image>>,
) -> HashMap<String, String> {
    let mut current = HashMap::new();
    for recipe in recipes {
        if let Some(newest) = images.get(&line_of(&recipe, origins, images)).and_then(|l| l.first()) {
            current.insert(recipe, newest.name.clone());
        }
    }
    current
}

/// Why a macOS guest cannot start now (`AppModel.macLimitReason`), None if it can
pub(crate) fn mac_limit_reason(running: &[String]) -> Option<String> {
    (running.len() >= VzBackend::MAX_RUNNING).then(|| {
        format!(
            "macOS allows at most {} macOS VMs at the same time – running: {}",
            VzBackend::MAX_RUNNING,
            running.join(", ")
        )
    })
}

/// How a start shows a VM: `requested`, or for `auto` (None) the app's choice – a window, with 3D for NixOS guests
/// where QEMU can and the recipe does not ask for a plain window (`AppModel.start`)
pub(crate) fn start_display(c: &InstanceConfig, gl: bool, requested: Option<DisplayMode>) -> DisplayMode {
    requested.unwrap_or(if c.kind == GuestKind::Nixos && gl && c.display != Some(DisplayMode::Window) {
        DisplayMode::Gpu
    } else {
        DisplayMode::Window
    })
}

/// VMs that were running and are stopped now (removed ones did not end)
pub(crate) fn ended(old: &[(String, VmStatus)], new: &[(String, VmStatus)]) -> Vec<String> {
    old.iter()
        .filter(|(_, s)| *s == VmStatus::Running)
        .filter(|(name, _)| new.iter().any(|(n, s)| n == name && *s == VmStatus::Stopped))
        .map(|(n, _)| n.clone())
        .collect()
}

/// "research" → "research-2", "research-3", … – the first free name after `name` (an import under another name)
pub(crate) fn next_numbered(name: &str, taken: &HashSet<String>) -> String {
    let mut n = 2;
    while taken.contains(&format!("{name}-{n}")) {
        n += 1;
    }
    format!("{name}-{n}")
}

/// The first free VM name for a recipe: its display name, then "-2", "-3", … (`AppModel.nextFreeName(for:)`)
pub(crate) fn next_free_name(recipe: &str, defaults: Option<&RecipeDefaults>, taken: &HashSet<String>) -> String {
    let name = RecipeId::display_name(recipe, defaults);
    if taken.contains(&name) {
        next_numbered(&name, taken)
    } else {
        name
    }
}

// MARK: Doctor (AppModel.buildBlockers, startBlockers, doctorIssues)

/// The Doctor's results as last checked
#[derive(Debug, Clone)]
pub(crate) struct Doctor {
    pub prerequisites: Vec<Prerequisite>,
    pub nixos: Vec<Prerequisite>,
    pub macos: Vec<Prerequisite>,
    pub latest: Option<MacOsRelease>,
    pub has_mac_base: bool,
    pub builder_system_is_omaflakes: bool,
    /// The `omaf` command's link (plan B122)
    pub cli: Option<CliLinkDto>,
    pub checked: chrono::DateTime<chrono::Utc>,
}

impl Doctor {
    pub fn all(&self) -> impl Iterator<Item = &Prerequisite> {
        self.prerequisites.iter().chain(&self.nixos).chain(&self.macos)
    }
}

/// What the Doctor reports against building images of a guest kind; empty = building is possible
pub(crate) fn build_blockers<'a>(
    kind: GuestKind,
    all: impl Iterator<Item = &'a Prerequisite>,
) -> Vec<&'a Prerequisite> {
    use PrerequisiteItem::*;
    let needed: &[PrerequisiteItem] = match kind {
        GuestKind::Nixos => {
            &[Nix, Flakes, NixDarwin, LinuxBuilder, Flake, Credential, Look, BuilderRunning, FlakeEval, NixSpace]
        }
        GuestKind::Macos => &[Host, Nix, Flakes, Flake, SshKey, Credential, Look, FlakeEval, MacBase, MacSpace],
    };
    all.filter(|p| {
        needed.contains(&p.item)
            && (p.status == PrerequisiteStatus::Missing || (p.item == MacBase && p.status != PrerequisiteStatus::Ok))
    })
    .collect()
}

/// What blocks starting VMs of a guest kind (NixOS needs QEMU; macOS has its own runner)
pub(crate) fn start_blockers(kind: GuestKind, prerequisites: &[Prerequisite]) -> Vec<&Prerequisite> {
    match kind {
        GuestKind::Nixos => prerequisites
            .iter()
            .filter(|p| p.item == PrerequisiteItem::Qemu && p.status == PrerequisiteStatus::Missing)
            .collect(),
        GuestKind::Macos => Vec::new(),
    }
}

/// Items that need attention (the warning sign on the Doctor)
pub(crate) fn doctor_issues<'a>(all: impl Iterator<Item = &'a Prerequisite>) -> usize {
    all.filter(|p| {
        (p.status == PrerequisiteStatus::Missing && p.required())
            || (p.item == PrerequisiteItem::MacBase && p.status == PrerequisiteStatus::Limited)
    })
    .count()
}

/// What a check is about, in English (OmaflakeApp.swift `RequirementText.title`); on a Linux host its own host line
pub(crate) fn requirement_title(item: PrerequisiteItem) -> &'static str {
    requirement_title_on(item, super::views::HostOs::this())
}

pub(crate) fn requirement_title_on(item: PrerequisiteItem, host: super::views::HostOs) -> &'static str {
    use PrerequisiteItem::*;
    match item {
        Host if host == super::views::HostOs::Linux => "Linux with x86_64 or aarch64",
        Host => "Mac with Apple silicon, macOS 27",
        Kvm => "KVM (hardware virtualization)",
        Nix => "Nix",
        Flakes => "Nix flakes",
        NixDarwin => "nix-darwin",
        LinuxBuilder => "Linux builder (NixOS images)",
        Qemu => "QEMU (NixOS guests)",
        Flake => "Recipes (your flake)",
        SshKey => "SSH key",
        Credential => "Credential",
        Look => "Look",
        Git => "git (optional)",
        Cli => "omaf command (optional)",
        BuilderRunning => "Linux builder answers",
        FlakeEval => "Recipes evaluate",
        NixSpace => "Space in the Nix store",
        Ipsw => "macOS from Apple (IPSW)",
        MacBase => "macOS base",
        MacSpace => "Space for macOS",
    }
}

pub(crate) fn prerequisite_dto(p: &Prerequisite) -> PrerequisiteDto {
    PrerequisiteDto {
        item: p.item,
        status: p.status,
        title: requirement_title(p.item).into(),
        required: p.required(),
        detail: p.detail.clone(),
        hint: p.hint.clone(),
        install: p.install.clone(),
        settable_path: p.settable_path(),
        free_bytes: p.free_bytes,
        needed_bytes: p.needed_bytes,
        base_user: p.base_user.clone(),
        expected_user: p.expected_user.clone(),
    }
}

pub(crate) fn release_dto(r: &MacOsRelease) -> MacOsReleaseDto {
    MacOsReleaseDto { version: r.version.clone(), build: r.build.clone(), url: r.url.clone(), bytes: r.bytes }
}

pub(crate) fn doctor_report(d: &Doctor) -> DoctorReport {
    let list = |v: &[Prerequisite]| v.iter().map(prerequisite_dto).collect::<Vec<_>>();
    let refs = |v: Vec<&Prerequisite>| v.into_iter().map(prerequisite_dto).collect::<Vec<_>>();
    DoctorReport {
        prerequisites: list(&d.prerequisites),
        nixos_preflight: list(&d.nixos),
        macos_preflight: list(&d.macos),
        latest_release: d.latest.as_ref().map(release_dto),
        build_blockers: PerKind {
            nixos: refs(build_blockers(GuestKind::Nixos, d.all())),
            macos: refs(build_blockers(GuestKind::Macos, d.all())),
        },
        start_blockers: PerKind {
            nixos: refs(start_blockers(GuestKind::Nixos, &d.prerequisites)),
            macos: refs(start_blockers(GuestKind::Macos, &d.prerequisites)),
        },
        issues: doctor_issues(d.all()),
        requirements_missing: d.prerequisites.iter().any(|p| p.required() && p.status == PrerequisiteStatus::Missing),
        has_mac_base: d.has_mac_base,
        builder_system_is_omaflakes: d.builder_system_is_omaflakes,
        cli: d.cli.clone(),
        mac_base_user: d.macos.iter().find(|p| p.item == PrerequisiteItem::MacBase).and_then(|p| p.base_user.clone()),
        checked: d.checked,
    }
}

/// Runs the Doctor's checks (`AppModel.checkAll`)
pub(crate) fn check_doctor(paths: &Paths) -> Doctor {
    let pre = crate::prerequisites::Prerequisites::new(paths.clone());
    let latest = VzBackend::locate_runner(paths).ok().and_then(|r| MacOsRelease::latest(&r));
    let (prerequisites, cli) = pre.check_with_cli();
    let link = crate::cli_link::CliLink::current();
    Doctor {
        prerequisites,
        cli: Some(cli_link_dto(&link, &cli)),
        nixos: pre.nixos_preflight(),
        macos: pre.macos_preflight(latest.as_ref()),
        latest,
        has_mac_base: MacImageBuilder::newest_base_in(paths).is_some(),
        builder_system_is_omaflakes: crate::builder_maintenance::BuilderMaintenance::new(paths.clone())
            .system_is_omaflakes(),
        checked: crate::json::now(),
    }
}

/// What Install/Uninstall of the `omaf` command would do (the Doctor's `cli` item)
pub(crate) fn cli_link_dto(link: &crate::cli_link::CliLink, check: &crate::cli_link::CliCheck) -> CliLinkDto {
    CliLinkDto {
        link: link.link.to_string_lossy().into_owned(),
        target: link.ours.as_ref().map(|o| o.to_string_lossy().into_owned()),
        installed: check.state.installed(),
        blocked: check.state.blocked(),
        on_path: check.on_path,
        admin: link.mac,
    }
}

// MARK: Catalogs (AppModel.offeredRecipes, recipeUpdates, suggestedRecipes)

/// A catalog entry with its source
pub(crate) type Offer<'a> = (&'a CatalogRecipe, &'a RecipeSource);

/// Recipes the remote catalogs offer that are not here yet (first source wins). By ID (author, kind, name and
/// content): what is here already is not offered, also not from another source; an older version of a recipe of this
/// source is updated in its details, not offered again. `hash` is `CatalogRecipe::content_hash` (a parameter for tests).
pub(crate) fn offered<'a>(
    sources: &'a [RecipeSource],
    catalogs: &'a HashMap<String, RecipeCatalog>,
    recipes: &RecipeCache,
    developer_mode: bool,
    hash: &dyn Fn(&CatalogRecipe) -> String,
) -> Vec<Offer<'a>> {
    let mut seen: HashSet<String> = recipes.defaults.keys().cloned().collect();
    let lines: HashSet<String> = recipes.origins.iter().map(|(id, s)| RecipeId::line_key(id, s)).collect();
    let mut out = Vec::new();
    for s in sources.iter().filter(|s| s.is_enabled() && s.kind != crate::recipe_sources::RecipeSourceKind::Folder) {
        let Some(catalog) = catalogs.get(&s.id()) else { continue };
        for r in &catalog.recipes {
            if !developer_mode && r.lab.unwrap_or(false) {
                continue;
            }
            if lines.contains(&RecipeId::line_key(&r.id, &s.id())) {
                continue;
            }
            if !seen.insert(RecipeId::make_with_hash(&r.id, &hash(r), OmaflakeId::LENGTH)) {
                continue;
            }
            out.push((r, s));
        }
    }
    out
}

/// The catalog entry of an installed recipe when its source has another version (`AppModel.update(for:)`)
pub(crate) fn update_for<'a>(
    id: &str,
    installed: &BTreeMap<String, Installed>,
    sources: &'a [RecipeSource],
    catalogs: &'a HashMap<String, RecipeCatalog>,
) -> Option<Offer<'a>> {
    let entry = installed.get(id)?;
    let s = sources.iter().find(|s| s.id() == entry.source)?;
    let r = catalogs.get(&s.id())?.recipes.iter().find(|r| r.id == RecipeId::base(id))?;
    (r.version != entry.version).then_some((r, s))
}

/// For each line of downloaded recipes whose newest version has a newer one in its source: that one, unless it is
/// installed already (plan B111)
pub(crate) fn recipe_updates<'a>(
    installed: &BTreeMap<String, Installed>,
    sources: &'a [RecipeSource],
    catalogs: &'a HashMap<String, RecipeCatalog>,
    hash: &dyn Fn(&CatalogRecipe) -> String,
) -> Vec<(String, Offer<'a>)> {
    let mut newest: HashMap<String, (&String, &Installed)> = HashMap::new();
    for (id, entry) in installed {
        let line = RecipeId::line_key(RecipeId::base(id), &entry.source);
        match newest.get(&line) {
            Some((_, e)) if e.installed >= entry.installed => {}
            _ => {
                newest.insert(line, (id, entry));
            }
        }
    }
    let mut out: Vec<(String, Offer<'a>)> = newest
        .values()
        .filter_map(|(id, _)| update_for(id, installed, sources, catalogs).map(|o| ((*id).clone(), o)))
        .filter(|(_, (r, _))| !installed.contains_key(&RecipeId::make_with_hash(&r.id, &hash(r), OmaflakeId::LENGTH)))
        .collect();
    out.sort_by(|a, b| a.0.cmp(&b.0));
    out
}

/// The official recipes (by omaflake from omaflake/recipes, no lab ones) – suggested at the first start
pub(crate) fn suggested<'a>(offered: &[Offer<'a>], runs_here: &dyn Fn(&CatalogRecipe) -> bool) -> Vec<Offer<'a>> {
    offered
        .iter()
        .filter(|(r, s)| s.is_official() && r.author == "omaflake" && !r.lab.unwrap_or(false) && runs_here(r))
        .copied()
        .collect()
}

pub(crate) fn catalog_dto(r: &CatalogRecipe, s: &RecipeSource, language: Option<&str>) -> CatalogRecipeDto {
    CatalogRecipeDto {
        id: r.id.clone(),
        author: r.author.clone(),
        kind: r.kind,
        name: r.name.clone(),
        description: r.description_in(language).to_string(),
        tags: r.tags.clone().unwrap_or_default(),
        license: r.license.clone(),
        lab: r.lab.unwrap_or(false),
        systems: r.systems.clone(),
        modules: r.modules.iter().map(crate::composition::PartRef::label).collect(),
        look: r.look.as_ref().map(crate::composition::PartRef::label),
        keymaps: r.keymaps.iter().map(crate::composition::PartRef::label).collect(),
        homepage: r.homepage.clone(),
        version: r.version.clone(),
        files: r
            .files
            .iter()
            .map(|f| CatalogFileDto { path: f.path.clone(), size: f.size, sha256: f.sha256.clone() })
            .collect(),
        source: s.id(),
        runs_here: r.runs_here(),
    }
}

pub(crate) fn source_dto(s: &RecipeSource) -> SourceDto {
    SourceDto {
        id: s.id(),
        kind: s.kind,
        location: s.location.clone(),
        branch: s.branch.clone(),
        display_name: s.display_name(),
        enabled: s.is_enabled(),
        private: s.is_private_repository(),
        official: s.is_official(),
        has_token: s.is_private_repository() && crate::recipe_sources::SourceTokens::has(s),
    }
}

pub(crate) fn unreadable(list: &[UnreadableFile]) -> Vec<Unreadable> {
    list.iter().map(|u| Unreadable { name: u.name.clone(), reason: u.reason.clone() }).collect()
}

// MARK: The API's views of a snapshot

/// Everything the API's lists are built from
pub(crate) struct View<'a> {
    pub paths: &'a Paths,
    pub snap: &'a Snapshot,
    pub recipes: &'a RecipeCache,
    pub busy: &'a HashSet<String>,
    /// Recipe → the store path a build would give now (`ImageStore::expected_store_paths`)
    pub expected: &'a HashMap<String, String>,
    pub language: Option<&'a str>,
    pub developer_mode: bool,
}

impl View<'_> {
    pub fn vm(&self, row: &VmRow) -> Vm {
        let c = &row.config;
        let n = self.snap.lengths.vm;
        let latest = self.snap.current.get(&c.recipe).cloned();
        Vm {
            name: c.name.clone(),
            recipe: c.recipe.clone(),
            image: c.image.clone(),
            kind: c.kind,
            system: c.system.clone(),
            status: row.status,
            busy: self.busy.contains(&c.name),
            cpus: c.cpus,
            memory_gb: c.memory_gb,
            disk_gb: c.disk_gb,
            network: c.network,
            share: c.share,
            share_path: c.share.then(|| self.paths.share_dir(c).to_string_lossy().into_owned()),
            ephemeral: c.ephemeral,
            ssh_port: (c.kind == GuestKind::Nixos).then_some(c.ssh_port),
            user: c.guest_user(),
            audio: c.sound_on(),
            microphone: c.microphone_on(),
            clipboard: c.clipboard_on(),
            camera: c.camera_on(),
            display: start_display(c, self.recipes.gl, None),
            created: c.created,
            identity: c.identity.as_deref().map(|w| OmaflakeId::short(w, n)),
            omaflake_id: OmaflakeId::vm(c, OmaflakeId::LENGTH),
            label: c.name.clone(),
            address: row.address.clone(),
            agent: row.agent.as_ref().and_then(|a| serde_json::to_value(a).ok()),
            accelerator: row.accelerator.map(|a| a.as_str().to_string()),
            location: row.location.clone(),
            upgradable: latest.as_ref().is_some_and(|l| *l != c.image),
            latest_image: latest,
        }
    }

    pub fn vms(&self) -> VmList {
        let mut vms: Vec<Vm> = self.snap.rows.iter().map(|r| self.vm(r)).collect();
        vms.sort_by(|a, b| a.name.cmp(&b.name));
        VmList { vms, unreadable: unreadable(&self.snap.unreadable_vms) }
    }

    pub fn image(&self, i: &Image, size: bool) -> ImageDto {
        let line = i.info.line();
        ImageDto {
            name: i.name.clone(),
            recipe: i.info.recipe.clone(),
            source: i.info.recipe_source.clone(),
            kind: i.info.kind,
            system: i.info.system.clone(),
            revision: i.info.revision.clone(),
            store_path: i.info.store_path.clone(),
            created: i.info.created,
            current: self.snap.images.get(&line).and_then(|l| l.first()).is_some_and(|f| f.name == i.name),
            line,
            path: i.path.to_string_lossy().into_owned(),
            size_bytes: if size { ImageStore::disk_usage(i) } else { None },
            label: OmaflakeId::image(i, self.snap.lengths.image),
            omaflake_id: OmaflakeId::image(i, OmaflakeId::LENGTH),
            outdated: self.expected.get(i.recipe()).map(|e| i.info.store_path.as_deref() != Some(e.as_str())),
            look: i.info.look.clone(),
            credential: i.info.credential.clone(),
            credential_name: i.info.credential_name.clone(),
            locale: i.info.locale.clone(),
            locale_name: i.info.locale_name.clone(),
            instances: self
                .snap
                .rows
                .iter()
                .filter(|r| r.config.image == i.name)
                .map(|r| r.config.name.clone())
                .collect(),
        }
    }

    /// Every built image, grouped by recipe (macOS, then NixOS, by name), newest first within a recipe (plan B93)
    pub fn images(&self) -> ImageList {
        let mut all: Vec<&Image> = self.snap.images.values().flatten().collect();
        all.sort_by(|a, b| {
            (b.info.kind, RecipeId::name_of_id(a.recipe()), a.recipe(), b.info.created).cmp(&(
                a.info.kind,
                RecipeId::name_of_id(b.recipe()),
                b.recipe(),
                a.info.created,
            ))
        });
        ImageList {
            images: all.into_iter().map(|i| self.image(i, false)).collect(),
            unreadable: unreadable(&self.snap.unreadable_images),
        }
    }

    pub fn line(&self, recipe: &str) -> String {
        line_of(recipe, &self.recipes.origins, &self.snap.images)
    }

    /// The recipe changed since its current image was built
    pub fn is_outdated(&self, recipe: &str) -> bool {
        let Some(expected) = self.expected.get(recipe) else { return false };
        let Some(current) = self.snap.current.get(recipe) else { return false };
        let Some(image) = self.snap.images.get(&self.line(recipe)).and_then(|l| l.iter().find(|i| &i.name == current))
        else {
            return false;
        };
        image.info.store_path.as_deref() != Some(expected.as_str())
    }

    pub fn recipe(&self, id: &str, d: &RecipeDefaults) -> RecipeDto {
        let line = self.line(id);
        RecipeDto {
            id: id.into(),
            base: RecipeId::base(id).into(),
            qualified: RecipeId::qualified(id),
            display_name: RecipeId::display_name(id, Some(d)),
            label: OmaflakeId::recipe(id, self.snap.lengths.recipe),
            kind: d.kind,
            systems: d.systems.clone(),
            runs_here: d.runs_here(),
            cpus: d.cpus,
            memory_gb: d.memory_gb,
            disk_gb: d.disk_gb,
            network: d.network,
            share: d.share,
            ephemeral: d.ephemeral,
            user: d.user.clone(),
            name: d.name.clone(),
            author: d.author.clone(),
            audio: d.audio,
            microphone: d.microphone,
            clipboard: d.clipboard,
            camera: d.camera,
            description: d.description_in(self.language).map(str::to_string),
            lab: d.is_lab(),
            display: d.display,
            tags: d.tags.clone(),
            license: d.license.clone(),
            homepage: d.homepage.clone(),
            source: self.recipes.origins.get(id).cloned(),
            distinction: self.recipes.distinctions.get(id).cloned(),
            folder: self.recipes.folders.get(id).map(|f| f.to_string_lossy().into_owned()),
            installed: self.recipes.installed.get(id).map(|i| InstalledDto {
                source: i.source.clone(),
                version: i.version.clone(),
                installed: i.installed,
            }),
            image: self.snap.current.get(id).cloned(),
            outdated: self.is_outdated(id),
            versions: self
                .recipes
                .versions
                .get(&line)
                .map(|v| v.iter().map(|v| version_dto(v, self.snap.lengths.recipe)).collect())
                .unwrap_or_default(),
            parts: self.recipes.parts.get(id).cloned(),
            line,
        }
    }

    /// The recipes – lab ones only in developer mode or with `all` – sorted by kind and name
    pub fn recipes(&self, all: bool) -> RecipeList {
        let show_lab = all || self.developer_mode;
        let mut recipes: Vec<RecipeDto> = self
            .recipes
            .defaults
            .iter()
            .filter(|(_, d)| show_lab || !d.is_lab())
            .map(|(id, d)| self.recipe(id, d))
            .collect();
        recipes.sort_by(|a, b| (a.kind, &a.display_name, &a.id).cmp(&(b.kind, &b.display_name, &b.id)));
        RecipeList {
            recipes,
            lines: self
                .recipes
                .versions
                .iter()
                .map(|(line, v)| (line.clone(), v.iter().map(|v| version_dto(v, self.snap.lengths.recipe)).collect()))
                .collect(),
            id_lengths: IdLengths {
                recipe: self.snap.lengths.recipe,
                image: self.snap.lengths.image,
                vm: self.snap.lengths.vm,
            },
            qemu_gl: self.recipes.gl,
            unreadable: unreadable(&self.recipes.unreadable),
            lab_hidden: !show_lab,
        }
    }
}

pub(crate) fn version_dto(v: &Version, words: usize) -> VersionDto {
    VersionDto {
        label: OmaflakeId::recipe(&v.id, words),
        id: v.id.clone(),
        source: v.source.clone(),
        added: v.added,
        newest: v.is_newest,
        live: v.is_live,
        images: v.images.clone(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::image_store::ImageInfo;
    use crate::json::parse_date;
    use crate::models::tests::{config, dev_defaults};
    use crate::models::NetworkMode;
    use crate::recipe_sources::RecipeSourceKind;

    fn image(recipe: &str, source: Option<&str>, name: &str, created: &str, store: &str) -> Image {
        let info = ImageInfo::new(
            recipe,
            source.map(str::to_string),
            GuestKind::Nixos,
            "aarch64-linux",
            "",
            "r",
            Some(store.into()),
            parse_date(created).unwrap(),
        );
        Image { name: name.into(), info, path: PathBuf::from(format!("/images/{name}")) }
    }

    fn prereq(item: PrerequisiteItem, status: PrerequisiteStatus) -> Prerequisite {
        Prerequisite::new(item, status, "", "", None)
    }

    #[test]
    fn current_image_is_the_newest_of_the_line() {
        let r = "omaflake.nixos.dev@aa";
        let older = image(r, Some("s"), "dev@1", "2026-10-01T00:00:00Z", "/nix/store/1");
        let newer = image("omaflake.nixos.dev@bb", Some("s"), "dev@2", "2026-10-02T00:00:00Z", "/nix/store/2");
        let other = image("omaflake.nixos.min@cc", None, "min@1", "2026-10-03T00:00:00Z", "/nix/store/3");
        let lines = by_line(vec![older, newer, other]);
        let origins: HashMap<String, String> = [(r.to_string(), "s".to_string())].into();
        assert_eq!(line_of(r, &origins, &lines), RecipeId::line_key(r, "s"));
        // An unknown recipe: the line of an image built from it
        assert_eq!(
            line_of("omaflake.nixos.min@cc", &HashMap::new(), &lines),
            RecipeId::line_key("omaflake.nixos.min", "")
        );
        let current = current_images(
            [r.to_string(), "omaflake.nixos.min@cc".into(), "x.nixos.y".into()].into_iter(),
            &origins,
            &lines,
        );
        assert_eq!(current[r], "dev@2");
        assert_eq!(current["omaflake.nixos.min@cc"], "min@1");
        assert!(!current.contains_key("x.nixos.y"));
    }

    #[test]
    fn outdated_compares_the_current_images_store_path() {
        let r = "omaflake.nixos.dev@aa";
        let lines = by_line(vec![image(r, Some("s"), "dev@1", "2026-10-01T00:00:00Z", "/nix/store/1")]);
        let mut recipes = RecipeCache::default();
        recipes.origins.insert(r.into(), "s".into());
        recipes.defaults.insert(r.into(), dev_defaults());
        let snap = Snapshot {
            current: current_images([r.to_string()].into_iter(), &recipes.origins, &lines),
            images: lines,
            ..Default::default()
        };
        let paths = Paths::new("/data", None);
        let busy = HashSet::new();
        let mut expected = HashMap::new();
        let view = |expected: &HashMap<String, String>| {
            View {
                paths: &paths,
                snap: &snap,
                recipes: &recipes,
                busy: &busy,
                expected,
                language: None,
                developer_mode: false,
            }
            .is_outdated(r)
        };
        assert!(!view(&expected));
        expected.insert(r.to_string(), "/nix/store/1".to_string());
        assert!(!view(&expected));
        expected.insert(r.to_string(), "/nix/store/2".to_string());
        assert!(view(&expected));
    }

    #[test]
    fn vm_rows_become_vms() {
        let mut c = config("research");
        c.recipe = "omaflake.nixos.dev@aa".into();
        c.image = "dev@1".into();
        c.identity = Some("amber lake quiet".into());
        let lines = by_line(vec![
            image(&c.recipe, None, "dev@1", "2026-10-01T00:00:00Z", "/1"),
            image(&c.recipe, None, "dev@2", "2026-10-02T00:00:00Z", "/2"),
        ]);
        let snap = Snapshot {
            rows: vec![VmRow {
                config: c.clone(),
                status: VmStatus::Running,
                address: None,
                accelerator: None,
                agent: None,
                location: None,
            }],
            current: current_images([c.recipe.clone()].into_iter(), &HashMap::new(), &lines),
            images: lines,
            ..Default::default()
        };
        let recipes = RecipeCache { gl: true, ..Default::default() };
        let paths = Paths::new("/data", None);
        let busy: HashSet<String> = ["research".to_string()].into();
        let expected = HashMap::new();
        let view = View {
            paths: &paths,
            snap: &snap,
            recipes: &recipes,
            busy: &busy,
            expected: &expected,
            language: None,
            developer_mode: false,
        };
        let vm = view.vm(&snap.rows[0]);
        assert!(vm.busy && vm.upgradable);
        assert_eq!(vm.latest_image.as_deref(), Some("dev@2"));
        assert_eq!(vm.display, DisplayMode::Gpu);
        assert_eq!(vm.ssh_port, Some(2230));
        assert_eq!(vm.share_path.as_deref(), Some("/data/share/research"));
        let images = view.images().images;
        assert_eq!(images.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["dev@2", "dev@1"]);
        assert!(images[0].current && !images[1].current);
        assert_eq!(images[1].instances, ["research"]);
    }

    #[test]
    fn start_display_follows_the_app() {
        let mut c = config("a");
        assert_eq!(start_display(&c, true, None), DisplayMode::Gpu);
        assert_eq!(start_display(&c, false, None), DisplayMode::Window);
        c.display = Some(DisplayMode::Window);
        assert_eq!(start_display(&c, true, None), DisplayMode::Window);
        assert_eq!(start_display(&c, true, Some(DisplayMode::None)), DisplayMode::None);
        c.kind = GuestKind::Macos;
        c.display = None;
        assert_eq!(start_display(&c, true, None), DisplayMode::Window);
    }

    #[test]
    fn ended_vms_were_running_and_are_stopped() {
        let old = vec![
            ("a".to_string(), VmStatus::Running),
            ("b".to_string(), VmStatus::Running),
            ("c".to_string(), VmStatus::Stopped),
            ("gone".to_string(), VmStatus::Running),
        ];
        let new = vec![
            ("a".to_string(), VmStatus::Stopped),
            ("b".to_string(), VmStatus::Running),
            ("c".to_string(), VmStatus::Stopped),
        ];
        assert_eq!(ended(&old, &new), ["a"]);
    }

    #[test]
    fn free_names_count_up() {
        let taken: HashSet<String> = ["dev".to_string(), "dev-2".to_string(), "research".to_string()].into();
        assert_eq!(next_free_name("omaflake.nixos.dev@aa", None, &taken), "dev-3");
        assert_eq!(next_free_name("omaflake.nixos.minimal", None, &taken), "minimal");
        let mut named = dev_defaults();
        named.name = Some("research".into());
        assert_eq!(next_free_name("omaflake.nixos.dev", Some(&named), &taken), "research-2");
        assert_eq!(next_numbered("dev", &taken), "dev-3");
    }

    #[test]
    fn mac_limit() {
        assert_eq!(mac_limit_reason(&["a".into()]), None);
        let reason = mac_limit_reason(&["a".into(), "b".into()]).unwrap();
        assert!(reason.ends_with("running: a, b"));
    }

    #[test]
    fn blockers_and_issues_as_in_the_app() {
        use PrerequisiteItem::*;
        use PrerequisiteStatus::*;
        let all = vec![
            prereq(Nix, Ok),
            prereq(Qemu, Missing),
            prereq(Git, Missing),
            prereq(LinuxBuilder, Missing),
            prereq(MacBase, Limited),
            prereq(SshKey, Ok),
        ];
        let nixos: Vec<PrerequisiteItem> =
            build_blockers(GuestKind::Nixos, all.iter()).iter().map(|p| p.item).collect();
        assert_eq!(nixos, [LinuxBuilder]);
        let macos: Vec<PrerequisiteItem> =
            build_blockers(GuestKind::Macos, all.iter()).iter().map(|p| p.item).collect();
        assert_eq!(macos, [MacBase]);
        assert_eq!(start_blockers(GuestKind::Nixos, &all).len(), 1);
        assert!(start_blockers(GuestKind::Macos, &all).is_empty());
        // Qemu and the builder are missing, the base is limited; git is optional
        assert_eq!(doctor_issues(all.iter()), 3);
        let d = Doctor {
            prerequisites: all.clone(),
            nixos: vec![],
            macos: vec![],
            latest: None,
            has_mac_base: false,
            builder_system_is_omaflakes: false,
            cli: None,
            checked: crate::json::now(),
        };
        let report = doctor_report(&d);
        assert!(report.requirements_missing);
        assert_eq!(report.build_blockers.nixos[0].title, "Linux builder (NixOS images)");
        assert_eq!(report.start_blockers.nixos[0].item, Qemu);
        assert!(!report.prerequisites.iter().find(|p| p.item == Git).unwrap().required);
    }

    fn catalog_recipe(id: &str, version: &str, lab: bool) -> CatalogRecipe {
        let p = RecipeId::parts(id).unwrap();
        CatalogRecipe {
            id: id.into(),
            author: p.author,
            kind: p.kind,
            name: p.name,
            path: String::new(),
            description: String::new(),
            descriptions: None,
            tags: None,
            license: None,
            lab: Some(lab),
            systems: vec!["aarch64-linux".into()],
            homepage: None,
            modules: vec![],
            look: None,
            keymaps: vec![],
            requirements: Default::default(),
            vm: Default::default(),
            locales: Default::default(),
            version: version.into(),
            files: vec![],
        }
    }

    fn catalog(recipes: Vec<CatalogRecipe>) -> RecipeCatalog {
        RecipeCatalog { recipes, ..RecipeCatalog::empty("x") }
    }

    fn hash(r: &CatalogRecipe) -> String {
        // Stands in for CatalogRecipe::content_hash: version-dependent hex
        format!("{:0<64}", hex::encode(r.version.as_bytes()))
    }

    #[test]
    fn offers_updates_and_suggestions() {
        let official = RecipeSource::official();
        let other = RecipeSource::new(RecipeSourceKind::Github, "someone/recipes", None, None, None);
        let folder = RecipeSource::new(RecipeSourceKind::Folder, "/r", None, None, None);
        let sources = vec![official.clone(), other.clone(), folder];
        let catalogs: HashMap<String, RecipeCatalog> = [
            (
                official.id(),
                catalog(vec![
                    catalog_recipe("omaflake.nixos.dev", "2", false),
                    catalog_recipe("omaflake.nixos.lab", "1", true),
                    catalog_recipe("omaflake.nixos.minimal", "1", false),
                ]),
            ),
            (other.id(), catalog(vec![catalog_recipe("omaflake.nixos.minimal", "1", false)])),
        ]
        .into();
        // dev is installed from the official source in version 1
        let installed_id =
            RecipeId::make_with_hash("omaflake.nixos.dev", &format!("{:0<64}", "31"), OmaflakeId::LENGTH);
        let mut recipes = RecipeCache::default();
        recipes.defaults.insert(installed_id.clone(), dev_defaults());
        recipes.origins.insert(installed_id.clone(), official.id());
        recipes.installed.insert(
            installed_id.clone(),
            Installed { source: official.id(), version: "1".into(), installed: crate::json::now() },
        );
        let offers = offered(&sources, &catalogs, &recipes, false, &hash);
        // dev: its line is here (an update instead); lab: hidden; minimal: once (the first source wins)
        let ids: Vec<(&str, String)> = offers.iter().map(|(r, s)| (r.id.as_str(), s.id())).collect();
        assert_eq!(ids, [("omaflake.nixos.minimal", official.id())]);
        assert_eq!(offered(&sources, &catalogs, &recipes, true, &hash).len(), 2);
        let updates = recipe_updates(&recipes.installed, &sources, &catalogs, &hash);
        assert_eq!(updates.len(), 1);
        assert_eq!(updates[0].0, installed_id);
        assert_eq!(updates[0].1 .0.version, "2");
        let s = suggested(&offers, &|_| true);
        assert_eq!(s.len(), 1);
        assert!(suggested(&offers, &|_| false).is_empty());
    }

    #[test]
    fn newest_change_skips_hidden_and_results() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("a");
        std::fs::create_dir_all(a.join("sub")).unwrap();
        std::fs::write(a.join("sub/f"), "x").unwrap();
        let first = newest_change(std::slice::from_ref(&a));
        assert!(first > SystemTime::UNIX_EPOCH);
        let later = SystemTime::now() + std::time::Duration::from_secs(100);
        std::fs::create_dir_all(a.join("result")).unwrap();
        std::fs::write(a.join(".hidden"), "x").unwrap();
        let set = |p: &Path| {
            let f = std::fs::File::options().write(true).open(p).or_else(|_| std::fs::File::open(p)).unwrap();
            f.set_modified(later).ok();
        };
        set(&a.join(".hidden"));
        assert!(newest_change(std::slice::from_ref(&a)) < later);
        std::fs::write(a.join("sub/g"), "y").unwrap();
        set(&a.join("sub/g"));
        assert_eq!(newest_change(&[a, dir.path().join("missing")]), later);
    }

    #[test]
    fn vm_network_none_is_kept() {
        let mut c = config("n");
        c.network = NetworkMode::None;
        let row = VmRow {
            config: c,
            status: VmStatus::Stopped,
            address: None,
            agent: None,
            accelerator: None,
            location: None,
        };
        let snap = Snapshot { rows: vec![row], ..Default::default() };
        let (recipes, busy, expected) = (RecipeCache::default(), HashSet::new(), HashMap::new());
        let paths = Paths::new("/d", None);
        let view = View {
            paths: &paths,
            snap: &snap,
            recipes: &recipes,
            busy: &busy,
            expected: &expected,
            language: None,
            developer_mode: false,
        };
        let list = view.vms();
        assert_eq!(list.vms[0].network, NetworkMode::None);
        assert!(!list.vms[0].upgradable);
    }
}
