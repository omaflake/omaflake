// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

/// What the user's Mac brings (spec §11.2): detected without elevated rights, shared by `omaf doctor` and the app's
/// Requirements page. Nothing here installs anything – `install` is a command the user runs in Terminal.
public struct Prerequisite: Sendable, Identifiable, Equatable {
    public enum Item: String, Sendable, CaseIterable {
        // what the Mac brings
        case host, nix, flakes, nixDarwin, linuxBuilder, qemu, flake, sshKey, git
        // preflight NixOS: builder VM answering, recipes evaluating, space in the Nix store
        case builderRunning, flakeEval, nixSpace
        // preflight macOS: IPSW from Apple, base, space for installation and images
        case ipsw, macBase, macSpace
    }

    public enum Status: String, Sendable {
        case ok, limited, missing
    }

    public let item: Item
    public var id: String { item.rawValue }
    public let status: Status
    /// What was found (path, version) or what is missing
    public let detail: String
    /// What to do about it (empty when ok)
    public let hint: String
    /// Command for Terminal that installs it (official installers only), nil if there is none
    public let install: String?
    /// Space checks: free and needed bytes (for a localized display; `detail` has them in English)
    public var freeBytes: Int64?
    public var neededBytes: Int64?
    /// macOS base: its guest user, and the recipes' macUser when it differs
    public var baseUser: String?
    public var expectedUser: String?
    /// The user can enter a path for it (`Settings.nix`, `Settings.qemu`, `Settings.flake`)
    public var settablePath: Bool { [.nix, .qemu, .flake].contains(item) }

    public init(item: Item, status: Status, detail: String, hint: String, install: String?) {
        self.item = item
        self.status = status
        self.detail = detail
        self.hint = hint
        self.install = install
    }

    /// Needed for every feature; `limited` items still allow some
    public var required: Bool { item != .git }
}

public struct Prerequisites: Sendable {
    public let paths: Paths

    public init(paths: Paths) { self.paths = paths }

    // MARK: Official install commands (typed into Terminal by the user's click)

    /// The official installer of the NixOS community (also what the app runs through the administrator dialog)
    public static let installNix = "sudo sh -c '" + BuilderMaintenance.installNixCommand + "'"
    /// nix-darwin's documented flake setup in /etc/nix-darwin (README of nix-darwin)
    /// nix-darwin with omaflake's Linux builder host, in two stages (`omaf builder setup`; the app's "Set Up …")
    public static let installNixDarwin = "omaf builder setup"
    /// omaflake's patched QEMU from the flake (spec §11.2)
    public static let installQEMU = "nix profile install github:omaflake/omaflake#qemu"
    public static let installSSHKey = "ssh-keygen -t ed25519"
    public static let installGit = "xcode-select --install"

    /// All checks, in the order shown.
    public func check() -> [Prerequisite] {
        let settings = Settings.load(paths)
        let nix = Self.locateNix(settings)
        let config = nix.map { Self.nixConfig($0) } ?? [:]
        return [host(), nixItem(nix), flakes(nix, config), nixDarwin(), linuxBuilder(nix, config),
                qemu(), flake(settings, nix: nix), sshKey(), git()]
    }

    // MARK: Checks

    func host() -> Prerequisite {
        let v = ProcessInfo.processInfo.operatingSystemVersion
        let version = "\(v.majorVersion).\(v.minorVersion)" + (v.patchVersion > 0 ? ".\(v.patchVersion)" : "")
        let arm = Self.machine() == "arm64"
        // The app and the CLI need macOS 27 (Package.swift, Info.plist); this check names it for builds run elsewhere
        let ok = arm && v.majorVersion >= 27
        return Prerequisite(item: .host, status: ok ? .ok : .missing,
                            detail: "macOS \(version), \(Self.machine())",
                            hint: ok ? "" : arm ? "omaflake needs macOS 27 or later"
                                : "omaflake needs a Mac with Apple silicon", install: nil)
    }

    func nixItem(_ nix: String?) -> Prerequisite {
        guard let nix else {
            return Prerequisite(item: .nix, status: .missing, detail: "not found",
                                hint: "Install Nix, or enter the path to the nix binary", install: Self.installNix)
        }
        let version = (try? Shell.run(nix, ["--version"]))?.stdout.trimmingCharacters(in: .whitespacesAndNewlines) ?? "?"
        return Prerequisite(item: .nix, status: .ok, detail: "\(version) – \(nix)", hint: "", install: nil)
    }

