// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Export and import of packages (spec §11.10, plan B108/B109): a VM, an image or recipes leave the data folder as a
/// package (to an external disk, a NAS, a folder) and come back from one. An import is always a copy into the data
/// folder – the source is never changed; an export changes nothing in the data folder. A VM can also run from where its
/// package is (`useExternal`).
///
/// Who a package is for: a VM carries its user, SSH keys and data, so it goes back to the same person (a backup,
/// another Mac of theirs); an image carries the user it was built for. What people share are recipes.
public struct Transfer: Sendable {
    public let paths: Paths
    public let qemuImg: String
    /// omaf-runner, for a macOS VM imported as a copy (new machine identifier and MAC); nil without one
    public let runner: String?

    public init(paths: Paths, qemuImg: String, runner: String?) {
        self.paths = paths
        self.qemuImg = qemuImg
        self.runner = runner
    }

    public init(_ backends: Backends) {
        self.init(paths: backends.paths, qemuImg: backends.store.qemuImg, runner: (try? backends.vz)?.runner)
    }

    var store: InstanceStore { InstanceStore(paths: paths, qemuImg: qemuImg) }
    var images: ImageStore { ImageStore(paths: paths) }

    /// Name of the image package a VM package carries
    public static let embeddedImageName = "image.\(Package.image)"

    // MARK: - Export

    /// Whether a VM's package should carry its image: NixOS always (the overlay is built on it), macOS when it
    /// needs the image to start (ephemeral) – otherwise only for a reset, which `withImage` asks for
    public static func carriesImage(_ c: InstanceConfig, withImage: Bool) -> Bool {
        c.guestKind == .nixos || c.ephemeral || withImage
    }

    /// A stopped VM → `<folder>/<name>.omaflake-vm` (a snapshot: a running VM would change while it is copied).
    @discardableResult
    public func exportVM(_ name: String, to folder: URL, withImage: Bool = false,
                         log: @escaping @Sendable (String) -> Void = { _ in }) throws -> URL {
        let fm = FileManager.default
        let c = try store.load(name)
        guard !DataFolder.isRunning(c, paths: paths) else {
            throw VMError("\(name) is running – shut it down first: an export is a snapshot of a stopped VM")
        }
        let ip = paths.instance(name)
        let dest = folder.appendingPathComponent("\(name).\(Package.vm)", isDirectory: true)
        guard !fm.fileExists(atPath: dest.path) else { throw VMError("\(dest.path) exists already") }
        let partial = folder.appendingPathComponent(".\(name).\(Package.vm).partial", isDirectory: true)
        try Disposal.delete([partial])
        do {
            log("Copying \(name)")
            try Package.copy(ip.dir, to: partial)
            let embedded = partial.appendingPathComponent(Self.embeddedImageName)
            if Self.carriesImage(c, withImage: withImage), !fm.fileExists(atPath: embedded.path) {
                let image = paths.imageDir(c.image)
                guard fm.fileExists(atPath: image.path) else {
                    throw VMError("The image \(c.image) is missing – the VM cannot be exported without it")
                }
                log("Copying its image \(c.image)")
                try Package.copy(image, to: embedded)
                try Package.writable(embedded) { try? fm.removeItem(at: embedded.appendingPathComponent("manifest.json")) }
            }
            if c.guestKind == .nixos {
                try rebase(partial.appendingPathComponent("disk.qcow2"), onto: embedded.appendingPathComponent("disk.qcow2"), in: partial)
            }
            try Package.Manifest(kind: .vm, name: name, guestKind: c.guestKind, recipe: c.recipe, image: c.image,
                                 containsImage: fm.fileExists(atPath: embedded.path)).write(partial)
            try fm.moveItem(at: partial, to: dest)
        } catch {
            try? Disposal.delete([partial])
            throw error
        }
        log("Exported to \(dest.path)")
        return dest
    }

