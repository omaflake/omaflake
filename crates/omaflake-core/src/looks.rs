// SPDX-License-Identifier: AGPL-3.0-or-later
//! Looks (plan B140, B141): how a guest looks – colours and a few choices, as data (`look.toml`, schema look/v1). They
//! come from the sources' catalogs (`looks/<author>/<name>/look.toml`) and are installed into
//! `<data>/looks/<author>.<name>/` (`parts::PartStore`); a local folder source's looks are used as they are on disk. Nothing is built
//! in: the image flake only turns a look's data into each program's configuration (nix/looks/adapter.nix), and
//! omaflake hands it the chosen look's data at the build (`GuestSettings::look`).
//!
//! There is always a default look while one is installed (settings `defaultLook`, else the first). The Doctor wants
//! at least one (`PrerequisiteItem::Look`) and recommends omaflake's own and Catppuccin.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{codes, IoContext, Result};
use crate::file_format::FileFormat;
use crate::parts::{CatalogPart, PartKind, PartStore};
use crate::paths::{Paths, Settings};
use crate::recipe_sources::{CatalogFile, RecipeCatalog, SourceClient};
use crate::vm_err;

/// The main colours of a look in the catalog, for a preview without downloading it
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Swatches {
    #[serde(default)]
    pub background: String,
    #[serde(default)]
    pub surface: String,
    #[serde(default)]
    pub text: String,
    #[serde(default)]
    pub accent: String,
    #[serde(default)]
    pub urgent: String,
    /// The terminal's 16 colours
    #[serde(default)]
    pub terminal: Vec<String>,
}

/// One look in catalog.json (catalog v4)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogLook {
    /// "<author>.look.<name>"
    pub id: String,
    pub author: String,
    pub name: String,
    pub path: String,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptions: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tags: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    /// "dark" | "light"; None for a plain look
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub appearance: Option<String>,
    #[serde(default)]
    pub plain: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub swatches: Option<Swatches>,
    /// The author's version ("1.0.0")
    pub version: String,
    /// The files' content hash: an installed look with another has an update
    pub hash: String,
    pub files: Vec<CatalogFile>,
}

impl CatalogLook {
    pub fn part(&self) -> CatalogPart<'_> {
        CatalogPart { id: &self.id, path: &self.path, version: &self.version, hash: &self.hash, files: &self.files }
    }

    pub fn description_in(&self, language: Option<&str>) -> &str {
        language.and_then(|l| self.descriptions.as_ref()?.get(l)).map(String::as_str).unwrap_or(&self.description)
    }
}

/// A look omaflake can use: installed, or in a local folder source
#[derive(Debug, Clone, PartialEq)]
pub struct Look {
    /// "<author>.look.<name>"
    pub id: String,
    pub author: String,
    pub name: String,
    pub title: String,
    pub description: String,
    pub descriptions: BTreeMap<String, String>,
    pub tags: Vec<String>,
    pub appearance: Option<String>,
    pub plain: bool,
    pub swatches: Option<Swatches>,
    pub homepage: Option<String>,
    /// `RecipeSource::id` it came from (a local folder's id for its live looks)
    pub source: String,
    /// The author's version (look.toml's `version`)
    pub version: String,
    /// The catalog's content hash it was installed at; None for a local folder's look
    pub hash: Option<String>,
    pub folder: PathBuf,
    /// look.toml as data – what the image flake gets
    pub data: Value,
}

impl Look {
    /// "<author>/<name>" – how people name it
    pub fn reference(&self) -> String {
        format!("{}/{}", self.author, self.name)
    }

    pub fn description_in(&self, language: Option<&str>) -> &str {
        language.and_then(|l| self.descriptions.get(l)).map(String::as_str).unwrap_or(&self.description)
    }
}

/// The looks omaflake can use: installed from remote sources (`<data>/looks/`) and those in enabled local folders
#[derive(Debug, Clone)]
pub struct LookStore {
    pub paths: Paths,
}

