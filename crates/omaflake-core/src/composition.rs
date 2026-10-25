// SPDX-License-Identifier: AGPL-3.0-or-later
//! What a recipe is made of (plan B141, B142). A recipe is data (`recipe.toml`, schema recipe/v2): a platform, the
//! modules in it, a suggested look, key binding sets, what a VM from it needs and how it runs, and which locales it
//! works with. Here a recipe is read and put together with the modules and key bindings that are here: which are
//! missing (or here in a version the recipe does not allow), what a module needs that no other provides, exclusive
//! capabilities twice, keys that overlap or break the key rule. Missing parts block a build; everything else is a
//! warning – a recipe stays buildable (the abstraction must not sprawl, B141). tools/catalog.py checks the same.

use std::collections::BTreeMap;
use std::fs;
use std::path::Path;

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::error::{IoContext, Result};
use crate::file_format::FileFormat;
use crate::keymaps::Keymap;
use crate::models::GuestKind;
use crate::modules::{Capability, Module};
use crate::vm_err;

/// A reference to a module, look or key binding set: "<author>/<name>" and which versions it allows – none given is
/// the newest, else an exact version, or a minimum and/or a maximum (both included)
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct PartRef {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<String>,
}

impl PartRef {
    pub fn new(id: &str) -> PartRef {
        PartRef { id: id.to_string(), ..Default::default() }
    }

    /// From recipe.toml: "author/name" or { id, version | min | max }
    pub fn parse(value: &Value) -> Option<PartRef> {
        let r = match value {
            Value::String(s) => PartRef::new(s),
            Value::Object(_) => serde_json::from_value(value.clone()).ok()?,
            _ => return None,
        };
        let (author, name) = r.id.split_once('/')?;
        let ok = crate::recipe_sources::RecipeRef::valid_name(author)
            && crate::recipe_sources::RecipeRef::valid_name(name)
            && [&r.version, &r.min, &r.max].into_iter().flatten().all(|v| version(v).is_some());
        ok.then_some(r)
    }

    /// Whether `v` (the author's "1.2.0") is a version this reference allows
    pub fn allows(&self, v: &str) -> bool {
        let Some(v) = version(v) else { return self.version.is_none() && self.min.is_none() && self.max.is_none() };
        if let Some(exact) = &self.version {
            return version(exact) == Some(v);
        }
        self.min.as_deref().and_then(version).is_none_or(|m| v >= m)
            && self.max.as_deref().and_then(version).is_none_or(|m| v <= m)
    }

    /// For people: "omaflake/latex", "omaflake/latex 1.2.0", "omaflake/latex ≥ 1.0.0 ≤ 1.9.0"
    pub fn label(&self) -> String {
        let mut s = self.id.clone();
        if let Some(v) = &self.version {
            s.push_str(&format!(" {v}"));
        }
        if let Some(v) = &self.min {
            s.push_str(&format!(" ≥ {v}"));
        }
        if let Some(v) = &self.max {
            s.push_str(&format!(" ≤ {v}"));
        }
        s
    }

    /// As recipe.toml writes it: a plain string without versions
    pub fn to_toml(&self) -> toml::Value {
        if self.version.is_none() && self.min.is_none() && self.max.is_none() {
            return toml::Value::String(self.id.clone());
        }
        toml::Value::try_from(self).unwrap_or_else(|_| toml::Value::String(self.id.clone()))
    }
}

/// "1.2.0" → (1, 2, 0)
pub fn version(v: &str) -> Option<(u64, u64, u64)> {
    let p: Vec<&str> = v.split('.').collect();
    if p.len() != 3 || p.iter().any(|x| x.is_empty() || (x.len() > 1 && x.starts_with('0'))) {
        return None;
    }
    Some((p[0].parse().ok()?, p[1].parse().ok()?, p[2].parse().ok()?))
}

/// What a VM from the recipe needs at least (`[requirements]`)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct Requirements {
    pub cpus: u32,
    pub memory_gb: u32,
    pub disk_gb: u32,
}

/// The locales a recipe works with (`[locales]`): no right-to-left languages, or only these languages
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct LocaleRules {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rtl: Option<bool>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub languages: Vec<String>,
}

