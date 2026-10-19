// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Data storage (spec §5): by default ~/.omaflake. Deliberately not in Documents/Desktop – those are
/// privacy-protected folders, and omaflake asks for no access to them unless the user chooses a folder there
/// (data folder, share folder, image flake). Can be created with a custom root folder for tests.
public struct Paths: Sendable {
    /// Data folder (images, instances, bases, …)
    public let root: URL
    /// Settings file; stays at ~/.omaflake/config.json when the data folder is elsewhere (it names the data folder)
    public let configFile: URL

    public init(root: URL, configFile: URL? = nil) {
        self.root = root
        self.configFile = configFile ?? root.appendingPathComponent("config.json")
    }

    /// ~/.omaflake – default data folder and home of the settings file
    public static var defaultHome: URL {
        FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".omaflake", isDirectory: true)
    }

    /// Data folder from VM_HOME (settings file inside it), otherwise `home` in ~/.omaflake/config.json,
    /// otherwise ~/.omaflake.
    public static func standard() -> Paths {
        if let custom = ProcessInfo.processInfo.environment["VM_HOME"], !custom.isEmpty {
            return Paths(root: URL(fileURLWithPath: custom, isDirectory: true))
        }
        let config = defaultHome.appendingPathComponent("config.json")
        let home = Settings.load(file: config).home.map { URL(fileURLWithPath: ($0 as NSString).expandingTildeInPath, isDirectory: true) }
        return Paths(root: home ?? defaultHome, configFile: config)
    }

    /// Top-level entries of the data folder that belong to omaflake – moved by `DataFolder.move`; anything else
    /// in the folder stays where it is: the settings file and `developer/` (contributors' own local files).
    public static let dataEntries = ["images", "instances", "bases", "ipsw", "logs", "qemu", "share", "_to_delete", "cache", "guest", "recipes", "run"]

    public var images: URL { root.appendingPathComponent("images", isDirectory: true) }
    public var currentImages: URL { images.appendingPathComponent("current", isDirectory: true) }
    public var instances: URL { root.appendingPathComponent("instances", isDirectory: true) }
    public var shares: URL { root.appendingPathComponent("share", isDirectory: true) }
    public var trash: URL { root.appendingPathComponent("_to_delete", isDirectory: true) }
    /// macOS: fresh installations (`mac-fresh-<build>`) and bases (`mac-base-<build>`), spec §6.5
    public var bases: URL { root.appendingPathComponent("bases", isDirectory: true) }
    /// IPSW downloads of `omaf-runner install --latest`
    public var ipsw: URL { root.appendingPathComponent("ipsw", isDirectory: true) }
    /// Logs of macOS image builds
    public var logs: URL { root.appendingPathComponent("logs", isDirectory: true) }
    /// Guest values for the image flake (GuestSettings, input `omaflake-guest`)
    public var guest: URL { root.appendingPathComponent("guest", isDirectory: true) }
    /// App icon as a file for QEMU's Dock icon (Brand.dockIconFile)
    public var dockIcon: URL { root.appendingPathComponent("cache/dock-icon.png") }
    public var cache: URL { root.appendingPathComponent("cache", isDirectory: true) }
    /// Recipes downloaded from remote sources: <author>/<kind>/<name>/ (spec §11.3, plan B107)
    public var recipes: URL { root.appendingPathComponent("recipes", isDirectory: true) }

    /// A VM: its package (in the data folder, or an external one – e.g. on an external disk – from config.json
    /// `externalVMs`) and its run folder on this Mac (plan B108)
    public func instance(_ name: String) -> InstancePaths {
        InstancePaths(dir: instanceDir(name), run: run.appendingPathComponent(name, isDirectory: true))
    }

    /// Runtime files of the VMs on this Mac – sockets, pid, logs, lock, the agent's report – never in a package
    public var run: URL { root.appendingPathComponent("run", isDirectory: true) }

    /// A VM's package: `instances/<name>.omaflake-vm`, otherwise an external package of that name, otherwise where
    /// a new one goes
    public func instanceDir(_ name: String) -> URL {
        let own = instances.appendingPathComponent("\(name).\(Package.vm)", isDirectory: true)
        if FileManager.default.fileExists(atPath: own.path) { return own }
        if let external = externalInstances().first(where: { Package.name(of: $0, kind: .vm) == name }) { return external }
        return own
    }

    /// VM packages outside the data folder that omaflake runs from where they are (`externalVMs`); those whose
    /// volume is not there are left out
    public func externalInstances() -> [URL] {
        (Settings.load(file: configFile).externalVMs ?? []).map { URL(fileURLWithPath: $0, isDirectory: true) }
            .filter { FileManager.default.fileExists(atPath: $0.appendingPathComponent("instance.json").path) }
    }

    /// An image's package: `images/<name>.omaflake-image`
    public func imageDir(_ name: String) -> URL {
        images.appendingPathComponent("\(name).\(Package.image)", isDirectory: true)
    }

    /// Default share folder of an instance (created by omaflake)
    public func share(_ name: String) -> URL { shares.appendingPathComponent(name, isDirectory: true) }

    /// Share folder of an instance: the folder the user chose (`sharePath`), otherwise the default one.
    public func shareDir(_ c: InstanceConfig) -> URL {
        c.sharePath.map { URL(fileURLWithPath: $0, isDirectory: true) } ?? share(c.name)
    }

    /// Creates the default share folder; a folder the user chose must exist and is left as it is.
    @discardableResult
    public func prepareShare(_ c: InstanceConfig, mode: Int? = nil) throws -> URL {
        let dir = shareDir(c)
        if c.sharePath != nil {
            var isDir: ObjCBool = false
            guard FileManager.default.fileExists(atPath: dir.path, isDirectory: &isDir), isDir.boolValue else {
                throw VMError("Share folder missing: \(dir.path)")
            }
            return dir
        }
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        if let mode { try FileManager.default.setAttributes([.posixPermissions: mode], ofItemAtPath: dir.path) }
        return dir
    }
}

