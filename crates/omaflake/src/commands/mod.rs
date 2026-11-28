// SPDX-License-Identifier: AGPL-3.0-or-later
//! The commands' bodies, one module per Swift file of Sources/omaf (Omaf.swift is split into recipes.rs and
//! instances.rs), and `serve` / `call` (docs/api.md).

pub mod credentials;
pub mod files;
pub mod instances;
pub mod locales;
pub mod looks;
pub mod macos;
pub mod modules;
pub mod networks;
pub mod packages;
pub mod profiles;
pub mod recipes;
pub mod service;
pub mod sources;
pub mod status;
pub mod ui;
pub mod volumes;

use omaflake_core::Error;

use crate::cli::*;
use crate::util::Outcome;
use omaflake_core::api;

pub fn run(command: Command) -> Outcome {
    match command {
        Command::Recipes(r) => match r.command {
            None => recipes::list(&r.list),
            Some(RecipesCommand::List(a)) => recipes::list(&a),
            Some(RecipesCommand::Versions(a)) => recipes::versions(&a),
            Some(RecipesCommand::Available(a)) => recipes::available(&a),
            Some(RecipesCommand::Install(a)) => recipes::install(&a),
            Some(RecipesCommand::Show(a)) => recipes::show(&a),
            Some(RecipesCommand::Remove(a)) => recipes::remove(&a),
        },
        Command::Sources(s) => match s.command {
            None | Some(SourcesCommand::List) => sources::list(),
            Some(SourcesCommand::Add(a)) => sources::add(&a),
            Some(SourcesCommand::Remove(a)) => sources::remove(&a.source),
            Some(SourcesCommand::Enable(a)) => sources::toggle(&a.source, true),
            Some(SourcesCommand::Disable(a)) => sources::toggle(&a.source, false),
            Some(SourcesCommand::Check) => sources::check(),
        },
        Command::Build(a) => recipes::build(&a),
        Command::Credentials(c) => match c.command {
            None | Some(CredentialsCommand::List) => credentials::list(),
            Some(CredentialsCommand::Add(a)) => credentials::add(&a),
            Some(CredentialsCommand::Edit(a)) => credentials::edit(&a),
            Some(CredentialsCommand::Remove(a)) => credentials::remove(&a.credential),
            Some(CredentialsCommand::Default(a)) => credentials::set_default(&a.credential),
            Some(CredentialsCommand::CopyPassword(a)) => credentials::copy_password(&a.credential),
        },
        Command::Locales(l) => match l.command {
            None | Some(LocalesCommand::List) => locales::list(),
            Some(LocalesCommand::Add(a)) => locales::add(&a),
            Some(LocalesCommand::Edit(a)) => locales::edit(&a),
            Some(LocalesCommand::Remove(a)) => locales::remove(&a.locale),
            Some(LocalesCommand::Default(a)) => locales::set_default(&a.locale),
        },
        Command::Looks(l) => match l.command {
            None | Some(LooksCommand::List) => looks::list(),
            Some(LooksCommand::Available) => looks::available(),
            Some(LooksCommand::Install(a)) => looks::install(&a.look),
            Some(LooksCommand::Remove(a)) => looks::remove(&a.look),
            Some(LooksCommand::Default(a)) => looks::set_default(&a.look),
        },
        Command::Modules(m) => match m.command {
            None | Some(ModulesCommand::List) => modules::list(),
            Some(ModulesCommand::Available) => modules::available(),
            Some(ModulesCommand::Install(a)) => modules::install(&a.module),
            Some(ModulesCommand::Remove(a)) => modules::remove(&a.module),
        },
        Command::Profiles(p) => match p.command {
            None | Some(ProfilesCommand::List) => profiles::list(),
            Some(ProfilesCommand::Add(a)) => profiles::add(&a),
            Some(ProfilesCommand::Edit(a)) => profiles::edit(&a),
            Some(ProfilesCommand::Remove(a)) => profiles::remove(&a.profile),
            Some(ProfilesCommand::Default(a)) => profiles::set_default(&a.profile),
        },
        Command::Networks(n) => match n.command {
            None | Some(NetworksCommand::List) => networks::list(),
            Some(NetworksCommand::Create(a)) => networks::create(&a.network),
            Some(NetworksCommand::Remove(a)) => networks::remove(&a.network),
            Some(NetworksCommand::Show(a)) => networks::show(&a.network),
            Some(NetworksCommand::Join(a)) => networks::join(&a.network, &a.vm),
            Some(NetworksCommand::Leave(a)) => networks::leave(&a.network, &a.vm),
        },
        Command::NetworkSwitch(a) => networks::switch(&a),
        Command::Files(f) => match f.command {
            None | Some(FilesCommand::List) => files::list(),
            Some(FilesCommand::Add(a)) => files::add(&a.module, a.path.as_deref()),
        },
        Command::Volumes(v) => match v.command {
            None | Some(VolumesCommand::List) => volumes::list(),
            Some(VolumesCommand::Create(a)) => volumes::create(&a.volume, a.path.as_deref()),
            Some(VolumesCommand::Remove(a)) => volumes::remove(&a.volume),
            Some(VolumesCommand::Attach(a)) => volumes::attach(&a.volume, &a.vm, a.mount.as_deref()),
            Some(VolumesCommand::Detach(a)) => volumes::detach(&a.volume, &a.vm),
            Some(VolumesCommand::Rename(a)) => volumes::rename(&a.volume, &a.vm, a.mount.as_deref()),
        },
        Command::MacOs { command: MacOsCommand::Install(a) } => macos::install(&a),
        Command::MacOs { command: MacOsCommand::Assign(a) } => macos::assign(&a),
        Command::MacOs { command: MacOsCommand::Bases } => macos::bases(),
        Command::MacOs { command: MacOsCommand::Remove(a) } => macos::remove(&a),
        Command::MacOs { command: MacOsCommand::Unchain(a) } => macos::unchain(&a),
        Command::Images(i) => match i.command {
            None | Some(ImagesCommand::List) => instances::images_list(),
            Some(ImagesCommand::Remove { name }) => instances::images_remove(&name),
            Some(ImagesCommand::Unchain { name, no_tools }) => instances::images_unchain(&name, no_tools),
        },
        Command::New(a) => instances::new(&a),
        Command::Set(a) => instances::set(&a),
        Command::Start(a) => instances::start(&a),
        Command::Stop(a) => instances::stop(&a),
        Command::Reset(a) => instances::reset(&a.name, a.yes),
        Command::Upgrade(a) => instances::upgrade(&a.name, a.keep),
        Command::Remove(a) => instances::remove(&a.name, a.yes),
        Command::List(a) => instances::list(&a),
        Command::Export { command } => match command {
            ExportCommand::Vm { name, folder, with_image } => packages::export_vm(&name, &folder, with_image),
            ExportCommand::Image { name, folder } => packages::export_image(&name, &folder),
            ExportCommand::Recipe { recipe, folder } => packages::export_recipe(&recipe, &folder),
        },
        Command::Import(a) => packages::import(&a.path, a.name.as_deref(), a.replace),
        Command::External(e) => match e.command {
            None | Some(ExternalCommand::List) => packages::external_list(),
            Some(ExternalCommand::Add { path }) => packages::external_add(&path),
            Some(ExternalCommand::Remove { name }) => packages::external_remove(&name),
        },
        Command::Ssh(a) => instances::ssh(&a),
        Command::Wait(a) => instances::wait(&a),
        Command::Agent(a) => instances::agent(&a),
        Command::Diagnose(a) => diagnose(&a),
        Command::Log(a) => instances::log(&a),
        Command::Config(a) => instances::config(&a),
        Command::Doctor(a) => instances::doctor(&a),
        Command::Builder { command } => match command {
            BuilderCommand::Setup => instances::builder_setup(),
            BuilderCommand::Apply { force } => instances::builder_apply(force),
            BuilderCommand::CleanUp => instances::builder_clean_up(),
            BuilderCommand::Check => instances::builder_check(),
            BuilderCommand::Repair { force } => instances::builder_repair(force),
        },
        Command::Serve(a) => service::serve(&a),
        Command::Service { command } => service::run(&command),
        Command::Call(a) => call(&a),
        Command::Ui(a) => ui::run(&a),
        Command::Status(a) => status::run(&a),
        Command::Bridge(a) => bridge(&a),
    }
}

