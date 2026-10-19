// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation
import Testing
@testable import OmaflakeCore

/// The agent protocol (spec §11.9): frames shared with the Rust agent, and the Mac side's link.
struct AgentProtocolTests {
    static let vectorsFile = URL(fileURLWithPath: #filePath)
        .deletingLastPathComponent().deletingLastPathComponent().deletingLastPathComponent()
        .appendingPathComponent("nix/agent/testdata/vectors.json")

    static func bytes(_ hex: String) -> [UInt8] {
        stride(from: 0, to: hex.count, by: 2).map {
            let i = hex.index(hex.startIndex, offsetBy: $0)
            return UInt8(hex[i..<hex.index(i, offsetBy: 2)], radix: 16)!
        }
    }

    @Test func sharedVectors() throws {
        let v = try JSONSerialization.jsonObject(with: Data(contentsOf: Self.vectorsFile)) as! [String: Any]
        for case let f as [String: Any] in v["frames"] as! [Any] {
            let payload = Data((f["payload"] as! String).utf8)
            let type = UInt16(f["type"] as! Int), flags = UInt16(f["flags"] as! Int)
            let wire = Self.bytes(f["hex"] as! String)
            #expect(Array(AgentProtocol.encode(type, flags: flags, payload: payload)) == wire, "\(f["name"]!)")
            var d = AgentDecoder()
            d.feed(wire)
            #expect(d.next() == AgentFrame(type: type, flags: flags, payload: payload))
            #expect(d.next() == nil)
        }
        for case let s as [String: Any] in v["streams"] as! [Any] {
            var d = AgentDecoder()
            d.feed(Self.bytes(s["hex"] as! String))
            var types: [Int] = []
            while let f = d.next() { types.append(Int(f.type)) }
            #expect(types == s["types"] as! [Int], "\(s["name"]!)")
            #expect(d.errors == s["errors"] as! Int, "\(s["name"]!)")
        }
    }

    @Test func versionAgreement() {
        #expect(AgentProtocol.agree(agentMin: 1, agentMax: 1) == 1)
        #expect(AgentProtocol.agree(agentMin: 1, agentMax: 9) == AgentProtocol.current)
        #expect(AgentProtocol.agree(agentMin: 99, agentMax: 100) == nil)
        #expect(AgentProtocol.agree(agentMin: 0, agentMax: 0) == nil)
    }

    /// A socket pair: the test plays the guest on one end, AgentLink reads the other
    final class Harness {
        /// The guest's end; -1 once closed. Each descriptor is closed exactly once: tests run in parallel, and a
        /// second close could hit a file another test has just opened under the same number (plan B105)
        private(set) var guest: Int32
        let mac: Int32
        let link: AgentLink
        let stateFile: URL

        init(allowed: Set<String> = ["status"]) throws {
            var fds: [Int32] = [0, 0]
            #expect(socketpair(AF_UNIX, SOCK_STREAM, 0, &fds) == 0)
            guest = fds[0]
            mac = fds[1]
            stateFile = FileManager.default.temporaryDirectory.appendingPathComponent("agent-\(UUID().uuidString).json")
            link = AgentLink(fd: fds[1], stateFile: stateFile, statusInterval: 7, allowed: allowed)
        }

        func send(_ type: AgentProtocol.Message, _ json: String) -> Bool {
            send(type, Data(json.utf8))
        }

        func send(_ type: AgentProtocol.Message, _ payload: Data) -> Bool {
            let frames = AgentProtocol.encodeMessage(type.rawValue, payload: payload)
            return link.receive(ArraySlice(frames))
        }

        /// What the Mac wrote back to the guest
        func reply() -> [AgentFrame] {
            var buf = [UInt8](repeating: 0, count: 4096)
            var flags = fcntl(guest, F_GETFL)
            flags |= O_NONBLOCK
            _ = fcntl(guest, F_SETFL, flags)
            let n = read(guest, &buf, buf.count)
            var d = AgentDecoder()
            if n > 0 { d.feed(buf[0..<n]) }
            var frames: [AgentFrame] = []
            while let f = d.next() { frames.append(f) }
            return frames
        }

        func closeGuest() {
            if guest >= 0 { close(guest) }
            guest = -1
        }

        deinit {
            closeGuest()
            close(mac)
        }
    }

    static let hello = #"{"agent":"0.1.0","protocol":{"min":1,"max":1},"os":"linux","osName":"NixOS\u001b[31m","features":["status"]}"#

    @Test func helloWelcomeStatus() throws {
        let h = try Harness()
        #expect(h.send(.status, #"{"uptime":1}"#))   // before the hello: ignored
        #expect(AgentState.load(h.stateFile) == nil)
        #expect(h.send(.hello, Self.hello))
        let replies = h.reply()
        #expect(replies.count == 1 && replies.first?.type == AgentProtocol.Message.welcome.rawValue)
        let welcome = try JSONDecoder().decode(AgentWelcome.self, from: replies[0].payload)
        #expect(welcome.protocol == 1 && welcome.statusInterval == 7)
        var state = try #require(AgentState.load(h.stateFile))
        #expect(state.isReady && !state.agentIsOlder)
        #expect(state.hello?.osName == "NixOS[31m")   // control characters from the guest are dropped
        #expect(h.send(.status, #"{"uptime":12.5,"load":[0.5,-1,0.1],"memory":{"total":4,"available":2},"addresses":[],"users":["alice"]}"#))
        state = try #require(AgentState.load(h.stateFile))
        #expect(state.status?.uptime == 12.5)
        #expect(state.status?.load == [0.5, 0.1])     // negative numbers are dropped
        #expect(state.status?.users == ["alice"])
    }

    @Test func noCommonVersion() throws {
        let h = try Harness()
        #expect(h.send(.hello, #"{"agent":"9.0","protocol":{"min":7,"max":8}}"#))
        #expect(h.reply().isEmpty)
        let state = try #require(AgentState.load(h.stateFile))
        #expect(!state.isReady)
        #expect(state.problem?.contains("7–8") == true)
    }

    @Test func goodbyeEndsReadiness() throws {
        let h = try Harness()
        #expect(h.send(.hello, Self.hello))
        #expect(h.send(.goodbye, #"{"reason":"stopping"}"#))
        #expect(AgentState.load(h.stateFile)?.isReady == false)
    }

    @Test func garbageSpendsTheBudget() throws {
        let h = try Harness()
        var ok = true
        for _ in 0...AgentLink.errorBudget where ok {
            ok = h.link.receive(ArraySlice(Array("junk".utf8)))
        }
        #expect(!ok)
        #expect(AgentState.load(h.stateFile)?.problem != nil)
    }

    /// Large messages go in chunks and come back whole; a message over its limit is dropped with its tail (B106)
    @Test func chunksAndReassembly() throws {
        let big = Data((0..<(AgentProtocol.chunk * 2 + 100)).map { UInt8($0 % 251) })
        var d = AgentDecoder()
        d.feed(AgentProtocol.encodeMessage(AgentProtocol.Message.clipboard.rawValue, payload: big))
        var r = AgentReassembler()
        var frames = 0, whole: Data?
        while let f = d.next() {
            frames += 1
            if let m = r.push(f, limit: big.count) { whole = m }
        }
        #expect(frames == 3 && whole == big)
        d.feed(AgentProtocol.encodeMessage(AgentProtocol.Message.clipboard.rawValue, payload: big))
        while let f = d.next() { #expect(r.push(f, limit: AgentProtocol.chunk) == nil) }
        #expect(r.dropped == 1)
        d.feed(AgentProtocol.encodeMessage(AgentProtocol.Message.clipboard.rawValue, payload: Data("small".utf8)))
        let next = d.next()
        let small = try #require(next)
        #expect(r.push(small, limit: AgentProtocol.chunk) == Data("small".utf8))
    }

    /// The welcome enables what the agent offers and this VM allows; other features' messages are dropped
    @Test func featuresAndClipboard() throws {
        let offering = #"{"agent":"0.2.0","protocol":{"min":1,"max":1},"features":["status","clipboard","camera"]}"#
        let got = Box<(String, Data)>()
        let h = try Harness(allowed: ["clipboard"])
        h.link.onClipboard = { got.value = ($0, $1) }
        #expect(h.send(.hello, offering))
        let welcome = try JSONDecoder().decode(AgentWelcome.self, from: try #require(h.reply().first).payload)
        #expect(welcome.features == ["clipboard", "status"])
        #expect(h.link.enabled == ["clipboard", "status"])
        let content = Data(String(repeating: "x", count: 600_000).utf8)   // more than two chunks
        let message = try JSONSerialization.data(withJSONObject: ["mime": "text/plain;charset=utf-8", "data": content.base64EncodedString()])
        #expect(h.send(.clipboard, message))
        #expect(got.value?.0 == "text/plain;charset=utf-8" && got.value?.1 == content)
        // the camera was not allowed for this VM: its requests go nowhere
        let camera = Box<AgentLink.CameraRequest>()
        h.link.onCamera = { camera.value = $0 }
        #expect(h.send(.cameraControl, #"{"action":"start"}"#))
        #expect(camera.value == nil)
        // other kinds of content are refused
        got.value = nil
        let html = try JSONSerialization.data(withJSONObject: ["mime": "text/html", "data": "aGk="])
        #expect(h.send(.clipboard, html))
        #expect(got.value == nil)
    }

    @Test func cameraCreditsAreCapped() throws {
        let offering = #"{"agent":"0.2.0","protocol":{"min":1,"max":1},"features":["status","camera"]}"#
        let h = try Harness(allowed: ["camera"])
        let requests = Box<[AgentLink.CameraRequest]>()
        requests.value = []
        h.link.onCamera = { requests.value?.append($0) }
        #expect(h.send(.hello, offering))
        #expect(h.send(.cameraControl, #"{"action":"start"}"#))
        #expect(h.send(.cameraCredit, #"{"credits":1000}"#))
        #expect(h.send(.cameraCredit, #"{"credits":-3}"#))
        #expect(requests.value == [.start, .credits(AgentProtocol.maxCameraCredits)])
    }

    /// A camera frame (1.4 MB) goes out as one frame, not in chunks – the guest writes it to /dev/video42 whole
    @Test func cameraFramesAreNotChunked() throws {
        let h = try Harness(allowed: ["camera"])
        let frame = Data(repeating: 7, count: AgentProtocol.cameraFrameBytes)
        let sender = Thread { _ = h.link.send(.cameraFrame, payload: frame) }
        sender.start()
        var d = AgentDecoder()
        var buf = [UInt8](repeating: 0, count: 1 << 20)
        var got: AgentFrame?
        let deadline = Date().addingTimeInterval(5)
        while got == nil, Date() < deadline {
            let n = read(h.guest, &buf, buf.count)
            if n > 0 { d.feed(buf[0..<n]) }
            got = d.next()
        }
        #expect(got?.type == AgentProtocol.Message.cameraFrame.rawValue)
        #expect(got?.flags == 0 && got?.payload.count == AgentProtocol.cameraFrameBytes)
    }

    @Test func runEndsWhenThePortCloses() throws {
        let h = try Harness()
        h.closeGuest()
        #expect(h.link.run() == .closed)
    }
}

/// A value set from a callback (tests only)
final class Box<T>: @unchecked Sendable {
    var value: T?
}
