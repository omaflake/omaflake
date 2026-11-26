// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Keys for typing into a guest's window (plan B184: macOS Recovery has no SSH – what must be typed there, the
/// runner types). A key is a position on the keyboard (macOS virtual key codes), so a text arrives as written only
/// when it is typed for the layout the guest reads: `Layout.us` (a base's Recovery) or `Layout.de` (seen
/// 2026-11-26: the Recovery of a German image reads the German keyboard – "y" on the US position gives "z").
/// Characters a layout does not have cannot be typed.
public enum GuestKeys {
    /// The keyboard layout the guest reads
    public enum Layout: String, Sendable {
        case us, de
        /// From a layout's name as macOS shows it in the input menu ("Deutsch", "German", "ABC", "U.S.")
        public static func named(_ name: String) -> Layout? {
            let n = name.lowercased()
            if n.contains("deutsch") || n.contains("german") { return .de }
            if n == "abc" || n.contains("u.s") || n.contains("us") || n.contains("english") { return .us }
            return nil
        }
    }

    public struct Stroke: Equatable, Sendable {
        public let keyCode: UInt16
        public let shift: Bool
        public var command = false
        public var option = false
        public var control = false
        public init(keyCode: UInt16, shift: Bool = false, command: Bool = false, option: Bool = false,
                    control: Bool = false) {
            self.keyCode = keyCode
            self.shift = shift
            self.command = command
            self.option = option
            self.control = control
        }
    }

    public static let shiftKey: UInt16 = 56
    public static let commandKey: UInt16 = 55
    public static let optionKey: UInt16 = 58
    public static let controlKey: UInt16 = 59

    /// Unshifted characters by key code
    private static let plain: [Character: UInt16] = [
        "a": 0, "s": 1, "d": 2, "f": 3, "h": 4, "g": 5, "z": 6, "x": 7, "c": 8, "v": 9, "b": 11, "q": 12, "w": 13,
        "e": 14, "r": 15, "y": 16, "t": 17, "1": 18, "2": 19, "3": 20, "4": 21, "6": 22, "5": 23, "=": 24, "9": 25,
        "7": 26, "-": 27, "8": 28, "0": 29, "]": 30, "o": 31, "u": 32, "[": 33, "i": 34, "p": 35, "l": 37, "j": 38,
        "'": 39, "k": 40, ";": 41, "\\": 42, ",": 43, "/": 44, "n": 45, "m": 46, ".": 47, "`": 50, " ": 49,
        "\n": 36, "\t": 48,
    ]

    /// Shifted characters and the unshifted one on the same key
    private static let shifted: [Character: Character] = [
        "!": "1", "@": "2", "#": "3", "$": "4", "%": "5", "^": "6", "&": "7", "*": "8", "(": "9", ")": "0",
        "_": "-", "+": "=", "{": "[", "}": "]", "|": "\\", ":": ";", "\"": "'", "<": ",", ">": ".", "?": "/",
        "~": "`",
    ]

    /// Keys with names, for `key`
    private static let named: [String: UInt16] = [
        "return": 36, "enter": 36, "tab": 48, "space": 49, "delete": 51, "backspace": 51, "escape": 53, "esc": 53,
        "left": 123, "right": 124, "down": 125, "up": 126,
    ]

    /// The Mac's "German" layout (id 3), where it differs from the US positions: Y and Z change places, and the
    /// symbols sit elsewhere. Dead keys (´ ` ^ ~) and < > (on a key ANSI keyboards do not have) are left out
    private static let german: [Character: Stroke] = {
        var t: [Character: Stroke] = [:]
        let plainKeys: [(Character, UInt16)] = [
            ("y", 6), ("z", 16), ("ß", 27), ("ü", 33), ("+", 30), ("ö", 41), ("ä", 39), ("#", 42), ("-", 44),
        ]
        for (c, code) in plainKeys { t[c] = Stroke(keyCode: code) }
        let shiftedKeys: [(Character, UInt16)] = [
            ("Y", 6), ("Z", 16), ("!", 18), ("\"", 19), ("§", 20), ("$", 21), ("%", 23), ("&", 22), ("/", 26),
            ("(", 28), (")", 25), ("=", 29), ("?", 27), ("Ü", 33), ("*", 30), ("Ö", 41), ("Ä", 39), ("'", 42),
            (";", 43), (":", 47), ("_", 44),
        ]
        for (c, code) in shiftedKeys { t[c] = Stroke(keyCode: code, shift: true) }
        let optionKeys: [(Character, UInt16, Bool)] = [
            ("@", 37, false), ("€", 14, false), ("[", 23, false), ("]", 22, false), ("{", 28, false),
            ("}", 25, false), ("|", 26, false), ("\\", 26, true),
        ]
        for (c, code, shift) in optionKeys { t[c] = Stroke(keyCode: code, shift: shift, option: true) }
        return t
    }()

    /// The stroke that types `c` with `layout`; nil when that layout has no such character
    public static func stroke(for c: Character, layout: Layout) -> Stroke? {
        switch layout {
        case .us: return stroke(for: c)
        case .de:
            if let s = german[c] { return s }
            // Letters (but Y and Z), digits, space, return, tab, comma and period are where the US layout has them
            if c.isLetter || c.isNumber || [" ", "\n", "\t", ",", "."].contains(c) { return stroke(for: c) }
            return nil
        }
    }

    /// The strokes of a text for `layout`; nil when a character cannot be typed (nothing is typed then)
    public static func strokes(for text: String, layout: Layout) -> [Stroke]? {
        var out: [Stroke] = []
        for c in text {
            guard let s = stroke(for: c, layout: layout) else { return nil }
            out.append(s)
        }
        return out
    }

    /// The stroke that types `c` on a US keyboard; nil when that keyboard has no such character
    public static func stroke(for c: Character) -> Stroke? {
        if let code = plain[c] { return Stroke(keyCode: code) }
        if let base = shifted[c], let code = plain[base] { return Stroke(keyCode: code, shift: true) }
        if c.isLetter, c.isUppercase, let code = plain[Character(c.lowercased())] {
            return Stroke(keyCode: code, shift: true)
        }
        return nil
    }

    /// The strokes of a text; nil when a character cannot be typed (nothing is typed then)
    public static func strokes(for text: String) -> [Stroke]? {
        var out: [Stroke] = []
        for c in text {
            guard let s = stroke(for: c) else { return nil }
            out.append(s)
        }
        return out
    }

    /// "cmd+shift+t", "return", "right": modifiers and one key (a name, or a character of the US keyboard)
    public static func stroke(named name: String) -> Stroke? {
        var parts = name.lowercased().split(separator: "+").map(String.init)
        // "cmd++" would be the plus key: not needed, not parsed
        guard let key = parts.popLast(), !key.isEmpty else { return nil }
        var stroke: Stroke
        if let code = named[key] {
            stroke = Stroke(keyCode: code)
        } else if key.count == 1, let s = Self.stroke(for: Character(key)) {
            stroke = s
        } else {
            return nil
        }
        for modifier in parts {
            switch modifier {
            case "cmd", "command": stroke.command = true
            case "shift": stroke = Stroke(keyCode: stroke.keyCode, shift: true, command: stroke.command,
                                          option: stroke.option, control: stroke.control)
            case "opt", "option", "alt": stroke.option = true
            case "ctrl", "control": stroke.control = true
            default: return nil
            }
        }
        return stroke
    }
}
