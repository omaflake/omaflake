// SPDX-License-Identifier: AGPL-3.0-or-later
//! Modules (plan B141, B142): what is in a recipe – a desktop, a terminal, an editor, LaTeX … – as Nix (`default.nix`)
//! with a description (`module.toml`, schema module/v2): its version, the capabilities it provides and needs, the
//! modules it enhances, and the actions people can bind keys to, each with a default key. They come from the sources'
//! catalogs (`modules/<author>/<platform>/<name>/`) and are installed into `<data>/modules/` (`parts::PartStore`), or
//! used as they are on disk from a local folder source. Modules are code: only reviewed sources should provide them.
//!
//! Capabilities are a short, fixed list in each catalog (`capabilities.toml`): what "needs a pdf-viewer" means, and
//! which are exclusive (two providers in one recipe are incompatible). omaflake keeps the last catalog's list it saw
//! (`<data>/modules/.capabilities.json`) and reads a local folder's `capabilities.toml`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{codes, IoContext, Result};
use crate::file_format::{write_atomic, FileFormat};
use crate::models::GuestKind;
use crate::parts::{CatalogPart, PartKind, PartStore};
use crate::paths::{Paths, Settings};
use crate::recipe_sources::{CatalogFile, RecipeCatalog, RecipeSourceKind, SourceClient};
use crate::vm_err;

/// One key or mode, or several: module.toml and keymap.toml write a single one as a string, several as a list
pub mod one_or_many {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    #[derive(Deserialize)]
    #[serde(untagged)]
    enum Form {
        One(String),
        Many(Vec<String>),
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
        Ok(match Form::deserialize(d)? {
            Form::One(s) => vec![s],
            Form::Many(v) => v,
        })
    }

    pub fn serialize<S: Serializer>(v: &[String], s: S) -> Result<S::Ok, S::Error> {
        match v {
            [one] => one.serialize(s),
            _ => v.serialize(s),
        }
    }
}

/// An action a module offers for key bindings (module.toml `[actions.<name>]`, module/v2: plan B147)
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModuleAction {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_de: Option<String>,
    /// The default keys ("Super+Return", "Space f f"; several: each does it)
    #[serde(with = "one_or_many")]
    pub key: Vec<String>,
    /// Only in these modes of the module (Zellij's "pane"); none: whenever its program has the keys
    #[serde(default, with = "one_or_many", skip_serializing_if = "Vec::is_empty")]
    pub mode: Vec<String>,
    /// The mode the program is in afterwards; None: it stays
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub after: Option<String>,
    /// What it does in the program's own words (Zellij's `NewPane "Down"`, a Hyprland dispatcher)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub does: Option<String>,
    /// The program's own key, shown for reference – key binding sets do not change it
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub own: bool,
    /// Works everywhere on the desktop (a launcher)
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub desktop: bool,
    /// Bound inside another module's program ("omaflake/neovim")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
}

impl ModuleAction {
    /// The title in `language` when the module has it
    pub fn title_in(&self, language: Option<&str>) -> &str {
        match (language, &self.title_de) {
            (Some("de"), Some(t)) => t,
            _ => &self.title,
        }
    }
}

/// A mode of a module's program (module.toml `[modes.<name>]`): its keys differ there
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct ModuleMode {
    pub title: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title_de: Option<String>,
    /// The program starts in it
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub start: bool,
}

impl ModuleMode {
    pub fn title_in(&self, language: Option<&str>) -> &str {
        match (language, &self.title_de) {
            (Some("de"), Some(t)) => t,
            _ => &self.title,
        }
    }
}

/// One module in catalog.json (catalog v4)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogModule {
    /// "<author>.<platform>.<name>"
    pub id: String,
    pub author: String,
    pub platform: GuestKind,
    pub name: String,
    pub path: String,
    pub version: String,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptions: Option<BTreeMap<String, String>>,
    #[serde(default)]
    pub tags: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default)]
    pub provides: Vec<String>,
    #[serde(default)]
    pub needs: Vec<String>,
    #[serde(default)]
    pub enhances: Vec<String>,
    #[serde(default)]
    pub unfree: bool,
    pub hash: String,
    pub files: Vec<CatalogFile>,
}

