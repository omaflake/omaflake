// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// The JSON files omaflake writes name their JSON Schema (plan B114): `"$schema":
/// "https://omaflake.dev/schema/<type>/v<n>.json"` – the type says what the file is, n its version, one count per type
/// (a change to instance.json needs no step for image.json). The schemas are in docs/schema/ and published on
/// omaflake.dev; a version, once published, stays there. An older file is brought up to date when it is read, step by
/// step (each step turns version n into n + 1), and the caller writes it back; a newer one (from a newer omaflake) is
/// refused instead of misread. A file without `$schema` is version 0 (written before B114).
public struct FileFormat: Sendable {
    public typealias Step = @Sendable (inout [String: Any]) throws -> Void

    /// Where the schemas live (docs/schema/ in this repository)
    public static let base = "https://omaflake.dev/schema/"

    /// The schema's type: "image", "instance", "settings", …
    public let type: String
    /// What the file is, for messages ("image.json")
    public let file: String
    /// steps[n] turns version n into n + 1
    let steps: [Step]

    /// The version this omaflake writes
    public var current: Int { steps.count }

    /// The `$schema` this omaflake writes
    public var schema: String { Self.schema(type: type, version: current) }

    public init(_ type: String, file: String, steps: [Step]) {
        self.type = type
        self.file = file
        self.steps = steps
    }

    public static func schema(type: String, version: Int) -> String { "\(base)\(type)/v\(version).json" }

    /// (type, version) of a schema URL of omaflake's, nil for anything else
    public static func parse(_ schema: String) -> (type: String, version: Int)? {
        guard schema.hasPrefix(base), schema.hasSuffix(".json") else { return nil }
        let parts = schema.dropFirst(base.count).dropLast(".json".count).split(separator: "/")
        guard parts.count == 2, parts[1].first == "v", let n = Int(parts[1].dropFirst()), n >= 1 else { return nil }
        return (String(parts[0]), n)
    }

    /// The version a file says it has: 0 without `$schema`; an error for another type or a foreign schema
    func version(of object: [String: Any]) throws -> Int {
        guard let value = object["$schema"] else { return 0 }
        guard let schema = value as? String, let (type, version) = Self.parse(schema) else {
            throw VMError("\(file): unknown $schema \(value)")
        }
        guard type == self.type else { throw VMError("\(file) is a \(type) file, not \(self.type)") }
        guard version <= current else {
            throw VMError("\(file) has version \(version) – this omaflake reads up to \(current); update omaflake")
        }
        return version
    }

    /// `data` at the current version; `migrated` when it was older
    public func upgrade(_ data: Data) throws -> (data: Data, migrated: Bool) {
        guard var object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else {
            throw VMError("\(file) is not a JSON object")
        }
        let version = try version(of: object)
        guard version < current else { return (data, false) }
        for n in version..<current {
            try steps[n](&object)
        }
        object["$schema"] = schema
        return (try JSONSerialization.data(withJSONObject: object, options: [.sortedKeys]), true)
    }

    /// `data` as `T` at the current version; `migrated` tells the caller to write it back
    public func decode<T: Decodable>(_ type: T.Type, from data: Data,
                                     decoder: JSONDecoder = JSONDecoder()) throws -> (value: T, migrated: Bool) {
        let (upgraded, migrated) = try upgrade(data)
        do {
            return (try decoder.decode(T.self, from: upgraded), migrated)
        } catch {
            throw VMError("\(file) cannot be read: \(error.localizedDescription)")
        }
    }

    /// `value` as this file, with its `$schema` (first, as keys are sorted)
    public func encode<T: Encodable>(_ value: T, encoder: JSONEncoder) throws -> Data {
        guard var object = try JSONSerialization.jsonObject(with: encoder.encode(value)) as? [String: Any] else {
            throw VMError("\(file): not a JSON object")
        }
        object["$schema"] = schema
        return try JSONSerialization.data(withJSONObject: object, options: [.prettyPrinted, .sortedKeys, .withoutEscapingSlashes])
    }

    /// For steps: a key renamed (e.g. "flavor" → "recipe", plan B92); a file that has the new one already keeps it
    public static func renameKey(_ old: String, to new: String, in object: inout [String: Any]) {
        guard let value = object.removeValue(forKey: old), object[new] == nil else { return }
        object[new] = value
    }

    /// Refuses to overwrite a file of a newer version (a newer omaflake wrote it; this one would lose what it added)
    public func checkWritable(_ url: URL) throws {
        guard let data = try? Data(contentsOf: url),
              let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any] else { return }
        _ = try version(of: object)
    }
}
