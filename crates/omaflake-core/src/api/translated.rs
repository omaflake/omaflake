// SPDX-License-Identifier: AGPL-3.0-or-later
//! Every text a user reads has German (plan B125): the keys of the views (docs/api.md §10.2) – from the golden files
//! of both hosts and from every page built for a Mac and for a Linux host, the Doctor with every check in every state –
//! and the keys of the errors (api.md §3) – `vm_err!`'s format strings, read out of the sources. The German is
//! `App/Resources/de.lproj/Localizable.strings` (through `omaflake-i18n`'s generated table).
//!
//! Keys without a letter (`%@ · %@`, `%@: %@`) are joints of other texts and need no translation.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use omaflake_i18n::Language;
use serde_json::{Value, json};

#[path = "../../../omaflake-macros/src/key.rs"]
#[allow(dead_code)]
mod key;

fn needs_german(key: &str) -> bool {
    let without = key.replace("%@", "").replace("%lld", "");
    without.chars().any(char::is_alphabetic)
}

fn missing(keys: &BTreeMap<String, String>) -> Vec<String> {
    keys.iter()
        .filter(|(k, _)| needs_german(k) && Language::German.lookup(k).is_none())
        .map(|(k, from)| format!("{k:?} ({from})"))
        .collect()
}

/// The keys of every text (`{key, text, args}`) in `v`, with where they were found
fn collect(v: &Value, from: &str, keys: &mut BTreeMap<String, String>) {
    match v {
        Value::Object(o) => {
            if let (Some(Value::String(k)), Some(Value::String(_))) = (o.get("key"), o.get("text")) {
                keys.entry(k.clone()).or_insert_with(|| from.to_string());
            }
            o.values().for_each(|x| collect(x, from, keys));
        }
        Value::Array(a) => a.iter().for_each(|x| collect(x, from, keys)),
        _ => {}
    }
}

fn manifest() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
}

/// The pages of `c` with the params the tests use
fn pages(c: &crate::api::views::Ctx, from: &str, keys: &mut BTreeMap<String, String>) {
    use crate::api::views::{PageRef, build};
    let pages = [
        ("home", json!({})),
        ("vms", json!({})),
        ("vms", json!({"filter": {"status": false, "kind": "nixos"}})),
        ("vm", json!({"name": "research"})),
        ("vm", json!({"name": "work"})),
        ("images", json!({})),
        ("image", json!({"name": "omaflake.macos.dev@stone-lamp"})),
        ("image", json!({"name": "omaflake.nixos.dev@cedar-mango-lift"})),
        ("recipes", json!({})),
        ("recipes", json!({"filter": {"otherSystems": true}})),
        ("newVm", json!({"image": "omaflake.nixos.dev@cedar-mango-lift"})),
        ("newVm", json!({"image": "omaflake.nixos.dev@cedar-mango-lift", "values": {"name": "bad name!"}})),
        ("newVm", json!({"image": "omaflake.macos.dev@stone-lamp", "values": {"name": "Work"}})),
        ("build", json!({"recipe": "omaflake.nixos.dev@velvet-tiger-orbit"})),
        ("build", json!({"recipe": "lab.macos.base@ocean-maple"})),
        ("doctor", json!({})),
        ("macBase", json!({})),
        ("settings", json!({})),
        ("settings", json!({"values": {"input": "acme/recipes", "private": true}})),
        ("activity", json!({})),
    ];
    for (page, params) in pages {
        if let Ok(model) = build(c, &PageRef { page: page.into(), params }) {
            collect(&model, &format!("{page} on {from}"), keys);
        }
    }
}

/// A Doctor with every check in every state (and the details the hints look at)
fn every_check(host: crate::api::views::HostOs) -> Vec<crate::api::types::PrerequisiteDto> {
    use crate::prerequisites::{PrerequisiteItem as I, PrerequisiteStatus as S};
    let items = [
        I::Host,
        I::Kvm,
        I::Nix,
        I::Flakes,
        I::NixDarwin,
        I::LinuxBuilder,
        I::Qemu,
        I::Flake,
        I::SshKey,
        I::Credential,
        I::Look,
        I::Git,
        I::Cli,
        I::BuilderRunning,
        I::FlakeEval,
        I::NixSpace,
        I::Ipsw,
        I::MacBase,
        I::MacSpace,
    ];
    let mut out = Vec::new();
    for item in items {
        for status in [S::Ok, S::Limited, S::Missing] {
            for (detail, install) in
                [("x", None), ("x", Some("sh -c install")), ("no /dev/kvm", None), ("UEFI firmware not found: x", None)]
            {
                let mut p = super::contract::prerequisite(item, status);
                p.title = super::model::requirement_title_on(item, host).into();
                p.detail = detail.into();
                p.install = install.map(str::to_string);
                p.free_bytes = Some(1);
                p.needed_bytes = Some(2);
                out.push(p);
            }
        }
    }
    out
}

