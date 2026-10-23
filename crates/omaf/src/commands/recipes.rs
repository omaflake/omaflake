// SPDX-License-Identifier: AGPL-3.0-or-later
//! Recipes and builds – port of `RecipesList` and `Build` (Sources/omaf/Omaf.swift) and of `RecipesAvailable`,
//! `RecipesInstall`, `RecipesRemove`, `RecipesVersions` (Sources/omaf/Sources.swift).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use omaflake_core::mac_image_builder::BuildRequest;
use omaflake_core::platform::user_name;
use omaflake_core::{
    BuildLog, Cancellation, Error, GuestKind, Lengths, MacImageBuilder, OmaflakeId, Platform, RecipeDefaults, RecipeId,
    RecipeLibrary, RecipeRef, RecipeSourceKind, SourceClient,
};

use omaflake_core::api::client::Client;
use omaflake_core::api::service::socket_path;
use omaflake_core::api::types::{BuildResult, JobState};
use omaflake_core::error::codes;
use serde_json::{json, Value};

use super::service;
use crate::cli;
use crate::tty::Interrupts;
use crate::util::{choose_recipe, folder_path, local_date, pad, read_password, shortest_argument, usage, Env, Outcome};

/// Header of `omaf recipes`
pub fn recipes_header() -> String {
    pad("RECIPE", 13)
        + &pad("AUTHOR", 10)
        + &pad("KIND", 7)
        + &pad("ARCH", 16)
        + &pad("CPU", 4)
        + &pad("RAM", 6)
        + &pad("DISK", 6)
        + &pad("NETWORK", 9)
        + &pad("SHARE", 6)
        + &pad("EPHEMERAL", 10)
        + "IMAGE"
}

/// One row of `omaf recipes`: `label` is the name (with ⚗ and the distinction), `image` the last column
pub fn recipes_row(label: &str, d: &RecipeDefaults, image: &str) -> String {
    let yes = |b: bool| if b { "yes" } else { "no" };
    let arch: Vec<&str> = d.systems.iter().map(|s| Platform::architecture(s)).collect();
    pad(label, 13)
        + &pad(d.author.as_deref().unwrap_or("–"), 10)
        + &pad(d.kind.as_str(), 7)
        + &pad(&arch.join(","), 16)
        + &pad(&d.cpus.to_string(), 4)
        + &pad(&format!("{}G", d.memory_gb), 6)
        + &pad(&format!("{}G", d.disk_gb), 6)
        + &pad(d.network.as_str(), 9)
        + &pad(yes(d.share), 6)
        + &pad(yes(d.ephemeral), 10)
        + image
}

/// Name column of `omaf recipes`: lab recipes marked ⚗, the source added where two lines share the name
pub fn recipe_label(name: &str, lab: bool, distinction: Option<&String>) -> String {
    let mut label = if lab { format!("{name} ⚗") } else { name.to_string() };
    if let Some(d) = distinction {
        label += &format!(" ({d})");
    }
    label
}