impl LookStore {
    /// look.toml's version this omaflake reads (`"$schema" = "https://omaflake.dev/schema/look/v1.json"`)
    pub const LOOK_VERSION: usize = 1;
    /// What the Doctor recommends when no look is installed: omaflake's own and Catppuccin (plan B141)
    pub const RECOMMENDED: [&'static str; 2] = ["omaflake.look.omaflake", "omaflake.look.catppuccin"];

    pub fn new(paths: Paths) -> Self {
        LookStore { paths }
    }

    fn parts(&self) -> PartStore {
        PartStore::new(self.paths.clone(), PartKind::Look)
    }

    pub fn dir(&self) -> PathBuf {
        self.parts().dir()
    }

    /// "<author>.look.<name>" from "<author>/<name>" or the id itself
    pub fn id_of(reference: &str) -> Option<String> {
        PartKind::Look.id_of(reference, None)
    }

    /// Installed looks: id → (source, version, hash, when)
    pub fn installed(&self) -> BTreeMap<String, crate::parts::InstalledPart> {
        self.parts().installed()
    }

    /// Reads a look's folder: look.toml as data, checked to be a look this omaflake reads
    pub fn read(id: &str, folder: &Path, source: String, hash: Option<String>) -> Result<Look> {
        let file = folder.join("look.toml");
        let text = fs::read_to_string(&file).with_path(&file)?;
        let table: toml::Table =
            toml::from_str(&text).map_err(|e| vm_err!("{}: not a look ({})", file.display(), e.message()))?;
        let data = serde_json::to_value(&table)?;
        let schema = data.get("$schema").and_then(Value::as_str).unwrap_or_default();
        match FileFormat::parse(schema) {
            Some((t, v)) if t == "look" => FileFormat::check(v, Self::LOOK_VERSION, Self::LOOK_VERSION, "look.toml")?,
            _ => return Err(vm_err!("{}: not a look (\"$schema\" = \"{schema}\")", file.display())),
        };
        let plain = data.get("plain").and_then(Value::as_bool).unwrap_or(false);
        if !plain
            && !(data.get("colors").is_some_and(Value::is_object) && data.get("terminal").is_some_and(Value::is_object))
        {
            return Err(vm_err!("{}: a look needs [colors] and [terminal]", file.display()));
        }
        let (author, name) = id.split_once(".look.").unwrap_or(("", id));
        let s = |k: &str| data.get(k).and_then(Value::as_str).map(str::to_string);
        let colors = data.get("colors");
        let color = |k: &str| colors.and_then(|c| c.get(k)).and_then(Value::as_str).unwrap_or_default().to_string();
        let terminal = |k: &str| -> Vec<String> {
            data.get("terminal")
                .and_then(|t| t.get(k))
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default()
        };
        Ok(Look {
            id: id.to_string(),
            author: author.to_string(),
            name: name.to_string(),
            title: s("title").unwrap_or_else(|| name.to_string()),
            description: s("description").unwrap_or_default(),
            descriptions: data
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| {
                            Some((k.strip_prefix("description_")?.to_string(), v.as_str()?.to_string()))
                        })
                        .collect()
                })
                .unwrap_or_default(),
            tags: data
                .get("tags")
                .and_then(Value::as_array)
                .map(|a| a.iter().filter_map(|x| x.as_str().map(str::to_string)).collect())
                .unwrap_or_default(),
            appearance: s("appearance"),
            plain,
            swatches: (!plain).then(|| Swatches {
                background: color("background"),
                surface: color("surface"),
                text: color("text"),
                accent: color("accent"),
                urgent: color("urgent"),
                terminal: [terminal("normal"), terminal("bright")].concat(),
            }),
            homepage: s("homepage"),
            source,
            version: s("version").unwrap_or_default(),
            hash,
            folder: folder.to_path_buf(),
            data,
        })
    }

    /// Every look omaflake can use: the enabled local folders' first (in their order), then the installed ones; the
    /// first with an id wins. Looks that cannot be read are left out (`unreadable` names them).
    pub fn list(&self, settings: &Settings) -> Vec<Look> {
        self.list_with_errors(settings).0
    }

    /// `list`, and the looks that could not be read: (id, message)
    pub fn list_with_errors(&self, settings: &Settings) -> (Vec<Look>, Vec<(String, String)>) {
        let mut out: Vec<Look> = Vec::new();
        let mut errors = Vec::new();
        let mut add = |r: Result<Look>, id: &str| match r {
            Ok(l) if !out.iter().any(|x| x.id == l.id) => out.push(l),
            Ok(_) => {}
            Err(e) => errors.push((id.to_string(), e.to_string())),
        };
        for e in self.parts().entries(settings) {
            add(Self::read(&e.id, &e.folder, e.source, e.installed.map(|i| i.hash)), &e.id);
        }
        (out, errors)
    }

    /// A look by its id, "<author>/<name>", its name or its title (any case)
    pub fn find(&self, settings: &Settings, reference: &str) -> Option<Look> {
        let all = self.list(settings);
        let id = Self::id_of(reference);
        all.iter()
            .find(|l| Some(&l.id) == id.as_ref())
            .or_else(|| {
                all.iter().find(|l| l.name.eq_ignore_ascii_case(reference) || l.title.eq_ignore_ascii_case(reference))
            })
            .cloned()
    }

    /// The default look: the settings' `defaultLook` while it can be used, else the first – None only without any
    pub fn default_look(&self, settings: &Settings) -> Option<Look> {
        let all = self.list(settings);
        settings
            .default_look
            .as_ref()
            .and_then(|id| all.iter().find(|l| &l.id == id).cloned())
            .or_else(|| all.into_iter().next())
    }

    /// The look a recipe suggests (recipe.toml `look`) when it is here in a version the recipe allows (plan B142)
    pub fn suggested(&self, settings: &Settings, spec: &crate::composition::RecipeSpec) -> Option<Look> {
        let r = spec.look.as_ref()?;
        self.list(settings).into_iter().find(|l| l.reference() == r.id && r.allows(&l.version))
    }

    /// Makes `id` the default look
    pub fn set_default(&self, id: &str) -> Result<()> {
        let mut s = Settings::load(&self.paths);
        if !self.list(&s).iter().any(|l| l.id == id) {
            return Err(vm_err!(codes::NOT_FOUND, "No look \"{}\"", id));
        }
        s.default_look = Some(id.to_string());
        s.save(&self.paths)
    }

    /// Downloads a look from a catalog (its files checked by size and SHA-256, `parts`) and keeps it; a newer version
    /// replaces the installed one. The first look installed becomes the default.
    pub fn install(&self, id: &str, catalog: &RecipeCatalog, client: &SourceClient) -> Result<Look> {
        let Some(entry) = catalog.looks.iter().find(|l| l.id == id) else {
            return Err(vm_err!(codes::NOT_FOUND, "{} is not in {}", id, client.source.display_name()));
        };
        let source = client.source.id();
        let folder = self
            .parts()
            .install(&entry.part(), client, |work| Self::read(id, work, source.clone(), None).map(|_| ()))?;
        let look = Self::read(id, &folder, source, Some(entry.hash.clone()))?;
        let s = Settings::load(&self.paths);
        if s.default_look.as_ref().is_none_or(|d| !self.list(&s).iter().any(|l| &l.id == d)) {
            self.set_default(&look.id)?;
        }
        Ok(look)
    }

    /// Removes an installed look; when it was the default, another one becomes it
    pub fn remove(&self, id: &str) -> Result<()> {
        self.parts().remove(id)?;
        let mut s = Settings::load(&self.paths);
        if s.default_look.as_deref() == Some(id) {
            s.default_look = self.list(&s).first().map(|l| l.id.clone());
            s.save(&self.paths)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;
    use crate::recipe_sources::{RecipeSource, RecipeSourceKind};

    const LOOK: &str = r##""$schema" = "https://omaflake.dev/schema/look/v1.json"
author = "alice"
license = "MIT"
title = "Nord"
description = "Arctic."
description_de = "Arktisch."
tags = ["dark"]
appearance = "dark"
[colors]
background = "#2e3440"
surface = "#3b4252"
selection = "#434c5e"
text = "#eceff4"
subtext = "#d8dee9"
muted = "#4c566a"
accent = "#88c0d0"
urgent = "#bf616a"
[terminal]
normal = ["#3b4252", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#88c0d0", "#e5e9f0"]
bright = ["#4c566a", "#bf616a", "#a3be8c", "#ebcb8b", "#81a1c1", "#b48ead", "#8fbcbb", "#eceff4"]
"##;

    fn source_with(dir: &Path, looks: &[(&str, &str, &str)]) {
        for (author, name, text) in looks {
            let f = dir.join("looks").join(author).join(name);
            fs::create_dir_all(&f).unwrap();
            fs::write(f.join("look.toml"), text).unwrap();
        }
    }

    #[test]
    fn local_folder_looks_install_remove_and_the_default() {
        let (_t, paths) = temp_paths();
        let src = tempfile::tempdir().unwrap();
        source_with(src.path(), &[("alice", "nord", LOOK), ("alice", "broken", "title = 1")]);
        let mut s = Settings::load(&paths);
        let folder = RecipeSource::new(RecipeSourceKind::Folder, &src.path().to_string_lossy(), None, None, None);
        s.sources = Some(vec![folder.clone()]);
        s.save(&paths).unwrap();
        let store = LookStore::new(paths.clone());
        let (looks, errors) = store.list_with_errors(&s);
        assert_eq!(looks.iter().map(|l| l.id.as_str()).collect::<Vec<_>>(), ["alice.look.nord"]);
        assert_eq!(errors.len(), 1, "the broken one is named");
        let nord = &looks[0];
        assert_eq!((nord.title.as_str(), nord.reference().as_str()), ("Nord", "alice/nord"));
        assert_eq!(nord.description_in(Some("de")), "Arktisch.");
        assert_eq!(nord.swatches.as_ref().unwrap().terminal.len(), 16);
        assert_eq!(nord.data["colors"]["accent"], "#88c0d0");
        // The default: the first, until one is set
        assert_eq!(store.default_look(&s).map(|l| l.id), Some("alice.look.nord".into()));
        assert_eq!(store.find(&s, "alice/nord").map(|l| l.id), Some("alice.look.nord".into()));
        assert_eq!(store.find(&s, "NORD").map(|l| l.id), Some("alice.look.nord".into()));
        assert!(store.set_default("alice.look.none").is_err());

        // Installed from a catalog (the folder as a client: same files, same checks)
        let data = fs::read(src.path().join("looks/alice/nord/look.toml")).unwrap();
        let sha = {
            use sha2::{Digest, Sha256};
            hex::encode(Sha256::digest(&data))
        };
        let entry = |path: &str, files: Vec<CatalogFile>| CatalogLook {
            id: "alice.look.nord".into(),
            author: "alice".into(),
            name: "nord".into(),
            path: path.into(),
            title: "Nord".into(),
            description: "Arctic.".into(),
            descriptions: None,
            tags: None,
            license: None,
            appearance: Some("dark".into()),
            plain: false,
            homepage: None,
            swatches: None,
            version: "1.0.0".into(),
            hash: "h1".into(),
            files,
        };
        let file = CatalogFile { path: "look.toml".into(), size: data.len() as u64, sha256: sha.clone() };
        let catalog = |l: CatalogLook| RecipeCatalog { looks: vec![l], ..RecipeCatalog::empty("x") };
        let client = SourceClient::new(folder.clone(), None);
        // Untrusted entries are refused: a path outside its folder, an unknown file, a wrong hash
        assert!(store.install("alice.look.nord", &catalog(entry("recipes/x", vec![file.clone()])), &client).is_err());
        let odd = CatalogFile { path: "run.sh".into(), size: 1, sha256: sha.clone() };
        assert!(store
            .install("alice.look.nord", &catalog(entry("looks/alice/nord", vec![file.clone(), odd])), &client)
            .is_err());
        let wrong = CatalogFile { sha256: "00".repeat(32), ..file.clone() };
        assert!(store.install("alice.look.nord", &catalog(entry("looks/alice/nord", vec![wrong])), &client).is_err());
        assert!(store.installed().is_empty());
        let look = store.install("alice.look.nord", &catalog(entry("looks/alice/nord", vec![file])), &client).unwrap();
        assert_eq!((look.hash.as_deref(), look.folder.clone()), (Some("h1"), paths.root.join("looks/alice.nord")));
        assert_eq!(
            Settings::load(&paths).default_look.as_deref(),
            Some("alice.look.nord"),
            "the first becomes the default"
        );
        // Without the folder source: the installed one is still there; removed, there is none
        let mut s = Settings::load(&paths);
        s.sources = Some(vec![]);
        s.save(&paths).unwrap();
        assert_eq!(store.list(&s).len(), 1);
        store.remove("alice.look.nord").unwrap();
        let s = Settings::load(&paths);
        assert!(store.list(&s).is_empty() && s.default_look.is_none() && store.default_look(&s).is_none());
        assert!(store.remove("alice.look.nord").is_err());
    }

    #[test]
    fn a_recipes_suggested_look() {
        let (_t, paths) = temp_paths();
        let src = tempfile::tempdir().unwrap();
        source_with(
            src.path(),
            &[("alice", "nord", &LOOK.replace("author = \"alice\"", "version = \"1.2.0\"\nauthor = \"alice\""))],
        );
        let mut s = Settings::load(&paths);
        s.sources =
            Some(vec![RecipeSource::new(RecipeSourceKind::Folder, &src.path().to_string_lossy(), None, None, None)]);
        let spec = |look: &str| {
            crate::composition::RecipeSpec::parse(&format!(
                "\"$schema\" = \"https://omaflake.dev/schema/recipe/v2.json\"\nplatform = \"nixos\"\nlook = {look}\n"
            ))
            .unwrap()
        };
        let store = LookStore::new(paths);
        assert_eq!(store.suggested(&s, &spec("\"alice/nord\"")).map(|l| l.id), Some("alice.look.nord".into()));
        assert!(
            store.suggested(&s, &spec("{ id = \"alice/nord\", min = \"2.0.0\" }")).is_none(),
            "not a version it allows"
        );
        assert!(store.suggested(&s, &spec("\"alice/other\"")).is_none());
    }

    #[test]
    fn references() {
        assert_eq!(LookStore::id_of("omaflake/catppuccin").as_deref(), Some("omaflake.look.catppuccin"));
        assert_eq!(LookStore::id_of("omaflake.look.catppuccin").as_deref(), Some("omaflake.look.catppuccin"));
        assert!(LookStore::id_of("catppuccin").is_none() && LookStore::id_of("../x").is_none());
    }
}
