// SPDX-License-Identifier: AGPL-3.0-or-later
import ArgumentParser
import Darwin
import Foundation
import OmaflakeCore

// omaf – build, create, start and reset recipe VMs (spec §6.2)

@main
struct VM: AsyncParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "omaf",
        abstract: "Clean NixOS and macOS VMs from Nix recipes.",
        subcommands: [Recipes.self, Sources.self, Build.self, MacOS.self, Images.self, New.self, SetCmd.self, Start.self, Stop.self, Reset.self, Upgrade.self,
                      Remove.self, List.self, Export.self, Import.self, External.self, SSH.self, Wait.self, Agent.self, Log.self, Config.self, Doctor.self, Builder.self]
    )
}

/// Shared access to paths, stores and runtime.
struct Env {
    var paths = Paths.standard()

    init() {
        // Older image layouts and recipe names (before image.json); cheap when there is nothing to do
        for note in ImageStore(paths: paths).migrateLegacy() { FileHandle.standardError.write(Data("migrated: \(note)\n".utf8)) }
    }

    init(paths: Paths) { self.paths = paths }
    var settings: Settings { Settings.load(paths) }

    func backends() throws -> Backends { try Backends(paths: paths) }

    var images: ImageStore { ImageStore(paths: paths) }
}

func gib(_ bytes: UInt64) -> String {
    bytes < 1_073_741_824 ? "\(bytes / 1_048_576) MiB" : String(format: "%.1f GiB", Double(bytes) / 1_073_741_824)
}

func pad(_ s: String, _ n: Int) -> String { s.count >= n ? s + " " : s + String(repeating: " ", count: n - s.count) }

// MARK: - Recipes & images

struct Recipes: AsyncParsableCommand {
    // "flavors": the name before the rename (plan B92)
    static let configuration = CommandConfiguration(
        abstract: "Recipes from your sources: list them, install and remove them (spec §11.3).",
        subcommands: [RecipesList.self, RecipesVersions.self, RecipesAvailable.self, RecipesInstall.self, RecipesRemove.self],
        defaultSubcommand: RecipesList.self, aliases: ["flavors"])
}

struct RecipesList: ParsableCommand {
    static let configuration = CommandConfiguration(commandName: "list",
        abstract: "The recipes omaflake can build (installed and in local folders) and the state of their images.")
    @Flag(help: "Also lab recipes (development; shown by default in developer mode)") var all = false
    @Flag(help: "Also recipes for other architectures (they cannot be built here)") var allSystems = false
    @Flag(help: "With each recipe's description") var long = false
    @Flag(help: "Evaluate each recipe and mark images it has outgrown (a few seconds per recipe)") var check = false

    func run() throws {
        let env = Env()
        let showLab = all || (env.settings.developerMode ?? false)
        let flake = try env.settings.flakePath()
        let all = try env.images.recipeDefaults(flake: flake).filter { showLab || !$0.value.isLab }
        let defaults = all.filter { allSystems || $0.value.runsHere }
        let outdated = check ? env.images.outdated(defaults.filter { $0.value.runsHere }, flake: flake) : []
        let distinctions = RecipeLibrary(paths: env.paths).distinctions(env.settings)
        let lengths = OmaflakeID.Lengths.current(env.paths)
        print(pad("RECIPE", 13) + pad("AUTHOR", 10) + pad("KIND", 7) + pad("ARCH", 16) + pad("CPU", 4) + pad("RAM", 6) + pad("DISK", 6) + pad("NETWORK", 9) + pad("SHARE", 6) + pad("EPHEMERAL", 10) + "IMAGE")
        for (id, d) in defaults.sorted(by: { ($0.value.kind.rawValue, $0.key) < ($1.value.kind.rawValue, $1.key) }) {
            let name = RecipeID.displayName(id, d)
            let arg = shortestArgument(id, in: defaults)
            let image = !d.runsHere ? "– (not for this computer)"
                : env.images.currentName(recipe: id).map { outdated.contains(id) ? "\($0)  ← recipe changed, rebuild: omaf build \(arg)" : $0 }
                ?? "– (omaf build \(arg))"
            let label = (d.isLab ? name + " ⚗" : name) + (distinctions[id].map { " (\($0))" } ?? "")
            let arch = d.systems.map(Platform.architecture).joined(separator: ",")
            print(pad(label, 13) + pad(d.author ?? "–", 10) + pad(d.kind.rawValue, 7) + pad(arch, 16) + pad("\(d.cpus)", 4) + pad("\(d.memoryGB)G", 6) + pad("\(d.diskGB)G", 6)
                  + pad(d.network.rawValue, 9) + pad(d.share ? "yes" : "no", 6) + pad(d.ephemeral ? "yes" : "no", 10) + image)
            if long {
                print("    \(OmaflakeID.recipe(id, words: lengths.recipe))")
                if let text = d.description { print("    \(text)") }
                if let origin = origins[id] { print("    from \(origin)") }
            }
        }
        for (id, version) in RecipeLibrary(paths: env.paths).unreadable(env.settings) {
            let found = version == 0 ? "no recipe $schema" : "recipe version \(version)"
            print("\(RecipeID.qualified(id)): recipe.toml has \(found), this omaflake reads version \(RecipeLibrary.recipeVersion) – update it from its source")
        }
        if defaults.count < all.count {
            print("\(all.count - defaults.count) more for other architectures: omaf recipes --all-systems")
        }
        if all.isEmpty {
            print("No recipes yet – install some: omaf recipes available, then omaf recipes install <author>/<kind>/<name>")
        }
    }

