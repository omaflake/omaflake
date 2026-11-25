// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation
import Testing
@testable import OmaflakeKit

/// The agent protocol (spec §11.9): frames shared with the Rust agent, and the Mac side's link.
/// A test that hangs fails after a minute instead of holding a CI runner for six hours (plan B176)
@Suite(.timeLimit(.minutes(1)))
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

    /// MIDI records: the vectors shared with the Rust agent (plan B161)
    @Test func midiVectors() throws {
        let v = try JSONSerialization.jsonObject(with: Data(contentsOf: Self.vectorsFile)) as! [String: Any]
        for case let c as [String: Any] in v["midi"] as! [Any] {
            let wire = Data(Self.bytes(c["hex"] as! String))
            if let want = c["records"] as? [[String: Any]] {
                let records = want.map {
                    MIDIRecord(id: UInt32($0["id"] as! Int), protocol: UInt8($0["protocol"] as! Int),
                               time: ($0["time"] as? NSNumber)?.uint64Value,
                               words: ($0["words"] as! [Int]).map { UInt32($0) })
                }
                #expect(MIDIRecord.decode(wire) == records, "\(c["name"]!)")
                #expect(MIDIRecord.encode(records) == [wire], "\(c["name"]!)")
            } else {
                #expect(MIDIRecord.decode(wire) == nil, "\(c["name"]!)")
            }
        }
        // A long SysEx is split at message boundaries and comes back whole
        let words = (0..<40_000).map { $0 % 2 == 0 ? UInt32(0x3016_0000) : UInt32($0) }
        let parts = MIDIRecord.encode([MIDIRecord(id: 7, protocol: 1, words: words)])
        #expect(parts.count > 1 && parts.allSatisfy { $0.count <= AgentProtocol.maxMIDIMessage })
        #expect(parts.flatMap { MIDIRecord.decode($0)! }.flatMap(\.words) == words)
        // … and every part keeps the record's time
        let timed = MIDIRecord.encode([MIDIRecord(id: 7, protocol: 1, time: 123_456_789, words: words)])
        #expect(timed.count > 1 && timed.allSatisfy { $0.count <= AgentProtocol.maxMIDIMessage })
        let back = timed.flatMap { MIDIRecord.decode($0)! }
        #expect(back.allSatisfy { $0.time == 123_456_789 } && back.flatMap(\.words) == words)
    }

    /// The clock mapping of timestamped MIDI (spec §11.9, "Timestamps"): the smallest offset over 30 s, delivery at
    /// time + offset + 2 ms, now when past or more than 1 s ahead
    @Test func midiClockMapper() {
        let ms: UInt64 = 1_000_000
        var m = MIDIClockMapper()
        #expect(m.offset == nil)
        // The guest's clock runs 5 s behind; the first message took 1 ms: offset 5 s + 1 ms, due 2 ms after it came
        let base: UInt64 = 100_000 * ms
        #expect(m.due(time: base - 5_000 * ms, now: base + 1 * ms) == base + 3 * ms)
        // A faster message lowers the offset; a later one that waited 3 ms longer comes due on time (spacing kept)
        #expect(m.due(time: base - 4_990 * ms, now: base + 10 * ms + 500_000) == base + 10 * ms + 2_500_000)
        #expect(m.offset == Int64(5_000 * ms + 500_000))
        #expect(m.due(time: base - 4_980 * ms, now: base + 21 * ms) == base + 22 * ms + 500_000)
        // 3 ms late beyond the lead: already past – now
        #expect(m.due(time: base - 4_970 * ms, now: base + 34 * ms) == base + 34 * ms)
        // A sender's clock ahead of ours works the same (negative offset)
        var ahead = MIDIClockMapper()
        #expect(ahead.due(time: 9_000_000 * ms, now: 10 * ms) == 12 * ms)
        #expect(ahead.offset! < 0)
        // The minimum slides out after 30 s: the newest minimum behind it takes over
        var w = MIDIClockMapper()
        _ = w.due(time: 0, now: 1 * ms)                                   // offset 1 ms
        _ = w.due(time: 10_000 * ms, now: 10_000 * ms + 4 * ms)          // 4 ms
        _ = w.due(time: 20_000 * ms, now: 20_000 * ms + 3 * ms)          // 3 ms – replaces 4 ms
        #expect(w.offset == Int64(1 * ms))
        _ = w.due(time: 31_000 * ms, now: 31_000 * ms + 9 * ms)          // 1 ms is 31 s old: 3 ms takes over
        #expect(w.offset == Int64(3 * ms))
        // A jump in the sender's clock becomes the new minimum: due within the lead, never further ahead
        var j = MIDIClockMapper()
        _ = j.due(time: 0, now: 1 * ms)
        #expect(j.due(time: 5_000 * ms, now: 2 * ms) == 4 * ms)
        #expect(j.due(time: 5_001 * ms, now: 3 * ms) == 5 * ms)
    }

    /// UMP endpoint info in the endpoint list: the spec's field names, limits applied, absent when there is none
    @Test func midiUMPInfo() throws {
        let long = String(repeating: "ä", count: 60)   // 120 bytes
        let blocks = (0..<40).map {
            MIDIUMPInfo.Block(id: UInt8($0), name: long, direction: "both", firstGroup: 15, groups: 4, midi1: "no", uiHint: "both")
        }
        let info = MIDIUMPInfo(name: long, productInstanceId: "SN1", manufacturer: MIDIUMPInfo.hex([0x00, 0x21, 0x3B]),
                               family: 2, model: 3, version: MIDIUMPInfo.hex([1, 0, 0, 0]), staticBlocks: true,
                               blocks: blocks).limited()
        #expect(info.name.utf8.count == 98 && info.blocks.count == 32 && info.blocks.allSatisfy { $0.name.utf8.count <= 98 })
        #expect(info.blocks.allSatisfy { $0.id <= 31 && $0.groups == 1 })
        let e = MIDIEndpointInfo(id: 1, direction: "in", name: "Piano", manufacturer: nil, model: nil, protocol: 2, ump: info)
        let obj = try JSONSerialization.jsonObject(with: JSONEncoder().encode(e)) as! [String: Any]
        let ump = obj["ump"] as! [String: Any]
        #expect(Set(ump.keys) == ["name", "productInstanceId", "manufacturer", "family", "model", "version", "staticBlocks", "blocks"])
        #expect(ump["manufacturer"] as? String == "00213b" && ump["version"] as? String == "01000000")
        let block = (ump["blocks"] as! [[String: Any]])[0]
        #expect(Set(block.keys) == ["id", "name", "direction", "firstGroup", "groups", "midi1", "uiHint"])
        let plain = MIDIEndpointInfo(id: 1, direction: "in", name: "Piano", manufacturer: nil, model: nil, protocol: 1)
        let plainObj = try JSONSerialization.jsonObject(with: JSONEncoder().encode(plain)) as! [String: Any]
        #expect(plainObj["ump"] == nil)
    }

    /// MIDI from the guest only once enabled, malformed data cost budget, the status lands in agent.json, a pong
    /// gives a round trip
    @Test func midiThroughTheLink() throws {
        let offering = #"{"agent":"0.8.0","protocol":{"min":1,"max":1},"features":["status","midi"]}"#
        let note = MIDIRecord.encode([MIDIRecord(id: 42, protocol: 1, words: [0x2090_3C64])])[0]
        let got = Box<[MIDIRecord]>()
        got.value = []

        let off = try Harness()   // MIDI not switched on for this VM
        off.link.onMIDI = { got.value?.append(contentsOf: $0) }
        #expect(off.send(.hello, offering))
        #expect(off.send(.midiData, note))
        #expect(got.value == [])

        let h = try Harness(allowed: ["midi"])
        h.link.onMIDI = { got.value?.append(contentsOf: $0) }
        #expect(h.send(.hello, offering))
        #expect(h.send(.midiData, note))
        #expect(got.value == [MIDIRecord(id: 42, protocol: 1, words: [0x2090_3C64])])
        #expect(h.send(.midiData, Data([1, 2, 3])))   // malformed: dropped, one error
        #expect(got.value?.count == 1)
        // A timestamped record keeps its time; unknown flags are malformed
        let timed = MIDIRecord(id: 42, protocol: 1, time: 5_000_000_000, words: [0x2080_3C00])
        #expect(h.send(.midiData, MIDIRecord.encode([timed])[0]))
        #expect(got.value?.last == timed)
        var flagged = MIDIRecord.encode([MIDIRecord(id: 42, protocol: 1, words: [0x2090_3C64])])[0]
        flagged[5] = 0x02
        #expect(h.send(.midiData, flagged))
        #expect(got.value?.count == 2)
        #expect(h.send(.midiStatus, #"{"endpoints":3,"failed":["Piano\u0007"]}"#))
        let state = try #require(AgentState.load(h.stateFile))
        #expect(state.midi?.endpoints == 3 && state.midi?.failed == ["Piano"])
        #expect(state.features == ["midi", "status"])
        let sent = DispatchTime.now().uptimeNanoseconds - 2_000_000
        #expect(h.send(.midiPong, withUnsafeBytes(of: sent.littleEndian) { Data($0) }))
        #expect(h.send(.status, #"{"uptime":2}"#))
        let rtt = try #require(AgentState.load(h.stateFile)?.midi?.roundTripMs)
        #expect(rtt.samples == 1 && rtt.median >= 2 && rtt.median < 1000)
        // the Mac's own messages from the guest are errors
        #expect(h.send(.midiEndpoints, #"{"endpoints":[]}"#))
    }

    /// More than the burst at once: the rest is dropped (a guest flooding the Mac's devices)
    @Test func midiRateLimit() throws {
        let offering = #"{"agent":"0.8.0","protocol":{"min":1,"max":1},"features":["status","midi"]}"#
        let h = try Harness(allowed: ["midi"])
        let count = Box<Int>()
        count.value = 0
        h.link.onMIDI = { _ in count.value! += 1 }
        #expect(h.send(.hello, offering))
        let big = MIDIRecord.encode([MIDIRecord(id: 1, protocol: 1, words: Array(repeating: 0x2090_3C64, count: 16_000))])[0]
        for _ in 0..<20 { _ = h.send(.midiData, big) }
        let accepted = count.value!
        #expect(accepted >= 7 && accepted <= 9)   // 512 KiB burst ≈ 8 of 64 KB, plus what refilled meanwhile
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