    /// An image → `<folder>/<name>.omaflake-image`
    @discardableResult
    public func exportImage(_ name: String, to folder: URL, log: @escaping @Sendable (String) -> Void = { _ in }) throws -> URL {
        let fm = FileManager.default
        let image = try images.image(named: name)
        let dest = folder.appendingPathComponent("\(name).\(Package.image)", isDirectory: true)
        guard !fm.fileExists(atPath: dest.path) else { throw VMError("\(dest.path) exists already") }
        let partial = folder.appendingPathComponent(".\(name).\(Package.image).partial", isDirectory: true)
        try Disposal.delete([partial])
        do {
            log("Copying \(name)")
            try Package.copy(paths.imageDir(name), to: partial)
            try Package.writable(partial) {
                try Package.Manifest(kind: .image, name: name, guestKind: image.kind, recipe: image.recipe, image: name).write(partial)
            }
            try fm.moveItem(at: partial, to: dest)
        } catch {
            try? Disposal.delete([partial])
            throw error
        }
        log("Exported to \(dest.path)")
        return dest
    }

    // MARK: - Import: always a copy into the data folder; the source is never changed

    public struct Imported: Sendable, Equatable {
        public let kind: Package.Kind
        /// VM or image name; recipe IDs for recipes
        public let names: [String]
        public let notes: [String]
    }

    /// What is here already for a package – checked before an import, so the app or the CLI can ask first
    public enum Conflict: Sendable, Equatable {
        case none
        /// A VM of that name is here: cancel, or import under another name (a second VM with its own identity)
        case vmName(String)
        /// The same image is here (equal names mean equal content): cancel or replace it
        case image(String)
        // Recipes have none (plan B110): the same content is here already, other content is a recipe of its own
    }

    /// The kind of what `url` holds: by extension, a plain folder with recipes counts as recipes
    public static func kind(of url: URL) -> Package.Kind { Package.kind(of: url) ?? .recipe }

    public func conflict(_ url: URL) throws -> Conflict {
        let url = url.standardizedFileURL
        switch Self.kind(of: url) {
        case .vm:
            let c = try Self.readConfig(url)
            if let external = externalVM(at: url), external == c.name { return .none }   // taking back a VM that runs from there
            return store.exists(c.name) ? .vmName(c.name) : .none
        case .image:
            guard ImageStore.readInfo(url) != nil, let name = Package.name(of: url, kind: .image) else {
                throw VMError("\(url.path) is not an image package (no image.json)")
            }
            return FileManager.default.fileExists(atPath: paths.imageDir(name).path) ? .image(name) : .none
        case .recipe:
            _ = try RecipeLibrary(paths: paths).recipes(in: url)
            return .none
        }
    }

    /// Imports a package by its kind. A conflict (see `conflict`) is an error unless it is resolved: a VM under
    /// another `name`, an image with `replace`. Recipes whose content is here already are skipped.
    public func importPackage(_ url: URL, name: String? = nil, replace: Bool = false,
                              log: @escaping @Sendable (String) -> Void = { _ in }) throws -> Imported {
        let url = url.standardizedFileURL
        switch Self.kind(of: url) {
        case .vm: return try importVM(url, name: name, log: log)
        case .image:
            let (n, notes) = try importImage(url, replace: replace, log: log)
            return Imported(kind: .image, names: [n], notes: notes)
        case .recipe:
            let ids = try RecipeLibrary(paths: paths).importRecipes(from: url)
            return Imported(kind: .recipe, names: ids, notes: ids.isEmpty ? ["These recipes are here already, unchanged"] : [])
        }
    }

