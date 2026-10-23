// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

/// The client of omaflake's Rust core: starts `omaf serve` and talks JSON-RPC 2.0 with it over stdio, one JSON object
/// per line (docs/api.md). Requests run concurrently and are matched to their responses by id; the server's
/// notifications come as `events`. When the server ends on its own, open requests fail and the client starts it again
/// (`.disconnected`, then `.connected`); `stop()` ends it for good.
public final class CoreClient: @unchecked Sendable {
    /// The API version this client speaks
    public static let apiVersion = 1

    /// One running server: where requests go, where responses come from, how to end it
    public struct Connection: Sendable {
        public let toServer: FileHandle
        public let fromServer: FileHandle
        public let terminate: @Sendable () -> Void

        public init(toServer: FileHandle, fromServer: FileHandle, terminate: @escaping @Sendable () -> Void) {
            self.toServer = toServer
            self.fromServer = fromServer
            self.terminate = terminate
        }
    }

    /// Starts a server; `ended` is called (once or more) when it is gone, with why
    public typealias Launcher = @Sendable (_ ended: @escaping @Sendable (String) -> Void) throws -> Connection

    public let events: AsyncStream<ServerEvent>
    private let eventSink: AsyncStream<ServerEvent>.Continuation
    private let launcher: Launcher
    private let initialize: InitializeParams
    private let topics: [String]
    /// Pauses before each new start after the server ended; the last one repeats
    private let restartDelays: [Duration]

    private let lock = NSLock()
    private enum Phase { case idle, connecting, ready, down, stopped }
    private var phase = Phase.idle
    private var connection: Connection?
    private var generation = 0
    private var nextID = 1
    private var pending: [Int: (generation: Int, continuation: CheckedContinuation<Data, Error>)] = [:]
    private var waitingForReady: [CheckedContinuation<Void, Error>] = []
    private var failures = 0
    /// What `initialize` answered last
    public private(set) var session: InitializeResult? {
        get { lock.withLock { _session } }
        set { lock.withLock { _session = newValue } }
    }
    private var _session: InitializeResult?

    public init(launcher: @escaping Launcher, initialize: InitializeParams, topics: [String] = ServerEvent.topics,
                restartDelays: [Duration] = [.seconds(1), .seconds(2), .seconds(5), .seconds(15), .seconds(30)]) {
        // Writing to a server that just ended must be an error, not SIGPIPE ending the app
        signal(SIGPIPE, SIG_IGN)
        self.launcher = launcher
        self.initialize = initialize
        self.topics = topics
        self.restartDelays = restartDelays.isEmpty ? [.seconds(1)] : restartDelays
        (events, eventSink) = AsyncStream.makeStream(of: ServerEvent.self, bufferingPolicy: .unbounded)
    }

    // MARK: Starting and stopping

    /// Starts the server (in the background); requests sent meanwhile wait until it is initialized
    public func start() {
        let go = lock.withLock { () -> Bool in
            guard phase == .idle || phase == .down else { return false }
            phase = .connecting
            return true
        }
        if go { Task.detached { await self.connect() } }
    }

