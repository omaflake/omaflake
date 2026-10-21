// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// omaflake's portable packages (spec §11.10, plan B108): a VM, an image or recipes as one folder with an extension –
/// the Finder shows it as one file; copying it is a backup, double-clicking it imports it. The data folder keeps VMs
/// and images in exactly this form, so `~/.omaflake/instances/research.omaflake-vm` already is the backup.
public enum Package {
    public enum Kind: String, Codable, Sendable, CaseIterable {
        case vm = "omaflake-vm", image = "omaflake-image", recipe = "omaflake-recipe"
    }

    public static let vm = Kind.vm.rawValue
    public static let image = Kind.image.rawValue
    public static let recipe = Kind.recipe.rawValue

    /// "research.omaflake-vm" → "research" (nil for another kind)
    public static func name(of url: URL, kind: Kind) -> String? {
        let last = url.standardizedFileURL.lastPathComponent
        let suffix = ".\(kind.rawValue)"
        guard last.hasSuffix(suffix), last.count > suffix.count else { return nil }
        return String(last.dropLast(suffix.count))
    }

    /// The kind of a package by its extension
    public static func kind(of url: URL) -> Kind? { Kind(rawValue: url.standardizedFileURL.pathExtension) }

    /// What a package says about itself (`manifest.json`): written on export, informational for the user and for
    /// a newer omaflake (format version). The content is self-describing too (instance.json, image.json, recipe.toml).
    public struct Manifest: Codable, Sendable, Equatable {
        /// manifest.json, schema `manifest` (plan B114). 0 → 1: `format` (always 1, plan B108) gave way to `$schema`
        public static let fileFormat = FileFormat("manifest", file: "manifest.json", steps: [
            { o in o.removeValue(forKey: "format") },
        ])
        public var kind: Kind
        public var name: String
        public var exported: Date
        /// omaflake's version that wrote it
        public var omaflake: String?
        public var guestKind: GuestKind?
        public var recipe: String?
        public var image: String?
        /// NixOS VM: its image is inside the package (`image.omaflake-image`)
        public var containsImage: Bool?

        public init(kind: Kind, name: String, guestKind: GuestKind? = nil, recipe: String? = nil, image: String? = nil,
                    containsImage: Bool? = nil) {
            self.kind = kind
            self.name = name
            exported = Date()
            omaflake = Bundle.main.infoDictionary?["CFBundleShortVersionString"] as? String
            self.guestKind = guestKind
            self.recipe = recipe
            self.image = image
            self.containsImage = containsImage
        }

        public static func read(_ package: URL) -> Manifest? {
            let d = JSONDecoder()
            d.dateDecodingStrategy = .iso8601
            return (try? Data(contentsOf: package.appendingPathComponent("manifest.json")))
                .flatMap { try? fileFormat.decode(Manifest.self, from: $0, decoder: d).value }
        }

        public func write(_ package: URL) throws {
            let e = JSONEncoder()
            e.dateEncodingStrategy = .iso8601
            e.outputFormatting = [.prettyPrinted, .sortedKeys]
            try Self.fileFormat.encode(self, encoder: e).write(to: package.appendingPathComponent("manifest.json"), options: .atomic)
        }
    }

    /// Path of `file` relative to the folder `from` ("../../images/x.omaflake-image/disk.qcow2") – for qcow2 backing
    /// files, which QEMU resolves relative to the overlay's folder, so a package moved with what it points to still works
    public static func relativePath(from: URL, to file: URL) -> String {
        let a = from.resolvingSymlinksInPath().standardizedFileURL.pathComponents
        let b = file.resolvingSymlinksInPath().standardizedFileURL.pathComponents
        var common = 0
        while common < min(a.count, b.count) && a[common] == b[common] { common += 1 }
        let up = Array(repeating: "..", count: a.count - common)
        return (up + b[common...]).joined(separator: "/")
    }

    /// Copies a folder: on the same APFS volume as clones (instant, no extra space), elsewhere as files that stay
    /// sparse (an 80 GB macOS disk takes on the stick what it uses, not 80 GB). Symlinks are copied as links.
    public static func copy(_ from: URL, to: URL) throws {
        try FileManager.default.createDirectory(at: to.deletingLastPathComponent(), withIntermediateDirectories: true)
        try copyItem(from, to)
    }

    /// Folders by hand, their permissions last: copyfile's own recursion creates a read-only folder (a macOS image)
    /// read-only first and then cannot fill it
    static func copyItem(_ from: URL, _ to: URL) throws {
        let fm = FileManager.default
        let attrs = try fm.attributesOfItem(atPath: from.path)
        switch attrs[.type] as? FileAttributeType {
        case .typeDirectory?:
            try fm.createDirectory(at: to, withIntermediateDirectories: false)
            for name in try fm.contentsOfDirectory(atPath: from.path) {
                try copyItem(from.appendingPathComponent(name), to.appendingPathComponent(name))
            }
            if let mode = attrs[.posixPermissions] { try fm.setAttributes([.posixPermissions: mode], ofItemAtPath: to.path) }
        case .typeSymbolicLink?:
            try fm.createSymbolicLink(atPath: to.path, withDestinationPath: fm.destinationOfSymbolicLink(atPath: from.path))
        default:
            let flags = copyfile_flags_t(COPYFILE_ALL | COPYFILE_CLONE | COPYFILE_DATA_SPARSE | COPYFILE_NOFOLLOW_SRC)
            guard copyfile(from.path, to.path, nil, flags) == 0 else {
                throw VMError("Cannot copy \(from.path) to \(to.path): \(String(cString: strerror(errno)))")
            }
        }
    }

    /// Writes into a package that may be read-only (macOS images): opened for the write and closed again
    static func writable<T>(_ dir: URL, _ body: () throws -> T) throws -> T {
        let fm = FileManager.default
        let readOnly = !fm.isWritableFile(atPath: dir.path)
        if readOnly { try fm.setAttributes([.posixPermissions: 0o755], ofItemAtPath: dir.path) }
        defer { if readOnly { try? fm.setAttributes([.posixPermissions: 0o555], ofItemAtPath: dir.path) } }
        return try body()
    }
}
