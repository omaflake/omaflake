// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation
import Testing
@testable import OmaflakeKit

/// A stand-in for `omaf serve` on pipes: it reads one JSON request per line and answers through `respond`, which may
/// send anything (responses out of order, notifications) or end the server.
final class FakeServer: @unchecked Sendable {
    struct Instance: @unchecked Sendable {
        let output: FileHandle
        let ended: @Sendable (String) -> Void

        func send(_ object: [String: Any]) {
            let data = try! JSONSerialization.data(withJSONObject: object)
            try? output.write(contentsOf: data + Data([0x0A]))
        }

        func result(_ id: Any, _ result: Any) { send(["jsonrpc": "2.0", "id": id, "result": result]) }

        func die() {
            try? output.close()
            ended("fake server ended")
        }
    }

    private let lock = NSLock()
    private var _launches = 0
    private var _requests: [[String: Any]] = []
    var launches: Int { lock.withLock { _launches } }
    var requests: [[String: Any]] { lock.withLock { _requests } }
    /// Handles one request; `initialize` and `events.subscribe` are answered before it is asked
    let respond: @Sendable (Instance, [String: Any]) -> Void
    /// `initialize`'s `core` (plan B165); nil: left out, as an older core does
    nonisolated(unsafe) let core: [String: Any]?
    /// Hands the connection out as the service's on this socket path (plan B165)
    let socket: String?

    init(core: [String: Any]? = nil, socket: String? = nil, respond: @escaping @Sendable (Instance, [String: Any]) -> Void) {
        self.core = core
        self.socket = socket
        self.respond = respond
    }

    var launcher: CoreClient.Launcher {
        { [self] ended in
            let toServer = Pipe(), fromServer = Pipe()
            let instance = Instance(output: fromServer.fileHandleForWriting, ended: ended)
            lock.withLock { _launches += 1 }
            let input = toServer.fileHandleForReading
            Thread {
                var buffer = Data()
                while true {
                    let chunk = input.availableData
                    if chunk.isEmpty { break }
                    buffer.append(chunk)
                    while let nl = buffer.firstIndex(of: 0x0A) {
                        let line = Data(buffer[buffer.startIndex..<nl])
                        buffer = Data(buffer[buffer.index(after: nl)...])
                        guard let request = try? JSONSerialization.jsonObject(with: line) as? [String: Any] else { continue }
                        self.lock.withLock { self._requests.append(request) }
                        switch request["method"] as? String {
                        case "initialize":
                            var result: [String: Any] = ["apiVersion": 1, "omafVersion": "0.0.0-test", "dataFolder": "/tmp/data",
                                                         "configFile": "/tmp/data/config.json",
                                                         "host": ["system": "aarch64-darwin", "os": "macOS 27.0"]]
                            if let core = self.core { result["core"] = core }
                            instance.result(request["id"]!, result)
                        case "events.subscribe", "shutdown":
                            instance.result(request["id"]!, NSNull())
                        default:
                            self.respond(instance, request)
                        }
                    }
                }
            }.start()
            return CoreClient.Connection(toServer: toServer.fileHandleForWriting, fromServer: fromServer.fileHandleForReading,
                                         socket: socket, terminate: { try? fromServer.fileHandleForWriting.close() })
        }
    }
}

/// Collects a client's events for the tests
final class EventLog: @unchecked Sendable {
    private let lock = NSLock()
    private var events: [ServerEvent] = []

    init(_ client: CoreClient) {
        Task.detached { for await e in client.events { self.lock.withLock { self.events.append(e) } } }
    }

    var all: [ServerEvent] { lock.withLock { events } }