    func flakes(_ nix: String?, _ config: [String: String]) -> Prerequisite {
        guard nix != nil else {
            return Prerequisite(item: .flakes, status: .missing, detail: "needs Nix", hint: "", install: nil)
        }
        let ok = Self.flakesEnabled(config)
        return Prerequisite(item: .flakes, status: ok ? .ok : .missing,
                            detail: ok ? "experimental-features: \(config["experimental-features"] ?? "")" : "flakes not enabled",
                            hint: ok ? "" : "Add \"nix-command flakes\" to experimental-features (nix-darwin: nix.settings.experimental-features)",
                            install: nil)
    }

    func nixDarwin() -> Prerequisite {
        let rebuild = "/run/current-system/sw/bin/darwin-rebuild"
        let ok = FileManager.default.isExecutableFile(atPath: rebuild)
        return Prerequisite(item: .nixDarwin, status: ok ? .ok : .missing, detail: ok ? rebuild : "not found",
                            hint: ok ? "" : "Set up nix-darwin (it provides the Linux builder for NixOS images)",
                            install: ok ? nil : Self.installNixDarwin)
    }

    func linuxBuilder(_ nix: String?, _ config: [String: String]) -> Prerequisite {
        let machines = Self.builderLines(config["builders"] ?? "")
        let linux = machines.first { Self.isLinuxBuilder($0) }
        return Prerequisite(item: .linuxBuilder, status: linux != nil ? .ok : .missing,
                            detail: linux.map { String($0.split(separator: " ").first ?? "") } ?? "no aarch64-linux builder",
                            hint: linux != nil ? "" : "Enable nix.linux-builder in your nix-darwin configuration (docs/linux-builder.md)",
                            install: nil)
    }

    func qemu() -> Prerequisite {
        guard let (rt, _) = try? QEMURuntime.locate(paths) else {
            return Prerequisite(item: .qemu, status: .missing, detail: "not found",
                                hint: "Needed for NixOS guests: install omaflake's QEMU, or enter a path", install: Self.installQEMU)
        }
        let gl = rt.capabilities().gl
        return Prerequisite(item: .qemu, status: gl ? .ok : .limited, detail: "\(rt.qemu)\(gl ? " (3D)" : " (no 3D)")",
                            hint: gl ? "" : "Works headless and in 2D – without 3D and without automatic window resolution; omaflake's QEMU adds both", install: gl ? nil : Self.installQEMU)
    }

    func flake(_ settings: Settings, nix: String?) -> Prerequisite {
        guard let flake = try? settings.flakePath() else {
            return Prerequisite(item: .flake, status: .missing, detail: "not set",
                                hint: "Choose the flake with your recipes (folder or flake URL)", install: nil)
        }
        let isFolder = flake.hasPrefix("/")
        let exists = !isFolder || FileManager.default.fileExists(atPath: (flake as NSString).appendingPathComponent("flake.nix"))
        return Prerequisite(item: .flake, status: exists ? .ok : .missing, detail: flake,
                            hint: exists ? "" : "No flake.nix in this folder", install: nil)
    }

    func sshKey() -> Prerequisite {
        let keys = MacBaseInstaller.defaultPublicKeys()
        let files = ((try? FileManager.default.contentsOfDirectory(atPath: NSHomeDirectory() + "/.ssh")) ?? [])
            .filter { $0.hasSuffix(".pub") }.sorted().map { "~/.ssh/\($0)" }
        return Prerequisite(item: .sshKey, status: keys.isEmpty ? .missing : .ok,
                            detail: keys.isEmpty ? "no public key in ~/.ssh" : files.joined(separator: ", "),
                            hint: keys.isEmpty ? "Create a key (used for omaf ssh, image builds and macOS bases)" : "",
                            install: keys.isEmpty ? Self.installSSHKey : nil)
    }

    func git() -> Prerequisite {
        let git = Self.locateGit()
        return Prerequisite(item: .git, status: git != nil ? .ok : .limited, detail: git ?? "not found",
                            hint: git != nil ? "" : "Optional: image names then show \"nogit\" instead of the flake's revision",
                            install: git != nil ? nil : Self.installGit)
    }

    // MARK: Preflight (before building images)

    /// Space the Nix store should have for building NixOS images
    public static let nixSpaceNeeded: Int64 = 20_000_000_000
    /// Space a macOS installation and one image need besides the IPSW
    public static let macSpaceNeeded: Int64 = 30_000_000_000

    public func nixosPreflight() -> [Prerequisite] {
        let settings = Settings.load(paths)
        return [builderRunning(), flakeEval(settings), space(.nixSpace, at: URL(fileURLWithPath: "/nix/store"), needed: Self.nixSpaceNeeded)]
    }

    /// `latest`: what Apple offers this Mac (`MacOSRelease.latest`), nil when unknown (offline).
    public func macosPreflight(latest: MacOSRelease?) -> [Prerequisite] {
        let ipswItem = ipsw(latest)
        let missingIPSW = ipswItem.status != .ok ? (latest?.bytes ?? 27_000_000_000) : 0
        return [ipswItem, macBase(), space(.macSpace, at: paths.root, needed: Self.macSpaceNeeded + missingIPSW)]
    }

