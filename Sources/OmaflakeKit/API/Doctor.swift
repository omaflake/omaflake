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
    /// What the check is about, in English (the app shows its own text per item; this one for items it does not know)
    public var title: String
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
    /// Something every feature needs is missing
    public var requirementsMissing: Bool
    /// A macOS base exists (needed to build macOS images)
    public var hasMacBase: Bool
    /// The guest user of the macOS base
    public var macBaseUser: String?
    /// This Mac's nix-darwin system comes from omaflake's image flake – `builder.apply` without `force` refuses to
    /// replace one that does not
    public var builderSystemIsOmaflakes: Bool
    /// When the checks ran; nil only before the first answer
    public var checked: Date?

    public init() {
        prerequisites = []
        nixosPreflight = []
        macosPreflight = []
        buildBlockers = Blockers()
        startBlockers = Blockers()
        issues = 0
        requirementsMissing = false
        hasMacBase = false
        builderSystemIsOmaflakes = true
    }

    public var all: [Prerequisite] { prerequisites + nixosPreflight + macosPreflight }

    enum CodingKeys: String, CodingKey {
        case prerequisites, nixosPreflight, macosPreflight, latestRelease, buildBlockers, startBlockers, issues
        case requirementsMissing, hasMacBase, macBaseUser, builderSystemIsOmaflakes, checked
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        prerequisites = try c.decode([Prerequisite].self, forKey: .prerequisites)
        nixosPreflight = try c.decode([Prerequisite].self, forKey: .nixosPreflight)
        macosPreflight = try c.decode([Prerequisite].self, forKey: .macosPreflight)
        latestRelease = try c.decodeIfPresent(MacOSRelease.self, forKey: .latestRelease)
        buildBlockers = try c.decode(Blockers.self, forKey: .buildBlockers)
        startBlockers = try c.decode(Blockers.self, forKey: .startBlockers)
        issues = try c.decode(Int.self, forKey: .issues)
        requirementsMissing = try c.decode(Bool.self, forKey: .requirementsMissing)
        hasMacBase = try c.decode(Bool.self, forKey: .hasMacBase)
        macBaseUser = try c.decodeIfPresent(String.self, forKey: .macBaseUser)
        builderSystemIsOmaflakes = try c.decode(Bool.self, forKey: .builderSystemIsOmaflakes)
        checked = try c.decode(Date.self, forKey: .checked)
    }
}
