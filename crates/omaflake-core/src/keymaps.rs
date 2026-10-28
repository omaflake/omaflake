// SPDX-License-Identifier: AGPL-3.0-or-later
//! Key binding sets (plan B141, B142): data (`keymap.toml`, schema keymap/v2) that gives keys to the actions modules
//! offer – `[keys."<author>/<name>"]` tables of action → key, keys, or `{ key, after }` (plan B147). A module works without any set (its module.toml has a
//! default key for each action); a recipe names sets in order and a later one wins for the same action. They come from
//! the sources' catalogs (`keymaps/<author>/<name>/`) and are installed into `<data>/keymaps/` (`parts::PartStore`),
//! or used as they are on disk from a local folder source.

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

/// What a set gives an action (keymap/v2, plan B147): a key ("Ctrl+g"), several keys, or a table with `key` and/or
/// `after` (the mode the program is in afterwards)
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(from = "EntryForm", into = "EntryForm")]
pub struct KeyEntry {
    /// Empty: the module's key stays
    pub key: Vec<String>,
    pub after: Option<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(untagged)]
enum EntryForm {
    One(String),
    Many(Vec<String>),
    Table {
        #[serde(default, with = "crate::modules::one_or_many", skip_serializing_if = "Vec::is_empty")]
        key: Vec<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        after: Option<String>,
    },
}

impl From<EntryForm> for KeyEntry {
    fn from(f: EntryForm) -> Self {
        match f {
            EntryForm::One(k) => KeyEntry { key: vec![k], after: None },
            EntryForm::Many(key) => KeyEntry { key, after: None },
            EntryForm::Table { key, after } => KeyEntry { key, after },
        }
    }
}

impl From<KeyEntry> for EntryForm {
    fn from(e: KeyEntry) -> Self {
        match (e.key.len(), e.after) {
            (1, None) => EntryForm::One(e.key.into_iter().next().unwrap_or_default()),
            (_, None) if !e.key.is_empty() => EntryForm::Many(e.key),
            (_, after) => EntryForm::Table { key: e.key, after },
        }
    }
}

impl KeyEntry {
    pub fn key(k: &str) -> Self {
        KeyEntry { key: vec![k.to_string()], after: None }
    }

    /// For people: "Ctrl+g", "h / Left", "Ctrl+g, then Pane"
    pub fn describe(&self) -> String {
        let keys = self.key.join(" / ");
        match &self.after {
            Some(a) if keys.is_empty() => format!("then {a}"),
            Some(a) => format!("{keys}, then {a}"),
            None => keys,
        }
    }
}

/// module "<author>/<name>" → action → what the set gives it
pub type Keys = BTreeMap<String, BTreeMap<String, KeyEntry>>;

/// One key binding set in catalog.json (catalog v4)
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CatalogKeymap {
    /// "<author>.keymap.<name>"
    pub id: String,
    pub author: String,
    pub name: String,
    pub path: String,
    pub version: String,
    pub title: String,
    pub description: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub descriptions: Option<BTreeMap<String, String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub license: Option<String>,
    #[serde(default)]
    pub keys: Keys,
    pub hash: String,
    pub files: Vec<CatalogFile>,
}

impl CatalogKeymap {
    pub fn part(&self) -> CatalogPart<'_> {
        CatalogPart { id: &self.id, path: &self.path, version: &self.version, hash: &self.hash, files: &self.files }
    }

    pub fn description_in(&self, language: Option<&str>) -> &str {
        language.and_then(|l| self.descriptions.as_ref()?.get(l)).map(String::as_str).unwrap_or(&self.description)
    }
}

/// A key binding set omaflake can use
#[derive(Debug, Clone, PartialEq)]
pub struct Keymap {
    /// "<author>.keymap.<name>"
    pub id: String,
    pub author: String,
    pub name: String,
    pub version: String,
    pub title: String,
    pub description: String,
    pub descriptions: BTreeMap<String, String>,
    pub keys: Keys,
    pub source: String,
    pub hash: Option<String>,
    pub folder: PathBuf,
}

impl Keymap {
    /// "<author>/<name>"
    pub fn reference(&self) -> String {
        format!("{}/{}", self.author, self.name)
    }

    pub fn description_in(&self, language: Option<&str>) -> &str {
        language.and_then(|l| self.descriptions.get(l)).map(String::as_str).unwrap_or(&self.description)
    }
}

