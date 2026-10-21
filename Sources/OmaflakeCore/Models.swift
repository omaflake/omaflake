// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Network mode from recipe.toml (spec §6.3).
public enum NetworkMode: String, Codable, Sendable, CaseIterable {
    case shared    // -netdev user with SSH forwarding
    case isolated  // -netdev user,restrict=on – no outside access
    case none      // no network card
}

/// Guest kind of a recipe or instance (spec §3): NixOS on QEMU or macOS on Virtualization.framework.
public enum GuestKind: String, Codable, Sendable, CaseIterable {
    case nixos
    case macos
}

/// Runtime defaults of a recipe (its recipe.toml).
public struct RecipeDefaults: Codable, Sendable, Equatable {
    /// "nixos" if missing
    public var kind: GuestKind
    /// What it can be built for (`Platform`, plan B114), e.g. ["aarch64-linux", "x86_64-linux"]
    public var systems: [String]
    public var cpus: Int
    public var memoryGB: Int
    public var diskGB: Int
    public var network: NetworkMode
    public var share: Bool
    public var ephemeral: Bool
    /// Login user inside the guest (for SSH); nil = the user running omaflake on the Mac
    public var user: String?
    /// Name shown to the user ("minimal"); the dictionary key is the recipe ID "<author>.<kind>.<name>"
    public var name: String?
    /// Who publishes the recipe ("omaflake" for the presets, "local" for a developer's own)
    public var author: String?
    /// Sound from the VM to the Mac (default on)
    public var audio: Bool?
    /// The Mac's microphone in the VM (default off – macOS asks for permission the first time)
    public var microphone: Bool?
    /// Clipboard shared with the Mac (default on)
    public var clipboard: Bool?
    /// The Mac's camera in the VM (default off; NixOS guests – only while an app in the VM uses it)
    public var camera: Bool?
    /// What the recipe is for (shown behind an "i"); `description_de` etc. in the app's language when present
    public var description: String?
    public var descriptions: [String: String] = [:]
    /// A development recipe: shown only in developer mode
    public var lab: Bool?
    /// How a plain "start" shows the VM: none (headless), window, gpu (window with 3D where QEMU can) – M5
    public var display: DisplayMode?
    public var isLab: Bool { lab ?? false }
    /// Words to find the recipe by (e.g. "latex", "neovim"); lowercase, shown in the app's recipe row (plan B97)
    public var tags: [String] = []
    /// SPDX identifier and where to read more (omaflake/recipes, CONTRIBUTING.md) – shown in the recipe's details
    public var license: String?
    public var homepage: String?

    /// It can be built and run on this host
    public var runsHere: Bool { Platform.hostSystem(for: kind).map(systems.contains) ?? false }

    /// The description in `language` ("de"), else the English one
    public func description(in language: String?) -> String? {
        language.flatMap { descriptions[$0] } ?? description
    }

    enum CodingKeys: String, CodingKey {
        case kind, systems, cpus, network, share, ephemeral, user, name, author, audio, microphone, clipboard, camera, description, lab, display, tags, license, homepage
        case descriptionDe = "description_de"
        case memoryGB = "memory_gb"
        case diskGB = "disk_gb"
    }