pub fn list(a: &cli::RecipesList) -> Outcome {
    let env = Env::new();
    let settings = env.settings();
    let show_lab = a.all || settings.developer_mode.unwrap_or(false);
    let flake = settings.flake_path()?;
    let images = env.images();
    let all: HashMap<String, RecipeDefaults> =
        images.recipe_defaults(&flake)?.into_iter().filter(|(_, d)| show_lab || !d.is_lab()).collect();
    let defaults: HashMap<String, RecipeDefaults> =
        all.iter().filter(|(_, d)| a.all_systems || d.runs_here()).map(|(k, d)| (k.clone(), d.clone())).collect();
    let outdated = if a.check {
        let here: HashMap<String, RecipeDefaults> =
            defaults.iter().filter(|(_, d)| d.runs_here()).map(|(k, d)| (k.clone(), d.clone())).collect();
        images.outdated(&here, &flake)
    } else {
        HashSet::new()
    };
    let library = RecipeLibrary::new(env.paths.clone());
    let distinctions = library.distinctions(&settings);
    let lengths = Lengths::current(&env.paths);
    let origins = if a.long { origins(&env) } else { HashMap::new() };
    println!("{}", recipes_header());
    let mut sorted: Vec<(&String, &RecipeDefaults)> = defaults.iter().collect();
    sorted.sort_by(|(a, x), (b, y)| (x.kind.as_str(), a).cmp(&(y.kind.as_str(), b)));
    for (id, d) in sorted {
        let name = RecipeId::display_name(id, Some(d));
        let arg = shortest_argument(id, &defaults);
        let image = if !d.runs_here() {
            "– (not for this computer)".to_string()
        } else {
            match images.current_name(id) {
                Some(n) if outdated.contains(id) => format!("{n}  ← recipe changed, rebuild: omaf build {arg}"),
                Some(n) => n,
                None => format!("– (omaf build {arg})"),
            }
        };
        println!("{}", recipes_row(&recipe_label(&name, d.is_lab(), distinctions.get(id)), d, &image));
        if a.long {
            println!("    {}", OmaflakeId::recipe(id, lengths.recipe));
            if let Some(text) = &d.description {
                println!("    {text}");
            }
            if let Some(origin) = origins.get(id) {
                println!("    from {origin}");
            }
        }
    }
    for u in library.unreadable(&settings) {
        println!("⚠ {}: {}", RecipeId::qualified(&u.name), u.reason);
    }
    if defaults.len() < all.len() {
        println!("{} more for other architectures: omaf recipes --all-systems", all.len() - defaults.len());
    }
    if all.is_empty() {
        println!(
            "No recipes yet – install some: omaf recipes available, then omaf recipes install <author>/<kind>/<name>"
        );
    }
    Ok(())
}

/// Where each recipe comes from (a source's name, or "downloaded from …")
fn origins(env: &Env) -> HashMap<String, String> {
    let settings = env.settings();
    let mut sources = HashMap::new();
    for s in settings.recipe_sources() {
        sources.entry(s.id()).or_insert(s);
    }
    RecipeLibrary::new(env.paths.clone())
        .available(&settings)
        .into_iter()
        .map(|r| {
            let origin = match sources.get(&r.source) {
                Some(s) if s.kind == RecipeSourceKind::Folder => format!("the folder {}", s.display_name()),
                Some(s) => format!("{} (downloaded)", s.display_name()),
                None => r.source.clone(),
            };
            (r.id, origin)
        })
        .collect()
}

/// `omaf build` as a job of the core service: Ctrl-C cancels the job; its log lines go to stderr as without
fn build_in_service(
    client: &Client,
    env: &Env,
    recipe: &str,
    password: Option<String>,
    base: Option<String>,
    a: &cli::Build,
) -> Outcome {
    let rpc = |method: &str, params: Value| client.call(method, params).map_err(Error::from);
    rpc("events.subscribe", json!({"topics": ["jobs"]}))?;
    let mut params = json!({"recipe": recipe, "keepFailed": a.keep_failed});
    if let Some(p) = password {
        params["password"] = json!(p);
    }
    if let Some(b) = base {
        params["base"] = json!(b);
    }
    let job = rpc("images.build", params)?;
    eprintln!("Building in the omaflake service (job {}) – Ctrl-C cancels it", job["id"].as_str().unwrap_or("?"));
    let mut print = |lines: &[String]| {
        for line in lines {
            eprintln!("{line}");
        }
    };
    let ended = service::follow(client, &job, &mut print)?;
    if a.verbose && ended.hidden_lines > 0 {
        if let Some(file) = &ended.log_file {
            eprintln!("({} routine lines are only in {file})", ended.hidden_lines);
        }
    }
    match ended.state {
        JobState::Finished => {
            let result: BuildResult = serde_json::from_value(ended.result.unwrap_or_default())
                .map_err(|e| Error::new(format!("The service's build result: {e}")))?;
            let n = Lengths::current(&env.paths);
            let image = match env.images().image(&result.image) {
                Ok(image) => OmaflakeId::image(&image, n.image),
                Err(_) => result.image.clone(),
            };
            println!("Image {image} ready – new VMs of {} use it.", OmaflakeId::recipe(recipe, n.recipe));
            Ok(())
        }
        JobState::Cancelled => Err(Error::with_code(codes::CANCELLED, "Build cancelled").into()),
        _ => Err(Error::new(ended.error.unwrap_or_else(|| "The build failed".into())).into()),
    }
}