/// A VM's files. `dir` is its package (`<name>.omaflake-vm`, plan B108) – what makes the VM and travels with it:
/// instance.json, the disks, UEFI variables, known host keys, for NixOS maybe its own image. `run` holds what only
/// exists while it runs on this Mac – sockets, pid, logs, lock, the agent's report – so a package on a stick carries
/// none of it and every Mac has its own.
public struct InstancePaths: Sendable {
    public let dir: URL
    public let run: URL

    public init(dir: URL, run: URL? = nil) {
        self.dir = dir
        self.run = run ?? dir
    }

    public var config: URL { dir.appendingPathComponent("instance.json") }
    public var disk: URL { dir.appendingPathComponent("disk.qcow2") }
    public var efivars: URL { dir.appendingPathComponent("efivars.fd") }
    public var knownHosts: URL { dir.appendingPathComponent("known_hosts") }
    /// NixOS: the image the overlay is built on, inside the package (exported and external VMs carry it)
    public var embeddedImage: URL { dir.appendingPathComponent("image.\(Package.image)", isDirectory: true) }
    // macOS guests (spec §5): clone of the image folder
    public var vm: URL { dir.appendingPathComponent("vm", isDirectory: true) }

    // Runtime
    public var pidFile: URL { run.appendingPathComponent("qemu.pid") }
    public var qmpSocket: URL { run.appendingPathComponent("qmp.sock") }
    public var qgaSocket: URL { run.appendingPathComponent("qga.sock") }
    public var serialLog: URL { run.appendingPathComponent("serial.log") }
    public var qemuLog: URL { run.appendingPathComponent("qemu.log") }
    public var lockFile: URL { run.appendingPathComponent(".lock") }
    public var runnerSocket: URL { run.appendingPathComponent("runner.sock") }
    public var runnerPid: URL { run.appendingPathComponent("runner.pid") }
    public var runnerLog: URL { run.appendingPathComponent("runner.log") }
    // Guest integration (clipboard, camera): the port's socket and the bridge process' log
    public var clipSocket: URL { run.appendingPathComponent("clip.sock") }
    public var cameraSocket: URL { run.appendingPathComponent("cam.sock") }
    public var bridgeLog: URL { run.appendingPathComponent("bridge.log") }
    // omaflake agent (spec §11.9): the port's socket and what the agent last reported (AgentState)
    public var agentSocket: URL { run.appendingPathComponent("agent.sock") }
    public var agentState: URL { run.appendingPathComponent("agent.json") }

    /// The names of the runtime files – to move them out of a package of the older layout
    static let runtimeNames = ["qemu.pid", "qmp.sock", "qga.sock", "serial.log", "qemu.log", ".lock", "runner.sock",
                               "runner.pid", "runner.log", "clip.sock", "cam.sock", "bridge.log", "agent.sock", "agent.json"]
}