    /// An image package → the data folder. `replace` puts it in place of the same image (refused while a VM on it runs).
    func importImage(_ url: URL, named: String? = nil, replace: Bool = false,
                     log: @escaping @Sendable (String) -> Void) throws -> (String, [String]) {
        let fm = FileManager.default
        guard let info = ImageStore.readInfo(url), let name = named ?? Package.name(of: url, kind: .image) else {
            throw VMError("\(url.path) is not an image package (no image.json)")
        }
        guard Platform.runsHere(info.system) else { throw Platform.notHere(name, system: info.system) }
        let dest = paths.imageDir(name)
        let exists = fm.fileExists(atPath: dest.path)
        if exists {
            guard replace else { throw VMError("The image \(name) is here already – replace it or cancel") }
            let running = store.list().filter { $0.image == name && DataFolder.isRunning($0, paths: paths) }.map(\.name)
            guard running.isEmpty else { throw VMError("\(running.joined(separator: ", ")) run on \(name) – shut them down first") }
        }
        try fm.createDirectory(at: paths.images, withIntermediateDirectories: true)
        let partial = paths.images.appendingPathComponent(".\(name).partial", isDirectory: true)
        try Disposal.delete([partial])
        do {
            log("Copying \(name)")
            try Package.copy(url, to: partial)
            try Package.writable(partial) { try? fm.removeItem(at: partial.appendingPathComponent("manifest.json")) }
            if exists { try Disposal.delete([dest]) }
            try fm.moveItem(at: partial, to: dest)
        } catch {
            try? Disposal.delete([partial])
            throw error
        }
        if images.currentName(line: info.line) == nil { try images.setCurrent(imageName: name) }
        return (name, [exists ? "image \(name) replaced" : "image \(name) imported"])
    }

    public static func readConfig(_ package: URL) throws -> InstanceConfig {
        guard let data = try? Data(contentsOf: package.appendingPathComponent("instance.json")) else {
            throw VMError("\(package.path) is not a VM package (no instance.json)")
        }
        return try InstanceConfig.fileFormat.decode(InstanceConfig.self, from: data, decoder: InstanceStore.decoder).value
    }

    /// The name of the registered external VM whose package is `url`
    func externalVM(at url: URL) -> String? {
        store.list().first { store.isExternal($0.name) && paths.instance($0.name).dir.standardizedFileURL.path == url.path }?.name
    }

    /// A VM package → the data folder. Under its own name it is the same VM again (a backup coming back, the next Mac
    /// of the same person); under another name a second VM with its own identity (SSH port, macOS machine identifier
    /// and MAC). The image it carries becomes an image of the data folder unless that is here already.
    func importVM(_ url: URL, name: String?, log: @escaping @Sendable (String) -> Void) throws -> Imported {
        let fm = FileManager.default
        var c = try Self.readConfig(url)
        try InstanceStore.checkRunsHere(c)
        var notes: [String] = []
        // Taking back a VM that runs from this place: it stays registered until it is back
        let external = externalVM(at: url)
        if let external, let e = try? store.load(external), DataFolder.isRunning(e, paths: paths) {
            throw VMError("\(external) is running – shut it down first")
        }
        let newName = name ?? c.name
        let newIdentity = newName != c.name
        let takingBack = external != nil && external == newName
        try store.validate(name: newName)
        guard takingBack || !store.exists(newName) else {
            throw VMError("A VM named \"\(newName)\" is here already – cancel, or import it under another name")
        }
        if newIdentity, c.guestKind == .macos, runner == nil {
            throw VMError("omaf-runner is missing – a macOS VM cannot get a new identity without it")
        }
        try fm.createDirectory(at: paths.instances, withIntermediateDirectories: true)
        let dest = paths.instances.appendingPathComponent("\(newName).\(Package.vm)", isDirectory: true)
        // Next to `dest`, so paths relative to the package are the same in both
        let partial = paths.instances.appendingPathComponent(".\(newName).\(Package.vm).partial", isDirectory: true)
        try Disposal.delete([partial])
        do {
            log("Copying \(c.name)")
            // Everything but the manifest and the image it carries: that becomes an image of the data folder
            try fm.createDirectory(at: partial, withIntermediateDirectories: true)
            for item in try fm.contentsOfDirectory(atPath: url.path) where item != "manifest.json" && item != Self.embeddedImageName {
                try Package.copy(url.appendingPathComponent(item), to: partial.appendingPathComponent(item))
            }
            let embedded = url.appendingPathComponent(Self.embeddedImageName)
            if fm.fileExists(atPath: embedded.path), !fm.fileExists(atPath: paths.imageDir(c.image).path) {
                notes += try importImage(embedded, named: c.image, log: log).1
            }
            c.name = newName
            if c.guestKind == .nixos {
                let image = paths.imageDir(c.image).appendingPathComponent("disk.qcow2")
                guard fm.fileExists(atPath: image.path) else {
                    throw VMError("The VM's image \(c.image) is missing – import the image (or a package with it) first")
                }
                try rebase(partial.appendingPathComponent("disk.qcow2"), onto: image, in: partial)
                c.imagePath = image.path
                if newIdentity || store.list().contains(where: { $0.name != newName && $0.sshPort == c.sshPort }) {
                    c.sshPort = store.nextSSHPort()
                    notes.append("SSH port \(c.sshPort)")
                }
            } else {
                c.imagePath = paths.imageDir(c.image).path
                if newIdentity { try newMacIdentity(partial.appendingPathComponent("vm")) }
            }
            if newIdentity || c.identity == nil { c.identity = OmaflakeID.randomWords() }
            try InstanceStore.encoder.encode(c).write(to: partial.appendingPathComponent("instance.json"), options: .atomic)
            try fm.moveItem(at: partial, to: dest)
        } catch {
            try? Disposal.delete([partial])
            throw error
        }
        if takingBack, let external { try store.forget(external) }
        notes.append("VM \(newName) imported")
        return Imported(kind: .vm, names: [newName], notes: notes)
    }

