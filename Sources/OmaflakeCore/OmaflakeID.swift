// SPDX-License-Identifier: AGPL-3.0-or-later
import CryptoKit
import Foundation

/// omaflake IDs (plan B110): one readable form for VMs, images and recipes – `<kind>:<identifier>:<words>`:
///   recipe:omaflake/nixos/minimal:velvet-tiger-orbit   identifier author/kind/name, words from the content hash
///   image:omaflake/nixos/minimal:cedar-mango-lift      identifier the recipe's author/kind/name, words from the store path
///   vm:research:quiet-otter-lamp                       identifier the VM's name, words from its identity (random, new
///                                                      for a copy under another name)
/// The words are the first 132 bits of a hash as twelve words of BIP-39's English list (plan B111) – unique worldwide,
/// like a UUID. They are shown only as long as needed (`shortest`): one word, two as soon as one word is not enough
/// for two objects of the same kind and identifier, and so on, the same length everywhere for a kind. Files and Nix
/// attributes use the file form `<identifier with dots>@<words>` (`RecipeID`, image package names); a VM's package
/// keeps its name.
public enum OmaflakeID {
    public enum Kind: String, Sendable, CaseIterable { case vm, image, recipe }

    static let list: [String] = wordText.split(whereSeparator: { $0 == " " || $0 == "\n" }).map(String.init)
    static let known = Set(list)

    /// Words in an ID: 12 × 11 = 132 bits
    public static let length = 12

    /// `count` words (11 bits each) for the first bits of `digest`
    public static func words(_ digest: some Sequence<UInt8>, count: Int = length) -> String {
        let bytes = Array(digest)
        guard bytes.count * 8 >= count * 11 else { return "" }
        func bit(_ i: Int) -> Int { Int(bytes[i / 8] >> (7 - UInt8(i % 8))) & 1 }
        return (0..<count).map { w in list[(0..<11).reduce(0) { $0 << 1 | bit(w * 11 + $1) }] }.joined(separator: "-")
    }

    /// Words of a hex digest ("3fa2c1d0…")
    public static func words(hex: String, count: Int = length) -> String {
        var bytes: [UInt8] = []
        var i = hex.startIndex
        while i < hex.endIndex, let j = hex.index(i, offsetBy: 2, limitedBy: hex.endIndex), let b = UInt8(hex[i..<j], radix: 16) {
            bytes.append(b)
            i = j
        }
        return words(bytes, count: count)
    }

    /// Words of SHA-256 of a string (e.g. a store path)
    public static func words(of string: String, count: Int = length) -> String {
        words(Array(SHA256.hash(data: Data(string.utf8))), count: count)
    }

    /// New random words – a VM's identity
    public static func randomWords() -> String { words((0..<17).map { _ in UInt8.random(in: 0...255) }) }

    /// Words of the list joined by "-": all of an ID's (`full`) or the start of them
    public static func isWords(_ s: String, full: Bool = false) -> Bool {
        let p = s.split(separator: "-", omittingEmptySubsequences: false).map(String.init)
        return (full ? p.count == length : (1...length).contains(p.count)) && p.allSatisfy { known.contains($0) }
    }

    // MARK: Shown as short as possible

    /// The first `count` words
    public static func short(_ words: String, _ count: Int) -> String {
        words.split(separator: "-").prefix(max(1, count)).joined(separator: "-")
    }

    /// How many words tell apart the objects of each group (one group: the same kind and identifier) – at least one
    public static func shortest(_ groups: some Sequence<[String]>) -> Int {
        let groups = Array(groups)
        for n in 1..<length {
            if groups.allSatisfy({ g in Set(g.map { short($0, n) }).count == Set(g).count }) { return n }
        }
        return length
    }

    /// How many words each kind shows (plan B111): the same for every object of a kind
    public struct Lengths: Sendable, Equatable {
        public var recipe = 1, image = 1, vm = 1
        public init() {}

        /// From the recipe IDs, image names and VMs there are
        public init(recipes: [String], images: [String], vms: [InstanceConfig]) {
            func groups(_ ids: [String]) -> [[String]] {
                Dictionary(grouping: ids, by: RecipeID.base).values.map { $0.compactMap(RecipeID.words) }
            }
            recipe = shortest(groups(recipes))
            image = shortest(groups(images))
            vm = shortest(Dictionary(grouping: vms, by: \.name).values.map { $0.compactMap(\.identity) })
        }

        public static let full: Lengths = { var l = Lengths(); l.recipe = length; l.image = length; l.vm = length; return l }()
    }

    public static func make(_ kind: Kind, identifier: String, words: String) -> String { "\(kind.rawValue):\(identifier):\(words)" }

    /// (kind, identifier, words) of an ID in its readable form; nil otherwise
    public static func parse(_ id: String) -> (kind: Kind, identifier: String, words: String)? {
        let p = id.split(separator: ":", omittingEmptySubsequences: false).map(String.init)
        guard p.count == 3, let kind = Kind(rawValue: p[0]), !p[1].isEmpty, isWords(p[2]) else { return nil }
        return (kind, p[1], p[2])
    }

    /// "omaflake.nixos.minimal@velvet-tiger-orbit-…" → "recipe:omaflake/nixos/minimal:velvet" (as many words as `words`)
    public static func recipe(_ recipeID: String, words n: Int = length) -> String {
        make(.recipe, identifier: RecipeID.qualified(recipeID), words: RecipeID.words(recipeID).map { short($0, n) } ?? "?")
    }

    public static func image(_ image: ImageStore.Image, words n: Int = length) -> String {
        make(.image, identifier: RecipeID.qualified(image.recipe), words: RecipeID.words(image.name).map { short($0, n) } ?? "?")
    }

    public static func vm(_ c: InstanceConfig, words n: Int = length) -> String {
        make(.vm, identifier: c.name, words: c.identity.map { short($0, n) } ?? "?")
    }
}

extension OmaflakeID.Lengths {
    /// The lengths for what this data folder has: every recipe version, image and VM
    public static func current(_ paths: Paths) -> Self {
        let settings = Settings.load(paths)
        let recipes = RecipeLibrary(paths: paths).versions(settings).values.flatMap { $0 }.map(\.id)
        return Self(recipes: recipes, images: ImageStore(paths: paths).list().map(\.name),
                    vms: InstanceStore(paths: paths, qemuImg: "qemu-img").list())
    }
}
