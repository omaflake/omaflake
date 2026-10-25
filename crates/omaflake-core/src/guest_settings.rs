// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of GuestSettings.swift – work package F3.
//!
//! The person-specific values of every recipe (spec §11.1): recipes never contain them, omaflake passes them in.
//! Written to `<data folder>/guest/settings.json` and handed to the flake as its `omaflake-guest` input
//! (`--override-input`, lock file untouched). A developer's `local/settings.nix` in the flake overrides single values.
//!
//! Sources: the Mac (user, full name, time zone, locale, keyboard, `~/.ssh/*.pub`), the macOS base (its guest user),
//! and the overrides in the app's settings (`Settings::guest`).
//!
//! Not 1:1 (no Foundation): Swift took the time zone from `TimeZone.current`, the language and region from
//! `Locale.current` and the keyboard from the HIToolbox preferences (`UserDefaults`). Here, on macOS: `$TZ`, else the
//! `/etc/localtime` link; `defaults read -g AppleLocale` (else the first of `AppleLanguages`); `defaults export
//! com.apple.HIToolbox` → `AppleSelectedInputSources` (through `plutil`). Checked on macOS 27 against the Swift
//! values ("Europe/Berlin", "de_DE.UTF-8", "de"). On Linux (OPEN, best effort): `$TZ`, `/etc/localtime`;
//! `$LC_ALL`/`$LC_MESSAGES`/`$LANG`; `localectl status` (X11 layout).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

use serde::{Deserialize, Serialize};

use crate::credentials::{Credential, Credentials};
use crate::error::{IoContext, Result};
use crate::file_format::write_atomic;
use crate::image_store::{ImageInfo, ImageStore};
use crate::mac_base_installer::MacBaseInstaller;
use crate::mac_image_builder::MacImageBuilder;
use crate::paths::{Paths, Settings};
use crate::platform::{full_user_name, user_name};
use crate::recipe_sources::RecipeLibrary;

/// guest/settings.json. Keys: user, fullName, email, timeZone, locale, keyboard, macUser, look, authorizedKeys (None
/// left out). Written like Swift's JSONEncoder (`json::to_pretty(…, escape_slashes: true)`), only when the bytes change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GuestSettings {
    pub user: String,
    #[serde(rename = "fullName")]
    pub full_name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[serde(rename = "timeZone")]
    pub time_zone: String,
    pub locale: String,
    pub keyboard: String,
    /// All the guest's keyboards, the first is `keyboard` (plan B141); empty: only `keyboard`
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub keyboards: Vec<String>,
    /// Guest user of macOS VMs – fixed by the base; None = same as `user`
    #[serde(rename = "macUser", default, skip_serializing_if = "Option::is_none")]
    pub mac_user: Option<String>,
    /// The guest's look as data (look.toml of an installed look, plan B141) – the image flake's looks/adapter.nix
    /// turns it into each program's configuration; None = no look (every program its own colours)
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub look: Option<serde_json::Value>,
    #[serde(rename = "authorizedKeys")]
    pub authorized_keys: Vec<String>,
}

impl GuestSettings {
    /// The Mac's own values, without overrides (shown as defaults in the app): `platform::user_name`,
    /// `platform::full_user_name`, the time zone, `mac_locale`, `mac_keyboard`, `MacBaseInstaller::default_public_keys`
    pub fn mac() -> GuestSettings {
        GuestSettings {
            user: user_name(),
            full_name: full_user_name(),
            email: None,
            time_zone: Self::mac_time_zone(),
            locale: Self::mac_locale(),
            keyboard: Self::mac_keyboard(),
            keyboards: vec![],
            mac_user: None,
            look: None,
            authorized_keys: MacBaseInstaller::default_public_keys(),
        }
    }

    /// What the flake gets with the settings' defaults (`BuildChoice::defaults`)
    pub fn current(paths: &Paths) -> GuestSettings {
        Self::for_choice(paths, &BuildChoice::defaults(paths))
    }

