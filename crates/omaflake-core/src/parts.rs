// SPDX-License-Identifier: AGPL-3.0-or-later
//! Catalog parts (plan B142): what recipes are made of besides themselves – looks, modules and key binding sets. They
//! come from the sources' catalogs (`looks/<author>/<name>/`, `modules/<author>/<platform>/<name>/`,
//! `keymaps/<author>/<name>/`, catalog v4) and are installed into `<data>/looks/`, `<data>/modules/`,
//! `<data>/keymaps/` – one folder each, one version each – or used as they are on disk from an enabled local folder
//! source. This is the store the three share: what is here, installing (every file checked against the catalog:
//! known paths inside the part's folder, sizes, SHA-256), removing. What a part *is* – a look, a module, key bindings –
//! is read by `looks`, `modules` and `keymaps`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use crate::error::{codes, IoContext, Result};
use crate::file_format::{write_atomic, FileFormat, Step};
use crate::models::GuestKind;
use crate::paths::{Paths, Settings};
use crate::recipe_sources::{CatalogFile, RecipeRef, RecipeSourceKind, SourceClient};
use crate::trash::Disposal;
use crate::vm_err;

/// The three kinds of parts
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PartKind {
    Look,
    Module,
    Keymap,
}

impl PartKind {
    /// The folder they live in, in a catalog and in the data folder
    pub fn dir(self) -> &'static str {
        match self {
            PartKind::Look => "looks",
            PartKind::Module => "modules",
            PartKind::Keymap => "keymaps",
        }
    }

    /// The file that makes a folder one of them
    pub fn file(self) -> &'static str {
        match self {
            PartKind::Look => "look.toml",
            PartKind::Module => "module.toml",
            PartKind::Keymap => "keymap.toml",
        }
    }

    /// The word in IDs: "<author>.look.<name>", "<author>.keymap.<name>"; a module's is its platform
    fn word(self) -> &'static str {
        match self {
            PartKind::Look => "look",
            PartKind::Module => "",
            PartKind::Keymap => "keymap",
        }
    }

    /// The ID of a part: "<author>.look.<name>", "<author>.keymap.<name>", "<author>.<platform>.<name>"
    pub fn id(self, author: &str, platform: Option<GuestKind>, name: &str) -> String {
        match (self, platform) {
            (PartKind::Module, Some(p)) => format!("{author}.{}.{name}", p.as_str()),
            _ => format!("{author}.{}.{name}", self.word()),
        }
    }

    /// (author, platform for a module, name) of an ID of this kind
    pub fn parse_id(self, id: &str) -> Option<(String, Option<GuestKind>, String)> {
        let p: Vec<&str> = id.split('.').collect();
        if p.len() != 3 || !RecipeRef::valid_name(p[0]) || !RecipeRef::valid_name(p[2]) {
            return None;
        }
        let platform = match self {
            PartKind::Module => Some(GuestKind::parse(p[1])?),
            _ if p[1] == self.word() => None,
            _ => return None,
        };
        Some((p[0].to_string(), platform, p[2].to_string()))
    }

    /// The ID of "<author>/<name>" (a module: on `platform`), or of an ID itself
    pub fn id_of(self, reference: &str, platform: Option<GuestKind>) -> Option<String> {
        if self.parse_id(reference).is_some() {
            return Some(reference.to_string());
        }
        let (author, name) = reference.split_once('/')?;
        if !RecipeRef::valid_name(author) || !RecipeRef::valid_name(name) {
            return None;
        }
        match self {
            PartKind::Module => Some(self.id(author, Some(platform?), name)),
            _ => Some(self.id(author, None, name)),
        }
    }

    /// "<author>/<name>" of an ID – how recipes and people name a part
    pub fn reference(self, id: &str) -> String {
        match self.parse_id(id) {
            Some((author, _, name)) => format!("{author}/{name}"),
            None => id.to_string(),
        }
    }

    /// Where a part lives in a catalog or a local folder: "looks/<author>/<name>", "modules/<author>/<platform>/<name>"
    pub fn catalog_path(self, id: &str) -> Option<String> {
        let (author, platform, name) = self.parse_id(id)?;
        Some(match platform {
            Some(p) => format!("{}/{author}/{}/{name}", self.dir(), p.as_str()),
            None => format!("{}/{author}/{name}", self.dir()),
        })
    }

    /// The files a part may have (tools/catalog.py checks the same): a look its look.toml and a preview, key bindings
    /// their keymap.toml, a module anything inside its folder
    fn allowed(self, rel: &str) -> bool {
        match self {
            PartKind::Look => rel == "look.toml" || rel == "preview.png",
            PartKind::Keymap => rel == "keymap.toml",
            PartKind::Module => !rel.starts_with('/') && rel.split('/').all(|p| !p.is_empty() && p != "." && p != ".."),
        }
    }
}