impl CatalogModule {
    pub fn part(&self) -> CatalogPart<'_> {
        CatalogPart { id: &self.id, path: &self.path, version: &self.version, hash: &self.hash, files: &self.files }
    }

    pub fn description_in(&self, language: Option<&str>) -> &str {
        language.and_then(|l| self.descriptions.as_ref()?.get(l)).map(String::as_str).unwrap_or(&self.description)
    }

    /// "<author>/<name>"
    pub fn reference(&self) -> String {
        format!("{}/{}", self.author, self.name)
    }
}

/// A module omaflake can use: installed, or in a local folder source
#[derive(Debug, Clone, PartialEq)]
pub struct Module {
    /// "<author>.<platform>.<name>"
    pub id: String,
    pub author: String,
    pub platform: GuestKind,
    pub name: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub descriptions: BTreeMap<String, String>,
    pub tags: Vec<String>,
    pub provides: Vec<String>,
    pub needs: Vec<String>,
    /// "<author>/<name>" of modules it adds to when they are in the recipe too
    pub enhances: Vec<String>,
    /// Capabilities whose programs it runs inside (for key overlaps)
    pub runs_in: Vec<String>,
    /// All its keys work on the whole desktop
    pub desktop: bool,
    /// An editor's keys: the key rule does not apply
    pub own_keys: bool,
    /// Not free software
    pub unfree: bool,
    pub actions: BTreeMap<String, ModuleAction>,
    pub modes: BTreeMap<String, ModuleMode>,
    /// `RecipeSource::id` it came from
    pub source: String,
    /// The catalog's content hash it was installed at; None for a local folder's
    pub hash: Option<String>,
    pub folder: PathBuf,
}

impl Module {
    /// "<author>/<name>" – how recipes name it
    pub fn reference(&self) -> String {
        format!("{}/{}", self.author, self.name)
    }

    pub fn description_in(&self, language: Option<&str>) -> &str {
        language.and_then(|l| self.descriptions.get(l)).map(String::as_str).unwrap_or(&self.description)
    }
}

/// module.toml as read
#[derive(Debug, Deserialize)]
struct ModuleToml {
    #[serde(rename = "$schema", default)]
    schema: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    tags: Vec<String>,
    #[serde(default)]
    provides: Vec<String>,
    #[serde(default)]
    needs: Vec<String>,
    #[serde(default)]
    enhances: Vec<String>,
    #[serde(default)]
    runs_in: Vec<String>,
    #[serde(default)]
    desktop: bool,
    #[serde(default)]
    own_keys: bool,
    #[serde(default)]
    unfree: bool,
    #[serde(default)]
    actions: BTreeMap<String, ModuleAction>,
    #[serde(default)]
    modes: BTreeMap<String, ModuleMode>,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

/// One capability (capabilities.toml)
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Capability {
    #[serde(default)]
    pub exclusive: bool,
    #[serde(default)]
    pub description: String,
}

/// The modules omaflake can use
#[derive(Debug, Clone)]
pub struct ModuleStore {
    pub paths: Paths,
}

impl ModuleStore {
    /// module.toml's version this omaflake reads (`"$schema" = "https://omaflake.dev/schema/module/v2.json"`; v2:
    /// modes, several keys and modes, `after`, `does`, `own`, `title_de` – plan B147)
    pub const MODULE_VERSION: usize = 2;

    pub fn new(paths: Paths) -> Self {
        ModuleStore { paths }
    }

    fn parts(&self) -> PartStore {
        PartStore::new(self.paths.clone(), PartKind::Module)
    }

