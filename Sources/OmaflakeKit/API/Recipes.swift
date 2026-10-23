// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

// MARK: - Recipe (§4.3)

public struct Recipe: Decodable, Sendable, Equatable, Identifiable {
    /// A downloaded recipe: where it came from, its catalog version, when
    public struct Installed: Decodable, Sendable, Equatable {
        public var source: String
        public var version: String
        public var installed: Date?

        enum CodingKeys: String, CodingKey { case source, version, installed }
        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            source = try c.value(.source, "")
            version = try c.value(.version, "")
            installed = c.lenientDate(.installed)
        }
    }

    /// One version of a recipe's line (plan B111)
    public struct Version: Decodable, Sendable, Equatable, Identifiable {
        public var id: String
        public var source: String?
        /// When it came (downloaded, imported, kept at a build); nil for the recipe a local folder has now
        public var added: Date?
        /// The line's newest
        public var isNewest: Bool
        /// It is the recipe in a local folder source (changed there, not removed here)
        public var isLive: Bool
        /// Images built from it – the version is kept while there are any
        public var images: [String]
        /// Its omaflake ID as short as it can be, when the server sends it
        public var label: String?

        enum CodingKeys: String, CodingKey { case id, source, added, isNewest, isLive, images, label }
        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            id = try c.decode(String.self, forKey: .id)
            source = try c.decodeIfPresent(String.self, forKey: .source)
            added = c.lenientDate(.added)
            isNewest = try c.value(.isNewest, false)
            isLive = try c.value(.isLive, false)
            images = try c.value(.images, [])
            label = try c.decodeIfPresent(String.self, forKey: .label)
        }
    }

    // recipe.toml
    public var kind: GuestKind
    public var systems: [String]
    public var cpus: Int
    public var memoryGB: Int
    public var diskGB: Int
    public var network: NetworkMode
    public var share: Bool
    public var ephemeral: Bool
    public var user: String?
    public var name: String
    public var author: String
    public var audio: Bool?
    public var microphone: Bool?
    public var clipboard: Bool?
    public var camera: Bool?
    /// In the session's language when the recipe has it
    public var description: String?
    public var lab: Bool
    public var display: DisplayMode?
    public var tags: [String]
    public var license: String?
    public var homepage: String?
    // plus
    public var id: String
    public var base: String?
    /// "omaflake/nixos/dev"
    public var qualified: String?
    public var displayName: String
    public var label: String?
    public var runsHere: Bool
    /// Source id
    public var source: String?
    public var line: String
    /// What tells it apart from a recipe of the same author/kind/name (plan B110)
    public var distinction: String?
    public var folder: String?
    public var installed: Installed?
    /// Its line's current image
    public var image: String?
    /// The recipe changed since that image was built (plan B73/B82)
    public var outdated: Bool
    /// Its line's versions, newest first
    public var versions: [Version]

    enum CodingKeys: String, CodingKey {
        case kind, systems, cpus, memoryGB, diskGB, network, share, ephemeral, user, name, author, audio, microphone
        case clipboard, camera, description, lab, display, tags, license, homepage, id, base, qualified, displayName
        case label, runsHere, source, line, distinction, folder, installed, image, outdated, versions
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        kind = try c.value(.kind, .unknown)
        systems = try c.value(.systems, [])
        cpus = try c.value(.cpus, 4)
        memoryGB = try c.value(.memoryGB, 4)
        diskGB = try c.value(.diskGB, 40)
        network = try c.value(.network, .shared)
        share = try c.value(.share, false)
        ephemeral = try c.value(.ephemeral, false)
        user = try c.decodeIfPresent(String.self, forKey: .user)
        name = try c.value(.name, id)
        author = try c.value(.author, "")
        audio = try c.decodeIfPresent(Bool.self, forKey: .audio)
        microphone = try c.decodeIfPresent(Bool.self, forKey: .microphone)
        clipboard = try c.decodeIfPresent(Bool.self, forKey: .clipboard)
        camera = try c.decodeIfPresent(Bool.self, forKey: .camera)
        description = try c.decodeIfPresent(String.self, forKey: .description)
        lab = try c.value(.lab, false)
        display = try c.decodeIfPresent(DisplayMode.self, forKey: .display)
        tags = try c.value(.tags, [])
        license = try c.decodeIfPresent(String.self, forKey: .license)
        homepage = try c.decodeIfPresent(String.self, forKey: .homepage)
        base = try c.decodeIfPresent(String.self, forKey: .base)
        qualified = try c.decodeIfPresent(String.self, forKey: .qualified)
        displayName = try c.value(.displayName, name)
        label = try c.decodeIfPresent(String.self, forKey: .label)
        runsHere = try c.value(.runsHere, true)
        source = try c.decodeIfPresent(String.self, forKey: .source)
        line = try c.value(.line, id)
        distinction = try c.decodeIfPresent(String.self, forKey: .distinction)
        folder = try c.decodeIfPresent(String.self, forKey: .folder)
        installed = try c.decodeIfPresent(Installed.self, forKey: .installed)
        image = try c.decodeIfPresent(String.self, forKey: .image)
        outdated = try c.value(.outdated, false)
        versions = try c.value(.versions, [])
    }

    /// What a new VM from it gets (`vms.create`)
    public var settings: RecipeSettings {
        RecipeSettings(kind: kind, cpus: cpus, memoryGB: memoryGB, diskGB: diskGB, network: network, share: share,
                       ephemeral: ephemeral, user: user, audio: audio ?? true, microphone: microphone ?? false,
                       clipboard: clipboard ?? true, camera: camera ?? false, display: display)
    }
}

