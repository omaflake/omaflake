// SPDX-License-Identifier: AGPL-3.0-or-later
import CoreMIDI
import Foundation
import OmaflakeKit

/// Mac side of MIDI (spec §11.9, plan B161): every MIDI endpoint of the Mac – a keyboard, a stage piano, a synthesizer,
/// Bluetooth or network MIDI, another app's virtual port – appears in the guest as a virtual endpoint of the same
/// name, and the data go both ways over the omaflake agent as Universal MIDI Packets. Nothing is captured: the Mac's
/// apps keep using the devices, and devices plugged in or out while the VM runs come and go in the guest.
///
/// - Each source is read on an input port of its own protocol (MIDI 1.0 or 2.0), so the data reach the guest as the
///   device sends them – no conversion on the way; the guest's virtual endpoint has the same protocol.
/// - Per VM one more pair on the Mac, "omaflake <VM>": what the Mac's apps play to it comes out of "omaflake Mac" in
///   the guest, what the guest plays to "omaflake Mac" comes out of it on the Mac.
/// - Endpoints carrying `mirrorProperty` (ours, and those of other omaflake VMs) are never mirrored: no loops.
/// - Data are sent as they arrive (timestamp "now"); the guest is untrusted – what it sends is checked in `AgentLink`
///   (whole UMP messages, rate limit) and goes only to endpoints of the current list.
final class MIDIBridge: @unchecked Sendable {
    /// Set to 1 on omaflake's own endpoints (here and in the guest), so no omaflake mirrors them again
    nonisolated(unsafe) static let mirrorProperty = "app.omaflake.mirror" as CFString   // immutable constant
    /// The id of the VM's own pair in the list (a CoreMIDI unique id is never 0)
    static let vmPortID: UInt32 = 0

    private let vmName: String
    private let log: @Sendable (String) -> Void
    private let queue = DispatchQueue(label: "omaflake.midi")
    private let lock = NSLock()

    private var client = MIDIClientRef()
    private var inputs: [UInt8: MIDIPortRef] = [:]
    private var output = MIDIPortRef()
    private var vmSource = MIDIEndpointRef()
    private var vmDestination = MIDIEndpointRef()

    // Under `lock`: read on CoreMIDI's and the link's threads
    private var link: AgentLink?
    private var destinations: [UInt32: MIDIEndpointRef] = [:]
    // On `queue`
    private var connected: [UInt32: (endpoint: MIDIEndpointRef, protocol: UInt8)] = [:]
    private var list: [MIDIEndpointInfo] = []
    private var rescanPending = false
    private var pings: DispatchSourceTimer?

    init(vmName: String, log: @escaping @Sendable (String) -> Void) throws {
        self.vmName = vmName
        self.log = log
        try check(MIDIClientCreateWithBlock("omaflake \(vmName)" as CFString, &client) { [weak self] message in
            if message.pointee.messageID == .msgSetupChanged { self?.scheduleRescan() }
        }, "MIDI client")
        for proto: UInt8 in [1, 2] {
            var port = MIDIPortRef()
            let id = MIDIProtocolID(rawValue: Int32(proto)) ?? ._1_0
            try check(MIDIInputPortCreateWithProtocol(client, "omaflake in \(proto)" as CFString, id, &port) { [weak self] list, refCon in
                self?.fromMac(list, id: UInt32(truncatingIfNeeded: UInt(bitPattern: refCon)))
            }, "MIDI input port")
            inputs[proto] = port
        }
        try check(MIDIOutputPortCreate(client, "omaflake out" as CFString, &output), "MIDI output port")
        // The VM's own pair, MIDI 2.0 (CoreMIDI converts for apps that speak 1.0)
        let name = "omaflake \(vmName)" as CFString
        try check(MIDISourceCreateWithProtocol(client, name, ._2_0, &vmSource), "MIDI source")
        try check(MIDIDestinationCreateWithProtocol(client, name, ._2_0, &vmDestination) { [weak self] list, _ in
            self?.fromMac(list, id: Self.vmPortID)
        }, "MIDI destination")
        for endpoint in [vmSource, vmDestination] {
            MIDIObjectSetIntegerProperty(endpoint, Self.mirrorProperty, 1)
            MIDIObjectSetStringProperty(endpoint, kMIDIPropertyManufacturer, "omaflake" as CFString)
        }
    }