    /// Reads a module's folder: module.toml, checked to be a module this omaflake reads, and its default.nix there
    pub fn read(id: &str, folder: &Path, source: String, hash: Option<String>) -> Result<Module> {
        let file = folder.join("module.toml");
        let text = fs::read_to_string(&file).with_path(&file)?;
        let m: ModuleToml =
            toml::from_str(&text).map_err(|e| vm_err!("{}: not a module ({})", file.display(), e.message()))?;
        match FileFormat::parse(&m.schema) {
            Some((t, v)) if t == "module" => {
                FileFormat::check(v, Self::MODULE_VERSION, Self::MODULE_VERSION, "module.toml")?;
            }
            _ => return Err(vm_err!("{}: not a module (\"$schema\" = \"{}\")", file.display(), m.schema)),
        }
        if !folder.join("default.nix").exists() {
            return Err(vm_err!("{}: a module needs default.nix", folder.display()));
        }
        let Some((author, Some(platform), name)) = PartKind::Module.parse_id(id) else {
            return Err(vm_err!("Not a module: {id}"));
        };
        Ok(Module {
            id: id.to_string(),
            author,
            platform,
            title: if m.title.is_empty() { name.clone() } else { m.title },
            name,
            version: m.version,
            description: m.description,
            descriptions: m
                .rest
                .iter()
                .filter_map(|(k, v)| Some((k.strip_prefix("description_")?.to_string(), v.as_str()?.to_string())))
                .collect(),
            tags: m.tags,
            provides: m.provides,
            needs: m.needs,
            enhances: m.enhances,
            runs_in: m.runs_in,
            desktop: m.desktop,
            own_keys: m.own_keys,
            unfree: m.unfree,
            actions: m.actions,
            modes: m.modes,
            source,
            hash,
            folder: folder.to_path_buf(),
        })
    }

    /// Every module omaflake can use, and those that could not be read: (id, message)
    pub fn list_with_errors(&self, settings: &Settings) -> (Vec<Module>, Vec<(String, String)>) {
        let mut out = Vec::new();
        let mut errors = Vec::new();
        for e in self.parts().entries(settings) {
            match Self::read(&e.id, &e.folder, e.source, e.installed.map(|i| i.hash)) {
                Ok(m) => out.push(m),
                Err(err) => errors.push((e.id, err.to_string())),
            }
        }
        (out, errors)
    }

    pub fn list(&self, settings: &Settings) -> Vec<Module> {
        self.list_with_errors(settings).0
    }

    /// A module of `platform` by "<author>/<name>" or its id
    pub fn find(&self, settings: &Settings, reference: &str, platform: GuestKind) -> Option<Module> {
        let id = PartKind::Module.id_of(reference, Some(platform))?;
        self.list(settings).into_iter().find(|m| m.id == id)
    }

    pub fn installed(&self) -> BTreeMap<String, crate::parts::InstalledPart> {
        self.parts().installed()
    }

    /// Downloads a module from a catalog and keeps it (and the catalog's capabilities)
    pub fn install(&self, id: &str, catalog: &RecipeCatalog, client: &SourceClient) -> Result<Module> {
        let Some(entry) = catalog.modules.iter().find(|m| m.id == id) else {
            return Err(vm_err!(codes::NOT_FOUND, "{} is not in {}", id, client.source.display_name()));
        };
        let source = client.source.id();
        let folder = self
            .parts()
            .install(&entry.part(), client, |work| Self::read(id, work, source.clone(), None).map(|_| ()))?;
        self.remember_capabilities(&catalog.capabilities)?;
        Self::read(id, &folder, source, Some(entry.hash.clone()))
    }

    /// Removes an installed module
    pub fn remove(&self, id: &str) -> Result<()> {
        self.parts().remove(id)
    }

    fn capabilities_file(&self) -> PathBuf {
        self.parts().dir().join(".capabilities.json")
    }

    /// Keeps a catalog's capabilities (the official catalog's win over others' for the same name)
    pub fn remember_capabilities(&self, from: &BTreeMap<String, Capability>) -> Result<()> {
        if from.is_empty() {
            return Ok(());
        }
        let mut known = self.saved_capabilities();
        for (k, v) in from {
            known.insert(k.clone(), v.clone());
        }
        let file = self.capabilities_file();
        if let Some(dir) = file.parent() {
            fs::create_dir_all(dir).with_path(dir)?;
        }
        write_atomic(&file, &crate::json::value_to_pretty(&serde_json::to_value(&known)?, false))
    }

    fn saved_capabilities(&self) -> BTreeMap<String, Capability> {
        fs::read(self.capabilities_file()).ok().and_then(|d| serde_json::from_slice(&d).ok()).unwrap_or_default()
    }

