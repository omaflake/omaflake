// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// What the guest user settings offer to choose from (the app's pickers) – all of it works offline.
/// Only what both guest kinds can do: one choice ("German, Berlin, German keyboard") means the matching values for
/// NixOS and for macOS; something only one of them knows is not offered (set it by hand in the recipe instead).
public enum GuestChoices {
    /// Locales for both: glibc can build it, macOS knows the region and has a user interface in that language.
    public static var locales: [String] {
        let apple = Set(Locale.availableIdentifiers)
        return glibcLocales.filter { l in
            let id = String(l.prefix { $0 != "." })
            let lang = String(id.prefix { $0 != "_" })
            return apple.contains(id) && macLanguages.contains(lang == "nb" ? "no" : lang)
        }
    }

    /// Languages macOS 27 has a user interface for (Finder's .lproj folders, region variants folded)
    static let macLanguages: Set<String> = [
        "ar", "ca", "cs", "da", "de", "el", "en", "es", "fi", "fr", "he", "hi", "hr", "hu", "id", "it", "ja", "ko", "ms",
        "nl", "no", "pl", "pt", "ro", "ru", "sk", "sl", "sv", "th", "tr", "uk", "vi", "zh",
    ]

    /// UTF-8 locales glibc supports (glibc 2.44 `localedata/SUPPORTED`, without @variants) – NixOS builds only these.
    static let glibcLocales: [String] = [
        "aa_DJ.UTF-8", "af_ZA.UTF-8", "an_ES.UTF-8", "ar_AE.UTF-8", "ar_BH.UTF-8", "ar_DZ.UTF-8",
        "ar_EG.UTF-8", "ar_IQ.UTF-8", "ar_JO.UTF-8", "ar_KW.UTF-8", "ar_LB.UTF-8", "ar_LY.UTF-8",
        "ar_MA.UTF-8", "ar_OM.UTF-8", "ar_QA.UTF-8", "ar_SA.UTF-8", "ar_SD.UTF-8", "ar_SY.UTF-8",
        "ar_TN.UTF-8", "ar_YE.UTF-8", "ast_ES.UTF-8", "be_BY.UTF-8", "bg_BG.UTF-8", "bhb_IN.UTF-8",
        "br_FR.UTF-8", "bs_BA.UTF-8", "ca_AD.UTF-8", "ca_ES.UTF-8", "ca_FR.UTF-8", "ca_IT.UTF-8",
        "cs_CZ.UTF-8", "cy_GB.UTF-8", "da_DK.UTF-8", "de_AT.UTF-8", "de_BE.UTF-8", "de_CH.UTF-8",
        "de_DE.UTF-8", "de_IT.UTF-8", "de_LI.UTF-8", "de_LU.UTF-8", "el_GR.UTF-8", "el_CY.UTF-8",
        "en_AU.UTF-8", "en_BW.UTF-8", "en_CA.UTF-8", "en_DK.UTF-8", "en_GB.UTF-8", "en_HK.UTF-8",
        "en_IE.UTF-8", "en_NZ.UTF-8", "en_PH.UTF-8", "en_SC.UTF-8", "en_SE.UTF-8", "en_SG.UTF-8",
        "en_US.UTF-8", "en_ZA.UTF-8", "en_ZW.UTF-8", "es_AR.UTF-8", "es_BO.UTF-8", "es_CL.UTF-8",
        "es_CO.UTF-8", "es_CR.UTF-8", "es_DO.UTF-8", "es_EC.UTF-8", "es_ES.UTF-8", "es_GT.UTF-8",
        "es_HN.UTF-8", "es_MX.UTF-8", "es_NI.UTF-8", "es_PA.UTF-8", "es_PE.UTF-8", "es_PR.UTF-8",
        "es_PY.UTF-8", "es_SV.UTF-8", "es_US.UTF-8", "es_UY.UTF-8", "es_VE.UTF-8", "et_EE.UTF-8",
        "eu_ES.UTF-8", "fi_FI.UTF-8", "fo_FO.UTF-8", "fr_BE.UTF-8", "fr_CA.UTF-8", "fr_CH.UTF-8",
        "fr_FR.UTF-8", "fr_LU.UTF-8", "ga_IE.UTF-8", "gd_GB.UTF-8", "gl_ES.UTF-8", "gv_GB.UTF-8",
        "he_IL.UTF-8", "hr_HR.UTF-8", "hrx_BR.UTF-8", "hsb_DE.UTF-8", "hu_HU.UTF-8", "id_ID.UTF-8",
        "is_IS.UTF-8", "it_CH.UTF-8", "it_IT.UTF-8", "ja_JP.UTF-8", "ka_GE.UTF-8", "kk_KZ.UTF-8",
        "kl_GL.UTF-8", "ko_KR.UTF-8", "ku_TR.UTF-8", "kw_GB.UTF-8", "lg_UG.UTF-8", "lt_LT.UTF-8",
        "ltg_LV.UTF-8", "lv_LV.UTF-8", "mg_MG.UTF-8", "mi_NZ.UTF-8", "mk_MK.UTF-8", "ms_MY.UTF-8",
        "mt_MT.UTF-8", "nb_NO.UTF-8", "nl_BE.UTF-8", "nl_NL.UTF-8", "nn_NO.UTF-8", "oc_FR.UTF-8",
        "om_KE.UTF-8", "pl_PL.UTF-8", "pt_BR.UTF-8", "pt_PT.UTF-8", "ro_RO.UTF-8", "ru_RU.UTF-8",
        "ru_UA.UTF-8", "sk_SK.UTF-8", "sl_SI.UTF-8", "so_DJ.UTF-8", "so_KE.UTF-8", "so_SO.UTF-8",
        "sq_AL.UTF-8", "st_ZA.UTF-8", "sv_FI.UTF-8", "sv_SE.UTF-8", "tcy_IN.UTF-8", "tg_TJ.UTF-8",
        "th_TH.UTF-8", "tl_PH.UTF-8", "tr_CY.UTF-8", "tr_TR.UTF-8", "uk_UA.UTF-8", "uz_UZ.UTF-8",
        "wa_BE.UTF-8", "xh_ZA.UTF-8", "yi_US.UTF-8", "zh_CN.UTF-8", "zh_HK.UTF-8", "zh_SG.UTF-8",
        "zh_TW.UTF-8", "zu_ZA.UTF-8",
    ]

