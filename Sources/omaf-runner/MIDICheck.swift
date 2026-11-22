// SPDX-License-Identifier: AGPL-3.0-or-later
import ArgumentParser
import CoreMIDI
import Darwin
import Foundation
import OmaflakeKit

/// `omaf-runner midi-check`: the whole MIDI path on this Mac, without a VM (plan B161) – the guest agent runs here on a
/// pseudo-terminal instead of the virtio port, the runner's bridge on the other end, and test endpoints stand in for a
/// device. Notes go Mac → guest and guest → Mac through CoreMIDI on both sides; prints what arrived and how long it
/// took. With `--device` it measures a real source instead (play a few notes on it).
struct MIDICheck: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "midi-check",
        abstract: "Check the MIDI bridge on this Mac: guest agent on a pseudo-terminal, notes both ways (development).")
    @Option(help: "The omaflake-agent binary (nix/agent, cargo build)") var agent: String
    @Option(help: "Notes per direction") var count = 200
    @Option(help: "Milliseconds between notes") var interval = 5

    func run() throws {
        let check = try Check(agent: url(agent).path, count: count, interval: interval)
        let ok = try check.run()
        if !ok { throw ExitCode(1) }
    }
}

private final class Check: @unchecked Sendable {
    let agentPath: String
    let count: Int
    let interval: Int
    let lock = NSLock()
    var received: [String: [UInt32: UInt64]] = [:]   // direction → note number → arrival (ns)
    var sysexIn: [UInt32] = []
    var midi2In: [UInt32] = []

    var client = MIDIClientRef()
    var source1 = MIDIEndpointRef(), source2 = MIDIEndpointRef(), destination = MIDIEndpointRef()
    var inPort1 = MIDIPortRef(), inPort2 = MIDIPortRef(), outPort = MIDIPortRef()

    init(agent: String, count: Int, interval: Int) throws {
        agentPath = agent
        self.count = count
        self.interval = interval
    }