    /// Where each recipe comes from (a source's name, or "downloaded from …")
    var origins: [String: String] {
        let env = Env()
        let sources = Dictionary(env.settings.recipeSources.map { ($0.id, $0) }, uniquingKeysWith: { a, _ in a })
        var out: [String: String] = [:]
        for (id, _, source) in RecipeLibrary(paths: env.paths).available(env.settings) {
            out[id] = sources[source].map { $0.kind == .folder ? "the folder \($0.displayName)" : "\($0.displayName) (downloaded)" } ?? source
        }
        return out
    }
}

struct Build: ParsableCommand {
    static let configuration = CommandConfiguration(
        abstract: "Build a recipe's image (NixOS: nix build <flake>#images.<recipe>; macOS: from a base, spec §6.5).")
    @Argument(help: "Recipe: its name (e.g. dev), or <kind>/<name> when a NixOS and a macOS recipe share it (e.g. macos/minimal)") var recipe: String
    @Option(help: "macOS: base folder (default: newest ~/.omaflake/bases/mac-base-*)") var base: String?
    @Flag(help: "macOS: read the guest user's password (for sudo, once) from stdin instead of prompting") var passwordStdin = false
    @Flag(help: "macOS: keep a failed build folder for inspection instead of deleting it") var keepFailed = false
    @Flag(help: "Show every line of the build log, also the disk image tool's routine ones") var verbose = false

    func run() throws {
        let env = Env()
        let flake = try env.settings.flakePath()
        let defaults = try env.images.recipeDefaults(flake: flake)
        let recipe = try RecipeID.resolve(self.recipe, in: defaults, choose: chooseRecipe(env))
        let d = defaults[recipe]!
        guard d.runsHere else {
            throw VMError("\(RecipeID.qualified(recipe)) is for \(d.systems.joined(separator: ", ")) – not for this computer")
        }
        // Routine lines of the disk image tool are left out (BuildLog.isNoise, plan B83); --verbose shows everything
        let verbose = self.verbose
        let stderr: @Sendable (String) -> Void = { line in
            if verbose || !BuildLog.isNoise(line) { FileHandle.standardError.write(Data((line + "\n").utf8)) }
        }
        // macOS: base and password first (Ctrl-C at the prompt still simply quits)
        var mac: (builder: MacImageBuilder, base: URL, user: String, password: String)?
        if d.kind == .macos {
            let builder = MacImageBuilder(paths: env.paths, vz: try env.backends().vz)
            let baseURL = base.map { URL(fileURLWithPath: ($0 as NSString).expandingTildeInPath).standardizedFileURL }
            guard let baseDir = baseURL ?? builder.newestBase() else {
                throw VMError("No base in \(env.paths.bases.path) – create one with: omaf macos install")
            }
            let user = d.user ?? NSUserName()
            let password = try readPassword("Password of \(user) in \(baseDir.lastPathComponent) (for sudo): ", stdin: passwordStdin)
            mac = (builder, baseDir, user, password)
        }
        // From here Ctrl-C cancels cleanly: the running step stops, a macOS build VM is forced off, its folder deleted
        let token = Cancellation()
        signal(SIGINT, SIG_IGN)
        let interrupt = DispatchSource.makeSignalSource(signal: SIGINT, queue: .global())
        interrupt.setEventHandler {
            stderr("Cancelling …")
            token.cancel()
        }
        interrupt.resume()
        defer { interrupt.cancel(); signal(SIGINT, SIG_DFL) }
        let image: ImageStore.Image = try Cancellation.$current.withValue(token) {
            if let mac {
                return try mac.builder.build(.init(recipe: recipe, flake: flake, user: mac.user, password: mac.password,
                                                   base: mac.base, keepFailed: keepFailed), log: stderr)
            }
            return try env.images.build(recipe: recipe, flake: flake, log: stderr)
        }
        let n = OmaflakeID.Lengths.current(env.paths)
        print("Image \(OmaflakeID.image(image, words: n.image)) ready – new VMs of \(OmaflakeID.recipe(recipe, words: n.recipe)) use it.")
        if let used = try? Set(env.backends().store.list().map(\.image)) {
            for name in env.images.prune(recipe: recipe, used: used) { print("Removed older image \(name)") }
        }
    }
}