    /// `systems` nil = what this host runs of `kind`
    public init(kind: GuestKind = .nixos, systems: [String]? = nil, cpus: Int, memoryGB: Int, diskGB: Int,
                network: NetworkMode, share: Bool, ephemeral: Bool, user: String? = nil) {
        self.kind = kind
        self.systems = systems ?? Platform.hostSystem(for: kind).map { [$0] } ?? []
        self.cpus = cpus
        self.memoryGB = memoryGB
        self.diskGB = diskGB
        self.network = network
        self.share = share
        self.ephemeral = ephemeral
        self.user = user
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        self.init(kind: try c.decodeIfPresent(GuestKind.self, forKey: .kind) ?? .nixos,
                  systems: try c.decode([String].self, forKey: .systems),
                  cpus: try c.decode(Int.self, forKey: .cpus),
                  memoryGB: try c.decode(Int.self, forKey: .memoryGB),
                  diskGB: try c.decode(Int.self, forKey: .diskGB),
                  network: try c.decode(NetworkMode.self, forKey: .network),
                  share: try c.decode(Bool.self, forKey: .share),
                  ephemeral: try c.decode(Bool.self, forKey: .ephemeral),
                  user: try c.decodeIfPresent(String.self, forKey: .user))
        name = try c.decodeIfPresent(String.self, forKey: .name)
        author = try c.decodeIfPresent(String.self, forKey: .author)
        audio = try c.decodeIfPresent(Bool.self, forKey: .audio)
        microphone = try c.decodeIfPresent(Bool.self, forKey: .microphone)
        clipboard = try c.decodeIfPresent(Bool.self, forKey: .clipboard)
        camera = try c.decodeIfPresent(Bool.self, forKey: .camera)
        description = try c.decodeIfPresent(String.self, forKey: .description)
        if let de = try c.decodeIfPresent(String.self, forKey: .descriptionDe) { descriptions["de"] = de }
        lab = try c.decodeIfPresent(Bool.self, forKey: .lab)
        display = try c.decodeIfPresent(DisplayMode.self, forKey: .display)
        tags = try c.decodeIfPresent([String].self, forKey: .tags) ?? []
        license = try c.decodeIfPresent(String.self, forKey: .license)
        homepage = try c.decodeIfPresent(String.self, forKey: .homepage)
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(kind, forKey: .kind)
        try c.encode(systems, forKey: .systems)
        try c.encode(cpus, forKey: .cpus)
        try c.encode(memoryGB, forKey: .memoryGB)
        try c.encode(diskGB, forKey: .diskGB)
        try c.encode(network, forKey: .network)
        try c.encode(share, forKey: .share)
        try c.encode(ephemeral, forKey: .ephemeral)
        try c.encodeIfPresent(user, forKey: .user)
        try c.encodeIfPresent(name, forKey: .name)
        try c.encodeIfPresent(author, forKey: .author)
        try c.encodeIfPresent(audio, forKey: .audio)
        try c.encodeIfPresent(microphone, forKey: .microphone)
        try c.encodeIfPresent(clipboard, forKey: .clipboard)
        try c.encodeIfPresent(camera, forKey: .camera)
        try c.encodeIfPresent(description, forKey: .description)
        try c.encodeIfPresent(descriptions["de"], forKey: .descriptionDe)
        try c.encodeIfPresent(lab, forKey: .lab)
        try c.encodeIfPresent(display, forKey: .display)
        if !tags.isEmpty { try c.encode(tags, forKey: .tags) }
        try c.encodeIfPresent(license, forKey: .license)
        try c.encodeIfPresent(homepage, forKey: .homepage)
    }
}

/// Contents of instances/<name>/instance.json. Settings are copied from the recipe on
/// `omaf new` and belong to the instance afterwards.
public struct InstanceConfig: Codable, Sendable, Equatable {
    /// instance.json, schema `instance` (plan B114). 0 → 1: "flavor" became "recipe" (plan B92), `kind` is always
    /// there (NixOS when missing), `system` added – every VM before it was made on a Mac with Apple silicon
    public static let fileFormat = FileFormat("instance", file: "instance.json", steps: [
        { o in
            FileFormat.renameKey("flavor", to: "recipe", in: &o)
            if o["kind"] == nil { o["kind"] = GuestKind.nixos.rawValue }
            o["system"] = (o["kind"] as? String) == GuestKind.macos.rawValue ? "aarch64-darwin" : "aarch64-linux"
        },
    ])

    public var name: String
    public var recipe: String
    /// Image name, e.g. "dev-ab12cd34ef-1a2b3c4d" (folder under ~/.omaflake/images)
    public var image: String
    /// Absolute path of the base image (backing file of the overlay)
    /// Where the image was when the VM was made (informational since plan B108: the overlay names its image by a
    /// relative path, `InstanceStore.backingImage` finds it)
    public var imagePath: String
    public var cpus: Int
    public var memoryGB: Int
    public var diskGB: Int
    public var network: NetworkMode
    public var share: Bool
    public var ephemeral: Bool
    public var sshPort: Int
    public var created: Date
    /// Login user inside the guest; missing in instances created before this field existed
    public var user: String?
    public var kind: GuestKind
    /// What it runs as – its image's system (`Platform`)
    public var system: String
    /// Sound to the Mac; nil = on
    public var audio: Bool?
    /// The Mac's microphone; nil = off
    public var microphone: Bool?
    /// Clipboard shared with the Mac; nil = on
    public var clipboard: Bool?
    public var clipboardOn: Bool { clipboard ?? true }
    /// The Mac's camera; nil = off
    public var camera: Bool?
    public var cameraOn: Bool { camera ?? false }
    /// How a plain start shows the VM (from the recipe); nil = headless in the CLI, a window in the app
    public var display: DisplayMode?
    public var soundOn: Bool { audio ?? true }
    public var microphoneOn: Bool { soundOn && (microphone ?? false) }
    /// Share folder the user chose (absolute path); nil = the default `share/<name>` in omaflake's folder
    public var sharePath: String?
    /// The VM's identity as three words (its omaflake ID `vm:<name>:<words>`, plan B110): random when it is made, new
    /// for a copy under another name, kept by a restore
    public var identity: String?

