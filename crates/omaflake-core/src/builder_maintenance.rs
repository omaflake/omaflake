// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of BuilderMaintenance.swift (with `AdminShell`) – work package F5.
//!
//! Admin tasks around the Linux builder: apply the image flake's nix-darwin host (`#linux-builder`) and clean up
//! the builder's Nix store. Both need root – the app asks through macOS' administrator dialog (`AdminShell`), the CLI
//! through sudo. After Developer ID signing a privileged helper replaces the dialog (spec §11.5).

use std::fs;
use std::path::{Path, PathBuf};

use crate::error::{codes, Error, IoContext, Result};
use crate::json;
use crate::paths::{Paths, Settings};
use crate::platform::is_executable_file;
use crate::prerequisites::Prerequisites;
use crate::shell::{Shell, StreamOptions};
use crate::vm_err;

#[derive(Debug, Clone)]
pub struct BuilderMaintenance {
    pub paths: Paths,
}

impl BuilderMaintenance {
    /// Written by nix/hosts/linux-builder.nix: the running nix-darwin system comes from omaflake's flake
    pub const MARKER: &'static str = "/etc/omaflake/linux-builder";

    /// Nix with the official installer of the NixOS community (github.com/NixOS/nix-installer): creates the /nix
    /// volume, the build users and the daemon; `--no-confirm` because the app has asked already. Uninstall:
    /// `/nix/nix-installer uninstall`. Pinned: one release, its macOS binary checked against the release's SHA-256
    /// before it runs (plan B66). `OMAFLAKE_NIX_INSTALLER_URL` points to a mirror of the same file.
    pub const NIX_INSTALLER_VERSION: &'static str = "2.35.3";
    pub const NIX_INSTALLER_SHA256: &'static str = "c4991562a71a7ef2dc52d9760545c8541db4a468bbcc91dcafd57175638bf17e";

    /// Unused store paths in the builder (everything can be rebuilt or downloaded again), with free space before/after.
    /// `ssh linux-builder` works for root only (the key in /etc/nix is root's).
    pub const CLEAN_UP_COMMAND: &'static str =
        "ssh linux-builder 'df -h / | tail -1; nix-store --gc 2>&1 | tail -1; df -h / | tail -1'";

    pub fn new(paths: Paths) -> Self {
        BuilderMaintenance { paths }
    }

    /// Is the current nix-darwin system omaflake's `#linux-builder`? If not, applying replaces the user's own
    /// nix-darwin configuration – the app says so before it asks.
    pub fn system_is_omaflakes(&self) -> bool {
        Path::new(Self::MARKER).exists()
    }

    /// logs/linux-builder.log
    pub fn log(&self) -> PathBuf {
        self.paths.logs().join("linux-builder.log")
    }

    /// Step 1 of `darwin-rebuild switch`, as the user: builds the image flake's builder host and returns the system's
    /// store path. Root cannot do this – macOS' privacy protection keeps it out of ~/Documents etc. (plan B40), and it
    /// would write a root-owned flake.lock into the checkout. Swift default `host`: "linux-builder".
    /// `nix build --no-link --print-out-paths --no-write-lock-file --extra-experimental-features "nix-command flakes"
    /// path:<flake>#darwinConfigurations.<host>.system`; failure: "Building the Linux builder host failed:\n<last 1500
    /// characters of stderr>" (the stream's stderr is empty – Swift quirk kept, OPEN: show the streamed lines instead).
    pub fn build_system(&self, host: &str, on_line: &(dyn Fn(&str) + Send + Sync)) -> Result<String> {
        let settings = Settings::load(&self.paths);
        let flake = settings.flake_path()?;
        let nix = Prerequisites::locate_nix(&settings)
            .ok_or_else(|| Error::with_code(codes::PREREQUISITE, "nix not found"))?;
        let target = format!("path:{flake}#darwinConfigurations.{host}.system");
        let args = [
            "build",
            "--no-link",
            "--print-out-paths",
            "--no-write-lock-file",
            "--extra-experimental-features",
            "nix-command flakes",
            target.as_str(),
        ];
        let r = Shell::stream(&nix, &args, &StreamOptions::default(), on_line)?;
        let out = r.stdout.trim();
        if r.status != 0 || !Self::is_system(out) {
            return Err(vm_err!("Building the Linux builder host failed:\n{}", suffix(&r.stderr, 1500)));
        }
        Ok(out.to_string())
    }

    /// A nix-darwin system in the store (the only thing root gets to activate)
    pub(crate) fn is_system(path: &str) -> bool {
        path.starts_with("/nix/store/")
            && !path.contains('\n')
            && !path.contains('\'')
            && is_executable_file(Path::new(&format!("{path}/activate")))
    }

    /// Step 2 of `darwin-rebuild switch`, as root: set the system profile and activate (as darwin-rebuild does).
    pub fn activate_command(system: &str) -> String {
        let s = shell_quote(system);
        format!("nix-env -p /nix/var/nix/profiles/system --set {s} && {s}/activate")
    }

