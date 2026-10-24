// SPDX-License-Identifier: AGPL-3.0-or-later
//! `omaf ui` – omaflake's terminal interface (plan B121), on macOS and Linux. A pure client of the core API's page
//! models (docs/api.md §10): it subscribes the pages it shows (`views.subscribe`), draws each `view.changed` model
//! with the brand's colours (`omaflake-brand`) and sends the actions the model names. It holds no logic of omaflake's:
//! what a row shows, which action is possible and why not, what is asked and confirmed is the core's.
//!
//! Texts are the model's English `text`; translating by `key` is OPEN for the TUI.

pub mod app;
pub mod core;
pub mod icons;
pub mod model;
pub mod paths;
pub mod text;
pub mod theme;
pub mod ui;

use std::io::{self, Write};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event, MouseButton, MouseEventKind};
use crossterm::execute;
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::app::{App, Command, Effect, Msg};
use crate::core::Connection;
use crate::icons::IconSet;
use crate::theme::{ColorMode, Theme};

/// How `omaf ui` looks and connects
pub struct Options {
    pub icons: IconSet,
    pub colors: ColorMode,
    /// Mouse clicks and the wheel (takes the terminal's own text selection away while it runs)
    pub mouse: bool,
}

impl Options {
    /// From the environment (`NO_COLOR`, `COLORTERM`, `OMAF_NERD_FONT`)
    pub fn detect() -> Options {
        Options { icons: IconSet::detect(), colors: ColorMode::detect(), mouse: false }
    }
}

/// Connects (again): the service, or a server in this process
pub type Connector = dyn Fn() -> Result<Connection, String>;

/// Runs the TUI until it quits. `connect` is called at the start and whenever the connection to the service closed
/// (the service ended or restarted).
pub fn run(options: Options, connect: &Connector) -> Result<(), String> {
    let conn = connect()?;
    let (tx, rx) = mpsc::channel();
    let mut app = App::new(conn.core.clone(), tx.clone(), Theme::new(options.colors), options.icons, &conn.via);
    let mut events = conn.events;
    app.start();

    let mut term =
        enter(options.mouse).map_err(|e| text::trf("Cannot use the terminal: %@", &[e.to_string().into()]))?;
    // Restore the terminal on a panic, so the shell stays usable
    let hook = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        let _ = leave(true);
        hook(info);
    }));
    let mut last_tick = Instant::now();
    let result = loop {
        for (method, params) in events.try_iter() {
            app.handle(Msg::Event(method, params));
        }
        for msg in rx.try_iter() {
            app.handle(msg);
        }
        if !app.core.is_open() {
            app.set_status(text::tr("The service ended – connecting again …"), theme::Tone::Warning);
            match connect() {
                Ok(c) => {
                    events = c.events;
                    app.reconnect(c.core, &c.via);
                }
                Err(e) => break Err(e),
            }
        }
        while let Some(effect) = app.effects.pop_front() {
            if let Err(e) = carry_out(&mut term, &mut app, effect, options.mouse) {
                app.set_status(e, theme::Tone::Danger);
            }
        }
        if app.quit {
            break Ok(());
        }
        if last_tick.elapsed() >= Duration::from_millis(250) {
            app.tick += 1;
            last_tick = Instant::now();
        }
        if let Some((_, _, at)) = &app.status {
            if at.elapsed() > Duration::from_secs(12) {
                app.status = None;
            }
        }
        if let Err(e) = term.draw(|f| ui::draw(f, &mut app)) {
            break Err(e.to_string());
        }
        match event::poll(Duration::from_millis(100)) {
            Ok(true) => match event::read() {
                Ok(Event::Key(k)) if k.kind != event::KeyEventKind::Release => app.key(k),
                Ok(Event::Mouse(m)) => mouse(&mut app, m),
                _ => {}
            },
            Ok(false) => {}
            Err(e) => break Err(e.to_string()),
        }
    };
    let _ = leave(options.mouse);
    app.core.close();
    result
}

