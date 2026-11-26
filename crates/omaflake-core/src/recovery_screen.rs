// SPDX-License-Identifier: AGPL-3.0-or-later
//! What macOS Recovery shows in a VM, read from the text on its screen (plan B184): the runner's `screen` gives each
//! line with its place, this module says which screen it is and what to do there. Plain functions over those
//! lines, tested with what Recovery showed on a Mac (2026-11-26, a German image and a base) – the steps that act on
//! them are `mac_base_unchain.rs`'s.
//!
//! Words in English and German: a base's Recovery is English (or asks for a language), an image's is its language;
//! omaflake sets it to English with a US keyboard before it starts Recovery (`prev-lang:kbd`), the German words are
//! for when that did not take.

use serde::Deserialize;

/// One line of text on the screen: its middle, both 0…1 from the top left
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Line {
    pub text: String,
    pub x: f64,
    pub y: f64,
}

impl Line {
    #[cfg(test)]
    pub(crate) fn new(text: &str, x: f64, y: f64) -> Line {
        Line { text: text.into(), x, y }
    }
}

/// A point to click
pub type Point = (f64, f64);

/// The keyboard Recovery reads, for typing (the runner's `type {layout}`)
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Layout {
    Us,
    De,
}

impl Layout {
    pub fn as_str(self) -> &'static str {
        match self {
            Layout::Us => "us",
            Layout::De => "de",
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum Screen {
    /// Two large icons, the disk and "Options": click Options (`options`, the icon above its name), then Continue
    StartupOptions {
        options: Point,
        continue_at: Option<Point>,
        layout: Option<Layout>,
    },
    /// A list of languages: click English (or the one there is), then Return
    Language {
        pick: Option<Point>,
    },
    /// Recovery asks for a user it knows the password of: click it (`user`), then type the password
    User {
        user: Option<Point>,
    },
    /// The utilities window: Terminal through ⇧⌘T, else through the menu (`menu`, "Utilities" in the menu bar)
    Utilities {
        menu: Option<Point>,
    },
    /// An open menu that has "Terminal"
    Menu {
        terminal: Point,
    },
    /// Terminal: its text below the menu bar, top to bottom
    Terminal {
        text: Vec<String>,
    },
    Unknown,
}

fn find<'a>(lines: &'a [Line], words: &[&str]) -> Option<&'a Line> {
    lines.iter().find(|l| words.iter().any(|w| l.text.to_lowercase().contains(&w.to_lowercase())))
}

fn find_in<'a>(lines: &'a [Line], words: &[&str], top: f64) -> Option<&'a Line> {
    lines
        .iter()
        .filter(|l| l.y <= top)
        .find(|l| words.iter().any(|w| l.text.to_lowercase().contains(&w.to_lowercase())))
}

/// The menu bar: everything above this
const MENU_BAR: f64 = 0.05;

pub fn classify(lines: &[Line], user: &str, menu_open: bool) -> Screen {
    // Terminal: the shell's prompt, and "Shell" in the menu bar
    if find(lines, &["bash-3", "-bash", "—bash", "root#", "# csrutil"]).is_some()
        && find_in(lines, &["Shell"], MENU_BAR).is_some()
    {
        let text = lines.iter().filter(|l| l.y > MENU_BAR).map(|l| l.text.clone()).collect();
        return Screen::Terminal { text };
    }
    // An open Utilities menu: "Terminal" below the menu bar, near the menu's title
    if menu_open && let Some(t) = lines.iter().find(|l| l.y > MENU_BAR && l.y < 0.4 && l.text.trim() == "Terminal") {
        return Screen::Menu { terminal: (t.x, t.y) };
    }
    // Startup options: the disk and "Options" side by side, Shut Down and Restart below
    if let Some(options) = find(lines, &["Options", "Optionen"])
        && find(lines, &["Shut Down", "Ausschalten", "Macintosh HD"]).is_some()
    {
        let continue_at = find(lines, &["Continue", "Fortfahren"]).map(|l| (l.x, l.y));
        // The input menu's name, top right: "Deutsch", "ABC", "U.S."
        let layout = lines.iter().filter(|l| l.y < MENU_BAR && l.x > 0.7).find_map(|l| {
            let word = l.text.split_whitespace().next().unwrap_or_default().to_lowercase();
            match word.as_str() {
                "deutsch" | "german" => Some(Layout::De),
                "abc" | "u.s." | "us" | "english" => Some(Layout::Us),
                _ => None,
            }
        });
        return Screen::StartupOptions {
            // The icon sits above its name
            options: (options.x, (options.y - 0.07).max(0.0)),
            continue_at,
            layout,
        };
    }
    // The utilities window
    if find(lines, &["Disk Utility", "Festplattendienstprogramm", "Time Machine", "Reinstall macOS"]).is_some() {
        let menu = find_in(lines, &["Utilities", "Dienstprogramme"], MENU_BAR).map(|l| (l.x, l.y));
        return Screen::Utilities { menu };
    }
    // A user to choose (Recovery wants one it knows the password of)
    if find(lines, &["Select a user", "Wähle einen Benutzer", "know the password", "Passwort kennst"]).is_some() {
        let user = lines.iter().find(|l| l.text.trim().eq_ignore_ascii_case(user)).map(|l| (l.x, l.y));
        return Screen::User { user };
    }
    // The language list: several language names one below the other
    const LANGUAGES: [&str; 8] =
        ["English", "Deutsch", "Français", "Español", "Italiano", "日本語", "Nederlands", "Português"];
    let names: Vec<&Line> = lines.iter().filter(|l| LANGUAGES.contains(&l.text.trim())).collect();
    if names.len() >= 3 {
        let pick = names.iter().find(|l| l.text.trim() == "English").map(|l| (l.x, l.y));
        return Screen::Language { pick };
    }
    Screen::Unknown
}

