// SPDX-License-Identifier: AGPL-3.0-or-later
import CryptoKit
import Foundation
import Security

// Recipe sources (spec §11.3, plan B107): omaflake ships no recipes. It reads them from sources – omaflake/recipes on
// GitHub by default, other GitHub repositories (also private ones, with a fine-grained token), any HTTPS folder with a
// catalog.json, or a local folder. Recipes from a remote source are downloaded (every file checked against the
// catalog's SHA-256) into <data folder>/recipes/<author>/<kind>/<name>/; a local folder is used as it is, so a
// recipe being written there is built as it is on disk. Before every Nix call the recipes are put together into one
// folder that the image flake gets as its input `omaflake-recipes`.

/// Where recipes come from. Kept in config.json (`sources`); a token for a private repository lives in the Keychain.
public struct RecipeSource: Codable, Sendable, Hashable, Identifiable {
    public enum Kind: String, Codable, Sendable { case github, url, folder }

    public var kind: Kind
    /// github: "owner/repo"; url: the HTTPS URL of catalog.json; folder: an absolute path
    public var location: String
    /// github: the branch (nil = main)
    public var branch: String?
    /// github: the repository is private – requests carry the token from the Keychain
    public var isPrivate: Bool?
    /// nil = on
    public var enabled: Bool?

    enum CodingKeys: String, CodingKey {
        case kind, location, branch, enabled
        case isPrivate = "private"
    }

    public init(kind: Kind, location: String, branch: String? = nil, isPrivate: Bool? = nil, enabled: Bool? = nil) {
        self.kind = kind
        self.location = location
        self.branch = branch
        self.isPrivate = isPrivate
        self.enabled = enabled
    }

    /// The default source: omaflake's own recipes
    public static let official = RecipeSource(kind: .github, location: "omaflake/recipes")

    public var id: String {
        switch kind {
        case .github: "github:\(location.lowercased())@\(branch ?? "main")"
        case .url: "url:\(location)"
        case .folder: "folder:\(location)"
        }
    }
    /// Short, file-name-safe key of the source in recipe IDs (plan B110): the first 8 hex digits of SHA-256 of `id`
    public var tag: String { Self.tag(forKey: id) }

    public static func tag(forKey key: String) -> String {
        SHA256.hash(data: Data(key.utf8)).prefix(4).map { String(format: "%02x", $0) }.joined()
    }

    public var isEnabled: Bool { enabled ?? true }
    public var isPrivateRepository: Bool { kind == .github && (isPrivate ?? false) }
    /// omaflake/recipes on GitHub – its recipes by `omaflake` are marked as official (an author name alone is not enough)
    public var isOfficial: Bool { kind == .github && location.lowercased() == "omaflake/recipes" }

    /// Short name for lists
    public var displayName: String {
        switch kind {
        case .github: branch.map { "\(location) (\($0))" } ?? location
        case .url: URL(string: location).map { ($0.host ?? "") + $0.deletingLastPathComponent().path } ?? location
        case .folder: (location as NSString).abbreviatingWithTildeInPath
        }
    }

    /// What the user typed: a GitHub repository ("https://github.com/owner/repo", "github.com/owner/repo/tree/branch",
    /// "owner/repo"), an HTTPS URL of a catalog.json (or of its folder), or a folder on the Mac.
    public static func parse(_ input: String, isPrivate: Bool = false) throws -> RecipeSource {
        let text = input.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !text.isEmpty else { throw VMError(String(localized: "Enter a GitHub repository, an HTTPS URL or a folder")) }
        if text.hasPrefix("/") || text.hasPrefix("~") || text.hasPrefix("file://") {
            let path = text.hasPrefix("file://") ? (URL(string: text)?.path ?? text) : (text as NSString).expandingTildeInPath
            let url = URL(fileURLWithPath: path).standardizedFileURL
            var isDir: ObjCBool = false
            guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir), isDir.boolValue else {
                throw VMError(String(localized: "No such folder: \(url.path)"))
            }
            return RecipeSource(kind: .folder, location: url.path)
        }
        let name = "[A-Za-z0-9][A-Za-z0-9._-]*"
        let github = try Regex("^(?:https?://)?(?:www\\.)?github\\.com/(\(name))/(\(name))(?:\\.git)?(?:/tree/([^/?#]+))?/?$")
        if let m = try github.wholeMatch(in: text) {
            let repo = String(m.output[2].substring ?? "").replacingOccurrences(of: ".git", with: "")
            return RecipeSource(kind: .github, location: "\(m.output[1].substring ?? "")/\(repo)",
                                branch: m.output[3].substring.map(String.init), isPrivate: isPrivate ? true : nil)
        }
        if try Regex("^\(name)/\(name)$").wholeMatch(in: text) != nil {
            return RecipeSource(kind: .github, location: text, isPrivate: isPrivate ? true : nil)
        }
        guard var url = URL(string: text), url.scheme?.lowercased() == "https", url.host != nil else {
            throw VMError(String(localized: "Only HTTPS addresses, GitHub repositories and folders can be recipe sources"))
        }
        if url.lastPathComponent != "catalog.json" { url = url.appendingPathComponent("catalog.json") }
        return RecipeSource(kind: .url, location: url.absoluteString)
    }
}

extension Settings {
    /// The sources in their order (earlier ones win when two have the same recipe); none set = omaflake/recipes
    public var recipeSources: [RecipeSource] { sources ?? [.official] }
}

// MARK: Tokens

/// Fine-grained GitHub tokens for private sources, in the login Keychain – never in config.json or a file.
public enum SourceTokens {
    static let service = "app.omaflake.recipe-source"