    public func builderRunning() -> Prerequisite {
        let target = Self.builderSSHTarget()
        let up = target.map { Ports.sshBanner(host: $0.host == "localhost" ? "127.0.0.1" : $0.host, port: $0.port) != nil } ?? false
        return Prerequisite(item: .builderRunning, status: up ? .ok : .missing,
                            detail: target.map { "\($0.host):\($0.port)" } ?? "no linux-builder in /etc/ssh/ssh_config.d",
                            hint: up ? "" : "The Linux builder VM does not answer – it starts with nix-darwin (launchd org.nixos.linux-builder)",
                            install: nil)
    }

    func flakeEval(_ settings: Settings) -> Prerequisite {
        guard let flake = try? settings.flakePath() else {
            return Prerequisite(item: .flakeEval, status: .missing, detail: "no flake set", hint: "", install: nil)
        }
        do {
            let recipes = try ImageStore(paths: paths).recipeDefaults(flake: flake)
            let nixos = recipes.filter { $0.value.kind == .nixos }.map { RecipeID.displayName($0.key, $0.value) }.sorted()
            return Prerequisite(item: .flakeEval, status: .ok, detail: "\(recipes.count) recipes (\(nixos.joined(separator: ", ")))",
                                hint: "", install: nil)
        } catch {
            return Prerequisite(item: .flakeEval, status: .missing, detail: String("\(error)".prefix(300)),
                                hint: "The flake does not evaluate – see the message", install: nil)
        }
    }

    func ipsw(_ latest: MacOSRelease?) -> Prerequisite {
        let fm = FileManager.default
        let local = ((try? fm.contentsOfDirectory(atPath: paths.ipsw.path)) ?? []).filter { $0.hasSuffix(".ipsw") }.sorted()
        guard let latest else {
            return Prerequisite(item: .ipsw, status: local.isEmpty ? .missing : .ok,
                                detail: local.last ?? "Apple not reachable", hint: "", install: nil)
        }
        let file = IPSWDownloader.file(latest, in: paths.ipsw)
        if fm.fileExists(atPath: file.path) {
            return Prerequisite(item: .ipsw, status: .ok, detail: "macOS \(latest.version) (\(latest.build)) – \(file.lastPathComponent)",
                                hint: "", install: nil)
        }
        let partial = fm.fileExists(atPath: file.appendingPathExtension("resume").path)
        return Prerequisite(item: .ipsw, status: partial ? .limited : .missing,
                            detail: "macOS \(latest.version) (\(latest.build)), \(latest.bytes.map(gigabytes) ?? "?") from Apple",
                            hint: partial ? "Partly downloaded – continues where it stopped" : "Download it from Apple", install: nil)
    }

    func macBase() -> Prerequisite {
        let base = MacImageBuilder.newestBase(paths)
        let unfinished = ((try? FileManager.default.contentsOfDirectory(atPath: paths.bases.path)) ?? [])
            .filter { $0.hasPrefix("mac-base-") }.sorted()
            .first { !MacBaseInstaller.isReady(paths.bases.appendingPathComponent($0)) }
        if base == nil, let unfinished {
            return Prerequisite(item: .macBase, status: .limited, detail: "\(unfinished) (unfinished)",
                                hint: "Unfinished – Create Base continues it (same password as before)", install: nil)
        }
        if let base, let user = MacBaseInstaller.user(of: base, logs: paths.logs) {
            // The macOS recipes' guest user (macUser) must be the base's: builds log in as it, nix-darwin configures it
            let expected = Set(((try? Settings.load(paths).flakePath()).flatMap { try? ImageStore(paths: paths).recipeDefaults(flake: $0) } ?? [:])
                .values.filter { $0.kind == .macos }.compactMap(\.user))
            if let other = expected.first(where: { $0 != user }) {
                var p = Prerequisite(item: .macBase, status: .limited, detail: "\(base.lastPathComponent) (\(user))",
                                     hint: "The base's user \(user) differs from the recipes' macUser \(other) – make them match", install: nil)
                p.baseUser = user
                p.expectedUser = other
                return p
            }
            var p = Prerequisite(item: .macBase, status: .ok, detail: "\(base.lastPathComponent) (\(user))", hint: "", install: nil)
            p.baseUser = user
            return p
        }
        return Prerequisite(item: .macBase, status: base != nil ? .ok : .missing, detail: base?.lastPathComponent ?? "none yet",
                            hint: base != nil ? "" : "Create it from the IPSW (about 5 minutes, no manual steps)", install: nil)
    }

