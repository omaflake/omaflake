// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of BuildProgress.swift.

use std::sync::Mutex;

use crate::models::GuestKind;

/// Progress of an image build, read from its log lines: the step ("2 of 4") and, while Nix builds or downloads,
/// how many of the announced packages are done. Both build paths log through here (CLI and app).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BuildProgress {
    pub step: u32,
    pub total: u32,
    pub title: String,
    /// Packages done / announced in the current step (Nix: "these N derivations will be built", "… paths will be fetched")
    pub done: u32,
    pub of: u32,
}

impl BuildProgress {
    pub fn new(step: u32, total: u32, title: &str) -> Self {
        BuildProgress { step, total, title: title.into(), done: 0, of: 0 }
    }

    /// 0…1 over all steps; the current step counts by its packages when Nix announced some
    pub fn fraction(&self) -> f64 {
        let in_step = if self.of > 0 { (self.done as f64 / self.of as f64).min(1.0) } else { 0.0 };
        ((self.step as f64 - 1.0 + in_step) / self.total as f64).min(1.0)
    }
}

/// Turns build log lines into `BuildProgress`. Steps follow the log messages of `ImageStore::build` (NixOS) and
/// `MacImageBuilder::build` (macOS); Nix's own lines give the package counts. Thread-safe (`feed` takes `&self`).
#[derive(Debug)]
pub struct BuildProgressTracker {
    progress: Mutex<BuildProgress>,
    kind: GuestKind,
}

impl BuildProgressTracker {
    /// macOS steps: the builder's log message that starts each one (prefix, title)
    pub const MAC_STEPS: [(&'static str, &'static str); 7] = [
        ("Building darwinConfigurations", "Building the macOS system on the Mac"),
        ("Cloning ", "Cloning the macOS base"),
        ("Waiting for the VM", "Starting the build VM"),
        ("Installing Nix", "Installing Nix in the guest"),
        ("Copying the closure", "Copying the system into the guest"),
        ("Activating", "Activating the system"),
        ("Shutting down", "Shutting down and storing the image"),
    ];

    /// NixOS: step 1 of 4 "Evaluating the recipe"; macOS: step 1 of 7 "Building the macOS system on the Mac"
    pub fn new(kind: GuestKind) -> Self {
        let progress = match kind {
            GuestKind::Nixos => BuildProgress::new(1, 4, "Evaluating the recipe"),
            GuestKind::Macos => BuildProgress::new(1, 7, "Building the macOS system on the Mac"),
        };
        BuildProgressTracker { progress: Mutex::new(progress), kind }
    }