    func run() throws -> Bool {
        // The "device": a MIDI 1.0 source, a MIDI 2.0 source and a destination – plain endpoints, so the bridge
        // mirrors them into the "guest"
        guard MIDIClientCreateWithBlock("omaflake check" as CFString, &client, nil) == noErr,
              MIDISourceCreateWithProtocol(client, "omaflake check 1.0" as CFString, ._1_0, &source1) == noErr,
              MIDISourceCreateWithProtocol(client, "omaflake check 2.0" as CFString, ._2_0, &source2) == noErr,
              MIDIDestinationCreateWithProtocol(client, "omaflake check out" as CFString, ._1_0, &destination, { [weak self] list, _ in
                  self?.arrived(list, direction: "guest → Mac")
              }) == noErr
        else { throw VMError("CoreMIDI: cannot create the test endpoints") }

        // The port: a pseudo-terminal – the agent opens its slave as its port, the bridge reads and writes the master
        let master = posix_openpt(O_RDWR | O_NOCTTY)
        guard master >= 0, grantpt(master) == 0, unlockpt(master) == 0, let name = ptsname(master) else {
            throw VMError("no pseudo-terminal")
        }
        let slave = String(cString: name)
        let state = FileManager.default.temporaryDirectory.appendingPathComponent("omaflake-midi-check-\(getpid()).json")
        let bridge = try MIDIBridge(vmName: "check", log: { print("  runner: \($0)") })
        let link = AgentLink(fd: master, stateFile: state, host: "omaflake check", allowed: ["status", "midi"],
                             log: { print("  runner: \($0)") })
        link.onWelcome = { [weak link] features in
            if let link, features.contains("midi") { bridge.useAgent(link) }
        }
        link.onMIDI = { bridge.fromGuest($0) }
        Thread.detachNewThread { _ = link.run() }

        let agent = Process()
        agent.executableURL = URL(fileURLWithPath: agentPath)
        agent.environment = ProcessInfo.processInfo.environment.merging(["OMAFLAKE_AGENT_PORT": slave]) { $1 }
        let log = FileManager.default.temporaryDirectory.appendingPathComponent("omaflake-midi-check-agent.log")
        FileManager.default.createFile(atPath: log.path, contents: nil)
        agent.standardError = try FileHandle(forWritingTo: log)
        try agent.run()
        defer {
            agent.terminate()
            agent.waitUntilExit()
            try? FileManager.default.removeItem(at: state)
        }
        print("agent \(agentPath) on \(slave) (log: \(log.path))")

        // The guest's mirrors of the test endpoints: same names, marked as omaflake's
        guard let guestIn1 = waitFor(source: "omaflake check 1.0"), let guestIn2 = waitFor(source: "omaflake check 2.0"),
              let guestOut = waitFor(destination: "omaflake check out")
        else {
            print("FAILED: the guest's endpoints did not appear within 10 s – see the agent's log")
            return false
        }
        print("guest endpoints: in 1.0, in 2.0, out")
        guard MIDIInputPortCreateWithProtocol(client, "check in 1" as CFString, ._1_0, &inPort1, { [weak self] list, _ in
                  self?.arrived(list, direction: "Mac → guest")
              }) == noErr,
              MIDIInputPortCreateWithProtocol(client, "check in 2" as CFString, ._2_0, &inPort2, { [weak self] list, _ in
                  self?.arrivedMIDI2(list)
              }) == noErr,
              MIDIOutputPortCreate(client, "check out" as CFString, &outPort) == noErr,
              MIDIPortConnectSource(inPort1, guestIn1, nil) == noErr,
              MIDIPortConnectSource(inPort2, guestIn2, nil) == noErr
        else { throw VMError("CoreMIDI: cannot listen to the guest's endpoints") }
        pump(0.3)

        // Notes: note number and velocity carry the note's index, so each arrival is matched to its send time
        var sent: [String: [UInt32: UInt64]] = ["Mac → guest": [:], "guest → Mac": [:]]
        for i in 0..<UInt32(count) {
            let word = 0x2090_0000 | ((i % 128) << 8) | (i / 128 + 1)
            sent["Mac → guest"]![i] = DispatchTime.now().uptimeNanoseconds
            MIDIBridge.eventLists(MIDIRecord(id: 0, protocol: 1, words: [word])) { MIDIReceivedEventList(source1, $0) }
            sent["guest → Mac"]![i] = DispatchTime.now().uptimeNanoseconds
            MIDIBridge.eventLists(MIDIRecord(id: 0, protocol: 1, words: [word])) { MIDISendEventList(outPort, guestOut, $0) }
            pump(Double(interval) / 1000)
        }
        // A SysEx of 1,000 bytes (167 UMP SysEx7 packets) and a MIDI 2.0 note with a 16-bit velocity
        let sysex = Self.sysex7(bytes: (0..<1000).map { UInt8($0 % 128) })
        MIDIBridge.eventLists(MIDIRecord(id: 0, protocol: 1, words: sysex)) { MIDIReceivedEventList(source1, $0) }
        let midi2: [UInt32] = [0x4090_3C00, 0xBEEF_0000]
        MIDIBridge.eventLists(MIDIRecord(id: 0, protocol: 2, words: midi2)) { MIDIReceivedEventList(source2, $0) }
        pump(1)

        var ok = true
        lock.lock()
        defer { lock.unlock() }
        for direction in ["Mac → guest", "guest → Mac"] {
            let arrivals = received[direction] ?? [:]
            let ms = arrivals.compactMap { i, t in sent[direction]![i].map { Double(t - $0) / 1e6 } }.sorted()
            if ms.count < count { ok = false }
            let p = { (q: Double) in ms.isEmpty ? .nan : ms[min(ms.count - 1, Int(Double(ms.count) * q))] }
            print(String(format: "%@: %d/%d notes, latency ms min %.3f · median %.3f · p99 %.3f · max %.3f",
                         direction, ms.count, count, ms.first ?? .nan, p(0.5), p(0.99), ms.last ?? .nan))
        }
        let sysexOK = sysexIn == sysex
        print("SysEx 1,000 bytes Mac → guest: \(sysexOK ? "identical" : "DIFFERENT (\(sysexIn.count) of \(sysex.count) words)")")
        let midi2OK = midi2In == midi2
        print("MIDI 2.0 note (16-bit velocity) Mac → guest: \(midi2OK ? "identical" : "DIFFERENT \(midi2In.map { String($0, radix: 16) })")")
        if let s = AgentState.load(state)?.midi?.roundTripMs {
            print(String(format: "agent round trip (ping): median %.3f ms over %d", s.median, s.samples))
        }
        return ok && sysexOK && midi2OK
    }

