// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of GuestChoices.swift – work package F3.
//!
//! What the guest user settings offer to choose from (the app's pickers) – all of it works offline.
//! Only what both guest kinds can do: one choice ("German, Berlin, German keyboard") means the matching values for
//! NixOS and for macOS; something only one of them knows is not offered (set it by hand in the recipe instead).
//!
//! Not 1:1 (no Foundation):
//! - Locales: Swift intersected with `Locale.availableIdentifiers`; of the glibc locales in a macOS language, Foundation
//!   lacks only the Chinese ones (it knows them with a script: zh_Hans_CN, …) – that is `NOT_IN_FOUNDATION` here, so
//!   `locales()` gives the same 99 as Swift on macOS 27.
//! - Time zones: Swift listed `TimeZone.knownTimeZoneIdentifiers` (with a "/"). Here: the zoneinfo database's zones in
//!   the areas Foundation lists (Africa, America, …, Pacific). That is a superset of Swift's list (checked on macOS 27)
//!   – it also has 47 older and newer aliases Foundation leaves out (Asia/Kolkata next to Asia/Calcutta,
//!   Australia/NSW, …). OPEN: whether macOS guests (`systemsetup -settimezone`) take those.
//! - Names: Swift named locales and keyboards in the app's language (`Locale.localizedString(forIdentifier:)`). The
//!   core returns English names (the same as Foundation's English ones for every offered locale) whatever `display`
//!   asks for; translated names belong in the UIs, which know their language – OPEN.

use std::path::{Path, PathBuf};

/// Swift `enum GuestChoices` (namespace)
pub enum GuestChoices {}

impl GuestChoices {
    /// Languages macOS 27 has a user interface for (Finder's .lproj folders, region variants folded)
    pub const MAC_LANGUAGES: [&'static str; 33] = [
        "ar", "ca", "cs", "da", "de", "el", "en", "es", "fi", "fr", "he", "hi", "hr", "hu", "id", "it", "ja", "ko",
        "ms", "nl", "no", "pl", "pt", "ro", "ru", "sk", "sl", "sv", "th", "tr", "uk", "vi", "zh",
    ];