    public static func token(for source: RecipeSource) -> String? {
        var result: CFTypeRef?
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service,
                                    kSecAttrAccount as String: source.id, kSecReturnData as String: true,
                                    kSecMatchLimit as String: kSecMatchLimitOne]
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess, let data = result as? Data else { return nil }
        return String(data: data, encoding: .utf8)
    }

    /// Stores the token (nil removes it)
    public static func set(_ token: String?, for source: RecipeSource) throws {
        let key: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service,
                                  kSecAttrAccount as String: source.id]
        SecItemDelete(key as CFDictionary)
        guard let token, !token.isEmpty else { return }
        var item = key
        item[kSecValueData as String] = Data(token.utf8)
        item[kSecAttrLabel as String] = "omaflake recipe source \(source.displayName)"
        let status = SecItemAdd(item as CFDictionary, nil)
        guard status == errSecSuccess else {
            throw VMError(String(localized: "Cannot store the token in the Keychain (\(status))"))
        }
    }
}

// MARK: Catalog

/// catalog.json, version 2 (omaflake/recipes, tools/catalog.py). Everything in it comes from the source – untrusted:
/// paths and names are checked before anything is written.
public struct RecipeCatalog: Decodable, Sendable {
    public struct Source: Decodable, Sendable { public var name: String; public var description: String?; public var homepage: String? }
    public struct File: Decodable, Sendable, Equatable { public var path: String; public var size: Int; public var sha256: String }
    public struct Recipe: Decodable, Sendable, Identifiable {
        public var id: String
        public var author: String
        public var kind: GuestKind
        public var name: String
        public var path: String
        public var description: String
        public var descriptions: [String: String]?
        public var tags: [String]?
        public var license: String?
        public var lab: Bool?
        /// What it can be built for (`Platform`, schema 2)
        public var systems: [String]
        public var requires: [String]?
        public var homepage: String?
        public var version: String
        public var files: [File]

        public func description(in language: String?) -> String { language.flatMap { descriptions?[$0] } ?? description }

        /// It can be built and run on this host
        public var runsHere: Bool { Platform.hostSystem(for: kind).map(systems.contains) ?? false }

        /// The recipe's content hash from the catalog's file list – the same as `RecipeLibrary.contentHash` of its
        /// folder, so whether a recipe is here already is known before it is downloaded (plan B110)
        public var contentHash: String {
            var hasher = SHA256()
            for f in files.sorted(by: { $0.path < $1.path }) {
                hasher.update(data: Data(f.path.utf8 + [0]))
                hasher.update(data: Self.bytes(hex: f.sha256.lowercased()))
            }
            return hasher.finalize().map { String(format: "%02x", $0) }.joined()
        }

        static func bytes(hex: String) -> Data {
            var data = Data()
            var i = hex.startIndex
            while i < hex.endIndex, let j = hex.index(i, offsetBy: 2, limitedBy: hex.endIndex) {
                if let b = UInt8(hex[i..<j], radix: 16) { data.append(b) }
                i = j
            }
            return data
        }
    }

    /// The catalog's version this omaflake knows – `$schema` https://omaflake.dev/schema/catalog/v2.json (2: `systems`
    /// per recipe, plan B114) – and the oldest it reads: version 1 (`"schema": 1`, before B114; its recipes ran on Apple
    /// silicon). A catalog comes from its source and is only read, never rewritten.
    public static let version = 2
    public static let minimum = 1
    public var source: Source
    public var recipes: [Recipe]

    public static func decode(_ data: Data) throws -> RecipeCatalog {
        guard var object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            throw VMError(String(localized: "Not a recipe catalog (catalog.json): \(String(localized: "not JSON"))"))
        }
        let found: Int
        if let schema = object["$schema"] as? String {
            guard let (type, v) = FileFormat.parse(schema), type == "catalog" else {
                throw VMError(String(localized: "Not a recipe catalog (catalog.json): \(schema)"))
            }
            found = v
        } else {
            found = (object["schema"] as? Int) ?? 0     // before B114
        }
        _ = try FileFormat.check(found, minimum: minimum, current: version, file: "catalog.json")
        if found < 2, let recipes = object["recipes"] as? [[String: Any]] {
            // Version 1 has no systems: its recipes were made for Apple silicon
            object["recipes"] = recipes.map { r -> [String: Any] in
                var r = r
                r["systems"] = [Platform.legacySystem(for: (r["kind"] as? String).flatMap(GuestKind.init) ?? .nixos)]
                return r
            }
        }
        let data = (try? JSONSerialization.data(withJSONObject: object)) ?? data
        do { return try JSONDecoder().decode(RecipeCatalog.self, from: data) } catch {
            throw VMError(String(localized: "Not a recipe catalog (catalog.json): \(error.localizedDescription)"))
        }
    }
}

/// "<author>/<kind>/<name>" or "<author>.<kind>.<name>" – both forms a recipe is named by
public enum RecipeRef {
    /// Lowercase letters, digits and "-", not starting with "-"
    static func validName(_ s: String) -> Bool {
        guard let first = s.unicodeScalars.first, first != "-" else { return false }
        return s.unicodeScalars.allSatisfy { ("a"..."z").contains($0) || ("0"..."9").contains($0) || $0 == "-" }
    }

    /// (author, kind, name) if the reference is well formed
    static func parts(_ ref: String) -> (String, GuestKind, String)? {
        let p = ref.split(separator: ref.contains("/") ? "/" : ".", omittingEmptySubsequences: false).map(String.init)
        guard p.count == 3, validName(p[0]), validName(p[2]), let kind = GuestKind(rawValue: p[1]) else { return nil }
        return (p[0], kind, p[2])
    }

    static func id(_ ref: String) -> String? { parts(ref).map { "\($0.0).\($0.1.rawValue).\($0.2)" } }

    /// For the CLI and the app: the internal ID of "<author>/<kind>/<name>" (or of an ID), nil if not well formed
    public static func publicID(_ ref: String) -> String? { id(ref) }
}

// MARK: Fetching

/// Reads from a remote source: public GitHub repositories over raw.githubusercontent.com, private ones over the
/// GitHub API with the token, other sources from their HTTPS folder. Only when asked (opening the Recipes tab,
/// installing) – never in the background.
public struct SourceClient: Sendable {
    public let source: RecipeSource
    let token: String?

