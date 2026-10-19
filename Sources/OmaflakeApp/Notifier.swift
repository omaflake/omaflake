// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import UserNotifications

/// System notifications for what one waits for: a VM ready, an image built or failed, a VM that ended (plan B91).
/// Only while the menu window is closed – when it is open, the footer line shows the same message.
/// macOS asks once whether omaflake may send them; that question comes with the first notification, not at launch.
@MainActor
enum Notifier {
    /// The menu window (set by `RoundedWindow`), to tell whether it is open
    static weak var menuWindow: NSWindow?
    private static var authorized: Bool?

    static var menuWindowOpen: Bool { menuWindow?.isVisible == true }

    static func post(_ title: String, _ body: String? = nil) {
        guard !menuWindowOpen else { return }
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
