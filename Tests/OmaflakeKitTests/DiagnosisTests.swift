// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation
import Testing
@testable import OmaflakeKit

/// The VM's diagnosis (plan B162): the agent link's request and answer (spec §11.9, `0x0600`/`0x0601`), the relay
/// over a control socket, and the page model (api.md "diagnostics").
struct DiagnosisTests {
    typealias Harness = AgentProtocolTests.Harness

    static let offering = #"{"agent":"0.9.0","protocol":{"min":1,"max":1},"features":["status","diagnose"]}"#

    /// A harness whose agent offered the diagnosis; the welcome is read away
    static func welcomed() throws -> Harness {
        let h = try Harness()
        #expect(h.send(.hello, offering))
        let welcome = try JSONDecoder().decode(AgentWelcome.self, from: try #require(h.reply().first).payload)
        #expect(welcome.features == ["diagnose", "status"])
        #expect(h.link.features == ["diagnose", "status"])
        return h
    }

    /// The guest reads the next diagnosis request the Mac wrote (waits up to 5 s)
    static func nextRequest(_ h: Harness) -> [String: Any]? {
        let deadline = Date().addingTimeInterval(5)
        while Date() < deadline {
            if let f = h.reply().first(where: { $0.type == AgentProtocol.Message.diagnoseRequest.rawValue }) {
                return (try? JSONSerialization.jsonObject(with: f.payload)) as? [String: Any]
            }
            usleep(10_000)
        }
        return nil
    }

    /// Runs a test body that blocks (semaphores, socket reads, the link's waits) on a thread of its own: blocked, it
    /// would hold one of the few threads of Swift's cooperative pool that the async tests running alongside need
    static func offPool(_ body: @escaping @Sendable () throws -> Void) async throws {
        try await withCheckedThrowingContinuation { (c: CheckedContinuation<Void, Error>) in
            Thread.detachNewThread {
                do { try body(); c.resume() } catch { c.resume(throwing: error) }
            }
        }
    }

    static func answer(_ object: [String: Any]) -> Data {
        try! JSONSerialization.data(withJSONObject: object)
    }

    @Test func requestAndAnswer() async throws { try await Self.offPool { try Self.requestAndAnswer() } }

    static func requestAndAnswer() throws {
        let h = try welcomed()
        let result = Box<ControlMessage>()
        let done = DispatchSemaphore(value: 0)
        let link = h.link
        Thread.detachNewThread {
            result.value = ControlMessage(link.diagnose(check: "clipboard-write", args: ["text": "token-1"], timeout: 5))
            done.signal()
        }
        let request = try #require(Self.nextRequest(h))
        #expect(request["check"] as? String == "clipboard-write")
        #expect((request["args"] as? [String: Any])?["text"] as? String == "token-1")
        let id = try #require(request["id"] as? Int)
        // Other traffic goes on meanwhile; an answer to an id nobody waits for is dropped quietly
        #expect(h.send(.status, #"{"uptime":3}"#))
        #expect(h.send(.diagnoseAnswer, Self.answer(["id": id + 7, "check": "clipboard-write", "status": "pass", "detail": ""])))
        #expect(done.wait(timeout: .now() + 0.2) == .timedOut)
        #expect(h.send(.diagnoseAnswer, Self.answer([
            "id": id, "check": "clipboard-write", "status": "pass", "detail": "written\u{1b}[31m",
            "data": ["text": "line 1\nline 2\u{7}", "level": 0.5, "ok": true, "bad": ["nested": [1, 2]]],
        ])))
        #expect(done.wait(timeout: .now() + 5) == .success)
        let answer = try #require(result.value?.object)
        #expect(answer["status"] as? String == "pass" && answer["check"] as? String == "clipboard-write")
        #expect(answer["detail"] as? String == "written[31m")   // control characters from the guest are dropped
        let data = try #require(answer["data"] as? [String: Any])
        #expect(data["text"] as? String == "line 1\nline 2")   // newlines stay
        #expect(data["level"] as? Double == 0.5 && data["ok"] as? Bool == true)
        #expect(AgentState.load(h.stateFile)?.status?.uptime == 3)
    }

    @Test func timeoutAndRefusals() async throws { try await Self.offPool { try Self.timeoutAndRefusals() } }

    static func timeoutAndRefusals() throws {
        let h = try welcomed()
        let start = Date()
        let answer = h.link.diagnose(check: "sound", timeout: 0.3)
        #expect((answer["error"] as? String)?.contains("no answer") == true)
        #expect(Date().timeIntervalSince(start) < 2)
        #expect(Self.nextRequest(h)?["check"] as? String == "sound")
        // Not a check name: never sent
        #expect(h.link.diagnose(check: "../x", timeout: 0.3)["error"] != nil)
        #expect(h.link.diagnose(check: "", timeout: 0.3)["error"] != nil)
        // An agent without the feature (an older image), and one that has not said hello
        let old = try Harness()
        #expect(old.link.diagnose(check: "info")["error"] as? String == "the guest's agent has not said hello yet")
        #expect(old.send(.hello, AgentProtocolTests.hello))
        #expect((old.link.diagnose(check: "info")["error"] as? String)?.contains("does not offer") == true)
    }

    @Test func malformedAnswersSpendTheBudget() async throws { try await Self.offPool { try Self.malformedAnswersSpendTheBudget() } }

    static func malformedAnswersSpendTheBudget() throws {
        #expect(AgentLink.diagnosis(Data(#"{"id":1,"check":"info","status":"great","detail":""}"#.utf8)) == nil)
        #expect(AgentLink.diagnosis(Data(#"{"id":-1,"check":"info","status":"pass","detail":""}"#.utf8)) == nil)
        #expect(AgentLink.diagnosis(Data(#"{"id":true,"check":"info","status":"pass","detail":""}"#.utf8)) == nil)
        #expect(AgentLink.diagnosis(Data(#"{"id":1,"check":"info","status":"pass","detail":3}"#.utf8)) == nil)
        #expect(AgentLink.diagnosis(Data(#"[1]"#.utf8)) == nil)
        #expect(AgentLink.diagnosis(Data(#"{"id":1,"check":"info","status":"skipped"}"#.utf8))?["detail"] as? String == "")
        let h = try welcomed()
        for _ in 0...AgentLink.errorBudget {
            _ = h.send(.diagnoseAnswer, #"{"id":1,"check":"info","status":"nope"}"#)
        }
        #expect(!h.send(.diagnoseAnswer, #"{"id":1}"#))
        // The Mac's own message never comes from the guest
        let other = try welcomed()
        #expect(other.send(.diagnoseRequest, #"{"id":1,"check":"info"}"#))
    }

    /// The relay (spec §11.9, "Relay"): `{"command": "diagnose", "check", "args"?}` on a control socket → the agent's
    /// answer, as `omaf-runner run` and `omaf-runner bridge --control` serve it
    @Test func relayOverTheControlSocket() async throws { try await Self.offPool { try Self.relayOverTheControlSocket() } }

    static func relayOverTheControlSocket() throws {
        let h = try welcomed()
        let socket = URL(fileURLWithPath: NSTemporaryDirectory()).appendingPathComponent("omaf-diag-\(getpid())-\(UInt16.random(in: 0...9999)).sock")
        let link = h.link
        let server = try ControlServer(path: socket, queue: .global()) { request, reply in
            switch request.object["command"] as? String ?? "" {
            case "diagnose": ControlServer.diagnose(link, request, reply: reply)
            case "status": reply(["state": "running"])
            case let other: reply(ControlServer.unknown(other, available: RunnerClient.bridgeCommands))
            }
        }
        defer { server.close() }
        // The guest: answers one request
        let guestDone = DispatchSemaphore(value: 0)
        Thread.detachNewThread {
            defer { guestDone.signal() }
            guard let request = Self.nextRequest(h), let id = request["id"] as? Int else { return }
            _ = h.send(.diagnoseAnswer, Self.answer([
                "id": id, "check": request["check"] ?? "", "status": "pass", "detail": "3 addresses",
                "data": ["addresses": ["192.168.64.5"], "dns": true, "internet": true, "echo": request["args"] ?? NSNull()],
            ]))
        }
        let answer = try RunnerClient.request(["command": "diagnose", "check": "network", "args": ["n": 1]], socket: socket, timeout: 10)
        #expect(guestDone.wait(timeout: .now() + 5) == .success)
        #expect(answer["check"] as? String == "network" && answer["status"] as? String == "pass")
        let data = try #require(answer["data"] as? [String: Any])
        #expect(data["dns"] as? Bool == true && (data["echo"] as? [String: Any])?["n"] as? Int == 1)
        // The other commands keep their string answers; mistakes answer with an error
        #expect(try RunnerClient.send("status", socket: socket) == ["state": "running"])
        #expect((try RunnerClient.request(["command": "diagnose"], socket: socket)["error"] as? String)?.contains("needs a check") == true)
        #expect(try RunnerClient.request(["command": "diagnose", "check": "info", "args": [1]], socket: socket)["error"] != nil)
        #expect((try RunnerClient.send("nope", socket: socket)["error"])?.contains("status, diagnose, midi") == true)
        #expect(RunnerClient.commands.contains("diagnose"))
    }

    static let page = """
    {"title": {"key": "Diagnose “%@”", "text": "Diagnose “research”", "args": ["research"]},
     "subtitle": {"text": "NixOS · running"}, "state": "finished", "checked": "2026-11-22T10:00:00Z",
     "intro": {"key": "x", "text": "x"}, "future": 1,
     "checks": [
       {"id": "agent", "title": {"key": "Agent", "text": "Agent"}, "icon": "vm", "status": "pass",
        "detail": {"text": "omaflake-agent 0.9.0"}, "actions": []},
       {"id": "sound", "title": {"key": "Sound", "text": "Sound"}, "icon": "sound", "status": "confirm",
        "hint": {"key": "Did you hear a tone?", "text": "Did you hear a tone?"},
        "actions": [{"id": "heard", "label": {"key": "Heard It", "text": "Heard It"}, "method": "diagnostics.confirm",
                     "params": {"name": "research", "check": "sound", "ok": true}, "enabled": true}]},
       {"id": "camera", "title": {"key": "Camera", "text": "Camera"}, "icon": "camera", "status": "skipped", "actions": []},
       {"id": "midi", "title": {"key": "MIDI", "text": "MIDI"}, "icon": "midi", "status": "sparkling", "actions": []}
     ],
     "actions": [{"id": "run", "label": {"key": "Run Again", "text": "Run Again"}, "icon": "diagnose",
                  "method": "diagnostics.run", "params": {"name": "research"}, "enabled": true, "primary": true},
                 {"id": "copy", "label": {"key": "Copy Log", "text": "Copy Log"}, "method": "diagnostics.copy",
                  "params": {"name": "research"}, "enabled": true},
                 {"id": "reveal", "label": {"key": "Show Log File", "text": "Show Log File"}, "method": "ui.reveal",
                  "params": {"path": "/data/logs/diagnostics-research-1.log"}, "enabled": true}],
     "log": {"job": "j7", "from": 0, "lines": 42}, "file": "/data/logs/diagnostics-research-1.log"}
    """

    @Test func pageModel() throws {
        guard case .diagnostics(let p) = try PageModel.decode(page: "diagnostics", from: Data(Self.page.utf8)) else {
            Issue.record("not the diagnostics page")
            return
        }
        #expect(p.state == .finished && p.checked != nil && p.subtitle?.text == "NixOS · running")
        #expect(p.checks.map(\.id) == ["agent", "sound", "camera", "midi"])
        #expect(p.checks.map(\.status) == [.pass, .confirm, .skipped, .unknown])   // an unknown status is no error
        #expect(p.checks[1].actions.first?.method == "diagnostics.confirm" && p.checks[1].hint != nil)
        #expect(p.actions.map(\.id) == ["run", "copy", "reveal"] && p.actions.primary?.id == "run")
        #expect(p.log?.job == "j7" && p.log?.lines == 42 && p.file?.hasSuffix(".log") == true)
        // The least the core may send: title, state and checks
        guard case .diagnostics(let small) = try PageModel.decode(
            page: "diagnostics", from: Data(#"{"title":{"text":"Diagnose"},"state":"idle","checks":[]}"#.utf8)) else {
            Issue.record("not the diagnostics page")
            return
        }
        #expect(small.state == .idle && small.actions.isEmpty && small.log == nil)
    }
}