/// What `csrutil disable` waits for, from the Terminal's last lines
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Prompt {
    /// "… [y/n]"
    YesNo,
    /// A user name
    User,
    /// A password
    Password,
    /// It said what happened: done (it asks to restart) – or an error
    Done,
    Failed,
    /// Nothing to answer yet
    Waiting,
}

/// The prompt in the Terminal's text: a question only in its last line (one answered stands above it); what
/// happened in its last two
pub fn csrutil_prompt(text: &[String]) -> Prompt {
    let tail: String = text.iter().rev().take(2).rev().cloned().collect::<Vec<_>>().join(" ").to_lowercase();
    let last = text.last().map(|l| l.to_lowercase()).unwrap_or_default();
    if tail.contains("restart") || tail.contains("neustart") || tail.contains("successfully") {
        Prompt::Done
    } else if tail.contains("error") || tail.contains("not authorized") || tail.contains("failed") {
        Prompt::Failed
    } else if last.contains("y/n") {
        Prompt::YesNo
    } else if last.contains("password") || last.contains("passwort") {
        Prompt::Password
    } else if last.contains("user") || last.contains("benutzer") {
        Prompt::User
    } else {
        Prompt::Waiting
    }
}

/// The probe `echo SHIFTOK` answered in capitals: Shift reaches the guest (a line that is just "SHIFTOK" – the
/// command line has "echo" before it, a guest without Shift answers "shiftok")
pub fn shift_works(text: &[String]) -> bool {
    text.iter().any(|l| l.trim() == "SHIFTOK")
}

/// A character that is typed with Shift held, on this layout (capitals, and the symbols on the number row …)
pub fn needs_shift(c: char, layout: Layout) -> bool {
    if c.is_uppercase() {
        return true;
    }
    let shifted: &str = match layout {
        Layout::Us => "~!@#$%^&*()_+{}|:\"<>?",
        Layout::De => "!\"§$%&/()=?*'_;:>",
    };
    shifted.contains(c)
}

/// "System Integrity Protection status: …" in the Terminal's text: Some(true) when disabled
pub fn protection_off(text: &[String]) -> Option<bool> {
    let line = text.iter().rfind(|l| l.contains("System Integrity Protection status"))?;
    Some(line.to_lowercase().contains("disabled"))
}