/// Password from the terminal (not echoed) or one line from stdin; never stored.
func readPassword(_ prompt: String, stdin: Bool, confirm: Bool = false) throws -> String {
    func ask(_ p: String) throws -> String {
        var buf = [CChar](repeating: 0, count: 1024)
        guard readpassphrase(p, &buf, buf.count, RPP_REQUIRE_TTY) != nil else { throw VMError("Cannot read a password (no terminal? use --password-stdin)") }
        defer { for i in buf.indices { buf[i] = 0 } }
        return String(decoding: buf.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
    }
    let password: String
    if stdin {
        password = readLine(strippingNewline: true) ?? ""
    } else {
        password = try ask(prompt)
        if confirm, try ask("Repeat: ") != password { throw VMError("Passwords do not match") }
    }
    guard !password.isEmpty else { throw VMError("Empty password") }
    return password
}

struct Images: ParsableCommand {
    static let configuration = CommandConfiguration(
        abstract: "Built images (Vorlagen): list them, remove one.",
        subcommands: [ImagesList.self, ImagesRemove.self], defaultSubcommand: ImagesList.self)
}

struct ImagesRemove: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "rm", abstract: "Delete an image (refused while instances use it).")
    @Argument(help: "Image name as listed by omaf images, its omaflake ID, or the start of either") var name: String

    func run() throws {
        let env = Env()
        let image = try resolveImage(name, env)
        try env.images.remove(image, instances: try env.backends().store)
        print("\(image) deleted.")
    }
}

struct ImagesList: ParsableCommand {
    static let configuration = CommandConfiguration(commandName: "list", abstract: "Built images and the instances using them.")

    func run() throws {
        let env = Env()
        let store = try env.backends().store
        let users = Dictionary(grouping: store.list(), by: \.image)
        let fmt = DateFormatter()
        fmt.dateFormat = "yyyy-MM-dd HH:mm"
        let lengths = OmaflakeID.Lengths.current(env.paths)
        for image in env.images.list() {
            let current = env.images.current(recipe: image.recipe)?.name == image.name ? "  (current)" : ""
            let used = users[image.name]?.map(\.name).joined(separator: ", ") ?? "–"
            print("\(image.name)\(current)")
            print("    ID:        \(OmaflakeID.image(image, words: lengths.image))")
            print("    Recipe:    \(RecipeID.qualified(image.recipe))   revision \(image.info.revision), \(fmt.string(from: image.info.created))")
            print("    System:    \(image.info.system)")
            print("    Instances: \(used)")
        }
    }
}

// MARK: - Instances

struct New: ParsableCommand {
    static let configuration = CommandConfiguration(
        abstract: "Create an instance (NixOS: overlay on the current image; macOS: APFS clone of it).")
    @Argument(help: "Instance name (a–z, 0–9, -)") var name: String
    @Option(name: .shortAndLong, help: "Recipe: its name, or <kind>/<name> (e.g. macos/minimal)") var recipe: String?
    /// --flavor / -f: the name before the rename (plan B92), not shown in the help
    @Option(name: [.customLong("flavor"), .customShort("f")], help: .hidden) var flavor: String?
    @Flag(inversion: .prefixedNo, help: "Ephemeral instance (changes are discarded on power-off)") var ephemeral: Bool?
    @Flag(inversion: .prefixedNo, help: "Share the Mac folder ~/.omaflake/share/<name> with the guest") var share: Bool?
    @Option(help: "Share this existing Mac folder instead (implies --share)") var shareDir: String?
    @Flag(inversion: .prefixedNo, help: "Sound from the VM to the Mac (default on)") var audio: Bool?
    @Flag(inversion: .prefixedNo, help: "The Mac's microphone in the VM (default off; macOS asks for permission)") var microphone: Bool?
    @Flag(inversion: .prefixedNo, help: "Clipboard shared with the Mac (default on)") var clipboard: Bool?
    @Flag(inversion: .prefixedNo, help: "The Mac's camera in the VM, NixOS (default off; on only while an app uses it)") var camera: Bool?
    @Option(help: "CPU cores") var cpus: Int?
    @Option(help: "RAM in GiB") var memory: Int?
    @Option(help: "Disk size in GiB") var disk: Int?
    @Option(help: "Network: shared | isolated | none") var network: String?
    @Option(help: "Guest user for SSH (default: user from the recipe, otherwise the Mac user)") var user: String?

    func validate() throws {
        guard (recipe ?? flavor) != nil else { throw ValidationError("Missing expected argument '--recipe <recipe>'") }
    }