#[derive(Debug, Deserialize)]
struct KeymapToml {
    #[serde(rename = "$schema", default)]
    schema: String,
    #[serde(default)]
    version: String,
    #[serde(default)]
    title: String,
    #[serde(default)]
    description: String,
    #[serde(default)]
    keys: Keys,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

#[derive(Debug, Clone)]
pub struct KeymapStore {
    pub paths: Paths,
}

impl KeymapStore {
    /// keymap.toml's version this omaflake reads
    pub const KEYMAP_VERSION: usize = 2;

    pub fn new(paths: Paths) -> Self {
        KeymapStore { paths }
    }

    fn parts(&self) -> PartStore {
        PartStore::new(self.paths.clone(), PartKind::Keymap)
    }

    pub fn read(id: &str, folder: &Path, source: String, hash: Option<String>) -> Result<Keymap> {
        let file = folder.join("keymap.toml");
        let text = fs::read_to_string(&file).with_path(&file)?;
        let k: KeymapToml =
            toml::from_str(&text).map_err(|e| vm_err!("{}: not key bindings ({})", file.display(), e.message()))?;
        match FileFormat::parse(&k.schema) {
            Some((t, v)) if t == "keymap" => {
                FileFormat::check(v, Self::KEYMAP_VERSION, Self::KEYMAP_VERSION, "keymap.toml")?;
            }
            _ => return Err(vm_err!("{}: not key bindings (\"$schema\" = \"{}\")", file.display(), k.schema)),
        }
        let Some((author, _, name)) = PartKind::Keymap.parse_id(id) else {
            return Err(vm_err!("Not key bindings: {id}"));
        };
        Ok(Keymap {
            id: id.to_string(),
            author,
            title: if k.title.is_empty() { name.clone() } else { k.title },
            name,
            version: k.version,
            description: k.description,
            descriptions: k
                .rest
                .iter()
                .filter_map(|(key, v)| Some((key.strip_prefix("description_")?.to_string(), v.as_str()?.to_string())))
                .collect(),
            keys: k.keys,
            source,
            hash,
            folder: folder.to_path_buf(),
        })
    }

    pub fn list_with_errors(&self, settings: &Settings) -> (Vec<Keymap>, Vec<(String, String)>) {
        let mut out = Vec::new();
        let mut errors = Vec::new();
        for e in self.parts().entries(settings) {
            match Self::read(&e.id, &e.folder, e.source, e.installed.map(|i| i.hash)) {
                Ok(k) => out.push(k),
                Err(err) => errors.push((e.id, err.to_string())),
            }
        }
        (out, errors)
    }

    pub fn list(&self, settings: &Settings) -> Vec<Keymap> {
        self.list_with_errors(settings).0
    }

    pub fn find(&self, settings: &Settings, reference: &str) -> Option<Keymap> {
        let id = PartKind::Keymap.id_of(reference, None)?;
        self.list(settings).into_iter().find(|k| k.id == id)
    }

    pub fn installed(&self) -> BTreeMap<String, crate::parts::InstalledPart> {
        self.parts().installed()
    }

    pub fn install(&self, id: &str, catalog: &RecipeCatalog, client: &SourceClient) -> Result<Keymap> {
        let Some(entry) = catalog.keymaps.iter().find(|k| k.id == id) else {
            return Err(vm_err!(codes::NOT_FOUND, "{} is not in {}", id, client.source.display_name()));
        };
        let source = client.source.id();
        let folder = self
            .parts()
            .install(&entry.part(), client, |work| Self::read(id, work, source.clone(), None).map(|_| ()))?;
        Self::read(id, &folder, source, Some(entry.hash.clone()))
    }

    pub fn remove(&self, id: &str) -> Result<()> {
        self.parts().remove(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;
    use crate::recipe_sources::{RecipeSource, RecipeSourceKind};

    #[test]
    fn a_local_set() {
        let (_t, paths) = temp_paths();
        let src = tempfile::tempdir().unwrap();
        let f = src.path().join("keymaps/alice/vim");
        fs::create_dir_all(&f).unwrap();
        fs::write(
            f.join("keymap.toml"),
            "\"$schema\" = \"https://omaflake.dev/schema/keymap/v2.json\"\nversion = \"1.0.0\"\ntitle = \"Vim\"\n\
             description = \"x\"\n[keys.\"omaflake/hyprland\"]\nterminal = \"Super+t\"\n",
        )
        .unwrap();
        let mut s = Settings::load(&paths);
        s.sources =
            Some(vec![RecipeSource::new(RecipeSourceKind::Folder, &src.path().to_string_lossy(), None, None, None)]);
        s.save(&paths).unwrap();
        let k = KeymapStore::new(paths).find(&s, "alice/vim").unwrap();
        assert_eq!((k.title.as_str(), k.keys["omaflake/hyprland"]["terminal"].describe().as_str()), ("Vim", "Super+t"));
    }
}
