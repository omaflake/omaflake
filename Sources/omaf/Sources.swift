// SPDX-License-Identifier: AGPL-3.0-or-later
import ArgumentParser
import Foundation
import OmaflakeCore

// MARK: - Recipe sources (spec §11.3, plan B107)

struct Sources: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        abstract: "Where recipes come from: GitHub repositories (also private), HTTPS folders and local folders.",
        subcommands: [SourcesList.self, SourcesAdd.self, SourcesRemove.self, SourcesEnable.self, SourcesDisable.self, SourcesCheck.self],
        defaultSubcommand: SourcesList.self)
}

/// A source by its number in `omaf sources` (1 = first) or its ID
func source(_ ref: String, in sources: [RecipeSource]) throws -> Int {
    if let n = Int(ref), (1...sources.count).contains(n) { return n - 1 }
    if let i = sources.firstIndex(where: { $0.id == ref || $0.location == ref }) { return i }
    throw ValidationError("No source \(ref) – see omaf sources")
}

struct SourcesList: ParsableCommand {
    static let configuration = CommandConfiguration(commandName: "list", abstract: "The sources in their order.")

    func run() throws {
        let env = Env()
        let sources = env.settings.recipeSources
        let installed = RecipeLibrary(paths: env.paths).installed()
        for (i, s) in sources.enumerated() {
            var notes: [String] = []
            if !s.isEnabled { notes.append("off") }
            if s.isPrivateRepository { notes.append(SourceTokens.token(for: s) == nil ? "private, no token" : "private") }
            if s.isOfficial { notes.append("official") }
            let count = s.kind == .folder ? RecipeLibrary.recipeFolders(in: URL(fileURLWithPath: s.location)).count
                : installed.values.filter { $0.source == s.id }.count
            notes.append(s.kind == .folder ? "\(count) recipes, used as they are" : "\(count) installed")
            print("\(i + 1)  \(pad(s.kind.rawValue, 7))\(pad(s.displayName, 40))\(notes.joined(separator: ", "))")
        }
        if env.settings.sources == nil { print("(the default – omaf sources add … makes the list your own)") }
    }
}

struct SourcesAdd: AsyncParsableCommand {
    static let configuration = CommandConfiguration(commandName: "add",
        abstract: "Add a source: a GitHub repository (https://github.com/owner/repo, owner/repo), the HTTPS URL of a catalog.json, or a folder.")
    @Argument(help: "GitHub repository, HTTPS URL or folder") var location: String
    @Flag(help: "A private GitHub repository: read a fine-grained token (Contents: Read-only) from stdin") var `private` = false
    @Option(help: "GitHub: the branch (default main)") var branch: String?
    @Flag(help: "Put it first (earlier sources win when two have the same recipe)") var first = false

    func run() async throws {
        let env = Env()
        var s = try RecipeSource.parse(location, isPrivate: `private`)
        if let branch { s.branch = branch }
        var settings = env.settings
        var list = settings.recipeSources
        guard !list.contains(where: { $0.id == s.id }) else { throw ValidationError("\(s.displayName) is already a source") }
        var token: String?
        if s.isPrivateRepository {
            guard let line = readLine(strippingNewline: true), !line.isEmpty else { throw ValidationError("No token on stdin") }
            token = line
        }
        // Checked before it is kept: the repository (with the token), the catalog, or the folder's recipes
        print("\(s.displayName): \(try await SourceClient(s, token: token).verify())")
        if let token { try SourceTokens.set(token, for: s) }
        if first { list.insert(s, at: 0) } else { list.append(s) }
        settings.sources = list
        try settings.save(env.paths)
        print("Added. Recipes: omaf recipes available")
    }
}

struct SourcesRemove: ParsableCommand {
    static let configuration = CommandConfiguration(commandName: "remove",
        abstract: "Remove a source (its token goes from the Keychain; installed recipes stay).")
    @Argument(help: "Number in omaf sources, or the source's ID") var source: String

