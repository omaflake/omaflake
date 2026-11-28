// SPDX-License-Identifier: AGPL-3.0-or-later
//! Packages (plan B108): export, import, run from another place – port of Sources/omaf/Packages.swift.

use std::path::Path;

use omaflake_core::{Package, PackageKind, RecipeId, RecipeLibrary, Settings, Transfer};
use omaflake_sdk::types::{ExportResult, ImportResult, PackageKindDto};
use serde_json::json;

use super::service;
use crate::util::{Env, Outcome, folder_path};

/// `packages.export` as a job of the core service: its log lines on stdout, the package's place as the answer
fn export(kind: &str, name: &str, folder: &str, with_image: Option<bool>) -> omaflake_core::Result<String> {
    let mut params = json!({"kind": kind, "name": name, "folder": folder_path(folder)});
    if let Some(w) = with_image {
        params["withImage"] = json!(w);
    }
    let job = service::rpc_job("packages.export", params, &mut service::to_stdout)?;
    Ok(service::finished_as::<ExportResult>(job, "The export")?.path)
}

pub fn export_vm(name: &str, folder: &str, with_image: bool) -> Outcome {
    export("vm", name, folder, Some(with_image))?;
    Ok(())
}

pub fn export_image(name: &str, folder: &str) -> Outcome {
    export("image", name, folder, None)?;
    Ok(())
}

pub fn export_recipe(recipe: &str, folder: &str) -> Outcome {
    let env = Env::new();
    let settings = Settings::load(&env.paths);
    let library = RecipeLibrary::new(env.paths.clone());
    let ids: Vec<String> = library.available(&settings).into_iter().map(|r| r.id).collect();
    let id = RecipeId::resolve_newest(recipe, &ids, &library.places(&settings), &|_| true, None)?;
    println!("Exported to {}", export("recipe", &id, folder, None)?);
    Ok(())
}

pub fn import(path: &str, name: Option<&str>, replace: bool) -> Outcome {
    let mut params = json!({"path": folder_path(path), "replace": replace});
    if let Some(n) = name {
        params["name"] = json!(n);
    }
    let job = service::rpc_job("packages.import", params, &mut service::to_stdout)?;
    let result: ImportResult = service::finished_as(job, "The import")?;
    for note in &result.notes {
        println!("{note}");
    }
    if result.kind == PackageKindDto::Recipe {
        println!("{}", imported_recipes_line(&result.names));
    }
    Ok(())
}

/// What `omaf import` says about recipes
pub fn imported_recipes_line(names: &[String]) -> String {
    if names.is_empty() {
        "These recipes are here already, unchanged – nothing to import".into()
    } else {
        format!("Recipes: {}", names.iter().map(|n| RecipeId::qualified(n)).collect::<Vec<_>>().join(", "))
    }
}

pub fn external_list() -> Outcome {
    let env = Env::new();
    let paths = Settings::load_file(&env.paths.config_file).external_vms.unwrap_or_default();
    if paths.is_empty() {
        println!("No external VMs. Add one with: omaf external add <package>");
    }
    for p in paths {
        let there = Path::new(&p).join("instance.json").exists();
        let name = Package::name_of(Path::new(&p), PackageKind::Vm).unwrap_or_else(|| p.clone());
        println!("{name}  {p}{}", if there { "" } else { "  (not there)" });
    }
    Ok(())
}

pub fn external_add(path: &str) -> Outcome {
    let url = folder_path(path);
    // The network volume's note is this command's own, after the fact – the service is told not to ask
    let vm = service::rpc("vms.useExternal", json!({"path": url, "confirmNetwork": true}))?;
    println!("{} runs from {}", vm["name"].as_str().unwrap_or("?"), url.display());
    if Transfer::is_on_network_volume(&url) {
        println!("Note: on a network volume – slower, and the VM breaks when the connection does.");
    }
    Ok(())
}

pub fn external_remove(name: &str) -> Outcome {
    // Refused by the service while it runs ("<name> is running – stop it first")
    service::rpc("vms.forget", json!({"name": name}))?;
    println!("{name} is no longer listed; its package stays at its place.");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn import_lines() {
        assert_eq!(imported_recipes_line(&[]), "These recipes are here already, unchanged – nothing to import");
        assert_eq!(
            imported_recipes_line(&["omaflake.nixos.dev@apple".into(), "me.macos.x".into()]),
            "Recipes: omaflake/nixos/dev, me/macos/x"
        );
    }
}