    public init(_ source: RecipeSource, token: String? = nil) {
        self.source = source
        self.token = token ?? (source.isPrivateRepository ? SourceTokens.token(for: source) : nil)
    }

    func url(_ relative: String) throws -> URL {
        let branch = source.branch ?? "main"
        let encoded = relative.split(separator: "/").map {
            String($0).addingPercentEncoding(withAllowedCharacters: .urlPathAllowed) ?? String($0)
        }.joined(separator: "/")
        switch source.kind {
        case .github where source.isPrivateRepository:
            guard let u = URL(string: "https://api.github.com/repos/\(source.location)/contents/\(encoded)?ref=\(branch)") else { break }
            return u
        case .github:
            guard let u = URL(string: "https://raw.githubusercontent.com/\(source.location)/\(branch)/\(encoded)") else { break }
            return u
        case .url:
            guard let base = URL(string: source.location)?.deletingLastPathComponent() else { break }
            return base.appendingPathComponent(relative)
        case .folder:
            return URL(fileURLWithPath: source.location).appendingPathComponent(relative)
        }
        throw VMError(String(localized: "Not a valid address: \(source.location)"))
    }

    func request(_ url: URL) -> URLRequest {
        var r = URLRequest(url: url, cachePolicy: .reloadIgnoringLocalCacheData, timeoutInterval: 30)
        if source.isPrivateRepository {
            if let token { r.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
            r.setValue("application/vnd.github.raw", forHTTPHeaderField: "Accept")
            r.setValue("2022-11-28", forHTTPHeaderField: "X-GitHub-Api-Version")
        }
        r.setValue("omaflake", forHTTPHeaderField: "User-Agent")
        return r
    }

    /// One file of the source, relative to its catalog.json
    public func fetch(_ relative: String) async throws -> Data {
        let url = try url(relative)
        if source.kind == .folder { return try Data(contentsOf: url) }
        if source.isPrivateRepository && token == nil {
            throw VMError(String(localized: "\(source.displayName) is private – add a token in Settings → Recipe Sources"))
        }
        let (data, response) = try await URLSession.shared.data(for: request(url))
        let status = (response as? HTTPURLResponse)?.statusCode ?? 0
        switch status {
        case 200: return data
        case 401: throw VMError(String(localized: "\(source.displayName): the token is not valid (or has expired)"))
        case 403, 404 where source.isPrivateRepository:
            throw VMError(String(localized: "\(source.displayName): no access to \(relative) – the token needs read access to the repository's contents (Contents: Read-only), and the branch must exist"))
        case 404: throw VMError(String(localized: "\(source.displayName): \(relative) not found (is it a recipe source, and public?)"))
        default: throw VMError(String(localized: "\(source.displayName): \(relative) – HTTP \(status)"))
        }
    }

    public func catalog() async throws -> RecipeCatalog { try RecipeCatalog.decode(try await fetch("catalog.json")) }

    /// Checks that the source can be read: a private repository with its token (the repository first, then the
    /// catalog), a public one or a URL by its catalog, a folder by its recipes. Returns a short description.
    public func verify() async throws -> String {
        switch source.kind {
        case .folder:
            let n = RecipeLibrary.recipeFolders(in: URL(fileURLWithPath: source.location)).count
            guard n > 0 else {
                throw VMError(String(localized: "No recipes in this folder – it needs recipes/<author>/<kind>/<name>/recipe.toml"))
            }
            return String(localized: "\(n) recipes")
        case .github where source.isPrivateRepository:
            guard let token, !token.isEmpty else { throw VMError(String(localized: "Enter a fine-grained token with read access to the repository")) }
            var r = request(URL(string: "https://api.github.com/repos/\(source.location)")!)
            r.setValue("application/vnd.github+json", forHTTPHeaderField: "Accept")
            let (data, response) = try await URLSession.shared.data(for: r)
            switch (response as? HTTPURLResponse)?.statusCode ?? 0 {
            case 200: break
            case 401: throw VMError(String(localized: "The token is not valid (or has expired)"))
            default: throw VMError(String(localized: "The token has no access to \(source.location) – give it this repository with Contents: Read-only"))
            }
            let repo = try? JSONSerialization.jsonObject(with: data) as? [String: Any]
            let c = try await catalog()
            let visibility = (repo?["private"] as? Bool) == false ? String(localized: "public") : String(localized: "private")
            return String(localized: "\(c.source.name): \(c.recipes.count) recipes (\(visibility) repository, token works)")
        default:
            let c = try await catalog()
            return String(localized: "\(c.source.name): \(c.recipes.count) recipes")
        }
    }
}

// MARK: Library

/// The recipes omaflake can build: those downloaded from remote sources (<data folder>/recipes) and those in the
/// enabled local folders. `stage` puts them together for the image flake.
public struct RecipeLibrary: Sendable {
    public let paths: Paths

    public init(paths: Paths) { self.paths = paths }

    /// Limits for downloaded files (as tools/catalog.py checks them in the source)
    static let maxFile = 1024 * 1024, maxRecipe = 5 * 1024 * 1024

    public struct Installed: Codable, Sendable, Equatable {
        /// RecipeSource.id it came from
        public var source: String
        public var version: String
        public var requires: [String]?
        public var installed: Date
    }

    var manifestFile: URL { paths.recipes.appendingPathComponent(".installed.json") }

    /// .installed.json, schema `installed`: `{"$schema": …, "recipes": {<id>: Installed}}` (plan B114). 0 → 1: the
    /// map was the whole file
    public static let fileFormat = FileFormat("installed", file: ".installed.json", steps: [
        { o in o = ["recipes": o] },
    ])

    struct InstalledFile: Codable {
        var recipes: [String: Installed]
    }

