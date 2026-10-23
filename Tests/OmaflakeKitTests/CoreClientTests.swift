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

    init(respond: @escaping @Sendable (Instance, [String: Any]) -> Void) {
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
                            instance.result(request["id"]!, ["apiVersion": 1, "omafVersion": "0.0.0-test", "dataFolder": "/tmp/data"])
                        case "events.subscribe", "shutdown":
                            instance.result(request["id"]!, NSNull())
                        default:
                            self.respond(instance, request)
                        }
                    }
                }
            }.start()
            return CoreClient.Connection(toServer: toServer.fileHandleForWriting, fromServer: fromServer.fileHandleForReading,
                                         terminate: { try? fromServer.fileHandleForWriting.close() })
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

    /// Waits (up to 5 s) until an event matches
    func wait(_ what: String, _ match: (ServerEvent) -> Bool) async throws {
        for _ in 0..<500 {
            if all.contains(where: match) { return }
            try await Task.sleep(for: .milliseconds(10))
        }
        Issue.record("no \(what) event")
    }
}

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
        let server = FakeServer { s, r in
            s.send(["jsonrpc": "2.0", "method": "message", "params": ["text": "hello", "level": "info"]])
            s.send(["jsonrpc": "2.0", "method": "vms.changed", "params": ["vms": [["name": "a", "kind": "macos", "status": "running"]], "unreadable": []]])
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
        let result = try await Task.detached { try c.callBlocking("packages.inspect", ["path": "/x"], as: Inspect.self) }.value
        #expect(result.valid && result.kind == "vm")
        c.stop()
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
