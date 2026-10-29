// SPDX-License-Identifier: AGPL-3.0-or-later
//! Recipes and builds – port of `RecipesList` and `Build` (Sources/omaf/Omaf.swift) and of `RecipesAvailable`,
//! `RecipesInstall`, `RecipesRemove`, `RecipesVersions` (Sources/omaf/Sources.swift).

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

use omaflake_core::mac_image_builder::BuildRequest;
use omaflake_core::{
    BuildLog, Cancellation, Error, GuestKind, Lengths, MacImageBuilder, OmaflakeId, Platform, RecipeDefaults, RecipeId,
    RecipeLibrary, RecipeRef, RecipeSourceKind, SourceClient,
};

use omaflake_core::api::client::Client;
use omaflake_core::api::service::socket_path;
use omaflake_core::api::types::{BuildResult, JobState};
use omaflake_core::error::codes;
use serde_json::{Value, json};

use super::service;
use crate::cli;
use crate::tty::Interrupts;
use crate::util::{Env, Outcome, choose_recipe, folder_path, local_date, pad, shortest_argument, usage};

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
        images.recipe_defaults().into_iter().filter(|(_, d)| show_lab || !d.is_lab()).collect();
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
    let places = library.places(&settings);
    let lengths = Lengths::current(&env.paths);
    let origins = if a.long { origins(&env) } else { HashMap::new() };
    println!("{}", recipes_header());
    let mut sorted: Vec<(&String, &RecipeDefaults)> = defaults.iter().collect();
    sorted.sort_by(|(a, x), (b, y)| (x.kind.as_str(), a).cmp(&(y.kind.as_str(), b)));
    for (id, d) in sorted {
        let name = RecipeId::display_name(id, Some(d));
        let arg = shortest_argument(id, &defaults, &places);
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
    choice: &omaflake_core::guest_settings::BuildChoice,
    base: Option<String>,
    a: &cli::Build,
) -> Outcome {
    let rpc = |method: &str, params: Value| client.call(method, params).map_err(Error::from);
    rpc("events.subscribe", json!({"topics": ["jobs"]}))?;
    let mut params = json!({
        "recipe": recipe,
        "keepFailed": a.keep_failed,
        "credential": choice.credential.as_ref().map_or("", |c| c.id.as_str()),
        "look": choice.look.as_deref().unwrap_or(""),
        "locale": choice.locale.as_ref().map_or("", |l| l.id.as_str()),
    });
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
    if a.verbose
        && ended.hidden_lines > 0
        && let Some(file) = &ended.log_file
    {
        eprintln!("({} routine lines are only in {file})", ended.hidden_lines);
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
    let defaults = images.recipe_defaults();
    let chooser = |ids: &[String]| choose_recipe(&env, ids);
    let places = env.recipe_places();
    let recipe = RecipeId::resolve_newest_in(&a.recipe, &defaults, &places, &|_| true, Some(&chooser))?;
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
    // The credential and look of the image (plan B137); without --look the recipe's when it is installed (B142)
    let suggested = if a.look.is_none() {
        let settings = env.settings();
        RecipeLibrary::new(env.paths.clone())
            .available(&settings)
            .into_iter()
            .find(|x| x.id == recipe)
            .and_then(|x| omaflake_core::composition::RecipeSpec::read(&x.folder).ok())
            .and_then(|spec| omaflake_core::looks::LookStore::new(env.paths.clone()).suggested(&settings, &spec))
            .map(|l| l.id)
    } else {
        None
    };
    let look = a.look.clone().or(suggested);
    let choice =
        super::credentials::build_choice(&env.paths, a.credential.as_deref(), look.as_deref(), a.locale.as_deref())?;
    // macOS: the base and its credential's password – nothing is asked (plan B139)
    let mut choice = choice;
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
        let login = omaflake_core::credentials::Credentials::new(env.paths.clone()).login_of_base(&base)?;
        let user = d.user.clone().unwrap_or_else(|| login.user.clone());
        choice.credential = Some(login.credential.clone());
        mac = Some((builder, base, user, login.password));
    }
    // With the core service running, the build is its job: the app and other clients see it, it waits in the same
    // queue (plan B120); this command follows its log
    if let Some((client, _)) = service::connect(&socket_path(&env.paths)) {
        // The service takes the base's credential itself
        let base = mac.as_ref().map(|(_, b, _, _)| b.to_string_lossy().into_owned());
        return build_in_service(&client, &env, &recipe, &choice, base, a);
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
                choice: choice.clone(),
            },
            &token,
            &log,
        ),
        None => images.build(&recipe, &flake, &choice, None, &token, &log),
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
        let library = RecipeLibrary::new(env.paths.clone());
        let written = library.install(&id, &catalog, &client)?;
        if written.is_empty() {
            println!("{} is up to date.", a.recipe);
        } else {
            let names: Vec<String> = written.iter().map(|w| RecipeId::qualified(w)).collect();
            println!("Installed from {}: {}", s.display_name(), names.join(", "));
        }
        // What it is made of comes along: modules, key bindings, its look (plan B142)
        let folder = written.first().and_then(|w| library.folder(w)).or_else(|| {
            library.available(&env.settings()).into_iter().find(|x| RecipeId::base(&x.id) == id).map(|x| x.folder)
        });
        if let Some(spec) = folder.and_then(|f| omaflake_core::composition::RecipeSpec::read(&f).ok()) {
            let catalogs = omaflake_core::parts::fetch_part_catalogs(&env.settings());
            omaflake_core::parts::install_recipe_parts(&env.paths, &spec, &catalogs, &mut |line| println!("{line}"))?;
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

/// `omaf recipes show`: what a recipe is made of (plan B142) – its modules (here or missing), look, key bindings, every
/// key, and what is wrong; `--install` installs what is missing
pub fn show(a: &cli::RecipesShow) -> Outcome {
    use omaflake_core::composition::{Composition, Issue, RecipeSpec};
    let env = Env::new();
    let settings = env.settings();
    let library = RecipeLibrary::new(env.paths.clone());
    let available = library.available(&settings);
    let ids: Vec<String> = available.iter().map(|x| x.id.clone()).collect();
    let id = RecipeId::resolve(&a.recipe, &ids, None)?;
    let Some(entry) = available.iter().find(|x| x.id == id) else {
        return Err(usage(&["recipes", "show"], format!("No recipe {} – see omaf recipes", a.recipe)));
    };
    let spec = RecipeSpec::read(&entry.folder)?;
    if a.install {
        let catalogs = omaflake_core::parts::fetch_part_catalogs(&settings);
        omaflake_core::parts::install_recipe_parts(&env.paths, &spec, &catalogs, &mut |line| println!("{line}"))?;
    }
    let modules = omaflake_core::modules::ModuleStore::new(env.paths.clone());
    let c = Composition::of(
        &spec,
        &modules.list(&settings),
        &omaflake_core::keymaps::KeymapStore::new(env.paths.clone()).list(&settings),
        &modules.capabilities(&settings),
    );
    println!("{} ({})", RecipeId::qualified(&id), spec.platform.as_str());
    println!("modules:");
    for r in &spec.modules {
        match c.modules.iter().find(|m| m.reference() == r.id) {
            Some(m) => println!("  {}{} {}", pad(&r.label(), 30), m.title, m.version),
            None => println!("  {}– not here", pad(&r.label(), 30)),
        }
    }
    if let Some(look) = &spec.look {
        println!("look:          {}", look.label());
    }
    if !spec.keymaps.is_empty() {
        println!("key bindings:  {}", spec.keymaps.iter().map(|k| k.label()).collect::<Vec<_>>().join(", "));
    }
    let r = spec.requirements;
    if r.cpus > 0 {
        println!("needs:         {} CPU · {} GB RAM · {} GB disk", r.cpus, r.memory_gb, r.disk_gb);
    }
    if !c.bindings.is_empty() {
        println!("keys:");
        for b in &c.bindings {
            let mode = match (b.modes.as_slice(), &b.after) {
                ([], None) => String::new(),
                ([], Some(after)) => format!(" (then {after})"),
                (modes, None) => format!(" (in {})", modes.join(", ")),
                (modes, Some(after)) => format!(" (in {}, then {after})", modes.join(", ")),
            };
            println!("  {}{}{}{mode}", pad(&b.key_text(), 22), pad(&b.module, 22), b.title);
        }
    }
    for i in &c.issues {
        let text = match i {
            Issue::MissingModule { reference, have } | Issue::MissingKeymap { reference, have } => match have {
                Some(v) => format!("{reference} is here in {v}, which the recipe does not allow"),
                None => format!("{reference} is not here – omaf recipes show {} --install", a.recipe),
            },
            Issue::Needs { module, capability } => format!("{module} needs a {capability} – no module provides one"),
            Issue::Incompatible { capability, modules } => {
                format!("{} do not go together: each is the {capability}", modules.join(" + "))
            }
            Issue::KeysOverlap { first, second } => format!(
                "keys overlap: {} {} ({}) and {} {} ({})",
                first.module,
                first.action,
                first.key_text(),
                second.module,
                second.action,
                second.key_text()
            ),
            Issue::KeyRule { binding } => format!(
                "{} {} ({}): without a mode a key starts with Super or Ctrl",
                binding.module,
                binding.action,
                binding.key_text()
            ),
        };
        println!("{} {text}", if i.blocks() { "✗" } else { "⚠" });
    }
    if !c.unfree.is_empty() {
        println!("not free software: {}", c.unfree.join(", "));
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
