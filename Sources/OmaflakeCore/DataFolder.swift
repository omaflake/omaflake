// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Moves omaflake's data folder (default ~/.omaflake) to another place and back, e.g. to ~/Documents/omaflake-vms.
/// Only omaflake's own entries move (`Paths.dataEntries`); the settings file stays in ~/.omaflake and records
/// the new place (`home`). Nothing is copied or deleted: the folders are renamed, which needs the same volume.
///
/// Absolute paths that point into the data folder are rewritten afterwards: `imagePath` of the instances (informational),
/// a share folder or QEMU/runner path inside the data folder. Images and VMs are self-contained packages and the
/// qcow2 overlays name their image by a relative path (plan B108), so they need nothing.
public enum DataFolder {
    /// Moves the data of `from` to `to` and stores `to` in `settingsFile`; returns the new paths.
    /// Refuses while an instance is running, if `to` already holds omaflake data, or across volumes.
    @discardableResult
    public static func move(from old: Paths, to target: URL, settingsFile: URL = Paths.defaultHome.appendingPathComponent("config.json"),
                            log: @escaping @Sendable (String) -> Void = { _ in }) throws -> Paths {
        let fm = FileManager.default
        let from = old.root.standardizedFileURL.resolvingSymlinksInPath()
        let to = target.standardizedFileURL.resolvingSymlinksInPath()
        guard from.path != to.path else { throw VMError("\(to.path) is the data folder already") }
        guard !to.path.hasPrefix(from.path + "/"), !from.path.hasPrefix(to.path + "/") else {
            throw VMError("The new data folder must not be inside the old one or contain it")
        }
        let running = InstanceStore(paths: old, qemuImg: "qemu-img").list().filter { isRunning($0, paths: old) }.map(\.name)
        guard running.isEmpty else { throw VMError("Stop these VMs first: \(running.joined(separator: ", "))") }

        let entries = Paths.dataEntries.filter { fm.fileExists(atPath: from.appendingPathComponent($0).path) }
        try fm.createDirectory(at: to, withIntermediateDirectories: true)
        let clashes = entries.filter { fm.fileExists(atPath: to.appendingPathComponent($0).path) }
        guard clashes.isEmpty else {
            throw VMError("\(to.path) already contains omaflake data (\(clashes.joined(separator: ", "))) – nothing was moved")
        }
        if !entries.isEmpty, try volume(of: from) != volume(of: to) {
            // Copying would expand sparse disks and break APFS clones of macOS images (OPEN: copy with clonefile/sparse)
            throw VMError("\(to.path) is on another volume – moving across volumes is not supported yet")
        }

        // Settings first (read from the old file, which may be inside the old data folder)
        var settings = Settings.load(old)
        settings.home = to.path == Paths.defaultHome.standardizedFileURL.resolvingSymlinksInPath().path ? nil : to.path
        settings.qemu = settings.qemu.map { rebase($0, from: from, to: to) }
        settings.runner = settings.runner.map { rebase($0, from: from, to: to) }

        for e in entries {
            try fm.moveItem(at: from.appendingPathComponent(e), to: to.appendingPathComponent(e))
            log("moved \(e)")
        }
        try settings.save(file: settingsFile)
        let paths = Paths(root: to, configFile: settingsFile)
        log("settings: \(settingsFile.path) (data folder \(to.path))")

        // Instances: image path (informational), share folder
        let store = InstanceStore(paths: paths, qemuImg: "qemu-img")
        for var c in store.list() {
            let image = rebase(c.imagePath, from: from, to: to)
            let share = c.sharePath.map { rebase($0, from: from, to: to) }
            guard image != c.imagePath || share != c.sharePath else { continue }
            c.imagePath = image
            c.sharePath = share
            try store.save(c)
            log("instance \(c.name) points to \(image)")
        }
        return paths
    }

    /// `path` inside `from` → the same place inside `to`; other paths unchanged.
    static func rebase(_ path: String, from: URL, to: URL) -> String {
        path.hasPrefix(from.path + "/") ? to.path + path.dropFirst(from.path.count) : path
    }

    static func isRunning(_ c: InstanceConfig, paths: Paths) -> Bool {
        let ip = paths.instance(c.name)
        return livePid(file: ip.pidFile, executable: "qemu") != nil || livePid(file: ip.runnerPid, executable: "omaf-runner") != nil
    }

    static func volume(of url: URL) throws -> AnyHashable {
        guard let id = try url.resourceValues(forKeys: [.volumeIdentifierKey]).volumeIdentifier as? AnyHashable else {
            throw VMError("Cannot determine the volume of \(url.path)")
        }
        return id
    }
}
