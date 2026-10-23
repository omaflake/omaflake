// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Any JSON value – for the parts of the API whose shape is open (a job's `result`, `settings.set` with `null` for
/// "back to the default").
public enum JSONValue: Codable, Sendable, Equatable, Hashable {
    case null
    case bool(Bool)
    case number(Double)
    case string(String)
    case array([JSONValue])
    case object([String: JSONValue])

    public init(from decoder: Decoder) throws {
        let c = try decoder.singleValueContainer()
        if c.decodeNil() { self = .null }
        else if let b = try? c.decode(Bool.self) { self = .bool(b) }
        else if let n = try? c.decode(Double.self) { self = .number(n) }
        else if let s = try? c.decode(String.self) { self = .string(s) }
        else if let a = try? c.decode([JSONValue].self) { self = .array(a) }
        else { self = .object(try c.decode([String: JSONValue].self)) }
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        switch self {
        case .null: try c.encodeNil()
        case .bool(let b): try c.encode(b)
        case .number(let n):
            // Whole numbers as integers (ports, sizes)
            if n.rounded() == n, abs(n) < 1e15 { try c.encode(Int64(n)) } else { try c.encode(n) }
        case .string(let s): try c.encode(s)
        case .array(let a): try c.encode(a)
        case .object(let o): try c.encode(o)
        }
    }

    public subscript(key: String) -> JSONValue? {
        if case .object(let o) = self { return o[key] }
        return nil
    }

    public var string: String? { if case .string(let s) = self { return s }; return nil }
    public var bool: Bool? { if case .bool(let b) = self { return b }; return nil }
    public var int: Int? { if case .number(let n) = self { return Int(n) }; return nil }
    public var array: [JSONValue]? { if case .array(let a) = self { return a }; return nil }
    public var strings: [String] { array?.compactMap(\.string) ?? [] }

    /// A value from an Encodable (e.g. a struct to send inside an object)
    public init(_ value: some Encodable) throws {
        self = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(value))
    }
}

extension JSONDecoder {
    /// The API's dates: ISO 8601 in UTC, whole seconds or with a fraction
    public static var api: JSONDecoder {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .custom { decoder in
            let text = try decoder.singleValueContainer().decode(String.self)
            if let date = APIDate.parse(text) { return date }
            throw DecodingError.dataCorrupted(.init(codingPath: decoder.codingPath, debugDescription: "Not an ISO 8601 date: \(text)"))
        }
        return d
    }
}

extension JSONEncoder {
    /// Requests: one line (NDJSON), no escaped slashes (paths), dates as ISO 8601
    public static var api: JSONEncoder {
        let e = JSONEncoder()
        e.outputFormatting = [.withoutEscapingSlashes, .sortedKeys]
        e.dateEncodingStrategy = .iso8601
        return e
    }
}

enum APIDate {
    static func parse(_ text: String) -> Date? {
        let plain = ISO8601DateFormatter()
        if let d = plain.date(from: text) { return d }
        let fractional = ISO8601DateFormatter()
        fractional.formatOptions = [.withInternetDateTime, .withFractionalSeconds]
        return fractional.date(from: text)
    }
}

/// An enum of the API whose values may grow (api.md §2: new enum values are shown as "unknown")
public protocol OpenEnum: RawRepresentable, Codable, Hashable, Sendable where RawValue == String {
    static var unknown: Self { get }
}

extension OpenEnum {
    public init(from decoder: Decoder) throws {
        let raw = try decoder.singleValueContainer().decode(String.self)
        self = Self(rawValue: raw) ?? .unknown
    }

    public func encode(to encoder: Encoder) throws {
        var c = encoder.singleValueContainer()
        try c.encode(rawValue)
    }
}
