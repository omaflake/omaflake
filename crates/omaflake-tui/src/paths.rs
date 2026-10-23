// SPDX-License-Identifier: AGPL-3.0-or-later
//! Path input with Tab completion – the TUI's open panel for asks of kind folder, file and package (api.md §10.3).

use std::path::{Path, PathBuf};

/// `~` and `~/…` expanded with `$HOME`
pub fn expand(input: &str) -> PathBuf {
    let home = std::env::var_os("HOME").map(PathBuf::from);
    match (input.strip_prefix('~'), home) {
        (Some(rest), Some(home)) if rest.is_empty() || rest.starts_with('/') => home.join(rest.trim_start_matches('/')),
        _ => PathBuf::from(input),
    }
}

/// The entries of the input's folder that start with its last part: (completed input, candidates). With one
/// candidate the input becomes it (a folder with a trailing `/`); with several, their longest common start.
/// `folders_only` leaves files out (an ask of kind folder).
pub fn complete(input: &str, folders_only: bool) -> (String, Vec<String>) {
    let (dir_part, prefix) = match input.rfind('/') {
        Some(i) => (&input[..=i], &input[i + 1..]),
        None => ("", input),
    };
    let dir = if dir_part.is_empty() { PathBuf::from(".") } else { expand(dir_part) };
    let Ok(entries) = std::fs::read_dir(&dir) else { return (input.to_string(), vec![]) };
    let mut names: Vec<(String, bool)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            let is_dir = e.path().is_dir();
            let hidden_unasked = name.starts_with('.') && !prefix.starts_with('.');
            (name.starts_with(prefix) && !hidden_unasked && (is_dir || !folders_only)).then_some((name, is_dir))
        })
        .collect();
    names.sort();
    match names.len() {
        0 => (input.to_string(), vec![]),
        1 => {
            let (name, is_dir) = &names[0];
            (format!("{dir_part}{name}{}", if *is_dir { "/" } else { "" }), vec![])
        }
        _ => {
            let first = &names[0].0;
            let common = names.iter().fold(first.len(), |n, (s, _)| {
                first.bytes().zip(s.bytes()).take(n).take_while(|(a, b)| a == b).count()
            });
            let mut common = common;
            while !first.is_char_boundary(common) {
                common -= 1;
            }
            let shown = names.iter().map(|(n, d)| format!("{n}{}", if *d { "/" } else { "" })).collect();
            (format!("{dir_part}{}", &first[..common]), shown)
        }
    }
}

/// What a typed path is, as the ask needs it: absolute, `~` expanded, without a trailing `/` (a package is a folder)
pub fn normalize(input: &str) -> String {
    let p = expand(input.trim());
    let p = if p.is_relative() { std::env::current_dir().map(|d| d.join(&p)).unwrap_or(p) } else { p };
    let s = p.to_string_lossy().into_owned();
    if s.len() > 1 {
        s.trim_end_matches('/').to_string()
    } else {
        s
    }
}

/// Whether the path fits the ask's kind before it is sent: a folder must be one, a file must be one, a package
/// exists (the `check` method says more)
pub fn fits(kind: &str, path: &Path) -> Result<(), String> {
    match kind {
        "folder" if !path.is_dir() => Err(format!("Not a folder: {}", path.display())),
        "file" if !path.is_file() => Err(format!("Not a file: {}", path.display())),
        "package" if !path.exists() => Err(format!("Not found: {}", path.display())),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn completes_paths() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("alpha")).unwrap();
        std::fs::create_dir(dir.path().join("alps")).unwrap();
        std::fs::write(dir.path().join("beta.ipsw"), b"").unwrap();
        std::fs::write(dir.path().join(".hidden"), b"").unwrap();
        let base = format!("{}/", dir.path().display());
        let (c, cands) = complete(&format!("{base}al"), false);
        assert_eq!(c, format!("{base}alp"));
        assert_eq!(cands, ["alpha/", "alps/"]);
        assert_eq!(complete(&format!("{base}alph"), false).0, format!("{base}alpha/"));
        assert_eq!(complete(&format!("{base}b"), false).0, format!("{base}beta.ipsw"));
        assert_eq!(complete(&format!("{base}b"), true), (format!("{base}b"), vec![]));
        assert_eq!(complete(&base, false).1, ["alpha/", "alps/", "beta.ipsw"]);
        assert_eq!(normalize(&format!("{base}alpha/")), format!("{}/alpha", dir.path().display()));
        assert!(fits("folder", &dir.path().join("alpha")).is_ok());
        assert!(fits("file", &dir.path().join("alpha")).is_err());
    }
}
