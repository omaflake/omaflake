// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf looks` (plan B141): how the guests look – installed from the sources' catalogs (or used from a local folder
//! source), one of them the default for new images; `omaf build --look` chooses another.

use omaflake_core::Settings;
use omaflake_core::looks::LookStore;
use omaflake_core::recipe_sources::{RecipeSource, RecipeSourceKind, SourceClient};

use crate::util::{Env, Outcome, pad, usage};

fn store(env: &Env) -> LookStore {
    LookStore::new(env.paths.clone())
}

pub fn list() -> Outcome {
    let env = Env::new();
    let s = Settings::load(&env.paths);
    let looks = store(&env);
    let (all, unreadable) = looks.list_with_errors(&s);
    let default = looks.default_look(&s).map(|l| l.id);
    let images = env.images().list();
    for l in &all {
        let used = images.iter().filter(|i| i.info.look.as_deref() == Some(l.id.as_str())).count();
        let kind = if l.plain { "plain" } else { l.appearance.as_deref().unwrap_or("dark") };
        println!(
            "{} {}{}{}{}",
            if default.as_deref() == Some(l.id.as_str()) { "*" } else { " " },
            pad(&l.reference(), 26),
            pad(&l.title, 22),
            pad(kind, 7),
            match used {
                0 => String::new(),
                1 => "  1 image".into(),
                n => format!("  {n} images"),
            }
        );
    }
    for (id, e) in unreadable {
        eprintln!("⚠ {id}: {e}");
    }
    if all.is_empty() {
        println!("(no look yet – images are built with one: omaf looks install recommended)");
    }
    Ok(())
}

/// The enabled remote sources with their catalogs (errors on stderr)
fn catalogs(s: &Settings) -> Vec<(RecipeSource, omaflake_core::recipe_sources::RecipeCatalog)> {
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
    let here: Vec<String> = store(&env).list(&s).into_iter().map(|l| l.id).collect();
    let mut any = false;
    for (source, catalog) in catalogs(&s) {
        for l in catalog.looks.iter().filter(|l| !here.contains(&l.id)) {
            any = true;
            let kind = if l.plain { "plain" } else { l.appearance.as_deref().unwrap_or("dark") };
            println!(
                "  {}{}{}{}",
                pad(&format!("{}/{}", l.author, l.name), 26),
                pad(&l.title, 22),
                pad(kind, 7),
                source.display_name()
            );
        }
    }
    if !any {
        println!("(nothing besides what is installed)");
    }
    Ok(())
}

pub fn install(reference: &str) -> Outcome {
    let env = Env::new();
    let s = Settings::load(&env.paths);
    let looks = store(&env);
    let wanted: Vec<String> = if reference == "recommended" {
        LookStore::RECOMMENDED.iter().map(|x| x.to_string()).collect()
    } else {
        vec![
            LookStore::id_of(reference)
                .ok_or_else(|| usage(&["looks", "install"], format!("{reference}: write <author>/<name>")))?,
        ]
    };
    let found = catalogs(&s);
    for id in &wanted {
        let Some((source, catalog)) = found.iter().find(|(_, c)| c.looks.iter().any(|l| &l.id == id)) else {
            return Err(usage(
                &["looks", "install"],
                format!("{id} is in none of your sources – see omaf looks available"),
            ));
        };
        let look = looks.install(id, catalog, &SourceClient::new(source.clone(), None))?;
        println!("Look {} installed ({}).", look.title, look.reference());
    }
    Ok(())
}

pub fn remove(reference: &str) -> Outcome {
    let env = Env::new();
    let s = Settings::load(&env.paths);
    let looks = store(&env);
    let Some(look) = looks.find(&s, reference) else {
        return Err(usage(&["looks", "rm"], format!("No look {reference} – see omaf looks")));
    };
    let used = env.images().list().iter().filter(|i| i.info.look.as_deref() == Some(look.id.as_str())).count();
    if used > 0 {
        return Err(usage(
            &["looks", "rm"],
            format!("Images are built with {} ({used}) – remove them first", look.title),
        ));
    }
    looks.remove(&look.id)?;
    println!("Look {} removed.", look.title);
    Ok(())
}

pub fn set_default(reference: &str) -> Outcome {
    let env = Env::new();
    let s = Settings::load(&env.paths);
    let looks = store(&env);
    let Some(look) = looks.find(&s, reference) else {
        return Err(usage(&["looks", "default"], format!("No look {reference} – see omaf looks")));
    };
    looks.set_default(&look.id)?;
    println!("New images get the look {}.", look.title);
    Ok(())
}