    pub fn progress(&self) -> BuildProgress {
        self.lock().clone()
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, BuildProgress> {
        self.progress.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Reads one log line; returns true when the progress changed.
    pub fn feed(&self, line: &str) -> bool {
        let mut p = self.lock();
        let before = p.clone();
        Self::count_packages(&mut p, self.kind, line);
        match self.kind {
            GuestKind::Nixos => Self::nixos_step(&mut p, line),
            GuestKind::Macos => Self::mac_step(&mut p, line),
        }
        *p != before
    }

    /// A later step (never back), its package counts start over
    fn set(p: &mut BuildProgress, step: u32, title: &str) {
        if step > p.step {
            *p = BuildProgress::new(step, p.total, title);
        }
    }

    fn nixos_step(p: &mut BuildProgress, line: &str) {
        if line.contains("nixos-disk-image>") {
            Self::set(p, 3, "Writing the disk image");
        } else if line.contains("copying path") && line.contains("nixos-disk-image") && line.contains("from 'ssh") {
            Self::set(p, 4, "Copying the image from the Linux builder");
        } else if p.step == 1 && (line.starts_with("building '") || Self::announced(line).is_some()) {
            Self::set(p, 2, "Building packages");
        }
    }

    fn mac_step(p: &mut BuildProgress, line: &str) {
        for (i, (prefix, title)) in Self::MAC_STEPS.iter().enumerate() {
            if line.starts_with(prefix) {
                Self::set(p, i as u32 + 1, title);
            }
        }
    }

    /// "these 57 derivations will be built:", "this derivation will be built:", "these 12 paths will be fetched (…)"
    pub(crate) fn announced(line: &str) -> Option<u32> {
        let l = trim_whitespaces(line);
        if l.starts_with("this derivation will be built") || l.starts_with("this path will be fetched") {
            return Some(1);
        }
        if !l.starts_with("these ") || !(l.contains(" will be built") || l.contains(" will be fetched")) {
            return None;
        }
        let digits: String = l[6..].chars().take_while(|c| c.is_ascii_digit()).collect();
        digits.parse().ok()
    }

    fn count_packages(p: &mut BuildProgress, kind: GuestKind, line: &str) {
        if let Some(n) = Self::announced(line) {
            p.of += n;
            if kind == GuestKind::Nixos && p.step == 1 {
                Self::set(p, 2, "Building packages");
                p.of = n;
            }
            return;
        }
        // One finished unit: a build starting (Nix prints it once per derivation) or a path downloaded
        if line.starts_with("building '/nix/store/")
            || (line.starts_with("copying path '") && line.contains("from 'https://"))
        {
            p.done = (p.done + 1).min(p.of.max(p.done + 1));
        }
    }
}

/// Lines of a build log that say nothing on their own: the disk image tool reports >100 000 "error while reading
/// directory" lines and one "copying path" per store path on every successful NixOS build; Nix lists and downloads
/// thousands of packages one line each – the progress counts those (BuildProgressTracker sees every line). They are
/// left out of what the app and the CLI show; the full log keeps them (plan B83).
pub enum BuildLog {}

impl BuildLog {
    pub fn is_noise(line: &str) -> bool {
        if line.contains("error while reading directory") {
            return true;
        }
        if line.starts_with("copying path '") || line.starts_with("  /nix/store/") {
            return true;
        }
        if let Some(rest) = line.strip_prefix("nixos-disk-image>") {
            let rest = trim_whitespaces(rest);
            return rest.is_empty() || rest.starts_with("copying path") || rest.starts_with("/nix/store/");
        }
        false
    }
}

/// Swift `trimmingCharacters(in: .whitespaces)`: spaces and tabs, not line breaks
fn trim_whitespaces(s: &str) -> &str {
    s.trim_matches(|c: char| {
        c.is_whitespace() && !matches!(c, '\n' | '\r' | '\u{0B}' | '\u{0C}' | '\u{85}' | '\u{2028}' | '\u{2029}')
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nixos_steps_and_package_counts() {
        let t = BuildProgressTracker::new(GuestKind::Nixos);
        assert!(t.progress().step == 1 && t.progress().total == 4);
        assert!(t.feed("these 3 derivations will be built:"));
        t.feed("these 2 paths will be fetched (1.2 MiB download, 5 MiB unpacked):");
        assert!(t.progress().step == 2 && t.progress().of == 5);
        t.feed("building '/nix/store/aaa-x.drv'...");
        t.feed("copying path '/nix/store/bbb-y' from 'https://cache.nixos.org'...");
        assert_eq!(t.progress().done, 2);
        t.feed("nixos-disk-image> copying the closure");
        assert!(t.progress().step == 3 && t.progress().done == 0);
        t.feed("copying path '/nix/store/ccc-nixos-disk-image' from 'ssh-ng://builder@linux-builder'...");
        assert_eq!(t.progress().step, 4);
        let f = t.progress().fraction();
        assert!(f > 0.74 && f <= 1.0);
        assert!(!t.feed("nothing to see"));
    }

    #[test]
    fn macos_steps_follow_the_builder_log() {
        let t = BuildProgressTracker::new(GuestKind::Macos);
        assert_eq!(t.progress().total, 7);
        t.feed("Cloning mac-base-26A434 (log: /x)");
        assert_eq!(t.progress().step, 2);
        t.feed("Activating");
        assert_eq!(t.progress().step, 6);
        t.feed("Cloning again"); // never goes back
        assert_eq!(t.progress().step, 6);
    }

    #[test]
    fn announcements_and_fraction() {
        assert_eq!(BuildProgressTracker::announced("  these 57 derivations will be built:"), Some(57));
        assert_eq!(BuildProgressTracker::announced("this derivation will be built:"), Some(1));
        assert_eq!(BuildProgressTracker::announced("this path will be fetched (1 MiB):"), Some(1));
        assert_eq!(BuildProgressTracker::announced("these paths will be fetched"), None);
        assert_eq!(BuildProgressTracker::announced("building '/nix/store/x.drv'"), None);
        let mut p = BuildProgress::new(2, 4, "x");
        assert_eq!(p.fraction(), 0.25);
        p.of = 4;
        p.done = 8;
        assert_eq!(p.fraction(), 0.5);
    }

    #[test]
    fn routine_lines_are_noise() {
        assert!(BuildLog::is_noise(
            "nixos-disk-image> error while reading directory /build/root/nix/store/x/share/locale/et: No such file or directory"
        ));
        assert!(BuildLog::is_noise("nixos-disk-image>   "));
        assert!(BuildLog::is_noise("nixos-disk-image> copying path '/nix/store/abc-foo' to 'local'..."));
        assert!(!BuildLog::is_noise("nixos-disk-image> Disk /dev/vda: 12 GiB"));
        assert!(!BuildLog::is_noise("error: build of '/nix/store/x.drv' failed"));
        assert!(BuildLog::is_noise("copying path '/nix/store/abc' from 'https://cache.nixos.org'..."));
        assert!(BuildLog::is_noise("  /nix/store/abc-foo-1.0"));
        assert!(BuildLog::is_noise("nixos-disk-image>   /nix/store/abc-perl-env"));
        assert!(!BuildLog::is_noise("these 57 paths will be fetched (1.2 GiB download):"));
    }
}
