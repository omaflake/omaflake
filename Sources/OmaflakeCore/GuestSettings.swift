// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// The person-specific values of every recipe (spec §11.1): recipes never contain them, omaflake passes them in.
/// Written to `<data folder>/guest/settings.json` and handed to the flake as its `omaflake-guest` input
/// (`--override-input`, lock file untouched). A developer's `local/settings.nix` in the flake overrides single values.
///
/// Sources: the Mac (user, full name, time zone, locale, keyboard, `~/.ssh/*.pub`), the macOS base (its guest user),
/// and the overrides in the app's settings (`Settings.guest`).
public struct GuestSettings: Codable, Sendable, Equatable {
    public var user: String
    public var fullName: String
    public var email: String?
    public var timeZone: String
    public var locale: String
    public var keyboard: String
    /// Guest user of macOS VMs – fixed by the base; nil = same as `user`
    public var macUser: String?
    public var authorizedKeys: [String]

    /// The Mac's own values, without overrides (shown as defaults in the app).
    public static func mac() -> GuestSettings {
        GuestSettings(user: NSUserName(), fullName: NSFullUserName(), email: nil,
                      timeZone: TimeZone.current.identifier, locale: macLocale(), keyboard: macKeyboard(),
                      macUser: nil, authorizedKeys: MacBaseInstaller.defaultPublicKeys())
    }

    /// What the flake gets for this Mac now.
    public static func current(_ paths: Paths) -> GuestSettings {
        var g = mac()
        if let o = Settings.load(paths).guest {
            if let v = o.user, !v.isEmpty { g.user = v }
            if let v = o.fullName { g.fullName = v }
            if let v = o.email, !v.isEmpty { g.email = v }
            if let v = o.timeZone, !v.isEmpty { g.timeZone = v }
            if let v = o.locale, !v.isEmpty { g.locale = v }
            if let v = o.keyboard, !v.isEmpty { g.keyboard = v }
        }
        if let base = MacImageBuilder.newestBase(paths) { g.macUser = MacBaseInstaller.user(of: base, logs: paths.logs) }
        return g
    }

    /// Writes `guest/settings.json` (only when the content changed) and returns the folder.
    @discardableResult
    public static func write(_ paths: Paths) throws -> URL {
        let file = paths.guest.appendingPathComponent("settings.json")
        try FileManager.default.createDirectory(at: paths.guest, withIntermediateDirectories: true)
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        let data = try enc.encode(current(paths))
        if (try? Data(contentsOf: file)) != data { try data.write(to: file, options: .atomic) }
        return paths.guest
    }

    /// "de_DE.UTF-8" from the Mac's language and region
    static func macLocale() -> String {
        let lang = Locale.current.language.languageCode?.identifier ?? "en"
        let region = Locale.current.region?.identifier ?? (lang == "en" ? "US" : lang.uppercased())
        return "\(lang)_\(region).UTF-8"
    }

    /// Keyboard layout of the Mac as an XKB/console name: "de", "us", … (the language as a fallback).
    /// Read from the HIToolbox preferences – the Carbon input source API must run on the main thread.
    static func macKeyboard() -> String {
        let sources = UserDefaults(suiteName: "com.apple.HIToolbox")?.array(forKey: "AppleSelectedInputSources") as? [[String: Any]] ?? []
        for s in sources {
            if let name = s["KeyboardLayout Name"] as? String, let k = keyboard(fromInputSource: name) { return k }
        }
        let lang = Locale.current.language.languageCode?.identifier ?? "en"
        return lang == "en" ? "us" : lang
    }

    /// "com.apple.keylayout.German" → "de"; nil when unknown
    static func keyboard(fromInputSource id: String) -> String? {
        let name = id.components(separatedBy: ".").last ?? id
        let known = ["German": "de", "ABC": "us", "US": "us", "British": "gb", "French": "fr", "Swiss German": "ch",
                     "Austrian": "de", "Spanish": "es", "Italian": "it", "Dutch": "nl", "Swedish": "se", "Norwegian": "no",
                     "Danish": "dk", "Finnish": "fi", "Polish": "pl", "Portuguese": "pt", "Czech": "cz"]
        return known[name] ?? known.first { name.hasPrefix($0.key) }?.value
    }
}

/// Overrides for the guest values, from the app's settings (config.json `guest`)
public struct GuestOverrides: Codable, Sendable, Equatable {
    public var user: String?
    public var fullName: String?
    public var email: String?
    public var timeZone: String?
    public var locale: String?
    public var keyboard: String?

    public init(user: String? = nil, fullName: String? = nil, email: String? = nil, timeZone: String? = nil,
                locale: String? = nil, keyboard: String? = nil) {
        self.user = user
        self.fullName = fullName
        self.email = email
        self.timeZone = timeZone
        self.locale = locale
        self.keyboard = keyboard
    }
}

extension ImageStore {
    /// Arguments for a `nix` call on the image flake: the guest values as its `omaflake-guest` input and the recipes
    /// of the sources as `omaflake-recipes` (RecipeLibrary.stage), without writing the lock file (it would get the
    /// data folder's paths). For a local folder an input is passed only when its flake.nix declares it.
    public func guestArguments(flake: String) -> [String] {
        var declared = (guest: true, recipes: true)
        if flake.hasPrefix("/") {
            let nix = (try? String(contentsOfFile: (flake as NSString).appendingPathComponent("flake.nix"), encoding: .utf8)) ?? ""
            declared = (nix.contains("omaflake-guest"), nix.contains("omaflake-recipes"))
        }
        var args: [String] = []
        if declared.guest, let dir = try? GuestSettings.write(paths) {
            args += ["--override-input", "omaflake-guest", "path:\(dir.path)"]
        }
        if declared.recipes, let dir = try? RecipeLibrary(paths: paths).stage(Settings.load(paths)) {
            args += ["--override-input", "omaflake-recipes", "path:\(dir.path)"]
        }
        return args.isEmpty ? [] : args + ["--no-write-lock-file"]
    }
}
