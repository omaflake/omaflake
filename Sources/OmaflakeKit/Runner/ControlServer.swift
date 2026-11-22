// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

/// A JSON object handed between threads (values are plain JSON: strings, numbers, booleans, null, lists, objects)
public struct ControlMessage: @unchecked Sendable {
    public var object: [String: Any]
    public init(_ object: [String: Any]) { self.object = object }
}

/// The control socket of `omaf-runner run` and `omaf-runner bridge --control` (spec §11.9, "Relay"): a Unix socket,
/// one request per connection – a JSON object on one line, `{"command": …}` plus the command's own fields
/// (`diagnose`: `"check"`, `"args"?`) – answered with one JSON object on one line, then closed. Counterparts:
/// `RunnerClient` here, `MacVmControl` in the core. The handler runs on `queue` and may answer later from any
/// thread (a diagnosis waits up to 15 s for the guest); it must answer exactly once.
public final class ControlServer: @unchecked Sendable {
    public typealias Reply = @Sendable ([String: Any]) -> Void
    public typealias Handler = @Sendable (ControlMessage, @escaping Reply) -> Void

    /// A request line longer than this is cut off (and then not valid JSON)
    public static let maxRequest = 256 * 1024

    private let path: URL
    private let fd: Int32
    private let source: DispatchSourceRead
    private let io = DispatchQueue(label: "app.omaflake.control", attributes: .concurrent)

    public init(path: URL, queue: DispatchQueue = .main, handler: @escaping Handler) throws {
        self.path = path
        unlink(path.path)   // leftover from an earlier run (socket file, no user content)
        let listener = socket(AF_UNIX, SOCK_STREAM, 0)
        guard listener >= 0 else { throw VMError("socket() failed") }
        _ = fcntl(listener, F_SETFD, FD_CLOEXEC)
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: addr.sun_path) else {
            Darwin.close(listener)
            throw VMError("Socket path too long (max. 103 bytes): \(path.path)")
        }
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        addr.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let rc = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard rc == 0, listen(listener, 8) == 0 else {
            let e = errno
            Darwin.close(listener)
            throw VMError("Socket \(path.path) not usable: \(String(cString: strerror(e)))")
        }
        fd = listener
        source = DispatchSource.makeReadSource(fileDescriptor: listener, queue: io)
        let io = self.io
        source.setEventHandler {
            let client = accept(listener, nil, nil)
            guard client >= 0 else { return }
            _ = fcntl(client, F_SETFD, FD_CLOEXEC)
            // Read off the handler's queue: a slow client never holds up the VM's main thread
            io.async {
                let request = ControlMessage(Self.readRequest(client))
                let answered = Answered()
                let reply: Reply = { object in
                    guard answered.once() else { return }
                    Self.write(object, to: client)
                    Darwin.close(client)
                }
                queue.async { handler(request, reply) }
            }
        }
        // The listener is closed only once the source is cancelled: closed earlier, its number could be reused by
        // another socket (a listener elsewhere in the process) while the source still watches it (plan B105)
        source.setCancelHandler { Darwin.close(listener) }
        source.resume()
    }

    /// Stops listening and removes the socket file; once only
    public func close() {
        guard closed.once() else { return }
        source.cancel()
        unlink(path.path)
    }

    private let closed = Answered()

    /// The request's JSON object; empty when it is not one (the handler then answers "Unknown command")
    static func readRequest(_ client: Int32) -> [String: Any] {
        var tv = timeval(tv_sec: 2, tv_usec: 0)
        setsockopt(client, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        var data = Data()
        var buf = [UInt8](repeating: 0, count: 4096)
        while !data.contains(UInt8(ascii: "\n")) && data.count < maxRequest {
            let n = read(client, &buf, buf.count)
            if n < 0 && errno == EINTR { continue }
            if n <= 0 { break }
            data.append(contentsOf: buf[0..<n])
        }
        if let nl = data.firstIndex(of: UInt8(ascii: "\n")) { data = data[data.startIndex..<nl] }
        return ((try? JSONSerialization.jsonObject(with: data)) as? [String: Any]) ?? [:]
    }

    static func write(_ object: [String: Any], to client: Int32) {
        var tv = timeval(tv_sec: 5, tv_usec: 0)
        setsockopt(client, SOL_SOCKET, SO_SNDTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        let json = JSONSerialization.isValidJSONObject(object) ? (try? JSONSerialization.data(withJSONObject: object)) : nil
        let out = (json ?? Data(#"{"error":"the answer is not JSON"}"#.utf8)) + Data("\n".utf8)
        out.withUnsafeBytes { buf in
            var off = 0
            while off < buf.count {
                let n = Darwin.write(client, buf.baseAddress! + off, buf.count - off)
                if n < 0 && errno == EINTR { continue }
                if n <= 0 { return }
                off += n
            }
        }
    }

    private final class Answered: @unchecked Sendable {
        private let lock = NSLock()
        private var done = false
        func once() -> Bool { lock.withLock { defer { done = true }; return !done } }
    }

    // MARK: Commands both servers answer

    /// `{"command": "diagnose", "check", "args"?}`: the agent's answer to that check (`AgentLink.diagnose`), or
    /// `{"error"}` – waits on a thread of its own, so the caller's queue goes on
    public static func diagnose(_ link: AgentLink?, _ request: ControlMessage, reply: @escaping Reply) {
        guard let link else { reply(["error": "this VM runs without the omaflake agent"]); return }
        guard let check = request.object["check"] as? String else {
            reply(["error": "diagnose needs a check (\"check\": \"info\", …)"])
            return
        }
        let raw = request.object["args"]
        guard raw == nil || raw is NSNull || raw is [String: Any] else {
            reply(["error": "the check's args must be a JSON object"])
            return
        }
        let args = ControlMessage(raw as? [String: Any] ?? [:])
        let hasArgs = raw is [String: Any]
        DispatchQueue.global(qos: .userInitiated).async {
            reply(link.diagnose(check: check, args: hasArgs ? args.object : nil))
        }
    }

    /// The answer to a command this server does not know
    public static func unknown(_ command: String, available: [String]) -> [String: String] {
        ["error": "Unknown command \(command); available: \(available.joined(separator: ", "))"]
    }
}