/// `omaf diagnose <name>` (plan B162): the diagnosis as a job of the core service – each check's line as it is done,
/// then where the log is; Ctrl-C cancels it; exit 1 when a check failed
fn diagnose(a: &Diagnose) -> Outcome {
    let mut checks = |lines: &[String]| {
        // The checks' lines (`✓ Clipboard: both ways`), not their details
        for line in lines.iter().filter(|l| ["✓ ", "✗ ", "? ", "– "].iter().any(|m| l.starts_with(m))) {
            println!("{line}");
        }
    };
    let job = service::rpc_job("diagnostics.run", serde_json::json!({"name": a.name}), &mut checks)?;
    if job.state != omaflake_sdk::types::JobState::Finished {
        return Err(Error::new(job.error.unwrap_or_else(|| "the diagnosis did not finish".into())).into());
    }
    let result = job.result.unwrap_or_default();
    if let Some(file) = result["file"].as_str() {
        println!("Log: {file}");
    }
    let failed = result["checks"].as_array().is_some_and(|c| c.iter().any(|x| x["status"] == "fail"));
    if failed { Err(crate::util::Failure::Exit(1)) } else { Ok(()) }
}

/// `omaf bridge` (hidden; plan B126): runs until the VM closes its agent port
fn bridge(a: &Bridge) -> Outcome {
    let options = omaflake_core::host_bridge::BridgeOptions {
        agent: a.agent.clone(),
        agent_state: a.agent_state.clone(),
        clipboard: a.clipboard,
        camera: a.camera,
        camera_test_pattern: a.camera_test_pattern,
        midi: a.midi,
        vm_name: a.vm_name.clone().unwrap_or_default(),
        control: a.control.clone(),
        forwards: a
            .forwards
            .iter()
            .map(|f| {
                omaflake_core::agent_protocol::AgentForward::parse(f)
                    .ok_or_else(|| Error::new(format!("--forward {f}: <port>=<address>:<port>")))
            })
            .collect::<Result<_, _>>()?,
    };
    match omaflake_core::host_bridge::run(&options) {
        Ok(end) => {
            eprintln!(
                "bridge: {}",
                if end == omaflake_core::agent_protocol::LinkEnd::Closed {
                    "the VM closed the port"
                } else {
                    "stopped listening to the guest"
                }
            );
            Ok(())
        }
        Err(e) => Err(Error::new(e).into()),
    }
}

