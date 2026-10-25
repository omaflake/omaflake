// SPDX-License-Identifier: AGPL-3.0-or-later
//! The commands' bodies, one module per Swift file of Sources/omaf (Omaf.swift is split into recipes.rs and
//! instances.rs), and `serve` / `call` (docs/api.md).

pub mod credentials;
pub mod instances;
pub mod looks;
pub mod macos;
pub mod packages;
pub mod recipes;
pub mod service;
pub mod sources;
pub mod status;
pub mod ui;

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
        Command::Looks(l) => match l.command {
            None | Some(LooksCommand::List) => looks::list(),
            Some(LooksCommand::Available) => looks::available(),
            Some(LooksCommand::Install(a)) => looks::install(&a.look),
            Some(LooksCommand::Remove(a)) => looks::remove(&a.look),
            Some(LooksCommand::Default(a)) => looks::set_default(&a.look),
        },
        Command::MacOs { command: MacOsCommand::Install(a) } => macos::install(&a),
        Command::MacOs { command: MacOsCommand::Assign(a) } => macos::assign(&a),
        Command::Images(i) => match i.command {
            None | Some(ImagesCommand::List) => instances::images_list(),
            Some(ImagesCommand::Remove { name }) => instances::images_remove(&name),
        },
        Command::New(a) => instances::new(&a),
        Command::Set(a) => instances::set(&a),
        Command::Start(a) => instances::start(&a),
        Command::Stop(a) => instances::stop(&a),
        Command::Reset(a) => instances::reset(&a.name),
        Command::Upgrade(a) => instances::upgrade(&a.name),
        Command::Remove(a) => instances::remove(&a.name),
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
        Command::Log(a) => instances::log(&a),
        Command::Config(a) => instances::config(&a),
        Command::Doctor(a) => instances::doctor(&a),
        Command::Builder { command } => match command {
            BuilderCommand::Setup => instances::builder_setup(),
            BuilderCommand::Apply { force } => instances::builder_apply(force),
            BuilderCommand::CleanUp => instances::builder_clean_up(),
        },
        Command::Serve(a) => service::serve(&a),
        Command::Service { command } => service::run(&command),
        Command::Call(a) => call(&a),
        Command::Ui(a) => ui::run(&a),
        Command::Status(a) => status::run(&a),
        Command::Bridge(a) => bridge(&a),
    }
}

/// `omaf bridge` (hidden; plan B126): runs until the VM closes its agent port
fn bridge(a: &Bridge) -> Outcome {
    let options = omaflake_core::host_bridge::BridgeOptions {
        agent: a.agent.clone(),
        agent_state: a.agent_state.clone(),
        clipboard: a.clipboard,
        camera: a.camera,
        camera_test_pattern: a.camera_test_pattern,
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
            service::connect(path).ok_or_else(|| Error::new(format!("No service answers on {}", path.display())))?.0
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
