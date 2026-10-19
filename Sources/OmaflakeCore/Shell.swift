// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Runs external tools (nix, qemu-img, git) synchronously.
public enum Shell {
    public struct Result: Sendable {
        public let status: Int32
        public let stdout: String
        public let stderr: String
    }

    /// Runs `executable` and collects output. `passthroughStderr` shows stderr live (e.g. the nix build log).
    /// `input` goes to stdin (e.g. a password, never as an argument); `environment` is added to this process's.
    @discardableResult
    public static func run(_ executable: String, _ args: [String], cwd: URL? = nil,
                           passthroughStderr: Bool = false, input: Data? = nil,
                           environment: [String: String] = [:]) throws -> Result {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: try which(executable))
        p.arguments = args
        if let cwd { p.currentDirectoryURL = cwd }
        if !environment.isEmpty {
            p.environment = ProcessInfo.processInfo.environment.merging(environment) { _, new in new }
        }
        let stdin = Pipe()
        if input != nil { p.standardInput = stdin }
        let out = Pipe()
        p.standardOutput = out
        let err = Pipe()
        p.standardError = passthroughStderr ? FileHandle.standardError : err
        try Cancellation.current?.check()
        try p.run()
        Cancellation.current?.track(p)
        defer { Cancellation.current?.untrack(p) }
        if let input {
            stdin.fileHandleForWriting.write(input)
            try? stdin.fileHandleForWriting.close()
        }
        // Read stdout before waitUntilExit, otherwise a full pipe buffer blocks
        let outData = out.fileHandleForReading.readDataToEndOfFile()
        let errData = passthroughStderr ? Data() : err.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        return Result(status: p.terminationStatus,
                      stdout: String(decoding: outData, as: UTF8.self),
                      stderr: String(decoding: errData, as: UTF8.self))
    }

    /// Like `run`, but stderr goes line by line to `onLine` (live log, e.g. nix build -L).
    /// `started` receives the process so the caller can cancel (`terminate()`).
    /// `mergeStdout` sends stdout to `onLine` too (programs that report progress on stdout, like omaf-runner);
    /// `input` goes to stdin.
    public static func stream(_ executable: String, _ args: [String], cwd: URL? = nil,
                              environment: [String: String] = [:], input: Data? = nil, mergeStdout: Bool = false,
                              started: (Process) -> Void = { _ in },
                              onLine: @escaping @Sendable (String) -> Void) throws -> Result {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: try which(executable))
        p.arguments = args
        if let cwd { p.currentDirectoryURL = cwd }
        if !environment.isEmpty {
            p.environment = ProcessInfo.processInfo.environment.merging(environment) { _, new in new }
        }
        let out = Pipe()
        let err = Pipe()
        p.standardOutput = mergeStdout ? err : out
        p.standardError = err
        let stdin = Pipe()
        if input != nil { p.standardInput = stdin }
        let splitter = LineSplitter(onLine)
        err.fileHandleForReading.readabilityHandler = { h in splitter.feed(h.availableData) }
        try Cancellation.current?.check()
        try p.run()
        Cancellation.current?.track(p)
        defer { Cancellation.current?.untrack(p) }
        started(p)
        if let input {
            stdin.fileHandleForWriting.write(input)
            try? stdin.fileHandleForWriting.close()
        }
        if mergeStdout { try? out.fileHandleForWriting.close() }
        let outData = out.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        err.fileHandleForReading.readabilityHandler = nil
        splitter.feed(err.fileHandleForReading.readDataToEndOfFile())
        splitter.flush()
        return Result(status: p.terminationStatus, stdout: String(decoding: outData, as: UTF8.self), stderr: "")
    }

    /// Like `run`, but throws on exit code ≠ 0.
    @discardableResult
    public static func check(_ executable: String, _ args: [String], cwd: URL? = nil,
                             passthroughStderr: Bool = false, environment: [String: String] = [:]) throws -> String {
        let r = try run(executable, args, cwd: cwd, passthroughStderr: passthroughStderr, environment: environment)
        guard r.status == 0 else {
            let msg = r.stderr.trimmingCharacters(in: .whitespacesAndNewlines)
            throw VMError("\(executable) \(args.joined(separator: " ")) failed (\(r.status))\(msg.isEmpty ? "" : ":\n\(msg)")")
        }
        return r.stdout
    }

    /// Finds the absolute path of a program: `nix` from `Settings.nix` if set, otherwise PATH, then common locations.
    public static func which(_ name: String) throws -> String {
        if name.hasPrefix("/") { return name }
        if name == "nix", let custom = Settings.load(Paths.standard()).nix {
            return (custom as NSString).expandingTildeInPath
        }
        for d in searchDirectories() {
            let path = (d as NSString).appendingPathComponent(name)
            if FileManager.default.isExecutableFile(atPath: path) { return path }
        }
        throw VMError("Program not found: \(name)")
    }

    /// PATH, then where Nix, nix-darwin and Homebrew put programs (an app started from Finder has a minimal PATH).
    public static func searchDirectories() -> [String] {
        let home = FileManager.default.homeDirectoryForCurrentUser.path
        var dirs = (ProcessInfo.processInfo.environment["PATH"] ?? "").split(separator: ":").map(String.init)
        dirs += ["/run/current-system/sw/bin", "/etc/profiles/per-user/\(NSUserName())/bin", "\(home)/.nix-profile/bin",
                 "/nix/var/nix/profiles/default/bin", "/opt/homebrew/bin", "/usr/bin", "/bin"]
        var seen = Set<String>()
        return dirs.filter { seen.insert($0).inserted }
    }
}

/// Splits incoming bytes into lines (thread-safe, readabilityHandler runs on a foreign queue).
final class LineSplitter: @unchecked Sendable {
    private var buffer = Data()
    private let lock = NSLock()
    private let onLine: @Sendable (String) -> Void

    init(_ onLine: @escaping @Sendable (String) -> Void) { self.onLine = onLine }

    func feed(_ data: Data) {
        guard !data.isEmpty else { return }
        lock.lock()
        buffer.append(data)
        var lines: [String] = []
        while let nl = buffer.firstIndex(of: 0x0A) {
            lines.append(String(decoding: buffer[buffer.startIndex..<nl], as: UTF8.self))
            buffer.removeSubrange(buffer.startIndex...nl)
        }
        lock.unlock()
        lines.forEach(onLine)
    }

    func flush() {
        lock.lock()
        let rest = buffer
        buffer.removeAll()
        lock.unlock()
        if !rest.isEmpty { onLine(String(decoding: rest, as: UTF8.self)) }
    }
}
