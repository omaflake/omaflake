// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Images ("Vorlagen") under <data folder>/images: one package per image (`<name>.omaflake-image`, plan B108) with an
/// `image.json` that says what it is (recipe, author, guest kind, name, flake, revision, time). Nothing is derived
/// from folder names. Packages are self-contained, so they can be copied to a stick and back:
///   NixOS: `image.json` + `disk.qcow2` (copied out of the Nix store, read-only)
///   macOS: the VM folder (read-only) + `image.json`
/// `<name>` = `<author>.<kind>.<name>@<words>` (plan B110), the file form of its omaflake ID – the words come from the
/// store path, so the name is unique per content and several images of one recipe live side by side;
/// `current/<line>` points to the newest of a recipe's line.
public struct ImageStore: Sendable {
    public let paths: Paths

    public init(paths: Paths) { self.paths = paths }

    /// Contents of `image.json`
    public struct Info: Codable, Sendable, Equatable {
        /// image.json, schema `image` (plan B114). 0 → 1: "flavor" became "recipe" (plan B92), `system` added – every
        /// image before it was built on a Mac with Apple silicon
        public static let fileFormat = FileFormat("image", file: "image.json", steps: [
            { o in
                FileFormat.renameKey("flavor", to: "recipe", in: &o)
                o["system"] = (o["kind"] as? String) == GuestKind.macos.rawValue ? "aarch64-darwin" : "aarch64-linux"
            },
        ])

        /// Recipe ID "<author>.<kind>.<name>@<hash>" (plan B110; older images: without the hash)
        public var recipe: String
        /// The recipe source it was built from (`RecipeSource.id`) – with the recipe's base its line (plan B110)
        public var recipeSource: String?
        public var author: String
        public var kind: GuestKind
        /// What it was built for (`Platform`): "aarch64-linux", "x86_64-linux", "aarch64-darwin"
        public var system: String
        public var name: String
        /// Flake the image was built from (folder or URL)
        public var source: String
        /// The recipe's state (plan B107): from a local folder its git revision ("-dirty" with uncommitted changes in
        /// the recipe's folder, "nogit" outside git), downloaded the start of its catalog version; images from before
        /// B107 carry the image flake's revision
        public var revision: String
        /// NixOS: the store path of the image; macOS: the nix-darwin system it was activated with
        public var storePath: String?
        public var created: Date

        /// The recipe's line: images of one line replace each other (current, outdated, upgrade, retention)
        public var line: String { RecipeID.lineKey(base: recipe, source: recipeSource ?? "") }

        public init(recipe: String, recipeSource: String? = nil, kind: GuestKind, system: String, source: String,
                    revision: String, storePath: String?, created: Date = Date()) {
            let p = RecipeID.parts(recipe)
            self.recipe = recipe
            self.recipeSource = recipeSource
            self.author = p?.author ?? "local"
            self.kind = kind
            self.system = system
            self.name = p?.name ?? recipe
            self.source = source
            self.revision = revision
            self.storePath = storePath
            self.created = created
        }
    }

    public struct Image: Sendable, Equatable {
        public let name: String
        public let info: Info
        /// NixOS: the .qcow2 file; macOS: the VM folder (spec §5)
        public let path: URL
        public var recipe: String { info.recipe }
        public var kind: GuestKind { info.kind }
    }

    static func infoFile(_ dir: URL) -> URL { dir.appendingPathComponent("image.json") }

    static func encoder() -> JSONEncoder {
        let e = JSONEncoder()
        e.outputFormatting = [.prettyPrinted, .sortedKeys]
        e.dateEncodingStrategy = .iso8601
        return e
    }

    /// image.json, brought up to date (and written back) if it is older; nil if there is none or it cannot be read
    public static func readInfo(_ dir: URL) -> Info? {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        guard let data = try? Data(contentsOf: infoFile(dir)),
              let read = try? Info.fileFormat.decode(Info.self, from: data, decoder: d) else { return nil }
        if read.migrated { try? writeInfo(read.value, to: dir) }
        return read.value
    }