    func run() throws {
        let env = Env()
        let backends = try env.backends()
        let defaults = try env.images.recipeDefaults(flake: try env.settings.flakePath())
        let recipe = try RecipeID.resolve(self.recipe ?? flavor!, in: defaults, choose: chooseRecipe(env))
        var settings = defaults[recipe]!
        let backend = try backends.backend(for: settings.kind)
        guard let image = env.images.current(recipe: recipe) else {
            throw VMError("No image for \"\(recipe)\" yet – first run: omaf build \(recipe)")
        }
        if let ephemeral { settings.ephemeral = ephemeral }
        if let audio { settings.audio = audio }
        if let microphone { settings.microphone = microphone }
        if let clipboard { settings.clipboard = clipboard }
        if let camera { settings.camera = camera }
        if settings.kind == .macos, settings.camera == true {
            FileHandle.standardError.write(Data("Note: macOS VMs get the camera port, but the guest's camera extension comes after M12 (plan B49).\n".utf8))
        }
        if let share { settings.share = share }
        if let cpus { settings.cpus = cpus }
        if let user { settings.user = user }
        if let memory { settings.memoryGB = memory }
        if let disk { settings.diskGB = disk }
        if let network {
            guard let mode = NetworkMode(rawValue: network) else { throw VMError("Unknown network mode \"\(network)\"") }
            settings.network = mode
        }
        var sharePath: String?
        if let shareDir {
            sharePath = URL(fileURLWithPath: (shareDir as NSString).expandingTildeInPath).standardizedFileURL.path
            var isDir: ObjCBool = false
            guard FileManager.default.fileExists(atPath: sharePath!, isDirectory: &isDir), isDir.boolValue else {
                throw VMError("Share folder missing: \(sharePath!)")
            }
            settings.share = true
        }
        var c = try backend.create(name: name, recipe: recipe, image: image, settings: settings)
        if let sharePath {
            c.sharePath = sharePath
            try backends.store.save(c)
        }
        let ssh = c.guestKind == .macos ? "SSH on its NAT address" : "SSH port \(c.sshPort)"
        print("Instance \(c.name) created (image \(c.image), \(ssh)\(c.ephemeral ? ", ephemeral" : "")).")
    }
}

struct Start: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "Start an instance (without a window by default).")
    @Argument var name: String
    @Flag(help: "Open a window (NixOS: with 3D if the QEMU binary supports virtio-gpu-gl)") var window = false
    @Flag(help: "Without a window, whatever the recipe's display says") var headless = false
    @Flag(help: "Window without 3D (virtio-gpu-pci)") var noGl = false
    @Option(help: "OpenGL path: on (native OpenGL, default) | es (ANGLE, only GL 3.0 in the guest)") var gl: String = "on"
    @Flag(help: "Wait until SSH is ready") var wait = false

    func run() throws {
        let env = Env()
        let backends = try env.backends()
        let c = try backends.store.load(name)
        let backend = try backends.backend(for: c)
        guard let glBackend = GLBackend(rawValue: gl) else { throw VMError("--gl must be es or on") }
        let lock = try InstanceLock(env.paths.instance(name))
        let display: DisplayMode
        // Without --window/--headless the recipe's `display` decides (default: headless)
        let wanted: DisplayMode = headless ? .none : window ? .gpu : (c.display ?? .none)
        if wanted == .none { display = .none }
        else if c.guestKind == .macos || noGl || wanted == .window { display = .window }
        else { display = try backends.qemu.runtime.capabilities().gl ? .gpu : .window }
        let pid = try backend.start(c, options: StartOptions(display: display, gl: glBackend))
        if window && c.guestKind == .nixos {
            print("Display: \(display == .gpu ? "3D (virtio-gpu-gl, gl=\(glBackend.rawValue))" : "no 3D")")
        }
        let ssh = c.network == .none ? "" : c.guestKind == .macos ? ", SSH once it has its NAT address (omaf wait)" : ", SSH 127.0.0.1:\(c.sshPort)"
        print("\(name) is running (PID \(pid)\(ssh)).")
        withExtendedLifetime(lock) {}
        if wait { try Wait.wait(c, backend: backend, timeout: 300) }
    }
}

struct Stop: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "Shut down cleanly; force off after the timeout.")
    @Argument var name: String
    @Option(help: "Seconds until forced power-off") var timeout: Double = 60

    func run() throws {
        let env = Env()
        let backends = try env.backends()
        let c = try backends.store.load(name)
        let backend = try backends.backend(for: c)
        let lock = try InstanceLock(env.paths.instance(name))
        let result = try backend.stop(c, timeout: timeout)
        withExtendedLifetime(lock) {}
        print("\(name): \(result.rawValue).")
    }
}

struct Reset: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "Reset: delete the instance's state and start over from its image.")
    @Argument var name: String

    func run() throws {
        let env = Env()
        let backends = try env.backends()
        let c = try backends.store.load(name)
        let backend = try backends.backend(for: c)
        let lock = try InstanceLock(env.paths.instance(name))
        guard backend.status(c) == .stopped else { throw VMError("\"\(name)\" is running – first run: omaf stop \(name)") }
        try backend.reset(c)
        withExtendedLifetime(lock) {}
        print("\(name) reset.")
    }
}

struct Upgrade: ParsableCommand {
    static let configuration = CommandConfiguration(
        abstract: "Upgrade to the recipe's current image (the old state is deleted).")
    @Argument var name: String

    func run() throws {
        let env = Env()
        let backends = try env.backends()
        let c = try backends.store.load(name)
        let backend = try backends.backend(for: c)
        guard let image = env.images.current(recipe: c.recipe) else {
            throw VMError("No image for \"\(c.recipe)\" – first run: omaf build \(c.recipe)")
        }
        let lock = try InstanceLock(env.paths.instance(name))
        guard backend.status(c) == .stopped else { throw VMError("\"\(name)\" is running – first run: omaf stop \(name)") }
        try backend.upgrade(c, to: image)
        withExtendedLifetime(lock) {}
        print("\(name) now uses \(image.name).")
    }
}