type Term = Terminal<CrosstermBackend<io::Stdout>>;

fn enter(mouse: bool) -> io::Result<Term> {
    enable_raw_mode()?;
    let mut out = io::stdout();
    execute!(out, EnterAlternateScreen)?;
    if mouse {
        execute!(out, EnableMouseCapture)?;
    }
    Terminal::new(CrosstermBackend::new(out))
}

fn leave(mouse: bool) -> io::Result<()> {
    let mut out = io::stdout();
    if mouse {
        execute!(out, DisableMouseCapture)?;
    }
    execute!(out, LeaveAlternateScreen)?;
    disable_raw_mode()
}

/// Clicks select a row (the list starts on the third line); the wheel moves
fn mouse(app: &mut App, m: event::MouseEvent) {
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    match m.kind {
        MouseEventKind::ScrollDown => app.key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)),
        MouseEventKind::ScrollUp => app.key(KeyEvent::new(KeyCode::Up, KeyModifiers::NONE)),
        MouseEventKind::Down(MouseButton::Left) if m.row >= 2 => {
            let key = format!("{:?}", app.current());
            let top = app.scroll.get(&key).copied().unwrap_or(0);
            let mut line = top + (m.row as usize - 2);
            for (i, h) in app.item_heights.clone().into_iter().enumerate() {
                if line < h {
                    app.click(i);
                    return;
                }
                line -= h;
            }
        }
        _ => {}
    }
}

/// Is there a desktop to open things on (not an SSH session without one)
fn has_desktop() -> bool {
    if cfg!(target_os = "macos") {
        std::env::var_os("SSH_CONNECTION").is_none()
    } else {
        std::env::var_os("WAYLAND_DISPLAY").is_some() || std::env::var_os("DISPLAY").is_some()
    }
}

fn opener() -> &'static str {
    if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    }
}

fn spawn_quiet(cmd: &mut std::process::Command) -> Result<(), String> {
    cmd.stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| e.to_string())
}

fn carry_out(term: &mut Term, app: &mut App, effect: Effect, mouse: bool) -> Result<(), String> {
    match effect {
        Effect::Reveal(path) => {
            if has_desktop() {
                let mut cmd = std::process::Command::new(opener());
                if cfg!(target_os = "macos") {
                    cmd.arg("-R").arg(&path);
                } else {
                    // xdg-open cannot select a file: its folder
                    let p = std::path::Path::new(&path);
                    cmd.arg(if p.is_dir() { p } else { p.parent().unwrap_or(p) });
                }
                spawn_quiet(&mut cmd)?;
            }
            app.set_status(path, theme::Tone::Accent);
        }
        Effect::Open(target) => {
            if has_desktop() {
                spawn_quiet(std::process::Command::new(opener()).arg(&target))?;
            }
            app.set_status(target, theme::Tone::Accent);
        }
        Effect::Terminal(command) => {
            leave(mouse).map_err(|e| e.to_string())?;
            let status = match &command {
                Command::Argv(argv) if !argv.is_empty() => {
                    std::process::Command::new(&argv[0]).args(&argv[1..]).status()
                }
                Command::Argv(_) => Ok(std::process::ExitStatus::default()),
                Command::Shell(line) => {
                    println!("$ {line}");
                    std::process::Command::new("/bin/sh").arg("-c").arg(line).status()
                }
            };
            let note = match &status {
                Ok(s) if s.success() => String::new(),
                Ok(s) => format!(" ({s})"),
                Err(e) => format!(" ({e})"),
            };
            print!("\n[omaf ui] {} ", text::trf("done%@ – press Enter to return", &[note.into()]));
            let _ = io::stdout().flush();
            let mut line = String::new();
            let _ = io::stdin().read_line(&mut line);
            *term = enter(mouse).map_err(|e| e.to_string())?;
            term.clear().map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}