    /// Keyboard layouts: XKB names for NixOS (the console follows XKB); nix/modules/darwin-guest.nix maps each to
    /// a macOS layout (name and ID from macOS 27's AppleKeyboardLayouts-L.dat, plan B46).
    public static let keyboards: [(code: String, locale: String)] = [
        ("us", "en_US"), ("gb", "en_GB"), ("de", "de_DE"), ("ch", "de_CH"), ("at", "de_AT"), ("fr", "fr_FR"),
        ("es", "es_ES"), ("it", "it_IT"), ("nl", "nl_NL"), ("se", "sv_SE"), ("no", "nb_NO"), ("dk", "da_DK"),
        ("fi", "fi_FI"), ("pl", "pl_PL"), ("pt", "pt_PT"), ("cz", "cs_CZ"),
    ]

    /// Time zones macOS knows (IANA names); NixOS has the full IANA database, so all of them work there too
    public static var timeZones: [String] { TimeZone.knownTimeZoneIdentifiers.filter { $0.contains("/") }.sorted() }

    /// "Deutsch (Deutschland)" for "de_DE.UTF-8", in the app's language
    public static func localeName(_ locale: String, in display: Locale = .current) -> String {
        let id = locale.components(separatedBy: ".").first ?? locale
        return display.localizedString(forIdentifier: id) ?? id
    }

    /// "Deutsch (Deutschland)" for keyboard "de"
    public static func keyboardName(_ code: String, in display: Locale = .current) -> String {
        guard let k = keyboards.first(where: { $0.code == code }) else { return code }
        return display.localizedString(forIdentifier: k.locale) ?? code
    }
}