    /// What the flake gets for an image built with `choice`: `mac()` with the overrides of `Settings::guest` (locale,
    /// keyboard, time zone; non-empty values), the credential's user, full name and e-mail, the chosen look and the
    /// newest base's guest user as `mac_user` (plan B137)
    pub fn for_choice(paths: &Paths, choice: &BuildChoice) -> GuestSettings {
        let mut g = Self::mac();
        let set = |v: Option<String>| v.filter(|v| !v.is_empty());
        if let Some(c) = &choice.credential {
            if let Some(v) = set(c.user.clone()) {
                g.user = v;
            }
            // Its own full name, else its user – never this computer's (plan B139)
            if let Some(v) = set(c.full_name.clone()).or_else(|| set(c.user.clone())) {
                g.full_name = v;
            }
            g.email = set(c.email.clone());
        }
        // The chosen look's data – a look that is gone gives none
        g.look = set(choice.look.clone()).and_then(|id| {
            let looks = crate::looks::LookStore::new(paths.clone());
            looks.list(&Settings::load(paths)).into_iter().find(|l| l.id == id).map(|l| l.data)
        });
        // The locale: language, region, time zone and the guest's keyboards (plan B141); without one this computer's
        // values with its keyboard (the setting's, else the system's)
        match &choice.locale {
            Some(l) => {
                g.locale = l.locale.clone();
                g.time_zone = l.time_zone.clone();
                if let Some(first) = l.keyboards.first() {
                    g.keyboard = first.clone();
                }
                g.keyboards = if l.keyboards.len() > 1 { l.keyboards.clone() } else { vec![] };
            }
            None => {
                if let Some(k) = set(Settings::load(paths).host_keyboard) {
                    g.keyboard = k;
                }
            }
        }
        if let Some(base) = MacImageBuilder::newest_base_in(paths) {
            g.mac_user = MacBaseInstaller::user_of(&base, &paths.logs());
        }
        g
    }

    /// Writes `guest/<choice>/settings.json` (only when the content changed) and returns the folder – one per choice,
    /// so builds and freshness checks with different credentials or looks never share a file (plan B137)
    pub fn write(paths: &Paths, choice: &BuildChoice) -> Result<PathBuf> {
        let dir = paths.guest().join(choice.key());
        let file = dir.join("settings.json");
        std::fs::create_dir_all(&dir).with_path(&dir)?;
        let data = crate::json::to_pretty(&Self::for_choice(paths, choice), true)?;
        if std::fs::read(&file).ok().as_deref() != Some(data.as_slice()) {
            write_atomic(&file, &data)?;
        }
        Ok(dir)
    }

    /// The Mac's time zone (Swift `TimeZone.current.identifier`): `$TZ` when it names a zone, else the zone the
    /// `/etc/localtime` link points to, else "GMT"
    pub(crate) fn mac_time_zone() -> String {
        if let Ok(tz) = std::env::var("TZ") {
            let tz = tz.trim_start_matches(':');
            if !tz.is_empty() && !tz.starts_with('/') && crate::guest_choices::zoneinfo_has(tz) {
                return tz.to_string();
            }
        }
        if let Ok(target) = std::fs::read_link("/etc/localtime") {
            if let Some(zone) = Self::zone_of_link(&target) {
                return zone;
            }
        }
        "GMT".into()
    }

    /// "/var/db/timezone/zoneinfo/Europe/Berlin" → "Europe/Berlin"
    pub(crate) fn zone_of_link(target: &Path) -> Option<String> {
        let text = target.to_string_lossy();
        let i = text.rfind("zoneinfo/")?;
        let zone = &text[i + "zoneinfo/".len()..];
        (!zone.is_empty()).then(|| zone.to_string())
    }

    /// "de_DE.UTF-8" from the Mac's language and region (region missing: "US" for English, else the language uppercased)
    pub(crate) fn mac_locale() -> String {
        Self::locale_from_identifier(&Self::locale_identifier())
    }

    /// The locale identifier Foundation's `Locale.current` is made from
    fn locale_identifier() -> String {
        #[cfg(target_os = "macos")]
        {
            if let Some(l) = command_output("/usr/bin/defaults", &["read", "-g", "AppleLocale"]) {
                return l;
            }
            if let Some(langs) = command_output("/usr/bin/defaults", &["read", "-g", "AppleLanguages"]) {
                // ( "de-DE", "ru-DE" )
                if let Some(first) = langs.split('"').nth(1) {
                    return first.to_string();
                }
            }
            "en_US".into()
        }
        #[cfg(not(target_os = "macos"))]
        {
            for key in ["LC_ALL", "LC_MESSAGES", "LANG"] {
                if let Ok(v) = std::env::var(key) {
                    if !v.is_empty() && v != "C" && v != "POSIX" && !v.starts_with("C.") {
                        return v;
                    }
                }
            }
            "en_US".into()
        }
    }