    /// UTF-8 locales glibc supports (glibc 2.44 `localedata/SUPPORTED`, without @variants) – NixOS builds only these.
    pub const GLIBC_LOCALES: [&'static str; 152] = [
        "aa_DJ.UTF-8",
        "af_ZA.UTF-8",
        "an_ES.UTF-8",
        "ar_AE.UTF-8",
        "ar_BH.UTF-8",
        "ar_DZ.UTF-8",
        "ar_EG.UTF-8",
        "ar_IQ.UTF-8",
        "ar_JO.UTF-8",
        "ar_KW.UTF-8",
        "ar_LB.UTF-8",
        "ar_LY.UTF-8",
        "ar_MA.UTF-8",
        "ar_OM.UTF-8",
        "ar_QA.UTF-8",
        "ar_SA.UTF-8",
        "ar_SD.UTF-8",
        "ar_SY.UTF-8",
        "ar_TN.UTF-8",
        "ar_YE.UTF-8",
        "ast_ES.UTF-8",
        "be_BY.UTF-8",
        "bg_BG.UTF-8",
        "bhb_IN.UTF-8",
        "br_FR.UTF-8",
        "bs_BA.UTF-8",
        "ca_AD.UTF-8",
        "ca_ES.UTF-8",
        "ca_FR.UTF-8",
        "ca_IT.UTF-8",
        "cs_CZ.UTF-8",
        "cy_GB.UTF-8",
        "da_DK.UTF-8",
        "de_AT.UTF-8",
        "de_BE.UTF-8",
        "de_CH.UTF-8",
        "de_DE.UTF-8",
        "de_IT.UTF-8",
        "de_LI.UTF-8",
        "de_LU.UTF-8",
        "el_GR.UTF-8",
        "el_CY.UTF-8",
        "en_AU.UTF-8",
        "en_BW.UTF-8",
        "en_CA.UTF-8",
        "en_DK.UTF-8",
        "en_GB.UTF-8",
        "en_HK.UTF-8",
        "en_IE.UTF-8",
        "en_NZ.UTF-8",
        "en_PH.UTF-8",
        "en_SC.UTF-8",
        "en_SE.UTF-8",
        "en_SG.UTF-8",
        "en_US.UTF-8",
        "en_ZA.UTF-8",
        "en_ZW.UTF-8",
        "es_AR.UTF-8",
        "es_BO.UTF-8",
        "es_CL.UTF-8",
        "es_CO.UTF-8",
        "es_CR.UTF-8",
        "es_DO.UTF-8",
        "es_EC.UTF-8",
        "es_ES.UTF-8",
        "es_GT.UTF-8",
        "es_HN.UTF-8",
        "es_MX.UTF-8",
        "es_NI.UTF-8",
        "es_PA.UTF-8",
        "es_PE.UTF-8",
        "es_PR.UTF-8",
        "es_PY.UTF-8",
        "es_SV.UTF-8",
        "es_US.UTF-8",
        "es_UY.UTF-8",
        "es_VE.UTF-8",
        "et_EE.UTF-8",
        "eu_ES.UTF-8",
        "fi_FI.UTF-8",
        "fo_FO.UTF-8",
        "fr_BE.UTF-8",
        "fr_CA.UTF-8",
        "fr_CH.UTF-8",
        "fr_FR.UTF-8",
        "fr_LU.UTF-8",
        "ga_IE.UTF-8",
        "gd_GB.UTF-8",
        "gl_ES.UTF-8",
        "gv_GB.UTF-8",
        "he_IL.UTF-8",
        "hr_HR.UTF-8",
        "hrx_BR.UTF-8",
        "hsb_DE.UTF-8",
        "hu_HU.UTF-8",
        "id_ID.UTF-8",
        "is_IS.UTF-8",
        "it_CH.UTF-8",
        "it_IT.UTF-8",
        "ja_JP.UTF-8",
        "ka_GE.UTF-8",
        "kk_KZ.UTF-8",
        "kl_GL.UTF-8",
        "ko_KR.UTF-8",
        "ku_TR.UTF-8",
        "kw_GB.UTF-8",
        "lg_UG.UTF-8",
        "lt_LT.UTF-8",
        "ltg_LV.UTF-8",
        "lv_LV.UTF-8",
        "mg_MG.UTF-8",
        "mi_NZ.UTF-8",
        "mk_MK.UTF-8",
        "ms_MY.UTF-8",
        "mt_MT.UTF-8",
        "nb_NO.UTF-8",
        "nl_BE.UTF-8",
        "nl_NL.UTF-8",
        "nn_NO.UTF-8",
        "oc_FR.UTF-8",
        "om_KE.UTF-8",
        "pl_PL.UTF-8",
        "pt_BR.UTF-8",
        "pt_PT.UTF-8",
        "ro_RO.UTF-8",
        "ru_RU.UTF-8",
        "ru_UA.UTF-8",
        "sk_SK.UTF-8",
        "sl_SI.UTF-8",
        "so_DJ.UTF-8",
        "so_KE.UTF-8",
        "so_SO.UTF-8",
        "sq_AL.UTF-8",
        "st_ZA.UTF-8",
        "sv_FI.UTF-8",
        "sv_SE.UTF-8",
        "tcy_IN.UTF-8",
        "tg_TJ.UTF-8",
        "th_TH.UTF-8",
        "tl_PH.UTF-8",
        "tr_CY.UTF-8",
        "tr_TR.UTF-8",
        "uk_UA.UTF-8",
        "uz_UZ.UTF-8",
        "wa_BE.UTF-8",
        "xh_ZA.UTF-8",
        "yi_US.UTF-8",
        "zh_CN.UTF-8",
        "zh_HK.UTF-8",
        "zh_SG.UTF-8",
        "zh_TW.UTF-8",
        "zu_ZA.UTF-8",
    ];

    /// Keyboard layouts: XKB names for NixOS (the console follows XKB); nix/modules/darwin-guest.nix maps each to
    /// a macOS layout (name and ID from macOS 27's AppleKeyboardLayouts-L.dat, plan B46). (code, locale)
    pub const KEYBOARDS: [(&'static str, &'static str); 16] = [
        ("us", "en_US"),
        ("gb", "en_GB"),
        ("de", "de_DE"),
        ("ch", "de_CH"),
        ("at", "de_AT"),
        ("fr", "fr_FR"),
        ("es", "es_ES"),
        ("it", "it_IT"),
        ("nl", "nl_NL"),
        ("se", "sv_SE"),
        ("no", "nb_NO"),
        ("dk", "da_DK"),
        ("fi", "fi_FI"),
        ("pl", "pl_PL"),
        ("pt", "pt_PT"),
        ("cz", "cs_CZ"),
    ];