struct Remove: ParsableCommand {
    static let configuration = CommandConfiguration(commandName: "rm", abstract: "Delete an instance (its share folder stays).")
    @Argument var name: String

    func run() throws {
        let env = Env()
        let backends = try env.backends()
        let c = try backends.store.load(name)
        let backend = try backends.backend(for: c)
        let lock = try InstanceLock(env.paths.instance(name))
        guard backend.status(c) == .stopped else { throw VMError("\"\(name)\" is running – first run: omaf stop \(name)") }
        try backend.remove(c)
        withExtendedLifetime(lock) {}
        print("\(name) deleted.")
    }
}

struct List: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "Instances with status.")
    @Flag(help: "With each VM's omaflake ID and the recipe it comes from") var long = false

    func run() throws {
        let env = Env()
        let backends = try env.backends()
        let all = backends.store.list()
        guard !all.isEmpty else { print("No instances. Create one with: omaf new <name> --recipe <recipe>"); return }
        let lengths = OmaflakeID.Lengths.current(env.paths)
        print(pad("NAME", 16) + pad("STATUS", 8) + pad("KIND", 7) + pad("RECIPE", 13) + pad("SSH", 16) + pad("EPHEMERAL", 10) + pad("DISK", 10) + "IMAGE")
        for c in all {
            var used = gib(allocatedSizeOf(env.paths.instance(c.name).disk))
            let backend = try? backends.backend(for: c)
            let status = backend?.status(c)
            var ssh = "\(c.sshPort)"
            if c.guestKind == .macos {
                // NAT address while running. The disk is an APFS clone of the image; its own share (the changed
                // blocks) cannot be read cheaply, the allocated size would count the shared blocks too.
                ssh = status == .running ? (try? backends.vz.address(c)).flatMap { $0 } ?? "–" : "–"
                used = "clone"
            }
            print(pad(c.name, 16) + pad(status?.rawValue ?? "?", 8) + pad(c.guestKind.rawValue, 7) + pad(RecipeID.name(ofID: c.recipe), 13) + pad(ssh, 16)
                  + pad(c.ephemeral ? "yes" : "no", 10) + pad(used, 10)
                  + RecipeID.base(c.image) + (RecipeID.words(c.image).map { "@" + OmaflakeID.short($0, lengths.image) } ?? ""))
            if backends.store.isExternal(c.name) { print("    runs from \(env.paths.instance(c.name).dir.path)") }
            if long { print("    \(OmaflakeID.vm(c, words: lengths.vm))  from \(OmaflakeID.recipe(c.recipe, words: lengths.recipe))") }
        }
    }
}

func allocatedSizeOf(_ url: URL) -> UInt64 {
    UInt64((try? url.resourceValues(forKeys: [.totalFileAllocatedSizeKey]).totalFileAllocatedSize) ?? 0)
}

struct SSH: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "Connect via SSH as the guest user (requires ssh-agent or a key).")
    @Argument var name: String
    @Argument(parsing: .captureForPassthrough, help: "Command to run in the guest (optional)") var command: [String] = []

    func run() throws {
        let env = Env()
        let backends = try env.backends()
        let c = try backends.store.load(name)
        guard c.network != .none else { throw VMError("\"\(name)\" has no network – serial console only (omaf log)") }
        let args = try backends.backend(for: c).sshArguments(c, command: Array(command.drop(while: { $0 == "--" })))
        let argv = args.map { strdup($0) } + [nil]
        execv("/usr/bin/ssh", argv)
        throw VMError("Could not start ssh: \(String(cString: strerror(errno)))")
    }
}

struct Wait: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "Wait until the instance's SSH server responds.")
    @Argument var name: String
    @Option(help: "Seconds") var timeout: Double = 300

    func run() throws {
        let env = Env()
        let backends = try env.backends()
        let c = try backends.store.load(name)
        try Wait.wait(c, backend: try backends.backend(for: c), timeout: timeout)
    }

    static func wait(_ c: InstanceConfig, backend: any VMBackend, timeout: Double) throws {
        guard c.network != .none else { throw VMError("\"\(c.name)\" has no network – cannot check SSH") }
        let t0 = Date()
        guard backend.waitForSSH(c, timeout: timeout) else {
            throw VMError(backend.status(c) == .stopped ? "\"\(c.name)\" is not running" : "SSH not ready after \(Int(timeout)) s")
        }
        print("\(c.name): SSH ready after \(Int(Date().timeIntervalSince(t0))) s.")
    }
}

struct Agent: ParsableCommand {
    static let configuration = CommandConfiguration(
        abstract: "Show what the VM's omaflake agent reports (NixOS guests): hello and status.")
    @Argument var name: String
    @Option(help: "Wait up to this many seconds for the agent's hello") var wait: Double = 0
    @Flag(help: "Print the raw state (JSON)") var json = false