pub fn build(a: &cli::Build) -> Outcome {
    let env = Env::new();
    let flake = env.settings().flake_path()?;
    let images = env.images();
    let defaults = images.recipe_defaults(&flake)?;
    let chooser = |ids: &[String]| choose_recipe(&env, ids);
    let recipe = RecipeId::resolve_in(&a.recipe, &defaults, Some(&chooser))?;
    let d = &defaults[&recipe];
    if !d.runs_here() {
        return Err(Error::new(format!(
            "{} is for {} – not for this computer",
            RecipeId::qualified(&recipe),
            d.systems.join(", ")
        ))
        .into());
    }
    // Routine lines of the disk image tool are left out (BuildLog::is_noise, plan B83); --verbose shows everything
    let verbose = a.verbose;
    let log = move |line: &str| {
        if verbose || !BuildLog::is_noise(line) {
            eprintln!("{line}");
        }
    };
    // macOS: base and password first (Ctrl-C at the prompt still simply quits)
    let mut mac: Option<(MacImageBuilder, PathBuf, String, String)> = None;
    if d.kind == GuestKind::Macos {
        let builder = MacImageBuilder::new(env.paths.clone(), env.backends()?.vz()?.clone());
        let Some(base) = a.base.as_deref().map(folder_path).or_else(|| builder.newest_base()) else {
            return Err(Error::new(format!(
                "No base in {} – create one with: omaf macos install",
                env.paths.bases().display()
            ))
            .into());
        };
        let user = d.user.clone().unwrap_or_else(user_name);
        let base_name = base.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
        let password =
            read_password(&format!("Password of {user} in {base_name} (for sudo): "), a.password_stdin, false)?;
        mac = Some((builder, base, user, password));
    }
    // With the core service running, the build is its job: the app and other clients see it, it waits in the same
    // queue (plan B120); this command follows its log
    if let Some((client, _)) = service::connect(&socket_path(&env.paths)) {
        let password = mac.as_ref().map(|(_, _, _, p)| p.clone());
        let base = mac.as_ref().map(|(_, b, _, _)| b.to_string_lossy().into_owned());
        return build_in_service(&client, &env, &recipe, password, base, a);
    }
    // From here Ctrl-C cancels cleanly: the running step stops, a macOS build VM is forced off, its folder deleted
    let token = Cancellation::new();
    let interrupts = {
        let token = token.clone();
        Interrupts::handle(move || {
            eprintln!("Cancelling …");
            token.cancel();
        })?
    };
    let built = match mac {
        Some((builder, base, user, password)) => builder.build(
            &BuildRequest {
                recipe: recipe.clone(),
                flake: flake.clone(),
                user,
                password,
                base: Some(base),
                keep_failed: a.keep_failed,
            },
            &token,
            &log,
        ),
        None => images.build(&recipe, &flake, None, &token, &log),
    };
    drop(interrupts);
    let image = built?;
    let n = Lengths::current(&env.paths);
    println!(
        "Image {} ready – new VMs of {} use it.",
        OmaflakeId::image(&image, n.image),
        OmaflakeId::recipe(&recipe, n.recipe)
    );
    if let Ok(backends) = env.backends() {
        let used: HashSet<String> = backends.store.list().into_iter().map(|c| c.image).collect();
        for name in images.prune(&recipe, &used) {
            println!("Removed older image {name}");
        }
    }
    Ok(())
}

// MARK: - Installing recipes (Sources.swift)

