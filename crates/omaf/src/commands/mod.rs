// SPDX-License-Identifier: AGPL-3.0-or-later
//! The commands' bodies, one module per Swift file of Sources/omaf (Omaf.swift is split into recipes.rs and
//! instances.rs), and `serve` / `call` (docs/api.md).

pub mod instances;
pub mod macos;
pub mod packages;
pub mod recipes;
pub mod sources;

use omaflake_core::Error;

use crate::api_shim as api;
use crate::cli::*;
use crate::util::Outcome;

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
        Command::MacOs { command: MacOsCommand::Install(a) } => macos::install(&a),
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
        Command::Doctor => instances::doctor(),
        Command::Builder { command } => match command {
            BuilderCommand::Setup => instances::builder_setup(),
            BuilderCommand::Apply { force } => instances::builder_apply(force),
            BuilderCommand::CleanUp => instances::builder_clean_up(),
        },
        Command::Serve => Ok(api::serve_stdio()?),
        Command::Call(a) => call(&a),
    }
}

/// `omaf call`: the params as JSON (none → null), the result pretty-printed
fn call(a: &Call) -> Outcome {
    let params = parse_params(a.params.as_deref())?;
    let result = api::call_once(&a.method, params)?;
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
