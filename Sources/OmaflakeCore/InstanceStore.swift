// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Instances under ~/.omaflake/instances: create, load, reset, remove.
public struct InstanceStore: Sendable {
    public let paths: Paths
    public let qemuImg: String

    public init(paths: Paths, qemuImg: String) {
        self.paths = paths
        self.qemuImg = qemuImg
    }

    public static let firstSSHPort = 2222
    static let efivarsSize: off_t = 64 * 1024 * 1024 // must match the code firmware (64 MiB)

    /// Lowercase letters, digits, hyphen; short enough for Unix sockets (< 104 bytes, plan B12).
    public func validate(name: String) throws {
        let ok = !name.isEmpty && name.count <= 32 && name.first != "-"
            && name.allSatisfy { ($0.isASCII && ($0.isLowercase || $0.isNumber)) || $0 == "-" }
        guard ok else { throw VMError("Invalid name \"\(name)\": 1–32 characters a–z, 0–9, \"-\"") }
        // longest socket name of both guest kinds (qmp.sock, runner.sock)
        let socket = paths.instance(name).runnerSocket.path
        guard socket.utf8.count < 104 else {
            throw VMError("Path \(socket) too long for a Unix socket (max. 103 bytes) – choose a shorter name or VM_HOME")
        }
    }

    public func exists(_ name: String) -> Bool {
        FileManager.default.fileExists(atPath: paths.instance(name).config.path)
    }

    /// instance.json, brought up to date (and written back) if it is older (plan B114)
    public func load(_ name: String) throws -> InstanceConfig {
        let file = paths.instance(name).config
        guard let data = try? Data(contentsOf: file) else { throw VMError("Instance \"\(name)\" does not exist") }
        let read = try InstanceConfig.fileFormat.decode(InstanceConfig.self, from: data, decoder: Self.decoder)
        if read.migrated { try? save(read.value) }
        return read.value
    }

    public func save(_ config: InstanceConfig) throws {
        let file = paths.instance(config.name).config
        try InstanceConfig.fileFormat.checkWritable(file)
        try InstanceConfig.fileFormat.encode(config, encoder: Self.encoder).write(to: file, options: .atomic)
    }

    /// The VMs: packages in the data folder and the external ones whose volume is there
    public func list() -> [InstanceConfig] {
        var names = ((try? FileManager.default.contentsOfDirectory(atPath: paths.instances.path)) ?? [])
            .compactMap { Package.name(of: URL(fileURLWithPath: $0), kind: .vm) }
        for url in paths.externalInstances() {
            if let n = Package.name(of: url, kind: .vm), !names.contains(n) { names.append(n) }
        }
        return names.sorted().compactMap { try? load($0) }
    }

    /// The VM's package lies outside the data folder (e.g. on an external disk, plan B108)
    public func isExternal(_ name: String) -> Bool {
        !paths.instance(name).dir.standardizedFileURL.path.hasPrefix(paths.instances.standardizedFileURL.path + "/")
    }

    /// The qcow2 a NixOS overlay is built on: the image inside the package (exported and external VMs carry it),
    /// otherwise the image's package in the data folder; nil if neither is there
    public func backingImage(_ c: InstanceConfig) -> URL? {
        let fm = FileManager.default
        let embedded = paths.instance(c.name).embeddedImage.appendingPathComponent("disk.qcow2")
        if fm.fileExists(atPath: embedded.path) { return embedded }
        return (try? ImageStore(paths: paths).image(named: c.image))?.path
    }

    /// A VM that carries its image (exported, external) carries the new one after an upgrade
    func replaceEmbeddedImage(_ name: String, with image: ImageStore.Image) throws {
        let embedded = paths.instance(name).embeddedImage
        guard FileManager.default.fileExists(atPath: embedded.path) else { return }
        try Disposal.delete([embedded])
        try Package.copy(paths.imageDir(image.name), to: embedded)
    }

    /// Runs an external VM package from where it is: registered in config.json (`externalVMs`)
    public func register(_ package: URL) throws {
        guard let name = Package.name(of: package, kind: .vm) else { throw VMError("Not a VM package: \(package.path)") }
        guard !exists(name) else { throw VMError("A VM named \"\(name)\" exists already") }
        var s = Settings.load(file: paths.configFile)
        s.externalVMs = (s.externalVMs ?? []).filter { $0 != package.path } + [package.path]
        try s.save(file: paths.configFile)
    }

    /// Stops running an external VM from its place – its package stays where it is (nothing is deleted). Works also
    /// while its volume is not there.
    public func forget(_ name: String) throws {
        var s = Settings.load(file: paths.configFile)
        let isIt = { (path: String) in Package.name(of: URL(fileURLWithPath: path), kind: .vm) == name }
        guard s.externalVMs?.contains(where: isIt) == true else { throw VMError("\(name) is not an external VM") }
        s.externalVMs = s.externalVMs?.filter { !isIt($0) }
        try s.save(file: paths.configFile)
        try? Disposal.delete([paths.instance(name).run])
    }

    /// Next free SSH port: not used by an instance and bindable locally.
    public func nextSSHPort(isFree: (Int) -> Bool = Ports.isFree) -> Int {
        let used = Set(list().map(\.sshPort))
        var port = Self.firstSSHPort
        while used.contains(port) || !isFree(port) { port += 1 }
        return port
    }

