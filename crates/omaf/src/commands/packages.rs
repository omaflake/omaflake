// SPDX-License-Identifier: AGPL-3.0-or-later
//! Packages (plan B108): export, import, run from another place – port of Sources/omaf/Packages.swift.

use std::path::Path;

use omaflake_core::{Error, Package, PackageKind, RecipeId, RecipeLibrary, Settings, Transfer, VmStatus};

use crate::util::{folder_path, Env, Outcome};

fn print_line(line: &str) {
    println!("{line}");
}

pub fn export_vm(name: &str, folder: &str, with_image: bool) -> Outcome {
    let env = Env::new();
    let transfer = Transfer::from_backends(&env.backends()?);
    transfer.export_vm(name, &folder_path(folder), with_image, &print_line)?;
    Ok(())
}

pub fn export_image(name: &str, folder: &str) -> Outcome {
    let env = Env::new();
    Transfer::from_backends(&env.backends()?).export_image(name, &folder_path(folder), &print_line)?;
    Ok(())
}

pub fn export_recipe(recipe: &str, folder: &str) -> Outcome {
    let env = Env::new();
    let settings = Settings::load(&env.paths);
    let library = RecipeLibrary::new(env.paths.clone());
    let ids: Vec<String> = library.available(&settings).into_iter().map(|r| r.id).collect();
    let id = RecipeId::resolve(recipe, &ids, None)?;
    let dest = library.export_recipe(&id, &folder_path(folder), &settings)?;
    println!("Exported to {}", dest.display());
    Ok(())
}

pub fn import(path: &str, name: Option<&str>, replace: bool) -> Outcome {
    let env = Env::new();
    let result =
        Transfer::from_backends(&env.backends()?).import_package(&folder_path(path), name, replace, &print_line)?;
    for note in &result.notes {
        println!("{note}");
    }
    if result.kind == PackageKind::Recipe {
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
    let env = Env::new();
    let url = folder_path(path);
    let name = Transfer::from_backends(&env.backends()?).use_external(&url)?;
    println!("{name} runs from {}", url.display());
    if Transfer::is_on_network_volume(&url) {
        println!("Note: on a network volume – slower, and the VM breaks when the connection does.");
    }
    Ok(())
}

pub fn external_remove(name: &str) -> Outcome {
    let env = Env::new();
    let backends = env.backends()?;
    if let Ok(c) = backends.store.load(name) {
        if backends.backend_for(&c)?.status(&c) != VmStatus::Stopped {
            return Err(Error::new(format!("\"{name}\" is running – first run: omaf stop {name}")).into());
        }
    }
    backends.store.forget(name)?;
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
