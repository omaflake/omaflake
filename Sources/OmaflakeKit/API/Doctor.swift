// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// One check of the Doctor (§4.4)
public struct Prerequisite: Decodable, Sendable, Equatable, Identifiable {
    public enum Item: String, OpenEnum {
        // what the Mac brings
        case host, nix, flakes, nixDarwin, linuxBuilder, qemu, flake, sshKey, git
        // preflight NixOS: builder VM answering, recipes evaluating, space in the Nix store
        case builderRunning, flakeEval, nixSpace
        // preflight macOS: IPSW from Apple, base, space for installation and images
        case ipsw, macBase, macSpace
        case unknown
    }

    public enum Status: String, OpenEnum { case ok, limited, missing, unknown }

    public var id: String { item.rawValue }
    public var item: Item
    public var status: Status
    public var required: Bool
    /// What was found (path, version) or what is missing
    public var detail: String
    /// What to do about it (English; the app shows its own text per item)
    public var hint: String
    /// Command for Terminal that installs it (official installers only)
    public var install: String?
    /// The user can enter a path for it (`doctor.setPath`)
    public var settablePath: Bool
    /// Space checks: free and needed bytes
    public var freeBytes: Int64?
    public var neededBytes: Int64?
    /// macOS base: its guest user, and the recipes' macUser when it differs
    public var baseUser: String?
    public var expectedUser: String?

    enum CodingKeys: String, CodingKey {
        case item, status, required, detail, hint, install, settablePath, freeBytes, neededBytes, baseUser, expectedUser
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        item = try c.value(.item, .unknown)
        status = try c.value(.status, .unknown)
        required = try c.value(.required, item != .git)
        detail = try c.value(.detail, "")
        hint = try c.value(.hint, "")
        install = try c.decodeIfPresent(String.self, forKey: .install)
        settablePath = try c.value(.settablePath, [.nix, .qemu, .flake].contains(item))
        freeBytes = try? c.decodeIfPresent(Int64.self, forKey: .freeBytes)
        neededBytes = try? c.decodeIfPresent(Int64.self, forKey: .neededBytes)
        baseUser = try c.decodeIfPresent(String.self, forKey: .baseUser)
        expectedUser = try c.decodeIfPresent(String.self, forKey: .expectedUser)
    }
}

public struct MacOSRelease: Decodable, Sendable, Equatable {
    public var version: String
    public var build: String
    public var url: String
    public var bytes: Int64?
}

/// `doctor.check`, `doctor.setPath` and the `doctor.changed` event
public struct DoctorReport: Decodable, Sendable, Equatable {
    /// Per guest kind
    public struct Blockers: Decodable, Sendable, Equatable {
        public var nixos: [Prerequisite]
        public var macos: [Prerequisite]

        public init() {
            nixos = []
            macos = []
        }

        public subscript(kind: GuestKind) -> [Prerequisite] { kind == .macos ? macos : kind == .nixos ? nixos : [] }

        enum CodingKeys: String, CodingKey { case nixos, macos }
        /// A list of Prerequisite objects – or of item names, looked up in `all` (api.md leaves it open)
        init(from decoder: Decoder, all: [Prerequisite]) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            func list(_ key: CodingKeys) throws -> [Prerequisite] {
                if let full = try? c.decodeIfPresent([Prerequisite].self, forKey: key) { return full }
                let names = (try? c.decodeIfPresent([String].self, forKey: key)) ?? nil
                return names?.compactMap { n in all.first { $0.item.rawValue == n } } ?? []
            }
            nixos = try list(.nixos)
            macos = try list(.macos)
        }

        public init(from decoder: Decoder) throws { try self.init(from: decoder, all: []) }
    }

    public var prerequisites: [Prerequisite]
    public var nixosPreflight: [Prerequisite]
    public var macosPreflight: [Prerequisite]
    public var latestRelease: MacOSRelease?
    /// What the Doctor reports against building images of a guest kind; empty = building is possible
    public var buildBlockers: Blockers
    /// What blocks starting VMs of a guest kind
    public var startBlockers: Blockers
    /// Items that need attention (the warning sign on the Doctor's icon)
    public var issues: Int
    /// This Mac's nix-darwin system comes from omaflake's image flake – `builder.apply` without `force` keeps a
    /// foreign one (nil: the server does not say – OPEN in api.md: proposed)
    public var builderSystemIsOmaflakes: Bool?

    public init() {
        prerequisites = []
        nixosPreflight = []
        macosPreflight = []
        buildBlockers = Blockers()
        startBlockers = Blockers()
        issues = 0
    }

    public var all: [Prerequisite] { prerequisites + nixosPreflight + macosPreflight }

    enum CodingKeys: String, CodingKey {
        case prerequisites, nixosPreflight, macosPreflight, latestRelease, buildBlockers, startBlockers, issues
        case builderSystemIsOmaflakes
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        prerequisites = try c.value(.prerequisites, [])
        nixosPreflight = try c.value(.nixosPreflight, [])
        macosPreflight = try c.value(.macosPreflight, [])
        latestRelease = try? c.decodeIfPresent(MacOSRelease.self, forKey: .latestRelease)
        let all = prerequisites + nixosPreflight + macosPreflight
        func blockers(_ key: CodingKeys) throws -> Blockers {
            guard c.contains(key), try !c.decodeNil(forKey: key) else { return Blockers() }
            return try Blockers(from: c.superDecoder(forKey: key), all: all)
        }
        buildBlockers = try blockers(.buildBlockers)
        startBlockers = try blockers(.startBlockers)
        issues = try c.value(.issues, 0)
        builderSystemIsOmaflakes = try c.decodeIfPresent(Bool.self, forKey: .builderSystemIsOmaflakes)
    }
}