    func run() throws {
        let env = Env()
        var settings = env.settings
        var list = settings.recipeSources
        let i = try OmaflakeCLI.source(source, in: list)
        let s = list.remove(at: i)
        try? SourceTokens.set(nil, for: s)
        settings.sources = list
        try settings.save(env.paths)
        print("Removed \(s.displayName).")
    }
}

struct SourcesEnable: ParsableCommand {
    static let configuration = CommandConfiguration(commandName: "enable", abstract: "Switch a source on.")
    @Argument(help: "Number in omaf sources, or the source's ID") var source: String
    func run() throws { try toggle(source, on: true) }
}

struct SourcesDisable: ParsableCommand {
    static let configuration = CommandConfiguration(commandName: "disable",
        abstract: "Switch a source off (a local folder's recipes are no longer offered; installed ones stay).")
    @Argument(help: "Number in omaf sources, or the source's ID") var source: String
    func run() throws { try toggle(source, on: false) }
}

func toggle(_ ref: String, on: Bool) throws {
    let env = Env()
    var settings = env.settings
    var list = settings.recipeSources
    let i = try source(ref, in: list)
    list[i].enabled = on ? nil : false
    settings.sources = list
    try settings.save(env.paths)
    print("\(list[i].displayName): \(on ? "on" : "off")")
}

struct SourcesCheck: AsyncParsableCommand {
    static let configuration = CommandConfiguration(commandName: "check", abstract: "Check that every source can be read.")

    func run() async throws {
        var failed = false
        for s in Env().settings.recipeSources where s.isEnabled {
            do { print("ok   \(s.displayName): \(try await SourceClient(s).verify())") } catch {
                failed = true
                print("FAIL \(s.displayName): \(error.localizedDescription)")
            }
        }
        if failed { throw ExitCode.failure }
    }
}

// MARK: - Installing recipes

struct RecipesAvailable: AsyncParsableCommand {
    static let configuration = CommandConfiguration(commandName: "available",
        abstract: "What the remote sources offer (fetches their catalogs), and what is installed or has an update.")
    @Flag(help: "Also lab recipes") var all = false
    @Flag(help: "Also recipes for other architectures") var allSystems = false

    func run() async throws {
        let env = Env()
        let installed = RecipeLibrary(paths: env.paths).installed()
        let here = Set(RecipeLibrary(paths: env.paths).available(env.settings).map(\.id))
        let showLab = all || (env.settings.developerMode ?? false)
        for s in env.settings.recipeSources where s.isEnabled && s.kind != .folder {
            let catalog: RecipeCatalog
            do { catalog = try await SourceClient(s).catalog() } catch {
                print("\(s.displayName): \(error.localizedDescription)")
                continue
            }
            print("\(catalog.source.name) – \(s.displayName)\(s.isOfficial ? " (official)" : "")")
            for r in catalog.recipes where (showLab || !(r.lab ?? false)) && (allSystems || r.runsHere) {
                // By ID (plan B110): the same content is here (from any source), or an older version of this source
                let state = here.contains(RecipeID.make(base: r.id, contentHash: r.contentHash)) ? "installed"
                    : installed.contains(where: { RecipeID.base($0.key) == r.id && $0.value.source == s.id }) ? "update available" : ""
                let shown = r.runsHere ? state : "for \(r.systems.map(Platform.architecture).joined(separator: ","))"
                print("  \(pad("\(r.author)/\(r.kind.rawValue)/\(r.name)", 34))\(pad(shown, 18))\(r.description)")
            }
        }
    }
}

struct RecipesInstall: AsyncParsableCommand {
    static let configuration = CommandConfiguration(commandName: "install",
        abstract: "Download a recipe (and those it requires) from the first source that has it; also updates it.")
    @Argument(help: "<author>/<kind>/<name>, e.g. omaflake/nixos/minimal") var recipe: String

