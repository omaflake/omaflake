// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Progress of an image build, read from its log lines: the step ("2 of 4") and, while Nix builds or downloads,
/// how many of the announced packages are done. Both build paths log through here (CLI and app).
public struct BuildProgress: Equatable, Sendable {
    public var step: Int
    public var total: Int
    public var title: String
    /// Packages done / announced in the current step (Nix: "these N derivations will be built", "… paths will be fetched")
    public var done = 0
    public var of = 0

    /// 0…1 over all steps; the current step counts by its packages when Nix announced some
    public var fraction: Double {
        let inStep = of > 0 ? min(1, Double(done) / Double(of)) : 0
        return min(1, (Double(step - 1) + inStep) / Double(total))
    }
}

/// Turns build log lines into `BuildProgress`. Steps follow the log messages of `ImageStore.build` (NixOS) and
/// `MacImageBuilder.build` (macOS); Nix's own lines give the package counts.
public final class BuildProgressTracker: @unchecked Sendable {
    public private(set) var progress: BuildProgress

    let kind: GuestKind
    private let lock = NSLock()

    public init(kind: GuestKind) {
        self.kind = kind
        progress = kind == .nixos
            ? BuildProgress(step: 1, total: 4, title: "Evaluating the recipe")
            : BuildProgress(step: 1, total: 7, title: "Building the macOS system on the Mac")
    }

    /// macOS steps: the builder's log message that starts each one
    static let macSteps: [(prefix: String, title: String)] = [
        ("Building darwinConfigurations", "Building the macOS system on the Mac"),
        ("Cloning ", "Cloning the macOS base"),
        ("Waiting for the VM", "Starting the build VM"),
        ("Installing Nix", "Installing Nix in the guest"),
        ("Copying the closure", "Copying the system into the guest"),
        ("Activating", "Activating the system"),
        ("Shutting down", "Shutting down and storing the image"),
    ]

    /// Reads one log line; returns true when the progress changed.
    @discardableResult
    public func feed(_ line: String) -> Bool {
        lock.withLock {
            let before = progress
            countPackages(line)
            if kind == .nixos { nixosStep(line) } else { macStep(line) }
            return progress != before
        }
    }

    private func set(_ step: Int, _ title: String) {
        guard step > progress.step else { return }
        progress = BuildProgress(step: step, total: progress.total, title: title)
    }

    private func nixosStep(_ line: String) {
        if line.contains("nixos-disk-image>") { set(3, "Writing the disk image") }
        else if line.contains("copying path") && line.contains("nixos-disk-image") && line.contains("from 'ssh") {
            set(4, "Copying the image from the Linux builder")
        } else if progress.step == 1 && (line.hasPrefix("building '") || announced(line) != nil) {
            set(2, "Building packages")
        }
    }

    private func macStep(_ line: String) {
        for (i, s) in Self.macSteps.enumerated() where line.hasPrefix(s.prefix) { set(i + 1, s.title) }
    }

    /// "these 57 derivations will be built:", "this derivation will be built:", "these 12 paths will be fetched (…)"
    func announced(_ line: String) -> Int? {
        let l = line.trimmingCharacters(in: .whitespaces)
        if l.hasPrefix("this derivation will be built") || l.hasPrefix("this path will be fetched") { return 1 }
        guard l.hasPrefix("these "), l.contains(" will be built") || l.contains(" will be fetched") else { return nil }
        return Int(l.dropFirst(6).prefix { $0.isNumber })
    }

    private func countPackages(_ line: String) {
        if let n = announced(line) {
            progress.of += n
            if kind == .nixos && progress.step == 1 { set(2, "Building packages"); progress.of = n }
            return
        }
        // One finished unit: a build starting (Nix prints it once per derivation) or a path downloaded
        if line.hasPrefix("building '/nix/store/") || (line.hasPrefix("copying path '") && line.contains("from 'https://")) {
            progress.done = min(progress.done + 1, max(progress.of, progress.done + 1))
        }
    }
}

/// Lines of a build log that say nothing on their own: the disk image tool reports >100 000 "error while reading
/// directory" lines and one "copying path" per store path on every successful NixOS build; Nix lists and downloads
/// thousands of packages one line each – the progress counts those (BuildProgressTracker sees every line). They are
/// left out of what the app and the CLI show; the full log keeps them (plan B83).
public enum BuildLog {
    public static func isNoise(_ line: String) -> Bool {
        if line.contains("error while reading directory") { return true }
        if line.hasPrefix("copying path '") || line.hasPrefix("  /nix/store/") { return true }
        if line.hasPrefix("nixos-disk-image>") {
            let rest = line.dropFirst("nixos-disk-image>".count).trimmingCharacters(in: .whitespaces)
            return rest.isEmpty || rest.hasPrefix("copying path") || rest.hasPrefix("/nix/store/")
        }
        return false
    }
}