    public var guestKind: GuestKind { kind }

    /// User for SSH: the recipe's `user`, otherwise the user running omaflake on the Mac.
    public var guestUser: String { user ?? NSUserName() }

    public init(name: String, recipe: String, image: String, imagePath: String, system: String,
                settings: RecipeDefaults, sshPort: Int, created: Date = Date()) {
        self.name = name
        self.recipe = recipe
        self.image = image
        self.imagePath = imagePath
        self.cpus = settings.cpus
        self.memoryGB = settings.memoryGB
        self.diskGB = settings.diskGB
        self.network = settings.network
        self.share = settings.share
        self.ephemeral = settings.ephemeral
        self.sshPort = sshPort
        self.created = created
        self.user = settings.user
        self.kind = settings.kind
        self.system = system
        self.audio = settings.audio
        self.microphone = settings.microphone
        self.clipboard = settings.clipboard
        self.camera = settings.camera
        self.display = settings.display
    }
}

/// How `stop` ended.
public enum StopResult: String, Sendable {
    case alreadyStopped = "was already stopped"
    case powerdown = "shut down cleanly"
    case quit = "stopped via QMP quit"
    case forcedOff = "forced off (omaf-runner kill)"
    case killed = "killed (SIGKILL)"
}

public enum VMStatus: String, Sendable {
    case stopped = "stopped"
    case running = "running"
}

/// Display mode at start.
public enum DisplayMode: String, Codable, Sendable, CaseIterable {
    case none    // headless: serial console + SSH only
    case window  // Cocoa window with virtio-gpu-pci (no 3D)
    case gpu     // Cocoa window with virtio-gpu-gl-pci (virglrenderer, custom QEMU build, M2)
}

/// OpenGL path for `-display cocoa,gl=…`: native macOS OpenGL (CGL, default) or ANGLE (OpenGL ES over Metal).
/// With ANGLE, virgl offers the guest only GLSL 130 (GL 3.0) – kitty then fails to start (plan B22).
public enum GLBackend: String, Sendable, CaseIterable {
    case es  // ANGLE
    case on  // CGL
}

/// What the QEMU binary supports (custom build vs. Homebrew).
public struct QEMUCapabilities: Sendable, Equatable {
    /// `virtio-gpu-gl-pci` available
    public var gl: Bool
    /// 9p option `guest_owner_uid/gid` (patch qemu-9p-guest-owner, plan B18)
    public var guestOwner: Bool

    public init(gl: Bool, guestOwner: Bool) {
        self.gl = gl
        self.guestOwner = guestOwner
    }
}

public struct VMError: LocalizedError, CustomStringConvertible, Sendable {
    public let description: String
    public init(_ description: String) { self.description = description }
    public var errorDescription: String? { description }
}

/// Recipe identity: <author>/<kind>/<name> – e.g. omaflake/nixos/minimal and omaflake/macos/minimal. As an ID (flake
/// outputs, images, instances, `current/` links) it is "<author>.<kind>.<name>"; users see and type the name, or as much
/// of <author>/<kind>/<name> as makes it unique.
/// A recipe's ID (plan B110), the file form of its omaflake ID (`OmaflakeID`): "<author>.<kind>.<name>@<words>" – the
/// base (author, kind, name) and its content hash (`RecipeLibrary.contentHash`) as three words. The same base with other content is another recipe, also from
/// the same source; the same base with the same content exists once. A recipe's **line** is its source and base
/// (`lineKey`): a changed recipe is a new recipe in the same line – images are outdated and VMs upgrade within their
/// line. IDs without a hash are from before B110 (migrated) or from the plain recipe layout (`nix flake check` against a
/// recipes checkout).
public enum RecipeID {
    public static func make(author: String, kind: GuestKind, name: String) -> String { "\(author).\(kind.rawValue).\(name)" }

    /// "omaflake.nixos.minimal" + a content hash (hex) → "omaflake.nixos.minimal@velvet-tiger-orbit"
    public static func make(base: String, contentHash: String, words count: Int = OmaflakeID.length) -> String {
        "\(base)@\(OmaflakeID.words(hex: contentHash, count: count))"
    }

    /// "omaflake.nixos.minimal@velvet-tiger-orbit" → "omaflake.nixos.minimal"
    public static func base(_ id: String) -> String { id.split(separator: "@", maxSplits: 1).first.map(String.init) ?? id }

