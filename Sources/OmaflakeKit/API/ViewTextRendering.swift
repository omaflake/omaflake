// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// How a UI turns a text of the views into what it shows (api.md §10.2): the key translated by its table, then the
/// args filled in – numbers into `%lld`, the rest into `%@` (also positional, `%2$@`, as translations may reorder);
/// times and sizes in the UI's language, nested texts translated first, asked params with their value. A text without
/// a key is shown as it is; a translation whose placeholders do not fit the args falls back to the English `text`.
public struct ViewTextRenderer: Sendable {
    /// The translation of a key (the key itself when there is none)
    public var translate: @Sendable (String) -> String
    public var date: @Sendable (Date) -> String
    public var bytes: @Sendable (Int64) -> String

    public init(translate: @escaping @Sendable (String) -> String,
                date: @escaping @Sendable (Date) -> String = { $0.formatted(date: .abbreviated, time: .shortened) },
                bytes: @escaping @Sendable (Int64) -> String = { ByteCountFormatter.string(fromByteCount: $0, countStyle: .file) }) {
        self.translate = translate
        self.date = date
        self.bytes = bytes
    }

    /// English, without a table
    public static let english = ViewTextRenderer(translate: { $0 })

    /// `params`: the values asked for an action (`{"param": "target"}` names them)
    public func render(_ t: ViewText, params: [String: String] = [:]) -> String {
        guard let key = t.key else { return t.text }
        let format = translate(key)
        let values = t.args.map { arg -> String in
            switch arg {
            case .int(let n): String(n)
            case .string(let s): s
            case .date(let d): date(d)
            case .bytes(let b): bytes(b)
            case .param(let p): params[p] ?? params[p.split(separator: ".").last.map(String.init) ?? p] ?? "<\(p)>"
            case .text(let inner): render(inner, params: params)
            }
        }
        let ints = t.args.map { if case .int = $0 { true } else { false } }
        return Self.fill(format, values, ints: ints) ?? (format == key ? t.text : (Self.fill(key, values, ints: ints) ?? t.text))
    }

    /// `format` with its placeholders replaced; nil when they do not fit the values (count or kind)
    static func fill(_ format: String, _ values: [String], ints: [Bool]) -> String? {
        var out = ""
        var next = 0
        var used = Set<Int>()
        var i = format.startIndex
        while i < format.endIndex {
            let ch = format[i]
            guard ch == "%" else { out.append(ch); i = format.index(after: i); continue }
            var j = format.index(after: i)
            guard j < format.endIndex else { return nil }
            if format[j] == "%" { out.append("%"); i = format.index(after: j); continue }
            // Optional position "n$"
            var position: Int?
            var digits = ""
            var k = j
            while k < format.endIndex, format[k].isNumber { digits.append(format[k]); k = format.index(after: k) }
            if !digits.isEmpty, k < format.endIndex, format[k] == "$" {
                position = Int(digits).map { $0 - 1 }
                j = format.index(after: k)
            }
            let rest = format[j...]
            let isInt: Bool
            if rest.hasPrefix("@") {
                isInt = false
                i = format.index(j, offsetBy: 1)
            } else if rest.hasPrefix("lld") {
                isInt = true
                i = format.index(j, offsetBy: 3)
            } else if rest.hasPrefix("d") {
                isInt = true
                i = format.index(j, offsetBy: 1)
            } else {
                return nil
            }
            let index = position ?? next
            next += 1
            guard index >= 0, index < values.count, ints[index] == isInt else { return nil }
            used.insert(index)
            out += values[index]
        }
        return used.count == values.count ? out : nil
    }
}
