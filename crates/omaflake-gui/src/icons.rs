// SPDX-License-Identifier: AGPL-3.0-or-later
//! The icon vocabulary of the views (docs/api.md §10.3) drawn with Lucide (ISC, see NOTICE): each name a glyph of
//! the bundled Lucide font. An unknown name is drawn as nothing (the UI shows the label instead).

use iced::widget::{Text, text};
use iced::{Color, Font};
use lucide_icons::Icon;

/// The Lucide font's family name (its bytes are loaded at start, `FONT_BYTES`)
pub const FONT: Font = Font::with_name("lucide");
pub const FONT_BYTES: &[u8] = lucide_icons::LUCIDE_FONT_BYTES;

/// The Lucide icon for a vocabulary name (and the UI's own: `chevron-*`, `info`, `check`, `dot`)
pub fn lucide(name: &str) -> Option<Icon> {
    Some(match name {
        "vm" => Icon::Monitor,
        "image" => Icon::Package,
        "recipe" => Icon::BookOpen,
        "activity" => Icon::Clock,
        "look" => Icon::Palette,
        "module" => Icon::Blocks,
        "gauge" => Icon::Gauge,
        "copy" => Icon::Copy,
        "edit" => Icon::Pencil,
        "license" => Icon::Scale,
        "update" => Icon::CircleArrowUp,
        "nixos" => Icon::Snowflake,
        "macos" => Icon::Apple,
        "start-window" => Icon::MonitorPlay,
        "start-headless" => Icon::SquareTerminal,
        "stop" => Icon::Square,
        "terminal" => Icon::Terminal,
        "folder" => Icon::Folder,
        "export" => Icon::Upload,
        "import" => Icon::Download,
        "external" => Icon::HardDrive,
        "hide" => Icon::EyeOff,
        "upgrade" | "upgradable" => Icon::CircleArrowUp,
        "reset" => Icon::RotateCcw,
        "trash" => Icon::Trash2,
        "build" => Icon::Hammer,
        "new-vm" => Icon::CirclePlus,
        "create" => Icon::Plus,
        "cancel" => Icon::X,
        "install" => Icon::HardDriveDownload,
        "refresh" => Icon::RefreshCw,
        "download" => Icon::Download,
        "pause" => Icon::Pause,
        "apply" => Icon::Check,
        "clean" => Icon::Brush,
        "guide" => Icon::BookMarked,
        "log" => Icon::ScrollText,
        "doctor" => Icon::Stethoscope,
        "settings" => Icon::Settings,
        "key" => Icon::KeyRound,
        "workshop" => Icon::Wrench,
        "studio" => Icon::Layers,
        "source" => Icon::Inbox,
        "minus" => Icon::CircleMinus,
        "keyboard" => Icon::Keyboard,
        "help" => Icon::CircleQuestionMark,
        "globe" => Icon::Globe,
        "quit" => Icon::Power,
        "up" => Icon::ChevronUp,
        "add" => Icon::Plus,
        "clipboard" => Icon::Clipboard,
        "sound" => Icon::Volume2,
        "microphone" => Icon::Mic,
        "camera" => Icon::Camera,
        "ephemeral" => Icon::Sparkles,
        "network-isolated" => Icon::ShieldCheck,
        "network-none" => Icon::WifiOff,
        "outdated" => Icon::RefreshCw,
        "lab" => Icon::FlaskConical,
        "arch" => Icon::Cpu,
        "official" => Icon::BadgeCheck,
        "lock" => Icon::Lock,
        "lock-warning" => Icon::ShieldAlert,
        // Lucide dropped brand logos; a repository is a branch
        "github" => Icon::GitBranch,
        "url" => Icon::Globe,
        "warning" => Icon::TriangleAlert,
        "ok" | "finished" => Icon::CircleCheck,
        "filter" => Icon::Funnel,
        "search" => Icon::Search,
        "failed" => Icon::CircleX,
        "cancelled" => Icon::Ban,
        // The UI's own
        "chevron-left" => Icon::ChevronLeft,
        "chevron-right" => Icon::ChevronRight,
        "chevron-down" => Icon::ChevronDown,
        "info" => Icon::Info,
        "check" => Icon::Check,
        "play" => Icon::Play,
        "window" => Icon::AppWindow,
        _ => return None,
    })
}

/// The glyph of a vocabulary name
pub fn glyph(name: &str) -> Option<char> {
    lucide(name).map(char::from)
}

/// An icon as text in the Lucide font; an unknown name is an empty text
pub fn icon<'a>(name: &str, size: f32, color: Color) -> Text<'a> {
    text(glyph(name).map(String::from).unwrap_or_default()).font(FONT).size(size).color(color)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every name of api.md §10.3's list has a glyph
    #[test]
    fn the_whole_vocabulary_is_mapped() {
        let documented = "vm image recipe activity nixos macos start-window start-headless stop terminal folder export \
            import external hide upgrade reset trash build new-vm create cancel install refresh download pause apply \
            clean guide log doctor settings quit up add clipboard sound microphone camera ephemeral network-isolated \
            network-none upgradable outdated lab arch official lock lock-warning github url warning ok filter finished \
            failed cancelled";
        for name in documented.split_whitespace() {
            assert!(glyph(name).is_some(), "{name}");
        }
        assert_eq!(glyph("no-such-icon"), None);
        assert_ne!(glyph("stop"), glyph("start-window"));
    }

    /// The icons the golden page models use are all known
    #[test]
    fn the_golden_models_use_known_icons() {
        for (file, json) in crate::pages::tests::golden_files() {
            let mut names = Vec::new();
            collect_icons(&json, &mut names);
            for n in names {
                assert!(glyph(&n).is_some(), "{file}: {n}");
            }
        }
    }

    fn collect_icons(v: &serde_json::Value, out: &mut Vec<String>) {
        match v {
            serde_json::Value::Object(m) => {
                for (k, v) in m {
                    if k == "icon"
                        && let Some(s) = v.as_str()
                    {
                        out.push(s.to_string());
                    }
                    collect_icons(v, out);
                }
            }
            serde_json::Value::Array(a) => a.iter().for_each(|v| collect_icons(v, out)),
            _ => {}
        }
    }
}