    /// "de_DE" / "de-DE" / "en_DE@rg=chzzzz" / "zh-Hans_CN" / "de_DE.UTF-8" → "<language>_<region>.UTF-8" as Swift's
    /// `macLocale` builds it from `Locale.language.languageCode` and `Locale.region`
    pub(crate) fn locale_from_identifier(identifier: &str) -> String {
        let (main, extension) = match identifier.split_once('@') {
            Some((m, e)) => (m, Some(e)),
            None => (identifier, None),
        };
        let main = main.split('.').next().unwrap_or(main);
        let parts: Vec<&str> = main.split(['_', '-']).filter(|p| !p.is_empty()).collect();
        let lang = parts.first().map(|l| l.to_lowercase()).filter(|l| l.chars().all(|c| c.is_ascii_alphabetic()));
        let lang = lang.unwrap_or_else(|| "en".into());
        // The region: "rg=" of the extension (its first two letters), else the first part that is a region code
        let from_extension = extension.and_then(|e| {
            e.split(';').find_map(|kv| {
                let v = kv.strip_prefix("rg=")?;
                let r: String = v.chars().take(2).collect();
                (r.len() == 2 && r.chars().all(|c| c.is_ascii_alphabetic())).then(|| r.to_uppercase())
            })
        });
        let region = from_extension.or_else(|| {
            parts.iter().skip(1).find_map(|p| {
                let alpha = p.len() == 2 && p.chars().all(|c| c.is_ascii_alphabetic());
                let numeric = p.len() == 3 && p.chars().all(|c| c.is_ascii_digit());
                (alpha || numeric).then(|| p.to_uppercase())
            })
        });
        let region = region.unwrap_or_else(|| if lang == "en" { "US".into() } else { lang.to_uppercase() });
        format!("{lang}_{region}.UTF-8")
    }

    /// Keyboard layout of the Mac as an XKB/console name: "de", "us", … (the language as a fallback, "us" for English).
    pub(crate) fn mac_keyboard() -> String {
        for name in Self::selected_layout_names() {
            if let Some(k) = Self::keyboard_from_input_source(&name) {
                return k;
            }
        }
        let locale = Self::mac_locale();
        let lang = locale.split('_').next().unwrap_or("en");
        if lang == "en" {
            "us".into()
        } else {
            lang.to_string()
        }
    }

    /// The "KeyboardLayout Name" of each selected input source (macOS: HIToolbox `AppleSelectedInputSources`; Linux:
    /// the X11 layout of `localectl status`, OPEN)
    fn selected_layout_names() -> Vec<String> {
        #[cfg(target_os = "macos")]
        {
            let Ok(mut export) = Command::new("/usr/bin/defaults")
                .args(["export", "com.apple.HIToolbox", "-"])
                .stdin(Stdio::null())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            else {
                return Vec::new();
            };
            let Some(stdout) = export.stdout.take() else { return Vec::new() };
            let output = Command::new("/usr/bin/plutil")
                .args(["-extract", "AppleSelectedInputSources", "json", "-o", "-", "-"])
                .stdin(Stdio::from(stdout))
                .stderr(Stdio::null())
                .output();
            let _ = export.wait();
            let Ok(output) = output else { return Vec::new() };
            Self::layout_names_in(&output.stdout)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let Some(status) = command_output("localectl", &["status"]) else { return Vec::new() };
            status
                .lines()
                .filter_map(|l| l.trim().strip_prefix("X11 Layout:"))
                .flat_map(|v| v.split(',').map(|s| s.trim().to_string()).collect::<Vec<_>>())
                .filter(|s| !s.is_empty())
                .collect()
        }
    }

    /// The "KeyboardLayout Name"s of `AppleSelectedInputSources` as JSON, in their order
    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // used on macOS only
    pub(crate) fn layout_names_in(json: &[u8]) -> Vec<String> {
        let Ok(serde_json::Value::Array(sources)) = serde_json::from_slice::<serde_json::Value>(json) else {
            return Vec::new();
        };
        sources
            .iter()
            .filter_map(|s| s.get("KeyboardLayout Name").and_then(|n| n.as_str()).map(str::to_string))
            .collect()
    }