#[test]
fn every_view_text_has_german() {
    use crate::api::views::{HostOs, tests::ctx};
    let mut keys = BTreeMap::new();
    let dir = manifest().join("testdata/api/views");
    for entry in std::fs::read_dir(&dir).unwrap().flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        let v: Value = serde_json::from_str(&std::fs::read_to_string(entry.path()).unwrap()).unwrap();
        collect(&v, &name, &mut keys);
    }
    let golden = keys.len();
    for host in [HostOs::Macos, HostOs::Linux] {
        let from = format!("{host:?}");
        let mut c = ctx();
        c.host = host;
        pages(&c, &from, &mut keys);
        // Each check in each state, on its own page (a Doctor only shows some of them in each section)
        for p in every_check(host) {
            let mut c = ctx();
            c.host = host;
            let report = c.doctor.as_mut().unwrap();
            report.prerequisites = vec![p.clone()];
            report.nixos_preflight = vec![p.clone()];
            report.macos_preflight = vec![p.clone()];
            report.build_blockers.nixos = vec![p.clone()];
            report.start_blockers.nixos = vec![p];
            pages(&c, &format!("{from} doctor"), &mut keys);
        }
        // Accelerators of Linux hosts, a VM without its image
        for accelerator in ["kvm", "tcg"] {
            let mut c = ctx();
            c.host = host;
            c.vms[0].accelerator = Some(accelerator.into());
            pages(&c, &format!("{from} {accelerator}"), &mut keys);
        }
        let mut c = ctx();
        c.host = host;
        c.images.clear();
        c.fetching = true;
        c.doctor = None;
        pages(&c, &format!("{from} empty"), &mut keys);
    }
    assert!(golden > 250 && keys.len() > golden, "{golden} keys in the golden files, {} in all", keys.len());
    let missing = missing(&keys);
    let joints = keys.keys().filter(|k| !needs_german(k)).count();
    eprintln!(
        "view texts: {} keys ({golden} in the golden files, {joints} joints), {} without German",
        keys.len(),
        missing.len()
    );
    assert!(
        missing.is_empty(),
        "{} view texts without German – add them to App/Resources/de.lproj/Localizable.strings, then run python3 \
         scripts/translations.py:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

/// The format strings of every `vm_err!` in `src` (tests left out): (key, file:line)
pub(crate) fn error_keys(src: &Path) -> BTreeMap<String, String> {
    let mut keys = BTreeMap::new();
    let mut files = vec![src.to_path_buf()];
    while let Some(path) = files.pop() {
        if path.is_dir() {
            files.extend(std::fs::read_dir(&path).unwrap().flatten().map(|e| e.path()));
            continue;
        }
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".rs") || name == "tests.rs" || name == "translated.rs" || name == "contract.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let text = text.split("\n#[cfg(test)]\nmod tests").next().unwrap();
        let mut rest = text;
        while let Some(i) = rest.find("vm_err!(") {
            let line = text[..text.len() - rest.len() + i].lines().count() + 1;
            rest = &rest[i + "vm_err!(".len()..];
            // The format string: the first string literal (after a code, if one comes first)
            let Some(q) = rest.find('"') else { break };
            let raw = rest[..q].ends_with('r');
            let start = if raw { q - 1 } else { q };
            let literal = if raw {
                let end = rest[q + 1..].find('"').unwrap() + q + 2;
                &rest[start..end]
            } else {
                let bytes = rest.as_bytes();
                let mut j = q + 1;
                while bytes[j] != b'"' {
                    j += if bytes[j] == b'\\' { 2 } else { 1 };
                }
                &rest[start..=j]
            };
            if rest[..start].contains(')') {
                // `vm_err!` named in a comment or a doc string, not called
                continue;
            }
            let value = key::unquote(literal).unwrap_or_else(|e| panic!("{name}:{line}: {e}"));
            let format = key::parse(&value).unwrap_or_else(|e| panic!("{name}:{line}: {e}"));
            keys.entry(format.key).or_insert_with(|| format!("{name}:{line}"));
        }
    }
    keys
}

#[test]
fn every_error_text_has_german() {
    let keys = error_keys(&manifest().join("src"));
    assert!(keys.len() > 150, "{} error keys", keys.len());
    // Their args are texts (`%@`): a translation must take each of them once
    for key in keys.keys().filter(|k| needs_german(k)) {
        let n = key.matches("%@").count();
        if let Some(german) = Language::German.lookup(key) {
            assert!(omaflake_i18n::fill(german, &vec!["x"; n], &vec![false; n]).is_some(), "{key} → {german}");
        }
    }
    let missing = missing(&keys);
    eprintln!("error messages: {} keys, {} without German", keys.len(), missing.len());
    assert!(
        missing.is_empty(),
        "{} error messages without German – add them to App/Resources/de.lproj/Localizable.strings, then run python3 \
         scripts/translations.py:\n{}",
        missing.len(),
        missing.join("\n")
    );
}

#[test]
fn the_macro_and_the_reader_agree() {
    // What vm_err! makes is what error_keys reads
    let e = crate::vm_err!(crate::error::codes::NOT_FOUND, "Image \"{}\" does not exist – {{x}} 5%", "a");
    let format = key::parse(&key::unquote(r#""Image \"{}\" does not exist – {{x}} 5%""#).unwrap()).unwrap();
    assert_eq!(e.key(), Some(format.key.as_str()));
}
