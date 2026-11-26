// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import CoreImage
import Foundation
import IOSurface
import OmaflakeKit
import ScreenCaptureKit
import Vision
import Virtualization

/// Hands and eyes for a guest's window (plan B184): keys and clicks into the VM's view, and the text on its screen.
/// For what has no other way in – macOS Recovery has no SSH, so `csrutil disable` must be typed there. Over the
/// control socket (`type`, `key`, `click`, `screen`), which only this user can reach.
///
/// Keys are US key positions (`GuestKeys`). The screen is taken from the view's own layers, else through
/// ScreenCaptureKit (which asks once for screen recording) and recognized on this Mac (Vision); `screen` says how it
/// got its picture and what the other ways gave.
@MainActor
final class Robot {
    private let view: VZVirtualMachineView
    private let window: NSWindow
    /// Between two keys: fast enough for a line of text, slow enough for a guest that is busy
    private let pace: TimeInterval = 0.04

    init(view: VZVirtualMachineView, window: NSWindow) {
        self.view = view
        self.window = window
    }

    // MARK: Keys

    /// How Shift, Command, Option and Control reach the guest (plan B184: in the first tries on a Mac upper case
    /// and ⇧⌘T did not arrive, plain keys did). `device` (the default): modifier events with the left key's device
    /// bit, as a real keyboard sends them; `plain`: without it; `keys`: no modifier events, the flags on the keys
    /// only. Which one a guest takes: OPEN – the core tries them in turn and keeps the one that types "SHIFTOK"
    enum Modifiers: String {
        case device, plain, keys
    }

    /// NX_DEVICEL…KEYMASK: which physical key is down, besides the device-independent flag
    private static let deviceBits: [UInt16: UInt] = [
        GuestKeys.controlKey: 0x01, GuestKeys.shiftKey: 0x02, GuestKeys.commandKey: 0x08, GuestKeys.optionKey: 0x20,
    ]

    /// Types the strokes one after the other, then calls `done`
    func press(_ strokes: [GuestKeys.Stroke], modifiers mode: Modifiers = .device,
               done: @escaping @MainActor () -> Void) {
        window.makeFirstResponder(view)
        var steps: [@MainActor () -> Void] = []
        for s in strokes {
            var flags: NSEvent.ModifierFlags = []
            let modifiers: [(Bool, NSEvent.ModifierFlags, UInt16)] = [
                (s.command, .command, GuestKeys.commandKey), (s.shift, .shift, GuestKeys.shiftKey),
                (s.option, .option, GuestKeys.optionKey), (s.control, .control, GuestKeys.controlKey),
            ]
            for (on, flag, code) in modifiers where on {
                flags.insert(flag)
                if mode == .device { flags.insert(NSEvent.ModifierFlags(rawValue: Self.deviceBits[code] ?? 0)) }
                let now = flags
                if mode != .keys { steps.append { [self] in modifier(code, flags: now) } }
            }
            let held = flags
            steps.append { [self] in key(s.keyCode, flags: held, down: true) }
            steps.append { [self] in key(s.keyCode, flags: held, down: false) }
            for (on, flag, code) in modifiers.reversed() where on {
                flags.remove(flag)
                if mode == .device { flags.remove(NSEvent.ModifierFlags(rawValue: Self.deviceBits[code] ?? 0)) }
                let now = flags
                if mode != .keys { steps.append { [self] in modifier(code, flags: now) } }
            }
        }
        run(steps, done: done)
    }

    private func run(_ steps: [@MainActor () -> Void], done: @escaping @MainActor () -> Void) {
        guard let first = steps.first else { return done() }
        first()
        let rest = Array(steps.dropFirst())
        DispatchQueue.main.asyncAfter(deadline: .now() + pace) { [self] in run(rest, done: done) }
    }