    /// Downloaded recipes by ID
    public func installed() -> [String: Installed] {
        guard let data = try? Data(contentsOf: manifestFile) else { return [:] }
        let d = JSONDecoder()
        // With fractions of a second (versions of a line come in order, plan B111), older files without
        d.dateDecodingStrategy = .custom { decoder in
            let text = try decoder.singleValueContainer().decode(String.self)
            let precise = ISO8601DateFormatter()
            precise.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            guard let date = precise.date(from: text) ?? ISO8601DateFormatter().date(from: text) else {
                throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "date \(text)"))
            }
            return date
        }
        guard let read = try? Self.fileFormat.decode(InstalledFile.self, from: data, decoder: d) else { return [:] }
        if read.migrated { try? saveManifest(read.value.recipes) }
        return read.value.recipes
    }

    func saveManifest(_ m: [String: Installed]) throws {
        try FileManager.default.createDirectory(at: paths.recipes, withIntermediateDirectories: true)
        let e = JSONEncoder()
        e.dateEncodingStrategy = .custom { date, encoder in
            let f = ISO8601DateFormatter()
            f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
            var c = encoder.singleValueContainer()
            try c.encode(f.string(from: date))
        }
        e.outputFormatting = [.prettyPrinted, .sortedKeys]
        try Self.fileFormat.checkWritable(manifestFile)
        try Self.fileFormat.encode(InstalledFile(recipes: m), encoder: e).write(to: manifestFile, options: .atomic)
    }

    /// Where a downloaded or imported recipe lives: `<data>/recipes/<id>/` (plan B110; one folder per ID, so versions
    /// of one base sit side by side)
    public func folder(_ id: String) -> URL? {
        guard RecipeRef.parts(RecipeID.base(id)) != nil, RecipeID.words(id) != nil else { return nil }
        return paths.recipes.appendingPathComponent(id, isDirectory: true)
    }

    /// <author>/<kind>/<name> folders with a recipe.toml under `root` – a source's checkout (with recipes/) or a
    /// folder of recipe folders
    public static func recipeFolders(in root: URL) -> [(id: String, folder: URL)] {
        let fm = FileManager.default
        let base = fm.fileExists(atPath: root.appendingPathComponent("recipes").path) ? root.appendingPathComponent("recipes") : root
        func dirs(_ u: URL) -> [String] {
            ((try? fm.contentsOfDirectory(atPath: u.path)) ?? []).filter { name in
                var isDir: ObjCBool = false
                return !name.hasPrefix(".") && fm.fileExists(atPath: u.appendingPathComponent(name).path, isDirectory: &isDir) && isDir.boolValue
            }.sorted()
        }
        var out: [(String, URL)] = []
        for author in dirs(base) where RecipeRef.validName(author) {
            for kind in GuestKind.allCases {
                let kindDir = base.appendingPathComponent("\(author)/\(kind.rawValue)")
                for name in dirs(kindDir) where RecipeRef.validName(name) {
                    let folder = kindDir.appendingPathComponent(name)
                    if fm.fileExists(atPath: folder.appendingPathComponent("recipe.toml").path) {
                        out.append(("\(author).\(kind.rawValue).\(name)", folder))
                    }
                }
            }
        }
        return out
    }

    /// Every recipe omaflake can build and where it comes from: the enabled local folders first (in their order),
    /// then the downloaded ones. The first one with an ID wins.
    public func available(_ settings: Settings) -> [(id: String, folder: URL, source: String)] {
        var seen = Set<String>()
        var out: [(String, URL, String)] = []
        // The ID carries the content hash (12 words, unique worldwide): the same base with the same content is one
        // recipe – the first source's
        func add(_ base: String, _ folder: URL, _ source: String) {
            guard let hash = try? Self.contentHash(folder) else { return }
            let id = RecipeID.make(base: base, contentHash: hash)
            if seen.insert(id).inserted { out.append((id, folder, source)) }
        }
        for s in settings.recipeSources where s.isEnabled && s.kind == .folder {
            for (base, folder) in Self.recipeFolders(in: URL(fileURLWithPath: s.location)) { add(base, folder, s.id) }
        }
        for (id, entry) in installed().sorted(by: { $0.key < $1.key }) {
            guard let folder = folder(id), FileManager.default.fileExists(atPath: folder.path) else { continue }
            add(RecipeID.base(id), folder, entry.source)
        }
        return out
    }

    /// The recipe a recipe's `requires` means: the one of that base from the same source, otherwise the first one
    func requirement(_ base: String, source: String, in available: [(id: String, folder: URL, source: String)]) -> (id: String, folder: URL, source: String)? {
        available.first { RecipeID.base($0.id) == base && $0.source == source } ?? available.first { RecipeID.base($0.id) == base }
    }

    /// A source's name for lists: the source's own, "imported" recipes by where they are kept
    public func sourceName(_ source: String, settings: Settings) -> String {
        if source == Self.importedSource { return (paths.recipes.path as NSString).abbreviatingWithTildeInPath }
        return settings.recipeSources.first { $0.id == source }?.displayName ?? source
    }

    /// What tells each available recipe apart from others of its base (`RecipeID.distinction`): ID → text
    public func distinctions(_ settings: Settings) -> [String: String] {
        let entries = Dictionary(available(settings).map { ($0.id, $0.source) }, uniquingKeysWith: { a, _ in a })
        var out: [String: String] = [:]
        for id in entries.keys {
            out[id] = RecipeID.distinction(id, among: entries) { sourceName($0, settings: settings) }
        }
        return out
    }

    // MARK: Install and remove

    /// Downloads a recipe and those it requires (from the same catalog), checking every file's size and SHA-256.
    /// A recipe whose content is here already (any source) is not downloaded again; a new version replaces the older
    /// one of the same source (its line), images built from that stay. Returns the IDs written.
    @discardableResult
    public func install(_ id: String, catalog: RecipeCatalog, client: SourceClient) async throws -> [String] {
        var order: [RecipeCatalog.Recipe] = []
        var visiting = Set<String>()
        func collect(_ ref: String) throws {
            guard let rid = RecipeRef.id(RecipeID.base(ref)) else { throw VMError(String(localized: "Not a recipe: \(ref)")) }
            guard visiting.insert(rid).inserted else { return }
            guard let r = catalog.recipes.first(where: { $0.id == rid }) else {
                throw VMError(String(localized: "\(ref) is not in \(client.source.displayName)"))
            }
            for req in r.requires ?? [] { try collect(req) }
            order.append(r)
        }
        try collect(id)
        var manifest = installed()
        var written: [String] = []
        let here = Set(available(Settings.load(paths)).map(\.id))
        for r in order {
            let full = RecipeID.make(base: r.id, contentHash: r.contentHash)
            guard !here.contains(full), manifest[full] == nil else { continue }
            guard let target = folder(full) else { continue }
            try await download(r, client: client, to: target)
            guard (try? Self.contentHash(target)) == r.contentHash else {
                try? Disposal.delete([target])
                throw VMError(String(localized: "\(r.id) does not match the catalog – nothing was installed"))
            }
            manifest[full] = Installed(source: client.source.id, version: r.version, requires: r.requires?.compactMap(RecipeRef.id),
                                       installed: Date())
            try saveManifest(manifest)
            written.append(full)
        }
        if !written.isEmpty { pruneVersions() }   // an older version of this line goes, unless kept (plan B111)
        return written
    }

    func download(_ r: RecipeCatalog.Recipe, client: SourceClient, to target: URL) async throws {
        // The catalog is untrusted: its path must be the recipe's own folder, file paths stay inside it
        guard let (author, kind, name) = RecipeRef.parts(r.id), author == r.author, kind == r.kind, name == r.name,
              r.path == "recipes/\(author)/\(kind.rawValue)/\(name)" else {
            throw VMError(String(localized: "The catalog's entry for \(r.id) is not well formed"))
        }
        guard r.files.reduce(0, { $0 + $1.size }) <= Self.maxRecipe else { throw VMError(String(localized: "\(r.id) is larger than 5 MiB")) }
        guard r.files.contains(where: { $0.path == "recipe.toml" }), r.files.contains(where: { $0.path == "default.nix" }) else {
            throw VMError(String(localized: "\(r.id) has no recipe.toml or default.nix"))
        }
        let fm = FileManager.default
        try fm.createDirectory(at: paths.recipes, withIntermediateDirectories: true)
        let work = paths.recipes.appendingPathComponent(".download-\(UUID().uuidString)", isDirectory: true)
        defer { try? Disposal.delete([work]) }
        for f in r.files {
            let parts = f.path.split(separator: "/", omittingEmptySubsequences: false).map(String.init)
            guard !f.path.hasPrefix("/"), !parts.isEmpty, parts.allSatisfy({ !$0.isEmpty && $0 != "." && $0 != ".." }) else {
                throw VMError(String(localized: "\(r.id): a file path leaves the recipe's folder (\(f.path))"))
            }
            guard f.size <= Self.maxFile else { throw VMError(String(localized: "\(r.id)/\(f.path) is larger than 1 MiB")) }
            let data = try await client.fetch("\(r.path)/\(f.path)")
            guard data.count == f.size, SHA256.hash(data: data).map({ String(format: "%02x", $0) }).joined() == f.sha256.lowercased() else {
                throw VMError(String(localized: "\(r.id)/\(f.path) does not match the catalog (size or SHA-256) – nothing was installed"))
            }
            let dest = work.appendingPathComponent(f.path)
            try fm.createDirectory(at: dest.deletingLastPathComponent(), withIntermediateDirectories: true)
            try data.write(to: dest)
        }
        try fm.createDirectory(at: target.deletingLastPathComponent(), withIntermediateDirectories: true)
        try Disposal.delete([target])
        try fm.moveItem(at: work, to: target)
    }

    // MARK: Versions (plan B111)

    /// One version of a recipe's line
    public struct Version: Sendable, Equatable, Identifiable {
        public let id: String
        public let source: String
        /// When it came (downloaded, imported, kept at a build); nil for the recipe a local folder has now
        public let added: Date?
        /// The line's newest: a local folder's recipe as it is now, otherwise the last one that came
        public let isNewest: Bool
        /// It is the recipe in a local folder source (changed there, not removed here)
        public let isLive: Bool
        /// Images built from it – the version is kept while there are any
        public let images: [String]
        public var line: String { RecipeID.lineKey(base: id, source: source) }
    }

    /// Every line with its versions, newest first (line key → versions)
    public func versions(_ settings: Settings) -> [String: [Version]] {
        let installed = installed()
        let images = Dictionary(grouping: ImageStore(paths: paths).list(), by: \.recipe).mapValues { $0.map(\.name).sorted() }
        let liveFolders = Set(settings.recipeSources.filter { $0.kind == .folder }.map(\.location))
        var out: [String: [Version]] = [:]
        for entry in available(settings) {
            // available() lists a local folder's recipe before a kept copy of the same content
            let live = liveFolders.contains { entry.folder.path.hasPrefix($0 + "/") }
            let v = Version(id: entry.id, source: entry.source, added: installed[entry.id]?.installed, isNewest: false, isLive: live,
                            images: images[entry.id] ?? [])
            out[v.line, default: []].append(v)
        }
        return out.mapValues { list in
            let sorted = list.sorted { a, b in
                if a.isLive != b.isLive { return a.isLive }
                return (a.added ?? .distantFuture) > (b.added ?? .distantFuture)
            }
            return sorted.enumerated().map { i, v in
                Version(id: v.id, source: v.source, added: v.added, isNewest: i == 0, isLive: v.isLive, images: v.images)
            }
        }
    }

    /// Without `keepRecipeVersions`: the versions of each line that are not its newest and no image was built from are
    /// removed (downloaded, imported and kept copies – a local folder's recipe stays). Returns the removed IDs.
    @discardableResult
    public func pruneVersions() -> [String] {
        let settings = Settings.load(paths)
        guard !(settings.keepRecipeVersions ?? false) else { return [] }
        var removed: [String] = []
        for (_, list) in versions(settings) {
            for v in list where !v.isNewest && !v.isLive && v.images.isEmpty {
                if (try? remove(v.id)) != nil { removed.append(v.id) }
            }
        }
        return removed
    }

    /// Keeps the version an image is built from (plan B111): a recipe from a local folder – and those it requires – is
    /// copied to `recipes/<id>/`, so it stays when the folder changes. Downloaded and imported ones are kept already.
    public func keep(_ id: String, settings: Settings) {
        let fm = FileManager.default
        let available = available(settings)
        var manifest = installed()
        func copy(_ entry: (id: String, folder: URL, source: String)) {
            guard manifest[entry.id] == nil, let target = folder(entry.id), !fm.fileExists(atPath: target.path) else { return }
            let work = paths.recipes.appendingPathComponent(".keep-\(UUID().uuidString)", isDirectory: true)
            defer { try? Disposal.delete([work]) }
            do {
                for rel in try Self.recipeFiles(entry.folder) {
                    let file = work.appendingPathComponent(rel)
                    try fm.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
                    try fm.copyItem(at: entry.folder.appendingPathComponent(rel), to: file)
                }
                try fm.createDirectory(at: paths.recipes, withIntermediateDirectories: true)
                try fm.moveItem(at: work, to: target)
                manifest[entry.id] = Installed(source: entry.source, version: RecipeID.words(entry.id) ?? "",
                                               requires: Self.requires(of: target), installed: Date())
            } catch { return }
            for req in Self.requires(of: entry.folder) {
                if let r = requirement(req, source: entry.source, in: available) { copy(r) }
            }
        }
        guard let entry = available.first(where: { $0.id == id }) else { return }
        copy(entry)
        if manifest != installed() { try? saveManifest(manifest) }
    }

    /// Removes every version of a line that can go (not a local folder's recipe, no image built from it, not required
    /// by another recipe); returns the removed ones and why the others stay
    public func removeLine(_ line: String) -> (removed: [String], kept: [String]) {
        var removed: [String] = [], kept: [String] = []
        for v in versions(Settings.load(paths))[line] ?? [] {
            do { try remove(v.id); removed.append(v.id) } catch { kept.append("\(OmaflakeID.recipe(v.id, words: 2)): \(error)") }
        }
        return (removed, kept)
    }

    /// Removes one version of a recipe (downloaded, imported or kept). Refused while an image is built from it (it is
    /// kept for that image), for a local folder's recipe as it is now, and while another recipe requires it and no other
    /// version of it is left; images built from it stay.
    public func remove(_ id: String) throws {
        var manifest = installed()
        guard manifest[id] != nil else {
            throw VMError(String(localized: "\(OmaflakeID.recipe(id, words: 2)) lives in a local folder – change or remove it there"))
        }
        let used = ImageStore(paths: paths).list().filter { $0.recipe == id }.map(\.name)
        guard used.isEmpty else {
            throw VMError(String(localized: "Images are built from \(OmaflakeID.recipe(id, words: 2)) – it is kept while they are there"))
        }
        // Required by another recipe of the same source, and no other recipe of that base left for it
        let source = manifest[id]?.source
        let others = available(Settings.load(paths)).filter { $0.id != id && RecipeID.base($0.id) == RecipeID.base(id) }
        let users = others.isEmpty ? manifest.filter { $0.key != id && $0.value.source == source && $0.value.requires?.contains(RecipeID.base(id)) == true }
            .map(\.key).sorted() : []
        guard users.isEmpty else {
            throw VMError(String(localized: "\(RecipeID.qualified(id)) is required by \(users.map(RecipeID.qualified).joined(separator: ", ")) – remove those first"))
        }
        if let folder = folder(id) { try Disposal.delete([folder]) }
        manifest[id] = nil
        try saveManifest(manifest)
    }

    // MARK: Packages (plan B108)

    /// Source name of recipes that came from a package or a folder (no catalog, no updates)
    public static let importedSource = "import"

    /// The regular files of a recipe folder (relative paths), within the download limits; symbolic links, Finder
    /// files and Nix `result` links are left out
    static func recipeFiles(_ folder: URL) throws -> [String] {
        let fm = FileManager.default
        guard let walker = fm.enumerator(atPath: folder.path) else { return [] }
        var files: [String] = []
        var total = 0
        while let rel = walker.nextObject() as? String {
            let name = (rel as NSString).lastPathComponent
            if name == ".DS_Store" || name == "manifest.json" && !rel.contains("/") || rel.hasPrefix("result") { continue }
            let attrs = try fm.attributesOfItem(atPath: folder.appendingPathComponent(rel).path)
            guard attrs[.type] as? FileAttributeType == .typeRegular else { continue }
            let size = (attrs[.size] as? Int) ?? 0
            guard size <= maxFile else { throw VMError(String(localized: "\(rel) is larger than 1 MiB")) }
            total += size
            files.append(rel)
        }
        guard total <= maxRecipe else { throw VMError(String(localized: "\(folder.lastPathComponent) is larger than 5 MiB")) }
        return files.sorted()
    }

    /// What a recipe is, as one SHA-256 over its files (sorted relative paths and each file's SHA-256; the same files
    /// `recipeFiles` takes): its first 8 hex digits are part of the recipe's ID (`RecipeID`, plan B110)
    public static func contentHash(_ folder: URL) throws -> String {
        var hasher = SHA256()
        for rel in try recipeFiles(folder) {
            hasher.update(data: Data(rel.utf8 + [0]))
            hasher.update(data: Data(SHA256.hash(data: try Data(contentsOf: folder.appendingPathComponent(rel)))))
        }
        return hasher.finalize().map { String(format: "%02x", $0) }.joined()
    }

    /// recipe.toml's version this omaflake knows – `"$schema" = "https://omaflake.dev/schema/recipe/v1.json"` – and the
    /// oldest it still builds (version 0: no `$schema`, the systems of Apple silicon). The image flake reads the same
    /// range (plan B114). A recipe is never migrated here: a newer version comes from its author.
    public static let recipeVersion = 1
    public static let recipeMinimum = 0

    /// A recipe version this omaflake builds
    static func readable(_ version: Int) -> Bool { (recipeMinimum...recipeVersion).contains(version) }

    /// The version a recipe.toml's `"$schema"` names (0 without one or with another schema, written before B114)
    static func version(of folder: URL) -> Int {
        guard let toml = try? String(contentsOf: folder.appendingPathComponent("recipe.toml"), encoding: .utf8) else { return 0 }
        for line in toml.split(separator: "\n") {
            let parts = line.split(separator: "=", maxSplits: 1)
            guard parts.count == 2, parts[0].trimmingCharacters(in: .whitespaces) == "\"$schema\"",
                  let value = parts[1].trimmingCharacters(in: .whitespaces).split(separator: "\"").first.map(String.init),
                  let (type, version) = FileFormat.parse(value), type == "recipe" else { continue }
            return version
        }
        return 0
    }

    /// Available recipes of a version this omaflake does not build (too old or too new): left out of what the image
    /// flake sees – one of them would stop every build – and named with the reason
    public func unreadable(_ settings: Settings) -> [UnreadableFile] {
        available(settings).compactMap { e in
            do {
                _ = try FileFormat.check(Self.version(of: e.folder), minimum: Self.recipeMinimum, current: Self.recipeVersion,
                                         file: "recipe.toml")
                return nil
            } catch { return UnreadableFile(name: e.id, reason: error.localizedDescription) }
        }
    }

    /// `requires = ["author/kind/name", …]` of a recipe.toml → recipe IDs
    static func requires(of folder: URL) -> [String] {
        guard let toml = try? String(contentsOf: folder.appendingPathComponent("recipe.toml"), encoding: .utf8),
              let line = toml.split(separator: "\n").first(where: { $0.trimmingCharacters(in: .whitespaces).hasPrefix("requires") }),
              let open = line.firstIndex(of: "["), let close = line.lastIndex(of: "]"), open < close else { return [] }
        return line[line.index(after: open)..<close].split(separator: ",").compactMap {
            RecipeRef.id($0.trimmingCharacters(in: CharacterSet(charactersIn: " \"'")))
        }
    }

    /// Recipes and the ones they require → `<folder>/<id>.omaflake-recipe` (`recipes/<author>/<kind>/<name>/` +
    /// manifest.json) – what people share
    @discardableResult
    public func exportRecipe(_ id: String, to folder: URL, settings: Settings) throws -> URL {
        let fm = FileManager.default
        let available = available(settings)
        guard let top = available.first(where: { $0.id == id }) else {
            throw VMError(String(localized: "\(RecipeID.qualified(id)) is not available here"))
        }
        // The recipe and what it requires, as its build would see them (same source first)
        var order: [(id: String, folder: URL)] = []
        func collect(_ entry: (id: String, folder: URL, source: String)) throws {
            guard !order.contains(where: { RecipeID.base($0.id) == RecipeID.base(entry.id) }) else { return }
            order.append((entry.id, entry.folder))
            for req in Self.requires(of: entry.folder) {
                guard let r = requirement(req, source: entry.source, in: available) else {
                    throw VMError(String(localized: "\(RecipeID.qualified(req)) is not available here"))
                }
                try collect(r)
            }
        }
        try collect(top)
        let base = RecipeID.base(id)
        let dest = folder.appendingPathComponent("\(base).\(Package.recipe)", isDirectory: true)
        guard !fm.fileExists(atPath: dest.path) else { throw VMError("\(dest.path) exists already") }
        let partial = folder.appendingPathComponent(".\(base).\(Package.recipe).partial", isDirectory: true)
        try Disposal.delete([partial])
        do {
            for (rid, source) in order {
                guard let (author, kind, name) = RecipeRef.parts(RecipeID.base(rid)) else { continue }
                let target = partial.appendingPathComponent("recipes/\(author)/\(kind.rawValue)/\(name)", isDirectory: true)
                for rel in try Self.recipeFiles(source) {
                    let file = target.appendingPathComponent(rel)
                    try fm.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
                    try fm.copyItem(at: source.appendingPathComponent(rel), to: file)
                }
            }
            try Package.Manifest(kind: .recipe, name: base, recipe: id).write(partial)
            try fm.moveItem(at: partial, to: dest)
        } catch {
            try? Disposal.delete([partial])
            throw error
        }
        return dest
    }

    /// The recipes in a package, a checkout, a folder of recipe folders or one recipe folder (whose place says who it
    /// is: …/<author>/<kind>/<name>); an error when there are none
    public func recipes(in url: URL) throws -> [(id: String, folder: URL)] {
        var found = Self.recipeFolders(in: url)
        if found.isEmpty, FileManager.default.fileExists(atPath: url.appendingPathComponent("recipe.toml").path) {
            let c = url.standardizedFileURL.pathComponents.suffix(3)
            if c.count == 3, let id = RecipeRef.id(c.joined(separator: "/")) { found = [(id, url)] }
        }
        guard !found.isEmpty else {
            throw VMError(String(localized: "No recipes in \(url.path) (expected <author>/<kind>/<name>/recipe.toml)"))
        }
        return found
    }

    /// Recipes from a package, a checkout or a folder of recipe folders (`<author>/<kind>/<name>/recipe.toml`), or
    /// one recipe folder → the downloaded recipes (source "import"). Same limits as a download; a recipe it requires
    /// must come along or be available already. A recipe whose content is here already is skipped; other content is a
    /// recipe of its own next to the ones there are (plan B110) – nothing is replaced. Returns the IDs written.
    @discardableResult
    public func importRecipes(from url: URL) throws -> [String] {
        let fm = FileManager.default
        let found = try recipes(in: url)
        let available = available(Settings.load(paths))
        let known = Set(available.map { RecipeID.base($0.id) }).union(found.map(\.id))
        for (base, folder) in found {
            guard fm.fileExists(atPath: folder.appendingPathComponent("default.nix").path) else {
                throw VMError(String(localized: "\(RecipeID.qualified(base)) has no default.nix"))
            }
            if let missing = Self.requires(of: folder).first(where: { !known.contains($0) }) {
                throw VMError(String(localized: "\(RecipeID.qualified(base)) requires \(RecipeID.qualified(missing)), which is not here – nothing was imported"))
            }
        }
        let here = Set(available.map(\.id))
        var manifest = installed()
        var written: [String] = []
        try fm.createDirectory(at: paths.recipes, withIntermediateDirectories: true)
        for (base, folder) in found {
            let id = RecipeID.make(base: base, contentHash: try Self.contentHash(folder))
            guard !here.contains(id), manifest[id] == nil, let target = self.folder(id) else { continue }
            let work = paths.recipes.appendingPathComponent(".import-\(UUID().uuidString)", isDirectory: true)
            defer { try? Disposal.delete([work]) }
            for rel in try Self.recipeFiles(folder) {
                let data = try Data(contentsOf: folder.appendingPathComponent(rel))
                let file = work.appendingPathComponent(rel)
                try fm.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
                try data.write(to: file)
            }
            try fm.moveItem(at: work, to: target)
            manifest[id] = Installed(source: Self.importedSource, version: RecipeID.words(id) ?? "", requires: Self.requires(of: target),
                                     installed: Date())
            try saveManifest(manifest)
            written.append(id)
        }
        if !written.isEmpty { pruneVersions() }
        return written
    }

    // MARK: Staging

    /// Puts every available recipe into one folder for the image flake's input `omaflake-recipes` (plan B110):
    /// `<cache>/recipes/<hash of the content>/<id>/<author>/<kind>/<name>/`, each recipe in a tree of its own with the
    /// recipes it requires next to it (same source first) – recipes import those by relative paths (`../minimal`), so
    /// each sees its own. `recipes.json` tells the flake which folder is which recipe (ID → path). The same content
    /// gives the same folder, so a change in a local folder is picked up by the next Nix call and nothing is copied
    /// twice. Symbolic links and Finder files are left out.
    public func stage(_ settings: Settings) throws -> URL {
        let fm = FileManager.default
        // Only recipes of a version this omaflake builds (`unreadable` names the others)
        let available = available(settings).filter { Self.readable(Self.version(of: $0.folder)) }
        var files: [(rel: String, source: URL)] = []
        var mapping: [String: String] = [:]
        func place(_ entry: (id: String, folder: URL, source: String), tree: String, placed: inout Set<String>) {
            let base = RecipeID.base(entry.id)
            guard placed.insert(base).inserted, let (author, kind, name) = RecipeRef.parts(base) else { return }
            let prefix = "\(tree)/\(author)/\(kind.rawValue)/\(name)"
            for rel in (try? Self.recipeFiles(entry.folder)) ?? [] {
                files.append(("\(prefix)/\(rel)", entry.folder.appendingPathComponent(rel)))
            }
            for req in Self.requires(of: entry.folder) {
                if let r = requirement(req, source: entry.source, in: available) { place(r, tree: tree, placed: &placed) }
            }
        }
        for entry in available {
            guard let (author, kind, name) = RecipeRef.parts(RecipeID.base(entry.id)) else { continue }
            var placed = Set<String>()
            place(entry, tree: entry.id, placed: &placed)
            mapping[entry.id] = "\(entry.id)/\(author)/\(kind.rawValue)/\(name)"
        }
        let e = JSONEncoder()
        e.outputFormatting = [.prettyPrinted, .sortedKeys]
        let mappingData = try e.encode(mapping)
        files.sort { $0.rel < $1.rel }
        var hasher = SHA256()
        var contents: [(String, Data)] = [("recipes.json", mappingData)]
        hasher.update(data: mappingData)
        for f in files {
            let data = try Data(contentsOf: f.source)
            hasher.update(data: Data(f.rel.utf8 + [0]))
            hasher.update(data: Data(SHA256.hash(data: data)))
            contents.append((f.rel, data))
        }
        let hash = hasher.finalize().prefix(10).map { String(format: "%02x", $0) }.joined()
        let root = paths.cache.appendingPathComponent("recipes", isDirectory: true)
        let target = root.appendingPathComponent(hash, isDirectory: true)
        if fm.fileExists(atPath: target.path) {
            try? fm.setAttributes([.modificationDate: Date()], ofItemAtPath: target.path)   // recently used
            return target
        }
        let work = root.appendingPathComponent(".\(hash)-\(UUID().uuidString)", isDirectory: true)
        try fm.createDirectory(at: work, withIntermediateDirectories: true)
        for (rel, data) in contents {
            let dest = work.appendingPathComponent(rel)
            try fm.createDirectory(at: dest.deletingLastPathComponent(), withIntermediateDirectories: true)
            try data.write(to: dest)
        }
        do { try fm.moveItem(at: work, to: target) } catch {
            try? Disposal.delete([work])   // another process staged the same content meanwhile
            guard fm.fileExists(atPath: target.path) else { throw error }
        }
        prune(root, keep: 8)
        return target
    }

    /// Keeps the most recently used staged folders
    func prune(_ root: URL, keep: Int) {
        let fm = FileManager.default
        let dirs = ((try? fm.contentsOfDirectory(at: root, includingPropertiesForKeys: [.contentModificationDateKey])) ?? [])
            .filter { !$0.lastPathComponent.hasPrefix(".") }
            .sorted {
                let a = (try? $0.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
                let b = (try? $1.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
                return a > b
            }
        try? Disposal.delete(Array(dirs.dropFirst(keep)))
    }
}