    // MARK: Setup by click (spec §11.5)

    /// `OMAFLAKE_NIX_INSTALLER_URL`, else the pinned release's aarch64-darwin binary on GitHub
    pub fn nix_installer_url() -> String {
        std::env::var("OMAFLAKE_NIX_INSTALLER_URL").unwrap_or_else(|_| {
            format!(
                "https://github.com/NixOS/nix-installer/releases/download/{}/nix-installer-aarch64-darwin",
                Self::NIX_INSTALLER_VERSION
            )
        })
    }

    /// Downloads the installer with curl into a temporary folder, checks its SHA-256 with `shasum -a 256 -c -`, runs
    /// `install --no-confirm` (Swift `installNixCommand`)
    pub fn install_nix_command() -> String {
        format!(
            "set -e; d=$(mktemp -d); curl -fsSL -o \"$d/nix-installer\" {}; \
             echo \"{}  $d/nix-installer\" | shasum -a 256 -c -; \
             chmod +x \"$d/nix-installer\"; \"$d/nix-installer\" install --no-confirm",
            shell_quote(&Self::nix_installer_url()),
            Self::NIX_INSTALLER_SHA256
        )
    }

    /// First activation of nix-darwin: it refuses to take over /etc files it did not write (the Nix installer's
    /// nix.conf and shell rc files) – move them aside as docs/linux-builder.md says (rename, never delete), then
    /// set the system profile and activate.
    pub fn first_activation_command(system: &str) -> String {
        let move_aside = ["/etc/nix/nix.conf", "/etc/bashrc", "/etc/zshrc", "/etc/zshenv"]
            .map(|f| format!("if [ -f {f} ] && [ ! -L {f} ]; then mv {f} {f}.before-nix-darwin; fi"))
            .join("; ");
        format!("{move_aside}; {}", Self::activate_command(system))
    }

    /// Wraps a command for a root shell: Nix tools on PATH, root's HOME, output appended to `log` (created by the
    /// user first, so it stays theirs); a header line "=== <ISO 8601 time> <command>"; `set -o pipefail`.
    pub fn root_script(&self, command: &str) -> Result<String> {
        let logs = self.paths.logs();
        fs::create_dir_all(&logs).with_path(&logs)?;
        let log = self.log();
        if !log.exists() {
            // Swift `createFile` ignored a failure; so does this
            let _ = fs::File::create(&log);
        }
        let header = format!("=== {} {command}", json::iso8601::format(&json::now()));
        let log = shell_quote(&log.to_string_lossy());
        // pipefail: the command's failure, not tee's success, is the script's result
        Ok(format!(
            "set -o pipefail; export HOME=/var/root \
             PATH=/run/current-system/sw/bin:/nix/var/nix/profiles/default/bin:/usr/bin:/bin:/usr/sbin:/sbin; \
             echo {} >> {log}; {{ {command} ; }} 2>&1 | tee -a {log}",
            shell_quote(&header)
        ))
    }
}

/// Runs a shell command as root after macOS' administrator dialog (AppleScript `do shell script … with administrator
/// privileges`). The command travels as an argument, never spliced into AppleScript source.
pub enum AdminShell {}

impl AdminShell {
    /// Blocks until the command ends; returns its output (`\r` turned into `\n`). The user cancelling the dialog
    /// (osascript "(-128)") is an error with code `codes::CANCELLED` (Swift `AdminShell.Cancelled`).
    /// macOS only; elsewhere `Error::unsupported`.
    pub fn run(command: &str, prompt: &str) -> Result<String> {
        #[cfg(target_os = "macos")]
        {
            let args = Self::arguments(command, prompt);
            let r = Shell::run("/usr/bin/osascript", &args, &crate::shell::RunOptions::default())?;
            Self::outcome(r.status, &r.stdout, &r.stderr)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let _ = (command, prompt);
            Err(Error::unsupported("The administrator dialog"))
        }
    }

    /// osascript's arguments: a fixed script that takes the command and the prompt from `argv`
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // used on macOS only
    pub(crate) fn arguments(command: &str, prompt: &str) -> Vec<String> {
        [
            "-e",
            "on run argv",
            "-e",
            "do shell script (item 1 of argv) with prompt (item 2 of argv) with administrator privileges",
            "-e",
            "end run",
            command,
            prompt,
        ]
        .map(String::from)
        .to_vec()
    }

    /// osascript's result: its output, "Cancelled" for "User canceled." (-128), else its error text
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // used on macOS only
    pub(crate) fn outcome(status: i32, stdout: &str, stderr: &str) -> Result<String> {
        if status != 0 {
            // -128: "User canceled."
            if stderr.contains("(-128)") {
                return Err(Error::cancelled());
            }
            return Err(Error::new(stderr.trim()));
        }
        Ok(stdout.replace('\r', "\n"))
    }
}

