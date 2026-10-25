// SPDX-License-Identifier: AGPL-3.0-or-later
//! My recipes (plan B141, B142): recipes put together in the apps – a platform, modules, a look, key bindings, what a VM
//! needs and how it runs – without writing a file by hand. They are written as recipe.toml (v2) into
//! `<data>/my-recipes/recipes/my/<platform>/<name>/`, a folder laid out like a catalog that is a local folder source
//! (added to the sources, first, the first time a recipe is saved there). So they are built like any recipe, and a
//! changed one makes its images outdated.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::composition::{LocaleRules, PartRef, RecipeSpec, Requirements};
use crate::error::{codes, IoContext, Result};
use crate::file_format::write_atomic;
use crate::models::GuestKind;
use crate::paths::{Paths, Settings};
use crate::recipe_sources::{RecipeRef, RecipeSource, RecipeSourceKind};
use crate::trash::Disposal;
use crate::vm_err;

/// A recipe as the editor has it
#[derive(Debug, Clone, PartialEq, Default)]
pub struct RecipeDraft {
    pub name: String,
    pub platform: Option<GuestKind>,
    pub description: String,
    pub modules: Vec<PartRef>,
    pub look: Option<PartRef>,
    pub keymaps: Vec<PartRef>,
    pub requirements: Requirements,
    /// display, network, share, ephemeral, audio, microphone, clipboard, camera
    pub vm: BTreeMap<String, Value>,
    pub locales: LocaleRules,
}

pub struct MyRecipes {
    paths: Paths,
}

impl MyRecipes {
    /// The author of every recipe made here
    pub const AUTHOR: &'static str = "my";

    pub fn new(paths: Paths) -> MyRecipes {
        MyRecipes { paths }
    }

    /// `<data>/my-recipes`
    pub fn root(&self) -> PathBuf {
        self.paths.root.join("my-recipes")
    }

    /// The local folder source they are read from
    pub fn source(&self) -> RecipeSource {
        RecipeSource::new(RecipeSourceKind::Folder, &self.root().to_string_lossy(), None, None, None)
    }

    pub fn folder(&self, platform: GuestKind, name: &str) -> PathBuf {
        self.root().join("recipes").join(Self::AUTHOR).join(platform.as_str()).join(name)
    }

    /// The recipe base ("my.nixos.work") of a folder of these, None for any other folder
    pub fn base_of(&self, folder: &Path) -> Option<String> {
        let rel = folder.strip_prefix(self.root().join("recipes")).ok()?;
        let parts: Vec<String> = rel.components().map(|c| c.as_os_str().to_string_lossy().into_owned()).collect();
        (parts.len() == 3 && parts[0] == Self::AUTHOR).then(|| parts.join("."))
    }

    /// recipe.toml v2 of a draft
    pub fn to_toml(d: &RecipeDraft) -> Result<String> {
        let platform = d.platform.ok_or_else(|| vm_err!("A recipe needs a platform"))?;
        let q = |s: &str| toml::Value::String(s.to_string()).to_string();
        let systems: &[&str] = match platform {
            GuestKind::Nixos => &["aarch64-linux", "x86_64-linux"],
            GuestKind::Macos => &["aarch64-darwin"],
        };
        let list = |refs: &[PartRef]| refs.iter().map(|r| r.to_toml().to_string()).collect::<Vec<_>>();
        let mut t = String::new();
        t.push_str(&format!(
            "# Recipe {}/{}/{} – made in omaflake (My Recipes); edit it there or here.\n\n",
            Self::AUTHOR,
            platform.as_str(),
            d.name
        ));
        t.push_str(&format!("\"$schema\" = {}\n", q("https://omaflake.dev/schema/recipe/v2.json")));
        t.push_str(&format!("author = {}\nlicense = {}\n", q(Self::AUTHOR), q("MIT")));
        t.push_str(&format!("description = {}\ntags = []\n", q(d.description.trim())));
        t.push_str(&format!(
            "platform = {}\nsystems = [{}]\n\n",
            q(platform.as_str()),
            systems.iter().map(|s| q(s)).collect::<Vec<_>>().join(", ")
        ));
        t.push_str("modules = [\n");
        for m in list(&d.modules) {
            t.push_str(&format!("  {m},\n"));
        }
        t.push_str("]\n");
        if let Some(l) = &d.look {
            t.push_str(&format!("look = {}\n", l.to_toml()));
        }
        t.push_str(&format!("keymaps = [{}]\n\n", list(&d.keymaps).join(", ")));
        let r = d.requirements;
        t.push_str(&format!(
            "[requirements]\ncpus = {}\nmemory_gb = {}\ndisk_gb = {}\n\n",
            r.cpus, r.memory_gb, r.disk_gb
        ));
        t.push_str("[vm]\n");
        for (k, v) in &d.vm {
            let value = match v {
                Value::Bool(b) => b.to_string(),
                Value::String(s) => q(s),
                _ => continue,
            };
            t.push_str(&format!("{k} = {value}\n"));
        }
        if !d.locales.is_empty() {
            t.push_str("\n[locales]\n");
            if let Some(rtl) = d.locales.rtl {
                t.push_str(&format!("rtl = {rtl}\n"));
            }
            if !d.locales.languages.is_empty() {
                t.push_str(&format!(
                    "languages = [{}]\n",
                    d.locales.languages.iter().map(|l| q(l)).collect::<Vec<_>>().join(", ")
                ));
            }
        }
        // What is written must be a recipe this omaflake reads
        RecipeSpec::parse(&t).map_err(|e| vm_err!("The recipe cannot be written: {}", e))?;
        Ok(t)
    }

