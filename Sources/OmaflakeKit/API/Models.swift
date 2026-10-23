// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

// The types of api.md §4 as the core sends them (crates/omaflake-core/src/api/types.rs). Fields the server always
// sends are decoded strictly – a missing one is a decoding error, never a silent default; optional ones are left out
// by the server when unset. Enums stay open (an unknown value is `.unknown`, api.md §2). The golden files in
// crates/omaflake-core/testdata/api/ pin both sides (APIContractTests).

public enum GuestKind: String, OpenEnum, CaseIterable {
    case nixos, macos, unknown
    public static var allCases: [GuestKind] { [.nixos, .macos] }
}

public enum VMStatus: String, OpenEnum { case stopped, running, unknown }

public enum NetworkMode: String, OpenEnum, CaseIterable {
    case shared, isolated, none, unknown
    public static var allCases: [NetworkMode] { [.shared, .isolated, .none] }
}

/// How a plain start shows a VM (Vm.display, Recipe.display)
public enum DisplayMode: String, OpenEnum { case none, window, gpu, unknown }

// MARK: - Vm (§4.1)

public struct Vm: Decodable, Sendable, Equatable, Identifiable {
    public var id: String { name }
    public var name: String
    public var recipe: String
    public var image: String
    public var kind: GuestKind
    public var system: String
    public var status: VMStatus
    /// An action on it is in progress in this server
    public var busy: Bool
    public var cpus: Int
    public var memoryGB: Int
    public var diskGB: Int
    public var network: NetworkMode
    public var share: Bool
    /// The folder shared (the default share/<name> or the user's), when `share` is on
    public var sharePath: String?
    public var ephemeral: Bool
    /// NixOS: forwarded SSH port on 127.0.0.1
    public var sshPort: Int?
    /// Guest user (SSH)
    public var user: String
    public var audio: Bool
    public var microphone: Bool
    public var clipboard: Bool
    public var camera: Bool
    /// How a plain start shows it
    public var display: DisplayMode
    public var created: Date
    /// Its omaflake ID words (spec §11.11), as many as `idLengths.vm`
    public var identity: String?
    /// Its whole omaflake ID (all words; shorten it to `idLengths.vm`)
    public var omaflakeId: String
    /// As the UI shows it
    public var label: String
    /// macOS guests while running: NAT address
    public var address: String?
    /// What the omaflake agent last reported (spec §11.9), while running – nil also when this app cannot read it
    public var agent: AgentState?
    /// The package's place when it runs from elsewhere (plan B108)
    public var location: String?
    /// The newest image of its recipe's line
    public var latestImage: String?
    public var upgradable: Bool

    public var isMac: Bool { kind == .macos }
    public var isRunning: Bool { status == .running }

    enum CodingKeys: String, CodingKey {
        case name, recipe, image, kind, system, status, busy, cpus, memoryGB, diskGB, network, share, sharePath, ephemeral
        case sshPort, user, audio, microphone, clipboard, camera, display, created, identity, omaflakeId, label, address
        case agent, location, latestImage, upgradable
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = try c.decode(String.self, forKey: .name)
        recipe = try c.decode(String.self, forKey: .recipe)
        image = try c.decode(String.self, forKey: .image)
        kind = try c.decode(GuestKind.self, forKey: .kind)
        system = try c.decode(String.self, forKey: .system)
        status = try c.decode(VMStatus.self, forKey: .status)
        busy = try c.decode(Bool.self, forKey: .busy)
        cpus = try c.decode(Int.self, forKey: .cpus)
        memoryGB = try c.decode(Int.self, forKey: .memoryGB)
        diskGB = try c.decode(Int.self, forKey: .diskGB)
        network = try c.decode(NetworkMode.self, forKey: .network)
        share = try c.decode(Bool.self, forKey: .share)
        sharePath = try c.decodeIfPresent(String.self, forKey: .sharePath)
        ephemeral = try c.decode(Bool.self, forKey: .ephemeral)
        sshPort = try c.decodeIfPresent(Int.self, forKey: .sshPort)
        user = try c.decode(String.self, forKey: .user)
        audio = try c.decode(Bool.self, forKey: .audio)
        microphone = try c.decode(Bool.self, forKey: .microphone)
        clipboard = try c.decode(Bool.self, forKey: .clipboard)
        camera = try c.decode(Bool.self, forKey: .camera)
        display = try c.decode(DisplayMode.self, forKey: .display)
        created = try c.decode(Date.self, forKey: .created)
        identity = try c.decodeIfPresent(String.self, forKey: .identity)
        omaflakeId = try c.decode(String.self, forKey: .omaflakeId)
        label = try c.decode(String.self, forKey: .label)
        address = try c.decodeIfPresent(String.self, forKey: .address)
        // The agent's report is the guest's (spec §11.9): a newer agent may say more than this app reads
        agent = try? c.decodeIfPresent(AgentState.self, forKey: .agent)
        location = try c.decodeIfPresent(String.self, forKey: .location)
        latestImage = try c.decodeIfPresent(String.self, forKey: .latestImage)
        upgradable = try c.decode(Bool.self, forKey: .upgradable)
    }
}

