// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the commands share (Swift `Env`, `gib`, `pad`, `readPassword`, `chooseRecipe`, `shortestArgument`,
//! `resolveImage`, `folderURL`, `sudo` in Sources/omaf/*.swift) and how a command fails.

use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};

use chrono::{DateTime, Local, Utc};
use omaflake_core::platform::{expand_tilde, standardize};
use omaflake_core::recipe_sources::RecipeRef;
use omaflake_core::{
    Backends, Error, ImageStore, OmaflakeId, OmaflakeIdKind, Paths, RecipeDefaults, RecipeId, Result, Settings,
};

/// How a command fails, and the exit code (swift-argument-parser's behaviour)
#[derive(Debug)]
pub enum Failure {
    /// A thrown `VMError`: "Error: <message>" on stderr, exit code 1
    Error(Error),
    /// A `ValidationError`: "Error: <message>", the command's usage and a help hint on stderr, exit code 64
    Usage { message: String, command: Vec<&'static str> },
    /// `ExitCode(n)`: nothing printed, exit code n
    Exit(i32),
}

impl From<Error> for Failure {
    fn from(e: Error) -> Self {
        Failure::Error(e)
    }
}

/// A `ValidationError` of the command `omaf <command…>`
pub fn usage(command: &[&'static str], message: impl Into<String>) -> Failure {
    Failure::Usage { message: message.into(), command: command.to_vec() }
}

pub type Outcome = std::result::Result<(), Failure>;

/// Exit code of swift-argument-parser for usage errors (EX_USAGE)
pub const EX_USAGE: i32 = 64;

/// Shared access to paths, stores and runtime (Swift `Env`)
pub struct Env {
    pub paths: Paths,
}

impl Env {
    /// The standard data folder. Older image layouts and recipe names (before image.json) are migrated first; cheap
    /// when there is nothing to do.
    pub fn new() -> Env {
        let paths = Paths::standard();
        for note in ImageStore::new(paths.clone()).migrate_legacy() {
            eprintln!("migrated: {note}");
        }
        Env { paths }
    }

    pub fn with_paths(paths: Paths) -> Env {
        Env { paths }
    }

    pub fn settings(&self) -> Settings {
        Settings::load(&self.paths)
    }

    pub fn backends(&self) -> Result<Backends> {
        Backends::new(self.paths.clone())
    }

    pub fn images(&self) -> ImageStore {
        ImageStore::new(self.paths.clone())
    }
}

/// "512 MiB" below one GiB, else "1.5 GiB"
pub fn gib(bytes: u64) -> String {
    if bytes < 1_073_741_824 {
        format!("{} MiB", bytes / 1_048_576)
    } else {
        format!("{:.1} GiB", bytes as f64 / 1_073_741_824.0)
    }
}

/// `s` padded with spaces to `n` characters; a longer one gets one space after it
pub fn pad(s: &str, n: usize) -> String {
    let count = s.chars().count();
    if count >= n {
        format!("{s} ")
    } else {
        format!("{s}{}", " ".repeat(n - count))
    }
}

/// A date in the Mac's time zone, e.g. "yyyy-MM-dd HH:mm" → `%Y-%m-%d %H:%M` (Swift `DateFormatter.dateFormat`)
pub fn local_date(date: &DateTime<Utc>, format: &str) -> String {
    date.with_timezone(&Local).format(format).to_string()
}

/// `~` expanded, then standardized (Swift `URL(fileURLWithPath: expandingTildeInPath).standardizedFileURL`)
pub fn folder_path(path: &str) -> PathBuf {
    standardize(Path::new(&expand_tilde(path)))
}

/// Swift `readLine(strippingNewline: true)` on stdin: None at the end of input
pub fn read_line() -> Option<String> {
    let mut line = String::new();
    match std::io::stdin().lock().read_line(&mut line) {
        Ok(0) | Err(_) => None,
        Ok(_) => {
            if line.ends_with('\n') {
                line.pop();
                if line.ends_with('\r') {
                    line.pop();
                }
            }
            Some(line)
        }
    }
}

/// Password from the terminal (not echoed) or one line from stdin; never stored.
pub fn read_password(prompt: &str, stdin: bool, confirm: bool) -> Result<String> {
    let password = if stdin {
        read_line().unwrap_or_default()
    } else {
        let password = crate::tty::read_passphrase(prompt)?;
        if confirm && crate::tty::read_passphrase("Repeat: ")? != password {
            return Err(Error::new("Passwords do not match"));
        }
        password
    };
    if password.is_empty() {
        return Err(Error::new("Empty password"));
    }
    Ok(password)
}

/// Shortest way to name a recipe on the command line: <name>, <kind>/<name> or <author>/<kind>/<name>.
pub fn shortest_argument(id: &str, defaults: &HashMap<String, RecipeDefaults>) -> String {
    let Some(p) = RecipeId::parts(id) else { return id.to_string() };
    for arg in [p.name.clone(), format!("{}/{}", p.kind.as_str(), p.name), RecipeId::qualified(id)] {
        if RecipeId::candidates_in(&arg, defaults) == [id] {
            return arg;
        }
    }
    let words: Vec<String> = defaults
        .keys()
        .filter(|k| RecipeId::base(k) == RecipeId::base(id))
        .filter_map(|k| RecipeId::words(k).map(str::to_string))
        .collect();
    OmaflakeId::recipe(id, OmaflakeId::shortest([words]))
}

/// The lines `choose_recipe` shows for several recipes: "  1) <omaflake ID padded to 52><image>"
pub fn choice_lines(ids: &[String], image: &dyn Fn(&str) -> String) -> Vec<String> {
    let words: Vec<String> = ids.iter().filter_map(|id| RecipeId::words(id).map(str::to_string)).collect();
    let n = OmaflakeId::shortest([words]);
    ids.iter()
        .enumerate()
        .map(|(i, id)| format!("  {}) {}{}", i + 1, pad(&OmaflakeId::recipe(id, n), 52), image(id)))
        .collect()
}

/// The number typed for `choose_recipe`, if it is one of 1…count
pub fn parse_choice(line: &str, count: usize) -> Option<usize> {
    let n: usize = line.trim_matches([' ', '\t']).parse().ok()?;
    n.checked_sub(1).filter(|&i| i < count)
}

/// In a terminal: lets the user pick one of several recipes (author, kind, name, current image); elsewhere None.
pub fn choose_recipe(env: &Env, ids: &[String]) -> Option<String> {
    // SAFETY: isatty only inspects the descriptor
    if unsafe { libc::isatty(libc::STDIN_FILENO) } == 0 {
        return None;
    }
    let images = env.images();
    println!("Several recipes match:");
    let image = |id: &str| {
        images
            .current(id)
            .map(|i| format!("image {}, {}", i.info.revision, local_date(&i.info.created, "%Y-%m-%d")))
            .unwrap_or_else(|| "no image yet".into())
    };
    for line in choice_lines(ids, &image) {
        println!("{line}");
    }
    print!("Choose [1-{}]: ", ids.len());
    let _ = std::io::stdout().flush();
    let line = read_line()?;
    parse_choice(&line, ids.len()).map(|i| ids[i].clone())
}

/// An image by its name, its omaflake ID (image:<author>/<kind>/<name>:<words>) or the start of its name's words
pub fn resolve_image_in(input: &str, names: &[String]) -> Result<String> {
    if names.iter().any(|n| n == input) {
        return Ok(input.to_string());
    }
    let mut base = RecipeId::base(input).to_string();
    let mut words = RecipeId::words(input).unwrap_or("").to_string();
    if let Some(o) = OmaflakeId::parse(input) {
        if o.kind == OmaflakeIdKind::Image {
            if let Some(b) = RecipeRef::public_id(&o.identifier) {
                (base, words) = (b, o.words);
            }
        }
    }
    let found: Vec<&String> = names
        .iter()
        .filter(|n| RecipeId::base(n) == base && RecipeId::words(n).is_some_and(|w| w.starts_with(&words)))
        .collect();
    match found.as_slice() {
        [one] => Ok(one.to_string()),
        [] => Err(Error::new(format!("No image {input} – see omaf images"))),
        more => Err(Error::new(format!(
            "{input} is more than one image: {}",
            more.iter().map(|s| s.as_str()).collect::<Vec<_>>().join(", ")
        ))),
    }
}

pub fn resolve_image(input: &str, env: &Env) -> Result<String> {
    let names: Vec<String> = env.images().list().into_iter().map(|i| i.name).collect();
    resolve_image_in(input, &names)
}

/// Runs a root shell script through sudo in this terminal (password or Touch ID, as sudo is set up); a failure exits
/// with sudo's status.
pub fn sudo(script: &str) -> Outcome {
    use std::os::unix::process::ExitStatusExt;
    let status = std::process::Command::new("/usr/bin/sudo")
        .args(["/bin/sh", "-c", script])
        .status()
        .map_err(|e| Error::new(omaflake_core::error::os_text(&e)))?;
    // Swift's terminationStatus: the exit code, or the signal's number
    let code = status.code().or_else(|| status.signal()).unwrap_or(1);
    if code != 0 {
        return Err(Failure::Exit(code));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pads_like_swift() {
        assert_eq!(pad("NAME", 8), "NAME    ");
        assert_eq!(pad("exactly8", 8), "exactly8 ");
        assert_eq!(pad("dev ⚗", 7), "dev ⚗  ");
        assert_eq!(pad("–", 3), "–  ");
    }

    #[test]
    fn sizes() {
        assert_eq!(gib(0), "0 MiB");
        assert_eq!(gib(512 * 1_048_576), "512 MiB");
        assert_eq!(gib(1_073_741_824), "1.0 GiB");
        assert_eq!(gib(3 * 1_073_741_824 / 2), "1.5 GiB");
    }

    #[test]
    fn choices() {
        assert_eq!(parse_choice(" 2 ", 3), Some(1));
        assert_eq!(parse_choice("0", 3), None);
        assert_eq!(parse_choice("4", 3), None);
        assert_eq!(parse_choice("x", 3), None);
    }

    #[test]
    fn images_by_name_or_words() {
        let names = vec!["omaflake.nixos.dev@apple-banana".to_string(), "omaflake.nixos.dev@cherry-date".to_string()];
        assert_eq!(resolve_image_in("omaflake.nixos.dev@apple-banana", &names).unwrap(), names[0]);
        assert_eq!(resolve_image_in("omaflake.nixos.dev@ch", &names).unwrap(), names[1]);
        assert_eq!(
            resolve_image_in("omaflake.nixos.dev", &names).unwrap_err().message(),
            "omaflake.nixos.dev is more than one image: omaflake.nixos.dev@apple-banana, omaflake.nixos.dev@cherry-date"
        );
        assert_eq!(resolve_image_in("other", &names).unwrap_err().message(), "No image other – see omaf images");
    }
}
