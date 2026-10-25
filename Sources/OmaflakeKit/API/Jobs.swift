// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Work that takes minutes, running in the server (api.md §5)
public struct Job: Decodable, Sendable, Equatable, Identifiable {
    public enum Kind: String, OpenEnum {
        case build, macosInstall = "macos-install", ipswDownload = "ipsw-download"
        case builderInstallNix = "builder-install-nix", builderMountNix = "builder-mount-nix", builderSetup = "builder-setup"
        case builderApply = "builder-apply"
        case builderCleanUp = "builder-clean-up", `import`, export, dataMove = "data-move", recipeInstall = "recipe-install", lookInstall = "look-install"
        case cliInstall = "cli-install"
        case unknown

        public var isBuilderTask: Bool { [.builderInstallNix, .builderMountNix, .builderSetup, .builderApply, .builderCleanUp].contains(self) }
    }

    public enum State: String, OpenEnum {
        case queued, checking, running, finished, failed, cancelled, unknown
    }

    public struct Progress: Decodable, Sendable, Equatable {
        /// Step of `total` (builds; 0 = no steps known – downloads)
        public var step: Int
        public var total: Int
        public var title: String
        /// Units of the current step: packages of a build step, bytes of a download
        public var done: Int64
        public var of: Int64
        /// 0…1 overall
        public var fraction: Double

        public init(step: Int = 0, total: Int = 0, title: String = "", done: Int64 = 0, of: Int64 = 0, fraction: Double = 0) {
            self.step = step
            self.total = total
            self.title = title
            self.done = done
            self.of = of
            self.fraction = fraction
        }
    }

    public var id: String
    public var kind: Kind
    public var title: String
    /// build, recipe-install: the recipe
    public var recipe: String?
    public var guestKind: GuestKind?
    public var state: State
    public var progress: Progress?
    public var started: Date?
    public var finished: Date?
    /// When the job last wrote a line (a stalled build shows it)
    public var lastOutput: Date?
    public var logLines: Int
    /// Routine lines left out of the log (still in logFile)
    public var hiddenLines: Int
    /// Every line of the job, also the routine ones
    public var logFile: String?
    /// `jobs.cancel` would do something now
    public var cancellable: Bool
    /// When finished: the kind's result (`result(as:)` with `BuildResult`, `ImportResult`, …)
    public var result: JSONValue?
    /// When failed
    public var error: String?
    /// `error` as a text when it has a key (api.md §3) – shown in the app's language
    public var errorText: ViewText?

    /// Waiting or running
    public var isActive: Bool { [.queued, .checking, .running].contains(state) }
    public var isRunning: Bool { [.checking, .running].contains(state) }
    public var isDone: Bool { [.finished, .failed, .cancelled].contains(state) }

    /// The result as its kind's type; nil when there is none or it does not decode
    public func result<R: Decodable>(as type: R.Type) -> R? {
        guard let result, let data = try? JSONEncoder.api.encode(result) else { return nil }
        return try? JSONDecoder.api.decode(R.self, from: data)
    }
}

/// `jobs.log`
public struct JobLog: Decodable, Sendable, Equatable {
    public var lines: [String]
    /// The `from` of the next call
    public var next: Int
}

// MARK: - Job results (per kind)

/// `build`
public struct BuildResult: Decodable, Sendable, Equatable {
    public var image: String
    /// Older images of its line removed (plan B52)
    public var removed: [String]
}

/// `recipe-install`
public struct InstallResult: Decodable, Sendable, Equatable {
    /// The recipe versions written (empty: it was up to date)
    public var installed: [String]
}

/// `import`
public struct ImportResult: Decodable, Sendable, Equatable {
    public var kind: PackageInspection.Kind
    /// The VM's or the image's name; recipe ids for recipes (empty: nothing new)
    public var names: [String]
    public var notes: [String]
}

/// `export`: the package written
public typealias ExportResult = PathAnswer

/// `data-move`
public struct DataMoveResult: Decodable, Sendable, Equatable { public var dataFolder: String }

/// `ipsw-download`
public struct IpswResult: Decodable, Sendable, Equatable {
    public var file: String
    public var version: String
}

/// `macos-install`
public struct BaseResult: Decodable, Sendable, Equatable {
    public var base: String
    public var existing: Bool
    public var ready: Bool
}

/// The Linux builder tasks
public struct MessageResult: Decodable, Sendable, Equatable {
    /// What it did, in English
    public var message: String
}