    /// The capabilities omaflake knows: the local folders' capabilities.toml, then those kept from catalogs
    pub fn capabilities(&self, settings: &Settings) -> BTreeMap<String, Capability> {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            capabilities: BTreeMap<String, Capability>,
        }
        let mut out = self.saved_capabilities();
        for s in settings.recipe_sources() {
            if s.is_enabled() && s.kind == RecipeSourceKind::Folder {
                let path = Path::new(&s.location).join("capabilities.toml");
                if let Some(f) = fs::read_to_string(&path).ok().and_then(|t| toml::from_str::<File>(&t).ok()) {
                    out.extend(f.capabilities);
                }
            }
        }
        out
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;
    use crate::recipe_sources::{RecipeSource, RecipeSourceKind};

    pub(crate) fn module_toml(provides: &[&str], needs: &[&str], actions: &[(&str, &str)]) -> String {
        let list = |l: &[&str]| l.iter().map(|x| format!("\"{x}\"")).collect::<Vec<_>>().join(", ");
        let mut s = format!(
            "\"$schema\" = \"https://omaflake.dev/schema/module/v2.json\"\nversion = \"1.2.0\"\ntitle = \"T\"\n\
             description = \"D\"\ndescription_de = \"De\"\nprovides = [{}]\nneeds = [{}]\n",
            list(provides),
            list(needs)
        );
        for (name, key) in actions {
            s.push_str(&format!("[actions.{name}]\ntitle = \"{name}\"\nkey = \"{key}\"\n"));
        }
        s
    }

    /// A module folder under `root` (a local source): modules/<author>/<platform>/<name>/
    pub(crate) fn put_module(root: &Path, author: &str, platform: &str, name: &str, toml: &str) -> PathBuf {
        let f = root.join("modules").join(author).join(platform).join(name);
        fs::create_dir_all(&f).unwrap();
        fs::write(f.join("module.toml"), toml).unwrap();
        fs::write(f.join("default.nix"), "{ ... }: { }\n").unwrap();
        f
    }

    #[test]
    fn local_modules_and_capabilities() {
        let (_t, paths) = temp_paths();
        let src = tempfile::tempdir().unwrap();
        put_module(src.path(), "alice", "nixos", "zathura", &module_toml(&["pdf-viewer"], &[], &[]));
        put_module(src.path(), "alice", "nixos", "latex", &module_toml(&[], &["pdf-viewer"], &[("build", "Ctrl+b")]));
        put_module(src.path(), "alice", "macos", "broken", "title = 1");
        fs::write(
            src.path().join("capabilities.toml"),
            "[capabilities.pdf-viewer]\nexclusive = false\ndescription = \"x\"\n",
        )
        .unwrap();
        let mut s = Settings::load(&paths);
        s.sources =
            Some(vec![RecipeSource::new(RecipeSourceKind::Folder, &src.path().to_string_lossy(), None, None, None)]);
        s.save(&paths).unwrap();
        let store = ModuleStore::new(paths.clone());
        let (modules, errors) = store.list_with_errors(&s);
        assert_eq!(
            modules.iter().map(|m| m.id.as_str()).collect::<Vec<_>>(),
            ["alice.nixos.latex", "alice.nixos.zathura"]
        );
        assert_eq!(errors.len(), 1);
        let latex = store.find(&s, "alice/latex", GuestKind::Nixos).unwrap();
        assert_eq!((latex.version.as_str(), latex.needs.clone()), ("1.2.0", vec!["pdf-viewer".to_string()]));
        assert_eq!(latex.actions["build"].key, ["Ctrl+b"]);
        assert_eq!(latex.description_in(Some("de")), "De");
        assert!(store.find(&s, "alice/latex", GuestKind::Macos).is_none());
        assert!(!store.capabilities(&s)["pdf-viewer"].exclusive);
        store
            .remember_capabilities(&BTreeMap::from([(
                "desktop".to_string(),
                Capability { exclusive: true, description: "d".into() },
            )]))
            .unwrap();
        assert!(store.capabilities(&s)["desktop"].exclusive);
    }
}
