// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf status [--waybar] [--follow]` (plans B119, B121): running VMs and builds in one line – for a shell prompt, a
//! status bar, and as waybar `custom` module JSON (docs/cli.md has the snippet). With `--follow` it stays connected
//! to the core service and prints a new line whenever the line changes (waybar's `exec` without `interval`).

use std::io::Write;
use std::time::Duration;

use omaflake_core::api::service::{is_running, socket_path};
use omaflake_core::{Error, Paths};
use omaflake_tui::core::Connection;
use serde_json::{Value, json};

use crate::cli::Status;
use crate::commands::{service, ui};
use crate::util::Outcome;

/// What the line is made of
#[derive(Debug, Default, PartialEq)]
pub struct Summary {
    pub running: Vec<String>,
    /// Builds waiting or running: (name, state, step title, fraction)
    pub builds: Vec<Build>,
    pub failed: Vec<String>,
}

#[derive(Debug, PartialEq)]
pub struct Build {
    pub name: String,
    pub state: String,
    pub step: String,
    pub fraction: Option<f64>,
}

/// From `vms.list` and `jobs.list`
pub fn summary(vms: &Value, jobs: &Value) -> Summary {
    let running = vms["vms"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|v| v["status"] == "running")
        .filter_map(|v| v["name"].as_str().map(str::to_string))
        .collect();
    let mut s = Summary { running, ..Summary::default() };
    for j in jobs.as_array().into_iter().flatten().filter(|j| j["kind"] == "build") {
        let name = j["recipe"]
            .as_str()
            .map(|r| r.split('@').next().unwrap_or(r).rsplit('.').next().unwrap_or(r).to_string())
            .unwrap_or_else(|| j["title"].as_str().unwrap_or_default().to_string());
        match j["state"].as_str().unwrap_or_default() {
            st @ ("queued" | "checking" | "running") => s.builds.push(Build {
                name,
                state: st.into(),
                step: j["progress"]["title"].as_str().unwrap_or_default().to_string(),
                fraction: j["progress"]["fraction"].as_f64(),
            }),
            "failed" => s.failed.push(name),
            _ => {}
        }
    }
    s
}

impl Summary {
    /// The running build's progress in percent
    fn percent(&self) -> Option<i64> {
        self.builds.iter().find(|b| b.state == "running").map(|b| (b.fraction.unwrap_or(0.0) * 100.0).round() as i64)
    }

    /// "2 VMs running (a, b) · building dev 62 %"
    pub fn line(&self) -> String {
        let mut parts = vec![match self.running.len() {
            0 => "no VM running".to_string(),
            1 => format!("1 VM running ({})", self.running[0]),
            n => format!("{n} VMs running ({})", self.running.join(", ")),
        }];
        for b in &self.builds {
            parts.push(match (b.state.as_str(), b.fraction) {
                ("running", Some(f)) => format!("building {} {} %", b.name, (f * 100.0).round() as i64),
                ("running", None) => format!("building {}", b.name),
                (st, _) => format!("{} {st}", b.name),
            });
        }
        for f in &self.failed {
            parts.push(format!("build of {f} failed"));
        }
        parts.join(" · ")
    }

    /// waybar's custom module JSON (`return-type: json`): text, tooltip, class, alt, percentage
    pub fn waybar(&self) -> Value {
        let state = if !self.builds.is_empty() {
            "building"
        } else if !self.running.is_empty() {
            "running"
        } else {
            "idle"
        };
        let mut text = self.running.len().to_string();
        if let Some(p) = self.percent() {
            text.push_str(&format!(" · {p}%"));
        } else if !self.builds.is_empty() {
            text.push_str(" · queued");
        }
        let mut tooltip = vec![if self.running.is_empty() {
            "No VM running".to_string()
        } else {
            format!("Running: {}", self.running.join(", "))
        }];
        for b in &self.builds {
            tooltip.push(match b.fraction {
                Some(f) if b.state == "running" => {
                    format!("Building {}: {} ({} %)", b.name, b.step, (f * 100.0).round() as i64)
                }
                _ => format!("Build of {}: {}", b.name, b.state),
            });
        }
        for f in &self.failed {
            tooltip.push(format!("Build of {f} failed"));
        }
        let mut class = vec![state];
        if !self.failed.is_empty() {
            class.push("failed");
        }
        json!({
            "text": text,
            "tooltip": tooltip.join("\n"),
            "class": class,
            "alt": state,
            "percentage": self.percent().unwrap_or(0),
        })
    }
}