/// Settings in ~/.omaflake/config.json (optional).
public struct Settings: Codable, Sendable {
    /// The image flake – omaflake's Nix side that builds images from recipes (folder or flake URL, e.g. <checkout>/nix)
    public var flake: String?
    /// Recipe sources in their order (spec §11.3, plan B107); nil = omaflake/recipes on GitHub
    public var sources: [RecipeSource]?
    /// VM packages that live outside the data folder (e.g. on an external disk) and run from there (plan B108)
    public var externalVMs: [String]?
    /// qemu-system-aarch64; default: from PATH or /opt/homebrew/bin
    public var qemu: String?
    /// omaf-runner (signed with scripts/sign-runner.sh); default: next to the running executable
    public var runner: String?
    /// nix binary, for unusual installations; default: PATH and the usual Nix locations
    public var nix: String?
    /// Overrides for the guest values (user, full name, locale, …); unset fields come from the Mac
    public var guest: GuestOverrides?
    /// Data folder; nil = ~/.omaflake. Only read from ~/.omaflake/config.json; change it with `DataFolder.move`.
    public var home: String?
    /// Developer mode: also show lab recipes (omaflake's development recipes and others marked `lab = true`)
    public var developerMode: Bool?
    /// Look for a newer omaflake (the only network access of its own, spec §11.6); nil = on
    public var checkForUpdates: Bool?
    /// System notifications (VM ready, image built, …) while the menu window is closed; nil = on
    public var notifications: Bool?
    /// Keep older versions of a recipe's line (plan B111) when a newer one comes (an update, an import, a change in a
    /// local folder); nil = no. A version an image was built from is kept either way.
    public var keepRecipeVersions: Bool?

    public init(flake: String? = nil, qemu: String? = nil, runner: String? = nil) {
        self.flake = flake
        self.qemu = qemu
        self.runner = runner
    }

    public static func load(_ paths: Paths) -> Settings { load(file: paths.configFile) }

    public static func load(file: URL) -> Settings {
        guard let data = try? Data(contentsOf: file) else { return Settings() }
        return (try? JSONDecoder().decode(Settings.self, from: data)) ?? Settings()
    }

    public func save(_ paths: Paths) throws { try save(file: paths.configFile) }

    /// The image flake's folder was `flavors/` (until plan B92), then `recipes/` (until B107), now `nix/`: a `flake`
    /// setting pointing to an old folder without a flake.nix moves to the `nix` folder next to it, if that holds one.
    /// Returns the new path.
    @discardableResult
    public static func migrateRecipesFolder(_ paths: Paths) -> String? {
        var s = load(paths)
        guard let flake = s.flake, !flake.contains(":") else { return nil }
        let url = URL(fileURLWithPath: (flake as NSString).expandingTildeInPath).standardizedFileURL
        let fm = FileManager.default
        let old = url.lastPathComponent
        guard ["flavors", "recipes"].contains(old), !fm.fileExists(atPath: url.appendingPathComponent("flake.nix").path) else { return nil }
        let nix = url.deletingLastPathComponent().appendingPathComponent("nix")
        guard fm.fileExists(atPath: nix.appendingPathComponent("flake.nix").path) else { return nil }
        // Keep the user's spelling (e.g. ~/…) and only swap the last part
        let trimmed = flake.hasSuffix("/") ? String(flake.dropLast()) : flake
        s.flake = String(trimmed.dropLast(old.count)) + "nix"
        do { try s.save(paths) } catch { return nil }
        return s.flake
    }

    public func save(file: URL) throws {
        try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        try enc.encode(self).write(to: file, options: .atomic)
    }

    /// Flake path: VM_FLAKE > config.json
    public func flakePath() throws -> String {
        if let env = ProcessInfo.processInfo.environment["VM_FLAKE"], !env.isEmpty { return env }
        if let flake { return (flake as NSString).expandingTildeInPath }
        if let bundled = Self.bundledFlake() { return bundled.path }
        throw VMError("Flake path unknown. Set it with: omaf config --flake <checkout>/nix  (or VM_FLAKE)")
    }

    /// The image flake that comes with omaflake.app (Contents/Resources/nix, scripts/bundle-app.sh): the default when
    /// no path is set – for the app itself, and for the CLI next to an installed app
    public static func bundledFlake() -> URL? {
        let home = FileManager.default.homeDirectoryForCurrentUser
        let candidates = [Bundle.main.resourceURL?.appendingPathComponent("nix"),
                          URL(fileURLWithPath: "/Applications/omaflake.app/Contents/Resources/nix"),
                          home.appendingPathComponent("Applications/omaflake.app/Contents/Resources/nix")].compactMap { $0 }
        return candidates.first { FileManager.default.fileExists(atPath: $0.appendingPathComponent("flake.nix").path) }
    }
}
