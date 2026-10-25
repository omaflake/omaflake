// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// `initialize` params (api.md §2)
public struct InitializeParams: Encodable, Sendable, Equatable {
    public var client: String
    public var clientVersion: String
    public var apiVersion: Int
    public var language: String?
    /// The update feed of this build (Info.plist OMUpdateFeedURL) – `updates.check` asks it
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
        /// "aarch64-darwin", …
        public var system: String
        /// "macOS 27.0.1", …
        public var os: String
    }

    public var apiVersion: Int
    public var omafVersion: String
    public var host: Host
    public var dataFolder: String
    public var configFile: String
}

/// `settings.get`
public struct SettingsInfo: Decodable, Sendable, Equatable {
    public struct Choices: Decodable, Sendable, Equatable {
        public struct Keyboard: Decodable, Sendable, Equatable, Hashable {
            /// XKB name ("de")
            public var code: String
            /// The locale whose name names it ("de_DE")
            public var locale: String
        }

        /// A guest look (plan B135)
        public struct Look: Decodable, Sendable, Equatable, Hashable {
            /// "rose-pine-dawn"
            public var name: String
            /// "Rosé Pine Dawn"
            public var title: String
        }

        public var timeZones: [String]
        public var locales: [String]
        public var keyboards: [Keyboard]
        /// nil from a core before looks
        public var looks: [Look]?

        public init() {
            timeZones = []
            locales = []
            keyboards = []
            looks = nil
        }
    }

    /// config.json's settings with their defaults applied
    public var settings: CoreSettings
    /// This computer's values the guest overrides fall back to
    public var guestDefaults: GuestValues
    public var choices: Choices
    public var isDefaultDataFolder: Bool

    public init() {
        settings = CoreSettings()
        guestDefaults = GuestValues()
        choices = Choices()
        isDefaultDataFolder = true
    }
}

/// config.json's settings with their defaults applied (`settings.get`, `settings.set`)
public struct CoreSettings: Decodable, Sendable, Equatable {
    public var developerMode = false
    public var checkForUpdates = true
    public var notifications = true
    public var keepRecipeVersions = false
    /// The guest values set here (an unset one is the computer's, `guestDefaults`)
    public var guest = GuestValues()
    public var flake: String?
    public var qemu: String?
    public var nix: String?
    public var runner: String?
    /// The image flake in use (the setting, `VM_FLAKE` or the bundled one)
    public var flakeInUse: String?
    /// The data folder now (after `dataFolder.move` the one `initialize` named is old); "" before the first answer
    public var dataFolder = ""

    public init() {}

    enum CodingKeys: String, CodingKey {
        case developerMode, checkForUpdates, notifications, keepRecipeVersions, guest, flake, qemu, nix, runner, flakeInUse
        case dataFolder
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        developerMode = try c.decode(Bool.self, forKey: .developerMode)
        checkForUpdates = try c.decode(Bool.self, forKey: .checkForUpdates)
        notifications = try c.decode(Bool.self, forKey: .notifications)
        keepRecipeVersions = try c.decode(Bool.self, forKey: .keepRecipeVersions)
        guest = try c.decode(GuestValues.self, forKey: .guest)
        flake = try c.decodeIfPresent(String.self, forKey: .flake)
        qemu = try c.decodeIfPresent(String.self, forKey: .qemu)
        nix = try c.decodeIfPresent(String.self, forKey: .nix)
        runner = try c.decodeIfPresent(String.self, forKey: .runner)
        flakeInUse = try c.decodeIfPresent(String.self, forKey: .flakeInUse)
        dataFolder = try c.decode(String.self, forKey: .dataFolder)
    }
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
    /// The guests' look (plan B135); nil = each recipe's own – there is no Mac's look
    public var look: String?

    public init(user: String? = nil, fullName: String? = nil, email: String? = nil, timeZone: String? = nil,
                locale: String? = nil, keyboard: String? = nil, look: String? = nil) {
        self.user = user
        self.fullName = fullName
        self.email = email
        self.timeZone = timeZone
        self.locale = locale
        self.keyboard = keyboard
        self.look = look
    }
}

/// `updates.check`
public struct UpdateStatus: Decodable, Sendable, Equatable {
    public enum State: String, OpenEnum { case noFeed, upToDate, available, off, unknown }
    public struct Release: Decodable, Sendable, Equatable {
        public var version: String
        public var url: String
        public var sha256: String?
        public var notes: String?
        public var page: String?
    }

    public var state: State
    public var release: Release?
}