impl LocaleRules {
    /// Languages written right to left (ISO 639-1/-3)
    const RTL: [&'static str; 9] = ["ar", "he", "fa", "ur", "yi", "ps", "dv", "sd", "ug"];

    /// Why a locale ("de_DE.UTF-8") is outside these rules – None when it is fine
    pub fn outside(&self, locale: &str) -> Option<Outside> {
        let language = locale.split(['_', '.']).next().unwrap_or(locale);
        if !self.languages.is_empty() && !self.languages.iter().any(|l| l == language) {
            return Some(Outside::Languages(self.languages.clone()));
        }
        if self.rtl == Some(false) && Self::RTL.contains(&language) {
            return Some(Outside::RightToLeft);
        }
        None
    }
}

/// Why a locale is not one a recipe is made for
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outside {
    /// Made for these languages only
    Languages(Vec<String>),
    RightToLeft,
}

impl LocaleRules {
    pub fn is_empty(&self) -> bool {
        self.rtl.is_none() && self.languages.is_empty()
    }
}

/// recipe.toml v2, as read
#[derive(Debug, Clone, PartialEq)]
pub struct RecipeSpec {
    pub platform: GuestKind,
    pub modules: Vec<PartRef>,
    pub look: Option<PartRef>,
    pub keymaps: Vec<PartRef>,
    pub requirements: Requirements,
    pub locales: LocaleRules,
    /// The whole file as data (description, tags, vm, …)
    pub data: Value,
}

impl RecipeSpec {
    /// recipe.toml's version this omaflake reads
    pub const VERSION: usize = 2;

    pub fn read(folder: &Path) -> Result<RecipeSpec> {
        let file = folder.join("recipe.toml");
        let text = fs::read_to_string(&file).with_path(&file)?;
        Self::parse(&text).map_err(|e| vm_err!("{}: {}", file.display(), e))
    }