    /// Writes a draft (replacing `previous` – the recipe's name or platform changed – or a recipe of the same name)
    /// and makes the folder a source; returns its base ID ("my.nixos.work")
    pub fn save(&self, d: &RecipeDraft, previous: Option<(GuestKind, String)>) -> Result<String> {
        if !RecipeRef::valid_name(&d.name) {
            return Err(vm_err!("A recipe's name is lowercase letters, digits and -"));
        }
        if d.modules.is_empty() {
            return Err(vm_err!("A recipe needs at least one module"));
        }
        let platform = d.platform.ok_or_else(|| vm_err!("A recipe needs a platform"))?;
        let text = Self::to_toml(d)?;
        let folder = self.folder(platform, &d.name);
        let renamed = previous.as_ref().is_some_and(|(p, n)| (*p, n.as_str()) != (platform, d.name.as_str()));
        if renamed && folder.exists() {
            return Err(vm_err!(codes::EXISTS, "There is a recipe \"{}\" already", d.name));
        }
        fs::create_dir_all(&folder).with_path(&folder)?;
        write_atomic(&folder.join("recipe.toml"), text.as_bytes())?;
        if let (true, Some((p, n))) = (renamed, previous) {
            let _ = Disposal::delete(&[self.folder(p, &n)]);
        }
        self.add_source()?;
        Ok(format!("{}.{}.{}", Self::AUTHOR, platform.as_str(), d.name))
    }

    /// Removes one of these recipes (images built from it stay)
    pub fn delete(&self, platform: GuestKind, name: &str) -> Result<()> {
        let folder = self.folder(platform, name);
        if !folder.join("recipe.toml").exists() {
            return Err(vm_err!(codes::NOT_FOUND, "No recipe \"{}\" of yours", name));
        }
        Disposal::delete(&[folder])
    }

    /// The folder as a source, first in the order (its recipes are the user's own)
    fn add_source(&self) -> Result<()> {
        let mut s = Settings::load(&self.paths);
        let mine = self.source();
        let mut list = s.recipe_sources();
        if list.iter().any(|x| x.id() == mine.id()) {
            return Ok(());
        }
        list.insert(0, mine);
        s.sources = Some(list);
        s.save(&self.paths)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;

    #[test]
    fn save_read_rename_delete() {
        let (_t, p) = temp_paths();
        let my = MyRecipes::new(p.clone());
        let mut d = RecipeDraft {
            name: "work".into(),
            platform: Some(GuestKind::Nixos),
            description: "Mine \"quoted\"".into(),
            modules: vec![
                PartRef::new("omaflake/hyprland"),
                PartRef { min: Some("1.0.0".into()), ..PartRef::new("omaflake/neovim") },
            ],
            look: Some(PartRef::new("omaflake/omaflake")),
            keymaps: vec![PartRef::new("omaflake/default")],
            requirements: Requirements { cpus: 2, memory_gb: 4, disk_gb: 30 },
            vm: BTreeMap::from([("display".into(), Value::String("gpu".into())), ("share".into(), Value::Bool(true))]),
            locales: LocaleRules { rtl: Some(false), languages: vec![] },
        };
        assert_eq!(my.save(&d, None).unwrap(), "my.nixos.work");
        let folder = my.folder(GuestKind::Nixos, "work");
        let spec = RecipeSpec::read(&folder).unwrap();
        assert_eq!((spec.modules.len(), spec.modules[1].min.as_deref(), spec.requirements.cpus), (2, Some("1.0.0"), 2));
        assert_eq!(spec.data["description"], "Mine \"quoted\"");
        assert_eq!(spec.locales.rtl, Some(false));
        assert_eq!(my.base_of(&folder).as_deref(), Some("my.nixos.work"));
        // the folder is a source, first
        assert_eq!(Settings::load(&p).recipe_sources()[0], my.source());
        // renamed: the old folder goes
        d.name = "play".into();
        my.save(&d, Some((GuestKind::Nixos, "work".into()))).unwrap();
        assert!(!folder.exists() && my.folder(GuestKind::Nixos, "play").exists());
        assert!(my.save(&RecipeDraft { name: "Bad Name".into(), ..d.clone() }, None).is_err());
        assert!(my.save(&RecipeDraft { modules: vec![], ..d.clone() }, None).is_err());
        my.delete(GuestKind::Nixos, "play").unwrap();
        assert!(my.delete(GuestKind::Nixos, "play").is_err());
    }
}