    func run() throws {
        let env = Env()
        let backends = try env.backends()
        let c = try backends.store.load(name)
        let backend = try backends.backend(for: c)
        guard backend.status(c) == .running else { throw VMError("\"\(name)\" is not running") }
        let t0 = Date()
        var state = backend.agentState(c)
        while state?.isReady != true, Date().timeIntervalSince(t0) < wait {
            usleep(500_000)
            state = backend.agentState(c)
        }
        guard let state else {
            throw VMError(c.guestKind == .macos ? "macOS guests have no omaflake agent yet (spec §11.9)"
                          : "No word from the agent (yet) – an image built before the agent has none; omaf wait checks SSH")
        }
        if json {
            print(String(decoding: try JSONEncoder.agent.encode(state), as: UTF8.self))
            return
        }
        if let problem = state.problem { print("Problem:  \(problem)") }
        if let h = state.hello {
            let at = state.helloAt.map { " at \($0.formatted(date: .omitted, time: .standard))" } ?? ""
            print("Agent:    \(h.agent), protocol \(state.protocolVersion.map(String.init) ?? "none")\(at)\(state.agentIsOlder ? " (older than this omaflake – rebuild the image)" : "")")
            print("Guest:    \([h.osName, h.kernel.map { "kernel \($0)" }, h.hostname].compactMap { $0 }.joined(separator: ", "))")
        }
        if let s = state.status {
            let gb = { (n: UInt64) in String(format: "%.1f GB", Double(n) / 1_073_741_824) }
            if let up = s.uptime { print("Uptime:   \(Int(up)) s") }
            if let l = s.load { print("Load:     \(l.map { String(format: "%.2f", $0) }.joined(separator: " "))") }
            if let m = s.memory { print("Memory:   \(gb(m.available)) of \(gb(m.total)) available") }
            if let d = s.disk { print("Disk:     \(gb(d.free)) of \(gb(d.total)) free (\(d.path))") }
            for a in s.addresses ?? [] { print("Address:  \(a.address) (\(a.interface))") }
            if let u = s.users, !u.isEmpty { print("Users:    \(u.joined(separator: ", "))") }
            if let at = state.statusAt { print("Reported: \(Int(Date().timeIntervalSince(at))) s ago") }
        }
        if state.goodbyeAt != nil { print("The agent said goodbye (the guest is shutting down).") }
    }
}

struct Log: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "Show the instance's serial console.")
    @Argument var name: String
    @Option(name: .customShort("n"), help: "Number of lines") var lines = 40
    @Flag(name: .short, help: "Keep following (tail -f)") var follow = false
    @Flag(help: "QEMU log instead of the serial console") var qemu = false

    func run() throws {
        let ip = Env().paths.instance(name)
        let file = qemu ? ip.qemuLog : ip.serialLog
        guard FileManager.default.fileExists(atPath: file.path) else { throw VMError("No log: \(file.path)") }
        let args = ["/usr/bin/tail", "-n", "\(lines)"] + (follow ? ["-f"] : []) + [file.path]
        let argv = args.map { strdup($0) } + [nil]
        execv("/usr/bin/tail", argv)
        throw VMError("Could not start tail")
    }
}