/// One part as the catalog describes it – what installing needs
#[derive(Debug, Clone)]
pub struct CatalogPart<'a> {
    pub id: &'a str,
    pub path: &'a str,
    /// The author's version ("1.2.0")
    pub version: &'a str,
    /// The files' content hash – changes with any file
    pub hash: &'a str,
    pub files: &'a [CatalogFile],
}

/// An installed part in `<data>/<kind>/.installed.json`
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InstalledPart {
    /// `RecipeSource::id` it came from
    pub source: String,
    /// The author's version
    pub version: String,
    /// The catalog's content hash it was installed at (an update has another)
    pub hash: String,
    #[serde(with = "crate::json::iso8601")]
    pub installed: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
struct InstalledPartsFile {
    parts: BTreeMap<String, InstalledPart>,
}

static INSTALLED_PARTS_STEPS: [Step; 1] = [|_| Ok(())];

/// A part omaflake can use: in an enabled local folder source, or installed
#[derive(Debug, Clone, PartialEq)]
pub struct PartEntry {
    pub id: String,
    pub folder: PathBuf,
    /// `RecipeSource::id` – a local folder's id for its parts
    pub source: String,
    /// What was installed; None for a local folder's part
    pub installed: Option<InstalledPart>,
}

/// The parts of one kind
#[derive(Debug, Clone)]
pub struct PartStore {
    pub paths: Paths,
    pub kind: PartKind,
}

impl PartStore {
    /// `<kind>/.installed.json`, schema `installed-parts`
    pub const FILE_FORMAT: FileFormat =
        FileFormat::new("installed-parts", ".installed.json", 1, &INSTALLED_PARTS_STEPS);
    /// Limits for downloaded files (as tools/catalog.py checks them in the source)
    pub const MAX_FILE: u64 = 1024 * 1024;
    pub const MAX_PART: u64 = 5 * 1024 * 1024;

    pub fn new(paths: Paths, kind: PartKind) -> Self {
        PartStore { paths, kind }
    }

    pub fn dir(&self) -> PathBuf {
        self.paths.root.join(self.kind.dir())
    }

    fn manifest_file(&self) -> PathBuf {
        self.dir().join(".installed.json")
    }

    pub fn installed(&self) -> BTreeMap<String, InstalledPart> {
        let Ok(data) = fs::read(self.manifest_file()) else { return BTreeMap::new() };
        Self::FILE_FORMAT.decode::<InstalledPartsFile>(&data).map(|(f, _)| f.parts).unwrap_or_default()
    }

    fn save(&self, parts: &BTreeMap<String, InstalledPart>) -> Result<()> {
        let dir = self.dir();
        fs::create_dir_all(&dir).with_path(&dir)?;
        let file = self.manifest_file();
        Self::FILE_FORMAT.check_writable(&file)?;
        write_atomic(&file, &Self::FILE_FORMAT.encode(&InstalledPartsFile { parts: parts.clone() })?)
    }

    /// An installed part's folder: `<data>/<kind>/<id without the kind's word>` – looks/alice.nord,
    /// modules/omaflake.nixos.neovim, keymaps/omaflake.default
    pub fn folder(&self, id: &str) -> Option<PathBuf> {
        let (author, platform, name) = self.kind.parse_id(id)?;
        Some(self.dir().join(match platform {
            Some(p) => format!("{author}.{}.{name}", p.as_str()),
            None => format!("{author}.{name}"),
        }))
    }

    /// The parts of this kind under a local source's folder: (id, folder)
    pub fn local_folders(&self, root: &Path) -> Vec<(String, PathBuf)> {
        let dirs = |u: &Path| -> Vec<String> {
            let Ok(entries) = fs::read_dir(u) else { return Vec::new() };
            let mut names: Vec<String> = entries
                .flatten()
                .map(|e| e.file_name().to_string_lossy().into_owned())
                .filter(|n| !n.starts_with('.') && RecipeRef::valid_name(n) && u.join(n).is_dir())
                .collect();
            names.sort();
            names
        };
        let base = root.join(self.kind.dir());
        let mut out = Vec::new();
        for author in dirs(&base) {
            let platforms: Vec<(Option<GuestKind>, PathBuf)> = match self.kind {
                PartKind::Module => {
                    GuestKind::ALL.iter().map(|k| (Some(*k), base.join(&author).join(k.as_str()))).collect()
                }
                _ => vec![(None, base.join(&author))],
            };
            for (platform, dir) in platforms {
                for name in dirs(&dir) {
                    let folder = dir.join(&name);
                    if folder.join(self.kind.file()).exists() {
                        out.push((self.kind.id(&author, platform, &name), folder));
                    }
                }
            }
        }
        out
    }

    /// Every part of this kind omaflake can use: the enabled local folders' first (in their order), then the installed
    /// ones; the first with an id wins
    pub fn entries(&self, settings: &Settings) -> Vec<PartEntry> {
        let mut out: Vec<PartEntry> = Vec::new();
        for s in settings.recipe_sources() {
            if s.is_enabled() && s.kind == RecipeSourceKind::Folder {
                for (id, folder) in self.local_folders(Path::new(&s.location)) {
                    if !out.iter().any(|e| e.id == id) {
                        out.push(PartEntry { id, folder, source: s.id(), installed: None });
                    }
                }
            }
        }
        for (id, entry) in self.installed() {
            let Some(folder) = self.folder(&id) else { continue };
            if folder.exists() && !out.iter().any(|e| e.id == id) {
                out.push(PartEntry { id, folder, source: entry.source.clone(), installed: Some(entry) });
            }
        }
        out
    }

    /// Downloads a part from a catalog and keeps it; a newer version replaces the installed one. `check` reads the
    /// downloaded folder before it is kept (a part this omaflake cannot read is not installed). Returns its folder.
    pub fn install(
        &self,
        part: &CatalogPart,
        client: &SourceClient,
        check: impl FnOnce(&Path) -> Result<()>,
    ) -> Result<PathBuf> {
        // The catalog is untrusted: the entry must be the part's own folder, with known files only, within the limits
        let well_formed = self.kind.parse_id(part.id).is_some()
            && self.kind.catalog_path(part.id).as_deref() == Some(part.path)
            && part.files.iter().any(|f| f.path == self.kind.file())
            && part.files.iter().all(|f| self.kind.allowed(&f.path) && f.size <= Self::MAX_FILE)
            && part.files.iter().map(|f| f.size).sum::<u64>() <= Self::MAX_PART;
        let target = self.folder(part.id);
        let (true, Some(target)) = (well_formed, target) else {
            return Err(vm_err!("The catalog's entry for {} is not well formed", part.id));
        };
        let dir = self.dir();
        fs::create_dir_all(&dir).with_path(&dir)?;
        let work = dir.join(format!(".download-{}", std::process::id()));
        let _ = Disposal::delete(&[&work]);
        let result = (|| -> Result<()> {
            for f in part.files {
                let data = client.fetch(&format!("{}/{}", part.path, f.path))?;
                let sha = {
                    use sha2::{Digest, Sha256};
                    hex::encode(Sha256::digest(&data))
                };
                if data.len() as u64 != f.size || sha != f.sha256.to_lowercase() {
                    return Err(vm_err!(
                        "{}/{} does not match the catalog (size or SHA-256) – nothing was installed",
                        part.id,
                        f.path
                    ));
                }
                let dest = work.join(&f.path);
                if let Some(parent) = dest.parent() {
                    fs::create_dir_all(parent).with_path(parent)?;
                }
                fs::write(&dest, &data).with_path(&dest)?;
            }
            check(&work)?;
            Disposal::delete(&[&target])?;
            fs::rename(&work, &target).with_path(&target)?;
            let mut installed = self.installed();
            installed.insert(
                part.id.to_string(),
                InstalledPart {
                    source: client.source.id(),
                    version: part.version.to_string(),
                    hash: part.hash.to_string(),
                    installed: Utc::now(),
                },
            );
            self.save(&installed)
        })();
        let _ = Disposal::delete(&[&work]);
        result.map(|()| target)
    }

    /// Removes an installed part (a local folder's stays there)
    pub fn remove(&self, id: &str) -> Result<()> {
        let mut installed = self.installed();
        if installed.remove(id).is_none() {
            return Err(vm_err!(codes::NOT_FOUND, "{} is not installed (a local folder's stays there)", id));
        }
        if let Some(folder) = self.folder(id) {
            Disposal::delete(&[&folder])?;
        }
        self.save(&installed)
    }
}

/// Installs the parts `spec` needs that are not here (or not in a version it allows) – its modules, key binding sets and
/// look – from the first of `catalogs` that has each in an allowed version (the sources in their order, the official
/// catalog last: references fall back to it, plan B141). What no catalog has is logged, and an error when nothing at
/// all could be installed; the recipe's page says what is still missing. Returns the ids installed.
pub fn install_recipe_parts(
    paths: &Paths,
    spec: &crate::composition::RecipeSpec,
    catalogs: &[(crate::recipe_sources::RecipeSource, crate::recipe_sources::RecipeCatalog)],
    log: &mut dyn FnMut(&str),
) -> Result<Vec<String>> {
    let settings = Settings::load(paths);
    let modules = crate::modules::ModuleStore::new(paths.clone());
    let keymaps = crate::keymaps::KeymapStore::new(paths.clone());
    let looks = crate::looks::LookStore::new(paths.clone());
    let here_modules = modules.list(&settings);
    let here_keymaps = keymaps.list(&settings);
    let here_looks = looks.list(&settings);
    let mut wanted: Vec<(PartKind, &crate::composition::PartRef)> = Vec::new();
    for r in &spec.modules {
        if !here_modules.iter().any(|m| m.platform == spec.platform && m.reference() == r.id && r.allows(&m.version)) {
            wanted.push((PartKind::Module, r));
        }
    }
    for r in &spec.keymaps {
        if !here_keymaps.iter().any(|k| k.reference() == r.id && r.allows(&k.version)) {
            wanted.push((PartKind::Keymap, r));
        }
    }
    if let Some(r) = &spec.look {
        if !here_looks.iter().any(|l| l.reference() == r.id && r.allows(&l.version)) {
            wanted.push((PartKind::Look, r));
        }
    }
    let mut installed = Vec::new();
    let mut missing = Vec::new();
    for (kind, r) in wanted {
        let Some(id) = kind.id_of(&r.id, Some(spec.platform)) else { continue };
        let found = catalogs.iter().find(|(_, c)| match kind {
            PartKind::Module => c.modules.iter().any(|m| m.id == id && r.allows(&m.version)),
            PartKind::Keymap => c.keymaps.iter().any(|k| k.id == id && r.allows(&k.version)),
            PartKind::Look => c.looks.iter().any(|l| l.id == id && r.allows(&l.version)),
        });
        let Some((source, catalog)) = found else {
            log(&format!("{}: in none of your sources", r.label()));
            missing.push(r.label());
            continue;
        };
        let client = SourceClient::new(source.clone(), None);
        let done = match kind {
            PartKind::Module => modules.install(&id, catalog, &client).map(|m| m.title),
            PartKind::Keymap => keymaps.install(&id, catalog, &client).map(|k| k.title),
            PartKind::Look => looks.install(&id, catalog, &client).map(|l| l.title),
        };
        match done {
            Ok(title) => {
                log(&format!("Installed {title} ({}) from {}", r.id, source.display_name()));
                installed.push(id);
            }
            Err(e) => {
                log(&format!("{}: {}", r.label(), e.message()));
                missing.push(r.label());
            }
        }
    }
    if installed.is_empty() && !missing.is_empty() {
        return Err(vm_err!("Not installed: {}", missing.join(", ")));
    }
    Ok(installed)
}

/// The enabled remote sources' catalogs, read now, the official one last when it is not among them (for
/// `install_recipe_parts` outside the core service, which keeps the catalogs it fetched)
pub fn fetch_part_catalogs(
    settings: &Settings,
) -> Vec<(crate::recipe_sources::RecipeSource, crate::recipe_sources::RecipeCatalog)> {
    let mut sources: Vec<crate::recipe_sources::RecipeSource> = settings
        .recipe_sources()
        .into_iter()
        .filter(|s| s.is_enabled() && s.kind != RecipeSourceKind::Folder)
        .collect();
    if !sources.iter().any(crate::recipe_sources::RecipeSource::is_official) {
        sources.push(crate::recipe_sources::RecipeSource::official());
    }
    sources.into_iter().filter_map(|s| SourceClient::new(s.clone(), None).catalog().ok().map(|c| (s, c))).collect()
}

/// An `.installed.json` of parts (tests: what omaflake writes matches its schema)
#[cfg(test)]
pub(crate) fn sample_file() -> Vec<u8> {
    let parts = BTreeMap::from([(
        "omaflake.look.omaflake".to_string(),
        InstalledPart {
            source: "github:omaflake/catalog@main".into(),
            version: "1.0.0".into(),
            hash: "ab".into(),
            installed: Utc::now(),
        },
    )]);
    PartStore::FILE_FORMAT.encode(&InstalledPartsFile { parts }).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The real catalog next to this checkout (../catalog) as a source: a recipe's parts are installed from it – every
    /// file checked against catalog.json –, and then nothing it is made of is missing. Skipped without the checkout.
    #[test]
    fn a_recipes_parts_from_the_catalog_checkout() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../../catalog");
        let Ok(data) = fs::read(root.join("catalog.json")) else { return };
        let catalog = crate::recipe_sources::RecipeCatalog::decode(&data).unwrap();
        let (_t, paths) = crate::paths::tests::temp_paths();
        let mut s = Settings::load(&paths);
        s.sources = Some(vec![]);
        s.save(&paths).unwrap();
        let source = crate::recipe_sources::RecipeSource::new(
            RecipeSourceKind::Folder,
            &root.to_string_lossy(),
            None,
            None,
            None,
        );
        let spec = crate::composition::RecipeSpec::read(&root.join("recipes/omaflake/nixos/neovim-research")).unwrap();
        let mut lines = Vec::new();
        let installed =
            install_recipe_parts(&paths, &spec, &[(source, catalog)], &mut |l| lines.push(l.to_string())).unwrap();
        // ten modules, the key bindings, the look
        assert_eq!(installed.len(), 12, "{lines:?}");
        let s = Settings::load(&paths);
        let modules = crate::modules::ModuleStore::new(paths.clone());
        let c = crate::composition::Composition::of(
            &spec,
            &modules.list(&s),
            &crate::keymaps::KeymapStore::new(paths.clone()).list(&s),
            &modules.capabilities(&s),
        );
        assert!(c.issues.is_empty(), "{:?}", c.issues);
        assert_eq!(s.default_look.as_deref(), Some("omaflake.look.omaflake"), "the first look became the default");
        assert!(modules.capabilities(&s)["desktop"].exclusive, "the catalog's capabilities are kept");
        // a second time: everything is here
        assert!(install_recipe_parts(&paths, &spec, &[], &mut |_| {}).unwrap().is_empty());
    }

    #[test]
    fn ids_references_and_paths() {
        let m = PartKind::Module;
        assert_eq!(m.id_of("omaflake/neovim", Some(GuestKind::Nixos)).as_deref(), Some("omaflake.nixos.neovim"));
        assert_eq!(m.id_of("omaflake.macos.shell", None).as_deref(), Some("omaflake.macos.shell"));
        assert!(m.id_of("omaflake/neovim", None).is_none(), "a module needs its platform");
        assert_eq!(m.reference("omaflake.nixos.neovim"), "omaflake/neovim");
        assert_eq!(m.catalog_path("omaflake.nixos.neovim").as_deref(), Some("modules/omaflake/nixos/neovim"));
        assert!(m.parse_id("omaflake.look.x").is_none());
        let l = PartKind::Look;
        assert_eq!(l.id_of("omaflake/catppuccin", None).as_deref(), Some("omaflake.look.catppuccin"));
        assert!(l.id_of("catppuccin", None).is_none() && l.id_of("../x", None).is_none());
        assert!(l.parse_id("omaflake.nixos.x").is_none());
        assert_eq!(
            PartKind::Keymap.catalog_path("omaflake.keymap.default").as_deref(),
            Some("keymaps/omaflake/default")
        );
        assert!(
            PartKind::Module.allowed("sub/x.nix")
                && !PartKind::Module.allowed("../x")
                && !PartKind::Look.allowed("x.sh")
        );
    }
}
