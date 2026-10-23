// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// `initialize` params (api.md §2)
public struct InitializeParams: Encodable, Sendable, Equatable {
    public var client: String
    public var clientVersion: String
    public var apiVersion: Int
    public var language: String?
    /// The update feed of this build (Info.plist OMUpdateFeedURL; OPEN in api.md: proposed)
    public var updateFeed: String?

    public init(client: String, clientVersion: String, apiVersion: Int = CoreClient.apiVersion, language: String? = nil,
                updateFeed: String? = nil) {
        self.client = client
        self.clientVersion = clientVersion
        self.apiVersion = apiVersion
        self.language = language
        self.updateFeed = updateFeed
    }
}

public struct InitializeResult: Decodable, Sendable, Equatable {
    public struct Host: Decodable, Sendable, Equatable {
        public var system: String?
        public var os: String?
    }

    public var apiVersion: Int
    public var omafVersion: String
    public var host: Host?
    public var dataFolder: String
    public var configFile: String?
}

/// `settings.get`
public struct SettingsInfo: Decodable, Sendable, Equatable {
    public struct Choices: Decodable, Sendable, Equatable {
        public struct Keyboard: Decodable, Sendable, Equatable, Hashable {
            /// XKB name ("de")
            public var code: String
            /// The locale whose name names it ("de_DE")
            public var locale: String?

            /// A string ("de") or an object ({code, locale})
            public init(from decoder: Decoder) throws {
                if let code = try? decoder.singleValueContainer().decode(String.self) {
                    self.code = code
                    locale = nil
                    return
                }
                if var pair = try? decoder.unkeyedContainer() {   // a tuple as [code, locale]
                    code = try pair.decode(String.self)
                    locale = try? pair.decode(String.self)
                    return
                }
                let c = try decoder.container(keyedBy: CodingKeys.self)
                code = try c.decode(String.self, forKey: .code)
                locale = try c.decodeIfPresent(String.self, forKey: .locale)
            }

            enum CodingKeys: String, CodingKey { case code, locale }
        }

        public var timeZones: [String]
        public var locales: [String]
        public var keyboards: [Keyboard]

        public init() {
            timeZones = []
            locales = []
            keyboards = []
        }

        enum CodingKeys: String, CodingKey { case timeZones, locales, keyboards }
        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            timeZones = try c.value(.timeZones, [])
            locales = try c.value(.locales, [])
            keyboards = try c.value(.keyboards, [])
        }
    }

    /// config.json as it is (only what the app shows is read: `Settings`)
    public var settings: CoreSettings
    /// This computer's values the guest overrides fall back to
    public var guestDefaults: GuestValues
    public var choices: Choices
    public var isDefaultDataFolder: Bool
    /// The data folder now (OPEN in api.md: proposed – after `dataFolder.move` the one of `initialize` is old)
    public var dataFolder: String?

    public init() {
        settings = CoreSettings()
        guestDefaults = GuestValues()
        choices = Choices()
        isDefaultDataFolder = true
    }

    enum CodingKeys: String, CodingKey { case settings, guestDefaults, choices, isDefaultDataFolder, dataFolder }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        settings = try c.value(.settings, CoreSettings())
        guestDefaults = try c.value(.guestDefaults, GuestValues())
        choices = try c.value(.choices, Choices())
        isDefaultDataFolder = try c.value(.isDefaultDataFolder, true)
        dataFolder = try c.decodeIfPresent(String.self, forKey: .dataFolder)
    }
}

/// The settings of config.json the app shows (unset = the default)
public struct CoreSettings: Decodable, Sendable, Equatable {
    public var developerMode: Bool?
    public var checkForUpdates: Bool?
    public var notifications: Bool?
    public var keepRecipeVersions: Bool?
    public var guest: GuestValues?
    public var flake: String?
    public var qemu: String?
    public var nix: String?
    public var runner: String?
    public var home: String?

    public init() {}
}

/// Guest user values: the overrides in config.json (`guest`, every field optional – nil = this Mac's value) and the
/// Mac's own values (`guestDefaults`)
public struct GuestValues: Codable, Sendable, Equatable {
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

/// `updates.check`
public struct UpdateStatus: Decodable, Sendable, Equatable {
    public enum State: String, OpenEnum { case noFeed, upToDate, available, off, unknown }
    public struct Release: Decodable, Sendable, Equatable {
        public var version: String
        public var url: String?
        public var sha256: String?
        public var notes: String?
        public var page: String?
    }

    public var state: State
    public var release: Release?
}
