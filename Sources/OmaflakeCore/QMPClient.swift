// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

/// QEMU Machine Protocol over the instance's Unix socket (line-based JSON).
/// QEMU also sends events between command and response (plan B13) – `execute`
/// skips them and records them in `events`.
public final class QMPClient {
    private let fd: Int32
    private var buffer = Data()
    public private(set) var events: [String] = []

    /// Connects and switches to command mode (`qmp_capabilities`).
    public init(socket: URL, timeout: TimeInterval = 5) throws {
        fd = try connectUnix(socket.path, timeout: timeout)
        let greeting = try readMessage()
        // No close here: `fd` is set, so a throw still runs deinit, which closes it (twice would hit whatever file got
        // the number in between – plan B105)
        guard greeting["QMP"] != nil else { throw VMError("Unexpected QMP greeting") }
        _ = try execute("qmp_capabilities")
    }

    deinit { close(fd) }

    /// Sends a command and returns `return`; QMP errors are thrown.
    @discardableResult
    public func execute(_ command: String) throws -> Any {
        var line = try JSONSerialization.data(withJSONObject: ["execute": command])
        line.append(0x0A)
        let sent = line.withUnsafeBytes { write(fd, $0.baseAddress, $0.count) }
        guard sent == line.count else { throw VMError("QMP: send failed") }
        while true {
            let msg = try readMessage()
            if let event = msg["event"] as? String { events.append(event); continue }
            if let error = msg["error"] as? [String: Any] {
                throw VMError("QMP \(command): \(error["desc"] as? String ?? "error")")
            }
            return msg["return"] ?? [:]
        }
    }

    /// `query-status` → e.g. "running", "paused", "shutdown"
    public func status() throws -> String {
        let r = try execute("query-status") as? [String: Any]
        return r?["status"] as? String ?? "unknown"
    }

    private func readMessage() throws -> [String: Any] {
        while true {
            if let nl = buffer.firstIndex(of: 0x0A) {
                let line = buffer[buffer.startIndex..<nl]
                buffer.removeSubrange(buffer.startIndex...nl)
                if line.isEmpty { continue }
                return try QMPClient.parse(Data(line))
            }
            var chunk = [UInt8](repeating: 0, count: 4096)
            let n = read(fd, &chunk, chunk.count)
            guard n > 0 else { throw VMError("QMP: connection closed or timed out") }
            buffer.append(contentsOf: chunk[0..<n])
        }
    }

    static func parse(_ line: Data) throws -> [String: Any] {
        guard let obj = try JSONSerialization.jsonObject(with: line) as? [String: Any] else {
            throw VMError("QMP: not a JSON object")
        }
        return obj
    }
}