struct Config: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "Show/set settings in ~/.omaflake/config.json.")
    @Option(help: "Move the data folder (images, instances, …) there – \"default\" = ~/.omaflake; VMs must be stopped") var home: String?
    @Option(help: "The image flake: a folder (e.g. <checkout>/nix) or a flake URL") var flake: String?
    @Option(help: "Path to qemu-system-aarch64 (e.g. a custom build from qemu/build.sh)") var qemu: String?
    @Option(help: "Path to omaf-runner signed by scripts/sign-runner.sh (default: next to omaf)") var runner: String?
    @Option(help: "Path to the nix binary (default: PATH and the usual Nix locations)") var nix: String?
    @Option(help: "Guest user of NixOS VMs (default: your Mac user; macOS VMs use the base's user)") var guestUser: String?
    @Option(help: "Full name of the guest user") var guestFullName: String?
    @Option(help: "Locale in the guests, e.g. de_DE.UTF-8 (default: the Mac's)") var guestLocale: String?
    @Option(help: "Keyboard layout in the guests, e.g. de or us (default: the Mac's)") var guestKeyboard: String?
    @Option(help: "Time zone in the guests, e.g. Europe/Berlin (default: the Mac's)") var guestTimeZone: String?
    @Option(help: "Keep older versions of a recipe when a newer one comes: on or off (default off; versions images use stay)")
    var keepRecipeVersions: String?

    func run() throws {
        var env = Env()
        if let home {
            let target = home == "default" ? Paths.defaultHome
                : URL(fileURLWithPath: (home as NSString).expandingTildeInPath, isDirectory: true).standardizedFileURL
            env = Env(paths: try DataFolder.move(from: env.paths, to: target) { print($0) })
        }
        var s = env.settings
        if let flake {
            // Flake URLs (path:, github:, git+file:, …) as given; folders as absolute paths
            s.flake = flake.contains(":") ? flake
                : URL(fileURLWithPath: (flake as NSString).expandingTildeInPath).standardizedFileURL.path
        }
        if let qemu { s.qemu = qemu }
        if let runner { s.runner = URL(fileURLWithPath: (runner as NSString).expandingTildeInPath).standardizedFileURL.path }
        if let nix { s.nix = URL(fileURLWithPath: (nix as NSString).expandingTildeInPath).standardizedFileURL.path }
        let guestChange = [guestUser, guestFullName, guestLocale, guestKeyboard, guestTimeZone].contains { $0 != nil }
        if guestChange {
            // An empty value removes the override (the Mac's value applies again)
            var g = s.guest ?? GuestOverrides()
            func set(_ v: String?, _ kp: WritableKeyPath<GuestOverrides, String?>) { if let v { g[keyPath: kp] = v.isEmpty ? nil : v } }
            set(guestUser, \.user); set(guestFullName, \.fullName); set(guestLocale, \.locale)
            set(guestKeyboard, \.keyboard); set(guestTimeZone, \.timeZone)
            s.guest = g
        }
        if let keepRecipeVersions {
            guard ["on", "off"].contains(keepRecipeVersions) else { throw ValidationError("--keep-recipe-versions: on or off") }
            s.keepRecipeVersions = keepRecipeVersions == "on"
        }
        if flake != nil || qemu != nil || runner != nil || nix != nil || guestChange || keepRecipeVersions != nil { try s.save(env.paths) }
        if keepRecipeVersions == "off" { _ = RecipeLibrary(paths: env.paths).pruneVersions() }
        print("flake:  \(s.flake ?? "–")\nqemu:   \(s.qemu ?? "– (PATH)")\nrunner: \(s.runner ?? "– (next to omaf, then PATH)")\nnix:    \(s.nix ?? "– (PATH)")\nhome:   \(env.paths.root.path)\nconfig: \(env.paths.configFile.path)")
        let g = GuestSettings.current(env.paths)
        print("guest:  \(g.user) (\(g.fullName)), \(g.locale), keyboard \(g.keyboard), \(g.timeZone), macOS user \(g.macUser ?? g.user), \(g.authorizedKeys.count) key(s)")
        print("recipe versions: \((s.keepRecipeVersions ?? false) ? "older ones are kept" : "the newest and those images use")")
    }
}

struct Doctor: ParsableCommand {
    static let configuration = CommandConfiguration(
        abstract: "Check what your Mac brings and whether NixOS and macOS images can be built (preflight).")

    func run() throws {
        let env = Env()
        let pre = Prerequisites(paths: env.paths)
        let latest = (try? env.backends().vz).flatMap { MacOSRelease.latest(runner: $0.runner) }
        let groups = [("Requirements", pre.check()), ("NixOS preflight", pre.nixosPreflight()),
                      ("macOS preflight", pre.macosPreflight(latest: latest))]
        for (title, items) in groups {
            print(title)
            for p in items {
                let mark = p.status == .ok ? "✓" : p.status == .limited ? "~" : "✗"
                print("  \(mark) \(pad(p.item.rawValue, 15))\(p.detail)")
                if !p.hint.isEmpty { print("      \(p.hint)") }
                if let cmd = p.install { print("      \(cmd.replacingOccurrences(of: "\\\n", with: "\\\n      "))") }
            }
        }
        if groups[0].1.contains(where: { $0.required && $0.status == .missing }) { throw ExitCode(1) }
    }
}

struct Builder: ParsableCommand {
    static let configuration = CommandConfiguration(
        abstract: "Linux builder: apply omaflake's builder host, clean up its disk (runs sudo).",
        subcommands: [BuilderSetup.self, BuilderApply.self, BuilderCleanUp.self])
}

struct BuilderSetup: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "setup", abstract: "Set up nix-darwin with the Linux builder on a Mac without it (two stages, sudo).")

    func run() throws {
        let m = BuilderMaintenance(paths: Env().paths)
        guard !FileManager.default.isExecutableFile(atPath: "/run/current-system/sw/bin/darwin-rebuild") else {
            throw VMError("nix-darwin is already set up – use: omaf builder apply")
        }
        print("Stage 1 of 2: nix-darwin with the standard builder …")
        let first = try m.buildSystem(host: "linux-builder-bootstrap") { FileHandle.standardError.write(Data(($0 + "\n").utf8)) }
        try sudo(try m.rootScript(BuilderMaintenance.firstActivationCommand(system: first)))
        print("Waiting for the Linux builder …")
        let pre = Prerequisites(paths: m.paths)
        for _ in 0..<60 where pre.builderRunning().status != .ok { sleep(2) }
        print("Stage 2 of 2: omaflake's builder settings …")
        let second = try m.buildSystem { FileHandle.standardError.write(Data(($0 + "\n").utf8)) }
        try sudo(try m.rootScript(BuilderMaintenance.activateCommand(system: second)))
        print("nix-darwin and the Linux builder are set up.")
    }
}

