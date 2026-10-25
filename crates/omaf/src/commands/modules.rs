// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf modules` (plan B142): the modules and key binding sets recipes are made of – installed from the sources'
//! catalogs (or used from a local folder source). Installing a recipe brings what it needs; these install and remove
//! one by hand.

use omaflake_core::keymaps::KeymapStore;
use omaflake_core::modules::ModuleStore;
use omaflake_core::parts::PartKind;
use omaflake_core::recipe_sources::{RecipeCatalog, RecipeSource, RecipeSourceKind, SourceClient};
use omaflake_core::Settings;

use crate::util::{pad, usage, Env, Outcome};

/// "<author>/<platform>/<name>" → a module's id, "<author>/<name>" → key bindings' (ids pass as they are)
fn id_of(reference: &str) -> Option<String> {
    let parts: Vec<&str> = reference.split('/').collect();
    match parts.as_slice() {
        [author, platform, name] => PartKind::Module.id_of(&format!("{author}.{platform}.{name}"), None),
        [_, _] => PartKind::Keymap.id_of(reference, None),
        _ => PartKind::Module.id_of(reference, None).or_else(|| PartKind::Keymap.id_of(reference, None)),
    }
}

pub fn list() -> Outcome {
    let env = Env::new();
    let s = Settings::load(&env.paths);
    let (modules, mut unreadable) = ModuleStore::new(env.paths.clone()).list_with_errors(&s);
    let (keymaps, more) = KeymapStore::new(env.paths.clone()).list_with_errors(&s);
    unreadable.extend(more);
    for m in &modules {
        println!(
            "  {}{}{}{}",
            pad(&format!("{}/{}/{}", m.author, m.platform.as_str(), m.name), 32),
            pad(&m.title, 22),
            pad(&m.version, 8),
            m.provides.join(", ")
        );
    }
    for k in &keymaps {
        println!("  {}{}{}key bindings", pad(&k.reference(), 32), pad(&k.title, 22), pad(&k.version, 8));
    }
    for (id, e) in unreadable {
        eprintln!("⚠ {id}: {e}");
    }
    if modules.is_empty() && keymaps.is_empty() {
        println!("(no module yet – installing a recipe brings the modules it is made of)");
    }
    Ok(())
}

/// The enabled remote sources with their catalogs (errors on stderr)
fn catalogs(s: &Settings) -> Vec<(RecipeSource, RecipeCatalog)> {
    s.recipe_sources()
        .into_iter()
        .filter(|x| x.is_enabled() && x.kind != RecipeSourceKind::Folder)
        .filter_map(|x| match SourceClient::new(x.clone(), None).catalog() {
            Ok(c) => Some((x, c)),
            Err(e) => {
                eprintln!("⚠ {}: {}", x.display_name(), e.message());
                None
            }
        })
        .collect()
}

pub fn available() -> Outcome {
    let env = Env::new();
    let s = Settings::load(&env.paths);
    let here: Vec<String> = ModuleStore::new(env.paths.clone())
        .list(&s)
        .into_iter()
        .map(|m| m.id)
        .chain(KeymapStore::new(env.paths.clone()).list(&s).into_iter().map(|k| k.id))
        .collect();
    let mut any = false;
    for (source, catalog) in catalogs(&s) {
        for m in catalog.modules.iter().filter(|m| !here.contains(&m.id)) {
            any = true;
            let reference = format!("{}/{}/{}", m.author, m.platform.as_str(), m.name);
            println!("  {}{}{}{}", pad(&reference, 32), pad(&m.title, 22), pad(&m.version, 8), source.display_name());
        }
        for k in catalog.keymaps.iter().filter(|k| !here.contains(&k.id)) {
            any = true;
            let reference = format!("{}/{}", k.author, k.name);
            println!("  {}{}{}{}", pad(&reference, 32), pad(&k.title, 22), pad(&k.version, 8), source.display_name());
        }
    }
    if !any {
        println!("(nothing besides what is here)");
    }
    Ok(())
}

pub fn install(reference: &str) -> Outcome {
    let env = Env::new();
    let s = Settings::load(&env.paths);
    let id = id_of(reference).ok_or_else(|| {
        usage(&["modules", "install"], format!("{reference}: write <author>/<platform>/<name> or <author>/<name>"))
    })?;
    let keymap = PartKind::Keymap.parse_id(&id).is_some();
    for (source, catalog) in catalogs(&s) {
        let has = if keymap {
            catalog.keymaps.iter().any(|k| k.id == id)
        } else {
            catalog.modules.iter().any(|m| m.id == id)
        };
        if !has {
            continue;
        }
        let client = SourceClient::new(source.clone(), None);
        let (title, version) = if keymap {
            let k = KeymapStore::new(env.paths.clone()).install(&id, &catalog, &client)?;
            (k.title, k.version)
        } else {
            let m = ModuleStore::new(env.paths.clone()).install(&id, &catalog, &client)?;
            (m.title, m.version)
        };
        println!("Installed {title} {version} from {}.", source.display_name());
        return Ok(());
    }
    Err(usage(&["modules", "install"], format!("{reference} is in none of your sources – see omaf modules available")))
}

pub fn remove(reference: &str) -> Outcome {
    let env = Env::new();
    let id = id_of(reference).ok_or_else(|| {
        usage(&["modules", "rm"], format!("{reference}: write <author>/<platform>/<name> or <author>/<name>"))
    })?;
    if PartKind::Keymap.parse_id(&id).is_some() {
        KeymapStore::new(env.paths.clone()).remove(&id)?;
    } else {
        ModuleStore::new(env.paths.clone()).remove(&id)?;
    }
    println!("Removed {reference} – recipes made of it cannot be built until it is back.");
    Ok(())
}