/// The runtime values of a recipe – what `vms.create` takes
public struct RecipeSettings: Codable, Sendable, Equatable {
    public var kind: GuestKind
    public var cpus: Int
    public var memoryGB: Int
    public var diskGB: Int
    public var network: NetworkMode
    public var share: Bool
    public var ephemeral: Bool
    public var user: String?
    public var audio: Bool
    public var microphone: Bool
    public var clipboard: Bool
    public var camera: Bool
    public var display: DisplayMode?

    public init(kind: GuestKind = .nixos, cpus: Int = 4, memoryGB: Int = 4, diskGB: Int = 40, network: NetworkMode = .shared,
                share: Bool = true, ephemeral: Bool = false, user: String? = nil, audio: Bool = true, microphone: Bool = false,
                clipboard: Bool = true, camera: Bool = false, display: DisplayMode? = nil) {
        self.kind = kind
        self.cpus = cpus
        self.memoryGB = memoryGB
        self.diskGB = diskGB
        self.network = network
        self.share = share
        self.ephemeral = ephemeral
        self.user = user
        self.audio = audio
        self.microphone = microphone
        self.clipboard = clipboard
        self.camera = camera
        self.display = display
    }
}

/// `recipes.list` and the `recipes.changed` event
public struct RecipeList: Decodable, Sendable, Equatable {
    /// How many words omaflake IDs show, per kind (plan B111)
    public struct IDLengths: Decodable, Sendable, Equatable {
        public var recipe = 1, image = 1, vm = 1
        public init() {}
        enum CodingKeys: String, CodingKey { case recipe, image, vm }
        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            recipe = try c.value(.recipe, 1)
            image = try c.value(.image, 1)
            vm = try c.value(.vm, 1)
        }
    }

    public var recipes: [Recipe]
    public var lines: [String: [Recipe.Version]]
    public var idLengths: IDLengths
    public var qemuGL: Bool
    public var unreadable: [Unreadable]

    public init(recipes: [Recipe] = []) {
        self.recipes = recipes
        lines = [:]
        idLengths = IDLengths()
        qemuGL = false
        unreadable = []
    }

    enum CodingKeys: String, CodingKey { case recipes, lines, idLengths, qemuGL, unreadable }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        recipes = try c.value(.recipes, [])
        lines = try c.value(.lines, [:])
        idLengths = try c.value(.idLengths, IDLengths())
        qemuGL = try c.value(.qemuGL, false)
        unreadable = try c.value(.unreadable, [])
    }
}