struct BuilderApply: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "apply", abstract: "darwin-rebuild switch with the image flake's #linux-builder host.")
    @Flag(help: "Also when this Mac's nix-darwin system is not omaflake's (it gets replaced)") var force = false

    func run() throws {
        let m = BuilderMaintenance(paths: Env().paths)
        guard m.systemIsOmaflakes || force else {
            throw VMError("This Mac's nix-darwin system is not omaflake's builder host (\(BuilderMaintenance.marker) is missing). "
                          + "Applying replaces your nix-darwin configuration – run with --force if that is what you want.")
        }
        print("Building the builder host …")
        let system = try m.buildSystem { FileHandle.standardError.write(Data(($0 + "\n").utf8)) }
        try sudo(try m.rootScript(BuilderMaintenance.activateCommand(system: system)))
    }
}

struct BuilderCleanUp: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "clean-up", abstract: "Remove unused store paths in the builder (free space before and after).")

    func run() throws {
        let m = BuilderMaintenance(paths: Env().paths)
        try sudo(try m.rootScript(BuilderMaintenance.cleanUpCommand))
    }
}

/// Runs a root shell script through sudo in this terminal (password or Touch ID, as sudo is set up).
func sudo(_ script: String) throws {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: "/usr/bin/sudo")
    p.arguments = ["/bin/sh", "-c", script]
    try p.run()
    p.waitUntilExit()
    if p.terminationStatus != 0 { throw ExitCode(p.terminationStatus) }
}

/// Shortest way to name a recipe on the command line: <name>, <kind>/<name> or <author>/<kind>/<name>.
func shortestArgument(_ id: String, in defaults: [String: RecipeDefaults]) -> String {
    guard let p = RecipeID.parts(id) else { return id }
    for arg in [p.name, "\(p.kind.rawValue)/\(p.name)", RecipeID.qualified(id)] where RecipeID.candidates(arg, in: defaults) == [id] {
        return arg
    }
    return OmaflakeID.recipe(id, words: OmaflakeID.shortest([defaults.keys.filter { RecipeID.base($0) == RecipeID.base(id) }.compactMap(RecipeID.words)]))
}

/// In a terminal: lets the user pick one of several recipes (author, kind, name, current image); elsewhere nil.
func chooseRecipe(_ env: Env) -> ([String]) -> String? {
    { ids in
        guard isatty(STDIN_FILENO) != 0 else { return nil }
        let fmt = DateFormatter()
        fmt.dateFormat = "yyyy-MM-dd"
        print("Several recipes match:")
        for (i, id) in ids.enumerated() {
            let image = env.images.current(recipe: id).map { "image \($0.info.revision), \(fmt.string(from: $0.info.created))" } ?? "no image yet"
            print("  \(i + 1)) \(pad(OmaflakeID.recipe(id, words: OmaflakeID.shortest([ids.compactMap(RecipeID.words)])), 52))\(image)")
        }
        print("Choose [1-\(ids.count)]: ", terminator: "")
        guard let line = readLine(), let n = Int(line.trimmingCharacters(in: .whitespaces)), (1...ids.count).contains(n) else { return nil }
        return ids[n - 1]
    }
}

struct SetCmd: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "set", abstract: "Change an instance's sound, microphone, clipboard and camera (from its next start).")
    @Argument var name: String
    @Flag(inversion: .prefixedNo, help: "Sound from the VM to the Mac") var audio: Bool?
    @Flag(inversion: .prefixedNo, help: "The Mac's microphone in the VM (macOS asks for permission the first time)") var microphone: Bool?
    @Flag(inversion: .prefixedNo, help: "Clipboard shared with the Mac") var clipboard: Bool?
    @Flag(inversion: .prefixedNo, help: "The Mac's camera in the VM (NixOS; macOS asks for permission the first time)") var camera: Bool?

    func run() throws {
        let env = Env()
        let store = try env.backends().store
        let lock = try InstanceLock(env.paths.instance(name))
        var c = try store.load(name)
        if let audio { c.audio = audio }
        if let microphone { c.microphone = microphone }
        if let clipboard { c.clipboard = clipboard }
        if let camera { c.camera = camera }
        try store.save(c)
        withExtendedLifetime(lock) {}
        print("\(name): sound \(c.soundOn ? "on" : "off"), microphone \(c.microphoneOn ? "on" : "off"), "
              + "clipboard \(c.clipboardOn ? "on" : "off"), camera \(c.cameraOn ? "on" : "off") (from the next start)")
    }
}

/// An image by its name, its omaflake ID (image:<author>/<kind>/<name>:<words>) or the start of its name's words
func resolveImage(_ input: String, _ env: Env) throws -> String {
    let names = env.images.list().map(\.name)
    if names.contains(input) { return input }
    var base = RecipeID.base(input), words = RecipeID.words(input) ?? ""
    if let o = OmaflakeID.parse(input), o.kind == .image, let b = RecipeRef.publicID(o.identifier) { (base, words) = (b, o.words) }
    let found = names.filter { RecipeID.base($0) == base && (RecipeID.words($0)?.hasPrefix(words) ?? false) }
    guard found.count == 1 else {
        throw VMError(found.isEmpty ? "No image \(input) – see omaf images" : "\(input) is more than one image: \(found.joined(separator: ", "))")
    }
    return found[0]
}
