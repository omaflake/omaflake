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
/// - Data carry their time (plan B161, C7): Mac → guest the event list's time stamp in monotonic nanoseconds (0:
///   now); guest → Mac the guest's time mapped onto the Mac's clock (`MIDIClockMapper`) and handed to CoreMIDI as the
///   event list's time stamp, so notes keep their spacing. The guest is untrusted – what it sends is checked in
///   `AgentLink` (whole UMP messages, rate limit) and goes only to endpoints of the current list.
/// - MIDI 2.0 devices (macOS 15+) bring what they say of themselves – CoreMIDI's UMP endpoint and its function
///   blocks – as `ump` in the list; the list is sent again when that changes.
///
/// CoreMIDI calls the client's notifications on the run loop of the thread that created the bridge: create it on a
/// thread whose run loop runs (the main thread with NSApplication or `RunLoop.main.run()`).
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
    private var clock = MIDIClockMapper()
    // On `queue`
    private var connected: [UInt32: (endpoint: MIDIEndpointRef, protocol: UInt8)] = [:]
    private var list: [MIDIEndpointInfo] = []
    private var rescanPending = false
    private var pings: DispatchSourceTimer?
    private var observers: [NSObjectProtocol] = []

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
        // A MIDI 2.0 device that comes, goes or changes its function blocks: the list again
        for name in [MIDIUMPEndpointManager.endpointWasAddedNotification, MIDIUMPEndpointManager.endpointWasRemovedNotification,
                     MIDIUMPEndpointManager.endpointWasUpdatedNotification, MIDIUMPEndpointManager.functionBlockWasUpdatedNotification] {
            observers.append(NotificationCenter.default.addObserver(forName: name, object: nil, queue: nil) { [weak self] _ in
                self?.scheduleRescan()
            })
        }
    }

    deinit {
        for o in observers { NotificationCenter.default.removeObserver(o) }
    }

    /// The bridge for a VM, or nil (logged) when CoreMIDI refuses – the VM then runs without MIDI
    static func start(vmName: String, log: @escaping @Sendable (String) -> Void) -> MIDIBridge? {
        do {
            return try MIDIBridge(vmName: vmName, log: log)
        } catch {
            log("MIDI: \(omaf_runner.describe(error)) – the VM runs without MIDI")
            return nil
        }
    }

    /// Wires the bridge to an agent link (`run` and `bridge` alike): what the guest plays comes here, and once a
    /// welcome enabled "midi" (call from the link's `onWelcome`) the guest gets the list
    func attach(_ link: AgentLink) {
        link.onMIDI = { [weak self] records in self?.fromGuest(records) }
    }

    /// For the link's `onWelcome`: MIDI enabled – the guest gets the list (again after each new hello)
    func welcomed(_ features: Set<String>, link: AgentLink) {
        if features.contains("midi") { useAgent(link) }
    }

    private func check(_ status: OSStatus, _ what: String) throws {
        guard status == noErr else { throw VMError("\(what): OSStatus \(status)") }
    }

    /// The guest's agent carries MIDI (its welcome enabled it, again after each new hello): it gets the whole list,
    /// and the round trip is measured once a second from now on
    func useAgent(_ link: AgentLink) {
        // A new hello may be a new boot of the guest: its clock starts over
        lock.withLock {
            self.link = link
            clock = MIDIClockMapper()
        }
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
        // MIDI 2.0 devices: what they say of themselves, matched by their source or destination
        let ump = Self.umpInfo()
        next = next.map { e in
            var e = e
            if e.id != Self.vmPortID, let ref = (e.direction == "in" ? sources[e.id]?.0 : dests[e.id]?.0) { e.ump = ump[ref] }
            return e
        }
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

    /// CoreMIDI's UMP endpoints (macOS 15+), by their source and destination, within the spec's limits
    private static func umpInfo() -> [MIDIEndpointRef: MIDIUMPInfo] {
        var out: [MIDIEndpointRef: MIDIUMPInfo] = [:]
        for u in MIDIUMPEndpointManager.shared.umpEndpoints {
            let d = u.deviceInfo
            let m = d.manufacturerID.sysExIDByte, r = d.revisionLevel.revisionLevel
            let info = MIDIUMPInfo(
                name: u.name, productInstanceId: u.productInstanceID,
                manufacturer: MIDIUMPInfo.hex([m.0, m.1, m.2]), family: d.family, model: d.modelNumber,
                version: MIDIUMPInfo.hex([r.0, r.1, r.2, r.3]), staticBlocks: u.hasStaticFunctionBlocks,
                blocks: u.functionBlocks.map { b in
                    MIDIUMPInfo.Block(id: b.functionBlockID, name: b.name, direction: direction(b.direction),
                                      firstGroup: b.firstGroup, groups: b.totalGroupsSpanned, midi1: midi1(b.midi1Info),
                                      uiHint: uiHint(b.uiHint))
                }).limited()
            for ref in [u.midiSource, u.midiDestination] where ref != 0 { out[ref] = info }
        }
        return out
    }

    private static func direction(_ d: MIDIUMPFunctionBlockDirection) -> String {
        switch d {
        case .input: "in"
        case .output: "out"
        default: "both"   // bidirectional, or unknown: both, so nothing is hidden
        }
    }

    private static func midi1(_ m: MIDIUMPFunctionBlockMIDI1Info) -> String {
        switch m {
        case .unrestrictedBandwidth: "yes"
        case .restrictedBandwidth: "low-speed"
        default: "no"
        }
    }

    private static func uiHint(_ h: MIDIUMPFunctionBlockUIHint) -> String {
        switch h {
        case .receiver: "receiver"
        case .sender: "sender"
        case .senderReceiver: "both"
        default: "unknown"
        }
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

    /// CoreMIDI's thread: a source of the Mac (or the VM's destination) sent something – straight to the guest, with
    /// the time it arrived (the event list's time stamp; 0 means now)
    private func fromMac(_ list: UnsafePointer<MIDIEventList>, id: UInt32) {
        guard let link = lock.withLock({ link }) else { return }
        let (proto, packets) = Self.packets(list)
        guard proto == 1 || proto == 2 else { return }
        let now = Self.nanoseconds(mach_absolute_time())
        // Packets of the same time go in one record
        var records: [MIDIRecord] = []
        for (stamp, words) in packets where !words.isEmpty {
            let time = stamp == 0 ? now : Self.nanoseconds(stamp)
            if let last = records.last, last.time == time {
                records[records.count - 1].words += words
            } else {
                records.append(MIDIRecord(id: id, protocol: proto, time: time, words: words))
            }
        }
        for payload in MIDIRecord.encode(records) { link.send(.midiData, payload: payload) }
    }

    /// The link's thread: what the guest's apps played, to the Mac's destination (or out of the VM's source) – at the
    /// guest's time mapped onto the Mac's clock, or now when it sent none
    func fromGuest(_ records: [MIDIRecord]) {
        for r in records {
            var stamp: MIDITimeStamp = 0
            if let time = r.time {
                let now = mach_absolute_time()
                let due = lock.withLock { clock.due(time: time, now: Self.nanoseconds(now)) }
                stamp = max(now, Self.ticks(due))
            }
            if r.id == Self.vmPortID {
                Self.eventLists(r, timeStamp: stamp) { MIDIReceivedEventList(vmSource, $0) }
            } else if let dest = lock.withLock({ destinations[r.id] }) {
                Self.eventLists(r, timeStamp: stamp) { MIDISendEventList(output, dest, $0) }
            }
            // else: an endpoint that is gone (or never was) – dropped
        }
    }

    // MARK: Clock

    /// Host ticks (mach_absolute_time, CoreMIDI's MIDITimeStamp) per nanosecond: numer / denom
    private static let timebase: mach_timebase_info_data_t = {
        var t = mach_timebase_info_data_t()
        mach_timebase_info(&t)
        return t
    }()

    /// Host ticks as monotonic nanoseconds (the clock of `DispatchTime.uptimeNanoseconds`)
    static func nanoseconds(_ ticks: UInt64) -> UInt64 {
        let t = timebase
        return t.numer == t.denom ? ticks : UInt64((UInt128(ticks) * UInt128(t.numer)) / UInt128(t.denom))
    }

    /// Monotonic nanoseconds as host ticks
    static func ticks(_ nanoseconds: UInt64) -> UInt64 {
        let t = timebase
        return t.numer == t.denom ? nanoseconds : UInt64((UInt128(nanoseconds) * UInt128(t.denom)) / UInt128(t.numer))
    }

    // MARK: Event lists

    /// The protocol and the packets of an event list – each packet's time stamp and its whole messages (packets
    /// follow each other with `wordCount` words each)
    static func packets(_ list: UnsafePointer<MIDIEventList>) -> (UInt8, [(MIDITimeStamp, [UInt32])]) {
        let raw = UnsafeRawPointer(list)
        let proto = raw.loadUnaligned(as: Int32.self)
        let count = raw.loadUnaligned(fromByteOffset: 4, as: UInt32.self)
        var packet = raw + 8
        var out: [(MIDITimeStamp, [UInt32])] = []
        for _ in 0..<count {
            let stamp = packet.loadUnaligned(as: UInt64.self)
            // Not capped at the 64 words MIDIEventPacket declares: CoreMIDI puts messages of the same time stamp
            // into one packet, which may be longer (seen: 192 words of SysEx)
            let n = Int(packet.loadUnaligned(fromByteOffset: 8, as: UInt32.self))
            var words = (0..<n).map { packet.loadUnaligned(fromByteOffset: 12 + 4 * $0, as: UInt32.self) }
            words.removeLast(words.count - MIDIRecord.wholeMessages(words))
            out.append((stamp, words))
            packet += 12 + 4 * n
        }
        return (UInt8(truncatingIfNeeded: proto), out)
    }

    /// The protocol and all words of an event list
    static func words(_ list: UnsafePointer<MIDIEventList>) -> (UInt8, [UInt32]) {
        let (proto, packets) = packets(list)
        return (proto, packets.flatMap(\.1))
    }

    /// A record as event lists with the time stamp `timeStamp` (host ticks; 0: now), one message per add; a full
    /// list is passed on and started again
    static func eventLists(_ r: MIDIRecord, timeStamp: MIDITimeStamp = 0, _ body: (UnsafePointer<MIDIEventList>) -> Void) {
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
                let next: UnsafeMutablePointer<MIDIEventPacket>? = MIDIEventListAdd(list, size, packet, timeStamp, n, buf.baseAddress! + i)
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