    func run() async throws {
        let env = Env()
        guard let id = RecipeRef.publicID(recipe) else { throw ValidationError("Write the recipe as <author>/<kind>/<name>") }
        for s in env.settings.recipeSources where s.isEnabled && s.kind != .folder {
            let client = SourceClient(s)
            guard let catalog = try? await client.catalog(), catalog.recipes.contains(where: { $0.id == id }) else { continue }
            let written = try await RecipeLibrary(paths: env.paths).install(id, catalog: catalog, client: client)
            print(written.isEmpty ? "\(recipe) is up to date." : "Installed from \(s.displayName): " + written.map(RecipeID.qualified).joined(separator: ", "))
            return
        }
        throw ValidationError("No enabled remote source has \(recipe) – see omaf recipes available")
    }
}

struct RecipesRemove: ParsableCommand {
    static let configuration = CommandConfiguration(commandName: "remove",
        abstract: "Remove a version of a downloaded, imported or kept recipe, or with --all every version of its line.",
        discussion: "A version images are built from stays while they are there; a local folder's recipe is changed in its folder.")
    @Argument(help: "<author>/<kind>/<name>, or its omaflake ID (omaf recipes versions) when there are several") var recipe: String
    @Flag(help: "Every version of the recipe's line (source and author/kind/name)") var all = false

    func run() throws {
        let env = Env()
        let library = RecipeLibrary(paths: env.paths)
        let versions = library.versions(env.settings).values.flatMap { $0 }
        let id = try RecipeID.resolve(recipe, among: (all ? versions : versions.filter { !$0.isLive }).map(\.id))
        if all {
            let line = RecipeID.lineKey(base: id, source: versions.first { $0.id == id }?.source ?? "")
            let (removed, kept) = library.removeLine(line)
            for r in removed { print("Removed \(OmaflakeID.recipe(r, words: 3))") }
            for k in kept { print("Kept \(k)") }
        } else {
            try library.remove(id)
            print("Removed \(OmaflakeID.recipe(id, words: 3)).")
        }
    }
}

struct RecipesVersions: ParsableCommand {
    static let configuration = CommandConfiguration(commandName: "versions",
        abstract: "Each recipe's versions per line (source and author/kind/name), newest first, and the images built from them.")
    @Argument(help: "Only this recipe (<name>, <kind>/<name>, <author>/<kind>/<name>)") var recipe: String?

    func run() throws {
        let env = Env()
        let library = RecipeLibrary(paths: env.paths)
        let lines = library.versions(env.settings)
        let all = lines.values.flatMap { $0 }
        let n = OmaflakeID.Lengths(recipes: all.map(\.id), images: [], vms: []).recipe
        let wanted = try recipe.map { r in Set(RecipeID.candidates(r, among: all.map(\.id)).map(RecipeID.base)) }
        let fmt = DateFormatter()
        fmt.dateFormat = "yyyy-MM-dd HH:mm"
        for (_, list) in lines.sorted(by: { $0.key < $1.key }) {
            guard let first = list.first, wanted?.contains(RecipeID.base(first.id)) ?? true else { continue }
            print("\(RecipeID.qualified(first.id))  from \(library.sourceName(first.source, settings: env.settings))")
            for v in list {
                let state = [v.isLive ? "in the folder" : v.added.map { fmt.string(from: $0) }, v.isNewest ? "newest" : nil,
                             v.images.isEmpty ? nil : "images: \(v.images.count)"].compactMap { $0 }.joined(separator: ", ")
                print("    \(pad(OmaflakeID.recipe(v.id, words: n), 50))\(state)")
            }
        }
        if wanted?.isEmpty == true { throw ValidationError("No recipe \(recipe ?? "")") }
    }
}

/// The module's name, to reach `source(_:in:)` from inside a command with a `source` property
enum OmaflakeCLI {
    static func source(_ ref: String, in sources: [RecipeSource]) throws -> Int { try omaf.source(ref, in: sources) }
}
