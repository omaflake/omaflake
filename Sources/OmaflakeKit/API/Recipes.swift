// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

// MARK: - Recipe (§4.3)

public struct Recipe: Decodable, Sendable, Equatable, Identifiable {
    /// A downloaded recipe: where it came from, its catalog version, when
    public struct Installed: Decodable, Sendable, Equatable {
        public var source: String
        public var version: String
        public var installed: Date
    }

    /// One version of a recipe's line (plan B111)
    public struct Version: Decodable, Sendable, Equatable, Identifiable {
        public var id: String
        public var source: String
        /// When it came (downloaded, imported, kept at a build); nil for the recipe a local folder has now
        public var added: Date?
        /// The line's newest
        public var isNewest: Bool
        /// It is the recipe in a local folder source (changed there, not removed here)
        public var isLive: Bool
        /// Images built from it – the version is kept while there are any
        public var images: [String]
        /// Its omaflake ID as short as it can be
        public var label: String

        enum CodingKeys: String, CodingKey {
            case id, source, added, isNewest = "newest", isLive = "live", images, label
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
    /// recipe.toml's name, else the display name
    public var name: String
    /// recipe.toml's author ("" when it has none)
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
    public var base: String
    /// "omaflake/nixos/dev"
    public var qualified: String
    public var displayName: String
    public var label: String
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
        kind = try c.decode(GuestKind.self, forKey: .kind)
        systems = try c.decode([String].self, forKey: .systems)
        cpus = try c.decode(Int.self, forKey: .cpus)
        memoryGB = try c.decode(Int.self, forKey: .memoryGB)
        diskGB = try c.decode(Int.self, forKey: .diskGB)
        network = try c.decode(NetworkMode.self, forKey: .network)
        share = try c.decode(Bool.self, forKey: .share)
        ephemeral = try c.decode(Bool.self, forKey: .ephemeral)
        user = try c.decodeIfPresent(String.self, forKey: .user)
        audio = try c.decodeIfPresent(Bool.self, forKey: .audio)
        microphone = try c.decodeIfPresent(Bool.self, forKey: .microphone)
        clipboard = try c.decodeIfPresent(Bool.self, forKey: .clipboard)
        camera = try c.decodeIfPresent(Bool.self, forKey: .camera)
        description = try c.decodeIfPresent(String.self, forKey: .description)
        lab = try c.decode(Bool.self, forKey: .lab)
        display = try c.decodeIfPresent(DisplayMode.self, forKey: .display)
        tags = try c.decode([String].self, forKey: .tags)
        license = try c.decodeIfPresent(String.self, forKey: .license)
        homepage = try c.decodeIfPresent(String.self, forKey: .homepage)
        base = try c.decode(String.self, forKey: .base)
        qualified = try c.decode(String.self, forKey: .qualified)
        displayName = try c.decode(String.self, forKey: .displayName)
        label = try c.decode(String.self, forKey: .label)
        runsHere = try c.decode(Bool.self, forKey: .runsHere)
        source = try c.decodeIfPresent(String.self, forKey: .source)
        line = try c.decode(String.self, forKey: .line)
        distinction = try c.decodeIfPresent(String.self, forKey: .distinction)
        folder = try c.decodeIfPresent(String.self, forKey: .folder)
        installed = try c.decodeIfPresent(Installed.self, forKey: .installed)
        image = try c.decodeIfPresent(String.self, forKey: .image)
        outdated = try c.decode(Bool.self, forKey: .outdated)
        versions = try c.decode([Version].self, forKey: .versions)
        // Optional in recipe.toml – the names the UI falls back to
        name = try c.decodeIfPresent(String.self, forKey: .name) ?? displayName
        author = try c.decodeIfPresent(String.self, forKey: .author) ?? ""
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
    }

    public var recipes: [Recipe]
    /// Line → its versions, newest first
    public var lines: [String: [Recipe.Version]]
    public var idLengths: IDLengths
    public var qemuGL: Bool
    public var unreadable: [Unreadable]
    /// Lab recipes are left out (developer mode off and no `all`)
    public var labHidden: Bool

    public init(recipes: [Recipe] = []) {
        self.recipes = recipes
        lines = [:]
        idLengths = IDLengths()
        qemuGL = false
        unreadable = []
        labHidden = true
    }
}

/// `recipes.removeLine`
public struct LineRemoval: Decodable, Sendable, Equatable {
    public var removed: [String]
    /// Versions images are built from, or that live in a local folder
    public var kept: [String]
}

/// `recipes.pruneVersions`
public struct Pruned: Decodable, Sendable, Equatable { public var removed: [String] }

// MARK: - Sources and catalogs (§4.4)

public struct Source: Decodable, Sendable, Equatable, Identifiable {
    public enum Kind: String, OpenEnum { case github, url, folder, unknown }

    public var id: String
    public var kind: Kind
    public var location: String
    public var branch: String?
    public var displayName: String
    public var enabled: Bool
    public var `private`: Bool
    /// omaflake/recipes
    public var official: Bool
    /// A private repository's token is in the Keychain
    public var hasToken: Bool
}

/// `sources.add`
public struct SourceAdded: Decodable, Sendable, Equatable {
    public var source: Source
    public var note: String
}

/// `sources.check`
public struct SourceChecks: Decodable, Sendable, Equatable {
    public struct Check: Decodable, Sendable, Equatable {
        public var ok: Bool
        public var note: String
    }

    /// Source id → its check
    public var results: [String: Check]
}

/// An entry of a source's catalog.json (spec §11.3) plus `source` and `runsHere`
public struct CatalogRecipe: Decodable, Sendable, Equatable, Identifiable {
    public struct File: Decodable, Sendable, Equatable {
        public var path: String
        public var size: Int64
        public var sha256: String
    }

    public var id: String
    public var author: String
    public var kind: GuestKind
    public var name: String
    /// In the session's language when the catalog has it
    public var description: String
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
}