fn look(conn: &Connection) -> Result<Summary, String> {
    let vms = conn.core.call("vms.list", Value::Null).map_err(|e| e.message)?;
    let jobs = conn.core.call("jobs.list", Value::Null).map_err(|e| e.message)?;
    Ok(summary(&vms, &jobs))
}

fn render(s: &Summary, waybar: bool) -> String {
    if waybar { s.waybar().to_string() } else { s.line() }
}

pub fn run(a: &Status) -> Outcome {
    let paths = Paths::standard();
    if !a.follow {
        // A running service knows the builds; without one, this process looks at the data folder
        let conn = if is_running(&socket_path(&paths)) {
            ui::connect(false, "omaf status")
        } else {
            ui::connect(true, "omaf status")
        }
        .map_err(Error::new)?;
        let s = look(&conn).map_err(Error::new)?;
        println!("{}", render(&s, a.waybar));
        conn.core.close();
        return Ok(());
    }
    let mut last = String::new();
    loop {
        let conn = match service::ensure_running(&paths) {
            Ok(r) => omaflake_tui::core::from_client(r.client, "service"),
            Err(e) => {
                eprintln!("omaf status: {e} – trying again in 10 s");
                std::thread::sleep(Duration::from_secs(10));
                continue;
            }
        };
        if conn.core.call("events.subscribe", json!({"topics": ["vms", "jobs"]})).is_err() {
            std::thread::sleep(Duration::from_secs(2));
            continue;
        }
        while conn.core.is_open() {
            if let Ok(s) = look(&conn) {
                let line = render(&s, a.waybar);
                if line != last {
                    println!("{line}");
                    if std::io::stdout().flush().is_err() {
                        return Ok(());
                    }
                    last = line;
                }
            }
            // Woken by the next event; several in a row (a build's progress) are taken together
            let _ = conn.events.recv_timeout(Duration::from_secs(30));
            std::thread::sleep(Duration::from_millis(300));
            while conn.events.try_recv().is_ok() {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture() -> (Value, Value) {
        let vms = json!({"vms": [
            {"name": "research", "status": "running"},
            {"name": "work", "status": "stopped"},
            {"name": "mac", "status": "running"}
        ], "unreadable": []});
        let jobs = json!([
            {"id": "j1", "kind": "build", "recipe": "omaflake.nixos.dev@velvet-tiger-orbit", "title": "Build dev",
             "state": "running", "progress": {"step": 3, "total": 5, "title": "Copying the image", "fraction": 0.62}},
            {"id": "j2", "kind": "build", "recipe": "omaflake.macos.base@x", "title": "Build base", "state": "queued"},
            {"id": "j3", "kind": "build", "recipe": "omaflake.nixos.gnome@y", "title": "Build gnome", "state": "failed"},
            {"id": "j4", "kind": "export", "title": "Export", "state": "running"}
        ]);
        (vms, jobs)
    }

    #[test]
    fn waybar_json() {
        let (vms, jobs) = fixture();
        let s = summary(&vms, &jobs);
        assert_eq!(s.running, ["research", "mac"]);
        assert_eq!(s.builds.len(), 2);
        let w = s.waybar();
        assert_eq!(w["text"], "2 · 62%");
        assert_eq!(w["percentage"], 62);
        assert_eq!(w["alt"], "building");
        assert_eq!(w["class"], json!(["building", "failed"]));
        assert_eq!(
            w["tooltip"],
            "Running: research, mac\nBuilding dev: Copying the image (62 %)\nBuild of base: queued\nBuild of gnome failed"
        );
        // One line, as waybar reads it
        assert!(!w.to_string().contains('\n'));
        assert_eq!(s.line(), "2 VMs running (research, mac) · building dev 62 % · base queued · build of gnome failed");
    }

    #[test]
    fn idle() {
        let s = summary(&json!({"vms": []}), &json!([]));
        assert_eq!(
            s.waybar(),
            json!({"text": "0", "tooltip": "No VM running", "class": ["idle"], "alt": "idle", "percentage": 0})
        );
        assert_eq!(s.line(), "no VM running");
        let one = summary(&json!({"vms": [{"name": "a", "status": "running"}]}), &json!([]));
        assert_eq!(one.waybar()["class"], json!(["running"]));
        assert_eq!(one.line(), "1 VM running (a)");
    }
}
