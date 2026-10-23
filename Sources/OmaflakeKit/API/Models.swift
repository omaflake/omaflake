// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

// The types of api.md §4 as the core sends them. Optional fields are left out by the server when unset; the
// decoders here fill in the documented defaults, so a missing field never fails a whole list.

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
    public var system: String?
    public var status: VMStatus
    /// An action on it is in progress in this server
    public var busy: Bool
    public var cpus: Int
    public var memoryGB: Int
    public var diskGB: Int
    public var network: NetworkMode
    public var share: Bool
    /// The folder shared (the default share/<name> or the user's)
    public var sharePath: String?
    public var ephemeral: Bool
    /// NixOS: forwarded SSH port on 127.0.0.1
    public var sshPort: Int?
    /// Guest user (SSH)
    public var user: String?
    public var audio: Bool
    public var microphone: Bool
    public var clipboard: Bool
    public var camera: Bool
    public var display: DisplayMode?
    public var created: Date?
    /// Its omaflake ID words (spec §11.11)
    public var identity: String?
    public var omaflakeId: String?
    /// As the UI shows it
    public var label: String?
    /// macOS guests while running: NAT address
    public var address: String?
    /// What the omaflake agent last reported (spec §11.9), while running
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
        recipe = try c.value(.recipe, "")
        image = try c.value(.image, "")
        kind = try c.value(.kind, .unknown)
        system = try c.decodeIfPresent(String.self, forKey: .system)
        status = try c.value(.status, .stopped)
        busy = try c.value(.busy, false)
        cpus = try c.value(.cpus, 0)
        memoryGB = try c.value(.memoryGB, 0)
        diskGB = try c.value(.diskGB, 0)
        network = try c.value(.network, .shared)
        share = try c.value(.share, false)
        sharePath = try c.decodeIfPresent(String.self, forKey: .sharePath)
        ephemeral = try c.value(.ephemeral, false)
        sshPort = try c.decodeIfPresent(Int.self, forKey: .sshPort)
        user = try c.decodeIfPresent(String.self, forKey: .user)
        audio = try c.value(.audio, false)
        microphone = try c.value(.microphone, false)
        clipboard = try c.value(.clipboard, false)
        camera = try c.value(.camera, false)
        display = try c.decodeIfPresent(DisplayMode.self, forKey: .display)
        created = c.lenientDate(.created)
        identity = try c.decodeIfPresent(String.self, forKey: .identity)
        omaflakeId = try c.decodeIfPresent(String.self, forKey: .omaflakeId)
        label = try c.decodeIfPresent(String.self, forKey: .label)
        address = try c.decodeIfPresent(String.self, forKey: .address)
        agent = try? c.decodeIfPresent(AgentState.self, forKey: .agent)
        location = try c.decodeIfPresent(String.self, forKey: .location)
        latestImage = try c.decodeIfPresent(String.self, forKey: .latestImage)
        upgradable = try c.value(.upgradable, false)
    }
}

/// A file this omaflake cannot read – too old, too new, damaged (plan B114)
public struct Unreadable: Decodable, Sendable, Equatable, Hashable {
    public var path: String
    public var type: String?
    public var version: Int?
    public var reason: String
    /// The VM, image or recipe it belongs to, when the server names it (OPEN in api.md: proposed)
    public var name: String?

    enum CodingKeys: String, CodingKey { case path, type, version, reason, name }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        path = try c.value(.path, "")
        type = try c.decodeIfPresent(String.self, forKey: .type)
        version = try? c.decodeIfPresent(Int.self, forKey: .version)
        reason = try c.value(.reason, "")
        name = try c.decodeIfPresent(String.self, forKey: .name)
    }

    /// What to call it in a list: its name, or the folder the file lies in (…/instances/<name>/config.json → <name>)
    public var displayName: String {
        if let name { return name }
        let url = URL(fileURLWithPath: path)
        return url.pathExtension.isEmpty ? url.lastPathComponent : url.deletingLastPathComponent().lastPathComponent
    }
}

public struct VmList: Decodable, Sendable, Equatable {
    public var vms: [Vm]
    public var unreadable: [Unreadable]

    public init(vms: [Vm] = [], unreadable: [Unreadable] = []) {
        self.vms = vms
        self.unreadable = unreadable
    }

    enum CodingKeys: String, CodingKey { case vms, unreadable }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        vms = try c.value(.vms, [])
        unreadable = try c.value(.unreadable, [])
    }
}

// MARK: - Image (§4.2) – `VmImage`, as SwiftUI has an `Image`

public struct VmImage: Decodable, Sendable, Equatable, Identifiable {
    public var id: String { name }
    public var name: String
    public var recipe: String
    public var kind: GuestKind
    public var system: String?
    /// The recipes' git revision ("abc123", "abc123-dirty", "nogit")
    public var revision: String?
    public var storePath: String?
    public var created: Date?
    public var line: String
    public var path: String?
    public var sizeBytes: Int64?
    /// Its omaflake ID as short as it can be
    public var label: String?
    /// VM names using it
    public var instances: [String]
    /// It is the newest of its line
    public var current: Bool
    /// The recipe evaluates to something else now (plan B73/B82); nil: not known (OPEN in api.md: proposed)
    public var outdated: Bool?
    /// Where the recipe was built from (the flake or recipe folder; OPEN in api.md: proposed)
    public var source: String?
    /// The whole omaflake ID (OPEN in api.md: proposed; `label` is the short one)
    public var omaflakeId: String?

    enum CodingKeys: String, CodingKey {
        case name, recipe, kind, system, revision, storePath, created, line, path, sizeBytes, label, instances, current
        case outdated, source, omaflakeId
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        name = try c.decode(String.self, forKey: .name)
        recipe = try c.value(.recipe, "")
        kind = try c.value(.kind, .unknown)
        system = try c.decodeIfPresent(String.self, forKey: .system)
        revision = try c.decodeIfPresent(String.self, forKey: .revision)
        storePath = try c.decodeIfPresent(String.self, forKey: .storePath)
        created = c.lenientDate(.created)
        line = try c.value(.line, "")
        path = try c.decodeIfPresent(String.self, forKey: .path)
        sizeBytes = try? c.decodeIfPresent(Int64.self, forKey: .sizeBytes)
        label = try c.decodeIfPresent(String.self, forKey: .label)
        instances = try c.value(.instances, [])
        current = try c.value(.current, false)
        outdated = try c.decodeIfPresent(Bool.self, forKey: .outdated)
        source = try c.decodeIfPresent(String.self, forKey: .source)
        omaflakeId = try c.decodeIfPresent(String.self, forKey: .omaflakeId)
    }
}

public struct ImageList: Decodable, Sendable, Equatable {
    public var images: [VmImage]
    public var unreadable: [Unreadable]

    public init(images: [VmImage] = [], unreadable: [Unreadable] = []) {
        self.images = images
        self.unreadable = unreadable
    }

    enum CodingKeys: String, CodingKey { case images, unreadable }
    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        images = try c.value(.images, [])
        unreadable = try c.value(.unreadable, [])
    }
}
