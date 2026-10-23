// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the app asks of the desktop: open a file or URL with its default app, show something in the file manager,
//! run a command line in a terminal, post a notification. Linux: `xdg-open`, the user's terminal emulator; macOS
//! (a development aid only): `open`, Terminal.app through osascript.

use std::path::Path;
use std::process::{Command, Stdio};

/// Terminal emulators tried in this order when `$TERMINAL` is not set
pub const TERMINALS: [&str; 7] = ["foot", "kitty", "wezterm", "alacritty", "gnome-terminal", "konsole", "xterm"];

/// The command that runs `line` (a shell line) in a terminal: `$TERMINAL` if set, else the first of `TERMINALS`
/// that `available` says is installed. Each emulator takes its command its own way.
pub fn terminal_command(
    line: &str,
    env_terminal: Option<&str>,
    available: &dyn Fn(&str) -> bool,
) -> Option<Vec<String>> {
    let shell = vec!["sh".to_string(), "-c".to_string(), format!("{line}; exec \"${{SHELL:-sh}}\"")];
    let chosen = match env_terminal.map(str::trim).filter(|t| !t.is_empty()) {
        Some(t) => t.to_string(),
        None => TERMINALS.iter().find(|t| available(t))?.to_string(),
    };
    // `$TERMINAL` may carry arguments ("kitty --single-instance")
    let mut argv: Vec<String> = chosen.split_whitespace().map(str::to_string).collect();
    let program = Path::new(&argv[0]).file_name().and_then(|n| n.to_str()).unwrap_or("").to_string();
    match program.as_str() {
        "gnome-terminal" | "kgx" => argv.push("--".into()),
        "wezterm" => {
            argv.push("start".into());
            argv.push("--".into());
        }
        "foot" | "kitty" => {}
        // alacritty, konsole, xterm, st, urxvt, xfce4-terminal and most others
        _ => argv.push("-e".into()),
    }
    argv.extend(shell);
    Some(argv)
}

fn spawn(argv: &[String]) -> Result<(), String> {
    let (program, args) = argv.split_first().ok_or("nothing to run")?;
    Command::new(program)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(|_| ())
        .map_err(|e| format!("{program}: {e}"))
}

/// Runs a shell line in a terminal
pub fn run_in_terminal(line: &str) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        let script = format!(
            "tell application \"Terminal\" to do script \"{}\"\ntell application \"Terminal\" to activate",
            line.replace('\\', "\\\\").replace('"', "\\\"")
        );
        return spawn(&["osascript".into(), "-e".into(), script]);
    }
    let env = std::env::var("TERMINAL").ok();
    let argv = terminal_command(line, env.as_deref(), &|t| crate::backend::which(t).is_some())
        .ok_or_else(|| format!("No terminal found – set $TERMINAL (tried {})", TERMINALS.join(", ")))?;
    spawn(&argv)
}

/// Opens a file, folder or URL with the system's default app
pub fn open(target: &str) -> Result<(), String> {
    let opener = if cfg!(target_os = "macos") { "open" } else { "xdg-open" };
    spawn(&[opener.into(), target.into()])
}

/// Shows a path in the file manager: its folder (Linux has no common "select this file"; a folder opens itself)
pub fn reveal(path: &str) -> Result<(), String> {
    if cfg!(target_os = "macos") {
        return spawn(&["open".into(), "-R".into(), path.into()]);
    }
    let p = Path::new(path);
    let target = if p.is_dir() { p } else { p.parent().unwrap_or(p) };
    open(&target.to_string_lossy())
}

/// A desktop notification (Linux: org.freedesktop.Notifications)
pub fn notify(title: &str, body: &str) {
    let mut n = notify_rust::Notification::new();
    n.summary(title).body(body).appname("omaflake");
    #[cfg(target_os = "linux")]
    n.icon("omaflake");
    if let Err(e) = n.show() {
        eprintln!("omaflake: notification: {e}");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(line: &str, env: Option<&str>, installed: &[&str]) -> Option<Vec<String>> {
        terminal_command(line, env, &|t| installed.contains(&t))
    }

    #[test]
    fn terminal_comes_from_terminal_env_first() {
        let a = argv("ssh vm", Some("kitty --single-instance"), &["foot"]).unwrap();
        assert_eq!(a[..2], ["kitty".to_string(), "--single-instance".to_string()]);
        assert_eq!(a[2..4], ["sh".to_string(), "-c".to_string()]);
        assert!(a[4].starts_with("ssh vm; exec"));
        let a = argv("x", Some("/usr/bin/alacritty"), &[]).unwrap();
        assert_eq!(a[1], "-e");
    }

    #[test]
    fn otherwise_the_first_installed_one() {
        assert_eq!(argv("x", None, &["xterm", "konsole"]).unwrap()[..2], ["konsole".to_string(), "-e".to_string()]);
        assert_eq!(argv("x", Some(" "), &["foot", "xterm"]).unwrap()[0], "foot");
        assert_eq!(
            argv("x", None, &["gnome-terminal"]).unwrap()[..2],
            ["gnome-terminal".to_string(), "--".to_string()]
        );
        assert_eq!(argv("x", None, &["wezterm"]).unwrap()[..3], ["wezterm", "start", "--"].map(String::from));
        assert_eq!(argv("x", None, &[]), None);
    }
}
