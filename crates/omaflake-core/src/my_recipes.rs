// SPDX-License-Identifier: AGPL-3.0-or-later
//! My recipes (plan B141, B142): recipes put together in the apps – a platform, modules, a look, key bindings, what a VM
//! needs and how it runs – without writing a file by hand. They are written as recipe.toml (v2) into
//! `<data>/my-recipes/recipes/my/<platform>/<name>/`, a folder laid out like a catalog that is a local folder source
//! (added to the sources, first, the first time a recipe is saved there). So they are built like any recipe, and a
//! changed one makes its images outdated. Key binding sets made in the key binding studio (plan B147) go into the same
//! folder: `keymaps/my/<name>/keymap.toml` (v2), found as any key binding set of a local folder source.

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

/// A key binding set as the key binding studio has it: module "<author>/<name>" → action → what the set gives it
#[derive(Debug, Clone, PartialEq, Default)]
pub struct KeymapDraft {
    pub name: String,
    pub title: String,
    pub description: String,
    pub keys: crate::keymaps::Keys,
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

    pub fn keymap_folder(&self, name: &str) -> PathBuf {
        self.root().join("keymaps").join(Self::AUTHOR).join(name)
    }

    /// keymap.toml v2 of a draft; an entry is a key, keys, or `{ key, after }`
    pub fn keymap_toml(d: &KeymapDraft) -> Result<String> {
        let q = |s: &str| toml::Value::String(s.to_string()).to_string();
        let list = |keys: &[String]| format!("[{}]", keys.iter().map(|k| q(k)).collect::<Vec<_>>().join(", "));
        let mut t = format!(
            "# Key bindings {}/{} – made in omaflake's key binding studio; edit them there or here.\n\n",
            Self::AUTHOR,
            d.name
        );
        t.push_str(&format!("\"$schema\" = {}\n", q("https://omaflake.dev/schema/keymap/v2.json")));
        t.push_str(&format!("version = {}\nauthor = {}\nlicense = {}\n", q("1.0.0"), q(Self::AUTHOR), q("MIT")));
        let title = if d.title.trim().is_empty() { d.name.as_str() } else { d.title.trim() };
        t.push_str(&format!("title = {}\ndescription = {}\n", q(title), q(d.description.trim())));
        for (module, actions) in d.keys.iter().filter(|(_, a)| !a.is_empty()) {
            t.push_str(&format!("\n[keys.{}]\n", q(module)));
            for (action, e) in actions {
                let value = match (e.key.as_slice(), &e.after) {
                    ([one], None) => q(one),
                    (keys, None) => list(keys),
                    ([], Some(a)) => format!("{{ after = {} }}", q(a)),
                    ([one], Some(a)) => format!("{{ key = {}, after = {} }}", q(one), q(a)),
                    (keys, Some(a)) => format!("{{ key = {}, after = {} }}", list(keys), q(a)),
                };
                t.push_str(&format!("{action} = {value}\n"));
            }
        }
        Ok(t)
    }

    /// Writes a key binding set (replacing `previous` when its name changed) and makes the folder a source; returns
    /// its reference ("my/vim")
    pub fn save_keymap(&self, d: &KeymapDraft, previous: Option<String>) -> Result<String> {
        if !RecipeRef::valid_name(&d.name) {
            return Err(vm_err!("A name is lowercase letters, digits and -"));
        }
        let text = Self::keymap_toml(d)?;
        let folder = self.keymap_folder(&d.name);
        let renamed = previous.as_ref().is_some_and(|p| *p != d.name);
        if renamed && folder.exists() {
            return Err(vm_err!(codes::EXISTS, "There are key bindings \"{}\" already", d.name));
        }
        fs::create_dir_all(&folder).with_path(&folder)?;
        write_atomic(&folder.join("keymap.toml"), text.as_bytes())?;
        // What is written must be key bindings this omaflake reads
        let id = format!("{}.keymap.{}", Self::AUTHOR, d.name);
        crate::keymaps::KeymapStore::read(&id, &folder, String::new(), None)?;
        if let (true, Some(p)) = (renamed, previous) {
            let _ = Disposal::delete(&[self.keymap_folder(&p)]);
        }
        self.add_source()?;
        Ok(format!("{}/{}", Self::AUTHOR, d.name))
    }

    /// `<data>/my-recipes/looks/my/<name>`
    pub fn look_folder(&self, name: &str) -> PathBuf {
        self.root().join("looks").join(Self::AUTHOR).join(name)
    }

    /// look.toml v1 of a look made in the look studio (plan B150): `look` is its data as the studio put it together
    /// (title, description, tags, appearance, plain, colors, terminal, desktop, neovim, emacs, macos)
    pub fn look_toml(name: &str, look: &Value) -> Result<String> {
        let mut table = toml::Table::new();
        table.insert("$schema".into(), toml::Value::String("https://omaflake.dev/schema/look/v1.json".into()));
        table.insert("version".into(), toml::Value::String("1.0.0".into()));
        table.insert("author".into(), toml::Value::String(Self::AUTHOR.into()));
        let Value::Object(map) = look else { return Err(vm_err!("A look is a table of values")) };
        for (k, v) in map {
            if ["$schema", "version", "author"].contains(&k.as_str()) || v.is_null() {
                continue;
            }
            let value: toml::Value =
                serde_json::from_value(v.clone()).map_err(|e| vm_err!("{k} of the look cannot be written: {e}"))?;
            table.insert(k.clone(), value);
        }
        let body = toml::to_string(&table).map_err(|e| vm_err!("The look cannot be written: {e}"))?;
        Ok(format!("# Look {}/{name} – made in omaflake's look studio; edit it there or here.\n\n{body}", Self::AUTHOR))
    }