    pub fn parse(text: &str) -> std::result::Result<RecipeSpec, String> {
        let table: toml::Table = toml::from_str(text).map_err(|e| format!("not a recipe ({})", e.message()))?;
        let data = serde_json::to_value(&table).map_err(|e| e.to_string())?;
        let schema = data.get("$schema").and_then(Value::as_str).unwrap_or_default();
        match FileFormat::parse(schema) {
            Some((t, v)) if t == "recipe" => {
                FileFormat::check(v, Self::VERSION, Self::VERSION, "recipe.toml")
                    .map_err(|e| e.message().to_string())?;
            }
            _ => return Err(format!("not a recipe (\"$schema\" = \"{schema}\")")),
        }
        let platform = data
            .get("platform")
            .and_then(Value::as_str)
            .and_then(GuestKind::parse)
            .ok_or("platform is nixos or macos")?;
        let refs = |key: &str| -> std::result::Result<Vec<PartRef>, String> {
            data.get(key)
                .and_then(Value::as_array)
                .map(|a| a.iter().map(|v| PartRef::parse(v).ok_or(format!("{key}: {v} is no reference"))).collect())
                .unwrap_or(Ok(Vec::new()))
        };
        let modules = refs("modules")?;
        let keymaps = refs("keymaps")?;
        let look = match data.get("look") {
            Some(v) => Some(PartRef::parse(v).ok_or("look: no reference")?),
            None => None,
        };
        let requirements = data
            .get("requirements")
            .map(|r| serde_json::from_value(r.clone()).map_err(|e| format!("requirements: {e}")))
            .transpose()?
            .unwrap_or_default();
        let locales = data
            .get("locales")
            .map(|r| serde_json::from_value(r.clone()).map_err(|e| format!("locales: {e}")))
            .transpose()?
            .unwrap_or_default();
        Ok(RecipeSpec { platform, modules, look, keymaps, requirements, locales, data })
    }
}

// MARK: Keys

/// One chord: modifiers and a key
type Chord = (Vec<&'static str>, String);

const MODIFIERS: [&str; 4] = ["Super", "Ctrl", "Alt", "Shift"];

/// A key in the catalog's notation → its chords ("Space f f" is three); None when it is not one
pub fn parse_key(key: &str) -> Option<Vec<Chord>> {
    let named = [
        "Return",
        "Space",
        "Escape",
        "Tab",
        "Backspace",
        "Left",
        "Right",
        "Up",
        "Down",
        "Digit",
        "Arrow",
        "F1",
        "F2",
        "F3",
        "F4",
        "F5",
        "F6",
        "F7",
        "F8",
        "F9",
        "F10",
        "F11",
        "F12",
    ];
    let chords: Vec<&str> = key.split_whitespace().collect();
    if chords.is_empty() {
        return None;
    }
    let mut out = Vec::new();
    for chord in chords {
        let parts: Vec<&str> = chord.split('+').collect();
        let (name, mods) = parts.split_last()?;
        let mut m: Vec<&'static str> = Vec::new();
        for x in mods {
            let known = MODIFIERS.iter().find(|k| *k == x)?;
            if m.contains(known) {
                return None;
            }
            m.push(known);
        }
        m.sort_by_key(|x| MODIFIERS.iter().position(|k| k == x));
        let single = name.chars().count() == 1 && !name.chars().any(|c| c.is_ascii_uppercase() || c.is_whitespace());
        if !named.contains(name) && !single {
            return None;
        }
        out.push((m, name.to_string()));
    }
    Some(out)
}

/// Every concrete sequence a key stands for: Digit is 1–9, Arrow the four arrows
fn expand(seq: &[Chord]) -> Vec<Vec<Chord>> {
    let mut result: Vec<Vec<Chord>> = vec![Vec::new()];
    for (mods, name) in seq {
        let names: Vec<String> = match name.as_str() {
            "Digit" => (1..=9).map(|i| i.to_string()).collect(),
            "Arrow" => ["Left", "Right", "Up", "Down"].iter().map(|s| s.to_string()).collect(),
            _ => vec![name.clone()],
        };
        result = result
            .into_iter()
            .flat_map(|r| {
                names.iter().map(move |n| {
                    let mut r = r.clone();
                    r.push((mods.clone(), n.clone()));
                    r
                })
            })
            .collect();
    }
    result
}

/// A key bound to a module's action in a recipe
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Binding {
    /// "<author>/<name>" of the module
    pub module: String,
    pub action: String,
    pub title: String,
    pub key: String,
    /// Where it works: "desktop" or a module ("omaflake/neovim")
    pub context: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub mode: Option<String>,
    /// The key binding set that gave the key; None: the module's default
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub from: Option<String>,
}

/// What is wrong with a recipe – blockers (`blocks`) and warnings
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Issue {
    /// A module of the recipe is not here (`have`: here, but in a version the recipe does not allow)
    #[serde(rename_all = "camelCase")]
    MissingModule { reference: String, have: Option<String> },
    #[serde(rename_all = "camelCase")]
    MissingKeymap { reference: String, have: Option<String> },
    /// A module needs a capability no module of the recipe provides
    #[serde(rename_all = "camelCase")]
    Needs { module: String, capability: String },
    /// Modules providing the same exclusive capability
    #[serde(rename_all = "camelCase")]
    Incompatible { capability: String, modules: Vec<String> },
    #[serde(rename_all = "camelCase")]
    KeysOverlap { first: Binding, second: Binding },
    /// Without a mode a key starts with Super or Ctrl (an editor's own keys excepted)
    #[serde(rename_all = "camelCase")]
    KeyRule { binding: Binding },
}

impl Issue {
    /// It stops a build: the image flake would not find the part
    pub fn blocks(&self) -> bool {
        matches!(self, Issue::MissingModule { .. } | Issue::MissingKeymap { .. })
    }
}

/// A recipe with the parts that are here
#[derive(Debug, Clone, PartialEq)]
pub struct Composition {
    pub modules: Vec<Module>,
    pub keymaps: Vec<Keymap>,
    pub bindings: Vec<Binding>,
    pub issues: Vec<Issue>,
    /// Modules that are not free software
    pub unfree: Vec<String>,
}

impl Composition {
    /// Puts `spec` together with the modules and key bindings here (each once – the first of an id wins)
    pub fn of(
        spec: &RecipeSpec,
        modules: &[Module],
        keymaps: &[Keymap],
        capabilities: &BTreeMap<String, Capability>,
    ) -> Composition {
        let mut issues = Vec::new();
        let mut present: Vec<Module> = Vec::new();
        for r in &spec.modules {
            let found = modules.iter().find(|m| m.platform == spec.platform && m.reference() == r.id);
            match found {
                Some(m) if r.allows(&m.version) => present.push(m.clone()),
                Some(m) => issues.push(Issue::MissingModule { reference: r.label(), have: Some(m.version.clone()) }),
                None => issues.push(Issue::MissingModule { reference: r.label(), have: None }),
            }
        }
        let mut sets: Vec<Keymap> = Vec::new();
        for r in &spec.keymaps {
            match keymaps.iter().find(|k| k.reference() == r.id) {
                Some(k) if r.allows(&k.version) => sets.push(k.clone()),
                Some(k) => issues.push(Issue::MissingKeymap { reference: r.label(), have: Some(k.version.clone()) }),
                None => issues.push(Issue::MissingKeymap { reference: r.label(), have: None }),
            }
        }
        // Capabilities
        let provided: Vec<&String> = present.iter().flat_map(|m| &m.provides).collect();
        for m in &present {
            for c in &m.needs {
                if !provided.contains(&c) {
                    issues.push(Issue::Needs { module: m.reference(), capability: c.clone() });
                }
            }
        }
        for (c, info) in capabilities {
            let who: Vec<String> = present.iter().filter(|m| m.provides.contains(c)).map(Module::reference).collect();
            if info.exclusive && who.len() > 1 {
                issues.push(Issue::Incompatible { capability: c.clone(), modules: who });
            }
        }
        // Keys
        let bindings = Self::bindings(&present, &sets);
        let by_ref: BTreeMap<String, &Module> = present.iter().map(|m| (m.reference(), m)).collect();
        for (a, b) in overlaps(&bindings, &by_ref) {
            issues.push(Issue::KeysOverlap { first: a, second: b });
        }
        for b in &bindings {
            if breaks_key_rule(b, &by_ref) {
                issues.push(Issue::KeyRule { binding: b.clone() });
            }
        }
        let unfree = present.iter().filter(|m| m.unfree).map(Module::reference).collect();
        Composition { modules: present, keymaps: sets, bindings, issues, unfree }
    }

    /// Each module's actions with their keys: the default, then the sets in order (a later one wins)
    fn bindings(modules: &[Module], sets: &[Keymap]) -> Vec<Binding> {
        let mut out = Vec::new();
        for m in modules {
            let reference = m.reference();
            for (action, a) in &m.actions {
                let mut key = a.key.clone();
                let mut from = None;
                for s in sets {
                    if let Some(k) = s.keys.get(&reference).and_then(|keys| keys.get(action)) {
                        key = k.clone();
                        from = Some(s.reference());
                    }
                }
                let context = if a.desktop || m.desktop {
                    "desktop".to_string()
                } else {
                    a.context.clone().unwrap_or_else(|| reference.clone())
                };
                out.push(Binding {
                    module: reference.clone(),
                    action: action.clone(),
                    title: a.title.clone(),
                    key,
                    context,
                    mode: a.mode.clone(),
                    from,
                });
            }
        }
        out
    }

    pub fn blocked(&self) -> bool {
        self.issues.iter().any(Issue::blocks)
    }
}

/// Bindings that can be active at once and collide (the same key, or one a prefix of the other). Active at once: the
/// same context and mode; a desktop-wide one with any binding without a mode; a program's binding without a mode with
/// one of a program that runs inside it (`runs_in` names a capability the outer one provides).
fn overlaps(bindings: &[Binding], modules: &BTreeMap<String, &Module>) -> Vec<(Binding, Binding)> {
    let provides = |r: &str| modules.get(r).map(|m| m.provides.clone()).unwrap_or_default();
    let runs_in = |r: &str| modules.get(r).map(|m| m.runs_in.clone()).unwrap_or_default();
    let nested = |outer: &str, inner: &str| provides(outer).iter().any(|c| runs_in(inner).contains(c));
    let together = |a: &Binding, b: &Binding| {
        if a.mode.is_some() || b.mode.is_some() {
            return a.context == b.context && a.mode == b.mode;
        }
        a.context == b.context
            || a.context == "desktop"
            || b.context == "desktop"
            || nested(&a.context, &b.context)
            || nested(&b.context, &a.context)
    };
    let mut out = Vec::new();
    for (i, a) in bindings.iter().enumerate() {
        let Some(ka) = parse_key(&a.key) else { continue };
        let ka = expand(&ka);
        for b in &bindings[i + 1..] {
            let Some(kb) = parse_key(&b.key) else { continue };
            if !together(a, b) {
                continue;
            }
            let kb = expand(&kb);
            let hit = ka.iter().any(|x| kb.iter().any(|y| x.starts_with(y) || y.starts_with(x)));
            if hit {
                out.push((a.clone(), b.clone()));
            }
        }
    }
    out
}

/// omaflake's key rule: without a mode a key is one chord starting with Super or Ctrl – except an editor's own keys
fn breaks_key_rule(b: &Binding, modules: &BTreeMap<String, &Module>) -> bool {
    if b.mode.is_some() || modules.get(&b.context).is_some_and(|m| m.own_keys) {
        return false;
    }
    match parse_key(&b.key) {
        Some(seq) => seq.len() != 1 || !seq[0].0.iter().any(|m| *m == "Super" || *m == "Ctrl"),
        None => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::modules::ModuleAction;
    use std::path::PathBuf;

    fn module(name: &str, provides: &[&str], needs: &[&str], actions: &[(&str, &str, Option<&str>)]) -> Module {
        Module {
            id: format!("o.nixos.{name}"),
            author: "o".into(),
            platform: GuestKind::Nixos,
            name: name.into(),
            version: "1.2.0".into(),
            title: name.into(),
            description: String::new(),
            descriptions: BTreeMap::new(),
            tags: vec![],
            provides: provides.iter().map(|s| s.to_string()).collect(),
            needs: needs.iter().map(|s| s.to_string()).collect(),
            enhances: vec![],
            runs_in: vec![],
            desktop: false,
            own_keys: false,
            unfree: false,
            actions: actions
                .iter()
                .map(|(a, k, mode)| {
                    (
                        a.to_string(),
                        ModuleAction {
                            title: a.to_string(),
                            key: k.to_string(),
                            mode: mode.map(str::to_string),
                            ..Default::default()
                        },
                    )
                })
                .collect(),
            source: "s".into(),
            hash: None,
            folder: PathBuf::new(),
        }
    }

    fn spec(modules: &[&str], keymaps: &[&str]) -> RecipeSpec {
        RecipeSpec {
            platform: GuestKind::Nixos,
            modules: modules.iter().map(|m| PartRef::parse(&Value::String(m.to_string())).unwrap()).collect(),
            look: None,
            keymaps: keymaps.iter().map(|m| PartRef::new(m)).collect(),
            requirements: Requirements::default(),
            locales: LocaleRules::default(),
            data: Value::Null,
        }
    }

    #[test]
    fn references_and_versions() {
        let r = PartRef::parse(&serde_json::json!({"id": "o/latex", "min": "1.0.0", "max": "1.9.0"})).unwrap();
        assert!(
            r.allows("1.2.0") && r.allows("1.0.0") && r.allows("1.9.0") && !r.allows("2.0.0") && !r.allows("0.9.9")
        );
        assert_eq!(r.label(), "o/latex ≥ 1.0.0 ≤ 1.9.0");
        let exact = PartRef::parse(&serde_json::json!({"id": "o/latex", "version": "1.2.0"})).unwrap();
        assert!(exact.allows("1.2.0") && !exact.allows("1.2.1"));
        assert!(PartRef::new("o/x").allows("7.0.0") && PartRef::new("o/x").allows(""), "none given: any, the newest");
        assert!(PartRef::parse(&Value::String("x".into())).is_none());
        assert!(PartRef::parse(&serde_json::json!({"id": "o/x", "min": "1.0"})).is_none());
        assert_eq!(version("1.10.0"), Some((1, 10, 0)));
        assert!(version("01.0.0").is_none());
    }

    #[test]
    fn a_recipe_file() {
        let s = RecipeSpec::parse(
            "\"$schema\" = \"https://omaflake.dev/schema/recipe/v2.json\"\nplatform = \"nixos\"\n\
             modules = [\"o/a\", { id = \"o/b\", min = \"1.0.0\" }]\nlook = \"o/l\"\nkeymaps = [\"o/k\"]\n\
             [requirements]\ncpus = 2\nmemory_gb = 4\ndisk_gb = 20\n[locales]\nrtl = false\n",
        )
        .unwrap();
        assert_eq!((s.modules.len(), s.modules[1].min.as_deref(), s.requirements.cpus), (2, Some("1.0.0"), 2));
        assert_eq!(s.locales.outside("ar_EG.UTF-8"), Some(Outside::RightToLeft));
        assert!(s.locales.outside("de_DE.UTF-8").is_none());
        assert!(RecipeSpec::parse("\"$schema\" = \"https://omaflake.dev/schema/recipe/v1.json\"\n").is_err());
    }

    #[test]
    fn issues() {
        let mut terminal = module("wezterm", &["terminal"], &[], &[]);
        terminal.runs_in = vec![];
        let mut zellij = module(
            "zellij",
            &["multiplexer"],
            &["terminal"],
            &[("unlock", "Ctrl+g", None), ("lock", "Ctrl+g", Some("normal"))],
        );
        zellij.runs_in = vec!["terminal".into()];
        let mut nvim = module(
            "neovim",
            &["editor", "default-editor"],
            &[],
            &[("files", "Space f f", None), ("x", "Ctrl+g", None)],
        );
        nvim.runs_in = vec!["terminal".into(), "multiplexer".into()];
        nvim.own_keys = true;
        let emacs = module("emacs", &["editor", "default-editor"], &[], &[]);
        let mut desk =
            module("hyprland", &["desktop"], &["terminal"], &[("terminal", "Super+Return", None), ("bad", "t", None)]);
        desk.desktop = true;
        let all = vec![terminal, zellij, nvim, emacs, desk];
        let caps = BTreeMap::from([(
            "default-editor".to_string(),
            Capability { exclusive: true, description: String::new() },
        )]);
        let c = Composition::of(
            &spec(&["o/hyprland", "o/zellij", "o/neovim", "o/emacs", "o/missing"], &["o/k"]),
            &all,
            &[],
            &caps,
        );
        assert!(c.blocked());
        let kinds: Vec<String> =
            c.issues.iter().map(|i| serde_json::to_value(i).unwrap()["kind"].as_str().unwrap().to_string()).collect();
        assert!(kinds.contains(&"missingModule".to_string()) && kinds.contains(&"missingKeymap".to_string()));
        assert!(c.issues.iter().any(
            |i| matches!(i, Issue::Needs { module, capability } if module == "o/hyprland" && capability == "terminal")
        ));
        assert!(c
            .issues
            .iter()
            .any(|i| matches!(i, Issue::Incompatible { capability, .. } if capability == "default-editor")));
        // Zellij's unlock and Neovim's Ctrl+g (Neovim runs inside Zellij); Zellij's lock is in a mode – no overlap there
        let overlaps: Vec<(String, String)> = c
            .issues
            .iter()
            .filter_map(|i| match i {
                Issue::KeysOverlap { first, second } => Some((first.action.clone(), second.action.clone())),
                _ => None,
            })
            .collect();
        assert_eq!(overlaps, [("unlock".to_string(), "x".to_string())]);
        // "t" on the desktop breaks the rule; Neovim's "Space f f" is its own
        let rule: Vec<&str> = c
            .issues
            .iter()
            .filter_map(|i| if let Issue::KeyRule { binding } = i { Some(binding.key.as_str()) } else { None })
            .collect();
        assert_eq!(rule, ["t"]);
        // A set overrides a default
        let set = Keymap {
            id: "o.keymap.k".into(),
            author: "o".into(),
            name: "k".into(),
            version: "1.0.0".into(),
            title: "k".into(),
            description: String::new(),
            descriptions: BTreeMap::new(),
            keys: BTreeMap::from([("o/hyprland".into(), BTreeMap::from([("terminal".into(), "Super+t".into())]))]),
            source: "s".into(),
            hash: None,
            folder: PathBuf::new(),
        };
        let c = Composition::of(&spec(&["o/hyprland", "o/wezterm"], &["o/k"]), &all, &[set], &caps);
        let t = c.bindings.iter().find(|b| b.action == "terminal").unwrap();
        assert_eq!((t.key.as_str(), t.from.as_deref(), t.context.as_str()), ("Super+t", Some("o/k"), "desktop"));
        assert!(!c.blocked());
    }

    #[test]
    fn keys() {
        assert!(parse_key("Super+Shift+Return").is_some() && parse_key("Space f f").unwrap().len() == 3);
        assert!(
            parse_key("Hyper+x").is_none() && parse_key("Super+Super+x").is_none() && parse_key("Super+Q").is_none()
        );
        assert_eq!(expand(&parse_key("Super+Digit").unwrap()).len(), 9);
    }
}
