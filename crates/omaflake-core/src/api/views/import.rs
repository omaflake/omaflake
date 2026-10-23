// SPDX-License-Identifier: AGPL-3.0-or-later
//! The import step (`import`, plans B108/B109): what a chosen package is and the one action that brings it in – with
//! the question when something of it is here already (a VM under another name, the same image) or it lies on a
//! network volume (a VM run from where it is). A UI shows no page for it: it runs `action` (asking `confirm`) or shows
//! `reason` (the app's `importPackage` and `chooseExternal`).

use serde::{Deserialize, Serialize};
use serde_json::json;

use super::common::*;
use super::text::{tf, Text};
use crate::api::types::{Inspection, PackageKindDto};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ImportPage {
    pub path: String,
    /// The package's file name
    pub name: String,
    pub kind: PackageKindDto,
    pub valid: bool,
    /// Why it cannot be imported ("<name>: <reason>")
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason: Option<Text>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub action: Option<Action>,
    pub inspection: Inspection,
}

#[derive(Debug, Deserialize)]
pub struct ImportParams {
    pub path: String,
    #[serde(default)]
    pub expect: Option<PackageKindDto>,
    /// Run a VM from where it is instead of copying it
    #[serde(default)]
    pub external: bool,
}

pub fn page(p: &ImportParams, inspection: Inspection) -> ImportPage {
    let name = p.path.trim_end_matches('/').rsplit('/').next().unwrap_or(&p.path).to_string();
    let reason = (!inspection.valid).then(|| {
        let why = inspection.reason.clone().map(Text::verbatim).unwrap_or_else(|| Text::new("Not an omaflake package"));
        tf!("%@: %@", &name, why)
    });
    let action = inspection.valid.then(|| {
        if p.external {
            let a = Action::call(
                "useExternal",
                Text::new("Run from There"),
                Some("external"),
                "vms.useExternal",
                json!({"path": p.path, "confirmNetwork": true}),
            );
            if inspection.network_volume {
                a.confirm(
                    Confirm::new(
                        Text::new("Run the VM over the network?"),
                        tf!(
                            "%@ is on a network volume. It runs from there, but slower – and it breaks when the connection does. Importing it (a copy into the data folder) avoids that.",
                            &name
                        ),
                    )
                    .button("Run from There"),
                )
            } else {
                a
            }
        } else {
            let a = Action::call("import", Text::new("Import"), Some("import"), "packages.import", json!({"path": p.path}));
            match &inspection.conflict {
                Some(c) if c.vm_name.is_some() => {
                    let existing = c.vm_name.clone().unwrap_or_default();
                    let suggested = c.suggested_name.clone().unwrap_or_else(|| format!("{existing}-2"));
                    a.confirm(
                        Confirm::new(
                            tf!("“%@” is here already", &existing),
                            Text::new("You probably have this VM already. Under another name it becomes a second VM with its own identity (SSH port, macOS machine identity); its user, keys and data are the same."),
                        )
                        .button("Import under This Name")
                        .input("name", suggested),
                    )
                }
                Some(c) if c.image.is_some() => {
                    let mut a = a.confirm(
                        Confirm::new(
                            Text::new("The image is here already"),
                            tf!(
                                "%@ is here already – the same name means the same content. Replacing it copies it again.",
                                c.image.clone().unwrap_or_default()
                            ),
                        )
                        .button("Replace"),
                    );
                    a.params["replace"] = json!(true);
                    a
                }
                _ => a,
            }
        }
    });
    ImportPage {
        path: p.path.clone(),
        name,
        kind: inspection.kind,
        valid: inspection.valid,
        reason,
        action,
        inspection,
    }
}