/// The last `n` characters of `s` (Swift `suffix(n)`)
fn suffix(s: &str, n: usize) -> &str {
    let count = s.chars().count();
    match s.char_indices().nth(count.saturating_sub(n)) {
        Some((i, _)) if count > n => &s[i..],
        _ => s,
    }
}

/// Single-quotes a word for /bin/sh.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;

    #[test]
    fn shell_quoting() {
        assert_eq!(shell_quote("a b"), "'a b'");
        assert_eq!(shell_quote("it's"), "'it'\\''s'");
    }

    #[test]
    fn activate_and_root_script() {
        let (_d, paths) = temp_paths();
        let mut settings = Settings::load(&paths);
        settings.flake = Some("/src/omaflake/nix".into());
        settings.save(&paths).unwrap();
        assert_eq!(
            BuilderMaintenance::activate_command("/nix/store/abc-darwin-system"),
            "nix-env -p /nix/var/nix/profiles/system --set '/nix/store/abc-darwin-system' && \
             '/nix/store/abc-darwin-system'/activate"
        );
        assert!(!BuilderMaintenance::is_system("/tmp/x"));
        assert!(!BuilderMaintenance::is_system("/nix/store/it's"));
        let script = BuilderMaintenance::new(paths.clone()).root_script("true").unwrap();
        assert!(script.starts_with("set -o pipefail; export HOME=/var/root"));
        assert!(paths.root.join("logs/linux-builder.log").exists());
        let log = shell_quote(&paths.root.join("logs/linux-builder.log").to_string_lossy());
        assert!(script.ends_with(&format!("{{ true ; }} 2>&1 | tee -a {log}")), "{script}");
        assert!(script.contains("PATH=/run/current-system/sw/bin:/nix/var/nix/profiles/default/bin:/usr/bin:/bin:/usr/sbin:/sbin; echo '=== 20"));
        assert!(script.contains(&format!(" true' >> {log}; ")));
    }

    #[test]
    fn first_activation_moves_unmanaged_etc_files_aside() {
        let cmd = BuilderMaintenance::first_activation_command("/nix/store/x-darwin-system");
        assert!(cmd.contains(
            "if [ -f /etc/nix/nix.conf ] && [ ! -L /etc/nix/nix.conf ]; then mv /etc/nix/nix.conf \
             /etc/nix/nix.conf.before-nix-darwin; fi"
        ));
        assert!(cmd.ends_with(
            "nix-env -p /nix/var/nix/profiles/system --set '/nix/store/x-darwin-system' && \
             '/nix/store/x-darwin-system'/activate"
        ));
        assert!(!cmd.contains("rm ")); // rename, never delete
    }

    #[test]
    fn installer_is_checked_before_it_runs() {
        let cmd = BuilderMaintenance::install_nix_command();
        let check = cmd.find("shasum -a 256 -c -").unwrap();
        let run = cmd.find("install --no-confirm").unwrap();
        assert!(check < run);
        assert!(cmd.starts_with("set -e;")); // a wrong checksum stops the script
        assert!(
            cmd.contains(&format!(
                "/releases/download/{}/nix-installer-aarch64-darwin",
                BuilderMaintenance::NIX_INSTALLER_VERSION
            )) || std::env::var("OMAFLAKE_NIX_INSTALLER_URL").is_ok()
        );
        assert!(cmd.contains(&format!("echo \"{}  $d/nix-installer\"", BuilderMaintenance::NIX_INSTALLER_SHA256)));
    }

    #[test]
    fn admin_shell_passes_the_command_as_an_argument() {
        let args = AdminShell::arguments("echo 'a\"b'", "omaflake wants to");
        assert_eq!(args.len(), 8);
        assert_eq!(
            args[3],
            "do shell script (item 1 of argv) with prompt (item 2 of argv) with administrator privileges"
        );
        assert_eq!(&args[6..], ["echo 'a\"b'", "omaflake wants to"]);
        // the command never appears in the AppleScript source
        assert!(args[..6].iter().all(|a| !a.contains("echo")));
    }

    #[test]
    fn admin_shell_outcome() {
        assert_eq!(AdminShell::outcome(0, "a\rb", "").unwrap(), "a\nb");
        let cancelled = AdminShell::outcome(1, "", "0:48: execution error: User canceled. (-128)\n").unwrap_err();
        assert!(cancelled.is_cancelled());
        let failed = AdminShell::outcome(1, "", "  boom (1)\n").unwrap_err();
        assert_eq!((failed.message(), failed.code()), ("boom (1)", None));
    }

    #[test]
    fn suffix_counts_characters() {
        assert_eq!(suffix("abc", 5), "abc");
        assert_eq!(suffix("abcdef", 2), "ef");
        assert_eq!(suffix("–ab", 2), "ab");
        assert_eq!(suffix("x–ab", 3), "–ab");
    }
}
