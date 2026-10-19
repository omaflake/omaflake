// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Deleting what the user removes – an instance, its old state on reset/upgrade, an image, a failed build. The
/// product deletes for real; "never delete, move to _to_delete" is a rule for the developers' (and agents') own
/// actions during development, not for the app (spec §9).
public enum Disposal {
    /// Deletes the items; read-only folders (macOS images and bases) are made writable first. Missing ones are skipped.
    public static func delete(_ items: [URL]) throws {
        let fm = FileManager.default
        for item in items {
            let isLink = (try? fm.destinationOfSymbolicLink(atPath: item.path)) != nil
            guard isLink || fm.fileExists(atPath: item.path) else { continue }
            if !isLink { makeWritable(item) }
            try fm.removeItem(at: item)
        }
    }

    /// u+w on a folder and everything in it (only what is needed to delete it)
    static func makeWritable(_ url: URL) {
        let fm = FileManager.default
        var isDir: ObjCBool = false
        guard fm.fileExists(atPath: url.path, isDirectory: &isDir), isDir.boolValue else { return }
        if !fm.isWritableFile(atPath: url.path) {
            let mode = (try? fm.attributesOfItem(atPath: url.path)[.posixPermissions] as? Int) ?? 0o555
            try? fm.setAttributes([.posixPermissions: mode | 0o700], ofItemAtPath: url.path)
        }
        for name in (try? fm.contentsOfDirectory(atPath: url.path)) ?? [] {
            let child = url.appendingPathComponent(name)
            if (try? fm.destinationOfSymbolicLink(atPath: child.path)) == nil { makeWritable(child) }
        }
    }
}