    /// Writes a look of yours; `previous` is its name before (a rename moves it). Returns "my/<name>".
    pub fn save_look(&self, name: &str, look: &Value, previous: Option<String>) -> Result<String> {
        if !RecipeRef::valid_name(name) {
            return Err(vm_err!("A name is lowercase letters, digits and -"));
        }
        let text = Self::look_toml(name, look)?;
        let folder = self.look_folder(name);
        let renamed = previous.as_ref().is_some_and(|p| p != name);
        if renamed && folder.exists() {
            return Err(vm_err!(codes::EXISTS, "There is a look \"{}\" already", name));
        }
        fs::create_dir_all(&folder).with_path(&folder)?;
        write_atomic(&folder.join("look.toml"), text.as_bytes())?;
        // What is written must be a look this omaflake reads
        let id = format!("{}.look.{}", Self::AUTHOR, name);
        crate::looks::LookStore::read(&id, &folder, String::new(), None)?;
        if let (true, Some(p)) = (renamed, previous) {
            let _ = Disposal::delete(&[self.look_folder(&p)]);
        }
        self.add_source()?;
        Ok(format!("{}/{}", Self::AUTHOR, name))
    }

    pub fn delete_look(&self, name: &str) -> Result<()> {
        let folder = self.look_folder(name);
        if !folder.join("look.toml").exists() {
            return Err(vm_err!(codes::NOT_FOUND, "No look \"{}\" of yours", name));
        }
        Disposal::delete(&[folder])
    }

    pub fn delete_keymap(&self, name: &str) -> Result<()> {
        let folder = self.keymap_folder(name);
        if !folder.join("keymap.toml").exists() {
            return Err(vm_err!(codes::NOT_FOUND, "No key bindings \"{}\" of yours", name));
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

    /// Looks of the look studio (plan B150): written as look.toml v1, read back as a look, renamed, deleted
    #[test]
    fn looks_save_rename_delete() {
        let (_t, p) = temp_paths();
        let my = MyRecipes::new(p.clone());
        let look = serde_json::json!({
            "title": "Dusk", "description": "Mine", "appearance": "dark", "tags": ["dark"],
            "colors": {"background": "#101010", "surface": "#181818", "selection": "#303030", "text": "#eeeeee",
                "subtext": "#bbbbbb", "muted": "#777777", "accent": "#33ccdd", "urgent": "#ee5566"},
            "terminal": {"normal": vec!["#000000"; 8], "bright": vec!["#ffffff"; 8]},
            "desktop": {"border": ["#33ccdd", "#5566ee"], "inactive_border": "#303030", "clock": "#33ccdd"},
            "neovim": {"theme": "catppuccin", "flavour": "mocha"},
            "version": "ignored", "author": "ignored"
        });
        assert_eq!(my.save_look("dusk", &look, None).unwrap(), "my/dusk");
        let read = crate::looks::LookStore::read("my.look.dusk", &my.look_folder("dusk"), String::new(), None).unwrap();
        assert_eq!(
            (read.author.as_str(), read.title.as_str(), read.appearance.as_deref()),
            ("my", "Dusk", Some("dark"))
        );
        assert_eq!(read.data["desktop"]["border"][1], "#5566ee");
        assert_eq!(read.data["neovim"]["flavour"], "mocha");
        assert_eq!(read.data["version"], "1.0.0", "the studio's own version, not the draft's");
        assert_eq!(Settings::load(&p).recipe_sources()[0], my.source());
        my.save_look("night", &look, Some("dusk".into())).unwrap();
        assert!(!my.look_folder("dusk").exists() && my.look_folder("night").exists());
        assert!(my.save_look("Bad Name", &look, None).is_err());
        my.delete_look("night").unwrap();
        assert!(my.delete_look("night").is_err());
    }

    /// Key binding sets of the studio (plan B147): a key, keys, `{ after }`, `{ key, after }`; read back as written
    #[test]
    fn key_bindings_save_rename_delete() {
        use crate::keymaps::{KeyEntry, KeymapStore};
        let (_t, p) = temp_paths();
        let my = MyRecipes::new(p.clone());
        let zellij = BTreeMap::from([
            ("unlock".to_string(), KeyEntry::key("Ctrl+a")),
            ("focus-left".to_string(), KeyEntry { key: vec!["h".into(), "Left".into()], after: None }),
            ("new-pane".to_string(), KeyEntry { key: vec![], after: Some("pane".into()) }),
            ("close-pane".to_string(), KeyEntry { key: vec!["q".into()], after: Some("pane".into()) }),
        ]);
        let mut d = KeymapDraft {
            name: "tmux-like".into(),
            title: "Like tmux".into(),
            description: "Ctrl+a \"first\"".into(),
            keys: BTreeMap::from([("omaflake/zellij".to_string(), zellij.clone())]),
        };
        assert_eq!(my.save_keymap(&d, None).unwrap(), "my/tmux-like");
        let k = KeymapStore::read("my.keymap.tmux-like", &my.keymap_folder("tmux-like"), String::new(), None).unwrap();
        assert_eq!((k.title.as_str(), k.description.as_str()), ("Like tmux", "Ctrl+a \"first\""));
        assert_eq!(k.keys["omaflake/zellij"], zellij);
        assert_eq!(Settings::load(&p).recipe_sources()[0], my.source());
        d.name = "mine".into();
        my.save_keymap(&d, Some("tmux-like".into())).unwrap();
        assert!(!my.keymap_folder("tmux-like").exists() && my.keymap_folder("mine").exists());
        assert!(my.save_keymap(&KeymapDraft { name: "Bad".into(), ..d.clone() }, None).is_err());
        my.delete_keymap("mine").unwrap();
        assert!(my.delete_keymap("mine").is_err());
    }
}