    /// A macOS VM folder gets a new machine identifier and MAC (omaf-runner clone on the same volume: instant)
    func newMacIdentity(_ vm: URL) throws {
        guard let runner else { throw VMError("omaf-runner is missing – a macOS VM cannot get a new identity without it") }
        guard FileManager.default.fileExists(atPath: vm.path) else { return }
        let fresh = vm.deletingLastPathComponent().appendingPathComponent("vm.new")
        try Disposal.delete([fresh])
        try Shell.check(runner, ["clone", vm.path, fresh.path])
        try Disposal.delete([vm])
        try FileManager.default.moveItem(at: fresh, to: vm)
    }

    /// The overlay's backing file → `image`, written relative to the package
    func rebase(_ disk: URL, onto image: URL, in package: URL) throws {
        guard FileManager.default.fileExists(atPath: disk.path) else { return }
        try Shell.check(qemuImg, ["rebase", "-u", "-F", "qcow2", "-b", Package.relativePath(from: package, to: image), disk.path],
                        cwd: package)
    }

    // MARK: - External VMs: run from where they are

    /// On a network volume (NAS): runs, but slower, and the VM breaks when the connection does
    public static func isOnNetworkVolume(_ url: URL) -> Bool {
        (try? url.resourceValues(forKeys: [.volumeIsLocalKey]).volumeIsLocal) == false
    }

    /// Runs a VM package from where it is (an external disk, a stick used on several Macs, a NAS, a folder): it must
    /// carry what it needs (NixOS: its image); its SSH port moves if this Mac uses it already. Nothing is copied, the
    /// package is registered in config.json (`externalVMs`).
    @discardableResult
    public func useExternal(_ url: URL) throws -> String {
        let fm = FileManager.default
        let url = url.standardizedFileURL
        guard let name = Package.name(of: url, kind: .vm) else { throw VMError("\(url.path) is not a VM package") }
        var c = try Self.readConfig(url)
        try InstanceStore.checkRunsHere(c)
        guard c.name == name else { throw VMError("The package is named \(name), the VM inside \(c.name) – rename it back first") }
        if externalVM(at: url) != nil { throw VMError("\(name) runs from there already") }
        guard !store.exists(name) else {
            throw VMError("A VM named \"\(name)\" is here already – remove or rename that one first")
        }
        let embedded = url.appendingPathComponent(Self.embeddedImageName)
        if c.guestKind == .nixos {
            guard fm.fileExists(atPath: embedded.appendingPathComponent("disk.qcow2").path) else {
                throw VMError("\(name) does not carry its image – export it again (omaf export vm) to run it from there")
            }
            try rebase(url.appendingPathComponent("disk.qcow2"), onto: embedded.appendingPathComponent("disk.qcow2"), in: url)
        }
        try store.register(url)
        if c.guestKind == .nixos, store.list().contains(where: { $0.name != name && $0.sshPort == c.sshPort }) {
            c.sshPort = store.nextSSHPort()
            try store.save(c)
        }
        return name
    }
}
