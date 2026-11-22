// SPDX-License-Identifier: AGPL-3.0-or-later
//! The diagnosis methods (plan B162, docs/api.md page `diagnostics`): `diagnostics.run {name}` starts the job
//! `diagnostics` (one per VM at a time), `diagnostics.confirm {name, check, ok}` turns a check the user judges into
//! pass or fail, `diagnostics.copy {name}` puts the newest log on this computer's clipboard. The server keeps each VM's
//! newest run (`Inner::diagnostics`); the page shows it while it belongs to the VM's current run.

use std::collections::BTreeMap;
use std::io::Write;

use serde::Deserialize;
use serde_json::{Value, json};

use super::jobs::{JobCtx, JobSpec};
use super::protocol::RpcError;
use super::server::{Answer, Reply, Server, lock};
use super::types::JobKind;
use super::views::PageRef;
use super::views::diagnostics::DiagnosisRecord;
use crate::backend::Backends;
use crate::diagnostics::{self, CheckResult, CheckStatus, Diagnosis, LiveProbe};
use crate::error::codes;
use crate::models::VmStatus;
use crate::paths::Paths;
use crate::vm_err;

#[derive(Deserialize)]
pub(crate) struct ConfirmParams {
    name: String,
    check: String,
    ok: bool,
}

impl Server {
    /// `diagnostics.run`: the job, or the one that runs for this VM already
    pub(crate) fn diagnostics_run(&self, name: &str) -> Reply {
        let paths = self.paths();
        let backends = Backends::new(paths.clone()).map_err(RpcError::from)?;
        let c = backends.store.load(name).map_err(RpcError::from)?;
        let backend = backends.backend_for(&c).map_err(RpcError::from)?;
        if backend.status(&c) != VmStatus::Running {
            return Err(RpcError::from(vm_err!(codes::VM_STOPPED, "“{}” is not running – start it first", name)));
        }
        let run = diagnostics::run_id(&paths, &c);
        // Held until the job is in the list: two calls at once start one job
        let mut records = lock(&self.inner.diagnostics);
        if let Some(job) = records.get(name).and_then(|r| self.inner.jobs.get(&r.job)).filter(|j| j.is_active()) {
            return Ok(Answer::Job(job));
        }
        let file = diagnostics::log_file(&paths, name);
        records.insert(name.to_string(), DiagnosisRecord::new(run, file.to_string_lossy().into_owned()));
        let s = self.clone();
        let vm = name.to_string();
        let mut spec = JobSpec::new(
            JobKind::Diagnostics,
            format!("Diagnosis of {name}"),
            Box::new(move |ctx: &JobCtx| {
                let probe = LiveProbe { paths: s.paths(), config: c.clone() };
                let sink = ctx.log_sink();
                let log = move |line: &str| sink(line);
                let on_check = |r: &CheckResult| {
                    if let Some(rec) = lock(&s.inner.diagnostics).get_mut(&vm) {
                        rec.set(r);
                    }
                    s.poke();
                };
                let mut d = Diagnosis::new(&c, &probe, &log, &on_check);
                d.cancel = Some(ctx.cancellation());
                let results = d.run();
                let file = {
                    let mut records = lock(&s.inner.diagnostics);
                    let rec = records.get_mut(&vm);
                    rec.map(|r| {
                        r.finished = Some(crate::json::now());
                        r.file.clone()
                    })
                    .unwrap_or_default()
                };
                Ok(json!({"checks": results, "file": file}))
            }),
        );
        spec.log_file = Some(file);
        let job = self.inner.jobs.submit(spec);
        if let Some(r) = records.get_mut(name) {
            r.job = job.id.clone();
        }
        drop(records);
        self.poke();
        Ok(Answer::Job(job))
    }

    /// `diagnostics.confirm`: what the user heard – logged and kept; answers the page's model
    pub(crate) fn diagnostics_confirm(&self, p: ConfirmParams) -> Reply {
        let line = {
            let mut records = lock(&self.inner.diagnostics);
            let check = records
                .get_mut(&p.name)
                .and_then(|r| r.checks.iter_mut().find(|c| c.id == p.check))
                .filter(|c| c.status == CheckStatus::Confirm)
                .ok_or_else(|| {
                    RpcError::invalid_params(format!("no check \"{}\" of \"{}\" waits for you", p.check, p.name))
                })?;
            check.status = if p.ok { CheckStatus::Pass } else { CheckStatus::Fail };
            let said = if p.ok { "confirmed by you" } else { "not heard, you said" };
            check.detail =
                if check.detail.is_empty() { said.to_string() } else { format!("{} – {said}", check.detail) };
            let line = check.line();
            records.get(&p.name).map(|r| (r.file.clone(), line))
        };
        if let Some((file, line)) = line
            && let Ok(mut f) = std::fs::OpenOptions::new().append(true).open(&file)
        {
            let _ = writeln!(f, "{line}");
        }
        self.poke();
        Answer::of(self.view(&PageRef { page: "diagnostics".into(), params: json!({"name": p.name}) })?)
    }

    /// `diagnostics.copy`: the newest log's text on this computer's clipboard
    pub(crate) fn diagnostics_copy(&self, name: &str) -> Reply {
        let paths = self.paths();
        let file = lock(&self.inner.diagnostics)
            .get(name)
            .map(|r| std::path::PathBuf::from(&r.file))
            .filter(|f| f.exists())
            .or_else(|| diagnostics::newest_log(&paths, name));
        let copied =
            file.and_then(|f| std::fs::read_to_string(f).ok()).is_some_and(|text| diagnostics::copy_text(&text));
        Answer::of(json!({"copied": copied}))
    }

    /// The newest diagnosis of each VM that belongs to its current run (the page's state)
    pub(crate) fn diagnoses_current(&self, paths: &Paths) -> BTreeMap<String, DiagnosisRecord> {
        let records: Vec<(String, DiagnosisRecord)> =
            lock(&self.inner.diagnostics).iter().map(|(k, v)| (k.clone(), v.clone())).collect();
        if records.is_empty() {
            return BTreeMap::new();
        }
        let store = self.store();
        records
            .into_iter()
            .filter(|(name, r)| {
                store.load(name).ok().is_some_and(|c| r.run.is_some() && diagnostics::run_id(paths, &c) == r.run)
            })
            .collect()
    }

    /// A subscribed (or fetched) diagnosis page runs one when its VM runs and has none since it started
    pub(crate) fn diagnose_if_none(&self, pages: &[PageRef]) {
        for p in pages.iter().filter(|p| p.page == "diagnostics") {
            let Some(name) = p.params.get("name").and_then(Value::as_str).map(str::to_string) else { continue };
            let paths = self.paths();
            let Ok(c) = self.store().load(&name) else { continue };
            let run = diagnostics::run_id(&paths, &c);
            if run.is_none() {
                continue; // not running
            }
            let known = lock(&self.inner.diagnostics)
                .get(&name)
                .is_some_and(|r| r.run == run || self.inner.jobs.get(&r.job).is_some_and(|j| j.is_active()));
            if known {
                continue;
            }
            let s = self.clone();
            std::thread::spawn(move || {
                let _ = s.diagnostics_run(&name);
            });
        }
    }
}