    /// Create the instance folder, overlay and UEFI variables.
    public func create(name: String, recipe: String, image: ImageStore.Image,
                       settings: RecipeDefaults, sshPort: Int? = nil) throws -> InstanceConfig {
        try validate(name: name)
        guard !exists(name) else { throw VMError("Instance \"\(name)\" already exists") }
        try Self.checkRunsHere(image)
        let ip = paths.instance(name)
        try FileManager.default.createDirectory(at: ip.dir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: ip.run, withIntermediateDirectories: true)
        // The recipe its image was built from (with its words, plan B110) and a new identity
        var config = InstanceConfig(name: name, recipe: image.recipe.isEmpty ? recipe : image.recipe, image: image.name,
                                    imagePath: image.path.path, system: image.info.system, settings: settings,
                                    sshPort: sshPort ?? nextSSHPort())
        config.identity = OmaflakeID.randomWords()
        try createDisks(config)
        try save(config)
        return config
    }

    /// Overlay (thin, backing file = image) + empty efivars. The backing file is written as a path relative to the
    /// package (plan B108), so the data folder – or a package with its image – can move as a whole.
    func createDisks(_ config: InstanceConfig) throws {
        let ip = paths.instance(config.name)
        guard let backing = backingImage(config) else {
            throw VMError("Base image missing: \(config.image) – rebuild it (omaf build \(config.recipe)) or import it")
        }
        try Shell.check(qemuImg, ["create", "-q", "-f", "qcow2", "-b", Package.relativePath(from: ip.dir, to: backing),
                                  "-F", "qcow2", ip.disk.path, "\(config.diskGB)G"], cwd: ip.dir)
        guard FileManager.default.createFile(atPath: ip.efivars.path, contents: nil) else {
            throw VMError("Could not create \(ip.efivars.path)")
        }
        guard truncate(ip.efivars.path, Self.efivarsSize) == 0 else {
            throw VMError("Could not set efivars to 64 MiB")
        }
    }

    /// Reset: the old state is deleted, a fresh overlay from the same image.
    public func reset(_ name: String) throws {
        let config = try load(name)
        let ip = paths.instance(name)
        try Disposal.delete([ip.disk, ip.efivars, ip.knownHosts])
        try createDisks(config)
    }

    /// Switch to another image (spec §4 "Update"): the old state is deleted,
    /// fresh overlay from the new image. Settings and SSH port are kept.
    public func upgrade(_ name: String, to image: ImageStore.Image) throws {
        var config = try load(name)
        guard config.image != image.name else { throw VMError("\"\(name)\" already uses \(image.name)") }
        let ip = paths.instance(name)
        try Disposal.delete([ip.disk, ip.efivars, ip.knownHosts])
        config.image = image.name
        config.imagePath = image.path.path
        try replaceEmbeddedImage(name, with: image)
        try createDisks(config)
        try save(config)
    }

    /// Remove an instance: its folder is deleted. Its default share folder goes too when it holds nothing but file system
    /// metadata; with files in it, or when it is a folder the user chose, it is kept (plan B85).
    public func remove(_ name: String) throws {
        guard exists(name) else { throw VMError("Instance \"\(name)\" does not exist") }
        let ownShare = (try? load(name))?.sharePath == nil
        let external = isExternal(name)
        let ip = paths.instance(name)
        try Disposal.delete([ip.dir, ip.run])
        if external {
            var s = Settings.load(file: paths.configFile)
            s.externalVMs = s.externalVMs?.filter { $0 != ip.dir.path }
            try s.save(file: paths.configFile)
        }
        if ownShare { Self.removeShareIfEmpty(paths.share(name)) }
    }

    /// What macOS guests and Finder leave in a shared folder on their own – not the user's files
    static let shareMetadata: Set<String> = [".fseventsd", ".Trashes", ".DS_Store", ".Spotlight-V100", ".TemporaryItems",
                                             ".localized", ".DocumentRevisions-V100"]

    /// Deletes a share folder that holds only metadata; returns whether it did. Anything else keeps it.
    @discardableResult
    static func removeShareIfEmpty(_ dir: URL) -> Bool {
        guard let entries = try? FileManager.default.contentsOfDirectory(atPath: dir.path),
              entries.allSatisfy({ shareMetadata.contains($0) }) else { return false }
        return (try? Disposal.delete([dir])) != nil
    }

    /// An image built for another architecture (plan B114) gives no VM here
    static func checkRunsHere(_ image: ImageStore.Image) throws {
        guard Platform.runsHere(image.info.system) else { throw Platform.notHere(image.name, system: image.info.system) }
    }

    /// A VM of another architecture does not start here
    static func checkRunsHere(_ c: InstanceConfig) throws {
        guard Platform.runsHere(c.system) else { throw Platform.notHere("\"\(c.name)\"", system: c.system) }
    }

    static let encoder: JSONEncoder = {
        let e = JSONEncoder()
        e.outputFormatting = [.prettyPrinted, .sortedKeys]
        e.dateEncodingStrategy = .iso8601
        return e
    }()
    static let decoder: JSONDecoder = {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return d
    }()
}