    /// The guests' looks (plan B135), in the image flake's order (nix/looks/default.nix): (name, display name). NixOS
    /// guests take the whole desktop from it, macOS guests their terminal's and editor's colours.
    pub const LOOKS: [(&'static str, &'static str); 4] = [
        ("omaflake", "omaflake"),
        ("catppuccin", "Catppuccin Mocha"),
        ("rose-pine-dawn", "Rosé Pine Dawn"),
        ("vanilla", "Vanilla"),
    ];

    /// glibc locales Foundation's `Locale.availableIdentifiers` lacks although their language has a macOS user interface
    /// (measured on macOS 27: Foundation names them with a script, zh_Hans_CN …)
    pub const NOT_IN_FOUNDATION: [&'static str; 4] = ["zh_CN", "zh_HK", "zh_SG", "zh_TW"];

    /// The areas of the time zones Foundation lists (`TimeZone.knownTimeZoneIdentifiers`, macOS 27)
    pub const TIME_ZONE_AREAS: [&'static str; 10] =
        ["Africa", "America", "Antarctica", "Arctic", "Asia", "Atlantic", "Australia", "Europe", "Indian", "Pacific"];

    /// English names of the languages of the offered locales (Foundation's, `en`)
    const LANGUAGE_NAMES: [(&'static str, &'static str); 30] = [
        ("ar", "Arabic"),
        ("ca", "Catalan"),
        ("cs", "Czech"),
        ("da", "Danish"),
        ("de", "German"),
        ("el", "Greek"),
        ("en", "English"),
        ("es", "Spanish"),
        ("fi", "Finnish"),
        ("fr", "French"),
        ("he", "Hebrew"),
        ("hr", "Croatian"),
        ("hu", "Hungarian"),
        ("id", "Indonesian"),
        ("it", "Italian"),
        ("ja", "Japanese"),
        ("ko", "Korean"),
        ("ms", "Malay"),
        ("nb", "Norwegian Bokmål"),
        ("nl", "Dutch"),
        ("pl", "Polish"),
        ("pt", "Portuguese"),
        ("ro", "Romanian"),
        ("ru", "Russian"),
        ("sk", "Slovak"),
        ("sl", "Slovenian"),
        ("sv", "Swedish"),
        ("th", "Thai"),
        ("tr", "Turkish"),
        ("uk", "Ukrainian"),
    ];

    /// English names of the regions of the offered locales (Foundation's, `en`)
    const REGION_NAMES: [(&'static str, &'static str); 83] = [
        ("AD", "Andorra"),
        ("AE", "United Arab Emirates"),
        ("AR", "Argentina"),
        ("AT", "Austria"),
        ("AU", "Australia"),
        ("BE", "Belgium"),
        ("BH", "Bahrain"),
        ("BO", "Bolivia"),
        ("BR", "Brazil"),
        ("BW", "Botswana"),
        ("CA", "Canada"),
        ("CH", "Switzerland"),
        ("CL", "Chile"),
        ("CO", "Colombia"),
        ("CR", "Costa Rica"),
        ("CY", "Cyprus"),
        ("CZ", "Czechia"),
        ("DE", "Germany"),
        ("DK", "Denmark"),
        ("DO", "Dominican Republic"),
        ("DZ", "Algeria"),
        ("EC", "Ecuador"),
        ("EG", "Egypt"),
        ("ES", "Spain"),
        ("FI", "Finland"),
        ("FR", "France"),
        ("GB", "United Kingdom"),
        ("GR", "Greece"),
        ("GT", "Guatemala"),
        ("HK", "Hong Kong"),
        ("HN", "Honduras"),
        ("HR", "Croatia"),
        ("HU", "Hungary"),
        ("ID", "Indonesia"),
        ("IE", "Ireland"),
        ("IL", "Israel"),
        ("IQ", "Iraq"),
        ("IT", "Italy"),
        ("JO", "Jordan"),
        ("JP", "Japan"),
        ("KR", "South Korea"),
        ("KW", "Kuwait"),
        ("LB", "Lebanon"),
        ("LI", "Liechtenstein"),
        ("LU", "Luxembourg"),
        ("LY", "Libya"),
        ("MA", "Morocco"),
        ("MX", "Mexico"),
        ("MY", "Malaysia"),
        ("NI", "Nicaragua"),
        ("NL", "Netherlands"),
        ("NO", "Norway"),
        ("NZ", "New Zealand"),
        ("OM", "Oman"),
        ("PA", "Panama"),
        ("PE", "Peru"),
        ("PH", "Philippines"),
        ("PL", "Poland"),
        ("PR", "Puerto Rico"),
        ("PT", "Portugal"),
        ("PY", "Paraguay"),
        ("QA", "Qatar"),
        ("RO", "Romania"),
        ("RU", "Russia"),
        ("SA", "Saudi Arabia"),
        ("SC", "Seychelles"),
        ("SD", "Sudan"),
        ("SE", "Sweden"),
        ("SG", "Singapore"),
        ("SI", "Slovenia"),
        ("SK", "Slovakia"),
        ("SV", "El Salvador"),
        ("SY", "Syria"),
        ("TH", "Thailand"),
        ("TN", "Tunisia"),
        ("TR", "Türkiye"),
        ("UA", "Ukraine"),
        ("US", "United States"),
        ("UY", "Uruguay"),
        ("VE", "Venezuela"),
        ("YE", "Yemen"),
        ("ZA", "South Africa"),
        ("ZW", "Zimbabwe"),
    ];

    /// Locales for both: glibc can build it, macOS knows the region and has a user interface in that language
    /// ("nb" counts as "no").
    pub fn locales() -> Vec<String> {
        Self::GLIBC_LOCALES
            .iter()
            .filter(|l| {
                let id = l.split('.').next().unwrap_or(l);
                let lang = id.split('_').next().unwrap_or(id);
                let lang = if lang == "nb" { "no" } else { lang };
                !Self::NOT_IN_FOUNDATION.contains(&id) && Self::MAC_LANGUAGES.contains(&lang)
            })
            .map(|l| l.to_string())
            .collect()
    }

    /// Time zones macOS knows (IANA names with a "/"), sorted; NixOS has the full IANA database, so all of them work
    /// there too. From the zoneinfo database (`$TZDIR`, /usr/share/zoneinfo, /var/db/timezone/zoneinfo,
    /// /etc/zoneinfo): its zone files in `TIME_ZONE_AREAS`.
    pub fn time_zones() -> Vec<String> {
        let Some(root) = zoneinfo() else { return Vec::new() };
        let mut out = Vec::new();
        for area in Self::TIME_ZONE_AREAS {
            collect_zones(&root, area, &mut out);
        }
        out.sort();
        out.dedup();
        out
    }

    /// "German (Germany)" for "de_DE.UTF-8" – English whatever `display` is (see the module doc); the identifier when
    /// the language or region is not one of the offered locales'
    pub fn locale_name(locale: &str, display: &str) -> String {
        let _ = display;
        let id = locale.split('.').next().unwrap_or(locale);
        Self::english_name(id).unwrap_or_else(|| id.to_string())
    }

    /// "German (Germany)" for keyboard "de" (English, see `locale_name`); the code itself when unknown
    pub fn keyboard_name(code: &str, display: &str) -> String {
        let _ = display;
        let Some((_, locale)) = Self::KEYBOARDS.iter().find(|(c, _)| *c == code) else { return code.to_string() };
        Self::english_name(locale).unwrap_or_else(|| code.to_string())
    }

    /// "Rosé Pine Dawn" for "rose-pine-dawn"; the name itself when unknown
    pub fn look_name(name: &str) -> String {
        Self::LOOKS.iter().find(|(n, _)| *n == name).map_or_else(|| name.to_string(), |(_, d)| d.to_string())
    }

    /// "de_DE" → "German (Germany)", "de" → "German"
    fn english_name(id: &str) -> Option<String> {
        let mut parts = id.split('_');
        let lang = parts.next()?;
        let language = Self::LANGUAGE_NAMES.iter().find(|(c, _)| *c == lang)?.1;
        match parts.next() {
            None => Some(language.to_string()),
            Some(r) => {
                let region = Self::REGION_NAMES.iter().find(|(c, _)| *c == r)?.1;
                Some(format!("{language} ({region})"))
            }
        }
    }
}

/// The zoneinfo database's folder
fn zoneinfo() -> Option<PathBuf> {
    let mut candidates: Vec<PathBuf> = Vec::new();
    if let Some(dir) = std::env::var_os("TZDIR").filter(|d| !d.is_empty()) {
        candidates.push(PathBuf::from(dir));
    }
    candidates.extend(
        ["/usr/share/zoneinfo", "/var/db/timezone/zoneinfo", "/etc/zoneinfo", "/usr/lib/zoneinfo"].map(PathBuf::from),
    );
    candidates.into_iter().find(|c| c.join("Europe").is_dir() || c.join("America").is_dir())
}

/// A time zone of that name is in the zoneinfo database
pub(crate) fn zoneinfo_has(name: &str) -> bool {
    if name.split('/').any(|p| p.is_empty() || p == "." || p == "..") {
        return false;
    }
    zoneinfo().is_some_and(|root| is_zone_file(&root.join(name)))
}

/// A compiled zone file (TZif)
fn is_zone_file(path: &Path) -> bool {
    use std::io::Read;
    let mut magic = [0u8; 4];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).is_ok() && &magic == b"TZif"
}

/// The zone files under `<root>/<rel>`, as names relative to `root`
fn collect_zones(root: &Path, rel: &str, out: &mut Vec<String>) {
    let path = root.join(rel);
    if path.is_dir() {
        let Ok(entries) = std::fs::read_dir(&path) else { return };
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            if !name.starts_with('.') {
                collect_zones(root, &format!("{rel}/{name}"), out);
            }
        }
    } else if rel.contains('/') && is_zone_file(&path) {
        out.push(rel.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every keyboard the app offers has a macOS layout in the darwin guest module
    #[test]
    fn every_keyboard_has_a_mac_layout() {
        let module = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../nix/modules/darwin-guest.nix");
        let text = std::fs::read_to_string(module).unwrap();
        for (code, _) in GuestChoices::KEYBOARDS {
            assert!(text.contains(&format!("    {code} = {{ id = ")), "{code}");
        }
    }

    /// The looks offered are the image flake's, in its order
    #[test]
    fn every_look_is_the_image_flakes() {
        let looks = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../nix/looks/default.nix");
        let text = std::fs::read_to_string(looks).unwrap();
        let names: Vec<String> = GuestChoices::LOOKS.iter().map(|(n, _)| format!("\"{n}\"")).collect();
        assert!(text.contains(&format!("names = [ {} ];", names.join(" "))), "nix/looks/default.nix `names`");
        assert_eq!(GuestChoices::look_name("rose-pine-dawn"), "Rosé Pine Dawn");
    }

    #[test]
    fn locales_are_the_intersection() {
        let l = GuestChoices::locales();
        let has = |x: &str| l.iter().any(|y| y == x);
        assert!(has("de_DE.UTF-8") && has("en_US.UTF-8") && has("nb_NO.UTF-8"));
        assert!(!has("aa_DJ.UTF-8")); // glibc has it, macOS has no user interface in Afar
        assert!(l.iter().all(|x| GuestChoices::GLIBC_LOCALES.contains(&x.as_str())));
        // What Swift offers on macOS 27 (Foundation's availableIdentifiers lack zh_CN & co.)
        assert_eq!(l.len(), 99);
        assert!(!has("zh_CN.UTF-8"));
    }

    /// Every offered locale and keyboard has an English name (Foundation's: "German (Germany)")
    #[test]
    fn names_are_english() {
        for l in GuestChoices::locales() {
            assert_ne!(GuestChoices::locale_name(&l, "en"), l.trim_end_matches(".UTF-8"), "{l}");
        }
        for (code, _) in GuestChoices::KEYBOARDS {
            assert_ne!(GuestChoices::keyboard_name(code, "en"), code, "{code}");
        }
        assert_eq!(GuestChoices::locale_name("de_DE.UTF-8", "de"), "German (Germany)");
        assert_eq!(GuestChoices::locale_name("nb_NO.UTF-8", "en"), "Norwegian Bokmål (Norway)");
        assert_eq!(GuestChoices::locale_name("xx_YY.UTF-8", "en"), "xx_YY");
        assert_eq!(GuestChoices::keyboard_name("ch", "en"), "German (Switzerland)");
        assert_eq!(GuestChoices::keyboard_name("cz", "en"), "Czech (Czechia)");
        assert_eq!(GuestChoices::keyboard_name("klingon", "en"), "klingon");
    }

    #[test]
    fn time_zones_from_zoneinfo() {
        if zoneinfo().is_none() {
            return; // no zoneinfo database on this host
        }
        let zones = GuestChoices::time_zones();
        assert!(zones.iter().any(|z| z == "Europe/Berlin"));
        assert!(zones.iter().any(|z| z == "America/Argentina/Buenos_Aires"));
        assert!(zones.iter().all(|z| z.contains('/') && !z.starts_with("Etc/") && !z.starts_with("posix/")));
        assert!(zones.windows(2).all(|w| w[0] < w[1]));
        assert!(zoneinfo_has("Europe/Berlin") && !zoneinfo_has("Europe/Atlantis") && !zoneinfo_has("../etc/passwd"));
    }
}