    private func check(_ status: OSStatus, _ what: String) throws {
        guard status == noErr else { throw VMError("\(what): OSStatus \(status)") }
    }

    /// The guest's agent carries MIDI (its welcome enabled it, again after each new hello): it gets the whole list,
    /// and the round trip is measured once a second from now on
    func useAgent(_ link: AgentLink) {
        lock.withLock { self.link = link }
        queue.async { [self] in
            rescan(forceSend: true)
            if pings == nil {
                let t = DispatchSource.makeTimerSource(queue: queue)
                t.schedule(deadline: .now() + 1, repeating: 1)
                t.setEventHandler { [weak self] in
                    guard let self, let link = lock.withLock({ self.link }) else { return }
                    link.sendMIDIPing()
                }
                t.resume()
                pings = t
            }
        }
    }

    /// The current list, for `omaf-runner control … midi`
    func describe() -> [String: String] {
        queue.sync {
            [
                "endpoints": "\(list.count)",
                "in": list.filter { $0.direction == "in" }.map(\.name).joined(separator: ", "),
                "out": list.filter { $0.direction == "out" }.map(\.name).joined(separator: ", "),
                "agent": lock.withLock { link != nil } ? "connected" : "waiting for the guest's agent",
            ]
        }
    }

    // MARK: Endpoints (on `queue`)

    private func scheduleRescan() {
        queue.async { [self] in
            guard !rescanPending else { return }
            rescanPending = true
            // Several notifications come for one device; look once they settled
            queue.asyncAfter(deadline: .now() + 0.3) { [self] in
                rescanPending = false
                rescan(forceSend: false)
            }
        }
    }

    private func rescan(forceSend: Bool) {
        var sources: [UInt32: (MIDIEndpointRef, MIDIEndpointInfo)] = [:]
        var dests: [UInt32: (MIDIEndpointRef, MIDIEndpointInfo)] = [:]
        for i in 0..<MIDIGetNumberOfSources() {
            let e = MIDIGetSource(i)
            if let info = Self.info(e, direction: "in") { sources[info.id] = (e, info) }
        }
        for i in 0..<MIDIGetNumberOfDestinations() {
            let e = MIDIGetDestination(i)
            if let info = Self.info(e, direction: "out") { dests[info.id] = (e, info) }
        }
        // Sources: listen to new ones on the port of their protocol, stop listening to gone or changed ones
        for (id, old) in connected where sources[id].map({ $0.0 != old.endpoint || $0.1.protocol != old.protocol }) ?? true {
            if let port = inputs[old.protocol] { MIDIPortDisconnectSource(port, old.endpoint) }
            connected[id] = nil
        }
        for (id, (e, info)) in sources where connected[id] == nil {
            guard let port = inputs[info.protocol] else { continue }
            let status = MIDIPortConnectSource(port, e, UnsafeMutableRawPointer(bitPattern: UInt(id)))
            if status == noErr { connected[id] = (e, info.protocol) } else { log("MIDI: cannot listen to \"\(info.name)\": OSStatus \(status)") }
        }
        lock.withLock { destinations = dests.mapValues(\.0) }

        let vmName = "omaflake Mac"
        var next = [
            MIDIEndpointInfo(id: Self.vmPortID, direction: "in", name: vmName, manufacturer: "omaflake", model: nil, protocol: 2),
            MIDIEndpointInfo(id: Self.vmPortID, direction: "out", name: vmName, manufacturer: "omaflake", model: nil, protocol: 2),
        ]
        next += sources.values.map(\.1).filter { connected[$0.id] != nil }.sorted { ($0.name, $0.id) < ($1.name, $1.id) }
        next += dests.values.map(\.1).sorted { ($0.name, $0.id) < ($1.name, $1.id) }
        guard forceSend || next != list else { return }
        if next != list {
            let names = next.dropFirst(2).map { "\($0.direction == "in" ? "→" : "←") \($0.name)" }
            log("MIDI: \(names.isEmpty ? "no devices" : names.joined(separator: ", "))")
        }
        list = next
        guard let link = lock.withLock({ link }),
              let json = try? JSONEncoder().encode(["endpoints": next]) else { return }
        link.send(.midiEndpoints, payload: json)
    }

