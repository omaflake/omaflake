// SPDX-License-Identifier: AGPL-3.0-or-later
import SwiftUI
import OmaflakeKit

/// Where omaflake shows itself (plan B119): the menu bar window, a regular app window with a Dock icon, or both.
/// A per-user UI preference in UserDefaults, not one of the core's settings. The pages are the same `MainView` in
/// both frames; only the frame differs.
enum AppFrame: String, CaseIterable, Identifiable {
    case menuBar, window, both

    static let key = "appFrame"
    /// Id of the app window's `Window` scene
    static let windowID = "main"

    var id: String { rawValue }

    var title: LocalizedStringKey {
        switch self {
        case .menuBar: "Menu Bar"
        case .window: "Window"
        case .both: "Both"
        }
    }

    var showsMenuBar: Bool { self != .window }
    var showsWindow: Bool { self != .menuBar }

    /// The stored choice; the menu bar (no Dock icon) by default
    static var current: AppFrame {
        UserDefaults.standard.string(forKey: key).flatMap(AppFrame.init(rawValue:)) ?? .menuBar
    }

    /// With the window: a Dock icon and the app menu (`.regular`); menu bar only: neither (`.accessory`, like
    /// Info.plist's `LSUIElement`, which the app starts with).
    @MainActor func applyActivationPolicy() {
        NSApp.setActivationPolicy(showsWindow ? .regular : .accessory)
    }

    /// Applies a new choice at once: the menu bar item follows the scene's `isInserted`, the window is opened or
    /// closed here.
    @MainActor func apply(openWindow: OpenWindowAction, dismissWindow: DismissWindowAction) {
        applyActivationPolicy()
        if showsWindow {
            openWindow(id: Self.windowID)
            NSApp.activate()
        } else {
            dismissWindow(id: Self.windowID)
        }
    }
}

/// Hooks the app window: tells `Notifier` about it, and remembers its frame if SwiftUI does not already (with state
/// restoration off, `restorationBehavior(.disabled)`, OPEN whether SwiftUI still saves it itself).
struct MainWindowHook: NSViewRepresentable {
    static let autosaveName = "omaflake main window"

    func makeNSView(context: Context) -> NSView { Hook() }
    func updateNSView(_ nsView: NSView, context: Context) {}

    final class Hook: NSView {
        override func viewDidMoveToWindow() {
            super.viewDidMoveToWindow()
            guard let window else { return }
            Notifier.mainWindow = window
            if window.frameAutosaveName.isEmpty {
                window.setFrameUsingName(MainWindowHook.autosaveName)
                window.setFrameAutosaveName(MainWindowHook.autosaveName)
            }
            // The app window is the whole app: while it is open, omaflake is in the Dock and the app switcher – also
            // when it lives in the menu bar (opened with "Open omaflake"); closed, a menu bar omaflake leaves the Dock
            // again (plan B151). The observer goes with the view (selector-based observers are removed then).
            NSApp.setActivationPolicy(.regular)
            let center = NotificationCenter.default
            center.removeObserver(self)
            center.addObserver(self, selector: #selector(windowWillClose), name: NSWindow.willCloseNotification, object: window)
            // SwiftUI may keep the window and show it again: each time it comes to the front, the Dock icon is back
            center.addObserver(self, selector: #selector(windowDidBecomeKey), name: NSWindow.didBecomeKeyNotification, object: window)
        }

        @objc private func windowWillClose(_ note: Notification) {
            if !AppFrame.current.showsWindow { NSApp.setActivationPolicy(.accessory) }
        }

        @objc private func windowDidBecomeKey(_ note: Notification) {
            if NSApp.activationPolicy() != .regular { NSApp.setActivationPolicy(.regular) }
        }
    }
}

/// App menu of the window frame: Settings opens the window on the Settings page; Quit asks like the power button
/// when VMs are running.
struct MainCommands: Commands {
    let model: AppModel
    @Environment(\.openWindow) private var openWindow

    var body: some Commands {
        CommandGroup(replacing: .appSettings) {
            Button("Settings…") {
                model.show(PageRef("settings"))
                openWindow(id: AppFrame.windowID)
                NSApp.activate()
            }
            .keyboardShortcut(",")
        }
        CommandGroup(replacing: .appTermination) {
            Button("Quit omaflake") { model.quit() }.keyboardShortcut("q")
        }
    }
}