    private func key(_ code: UInt16, flags: NSEvent.ModifierFlags, down: Bool) {
        guard let event = NSEvent.keyEvent(with: down ? .keyDown : .keyUp, location: .zero, modifierFlags: flags,
                                           timestamp: ProcessInfo.processInfo.systemUptime,
                                           windowNumber: window.windowNumber, context: nil, characters: "",
                                           charactersIgnoringModifiers: "", isARepeat: false, keyCode: code)
        else { return }
        if down { view.keyDown(with: event) } else { view.keyUp(with: event) }
    }

    private func modifier(_ code: UInt16, flags: NSEvent.ModifierFlags) {
        guard let event = NSEvent.keyEvent(with: .flagsChanged, location: .zero, modifierFlags: flags,
                                           timestamp: ProcessInfo.processInfo.systemUptime,
                                           windowNumber: window.windowNumber, context: nil, characters: "",
                                           charactersIgnoringModifiers: "", isARepeat: false, keyCode: code)
        else { return }
        view.flagsChanged(with: event)
    }

    // MARK: Clicks

    /// A click at a point of the screen, both from 0 to 1, the origin at the top left (as `screen` reports text)
    func click(x: Double, y: Double) {
        let inView = NSPoint(x: view.bounds.width * x, y: view.bounds.height * (view.isFlipped ? y : 1 - y))
        let location = view.convert(inView, to: nil)
        func event(_ type: NSEvent.EventType, clicks: Int) -> NSEvent? {
            NSEvent.mouseEvent(with: type, location: location, modifierFlags: [],
                               timestamp: ProcessInfo.processInfo.systemUptime, windowNumber: window.windowNumber,
                               context: nil, eventNumber: 0, clickCount: clicks, pressure: type == .leftMouseDown ? 1 : 0)
        }
        if let moved = event(.mouseMoved, clicks: 0) { view.mouseMoved(with: moved) }
        if let down = event(.leftMouseDown, clicks: 1) { view.mouseDown(with: down) }
        DispatchQueue.main.asyncAfter(deadline: .now() + 0.08) { [self] in
            if let up = event(.leftMouseUp, clicks: 1) { view.mouseUp(with: up) }
        }
    }

    // MARK: The screen

    /// The guest's screen as a picture, and how it was taken; tries the ways in turn and leaves out black pictures
    /// (a way that gives the window's size but not its content). `done` gets nil and what each way gave when none
    /// gave a picture. 2026-11-26 on the Mac: the view's own layers gave a black picture of the window's size
    func picture(done: @escaping @MainActor ((image: CGImage, from: String)?, [String]) -> Void) {
        var tried: [String] = []
        for (from, image) in [("the view's surface", surfaceImage()), ("the view's drawing", drawnImage())] {
            guard let image else { tried.append("\(from): none"); continue }
            if Self.isBlack(image) { tried.append("\(from): black"); continue }
            return done((image, from), tried)
        }
        // ScreenCaptureKit: the window as the window server composes it – asks once for screen recording
        Task { @MainActor in
            do {
                let image = try await windowCapture()
                if Self.isBlack(image) {
                    tried.append("ScreenCaptureKit: black")
                    done(nil, tried)
                } else {
                    done((image, "ScreenCaptureKit"), tried)
                }
            } catch {
                tried.append("ScreenCaptureKit: \(error.localizedDescription)")
                done(nil, tried)
            }
        }
    }

    /// The first IOSurface in the view's layers (the guest's frame buffer, if the view keeps it there)
    private func surfaceImage() -> CGImage? {
        var layers: [CALayer] = view.layer.map { [$0] } ?? []
        while let layer = layers.popLast() {
            if let contents = layer.contents, CFGetTypeID(contents as CFTypeRef) == IOSurfaceGetTypeID() {
                let surface = unsafeBitCast(contents as AnyObject, to: IOSurface.self)
                let image = CIImage(ioSurface: surface)
                if let cg = CIContext().createCGImage(image, from: image.extent) { return cg }
            }
            layers.append(contentsOf: layer.sublayers ?? [])
        }
        return nil
    }