    /// An endpoint worth mirroring: online, not omaflake's own, with a unique id
    private static func info(_ e: MIDIEndpointRef, direction: String) -> MIDIEndpointInfo? {
        guard integer(e, mirrorProperty) != 1, integer(e, kMIDIPropertyOffline) != 1,
              let unique = integer(e, kMIDIPropertyUniqueID), unique != 0 else { return nil }
        let name = string(e, kMIDIPropertyDisplayName) ?? string(e, kMIDIPropertyName) ?? "MIDI"
        return MIDIEndpointInfo(id: UInt32(bitPattern: unique), direction: direction, name: name,
                                manufacturer: string(e, kMIDIPropertyManufacturer), model: string(e, kMIDIPropertyModel),
                                protocol: integer(e, kMIDIPropertyProtocolID) == 2 ? 2 : 1)
    }

    private static func integer(_ obj: MIDIObjectRef, _ key: CFString) -> Int32? {
        var value: Int32 = 0
        return MIDIObjectGetIntegerProperty(obj, key, &value) == noErr ? value : nil
    }

    private static func string(_ obj: MIDIObjectRef, _ key: CFString) -> String? {
        var value: Unmanaged<CFString>?
        guard MIDIObjectGetStringProperty(obj, key, &value) == noErr, let s = value?.takeRetainedValue() else { return nil }
        return s as String
    }

    // MARK: Data

    /// CoreMIDI's thread: a source of the Mac (or the VM's destination) sent something – straight to the guest
    private func fromMac(_ list: UnsafePointer<MIDIEventList>, id: UInt32) {
        guard let link = lock.withLock({ link }) else { return }
        let (proto, words) = Self.words(list)
        guard proto == 1 || proto == 2, !words.isEmpty else { return }
        for payload in MIDIRecord.encode([MIDIRecord(id: id, protocol: proto, words: words)]) {
            link.send(.midiData, payload: payload)
        }
    }

    /// The link's thread: what the guest's apps played, to the Mac's destination (or out of the VM's source)
    func fromGuest(_ records: [MIDIRecord]) {
        for r in records {
            if r.id == Self.vmPortID {
                Self.eventLists(r) { MIDIReceivedEventList(vmSource, $0) }
            } else if let dest = lock.withLock({ destinations[r.id] }) {
                Self.eventLists(r) { MIDISendEventList(output, dest, $0) }
            }
            // else: an endpoint that is gone (or never was) – dropped
        }
    }

    /// The protocol and all words of an event list (packets follow each other with `wordCount` words each)
    static func words(_ list: UnsafePointer<MIDIEventList>) -> (UInt8, [UInt32]) {
        let raw = UnsafeRawPointer(list)
        let proto = raw.loadUnaligned(as: Int32.self)
        let count = raw.loadUnaligned(fromByteOffset: 4, as: UInt32.self)
        var packet = raw + 8
        var words: [UInt32] = []
        for _ in 0..<count {
            let n = min(Int(packet.loadUnaligned(fromByteOffset: 8, as: UInt32.self)), 64)
            for i in 0..<n { words.append(packet.loadUnaligned(fromByteOffset: 12 + 4 * i, as: UInt32.self)) }
            packet += 12 + 4 * n
        }
        words.removeLast(words.count - MIDIRecord.wholeMessages(words))
        return (UInt8(truncatingIfNeeded: proto), words)
    }

    /// A record as event lists (timestamp 0: now), one message per add; a full list is passed on and started again
    static func eventLists(_ r: MIDIRecord, _ body: (UnsafePointer<MIDIEventList>) -> Void) {
        let size = 16 * 1024
        let raw = UnsafeMutableRawPointer.allocate(byteCount: size, alignment: 8)
        defer { raw.deallocate() }
        let list = raw.bindMemory(to: MIDIEventList.self, capacity: 1)
        let proto = MIDIProtocolID(rawValue: Int32(r.protocol)) ?? ._1_0
        var packet = MIDIEventListInit(list, proto)
        r.words.withUnsafeBufferPointer { buf in
            var i = 0
            while i < buf.count {
                let n = MIDIRecord.messageWords(buf[i])
                guard i + n <= buf.count else { break }
                let next: UnsafeMutablePointer<MIDIEventPacket>? = MIDIEventListAdd(list, size, packet, 0, n, buf.baseAddress! + i)
                if let next {
                    packet = next
                    i += n
                } else {
                    body(list)
                    packet = MIDIEventListInit(list, proto)
                }
            }
        }
        if list.pointee.numPackets > 0 { body(list) }
    }
}
