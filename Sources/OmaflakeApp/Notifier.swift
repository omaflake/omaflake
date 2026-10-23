// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import UserNotifications

/// System notifications for what one waits for: a VM ready, an image built or failed, a VM that ended (plan B91).
/// Only while omaflake is not in front (menu window closed, app window hidden or in the background) – otherwise the
/// footer line shows the same message.
/// macOS asks once whether omaflake may send them; that question comes with the first notification, not at launch.
@MainActor
enum Notifier {
    /// The menu window (set by `RoundedWindow`), to tell whether it is open
    static weak var menuWindow: NSWindow?
    /// The app window (set by `MainWindowHook`, plan B119)
    static weak var mainWindow: NSWindow?
    private static var authorized: Bool?

    static var menuWindowOpen: Bool { menuWindow?.isVisible == true }
    static var inFront: Bool { menuWindowOpen || (NSApp.isActive && mainWindow?.isVisible == true) }

    static func post(_ title: String, _ body: String? = nil) {
        guard !inFront else { return }
        Task { @MainActor in
            let center = UNUserNotificationCenter.current()
            if authorized == nil {
                authorized = (try? await center.requestAuthorization(options: [.alert, .sound])) ?? false
            }
            guard authorized == true else { return }
            let content = UNMutableNotificationContent()
            content.title = title
            if let body { content.body = body }
            content.sound = .default
            try? await center.add(UNNotificationRequest(identifier: UUID().uuidString, content: content, trigger: nil))
        }
    }
}