    /// "com.apple.keylayout.German" → "de"; None when unknown (exact name first, then a known name it starts with)
    pub(crate) fn keyboard_from_input_source(id: &str) -> Option<String> {
        const KNOWN: [(&str, &str); 17] = [
            ("German", "de"),
            ("ABC", "us"),
            ("US", "us"),
            ("British", "gb"),
            ("French", "fr"),
            ("Swiss German", "ch"),
            ("Austrian", "de"),
            ("Spanish", "es"),
            ("Italian", "it"),
            ("Dutch", "nl"),
            ("Swedish", "se"),
            ("Norwegian", "no"),
            ("Danish", "dk"),
            ("Finnish", "fi"),
            ("Polish", "pl"),
            ("Portuguese", "pt"),
            ("Czech", "cz"),
        ];
        let name = id.rsplit('.').next().unwrap_or(id);
        if let Some((_, k)) = KNOWN.iter().find(|(n, _)| *n == name) {
            return Some(k.to_string());
        }
        // Swift took the first match of an unordered dictionary; the longest name is the deterministic choice
        // ("US International" → "us", "Swiss German – …" → "ch")
        KNOWN.iter().filter(|(n, _)| name.starts_with(n)).max_by_key(|(n, _)| n.len()).map(|(_, k)| k.to_string())
    }
}