/// One line of `omaf recipes available`
pub fn available_line(qualified: &str, shown: &str, description: &str) -> String {
    format!("  {}{}{description}", pad(qualified, 34), pad(shown, 18))
}

pub fn available(a: &cli::RecipesAvailable) -> Outcome {
    let env = Env::new();
    let settings = env.settings();
    let library = RecipeLibrary::new(env.paths.clone());
    let installed = library.installed();
    let here: HashSet<String> = library.available(&settings).into_iter().map(|r| r.id).collect();
    let show_lab = a.all || settings.developer_mode.unwrap_or(false);
    for s in settings.recipe_sources().into_iter().filter(|s| s.is_enabled() && s.kind != RecipeSourceKind::Folder) {
        let catalog = match SourceClient::new(s.clone(), None).catalog() {
            Ok(c) => c,
            Err(e) => {
                println!("{}: {e}", s.display_name());
                continue;
            }
        };
        println!("{} – {}{}", catalog.source.name, s.display_name(), if s.is_official() { " (official)" } else { "" });
        for r in
            catalog.recipes.iter().filter(|r| (show_lab || !r.lab.unwrap_or(false)) && (a.all_systems || r.runs_here()))
        {
            // By ID (plan B110): the same content is here (from any source), or an older version of this source
            let state = if here.contains(&RecipeId::make_with_hash(&r.id, &r.content_hash(), OmaflakeId::LENGTH)) {
                "installed"
            } else if installed.iter().any(|(k, v)| RecipeId::base(k) == r.id && v.source == s.id()) {
                "update available"
            } else {
                ""
            };
            let shown = if r.runs_here() {
                state.to_string()
            } else {
                let arch: Vec<&str> = r.systems.iter().map(|s| Platform::architecture(s)).collect();
                format!("for {}", arch.join(","))
            };
            println!(
                "{}",
                available_line(&format!("{}/{}/{}", r.author, r.kind.as_str(), r.name), &shown, &r.description)
            );
        }
    }
    Ok(())
}

pub fn install(a: &cli::RecipesInstall) -> Outcome {
    let env = Env::new();
    let Some(id) = RecipeRef::public_id(&a.recipe) else {
        return Err(usage(&["recipes", "install"], "Write the recipe as <author>/<kind>/<name>"));
    };
    for s in
        env.settings().recipe_sources().into_iter().filter(|s| s.is_enabled() && s.kind != RecipeSourceKind::Folder)
    {
        let client = SourceClient::new(s.clone(), None);
        let Ok(catalog) = client.catalog() else { continue };
        if !catalog.recipes.iter().any(|r| r.id == id) {
            continue;
        }
        let written = RecipeLibrary::new(env.paths.clone()).install(&id, &catalog, &client)?;
        if written.is_empty() {
            println!("{} is up to date.", a.recipe);
        } else {
            let names: Vec<String> = written.iter().map(|w| RecipeId::qualified(w)).collect();
            println!("Installed from {}: {}", s.display_name(), names.join(", "));
        }
        return Ok(());
    }
    Err(usage(
        &["recipes", "install"],
        format!("No enabled remote source has {} – see omaf recipes available", a.recipe),
    ))
}

pub fn remove(a: &cli::RecipesRemove) -> Outcome {
    let env = Env::new();
    let library = RecipeLibrary::new(env.paths.clone());
    let versions: Vec<_> = library.versions(&env.settings()).into_values().flatten().collect();
    let ids: Vec<String> = versions.iter().filter(|v| a.all || !v.is_live).map(|v| v.id.clone()).collect();
    let id = RecipeId::resolve(&a.recipe, &ids, None)?;
    if a.all {
        let source = versions.iter().find(|v| v.id == id).map(|v| v.source.as_str()).unwrap_or("");
        let (removed, kept) = library.remove_line(&RecipeId::line_key(&id, source));
        for r in removed {
            println!("Removed {}", OmaflakeId::recipe(&r, 3));
        }
        for k in kept {
            println!("Kept {k}");
        }
    } else {
        library.remove(&id)?;
        println!("Removed {}.", OmaflakeId::recipe(&id, 3));
    }
    Ok(())
}