    /// What AppKit draws of the view (empty for a view that draws with Metal)
    private func drawnImage() -> CGImage? {
        guard let rep = view.bitmapImageRepForCachingDisplay(in: view.bounds) else { return nil }
        view.cacheDisplay(in: view.bounds, to: rep)
        return rep.cgImage
    }

    /// This process's VM window through ScreenCaptureKit, at its pixel size
    private func windowCapture() async throws -> CGImage {
        let content = try await SCShareableContent.excludingDesktopWindows(false, onScreenWindowsOnly: false)
        let number = CGWindowID(window.windowNumber)
        guard let target = content.windows.first(where: { $0.windowID == number }) else {
            throw VMError("the VM's window is not among the shareable windows")
        }
        let filter = SCContentFilter(desktopIndependentWindow: target)
        let config = SCStreamConfiguration()
        let scale = window.backingScaleFactor
        config.width = Int(target.frame.width * scale)
        config.height = Int(target.frame.height * scale)
        config.showsCursor = false
        return try await SCScreenshotManager.captureImage(contentFilter: filter, configuration: config)
    }

    /// Every one of a coarse grid of pixels is (nearly) black
    static func isBlack(_ image: CGImage) -> Bool {
        let side = 32
        var pixels = [UInt8](repeating: 0, count: side * side * 4)
        let drawn = pixels.withUnsafeMutableBytes { buffer -> Bool in
            guard let context = CGContext(data: buffer.baseAddress, width: side, height: side, bitsPerComponent: 8,
                                          bytesPerRow: side * 4, space: CGColorSpaceCreateDeviceRGB(),
                                          bitmapInfo: CGImageAlphaInfo.premultipliedLast.rawValue) else { return false }
            context.draw(image, in: CGRect(x: 0, y: 0, width: side, height: side))
            return true
        }
        guard drawn else { return false }
        return stride(from: 0, to: pixels.count, by: 4).allSatisfy { i in
            pixels[i] < 8 && pixels[i + 1] < 8 && pixels[i + 2] < 8
        }
    }

    /// The text on the guest's screen, line by line with where it is (0 to 1, origin top left); `png`: the picture
    /// is also written there. Recognition runs off the main thread; `reply` is called once
    func screen(png: String?, reply: @escaping ControlServer.Reply) {
        picture { taken, tried in
            guard let (image, from) = taken else {
                return reply(["error": "no picture of the guest's screen – \(tried.joined(separator: "; "))"])
            }
            Self.recognize(image, from: from, tried: tried, png: png, reply: reply)
        }
    }

    private static func recognize(_ image: CGImage, from: String, tried: [String], png: String?,
                                  reply: @escaping ControlServer.Reply) {
        if let png {
            let rep = NSBitmapImageRep(cgImage: image)
            try? rep.representation(using: .png, properties: [:])?.write(to: URL(fileURLWithPath: png))
        }
        let size = ["width": image.width, "height": image.height]
        DispatchQueue.global(qos: .userInitiated).async {
            let request = VNRecognizeTextRequest()
            request.recognitionLevel = .accurate
            request.usesLanguageCorrection = false
            var lines: [[String: Any]] = []
            do {
                try VNImageRequestHandler(cgImage: image).perform([request])
                for observation in request.results ?? [] {
                    guard let best = observation.topCandidates(1).first else { continue }
                    let box = observation.boundingBox   // origin bottom left
                    lines.append([
                        "text": best.string,
                        "x": (box.midX * 1000).rounded() / 1000,
                        "y": ((1 - box.midY) * 1000).rounded() / 1000,
                    ])
                }
            } catch {
                return reply(["error": "text recognition failed: \(error.localizedDescription)"])
            }
            // Top to bottom, as read
            lines.sort { ($0["y"] as? Double ?? 0, $0["x"] as? Double ?? 0) < ($1["y"] as? Double ?? 0, $1["x"] as? Double ?? 0) }
            reply(["from": from, "tried": tried, "size": size, "lines": lines])
        }
    }
}