    /// Ends the server for good (the app quits): `shutdown`, then its stdin closes – it ends either way (api.md §1).
    /// Open requests fail. Does not wait.
    public func stop() {
        let (conn, open, waiters) = lock.withLock { () -> (Connection?, [CheckedContinuation<Data, Error>], [CheckedContinuation<Void, Error>]) in
            let wasReady = phase == .ready
            phase = .stopped
            let c = connection
            connection = nil
            let open = pending.values.map(\.continuation)
            pending = [:]
            let waiters = waitingForReady
            waitingForReady = []
            if wasReady, let c {
                let id = nextID
                nextID += 1
                try? c.toServer.write(contentsOf: Data(#"{"jsonrpc":"2.0","id":\#(id),"method":"shutdown"}"#.utf8) + Data([0x0A]))
            }
            return (c, open, waiters)
        }
        try? conn?.toServer.close()
        let gone = APIError.serverGone("omaflake's core was stopped")
        open.forEach { $0.resume(throwing: gone) }
        waiters.forEach { $0.resume(throwing: gone) }
        eventSink.finish()
        if let conn {
            // It ends when its stdin closes; a server that hangs is ended after a moment
            DispatchQueue.global().asyncAfter(deadline: .now() + 3) { conn.terminate() }
        }
    }

    private func connect() async {
        let gen: Int
        let conn: Connection
        do {
            let next = lock.withLock { generation + 1 }
            conn = try launcher { [weak self] reason in self?.lost(generation: next, reason) }
            let stopped = lock.withLock { () -> Bool in
                guard phase != .stopped else { return true }
                generation = next
                connection = conn
                return false
            }
            if stopped { conn.terminate(); return }
            gen = next
            startReading(conn, generation: gen)
        } catch {
            failed("omaflake's core could not be started: \(error.localizedDescription)", generation: nil)
            return
        }
        do {
            let result: InitializeResult = try await request("initialize", initialize, generation: gen)
            guard result.apiVersion == Self.apiVersion else {
                throw APIError(message: "omaflake's core speaks API \(result.apiVersion), this app \(Self.apiVersion)")
            }
            let _: Ignored = try await request("events.subscribe", ["topics": topics], generation: gen)
            let waiters = lock.withLock { () -> [CheckedContinuation<Void, Error>]? in
                guard generation == gen, phase == .connecting else { return nil }
                phase = .ready
                failures = 0
                _session = result
                defer { waitingForReady = [] }
                return waitingForReady
            }
            guard let waiters else { return }
            eventSink.yield(.connected(result))
            waiters.forEach { $0.resume() }
        } catch {
            conn.terminate()
            failed("\(error.localizedDescription)", generation: gen)
        }
    }

    /// The server is gone (its process ended, or its output closed)
    private func lost(generation gen: Int, _ reason: String) {
        failed(reason, generation: gen)
    }

    /// Fails what waits, reports it and starts again after a pause (unless stopped)
    private func failed(_ reason: String, generation gen: Int?) {
        let state = lock.withLock { () -> (open: [CheckedContinuation<Data, Error>], waiters: [CheckedContinuation<Void, Error>], delay: Duration)? in
            if let gen, gen != generation { return nil }   // an older server
            guard phase != .stopped, phase != .down else { return nil }
            phase = .down
            connection = nil
            let open = pending.filter { gen == nil || $0.value.generation == gen }
            for id in open.keys { pending.removeValue(forKey: id) }
            let waiters = waitingForReady
            waitingForReady = []
            let delay = restartDelays[min(failures, restartDelays.count - 1)]
            failures += 1
            return (open.values.map(\.continuation), waiters, delay)
        }
        guard let state else { return }
        let error = APIError.serverGone(reason)
        state.open.forEach { $0.resume(throwing: error) }
        state.waiters.forEach { $0.resume(throwing: error) }
        eventSink.yield(.disconnected(reason))
        Task.detached {
            try? await Task.sleep(for: state.delay)
            self.start()
        }
    }

    // MARK: Requests

    /// A request with params; its result decoded as `R` (`Ignored` when it does not matter)
    public func call<P: Encodable & Sendable, R: Decodable>(_ method: String, _ params: P, as: R.Type = R.self) async throws -> R {
        try await waitUntilReady()
        return try await request(method, params, generation: nil)
    }

    /// A request without params
    public func call<R: Decodable>(_ method: String, as: R.Type = R.self) async throws -> R {
        try await waitUntilReady()
        return try await request(method, Optional<[String: String]>.none, generation: nil)
    }

    /// For code that cannot wait asynchronously (an open panel's delegate): the same request, blocking this thread
    /// for at most `timeout`. Never call it from inside an event or response handler of this client.
    public func callBlocking<P: Encodable & Sendable, R: Decodable & Sendable>(_ method: String, _ params: P, as: R.Type = R.self,
                                                                             timeout: TimeInterval = 10) throws -> R {
        let box = ResultBox<R>()
        let done = DispatchSemaphore(value: 0)
        Task.detached {
            do { box.set(.success(try await self.call(method, params, as: R.self))) } catch { box.set(.failure(error)) }
            done.signal()
        }
        guard done.wait(timeout: .now() + timeout) == .success, let result = box.get() else {
            throw APIError(message: "omaflake's core did not answer \(method) in time")
        }
        return try result.get()
    }

    private func waitUntilReady() async throws {
        try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
            let outcome = lock.withLock { () -> Result<Void, Error>? in
                switch phase {
                case .ready: return .success(())
                case .stopped: return .failure(APIError.serverGone("omaflake's core was stopped"))
                case .down: return .failure(APIError.serverGone("omaflake's core is not running – it is started again"))
                case .idle, .connecting:
                    waitingForReady.append(c)
                    return nil
                }
            }
            if let outcome { c.resume(with: outcome) }
        }
    }

    /// Sends one request on the current server (`generation`: only on that one) and decodes its response
    private func request<P: Encodable, R: Decodable>(_ method: String, _ params: P?, generation gen: Int?) async throws -> R {
        let line = try await withCheckedThrowingContinuation { (c: CheckedContinuation<Data, Error>) in
            let failure = lock.withLock { () -> Error? in
                guard let conn = connection, phase != .stopped, gen == nil || gen == generation else {
                    return APIError.serverGone("omaflake's core is not running")
                }
                let id = nextID
                nextID += 1
                let data: Data
                do {
                    data = try JSONEncoder.api.encode(Request(id: id, method: method, params: params)) + Data([0x0A])
                } catch {
                    return error
                }
                pending[id] = (generation, c)
                do {
                    try conn.toServer.write(contentsOf: data)
                } catch {
                    pending.removeValue(forKey: id)
                    return APIError.serverGone("omaflake's core does not take requests: \(error.localizedDescription)")
                }
                return nil
            }
            if let failure { c.resume(throwing: failure) }
        }
        let response = try JSONDecoder.api.decode(Response<R>.self, from: line)
        if let error = response.error { throw error.apiError }
        if let result = response.result { return result }
        if let null = R.self as? NullDecodable.Type, let value = null.null as? R { return value }
        throw APIError(code: APIError.Code.invalidRequest, message: "\(method): the core sent no result")
    }

    // MARK: Reading

    private func startReading(_ conn: Connection, generation gen: Int) {
        let thread = Thread { [weak self] in
            var buffer = Data()
            while true {
                let chunk = conn.fromServer.availableData
                if chunk.isEmpty { break }
                buffer.append(chunk)
                while let newline = buffer.firstIndex(of: 0x0A) {
                    let line = buffer[buffer.startIndex..<newline]
                    buffer = Data(buffer[buffer.index(after: newline)...])
                    if !line.isEmpty { self?.handle(Data(line), generation: gen) }
                }
            }
            self?.lost(generation: gen, "omaflake's core ended")
        }
        thread.name = "omaf serve reader"
        thread.start()
    }

    private func handle(_ line: Data, generation gen: Int) {
        guard let peek = try? JSONDecoder().decode(Peek.self, from: line) else {
            FileHandle.standardError.write(Data("omaflake: not JSON from the core: \(String(decoding: line.prefix(200), as: UTF8.self))\n".utf8))
            return
        }
        if let method = peek.method {
            if peek.id == nil { eventSink.yield(ServerEvent.decode(method: method, line: line)) }
            return
        }
        guard let id = peek.id else { return }
        let waiting = lock.withLock { () -> CheckedContinuation<Data, Error>? in
            guard let p = pending[id], p.generation == gen else { return nil }
            pending.removeValue(forKey: id)
            return p.continuation
        }
        waiting?.resume(returning: line)
    }

    // MARK: Finding omaf

    /// The `omaf` binary: `OMAF_BIN`; next to the app's executable (`Contents/MacOS/omaf`); for development the
    /// newer of the repository's `target/{debug,release}/omaf` above the executable (`swift run`, `.build/…`).
    public static func locateServer(bundle: Bundle = .main,
                                    environment: [String: String] = ProcessInfo.processInfo.environment) -> URL? {
        let fm = FileManager.default
        if let custom = environment["OMAF_BIN"], !custom.isEmpty { return URL(fileURLWithPath: custom) }
        if let bundled = bundle.url(forAuxiliaryExecutable: "omaf"), fm.isExecutableFile(atPath: bundled.path) { return bundled }
        var dir = bundle.executableURL?.resolvingSymlinksInPath().deletingLastPathComponent()
        for _ in 0..<8 {
            guard let d = dir else { break }
            if fm.fileExists(atPath: d.appendingPathComponent("Cargo.toml").path) {
                let built = ["debug", "release"].map { d.appendingPathComponent("target/\($0)/omaf") }
                    .filter { fm.isExecutableFile(atPath: $0.path) }
                func modified(_ url: URL) -> Date {
                    (try? url.resourceValues(forKeys: [.contentModificationDateKey]).contentModificationDate) ?? .distantPast
                }
                return built.max { modified($0) < modified($1) }
            }
            dir = d.deletingLastPathComponent()
        }
        return nil
    }

    /// Starts `<executable> serve` as a child process; its stderr goes where this process' goes
    public static func processLauncher(executable: URL, arguments: [String] = ["serve"],
                                       environment: [String: String]? = nil) -> Launcher {
        { ended in
            let process = Process()
            process.executableURL = executable
            process.arguments = arguments
            if let environment { process.environment = environment }
            let input = Pipe(), output = Pipe()
            process.standardInput = input
            process.standardOutput = output
            process.terminationHandler = { p in
                ended("omaflake's core ended (\(p.terminationReason == .uncaughtSignal ? "signal" : "status") \(p.terminationStatus))")
            }
            try process.run()
            let pid = process.processIdentifier
            return Connection(toServer: input.fileHandleForWriting, fromServer: output.fileHandleForReading,
                              terminate: { kill(pid, SIGTERM) })
        }
    }
}

/// A result that does not matter (`null`, or anything)
public struct Ignored: Decodable, Sendable, NullDecodable {
    public init() {}
    public init(from decoder: Decoder) throws {}
    public static var null: Any { Ignored() }
}

/// A result type that also stands for `null`
protocol NullDecodable {
    static var null: Any { get }
}

extension Optional: NullDecodable {
    static var null: Any { Optional<Wrapped>.none as Any }
}

private struct Request<P: Encodable>: Encodable {
    let jsonrpc = "2.0"
    let id: Int
    let method: String
    let params: P?

    enum CodingKeys: String, CodingKey { case jsonrpc, id, method, params }
    func encode(to encoder: Encoder) throws {
        var c = encoder.container(keyedBy: CodingKeys.self)
        try c.encode(jsonrpc, forKey: .jsonrpc)
        try c.encode(id, forKey: .id)
        try c.encode(method, forKey: .method)
        try c.encodeIfPresent(params, forKey: .params)
    }
}

private struct Response<R: Decodable>: Decodable {
    let result: R?
    let error: RPCError?
}

private struct Peek: Decodable {
    let id: Int?
    let method: String?
}

private final class ResultBox<R: Sendable>: @unchecked Sendable {
    private let lock = NSLock()
    private var value: Result<R, Error>?
    func set(_ v: Result<R, Error>) { lock.withLock { value = v } }
    func get() -> Result<R, Error>? { lock.withLock { value } }
}