/// The trimmed standard output of a command that succeeded with some
fn command_output(exe: &str, args: &[&str]) -> Option<String> {
    let out = Command::new(exe).args(args).stdin(Stdio::null()).stderr(Stdio::null()).output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

/// What an image is built with besides its recipe (plan B137, B141): a credential (None: this computer's values,
/// images from before credentials), a look – an installed look's id (None: no look) – and a locale (None: this
/// computer's values)
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BuildChoice {
    pub credential: Option<Credential>,
    pub look: Option<String>,
    pub locale: Option<crate::locales::GuestLocale>,
}

impl BuildChoice {
    /// The settings' defaults: the default credential and the default look
    pub fn defaults(paths: &Paths) -> BuildChoice {
        BuildChoice {
            credential: Credentials::new(paths.clone()).default_credential(),
            look: crate::looks::LookStore::new(paths.clone()).default_look(&Settings::load(paths)).map(|l| l.id),
            locale: crate::locales::Locales::new(paths.clone()).default_locale(),
        }
    }

    /// An image's own: its credential (when it still exists) and its chosen look
    pub fn of_image(paths: &Paths, info: &ImageInfo) -> BuildChoice {
        BuildChoice {
            credential: info.credential.as_deref().and_then(|id| Credentials::new(paths.clone()).get(id)),
            // Images from before installable looks name a look of omaflake's own (plan B141)
            look: info.look_choice.clone().map(|l| if l.contains(".look.") { l } else { format!("omaflake.look.{l}") }),
            locale: info.locale.as_deref().and_then(|id| crate::locales::Locales::new(paths.clone()).get(id)),
        }
    }

    /// The folder under guest/ for this choice: "<credential id or computer>-<look or recipe>"
    pub fn key(&self) -> String {
        let credential = self.credential.as_ref().map_or("computer", |c| c.id.as_str());
        let look = self.look.as_deref().unwrap_or("recipe");
        let locale = self.locale.as_ref().map_or("computer", |l| l.id.as_str());
        format!("{credential}-{look}-{locale}")
            .chars()
            .map(|c| if c.is_ascii_alphanumeric() || c == '-' { c } else { '_' })
            .collect()
    }
}

impl ImageStore {
    /// Arguments for a `nix` call on the image flake: the guest values as its `omaflake-guest` input and the recipes
    /// with their modules and key bindings as `omaflake-catalog` (RecipeLibrary::stage), without writing the lock file (it would get the
    /// data folder's paths). For a local folder an input is passed only when its flake.nix declares it; the `nix/`
    /// folder of an omaflake checkout also gets the checkout as its `omaflake` input (`omaflake_checkout_arguments`).
    /// `["--override-input", "omaflake-guest", "path:<dir>", "--override-input", "omaflake-catalog", "path:<dir>",
    /// "--no-write-lock-file"]`, or nothing.
    /// `guest_arguments_for` with the settings' defaults
    pub fn guest_arguments(&self, flake: &str) -> Vec<String> {
        self.guest_arguments_for(flake, &BuildChoice::defaults(&self.paths))
    }

    /// Arguments for a `nix` call on the image flake with `choice`'s guest values (see `guest_arguments`' doc below)
    pub fn guest_arguments_for(&self, flake: &str, choice: &BuildChoice) -> Vec<String> {
        let (mut guest, mut recipes) = (true, true);
        if flake.starts_with('/') {
            let nix = std::fs::read_to_string(Path::new(flake).join("flake.nix")).unwrap_or_default();
            guest = nix.contains("omaflake-guest");
            recipes = nix.contains("omaflake-catalog");
        }
        let mut args = Self::omaflake_checkout_arguments(flake);
        if guest {
            if let Ok(dir) = GuestSettings::write(&self.paths, choice) {
                args.extend(["--override-input".into(), "omaflake-guest".into(), format!("path:{}", dir.display())]);
            }
        }
        if recipes {
            if let Ok(dir) = RecipeLibrary::new(self.paths.clone()).stage(&Settings::load(&self.paths)) {
                args.extend(["--override-input".into(), "omaflake-catalog".into(), format!("path:{}", dir.display())]);
            }
        }
        if args.is_empty() {
            return args;
        }
        args.push("--no-write-lock-file".into());
        args
    }

    /// The image flake as the `nix/` folder of an omaflake checkout (its parent has the root flake.nix and crates/,
    /// its flake.nix declares the input `omaflake`): `["--override-input", "omaflake", <checkout>]`, so recipes that
    /// bring omaflake (`pkgs.omaflake`, nixos/lab) get the code as it is on disk (plan B123). A checkout with its own
    /// git repository is passed as a plain path – Nix takes the tracked files with their changes, not `target/` or
    /// other build output –, else as `path:`. Any other flake (the app's bundled copy) keeps its locked default.
    pub fn omaflake_checkout_arguments(flake: &str) -> Vec<String> {
        let flake = flake.strip_prefix("path:").unwrap_or(flake);
        if !flake.starts_with('/') {
            return Vec::new();
        }
        let nix = std::fs::read_to_string(Path::new(flake).join("flake.nix")).unwrap_or_default();
        let declares = nix.contains("omaflake = {") || nix.contains("omaflake.url");
        let Some(checkout) = Path::new(flake.trim_end_matches('/')).parent() else { return Vec::new() };
        if !declares || !checkout.join("flake.nix").is_file() || !checkout.join("crates").is_dir() {
            return Vec::new();
        }
        let reference = if checkout.join(".git").exists() {
            checkout.display().to_string()
        } else {
            format!("path:{}", checkout.display())
        };
        vec!["--override-input".into(), "omaflake".into(), reference]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::guest_choices::GuestChoices;
    use crate::paths::tests::temp_paths;

    #[test]
    fn keyboard_from_input_source() {
        assert_eq!(GuestSettings::keyboard_from_input_source("com.apple.keylayout.German").as_deref(), Some("de"));
        assert_eq!(GuestSettings::keyboard_from_input_source("com.apple.keylayout.ABC").as_deref(), Some("us"));
        assert_eq!(GuestSettings::keyboard_from_input_source("com.apple.keylayout.Klingon"), None);
        assert_eq!(GuestSettings::keyboard_from_input_source("German").as_deref(), Some("de"));
        assert_eq!(GuestSettings::keyboard_from_input_source("Swiss German").as_deref(), Some("ch"));
        assert_eq!(GuestSettings::keyboard_from_input_source("US International PC").as_deref(), Some("us"));
    }

    /// The HIToolbox list as `plutil -extract … json` gives it (macOS 27)
    #[test]
    fn layout_names_from_hitoolbox() {
        let json = br#"[{"Bundle ID":"com.apple.PressAndHold","InputSourceKind":"Non Keyboard Input Method"},{"InputSourceKind":"Keyboard Layout","KeyboardLayout Name":"German","KeyboardLayout ID":3}]"#;
        assert_eq!(GuestSettings::layout_names_in(json), vec!["German".to_string()]);
        assert!(GuestSettings::layout_names_in(b"not json").is_empty());
    }

    /// What Swift's `macLocale` makes of `Locale.current` (language code and region)
    #[test]
    fn locale_from_identifiers() {
        let l = GuestSettings::locale_from_identifier;
        assert_eq!(l("de_DE"), "de_DE.UTF-8");
        assert_eq!(l("de-DE"), "de_DE.UTF-8");
        assert_eq!(l("en_DE@rg=chzzzz"), "en_CH.UTF-8");
        assert_eq!(l("zh-Hans_CN"), "zh_CN.UTF-8");
        assert_eq!(l("es_419"), "es_419.UTF-8");
        assert_eq!(l("de"), "de_DE.UTF-8");
        assert_eq!(l("en"), "en_US.UTF-8");
        assert_eq!(l("fr_FR.UTF-8"), "fr_FR.UTF-8");
    }

    #[test]
    fn zone_from_the_localtime_link() {
        let z = GuestSettings::zone_of_link(Path::new("/var/db/timezone/zoneinfo/Europe/Berlin"));
        assert_eq!(z.as_deref(), Some("Europe/Berlin"));
        let z = GuestSettings::zone_of_link(Path::new("../usr/share/zoneinfo/America/Argentina/Buenos_Aires"));
        assert_eq!(z.as_deref(), Some("America/Argentina/Buenos_Aires"));
        assert_eq!(GuestSettings::zone_of_link(Path::new("/etc/foo")), None);
    }

    /// The Mac's values have the shapes the flake expects (the values themselves depend on the host)
    #[test]
    fn host_values_have_their_shape() {
        let tz = GuestSettings::mac_time_zone();
        // GMT/UTC on hosts set to it (CI runners), else an area/location zone
        assert!(tz == "GMT" || tz == "UTC" || tz.contains('/'), "{tz}");
        let locale = GuestSettings::mac_locale();
        assert!(locale.ends_with(".UTF-8") && locale.contains('_'), "{locale}");
        let k = GuestSettings::mac_keyboard();
        assert!(!k.is_empty() && k.chars().all(|c| c.is_ascii_lowercase()), "{k}");
        if cfg!(target_os = "macos") && tz.contains('/') {
            assert!(GuestChoices::time_zones().contains(&tz), "{tz}");
        }
    }

    /// Needs `MacBaseInstaller::{default_public_keys, user_of}` and `MacImageBuilder::newest_base_in` (F4) and
    /// `MacVmBundle` – run after the merge with `cargo test -- --ignored`
    #[test]
    fn overrides_and_base_user_win() {
        use crate::mac_vm::{MacVmBundle, MacVmConfig};
        let (_t, p) = temp_paths();
        // A look in a local folder source: the default (the only one)
        let src = tempfile::tempdir().unwrap();
        let look = src.path().join("looks/alice/plain");
        std::fs::create_dir_all(&look).unwrap();
        std::fs::write(
            look.join("look.toml"),
            "\"$schema\" = \"https://omaflake.dev/schema/look/v1.json\"\nauthor = \"alice\"\nlicense = \"MIT\"\ntitle = \"Plain\"\ndescription = \"x\"\ntags = [\"plain\"]\nplain = true\n",
        )
        .unwrap();
        let folder = crate::recipe_sources::RecipeSource::new(
            crate::recipe_sources::RecipeSourceKind::Folder,
            &src.path().to_string_lossy(),
            None,
            None,
            None,
        );
        let s = Settings {
            sources: Some(vec![folder]),
            host_keyboard: Some("us".into()),
            credentials: Some(vec![Credential {
                id: "c-1".into(),
                name: "Dev".into(),
                user: Some("dev".into()),
                ..Default::default()
            }]),
            default_credential: Some("c-1".into()),
            ..Default::default()
        };
        s.save(&p).unwrap();
        let base = p.bases().join("mac-base-1");
        std::fs::create_dir_all(&base).unwrap();
        MacVmBundle::new(base.clone()).save(&MacVmConfig::new(4, 8192, 80, "02:00:00:00:00:01", None)).unwrap();
        std::fs::write(base.join(MacBaseInstaller::READY_MARKER), "carol\n").unwrap();
        let g = GuestSettings::current(&p);
        assert!(g.user == "dev" && g.keyboard == "us" && g.mac_user.as_deref() == Some("carol"));
        // The look goes to the image flake as data (plan B141)
        assert_eq!(g.look.as_ref().and_then(|l| l.get("title")).and_then(|t| t.as_str()), Some("Plain"));
        // written once, stable content (no rewrite when unchanged)
        let choice = BuildChoice::defaults(&p);
        let dir = GuestSettings::write(&p, &choice).unwrap();
        assert!(dir.ends_with("guest/c-1-alice_look_plain-computer"));
        let file = dir.join("settings.json");
        let first = std::fs::metadata(&file).unwrap().modified().unwrap();
        GuestSettings::write(&p, &choice).unwrap();
        let second = std::fs::metadata(&file).unwrap().modified().unwrap();
        assert_eq!(first, second);
        // Another choice: this computer's values, the recipe's look – a folder of its own
        let other = BuildChoice::default();
        let g = GuestSettings::for_choice(&p, &other);
        assert!(g.user != "dev" && g.look.is_none() && g.keyboard == "us");
        assert!(GuestSettings::write(&p, &other).unwrap().ends_with("guest/computer-recipe-computer"));
    }

    /// Needs `GuestSettings::current` → F4 (see above)
    #[test]
    fn guest_arguments_only_for_flakes_with_the_input() {
        let (_t, p) = temp_paths();
        let with_input = p.root.join("a");
        let without = p.root.join("b");
        for d in [&with_input, &without] {
            std::fs::create_dir_all(d).unwrap();
        }
        std::fs::write(
            with_input.join("flake.nix"),
            "{ inputs.omaflake-guest = { url = \"path:./guest\"; flake = false; }; }",
        )
        .unwrap();
        std::fs::write(without.join("flake.nix"), "{ }").unwrap();
        let store = ImageStore::new(p.clone());
        assert!(store.guest_arguments(&with_input.to_string_lossy()).contains(&"--no-write-lock-file".to_string()));
        assert!(store.guest_arguments(&without.to_string_lossy()).is_empty());
    }

    /// The `nix/` folder of a checkout gets the checkout as its `omaflake` input; other flakes do not (plan B123)
    #[test]
    fn omaflake_input_from_the_checkout() {
        let (_t, p) = temp_paths();
        let checkout = p.root.join("checkout");
        let nix = checkout.join("nix");
        std::fs::create_dir_all(nix.join("guest")).unwrap();
        std::fs::create_dir_all(checkout.join("crates")).unwrap();
        std::fs::write(checkout.join("flake.nix"), "{ }").unwrap();
        std::fs::write(
            nix.join("flake.nix"),
            "{ inputs.omaflake = { url = \"github:omaflake/omaflake\"; inputs.nixpkgs.follows = \"nixpkgs\"; }; }",
        )
        .unwrap();
        let args = |f: &Path| ImageStore::omaflake_checkout_arguments(&f.to_string_lossy());
        let expected = |r: String| vec!["--override-input".to_string(), "omaflake".into(), r];
        // without git: path:, with git: the folder (Nix takes the tracked files)
        assert_eq!(args(&nix), expected(format!("path:{}", checkout.display())));
        assert_eq!(
            ImageStore::omaflake_checkout_arguments(&format!("path:{}/", nix.display())),
            expected(format!("path:{}", checkout.display()))
        );
        std::fs::create_dir_all(checkout.join(".git")).unwrap();
        assert_eq!(args(&nix), expected(checkout.display().to_string()));
        // part of the arguments of every nix call, before the lock-file flag
        let all = ImageStore::new(p.clone()).guest_arguments(&nix.to_string_lossy());
        assert_eq!(all[..3], expected(checkout.display().to_string())[..]);
        assert_eq!(all.last().map(String::as_str), Some("--no-write-lock-file"));
        // a bundled copy (no checkout around it), a flake without the input, a flake URL: nothing
        let bundled = p.root.join("Resources").join("nix");
        std::fs::create_dir_all(&bundled).unwrap();
        std::fs::copy(nix.join("flake.nix"), bundled.join("flake.nix")).unwrap();
        assert!(args(&bundled).is_empty());
        std::fs::write(nix.join("flake.nix"), "{ inputs.omaflake-catalog.url = \"path:./no-catalog\"; }").unwrap();
        assert!(args(&nix).is_empty());
        assert!(ImageStore::omaflake_checkout_arguments("github:omaflake/omaflake?dir=nix").is_empty());
    }
}