/// `omaf call`: the params as JSON (none → null), the result pretty-printed. With `--socket` through the service: a
/// job is followed until it ended (Ctrl-C cancels it) and printed then, as without.
fn call(a: &Call) -> Outcome {
    let params = parse_params(a.params.as_deref())?;
    let Some(socket) = &a.socket else {
        let result = api::call_once(&a.method, params)?;
        println!("{}", pretty(&result));
        return Ok(());
    };
    let client = match socket {
        Some(path) => {
            service::connect_at(path).ok_or_else(|| Error::new(format!("No service answers on {}", path.display())))?
        }
        None => service::ensure_running(&omaflake_core::Paths::standard())?.client,
    };
    let rpc = |method: &str, params: serde_json::Value| client.call(method, params).map_err(Error::from);
    rpc("events.subscribe", serde_json::json!({"topics": ["jobs"]}))?;
    let result = rpc(&a.method, params)?;
    let mut quiet = |_: &[String]| {};
    // A method that starts work answers with its job(s) – waited for, as without --socket; `jobs.*` only report them
    let starts_jobs = !a.method.starts_with("jobs.");
    let result = if starts_jobs && service::is_job(&result) {
        serde_json::to_value(service::follow(&client, &result, &mut quiet)?).unwrap_or_default()
    } else if starts_jobs && result.as_array().is_some_and(|a| !a.is_empty() && a.iter().all(service::is_job)) {
        let mut ended = Vec::new();
        for job in result.as_array().into_iter().flatten() {
            ended.push(serde_json::to_value(service::follow(&client, job, &mut quiet)?).unwrap_or_default());
        }
        serde_json::Value::Array(ended)
    } else {
        result
    };
    println!("{}", pretty(&result));
    Ok(())
}

pub fn parse_params(text: Option<&str>) -> omaflake_core::Result<serde_json::Value> {
    match text {
        None => Ok(serde_json::Value::Null),
        Some(t) => serde_json::from_str(t).map_err(|e| Error::new(format!("The params are not valid JSON: {e}"))),
    }
}

pub fn pretty(v: &serde_json::Value) -> String {
    serde_json::to_string_pretty(v).unwrap_or_else(|_| v.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn call_params_and_result() {
        assert_eq!(parse_params(None).unwrap(), serde_json::Value::Null);
        assert_eq!(parse_params(Some(r#"{"name":"dev"}"#)).unwrap(), json!({"name": "dev"}));
        assert!(parse_params(Some("{name")).unwrap_err().message().starts_with("The params are not valid JSON: "));
        assert_eq!(pretty(&json!({"a": [1]})), "{\n  \"a\": [\n    1\n  ]\n}");
    }
}