// MARK: - Sources and catalogs (§4.4)

public struct Source: Decodable, Sendable, Equatable, Identifiable {
    public enum Kind: String, OpenEnum { case github, https, folder, unknown }

    public var id: String
    public var kind: Kind
    public var location: String
    public var branch: String?
    public var displayName: String
    public var enabled: Bool
    public var `private`: Bool
    /// omaflake/recipes
    public var official: Bool
    /// A private repository's token is in the Keychain (nil: the server does not say – OPEN in api.md: proposed)
    public var hasToken: Bool?

    enum CodingKeys: String, CodingKey { case id, kind, location, branch, displayName, enabled, `private`, official, hasToken }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        // "url" was the Swift core's name for an HTTPS catalog
        let rawKind = try c.value(.kind, "")
        kind = rawKind == "url" ? .https : Kind(rawValue: rawKind) ?? .unknown
        location = try c.value(.location, "")
        branch = try c.decodeIfPresent(String.self, forKey: .branch)
        displayName = try c.value(.displayName, location)
        enabled = try c.value(.enabled, true)
        `private` = try c.value(.private, false)
        official = try c.value(.official, false)
        hasToken = try c.decodeIfPresent(Bool.self, forKey: .hasToken)
    }
}

/// An entry of a source's catalog.json (spec §11.3) plus `source` and `runsHere`
public struct CatalogRecipe: Decodable, Sendable, Equatable, Identifiable {
    public struct File: Decodable, Sendable, Equatable {
        public var path: String
        public var size: Int64
        public var sha256: String?
    }

    public var id: String
    public var author: String
    public var kind: GuestKind
    public var name: String
    public var path: String?
    public var description: String
    public var descriptions: [String: String]?
    public var tags: [String]
    public var license: String?
    public var lab: Bool
    public var systems: [String]
    public var requires: [String]
    public var homepage: String?
    public var version: String
    public var files: [File]
    /// Source id
    public var source: String
    public var runsHere: Bool

    enum CodingKeys: String, CodingKey {
        case id, author, kind, name, path, description, descriptions, tags, license, lab, systems, requires, homepage
        case version, files, source, runsHere
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        author = try c.value(.author, "")
        kind = try c.value(.kind, .unknown)
        name = try c.value(.name, id)
        path = try c.decodeIfPresent(String.self, forKey: .path)
        description = try c.value(.description, "")
        descriptions = try c.decodeIfPresent([String: String].self, forKey: .descriptions)
        tags = try c.value(.tags, [])
        license = try c.decodeIfPresent(String.self, forKey: .license)
        lab = try c.value(.lab, false)
        systems = try c.value(.systems, [])
        requires = try c.value(.requires, [])
        homepage = try c.decodeIfPresent(String.self, forKey: .homepage)
        version = try c.value(.version, "")
        files = try c.value(.files, [])
        source = try c.value(.source, "")
        runsHere = try c.value(.runsHere, true)
    }

    /// The description in `language` when the catalog has it
    public func description(in language: String?) -> String { language.flatMap { descriptions?[$0] } ?? description }

    /// "omaflake/nixos/dev" from the ID "omaflake.nixos.dev"
    public var qualified: String { [author, kind.rawValue, name].joined(separator: "/") }
}

/// `recipes.available`
public struct AvailableRecipes: Decodable, Sendable, Equatable {
    public struct Update: Decodable, Sendable, Equatable {
        /// The installed recipe that has a newer version
        public var id: String
        public var recipe: CatalogRecipe
    }

    public var offered: [CatalogRecipe]
    public var updates: [Update]
    public var suggested: [CatalogRecipe]
    /// sourceId → message
    public var errors: [String: String]

    public init() {
        offered = []
        updates = []
        suggested = []
        errors = [:]
    }

    enum CodingKeys: String, CodingKey { case offered, updates, suggested, errors }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        offered = try c.value(.offered, [])
        updates = try c.value(.updates, [])
        suggested = try c.value(.suggested, [])
        errors = try c.value(.errors, [:])
    }
}
