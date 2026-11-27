// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf ui` (plan B121): the terminal interface of `omaflake-tui`, connected to the core service (started on demand,
//! as `omaf build` does) or – when that fails or with `--in-process` – to a server in this process.

use std::io::IsTerminal;

use omaflake_core::{Error, Paths};
use omaflake_tui::Options;
use omaflake_tui::core::{self as tui_core, Connection};
use omaflake_tui::icons::IconSet;

use crate::cli::Ui;
use crate::commands::service;
use crate::util::Outcome;

/// The service, else a server of this process (with why the service was not used)
pub fn connect(in_process: bool, client: &str) -> Result<Connection, String> {
    let paths = Paths::standard();
    let mut why = String::new();
    if !in_process {
        match service::ensure_running(&paths) {
            Ok(running) => return Ok(tui_core::from_client(running.client, "service")),
            Err(e) => why = format!(" (service: {e})"),
        }
    }
    let mut conn = tui_core::in_process(paths, client).map_err(|e| e.message)?;
    conn.via = format!("in-process{why}");
    Ok(conn)
}

pub fn run(a: &Ui) -> Outcome {
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return Err(Error::new("omaf ui needs a terminal").into());
    }
    let mut options = Options::detect();
    if a.nerd_font {
        options.icons = IconSet::NerdFont;
    }
    options.mouse = a.mouse;
    let in_process = a.in_process;
    // Also when the service ended while the TUI ran: it is started again
    omaflake_tui::run(options, &move || connect(in_process, "omaf ui")).map_err(|e| Error::new(e).into())
}
