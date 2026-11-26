// SPDX-License-Identifier: AGPL-3.0-or-later
import Testing
@testable import OmaflakeKit

/// Typing into a guest's window (plan B184)
@Suite(.timeLimit(.minutes(1)))
struct GuestKeysTests {
    @Test func charactersAreUSKeyPositions() {
        #expect(GuestKeys.stroke(for: "y") == .init(keyCode: 16))
        #expect(GuestKeys.stroke(for: "Y") == .init(keyCode: 16, shift: true))
        #expect(GuestKeys.stroke(for: "z") == .init(keyCode: 6))
        #expect(GuestKeys.stroke(for: " ") == .init(keyCode: 49))
        #expect(GuestKeys.stroke(for: "\n") == .init(keyCode: 36))
        #expect(GuestKeys.stroke(for: "!") == .init(keyCode: 18, shift: true))
        #expect(GuestKeys.stroke(for: "_") == .init(keyCode: 27, shift: true))
        #expect(GuestKeys.stroke(for: "ä") == nil && GuestKeys.stroke(for: "€") == nil)
    }

    @Test func aTextIsTypedWholeOrNotAtAll() throws {
        let strokes = try #require(GuestKeys.strokes(for: "csrutil disable\n"))
        #expect(strokes.count == 16 && strokes.allSatisfy { !$0.shift })
        #expect(strokes.first == .init(keyCode: 8) && strokes.last == .init(keyCode: 36))
        #expect(GuestKeys.strokes(for: "pässword") == nil, "a character the US keyboard does not have")
        #expect(GuestKeys.strokes(for: "") == [])
    }

    /// The Recovery of a German image reads the German keyboard (2026-11-26)
    @Test func theGermanLayoutSwapsYAndZAndMovesTheSymbols() throws {
        #expect(GuestKeys.stroke(for: "y", layout: .de) == .init(keyCode: 6))
        #expect(GuestKeys.stroke(for: "z", layout: .de) == .init(keyCode: 16))
        #expect(GuestKeys.stroke(for: "Y", layout: .de) == .init(keyCode: 6, shift: true))
        #expect(GuestKeys.stroke(for: "-", layout: .de) == .init(keyCode: 44))
        #expect(GuestKeys.stroke(for: "/", layout: .de) == .init(keyCode: 26, shift: true))
        #expect(GuestKeys.stroke(for: "@", layout: .de) == .init(keyCode: 37, option: true))
        #expect(GuestKeys.stroke(for: "ä", layout: .de) == .init(keyCode: 39))
        #expect(GuestKeys.stroke(for: "^", layout: .de) == nil, "a dead key")
        // A command of letters and a space is the same on both
        #expect(GuestKeys.strokes(for: "csrutil disable\n", layout: .de) == GuestKeys.strokes(for: "csrutil disable\n"))
        #expect(GuestKeys.strokes(for: "Pässwort-1!", layout: .us) == nil)
        let password = try #require(GuestKeys.strokes(for: "Pässwort-1!", layout: .de))
        #expect(password.count == 11 && password[1] == .init(keyCode: 39) && password[8] == .init(keyCode: 44))
        #expect(GuestKeys.Layout.named("Deutsch") == .de && GuestKeys.Layout.named("ABC") == .us)
        #expect(GuestKeys.Layout.named("Français") == nil)
    }

    @Test func namedKeysAndModifiers() {
        #expect(GuestKeys.stroke(named: "return") == .init(keyCode: 36))
        #expect(GuestKeys.stroke(named: "Right") == .init(keyCode: 124))
        #expect(GuestKeys.stroke(named: "cmd+shift+t") == .init(keyCode: 17, shift: true, command: true))
        #expect(GuestKeys.stroke(named: "ctrl+c") == .init(keyCode: 8, control: true))
        #expect(GuestKeys.stroke(named: "cmd+q") == .init(keyCode: 12, command: true))
        #expect(GuestKeys.stroke(named: "hyper+t") == nil && GuestKeys.stroke(named: "") == nil)
        #expect(GuestKeys.stroke(named: "f13") == nil)
    }
}