/// `nvram prev-lang:kbd` → its value ("de-DE:3"); None when it is not set
pub fn nvram_value(output: &str) -> Option<String> {
    let value = output.lines().find_map(|l| l.strip_prefix("prev-lang:kbd"))?.trim();
    (!value.is_empty()).then(|| value.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn l(text: &str, x: f64, y: f64) -> Line {
        Line::new(text, x, y)
    }

    /// The startup options of a German image, as read on the Mac (2026-11-26)
    fn startup_options(with_continue: bool) -> Vec<Line> {
        let mut lines = vec![
            l("Deutsch E", 0.908, 0.027),
            l("Macintosh HD", 0.38, 0.541),
            l("Optionen", 0.617, 0.542),
            l("Ausschalten", 0.448, 0.915),
            l("Neustart", 0.564, 0.915),
        ];
        if with_continue {
            lines.insert(3, l("Fortfahren", 0.616, 0.638));
        }
        lines
    }

    #[test]
    fn the_startup_options_and_their_keyboard() {
        let Screen::StartupOptions { options, continue_at, layout } = classify(&startup_options(false), "alice", false)
        else {
            panic!()
        };
        assert!((options.0 - 0.617).abs() < 1e-9 && (options.1 - 0.472).abs() < 1e-9, "the icon above its name");
        assert_eq!((continue_at, layout), (None, Some(Layout::De)));
        let Screen::StartupOptions { continue_at, .. } = classify(&startup_options(true), "alice", false) else {
            panic!()
        };
        assert_eq!(continue_at, Some((0.616, 0.638)));
        let english = vec![
            l("ABC", 0.9, 0.027),
            l("Macintosh HD", 0.38, 0.54),
            l("Options", 0.62, 0.54),
            l("Shut Down", 0.45, 0.9),
        ];
        assert!(matches!(classify(&english, "alice", false), Screen::StartupOptions { layout: Some(Layout::Us), .. }));
    }

    #[test]
    fn the_utilities_the_menu_and_the_terminal() {
        let utilities = vec![
            l("Wiederherstellung", 0.11, 0.025),
            l("Ablage", 0.209, 0.026),
            l("Dienstprogramme", 0.386, 0.026),
            l("Aus Time Machine wiederherstellen", 0.474, 0.193),
            l("Festplattendienstprogramm", 0.451, 0.637),
            l("Fortfahren", 0.681, 0.76),
        ];
        assert_eq!(classify(&utilities, "alice", false), Screen::Utilities { menu: Some((0.386, 0.026)) });
        let mut menu = utilities.clone();
        menu.push(l("Terminal", 0.358, 0.152));
        assert_eq!(classify(&menu, "alice", true), Screen::Menu { terminal: (0.358, 0.152) });
        assert!(matches!(classify(&menu, "alice", false), Screen::Utilities { .. }), "a menu only once it was opened");
        let terminal = vec![
            l("Terminal", 0.08, 0.024),
            l("Shell", 0.141, 0.024),
            l("Terminal - -bash - 120×30", 0.21, 0.106),
            l("—bash-3.2#", 0.084, 0.154),
        ];
        assert_eq!(
            classify(&terminal, "alice", false),
            Screen::Terminal { text: vec!["Terminal - -bash - 120×30".into(), "—bash-3.2#".into()] }
        );
    }

    #[test]
    fn a_language_list_and_a_user_to_choose() {
        let languages =
            vec![l("English", 0.5, 0.3), l("Deutsch", 0.5, 0.33), l("Français", 0.5, 0.36), l("Español", 0.5, 0.39)];
        assert_eq!(classify(&languages, "alice", false), Screen::Language { pick: Some((0.5, 0.3)) });
        let user = vec![l("Select a user you know the password for:", 0.5, 0.3), l("alice", 0.5, 0.5)];
        assert_eq!(classify(&user, "alice", false), Screen::User { user: Some((0.5, 0.5)) });
        assert_eq!(classify(&[l("Bitte warten", 0.5, 0.5)], "alice", false), Screen::Unknown);
    }

    #[test]
    fn what_csrutil_asks() {
        let t = |lines: &[&str]| lines.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert_eq!(csrutil_prompt(&t(&["-bash-3.2# csrutil disable"])), Prompt::Waiting);
        assert_eq!(
            csrutil_prompt(&t(&["Turning off System Integrity Protection …", "… for OS \"Macintosh HD\"? [y/n]:"])),
            Prompt::YesNo
        );
        assert_eq!(csrutil_prompt(&t(&["… [y/n]: y", "Enter admin user name:"])), Prompt::User);
        assert_eq!(csrutil_prompt(&t(&["Enter admin user name: alice", "Password:"])), Prompt::Password);
        assert_eq!(
            csrutil_prompt(&t(&[
                "System Integrity Protection is off.",
                "Restart the machine for the changes to take effect."
            ])),
            Prompt::Done
        );
        assert_eq!(csrutil_prompt(&t(&["Error: The user is not authorized."])), Prompt::Failed);
        assert_eq!(protection_off(&t(&["System Integrity Protection status: disabled."])), Some(true));
        assert_eq!(protection_off(&t(&["System Integrity Protection status: enabled."])), Some(false));
        assert_eq!(protection_off(&t(&["-bash-3.2#"])), None);
    }

    #[test]
    fn whether_shift_arrives_and_who_needs_it() {
        let t = |lines: &[&str]| lines.iter().map(|s| s.to_string()).collect::<Vec<_>>();
        assert!(shift_works(&t(&["-bash-3.2# echo SHIFTOK", "SHIFTOK", "-bash-3.2#"])));
        assert!(!shift_works(&t(&["-bash-3.2# echo shiftok", "shiftok"])));
        assert!(!shift_works(&t(&["-bash-3.2# echo SHIFTOK"])), "only the command line");
        assert!(needs_shift('A', Layout::Us) && needs_shift('!', Layout::Us) && needs_shift('_', Layout::De));
        assert!(!needs_shift('a', Layout::Us) && !needs_shift('1', Layout::De) && !needs_shift('-', Layout::Us));
        assert!(needs_shift('/', Layout::De) && !needs_shift('/', Layout::Us), "the layouts differ");
    }

    #[test]
    fn the_recovery_language_in_nvram() {
        assert_eq!(nvram_value("prev-lang:kbd\tde-DE:3\n").as_deref(), Some("de-DE:3"));
        assert_eq!(
            nvram_value("nvram: Error getting variable - 'prev-lang:kbd': (iokit/common) data was not found\n"),
            None
        );
        assert_eq!(nvram_value(""), None);
    }
}
