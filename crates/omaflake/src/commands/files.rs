// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf files` (plan B198): files modules need to be built, which nobody may pass on (Xcode) – which are in the Nix
//! Store, found on this computer or missing, and adding one (`files.add` in the core service: unpacked, added to the
//! Nix Store, checked against the module's hash).

use serde_json::json;

use omaflake_core::modules::{Module, ModuleStore};
use omaflake_core::paths::Settings;
use omaflake_sdk::codes;

use super::service;
use omaflake_core::Error;

use crate::util::{Env, Outcome, pad};

fn modules(env: &Env) -> Vec<Module> {
    ModuleStore::new(env.paths.clone()).list(&Settings::load(&env.paths))
}

pub fn list() -> Outcome {
    let env = Env::new();
    let mut any = false;
    for m in modules(&env) {
        for f in &m.files {
            any = true;
            let state = if f.present() {
                "in the Nix Store".to_string()
            } else if let Some(p) = f.find() {
                format!("found: {} (added at the next build)", p.display())
            } else {
                format!("missing – omaf files add {} <file>", m.reference())
            };
            println!("{}{}  {}", pad(&m.reference(), 22), pad(f.title(), 16), state);
            if !f.present()
                && f.find().is_none()
                && let Some(url) = &f.url
            {
                println!("{}download: {url}", pad("", 40));
            }
        }
    }
    if !any {
        println!("(no module here needs a file)");
    }
    Ok(())
}

/// The module `name` means: its reference (omaflake/xcode), id (omaflake.macos.xcode), or a recipe's way of naming
/// one of its name (macos/xcode, omaflake/macos/xcode)
fn module_for<'a>(all: &'a [Module], name: &str) -> Option<&'a Module> {
    all.iter().filter(|m| !m.files.is_empty()).find(|m| {
        m.reference() == name
            || m.id == name
            || format!("{}/{}", m.platform.as_str(), m.name) == name
            || format!("{}/{}/{}", m.author, m.platform.as_str(), m.name) == name
    })
}

pub fn add(name: &str, path: Option<&str>) -> Outcome {
    let env = Env::new();
    let all = modules(&env);
    let m = module_for(&all, name).ok_or_else(|| {
        Error::with_code(codes::NOT_FOUND, format!("No module {name} here needs a file – omaf files"))
    })?;
    let path = path
        .map(|p| std::fs::canonicalize(p).map(|c| c.to_string_lossy().into_owned()).unwrap_or_else(|_| p.to_string()));
    let mut params = json!({"module": m.id});
    if let Some(p) = path {
        params["path"] = json!(p);
    }
    let job = service::rpc_job("files.add", params, &mut service::to_stderr)?;
    service::finished(job, "Adding the file").map_err(|e| match e.code() {
        Some(codes::CANCELLED) => Error::with_code(codes::CANCELLED, "Cancelled"),
        _ => e,
    })?;
    Ok(())
}