    /// "omaflake.nixos.minimal@velvet-tiger-orbit" → "velvet-tiger-orbit"; nil without words
    public static func words(_ id: String) -> String? {
        let p = id.split(separator: "@", maxSplits: 1)
        return p.count == 2 ? String(p[1]) : nil
    }

    /// The line of a recipe – its source (`RecipeSource.id`) and base – as a file-name-safe key ("<base>~<source tag>")
    public static func lineKey(base: String, source: String) -> String {
        "\(self.base(base))~\(RecipeSource.tag(forKey: source))"
    }

    /// (author, kind, name) of an ID, with or without its hash; nil for anything else
    public static func parts(_ id: String) -> (author: String, kind: GuestKind, name: String)? {
        let p = base(id).split(separator: ".", omittingEmptySubsequences: false).map(String.init)
        guard p.count == 3, let kind = GuestKind(rawValue: p[1]) else { return nil }
        return (p[0], kind, p[2])
    }

    /// What to show next to a recipe's name so its line can be told apart (plan B110, B111): its source when a recipe of
    /// the same base comes from another source; nil otherwise. Versions of one line are told apart by their words (shown
    /// as short as `OmaflakeID.Lengths` allows). `entries`: the recipes there are, ID → source.
    public static func distinction(_ id: String, among entries: [String: String], sourceName: (String) -> String) -> String? {
        guard let source = entries[id] else { return nil }
        let other = entries.contains { $0.key != id && base($0.key) == base(id) && $0.value != source }
        return other ? sourceName(source) : nil
    }

    /// Name to show: the recipe's `name`, else the name part of the ID
    public static func displayName(_ id: String, _ d: RecipeDefaults?) -> String { d?.name ?? name(ofID: id) }

    /// "omaflake.nixos.minimal" → "minimal"; other strings stay as they are
    public static func name(ofID id: String) -> String { parts(id)?.name ?? id }

    /// "omaflake.nixos.minimal" → "omaflake/nixos/minimal"
    public static func qualified(_ id: String) -> String {
        parts(id).map { "\($0.author)/\($0.kind.rawValue)/\($0.name)" } ?? id
    }

    /// Recipes matching what the user typed: an ID, <author>/<kind>/<name>, <kind>/<name> or <name>.
    public static func candidates(_ input: String, in defaults: [String: RecipeDefaults]) -> [String] {
        candidates(input, among: Array(defaults.keys))
    }

    /// "…@<words>" (or the start of them) in the input picks one; so does a recipe's omaflake ID.
    public static func candidates(_ input: String, among ids: [String]) -> [String] {
        if ids.contains(input) { return [input] }
        if let o = OmaflakeID.parse(input), o.kind == .recipe, let base = RecipeRef.publicID(o.identifier) {
            return ids.filter { RecipeID.base($0) == base && RecipeID.words($0)?.hasPrefix(o.words) == true }.sorted()
        }
        let wanted = words(input)
        func hashMatches(_ id: String) -> Bool { wanted.map { w in words(id)?.hasPrefix(w) == true } ?? true }
        let p = base(input).split(separator: "/").map(String.init)
        if p.count == 1, input.contains("."), ids.contains(where: { base($0) == base(input) }) {
            return ids.filter { base($0) == base(input) && hashMatches($0) }.sorted()
        }
        return Set(ids).filter { id in
            guard let f = parts(id), hashMatches(id) else { return false }
            switch p.count {
            case 3: return f.author == p[0] && f.kind.rawValue == p[1] && f.name == p[2]
            case 2: return f.kind.rawValue == p[0] && f.name == p[1]
            case 1: return f.name == p[0]
            default: return false
            }
        }.sorted()
    }

    /// The single recipe for `input`; `choose` decides between several (e.g. asks in a terminal), nil = error.
    public static func resolve(_ input: String, in defaults: [String: RecipeDefaults],
                               choose: (([String]) -> String?)? = nil) throws -> String {
        try resolve(input, among: Array(defaults.keys), choose: choose)
    }

    public static func resolve(_ input: String, among ids: [String], choose: (([String]) -> String?)? = nil) throws -> String {
        let found = candidates(input, among: ids)
        if found.count == 1 { return found[0] }
        if found.isEmpty { throw VMError("Recipe \"\(input)\" is not in your recipes") }
        if let choose, let picked = choose(found) { return picked }
        throw VMError("Recipe \"\(input)\" exists several times – choose one: \(found.map { OmaflakeID.recipe($0, words: OmaflakeID.shortest(Dictionary(grouping: found, by: base).values.map { $0.compactMap(words) })) }.joined(separator: ", "))")
    }
}