    func space(_ item: Prerequisite.Item, at url: URL, needed: Int64) -> Prerequisite {
        let free = freeSpace(at: url) ?? 0
        var p = Prerequisite(item: item, status: free >= needed ? .ok : .missing,
                             detail: "\(gigabytes(free)) free, about \(gigabytes(needed)) needed",
                             hint: free >= needed ? "" : "Free up space first", install: nil)
        p.freeBytes = free
        p.neededBytes = needed
        return p
    }

    /// Host and port of the Linux builder from the SSH configuration nix-darwin writes (`Host linux-builder`);
    /// its key is root's, so only the SSH banner is checked.
    static func builderSSHTarget(configs: [String]? = nil) -> (host: String, port: Int)? {
        let texts = configs ?? {
            let dir = "/etc/ssh/ssh_config.d"
            let files = ((try? FileManager.default.contentsOfDirectory(atPath: dir)) ?? []).sorted().map { "\(dir)/\($0)" }
            return (files + ["/etc/ssh/ssh_config"]).compactMap { try? String(contentsOfFile: $0, encoding: .utf8) }
        }()
        for text in texts {
            var inBlock = false, host: String?, port = 22
            for raw in text.split(separator: "\n") {
                let parts = raw.trimmingCharacters(in: .whitespaces).split(separator: " ", maxSplits: 1).map(String.init)
                guard parts.count == 2 else { continue }
                switch parts[0].lowercased() {
                case "host":
                    if inBlock { return (host ?? "linux-builder", port) }
                    inBlock = parts[1].split(separator: " ").contains("linux-builder")
                case "hostname" where inBlock: host = parts[1]
                case "port" where inBlock: port = Int(parts[1]) ?? 22
                default: break
                }
            }
            if inBlock { return (host ?? "linux-builder", port) }
        }
        return nil
    }

    // MARK: Detection helpers

    /// `nix`: `Settings.nix`, then PATH and the usual Nix locations (Shell.which).
    public static func locateNix(_ settings: Settings) -> String? {
        if let custom = settings.nix.map({ ($0 as NSString).expandingTildeInPath }) {
            return FileManager.default.isExecutableFile(atPath: custom) ? custom : nil
        }
        return try? Shell.which("nix")
    }

    /// git that does not trigger the Command Line Tools install dialog: /usr/bin/git is only a shim that asks
    /// to install them when they are missing, so it counts only when `xcode-select -p` finds developer tools.
    public static func locateGit() -> String? {
        let dirs = Shell.searchDirectories().filter { $0 != "/usr/bin" }
        if let git = dirs.map({ ($0 as NSString).appendingPathComponent("git") })
            .first(where: { FileManager.default.isExecutableFile(atPath: $0) }) { return git }
        guard (try? Shell.run("/usr/bin/xcode-select", ["-p"]))?.status == 0 else { return nil }
        return "/usr/bin/git"
    }

    /// `nix config show` as key → value
    static func nixConfig(_ nix: String) -> [String: String] {
        parseConfig((try? Shell.run(nix, ["config", "show"]))?.stdout ?? "")
    }

    static func parseConfig(_ text: String) -> [String: String] {
        var result: [String: String] = [:]
        for line in text.split(separator: "\n") {
            guard let eq = line.firstIndex(of: "=") else { continue }
            result[line[..<eq].trimmingCharacters(in: .whitespaces)] = line[line.index(after: eq)...].trimmingCharacters(in: .whitespaces)
        }
        return result
    }

    static func flakesEnabled(_ config: [String: String]) -> Bool {
        (config["experimental-features"] ?? "").split(separator: " ").contains("flakes")
    }

    /// `builders` is a list separated by ";" or newlines, or "@<file>" with one machine per line.
    static func builderLines(_ builders: String, read: (String) -> String? = { try? String(contentsOfFile: $0, encoding: .utf8) }) -> [String] {
        let text = builders.hasPrefix("@") ? (read(String(builders.dropFirst())) ?? "") : builders
        return text.split(whereSeparator: { $0 == "\n" || $0 == ";" })
            .map { $0.trimmingCharacters(in: .whitespaces) }.filter { !$0.isEmpty && !$0.hasPrefix("#") }
    }

    /// Machine line: `<uri> <systems, comma-separated> …`
    static func isLinuxBuilder(_ line: String) -> Bool {
        let fields = line.split(separator: " ")
        return fields.count > 1 && fields[1].split(separator: ",").contains("aarch64-linux")
    }

    static func machine() -> String {
        var u = utsname()
        uname(&u)
        return withUnsafeBytes(of: &u.machine) { String(decoding: $0.prefix { $0 != 0 }, as: UTF8.self) }
    }
}
