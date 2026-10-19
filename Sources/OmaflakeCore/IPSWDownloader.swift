// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// A macOS release as Apple offers it for this Mac (`omaf-runner latest`, VZMacOSRestoreImage.latestSupported).
public struct MacOSRelease: Sendable, Equatable {
    public var version: String
    public var build: String
    public var url: URL
    public var bytes: Int64?

    public init(version: String, build: String, url: URL, bytes: Int64?) {
        self.version = version
        self.build = build
        self.url = url
        self.bytes = bytes
    }

    /// Output of `omaf-runner latest`:
    /// ```
    /// macOS 27.0.1 (26A434)
    /// URL:   https://updates.cdn-apple.com/…/UniversalMac_27.0.1_26A434_Restore.ipsw
    /// Size:  26.6 GB
    /// ```
    public static func parse(_ text: String) -> MacOSRelease? {
        let lines = text.split(separator: "\n").map { $0.trimmingCharacters(in: .whitespaces) }
        guard let first = lines.first, first.hasPrefix("macOS "), let open = first.lastIndex(of: "("),
              let close = first.lastIndex(of: ")"), open < close,
              let urlLine = lines.first(where: { $0.hasPrefix("URL:") }),
              let url = URL(string: urlLine.dropFirst(4).trimmingCharacters(in: .whitespaces)), url.scheme == "https" else { return nil }
        let version = first.dropFirst("macOS ".count).prefix { $0 != " " }
        var bytes: Int64?
        if let size = lines.first(where: { $0.hasPrefix("Size:") })?.dropFirst(5).trimmingCharacters(in: .whitespaces),
           let gb = Double(size.replacingOccurrences(of: " GB", with: "")) {
            bytes = Int64(gb * 1e9)
        }
        return MacOSRelease(version: String(version), build: String(first[first.index(after: open)..<close]), url: url, bytes: bytes)
    }

    /// Asks Apple via the runner (needs its entitlement); nil when offline or the runner is unusable.
    public static func latest(runner: String) -> MacOSRelease? {
        guard let r = try? Shell.run(runner, ["latest"]), r.status == 0 else { return nil }
        return parse(r.stdout)
    }
}

/// Downloads an IPSW from Apple into `ipsw/<build>.ipsw`. An interrupted or cancelled download keeps its resume data
/// in `ipsw/<build>.ipsw.resume` and continues from there next time (URLSession resume data).
public final class IPSWDownloader: NSObject, URLSessionDownloadDelegate, @unchecked Sendable {
    public let release: MacOSRelease
    public let destination: URL
    public var resumeFile: URL { destination.appendingPathExtension("resume") }

    private let lock = NSLock()
    private var task: URLSessionDownloadTask?
    private var result: Result<URL, Error>?
    private let done = DispatchSemaphore(value: 0)
    private var progress: (@Sendable (Int64, Int64) -> Void)?

    public init(release: MacOSRelease, directory: URL) {
        self.release = release
        self.destination = directory.appendingPathComponent("\(release.build).ipsw")
    }

    public static func file(_ release: MacOSRelease, in directory: URL) -> URL {
        directory.appendingPathComponent("\(release.build).ipsw")
    }

    /// Blocks until the file is complete (returns it) or the download failed/was cancelled (resume data kept).
    /// `progress` gets (received, expected) bytes.
    public func run(progress: @escaping @Sendable (Int64, Int64) -> Void) throws -> URL {
        let fm = FileManager.default
        if fm.fileExists(atPath: destination.path) { return destination }
        try fm.createDirectory(at: destination.deletingLastPathComponent(), withIntermediateDirectories: true)
        self.progress = progress
        let session = URLSession(configuration: .default, delegate: self, delegateQueue: nil)
        defer { session.finishTasksAndInvalidate() }
        let t: URLSessionDownloadTask
        if let data = try? Data(contentsOf: resumeFile) {
            t = session.downloadTask(withResumeData: data)
        } else {
            t = session.downloadTask(with: release.url)
        }
        lock.withLock { task = t }
        t.resume()
        done.wait()
        return try lock.withLock { result! }.get()
    }

    /// Stops the download and keeps what was received for the next `run`.
    public func cancel() {
        lock.withLock { task }?.cancel(byProducingResumeData: { [self] data in
            if let data { try? data.write(to: resumeFile, options: .atomic) }
        })
    }

    public func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didWriteData bytesWritten: Int64,
                           totalBytesWritten: Int64, totalBytesExpectedToWrite: Int64) {
        progress?(totalBytesWritten, totalBytesExpectedToWrite)
    }

    public func urlSession(_ session: URLSession, downloadTask: URLSessionDownloadTask, didFinishDownloadingTo location: URL) {
        // The temporary file is gone after this callback – move it now
        let fm = FileManager.default
        do {
            try fm.moveItem(at: location, to: destination)
            // Resume data of a finished download is useless
            try? Disposal.delete([resumeFile])
            lock.withLock { result = .success(destination) }
        } catch {
            lock.withLock { result = .failure(error) }
        }
    }

    public func urlSession(_ session: URLSession, task: URLSessionTask, didCompleteWithError error: Error?) {
        lock.withLock {
            if let error {
                if let data = (error as NSError).userInfo[NSURLSessionDownloadTaskResumeData] as? Data {
                    try? data.write(to: resumeFile, options: .atomic)
                }
                let cancelled = (error as NSError).code == NSURLErrorCancelled
                result = .failure(VMError(cancelled ? "Download paused – it continues from here next time"
                                                    : "Download interrupted (\(error.localizedDescription)) – it continues from here next time"))
            } else if result == nil {
                result = .failure(VMError("Download finished without a file"))
            }
        }
        done.signal()
    }
}

/// Keeps the Mac awake (no idle sleep) while long work runs: downloads, macOS installation, image builds.
public enum KeepAwake {
    public static func during<T>(_ reason: String, _ body: () throws -> T) rethrows -> T {
        let token = ProcessInfo.processInfo.beginActivity(options: [.idleSystemSleepDisabled, .userInitiated], reason: reason)
        defer { ProcessInfo.processInfo.endActivity(token) }
        return try body()
    }
}

/// Free space for the volume of `url` (what macOS would make available for important data).
public func freeSpace(at url: URL) -> Int64? {
    var dir = url
    while !FileManager.default.fileExists(atPath: dir.path), dir.path != "/" { dir.deleteLastPathComponent() }
    let v = try? dir.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey, .volumeAvailableCapacityKey])
    // The read-only Nix store volume reports 0 for "important usage"; the plain free space is right there
    if let important = v?.volumeAvailableCapacityForImportantUsage, important > 0 { return important }
    return v?.volumeAvailableCapacity.map(Int64.init)
}

public func gigabytes(_ bytes: Int64) -> String { String(format: "%.1f GB", Double(bytes) / 1e9) }
