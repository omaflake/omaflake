// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Work that takes minutes, running in the server (api.md §5)
public struct Job: Decodable, Sendable, Equatable, Identifiable {
    public enum Kind: String, OpenEnum {
        case build, macosInstall = "macos-install", ipswDownload = "ipsw-download"
        case builderInstallNix = "builder-install-nix", builderSetup = "builder-setup", builderApply = "builder-apply"
        case builderCleanUp = "builder-clean-up", `import`, export, dataMove = "data-move", recipeInstall = "recipe-install"
        case unknown

        public var isBuilderTask: Bool { [.builderInstallNix, .builderSetup, .builderApply, .builderCleanUp].contains(self) }
    }

    public enum State: String, OpenEnum {
        case queued, checking, running, finished, failed, cancelled, unknown
    }

    public struct Progress: Decodable, Sendable, Equatable {
        public var step = 0
        public var total = 0
        public var title = ""
        /// Packages done / announced in the current step (a download: bytes)
        public var done: Int64 = 0
        public var of: Int64 = 0
        public var fraction: Double = 0

        public init() {}

        enum CodingKeys: String, CodingKey { case step, total, title, done, of, fraction }
        public init(from decoder: Decoder) throws {
            let c = try decoder.container(keyedBy: CodingKeys.self)
            step = try c.value(.step, 0)
            total = try c.value(.total, 0)
            title = try c.value(.title, "")
            done = try c.value(.done, 0)
            of = try c.value(.of, 0)
            fraction = try c.value(.fraction, 0)
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
    /// When the job last printed something (a time – OPEN in api.md: its format is not stated)
    public var lastOutput: Date?
    public var logLines: Int
    /// Routine lines left out of the log (still in logFile)
    public var hiddenLines: Int
    public var logFile: String?
    /// When finished: e.g. {"image": "…", "removed": ["…"]} for a build
    public var result: JSONValue?
    /// When failed
    public var error: String?

    enum CodingKeys: String, CodingKey {
        case id, kind, title, recipe, guestKind, state, progress, started, finished, lastOutput, logLines, hiddenLines
        case logFile, result, error
    }

    public init(from decoder: Decoder) throws {
        let c = try decoder.container(keyedBy: CodingKeys.self)
        id = try c.decode(String.self, forKey: .id)
        kind = try c.value(.kind, .unknown)
        title = try c.value(.title, "")
        recipe = try c.decodeIfPresent(String.self, forKey: .recipe)
        guestKind = try c.decodeIfPresent(GuestKind.self, forKey: .guestKind)
        state = try c.value(.state, .unknown)
        progress = try c.decodeIfPresent(Progress.self, forKey: .progress)
        started = c.lenientDate(.started)
        finished = c.lenientDate(.finished)
        lastOutput = c.lenientDate(.lastOutput)
        logLines = try c.value(.logLines, 0)
        hiddenLines = try c.value(.hiddenLines, 0)
        logFile = try c.decodeIfPresent(String.self, forKey: .logFile)
        result = try c.decodeIfPresent(JSONValue.self, forKey: .result)
        error = try c.decodeIfPresent(String.self, forKey: .error)
    }

    /// Waiting or running
    public var isActive: Bool { [.queued, .checking, .running].contains(state) }
    public var isRunning: Bool { [.checking, .running].contains(state) }
    public var isDone: Bool { [.finished, .failed, .cancelled].contains(state) }
}

/// `jobs.log`
public struct JobLog: Decodable, Sendable, Equatable {
    public var lines: [String]
    public var next: Int
}