    /// Writes image.json; a read-only (macOS) folder is opened for it and closed again.
    static func writeInfo(_ info: Info, to dir: URL) throws {
        let fm = FileManager.default
        let readOnly = !fm.isWritableFile(atPath: dir.path)
        if readOnly { try fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: dir.path) }
        defer { if readOnly { try? fm.setAttributes([.posixPermissions: 0o555], ofItemAtPath: dir.path) } }
        try Info.fileFormat.encode(info, encoder: encoder()).write(to: infoFile(dir), options: .atomic)
        if readOnly { try fm.setAttributes([.posixPermissions: 0o444], ofItemAtPath: infoFile(dir).path) }
    }

    /// Recipe defaults from the flake (`lib.recipeDefaults`, from each recipe's recipe.toml).
    public func recipeDefaults(flake: String) throws -> [String: RecipeDefaults] {
        let json = try Shell.check("nix", ["eval", "--json", "\(Self.flakeRef(flake))#lib.recipeDefaults"] + guestArguments(flake: flake))
        return try JSONDecoder().decode([String: RecipeDefaults].self, from: Data(json.utf8))
    }

    /// The store path a build of `recipe` would produce now – evaluation only, nothing is built. NixOS: the disk image,
    /// macOS: the darwin system; both are what image.json records as `storePath` (plan B82).
    public func expectedStorePath(recipe: String, kind: GuestKind, flake: String) throws -> String {
        let attr = kind == .macos ? "darwinConfigurations.\"\(recipe)\".system" : try Self.imageAttribute(recipe)
        return try Shell.check("nix", ["eval", "--raw", "\(Self.flakeRef(flake))#\(attr).outPath"] + guestArguments(flake: flake))
            .trimmingCharacters(in: .whitespacesAndNewlines)
    }

    /// What a build of each recipe (with an image) would give now – recipes that fail to evaluate are left out
    public func expectedStorePaths(_ recipes: [String: RecipeDefaults], flake: String) -> [String: String] {
        var result: [String: String] = [:]
        for (id, d) in recipes where !images(of: id).isEmpty {
            result[id] = try? expectedStorePath(recipe: id, kind: d.kind, flake: flake)
        }
        return result
    }

    /// Recipes whose current image no longer matches the recipe (its files, the modules it uses, nixpkgs, the guest
    /// settings): a build now would give another store path.
    public func outdated(_ recipes: [String: RecipeDefaults], flake: String) -> Set<String> {
        let expected = expectedStorePaths(recipes, flake: flake)
        return Set(expected.compactMap { id, path in current(recipe: id).map { $0.info.storePath == path ? nil : id } ?? nil })
    }

    final class GCWatch: @unchecked Sendable {
        private let lock = NSLock()
        private var hit = false
        func set() { lock.withLock { hit = true } }
        var value: Bool { lock.withLock { hit } }
    }

    /// The flake output of a NixOS recipe's image for this host: `images.<system>."<recipe>"` (plan B114)
    static func imageAttribute(_ recipe: String) throws -> String {
        guard let system = Platform.hostSystem(for: .nixos) else { throw VMError("NixOS guests do not run on this computer") }
        return "images.\(system).\"\(recipe)\""
    }

    /// `nix build <flake>#images.<system>.<recipe>`, create the GC root, switch `current/<recipe>`.
    /// The build log goes line by line to `log`; `started` hands over the nix process for cancelling.
    public func build(recipe: String, flake: String, started: (Process) -> Void = { _ in },
                      log: @escaping @Sendable (String) -> Void) throws -> Image {
        // The Linux builder collects garbage when it runs low on space (min-free) – mid-build that has removed paths
        // the build still needed ("dependencies failed", "is not valid"); a second run then finds the space (plan B79)
        let attribute = try Self.imageAttribute(recipe)
        let gcHit = GCWatch()
        var r = Shell.Result(status: -1, stdout: "", stderr: "")
        for run in 1...2 {
            if run == 2 {
                guard r.status != 0, gcHit.value, Cancellation.current?.isCancelled != true else { break }
                log("The Linux builder cleaned up during the build – building again")
            }
            r = try KeepAwake.during("Building a NixOS image") {
                try Shell.stream("nix", ["build", "\(Self.flakeRef(flake))#\(attribute)", "--no-link", "--print-out-paths", "-L"]
                                        + guestArguments(flake: flake),
                                 started: started) { line in
                    if line.contains("running auto-GC") { gcHit.set() }
                    log(line)
                }
            }
        }
        guard r.status == 0 else {
            let cancelled = r.status == 15 || Cancellation.current?.isCancelled == true
            throw VMError(cancelled ? "Build cancelled" : "nix build failed (\(r.status)) – see log")
        }
        let out = r.stdout
        guard let storePath = out.split(separator: "\n").last.map(String.init), storePath.hasPrefix("/nix/store/") else {
            throw VMError("nix build returned no store path")
        }
        let rev = recipeRevision(recipe, flake: flake)
        let name = Self.imageName(recipe: recipe, storePath: storePath)
        let recipeSource = RecipeLibrary(paths: paths).available(Settings.load(paths)).first { $0.id == recipe }?.source
        let dir = paths.imageDir(name)
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        if Self.readInfo(dir) == nil {
            log("Copying the image out of the Nix store")
            try Self.copyDisk(fromStore: URL(fileURLWithPath: storePath), into: dir)
            try Self.writeInfo(Info(recipe: recipe, recipeSource: recipeSource, kind: .nixos,
                                    system: Platform.hostSystem(for: .nixos) ?? "", source: flake, revision: rev,
                                    storePath: storePath), to: dir)
        }
        try setCurrent(imageName: name)
        // The recipe version it is built from stays (plan B111); older ones may go now
        let library = RecipeLibrary(paths: paths)
        library.keep(recipe, settings: Settings.load(paths))
        library.pruneVersions()
        return try image(named: name)
    }

    /// The qcow2 of a NixOS store path → `<package>/disk.qcow2` (read-only). The package then needs no GC root and
    /// no Nix store – it can be copied anywhere (plan B108). Written under a temporary name first, so an interrupted
    /// copy never looks like an image.
    static func copyDisk(fromStore store: URL, into dir: URL) throws {
        let fm = FileManager.default
        let files = (try? fm.contentsOfDirectory(atPath: store.path)) ?? []
        guard let file = files.first(where: { $0.hasSuffix(".qcow2") }) else {
            throw VMError("No .qcow2 in \(store.path)")
        }
        let disk = dir.appendingPathComponent("disk.qcow2")
        let partial = dir.appendingPathComponent(".disk.qcow2.partial")
        if fm.fileExists(atPath: partial.path) { try fm.removeItem(at: partial) }
        try fm.copyItem(at: store.appendingPathComponent(file), to: partial)
        try fm.setAttributes([.posixPermissions: 0o444], ofItemAtPath: partial.path)
        try fm.moveItem(at: partial, to: disk)
    }

    /// Flake reference for `nix`: a folder without its own git repository (like `recipes/` in the
    /// checkout) becomes `path:<folder>`, so Nix also sees its git-ignored `local/` settings.
    /// A git reference would copy only tracked files. Flake URLs (`path:`, `github:`, …) stay as they are.
    static func flakeRef(_ flake: String, fileManager: FileManager = .default) -> String {
        guard flake.hasPrefix("/") else { return flake }
        let ownRepo = fileManager.fileExists(atPath: (flake as NSString).appendingPathComponent(".git"))
        return ownRepo ? flake : "path:\(flake)"
    }

    /// "omaflake.nixos.minimal" + a store path → "omaflake.nixos.minimal@cedar-mango-lift-…" (12 words, plan B111; the
    /// revision is in image.json)
    static func imageName(recipe: String, storePath: String) -> String {
        "\(RecipeID.base(recipe))@\(OmaflakeID.words(of: storePath))"
    }

    /// The state of a recipe for the image's name and image.json: where the recipe comes from decides (plan B107);
    /// unknown recipes fall back to the image flake's revision
    public func recipeRevision(_ recipe: String, flake: String) -> String {
        let library = RecipeLibrary(paths: paths)
        if let entry = library.available(Settings.load(paths)).first(where: { $0.id == recipe }) {
            if let installed = library.installed()[recipe], installed.source == entry.source {
                return String(installed.version.prefix(10))
            }
            return gitRev(entry.folder.path)
        }
        return gitRev(flake)
    }

    /// Short git hash of the folder's repository, "dirty" with uncommitted changes in that folder.
    func gitRev(_ folder: String) -> String {
        let dir = URL(fileURLWithPath: folder.hasPrefix("path:") ? String(folder.dropFirst(5)) : folder)
        // Never /usr/bin/git without developer tools: it would ask to install them (Prerequisites.locateGit)
        guard let git = Prerequisites.locateGit(),
              let rev = try? Shell.check(git, ["rev-parse", "--short=10", "HEAD"], cwd: dir)
            .trimmingCharacters(in: .whitespacesAndNewlines) else { return "nogit" }
        let dirty = (try? Shell.check(git, ["status", "--porcelain", "--", "."], cwd: dir))?.isEmpty == false
        return dirty ? "\(rev)-dirty" : rev
    }

    /// The line of a recipe ID (plan B110): from the recipe there is, else from an image built from it
    public func line(of recipe: String) -> String {
        if let entry = RecipeLibrary(paths: paths).available(Settings.load(paths)).first(where: { $0.id == recipe }) {
            return RecipeID.lineKey(base: recipe, source: entry.source)
        }
        return list().first { $0.recipe == recipe }?.info.line ?? RecipeID.lineKey(base: recipe, source: "")
    }

    /// Points `current/<line>` to an image – the newest of its recipe's line, used for new VMs (plan B56, B110)
    public func setCurrent(imageName: String) throws {
        let fm = FileManager.default
        let image = try image(named: imageName)
        try fm.createDirectory(at: paths.currentImages, withIntermediateDirectories: true)
        let link = paths.currentImages.appendingPathComponent(image.info.line)
        // Replacing the symlink loses no data: the target stays under images/
        if (try? fm.destinationOfSymbolicLink(atPath: link.path)) != nil { try fm.removeItem(at: link) }
        try fm.createSymbolicLink(atPath: link.path, withDestinationPath: "../\(imageName).\(Package.image)")
    }

    /// Name of the current image of a recipe's line, of any guest kind (macOS images are VM folders, not qcow2).
    public func currentName(recipe: String) -> String? {
        currentName(line: line(of: recipe))
    }

    public func currentName(line: String) -> String? {
        let link = paths.currentImages.appendingPathComponent(line)
        return (try? FileManager.default.destinationOfSymbolicLink(atPath: link.path)).map(Self.imageName(ofLink:))
    }

    /// Link target "../<name>.omaflake-image" → "<name>" (links from before B108 have no extension)
    static func imageName(ofLink dest: String) -> String {
        let url = URL(fileURLWithPath: dest)
        return Package.name(of: url, kind: .image) ?? url.lastPathComponent
    }

    /// Current image of a recipe (nil if never built).
    public func current(recipe: String) -> Image? {
        currentName(recipe: recipe).flatMap { try? image(named: $0) }
    }

    public func image(named name: String) throws -> Image {
        try image(at: paths.imageDir(name))
    }

    /// An image package anywhere (in the data folder, inside a VM package, on a stick)
    public func image(at dir: URL) throws -> Image {
        let name = Package.name(of: dir, kind: .image) ?? dir.lastPathComponent
        guard let info = Self.readInfo(dir) else { throw VMError("Image \(name) has no image.json (\(dir.path))") }
        if info.kind == .macos { return Image(name: name, info: info, path: dir) }
        let disk = dir.appendingPathComponent("disk.qcow2")
        guard FileManager.default.fileExists(atPath: disk.path) else {
            throw VMError("Image \(name): no disk.qcow2 in \(dir.path) – build it again")
        }
        return Image(name: name, info: info, path: disk)
    }

    /// Legacy folder names `<recipe>-<git-rev>[-dirty]-<hash>` → recipe part (used only to migrate old images).
    static func recipe(ofImage name: String) -> String {
        var parts = name.split(separator: "-", omittingEmptySubsequences: false)
        guard parts.count >= 3 else { return name }
        parts.removeLast()                              // store hash
        if parts.last == "dirty" { parts.removeLast() }
        parts.removeLast()                              // git rev or "nogit"
        return parts.joined(separator: "-")
    }

    /// Space the image takes on disk: allocated blocks (sparse disks count what they use), symlinks followed
    /// (NixOS: the .qcow2 in the Nix store). nil when it can't be read.
    public static func diskUsage(_ image: Image) -> Int64? {
        let url = image.path.resolvingSymlinksInPath()
        let keys: Set<URLResourceKey> = [.totalFileAllocatedSizeKey, .fileAllocatedSizeKey, .isRegularFileKey]
        func size(_ u: URL) -> Int64 {
            let v = try? u.resourceValues(forKeys: keys)
            return Int64(v?.totalFileAllocatedSize ?? v?.fileAllocatedSize ?? 0)
        }
        var isDir: ObjCBool = false
        guard FileManager.default.fileExists(atPath: url.path, isDirectory: &isDir) else { return nil }
        guard isDir.boolValue else { return size(url) }
        guard let files = FileManager.default.enumerator(at: url, includingPropertiesForKeys: Array(keys)) else { return nil }
        var total: Int64 = 0
        for case let f as URL in files where (try? f.resourceValues(forKeys: [.isRegularFileKey]))?.isRegularFile == true {
            total += size(f)
        }
        return total
    }

    /// Images of a recipe's line, newest first.
    public func images(of recipe: String) -> [Image] {
        images(inLine: line(of: recipe))
    }

    public func images(inLine line: String) -> [Image] {
        list().filter { $0.info.line == line }.sorted { $0.info.created > $1.info.created }
    }

    /// Deletes an image (NixOS: its GC root goes with it, the store path can then be collected). Refused while
    /// instances use it: NixOS overlays are built on it, macOS instances clone it again on reset (ephemeral ones on
    /// every start). If it was the recipe's current image, `current/` moves to the next newest image of that recipe.
    public func remove(_ name: String, instances: InstanceStore) throws {
        let image = try image(named: name)
        let users = instances.list().filter { $0.image == name }.map(\.name).sorted()
        guard users.isEmpty else {
            throw VMError("\(name) is used by \(users.joined(separator: ", ")) – remove or upgrade them first")
        }
        let line = image.info.line
        let wasCurrent = currentName(line: line) == name
        try Disposal.delete([paths.imageDir(name)])
        defer { RecipeLibrary(paths: paths).pruneVersions() }   // its recipe version may go now (plan B111)
        if wasCurrent {
            let link = paths.currentImages.appendingPathComponent(line)
            if let next = images(inLine: line).first {
                try setCurrent(imageName: next.name)
            } else {
                try? Disposal.delete([link])
            }
        }
    }

    /// Retention (plan B52): after a build, a recipe keeps its current image, the one before it (to go back to) and
    /// every image a VM still uses; older ones are deleted. `used` must list the images of *all* instances – callers
    /// skip pruning when they cannot read them. Returns the deleted names.
    public static let keepPrevious = 1

    @discardableResult
    public func prune(recipe: String, used: Set<String>) -> [String] {
        let line = line(of: recipe)
        let current = currentName(line: line)
        let older = images(inLine: line).filter { $0.name != current }
        let stale = older.dropFirst(Self.keepPrevious).filter { !used.contains($0.name) }
        var removed: [String] = []
        for image in stale where (try? Disposal.delete([paths.imageDir(image.name)])) != nil {
            removed.append(image.name)
        }
        if !removed.isEmpty { RecipeLibrary(paths: paths).pruneVersions() }
        return removed
    }

    /// All image packages with an image.json (not `current/` and not unfinished macOS builds `.build-*`).
    public func list() -> [Image] {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: paths.images.path)) ?? []
        return names.filter { !$0.hasPrefix(".") }
            .compactMap { Package.name(of: URL(fileURLWithPath: $0), kind: .image) }
            .sorted().compactMap { try? image(named: $0) }
    }
}
