// SPDX-License-Identifier: AGPL-3.0-or-later
//! Recipe sources (spec §11.3, plan B107) – port of the `Sources*` commands of Sources/omaf/Sources.swift.

use omaflake_core::{RecipeLibrary, RecipeSource, RecipeSourceKind, SourceClient, SourceTokens};

use crate::cli;
use crate::util::{pad, read_line, usage, Env, Failure, Outcome};

/// Swift `RecipeSource.Kind.rawValue`
pub fn kind_name(kind: RecipeSourceKind) -> &'static str {
    match kind {
        RecipeSourceKind::Github => "github",
        RecipeSourceKind::Url => "url",
        RecipeSourceKind::Folder => "folder",
    }
}

/// A source by its number in `omaf sources` (1 = first) or its ID (or location)
pub fn source_index(reference: &str, sources: &[RecipeSource], command: &'static str) -> Result<usize, Failure> {
    if let Ok(n) = reference.parse::<usize>() {
        if (1..=sources.len()).contains(&n) {
            return Ok(n - 1);
        }
    }
    sources
        .iter()
        .position(|s| s.id() == reference || s.location == reference)
        .ok_or_else(|| usage(&["sources", command], format!("No source {reference} – see omaf sources")))
}

/// One line of `omaf sources`
pub fn source_line(number: usize, kind: RecipeSourceKind, name: &str, notes: &[String]) -> String {
    format!("{number}  {}{}{}", pad(kind_name(kind), 7), pad(name, 40), notes.join(", "))
}

pub fn list() -> Outcome {
    let env = Env::new();
    let settings = env.settings();
    let installed = RecipeLibrary::new(env.paths.clone()).installed();
    for (i, s) in settings.recipe_sources().iter().enumerate() {
        let mut notes: Vec<String> = Vec::new();
        if !s.is_enabled() {
            notes.push("off".into());
        }
        if s.is_private_repository() {
            notes.push(if !SourceTokens::has(s) { "private, no token" } else { "private" }.into());
        }
        if s.is_official() {
            notes.push("official".into());
        }
        if s.kind == RecipeSourceKind::Folder {
            let count = RecipeLibrary::recipe_folders(std::path::Path::new(&s.location)).len();
            notes.push(format!("{count} recipes, used as they are"));
        } else {
            let id = s.id();
            let count = installed.values().filter(|r| r.source == id).count();
            notes.push(format!("{count} installed"));
        }
        println!("{}", source_line(i + 1, s.kind, &s.display_name(), &notes));
    }
    if settings.sources.is_none() {
        println!("(the default – omaf sources add … makes the list your own)");
    }
    Ok(())
}

pub fn add(a: &cli::SourcesAdd) -> Outcome {
    let env = Env::new();
    let mut s = RecipeSource::parse(&a.location, a.private)?;
    if let Some(branch) = &a.branch {
        s.branch = Some(branch.clone());
    }
    let mut settings = env.settings();
    let mut list = settings.recipe_sources();
    if list.iter().any(|x| x.id() == s.id()) {
        return Err(usage(&["sources", "add"], format!("{} is already a source", s.display_name())));
    }
    let mut token = None;
    if s.is_private_repository() {
        match read_line() {
            Some(line) if !line.is_empty() => token = Some(line),
            _ => return Err(usage(&["sources", "add"], "No token on stdin")),
        }
    }
    // Checked before it is kept: the repository (with the token), the catalog, or the folder's recipes
    println!("{}: {}", s.display_name(), SourceClient::new(s.clone(), token.clone()).verify()?);
    if let Some(token) = &token {
        // Without a keyring (Linux server, SSH session): a 0600 file, and the user is told
        if let Some(note) = SourceTokens::set(Some(token), &s)?.and_then(|p| p.note()) {
            println!("{note}");
        }
    }
    if a.first {
        list.insert(0, s);
    } else {
        list.push(s);
    }
    settings.sources = Some(list);
    settings.save(&env.paths)?;
    println!("Added. Recipes: omaf recipes available");
    Ok(())
}

pub fn remove(reference: &str) -> Outcome {
    let env = Env::new();
    let mut settings = env.settings();
    let mut list = settings.recipe_sources();
    let i = source_index(reference, &list, "remove")?;
    let s = list.remove(i);
    let _ = SourceTokens::set(None, &s);
    settings.sources = Some(list);
    settings.save(&env.paths)?;
    println!("Removed {}.", s.display_name());
    Ok(())
}

pub fn toggle(reference: &str, on: bool) -> Outcome {
    let env = Env::new();
    let mut settings = env.settings();
    let mut list = settings.recipe_sources();
    let i = source_index(reference, &list, if on { "enable" } else { "disable" })?;
    list[i].enabled = if on { None } else { Some(false) };
    let name = list[i].display_name();
    settings.sources = Some(list);
    settings.save(&env.paths)?;
    println!("{name}: {}", if on { "on" } else { "off" });
    Ok(())
}

pub fn check() -> Outcome {
    let mut failed = false;
    for s in Env::new().settings().recipe_sources().into_iter().filter(RecipeSource::is_enabled) {
        match SourceClient::new(s.clone(), None).verify() {
            Ok(text) => println!("ok   {}: {text}", s.display_name()),
            Err(e) => {
                failed = true;
                println!("FAIL {}: {e}", s.display_name());
            }
        }
    }
    if failed {
        return Err(Failure::Exit(1));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn folder(location: &str) -> RecipeSource {
        RecipeSource::new(RecipeSourceKind::Folder, location, None, None, None)
    }

    #[test]
    fn sources_by_number_or_id() {
        let list = vec![folder("/a"), folder("/b")];
        assert_eq!(source_index("2", &list, "remove").unwrap(), 1);
        assert_eq!(source_index("/a", &list, "remove").unwrap(), 0);
        assert_eq!(source_index(&list[1].id(), &list, "remove").unwrap(), 1);
        match source_index("3", &list, "remove") {
            Err(Failure::Usage { message, command }) => {
                assert_eq!(message, "No source 3 – see omaf sources");
                assert_eq!(command, ["sources", "remove"]);
            }
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn source_lines() {
        assert_eq!(
            source_line(
                1,
                RecipeSourceKind::Folder,
                "~/recipes",
                &["off".into(), "2 recipes, used as they are".into()]
            ),
            format!("1  folder {}off, 2 recipes, used as they are", pad("~/recipes", 40))
        );
        assert_eq!(
            source_line(2, RecipeSourceKind::Github, "omaflake/recipes", &["official".into(), "3 installed".into()]),
            format!("2  github {}official, 3 installed", pad("omaflake/recipes", 40))
        );
    }
}