    /// Waits while the run loop runs: CoreMIDI tells a client about new endpoints through it
    private func pump(_ seconds: Double) {
        RunLoop.current.run(until: Date().addingTimeInterval(seconds))
    }

    private func waitFor(source name: String) -> MIDIEndpointRef? {
        waitFor(name) { (0..<MIDIGetNumberOfSources()).map(MIDIGetSource) }
    }

    private func waitFor(destination name: String) -> MIDIEndpointRef? {
        waitFor(name) { (0..<MIDIGetNumberOfDestinations()).map(MIDIGetDestination) }
    }

    private func waitFor(_ name: String, _ all: () -> [MIDIEndpointRef]) -> MIDIEndpointRef? {
        let deadline = Date().addingTimeInterval(10)
        while Date() < deadline {
            for e in all() {
                var mirror: Int32 = 0
                var value: Unmanaged<CFString>?
                guard MIDIObjectGetIntegerProperty(e, MIDIBridge.mirrorProperty, &mirror) == noErr, mirror == 1,
                      MIDIObjectGetStringProperty(e, kMIDIPropertyName, &value) == noErr,
                      (value?.takeRetainedValue() as String?) == name else { continue }
                return e
            }
            pump(0.1)
        }
        return nil
    }

    private func arrived(_ list: UnsafePointer<MIDIEventList>, direction: String) {
        let now = DispatchTime.now().uptimeNanoseconds
        let (_, words) = MIDIBridge.words(list)
        lock.withLock {
            var i = 0
            while i < words.count {
                let w = words[i]
                if w >> 28 == 0x3 {
                    sysexIn.append(contentsOf: words[i..<min(i + 2, words.count)])
                } else if w >> 28 == 0x2, (w >> 20) & 0xF == 0x9 {
                    let index = ((w & 0x7F) - 1) * 128 + ((w >> 8) & 0x7F)
                    received[direction, default: [:]][index] = now
                }
                i += MIDIRecord.messageWords(w)
            }
        }
    }

    private func arrivedMIDI2(_ list: UnsafePointer<MIDIEventList>) {
        let (_, words) = MIDIBridge.words(list)
        lock.withLock { midi2In.append(contentsOf: words) }
    }

    /// Bytes as UMP SysEx7 packets (message type 3, group 0): start, continue…, end – 6 bytes each
    static func sysex7(bytes: [UInt8]) -> [UInt32] {
        var words: [UInt32] = []
        let chunks = stride(from: 0, to: bytes.count, by: 6).map { Array(bytes[$0..<min($0 + 6, bytes.count)]) }
        for (n, chunk) in chunks.enumerated() {
            let status: UInt32 = chunks.count == 1 ? 0 : n == 0 ? 1 : n == chunks.count - 1 ? 3 : 2
            var b = chunk + [UInt8](repeating: 0, count: 6 - chunk.count)
            b = b.map { $0 & 0x7F }
            words.append(0x3000_0000 | status << 20 | UInt32(chunk.count) << 16 | UInt32(b[0]) << 8 | UInt32(b[1]))
            words.append(UInt32(b[2]) << 24 | UInt32(b[3]) << 16 | UInt32(b[4]) << 8 | UInt32(b[5]))
        }
        return words
    }
}
