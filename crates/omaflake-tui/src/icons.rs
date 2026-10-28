// SPDX-License-Identifier: AGPL-3.0-or-later
//! The views' icon vocabulary (docs/api.md §10.3) as terminal symbols: Nerd Font glyphs when the terminal's font has
//! them (`OMAF_NERD_FONT=1` or `omaf ui --nerd-font`; a font cannot be detected from inside the terminal), else plain
//! Unicode characters every monospaced font has. An unknown name is drawn as nothing.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IconSet {
    Plain,
    NerdFont,
}

impl IconSet {
    /// From `OMAF_NERD_FONT` (`1`, `true`, `yes` → Nerd Font)
    pub fn detect() -> IconSet {
        match std::env::var("OMAF_NERD_FONT").ok().as_deref().map(str::to_ascii_lowercase).as_deref() {
            Some("1" | "true" | "yes" | "on") => IconSet::NerdFont,
            _ => IconSet::Plain,
        }
    }

    /// The symbol of `name`; "" for a name it does not know
    pub fn get(self, name: &str) -> &'static str {
        let (nerd, plain) = match name {
            "vm" => ("\u{f0379}", "▣"),
            "image" => ("\u{f0a0}", "◫"),
            "recipe" => ("\u{f02d}", "≡"),
            "activity" => ("\u{f0f3}", "≋"),
            "look" => ("\u{f53f}", "◈"),
            "module" => ("\u{f1b3}", "▦"),
            "gauge" => ("\u{f0e4}", "◔"),
            "copy" => ("\u{f0c5}", "⧉"),
            "edit" => ("\u{f044}", "✎"),
            "license" => ("\u{f24e}", "⚖"),
            "nixos" => ("\u{f313}", "❄"),
            "macos" => ("\u{f179}", "⌘"),
            "start-window" => ("\u{f04b}", "▶"),
            "start-headless" => ("\u{f01d}", "▷"),
            "stop" => ("\u{f04d}", "■"),
            "terminal" => ("\u{f120}", "›_"),
            "folder" => ("\u{f07c}", "▤"),
            "export" => ("\u{f093}", "⇧"),
            "import" => ("\u{f090}", "⇩"),
            "external" => ("\u{f08e}", "↗"),
            "hide" => ("\u{f070}", "⊖"),
            "upgrade" | "upgradable" => ("\u{f0aa}", "⇡"),
            "reset" => ("\u{f0e2}", "↺"),
            "trash" => ("\u{f1f8}", "✕"),
            "build" => ("\u{f0ad}", "⚒"),
            "new-vm" => ("\u{f0fe}", "⊞"),
            "create" | "ok" => ("\u{f00c}", "✓"),
            "cancel" => ("\u{f00d}", "✕"),
            "install" => ("\u{f019}", "↓"),
            "refresh" => ("\u{f021}", "⟳"),
            "download" => ("\u{f0ed}", "↓"),
            "pause" => ("\u{f04c}", "‖"),
            "apply" | "finished" => ("\u{f058}", "✓"),
            "clean" => ("\u{f00e2}", "⌫"),
            "guide" => ("\u{f059}", "?"),
            "log" => ("\u{f0f6}", "≣"),
            "doctor" => ("\u{f0f1}", "✚"),
            "settings" => ("\u{f013}", "⚙"),
            "key" => ("\u{f084}", "⚷"),
            "workshop" => ("\u{f0ad}", "⚒"),
            "studio" => ("\u{f5fd}", "▤"),
            "source" => ("\u{f01c}", "⊟"),
            "minus" => ("\u{f056}", "−"),
            "keyboard" => ("\u{f11c}", "⌨"),
            "globe" => ("\u{f0ac}", "◍"),
            "quit" => ("\u{f011}", "⏻"),
            "up" => ("\u{f062}", "↑"),
            "add" => ("\u{f067}", "+"),
            "clipboard" => ("\u{f0ea}", "⎘"),
            "sound" => ("\u{f028}", "♪"),
            "microphone" => ("\u{f130}", "◖"),
            "camera" => ("\u{f030}", "◎"),
            "ephemeral" => ("\u{f252}", "⧗"),
            "network-isolated" => ("\u{f132}", "⛨"),
            "network-none" => ("\u{f0318}", "⊘"),
            "outdated" => ("\u{f017}", "◷"),
            "lab" => ("\u{f0c3}", "⚗"),
            "arch" => ("\u{f2db}", "⌬"),
            "official" => ("\u{f0a3}", "✓"),
            "lock" => ("\u{f023}", "⚿"),
            "lock-warning" => ("\u{f13e}", "⚿"),
            "github" => ("\u{f09b}", "⌥"),
            "url" => ("\u{f0ac}", "⊕"),
            "warning" => ("\u{f071}", "⚠"),
            "filter" => ("\u{f0b0}", "⊻"),
            "failed" => ("\u{f057}", "✗"),
            "cancelled" => ("\u{f05e}", "⊘"),
            _ => ("", ""),
        };
        match self {
            IconSet::NerdFont => nerd,
            IconSet::Plain => plain,
        }
    }
}

/// Every name of the vocabulary (api.md §10.3)
pub const VOCABULARY: &[&str] = &[
    "vm",
    "image",
    "recipe",
    "activity",
    "nixos",
    "macos",
    "start-window",
    "start-headless",
    "stop",
    "terminal",
    "folder",
    "export",
    "import",
    "external",
    "hide",
    "upgrade",
    "reset",
    "trash",
    "build",
    "new-vm",
    "create",
    "cancel",
    "install",
    "refresh",
    "download",
    "pause",
    "apply",
    "clean",
    "guide",
    "log",
    "doctor",
    "settings",
    "key",
    "globe",
    "look",
    "quit",
    "up",
    "add",
    "clipboard",
    "sound",
    "microphone",
    "camera",
    "ephemeral",
    "network-isolated",
    "network-none",
    "upgradable",
    "outdated",
    "lab",
    "arch",
    "official",
    "lock",
    "lock-warning",
    "github",
    "url",
    "warning",
    "ok",
    "filter",
    "finished",
    "failed",
    "cancelled",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_name_has_a_symbol_in_both_sets() {
        for name in VOCABULARY {
            assert!(!IconSet::Plain.get(name).is_empty(), "{name}");
            assert!(!IconSet::NerdFont.get(name).is_empty(), "{name}");
        }
        assert_eq!(IconSet::Plain.get("no-such-icon"), "");
    }
}