    /// Waits (up to `seconds`) until an event matches
    func wait(_ what: String, seconds: Int = 5, _ match: (ServerEvent) -> Bool) async throws {
        for _ in 0..<(seconds * 100) {
            if all.contains(where: match) { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        Issue.record("no \(what) event")
    }
}

/// A test that hangs fails after a minute instead of holding a CI runner for six hours (plan B176)
@Suite(.timeLimit(.minutes(1)))
struct CoreClientTests {
    static let params = InitializeParams(client: "omaflake-macos", clientVersion: "0.3.0", language: "de",
                                         updateFeed: "https://x/update.json")

    func client(_ server: FakeServer) -> CoreClient {
        CoreClient(launcher: server.launcher, initialize: Self.params, restartDelays: [.milliseconds(50)])
    }

    @Test func initializesSubscribesAndWaitsWithEarlyRequests() async throws {
        let server = FakeServer { s, r in s.result(r["id"]!, ["vms": [], "unreadable": []]) }
        let c = client(server)
        let events = EventLog(c)
        // Sent before the server runs: it waits for `initialize`
        async let early: VmList = c.call("vms.list")
        c.start()
        let list = try await early
        #expect(list.vms.isEmpty)
        try await events.wait("connected") { if case .connected(let r) = $0 { r.dataFolder == "/tmp/data" } else { false } }
        let methods = server.requests.compactMap { $0["method"] as? String }
        #expect(Array(methods.prefix(2)) == ["initialize", "events.subscribe"])
        let initialize = try #require(server.requests.first?["params"] as? [String: Any])
        #expect(initialize["client"] as? String == "omaflake-macos" && initialize["apiVersion"] as? Int == 1)
        #expect(initialize["language"] as? String == "de" && initialize["updateFeed"] as? String == "https://x/update.json")
        #expect(initialize["screen"] == nil)   // the core reads the screen size itself
        let topics = (server.requests[1]["params"] as? [String: Any])?["topics"] as? [String]
        #expect(topics == ServerEvent.topics)
        #expect(server.requests.allSatisfy { $0["jsonrpc"] as? String == "2.0" })
        c.stop()
    }

    @Test func responsesInAnotherOrderFindTheirRequest() async throws {
        let slow = Slow()
        let server = FakeServer { s, r in
            switch r["method"] as? String {
            case "test.slow": slow.hold(r["id"]!)
            case "test.fast":
                s.result(r["id"]!, ["name": "fast"])
                if let id = slow.take() { s.result(id, ["name": "slow"]) }
            default: s.send(["jsonrpc": "2.0", "id": r["id"]!, "error": ["code": -32601, "message": "unknown method"]])
            }
        }
        let c = client(server)
        c.start()
        struct Named: Decodable, Sendable { let name: String }
        async let a: Named = c.call("test.slow", ["n": 1])
        try await Task.sleep(for: .milliseconds(100))   // the slow one is sent first
        async let b: Named = c.call("test.fast", ["n": 2])
        let (first, second) = try await (a, b)
        #expect(first.name == "slow" && second.name == "fast")
        await #expect(throws: APIError.self) { let _: Ignored = try await c.call("nope") }
        c.stop()
    }

    @Test func errorsCarryTheirKind() async throws {
        let server = FakeServer { s, r in
            s.send(["jsonrpc": "2.0", "id": r["id"]!,
                    "error": ["code": -32000, "message": "running – stop it first", "data": ["kind": "vm-running"]]])
        }
        let c = client(server)
        c.start()
        do {
            let _: Ignored = try await c.call("vms.remove", ["name": "research"])
            Issue.record("no error")
        } catch let error as APIError {
            #expect(error.kind == .vmRunning && error.message == "running – stop it first")
        }
        let sent = try #require(server.requests.last)
        #expect((sent["params"] as? [String: Any])?["name"] as? String == "research")
        c.stop()
    }

    @Test func notificationsBecomeEvents() async throws {
        // A VM as the server sends it (the golden file of api::contract), turned into a macOS guest
        var vm = try #require(try JSONSerialization.jsonObject(with: Data(contentsOf: APIContractTests.golden("vms.get"))) as? [String: Any])
        vm["kind"] = "macos"
        nonisolated(unsafe) let changed = ["vms": [vm], "unreadable": []] as [String: Any]
        let server = FakeServer { s, r in
            s.send(["jsonrpc": "2.0", "method": "message", "params": ["text": "hello", "level": "info"]])
            s.send(["jsonrpc": "2.0", "method": "vms.changed", "params": changed])
            s.result(r["id"]!, NSNull())
        }
        let c = client(server)
        let events = EventLog(c)
        c.start()
        let _: Ignored = try await c.call("test.poke")
        try await events.wait("message") { if case .message("hello", .info) = $0 { true } else { false } }
        try await events.wait("vms.changed") { if case .vmsChanged(let l) = $0 { l.vms.first?.isMac == true } else { false } }
        c.stop()
    }

    @Test func aServerThatDiesIsStartedAgain() async throws {
        let server = FakeServer { s, r in
            if r["method"] as? String == "test.die" { s.die() } else { s.result(r["id"]!, ["ok": true]) }
        }
        let c = client(server)
        let events = EventLog(c)
        c.start()
        struct OK: Decodable, Sendable { let ok: Bool }
        #expect(try await c.call("test.ping", as: OK.self).ok)
        do {
            let _: Ignored = try await c.call("test.die")
            Issue.record("the request outlived its server")
        } catch let error as APIError {
            #expect(error.code == APIError.Code.serverGone)
        }
        try await events.wait("disconnected") { if case .disconnected = $0 { true } else { false } }
        try await events.wait("second connected") { _ in events.all.filter { if case .connected = $0 { true } else { false } }.count >= 2 }
        #expect(server.launches == 2)
        #expect(try await c.call("test.ping", as: OK.self).ok)
        c.stop()
        await #expect(throws: APIError.self) { let _: Ignored = try await c.call("test.ping") }
    }

    @Test func blockingCallsForPanelDelegates() async throws {
        let server = FakeServer { s, r in s.result(r["id"]!, ["kind": "vm", "valid": true]) }
        let c = client(server)
        c.start()
        struct Inspect: Decodable, Sendable { let kind: String; let valid: Bool }
        // From a thread of its own, as from the main thread of an open panel: a task of the cooperative pool must not
        // block – with few cores (CI) the pool has no thread left for the request it waits for, and every test hangs
        let result = try await withCheckedThrowingContinuation { (k: CheckedContinuation<Inspect, Error>) in
            Thread { k.resume(with: Result { try c.callBlocking("packages.inspect", ["path": "/x"], as: Inspect.self) }) }.start()
        }
        #expect(result.valid && result.kind == "vm")
        c.stop()
    }

    // MARK: Which core (plan B165)

    static let own = CoreIdentity(executable: "/Applications/omaflake.app/Contents/MacOS/omaf", size: 1000,
                                  modified: "2026-11-22T10:00:00Z")
    nonisolated(unsafe) static let older: [String: Any] = ["executable": "/Applications/omaflake.app/Contents/MacOS/omaf", "size": 900,
                                       "modified": "2026-11-01T10:00:00Z"]

    @Test func coreIdentityComparison() throws {
        let own = Self.own
        #expect(own.isSame(as: own))
        // A core before the check says nothing: a different one (that is the case to restart)
        #expect(!own.isSame(as: nil))
        var other = own
        other.size = 999
        #expect(!own.isSame(as: other))
        other = own
        other.executable = "/usr/local/bin/omaf"
        #expect(!own.isSame(as: other))
        other = own
        other.modified = "2026-11-22T10:00:01Z"
        #expect(!own.isSame(as: other))
        // The same second, written with other fractions (Rust's nanoseconds, Foundation's milliseconds)
        other.modified = "2026-11-22T10:00:00.123456789Z"
        var mine = own
        mine.modified = "2026-11-22T10:00:00.120Z"
        #expect(mine.isSame(as: other))
        other.modified = "2026-11-22T11:00:00+01:00"
        #expect(own.isSame(as: other))
        // Unreadable times are compared as they are
        other.modified = "yesterday"
        #expect(!own.isSame(as: other))
    }

    @Test func coreIdentityOfAFileResolvesLinks() throws {
        let dir = FileManager.default.temporaryDirectory.appendingPathComponent("omaflake-core-\(UUID().uuidString)")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        let file = dir.appendingPathComponent("omaf")
        try Data(repeating: 1, count: 42).write(to: file)
        let link = dir.appendingPathComponent("link")
        try FileManager.default.createSymbolicLink(at: link, withDestinationURL: file)
        let a = try #require(CoreIdentity.of(executable: file))
        let b = try #require(CoreIdentity.of(executable: link))
        #expect(a.size == 42 && a == b && a.isSame(as: b))
        #expect(CoreIdentity.of(executable: dir.appendingPathComponent("missing")) == nil)
        // A core's answer with a path through a link is the same file
        var viaLink = a
        viaLink.executable = link.path
        #expect(a.isSame(as: viaLink))
        try? FileManager.default.removeItem(at: dir)   // the test's own temporary folder
    }

    @Test func initializeCarriesTheCore() throws {
        let json = #"{"apiVersion":1,"omafVersion":"0.7.4","host":{"system":"aarch64-darwin","os":"macOS 27"},"dataFolder":"/d","configFile":"/d/c.json","core":{"executable":"/x/omaf","size":12,"modified":"2026-11-22T10:00:00.5Z"}}"#
        let r = try JSONDecoder.api.decode(InitializeResult.self, from: Data(json.utf8))
        #expect(r.core == CoreIdentity(executable: "/x/omaf", size: 12, modified: "2026-11-22T10:00:00.5Z"))
    }

    /// A service of another omaf: asked to restart once; when it ended, the client connects again – and a core that
    /// comes back the same (an agent of another omaf) is used as it is, not asked again
    @Test func anotherCoreIsAskedToRestartOnce() async throws {
        let server = FakeServer(core: Self.older, socket: "/nonexistent-omaflake-test/service.sock") { s, r in
            if r["method"] as? String == "service.restart" {
                s.result(r["id"]!, ["jobs": 2])
                // It ends once its jobs are done – after its answer has been read
                DispatchQueue.global().asyncAfter(deadline: .now() + 0.2) { s.die() }
            } else {
                s.result(r["id"]!, NSNull())
            }
        }
        let c = CoreClient(launcher: server.launcher, initialize: Self.params, restartDelays: [.milliseconds(50)],
                           ownCore: Self.own, handoverPolls: [.milliseconds(10)])
        let events = EventLog(c)
        c.start()
        try await events.wait("waiting") { if case .coreHandover(.waiting(jobs: 2)) = $0 { true } else { false } }
        try await events.wait("kept") { if case .coreHandover(.kept) = $0 { true } else { false } }
        #expect(server.launches == 2)
        #expect(server.requests.filter { $0["method"] as? String == "service.restart" }.count == 1)
        c.stop()
    }

    @Test func theSameCoreOrAPrivateServerIsLeftAlone() async throws {
        let own: [String: Any] = ["executable": Self.own.executable, "size": Self.own.size, "modified": Self.own.modified]
        for server in [FakeServer(core: own, socket: "/nonexistent-omaflake-test/service.sock") { s, r in s.result(r["id"]!, NSNull()) },
                       FakeServer(core: Self.older, socket: nil) { s, r in s.result(r["id"]!, NSNull()) }] {
            let c = CoreClient(launcher: server.launcher, initialize: Self.params, restartDelays: [.milliseconds(50)],
                               ownCore: Self.own)
            let events = EventLog(c)
            c.start()
            let _: Ignored = try await c.call("test.ping")
            try await Task.sleep(for: .milliseconds(100))
            #expect(!server.requests.contains { $0["method"] as? String == "service.restart" })
            #expect(!events.all.contains { if case .coreHandover = $0 { true } else { false } })
            c.stop()
        }
    }

    @Test func waitingForTheOldSocketEnds() async throws {
        let c = CoreClient(launcher: { _ in throw APIError(message: "unused") }, initialize: Self.params,
                           handoverPolls: [.milliseconds(10)])
        #expect(await c.waitUntilGone("/nonexistent-omaflake-test/service.sock"))
        // A socket that still answers: bounded
        let path = "/tmp/omf-test-\(UInt32.random(in: 0...UInt32.max)).sock"
        let fd = try listenUnix(path)
        defer { close(fd); unlink(path) }
        #expect(CoreClient.answers(path))
        #expect(await c.waitUntilGone(path, limit: .milliseconds(100)) == false)
    }

    @Test func findsTheServerFromOMAF_BIN() {
        let url = CoreClient.locateServer(environment: ["OMAF_BIN": "/opt/omaf"])
        #expect(url?.path == "/opt/omaf")
    }
}

/// Holds back the id of the slow request
final class Slow: @unchecked Sendable {
    private let lock = NSLock()
    private var id: Any?
    func hold(_ id: Any) { lock.withLock { self.id = id } }
    func take() -> Any? { lock.withLock { defer { id = nil }; return id } }
}

/// A Unix socket that accepts connections (the kernel queues them; nobody answers) – an old service still there
private func listenUnix(_ path: String) throws -> Int32 {
    let fd = socket(AF_UNIX, SOCK_STREAM, 0)
    var addr = sockaddr_un()
    addr.sun_family = sa_family_t(AF_UNIX)
    let bytes = Array(path.utf8)
    guard fd >= 0, bytes.count < MemoryLayout.size(ofValue: addr.sun_path) else { throw APIError(message: "socket \(path)") }
    withUnsafeMutableBytes(of: &addr.sun_path) { raw in
        raw.copyBytes(from: bytes)
        raw[bytes.count] = 0
    }
    addr.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
    let rc = withUnsafePointer(to: &addr) {
        $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(fd, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
    }
    guard rc == 0, listen(fd, 128) == 0 else { close(fd); throw APIError(message: "bind \(path)") }
    return fd
}