/// A file this omaflake cannot read – too old, too new, damaged (plan B114)
public struct Unreadable: Decodable, Sendable, Equatable, Hashable {
    /// The VM, image or recipe it belongs to (the list it comes in tells which)
    public var name: String
    public var reason: String
}

public struct VmList: Decodable, Sendable, Equatable {
    public var vms: [Vm]
    public var unreadable: [Unreadable]

    public init(vms: [Vm] = [], unreadable: [Unreadable] = []) {
        self.vms = vms
        self.unreadable = unreadable
    }
}

// MARK: - Image (§4.2) – `VmImage`, as SwiftUI has an `Image`

public struct VmImage: Decodable, Sendable, Equatable, Identifiable {
    public var id: String { name }
    public var name: String
    public var recipe: String
    /// The recipe source it was built from (its id), when known
    public var source: String?
    public var kind: GuestKind
    public var system: String
    /// The recipes' git revision ("abc123", "abc123-dirty", "nogit")
    public var revision: String
    public var storePath: String?
    public var created: Date
    public var line: String
    public var path: String
    /// Only in `images.get`
    public var sizeBytes: Int64?
    /// Its omaflake ID as short as it can be
    public var label: String
    /// Its whole omaflake ID
    public var omaflakeId: String
    /// VM names using it
    public var instances: [String]
    /// It is the newest of its line
    public var current: Bool
    /// The recipe evaluates to something else now (plan B73/B82); nil: not known
    public var outdated: Bool?
}

public struct ImageList: Decodable, Sendable, Equatable {
    public var images: [VmImage]
    public var unreadable: [Unreadable]

    public init(images: [VmImage] = [], unreadable: [Unreadable] = []) {
        self.images = images
        self.unreadable = unreadable
    }
}

// MARK: - Small answers (§7)

public enum StopResult: String, OpenEnum { case alreadyStopped, powerdown, quit, forcedOff, killed, unknown }

/// `vms.stop`
public struct StopAnswer: Decodable, Sendable, Equatable { public var result: StopResult }

/// `vms.set`
public struct VmSetAnswer: Decodable, Sendable, Equatable {
    public var vm: Vm
    public var takesEffectAtNextStart: Bool
}

/// `vms.ssh`
public struct SshAnswer: Decodable, Sendable, Equatable {
    public var argv: [String]
    public var shellLine: String
}

/// How a VM answered (`vms.wait`, the `vm.ready` event)
public enum Via: String, OpenEnum { case agent, ssh, unknown }

/// `vms.wait`
public struct WaitAnswer: Decodable, Sendable, Equatable { public var via: Via }

/// `vms.log`
public struct LogLines: Decodable, Sendable, Equatable { public var lines: [String] }

/// `vms.share`, the `export` job's result
public struct PathAnswer: Decodable, Sendable, Equatable { public var path: String }

/// `vms.nextName`
public struct NameAnswer: Decodable, Sendable, Equatable { public var name: String }

/// `vms.limits`
public struct Limits: Decodable, Sendable, Equatable {
    public var macMax: Int
    public var macRunning: [String]
}

/// `packages.inspect`
public struct PackageInspection: Decodable, Sendable, Equatable {
    public enum Kind: String, OpenEnum { case vm, image, recipe, unknown }
    public struct Conflict: Decodable, Sendable, Equatable {
        /// A VM of that name is here already …
        public var vmName: String?
        /// … and this is the first free name to import it under instead
        public var suggestedName: String?
        /// The same image is here already
        public var image: String?
    }

    public var kind: Kind
    public var valid: Bool
    public var reason: String?
    public var conflict: Conflict?
    public var networkVolume: Bool
}