/// A version's state in `omaf recipes versions`: "in the folder" or when it came, "newest", "images: n"
pub fn version_state(v: &omaflake_core::recipe_sources::Version) -> String {
    let added =
        if v.is_live { Some("in the folder".to_string()) } else { v.added.map(|d| local_date(&d, "%Y-%m-%d %H:%M")) };
    let parts: Vec<String> = [
        added,
        v.is_newest.then(|| "newest".to_string()),
        (!v.images.is_empty()).then(|| format!("images: {}", v.images.len())),
    ]
    .into_iter()
    .flatten()
    .collect();
    parts.join(", ")
}

pub fn versions(a: &cli::RecipesVersions) -> Outcome {
    let env = Env::new();
    let settings = env.settings();
    let library = RecipeLibrary::new(env.paths.clone());
    let lines = library.versions(&settings);
    let all: Vec<String> = lines.values().flatten().map(|v| v.id.clone()).collect();
    let n = Lengths::new(&all, &[], &[]).recipe;
    let wanted: Option<HashSet<String>> = a
        .recipe
        .as_ref()
        .map(|r| RecipeId::candidates(r, &all).iter().map(|id| RecipeId::base(id).to_string()).collect());
    let mut sorted: Vec<_> = lines.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(b.0));
    for (_, list) in sorted {
        let Some(first) = list.first() else { continue };
        if !wanted.as_ref().is_none_or(|w| w.contains(RecipeId::base(&first.id))) {
            continue;
        }
        println!("{}  from {}", RecipeId::qualified(&first.id), library.source_name(&first.source, &settings));
        for v in list {
            println!("    {}{}", pad(&OmaflakeId::recipe(&v.id, n), 50), version_state(v));
        }
    }
    if wanted.is_some_and(|w| w.is_empty()) {
        return Err(usage(&["recipes", "versions"], format!("No recipe {}", a.recipe.as_deref().unwrap_or(""))));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use omaflake_core::NetworkMode;

    #[test]
    fn recipe_rows() {
        assert_eq!(
            recipes_header(),
            "RECIPE       AUTHOR    KIND   ARCH            CPU RAM   DISK  NETWORK  SHARE EPHEMERAL IMAGE"
        );
        let mut d = RecipeDefaults::new(
            GuestKind::Nixos,
            Some(vec!["aarch64-linux".into(), "x86_64-linux".into()]),
            4,
            8,
            40,
            NetworkMode::Shared,
            true,
            false,
            None,
        );
        d.author = Some("omaflake".into());
        assert_eq!(
            recipes_row(&recipe_label("dev", true, None), &d, "– (omaf build dev)"),
            format!(
                "dev ⚗        omaflake  nixos  {}4   8G    40G   shared   yes   no        – (omaf build dev)",
                pad(
                    &format!("{},{}", Platform::architecture("aarch64-linux"), Platform::architecture("x86_64-linux")),
                    16
                )
            )
        );
        assert_eq!(recipe_label("dev", false, Some(&"my-fork".to_string())), "dev (my-fork)");
    }

    #[test]
    fn available_lines() {
        assert_eq!(
            available_line("omaflake/nixos/minimal", "installed", "A small NixOS"),
            "  omaflake/nixos/minimal            installed         A small NixOS"
        );
    }

    #[test]
    fn version_states() {
        let mut v = omaflake_core::recipe_sources::Version {
            id: "omaflake.nixos.dev@apple".into(),
            source: "folder".into(),
            added: None,
            is_newest: true,
            is_live: true,
            images: vec!["a".into(), "b".into()],
        };
        assert_eq!(version_state(&v), "in the folder, newest, images: 2");
        v.is_live = false;
        v.is_newest = false;
        v.images.clear();
        assert_eq!(version_state(&v), "");
    }
}
