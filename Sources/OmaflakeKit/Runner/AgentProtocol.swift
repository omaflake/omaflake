// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

/// The omaflake agent protocol (spec §11.9): frames over the virtio port org.omaflake.agent. The guest side is
/// nix/agent (Rust); both check the vectors in nix/agent/testdata/vectors.json.
///
/// Frame: 12-byte header, little endian – magic "OMFA", type u16, flags u16 (bit 0: payload continues in the next
/// frame of the same type), payload length u32 (at most `maxPayload`) – then the payload. The protocol version is
/// agreed once per connection: the guest's hello names the range it speaks, the Mac's welcome the version chosen.
public enum AgentProtocol {
    public static let portName = "org.omaflake.agent"
    public static let magic: [UInt8] = Array("OMFA".utf8)
    public static let headerLength = 12
    public static let maxPayload = 2 * 1024 * 1024
    public static let flagMore: UInt16 = 1
    /// Large messages go in chunks of at most this size (a control message waits for one chunk at most)
    public static let chunk = 256 * 1024
    /// Clipboard: at most 32 MiB of content, as JSON with base64 a third more
    public static let maxClipboard = 32 * 1024 * 1024
    public static let maxClipboardMessage = maxClipboard / 3 * 4 + 1024
    /// Camera: 1280×720 NV12, and the frames the guest may grant at once (spec §11.9, credits)
    public static let cameraFrameBytes = 1280 * 720 * 3 / 2
    public static let maxCameraCredits = 4

    /// Versions the Mac side speaks: the current one and, once there is one, the previous one (spec §11.9)
    public static let supported: ClosedRange<Int> = 1...1
    public static var current: Int { supported.upperBound }

    public enum Message: UInt16, Sendable {
        case hello = 0x0001     // guest → Mac
        case welcome = 0x0002   // Mac → guest
        case status = 0x0003    // guest → Mac
        case goodbye = 0x0004   // guest → Mac
        case clipboard = 0x0100       // both: {"mime", "data" (base64)} in chunks
        case cameraControl = 0x0200   // guest → Mac: {"action": "start" | "stop"}
        case cameraCredit = 0x0201    // guest → Mac: {"credits": n}
        case cameraFrame = 0x0202     // Mac → guest: one NV12 frame
        case cameraStatus = 0x0203    // Mac → guest: {"status", "detail"}
        case midiEndpoints = 0x0500   // Mac → guest: {"endpoints": [...]}, the whole list after each change
        case midiData = 0x0501        // both: MIDI records (`MIDIRecord`)
        case midiStatus = 0x0502      // guest → Mac: {"endpoints": n, "failed": [names]}
        case midiPing = 0x0503        // Mac → guest: at most `maxMIDIPing` bytes, answered at once
        case midiPong = 0x0504        // guest → Mac: the ping's payload unchanged
    }

    /// MIDI: one data message at most (never chunked), a ping's payload at most
    public static let maxMIDIMessage = 64 * 1024
    public static let maxMIDIPing = 64
    /// MIDI from the guest: bytes a second it may send on average, and the burst above that (a SysEx dump is fine,
    /// a guest flooding the Mac's devices is cut off)
    public static let midiRate = 1024 * 1024
    public static let midiBurst = 512 * 1024

    /// A message as frames: one frame, or chunks of `chunk` bytes with MORE on all but the last
    public static func encodeMessage(_ type: UInt16, payload: Data) -> Data {
        guard payload.count > chunk else { return encode(type, payload: payload) }
        var out = Data()
        var start = payload.startIndex
        while start < payload.endIndex {
            let end = min(start + chunk, payload.endIndex)
            out.append(encode(type, flags: end < payload.endIndex ? flagMore : 0, payload: payload[start..<end]))
            start = end
        }
        return out
    }

    /// The version both sides speak (the highest), nil if their ranges do not overlap
    public static func agree(agentMin: Int, agentMax: Int) -> Int? {
        let v = min(agentMax, supported.upperBound)
        return v >= max(agentMin, supported.lowerBound) ? v : nil
    }

    public static func encode(_ type: UInt16, flags: UInt16 = 0, payload: Data) -> Data {
        precondition(payload.count <= maxPayload, "payload too large")
        var out = Data(magic)
        withUnsafeBytes(of: type.littleEndian) { out.append(contentsOf: $0) }
        withUnsafeBytes(of: flags.littleEndian) { out.append(contentsOf: $0) }
        withUnsafeBytes(of: UInt32(payload.count).littleEndian) { out.append(contentsOf: $0) }
        out.append(payload)
        return out
    }
}

public struct AgentFrame: Sendable, Equatable {
    public var type: UInt16
    public var flags: UInt16
    public var payload: Data
}

/// Puts chunked messages back together; one that grows beyond its limit is dropped up to its last frame, so its
/// tail never passes for a message of its own (the same as the Rust side's Reassembler).
public struct AgentReassembler: Sendable {
    /// Per type: the message so far, or nil while the rest of a dropped one is skipped
    private var parts: [UInt16: Data?] = [:]
    public private(set) var dropped = 0

    public init() {}

    public mutating func push(_ frame: AgentFrame, limit: Int) -> Data? {
        let last = frame.flags & AgentProtocol.flagMore == 0
        switch parts[frame.type] {
        case .some(.none): break   // skipping
        case .some(.some(var buf)):
            if buf.count + frame.payload.count > limit {
                parts[frame.type] = .some(nil)
                dropped += 1
            } else {
                buf.append(frame.payload)
                parts[frame.type] = .some(buf)
            }
        case .none:
            if frame.payload.count > limit {
                parts[frame.type] = .some(nil)
                dropped += 1
            } else {
                parts[frame.type] = .some(frame.payload)
            }
        }
        guard last else { return nil }
        defer { parts[frame.type] = nil }
        return parts[frame.type] ?? nil
    }
}

/// Incremental frame decoder. A bad header costs one error and the bytes up to the next magic (resynchronisation).
public struct AgentDecoder: Sendable {
    private var buffer: [UInt8] = []
    public private(set) var errors = 0

    public init() {}

    public mutating func feed<S: Sequence>(_ bytes: S) where S.Element == UInt8 { buffer.append(contentsOf: bytes) }

    public mutating func next() -> AgentFrame? {
        while true {
            if buffer.count >= 4, !buffer[0..<4].elementsEqual(AgentProtocol.magic) {
                errors += 1
                skipToMagic()
                continue
            }
            guard buffer.count >= AgentProtocol.headerLength else { return nil }
            let length = Int(UInt32(buffer[8]) | UInt32(buffer[9]) << 8 | UInt32(buffer[10]) << 16 | UInt32(buffer[11]) << 24)
            if length > AgentProtocol.maxPayload {
                errors += 1
                skipToMagic()
                continue
            }
            guard buffer.count >= AgentProtocol.headerLength + length else { return nil }
            let type = UInt16(buffer[4]) | UInt16(buffer[5]) << 8
            let flags = UInt16(buffer[6]) | UInt16(buffer[7]) << 8
            let payload = Data(buffer[AgentProtocol.headerLength..<AgentProtocol.headerLength + length])
            buffer.removeFirst(AgentProtocol.headerLength + length)
            return AgentFrame(type: type, flags: flags, payload: payload)
        }
    }

    /// Drops at least one byte, then everything before the next possible magic
    private mutating func skipToMagic() {
        var start = 1
        while start < buffer.count {
            let tail = buffer[start..<min(buffer.count, start + 4)]
            if AgentProtocol.magic.starts(with: tail) { break }
            start += 1
        }
        buffer.removeFirst(min(start, buffer.count))
    }
}

// MARK: MIDI

/// MIDI data on the agent's port (spec §11.9, plan B161): Universal MIDI Packets (MIDI 2.0's format, which carries
/// MIDI 1.0 too) in records naming their endpoint – `id u32, protocol u8 (1 | 2), 0 u8, words u16`, then the words
/// (u32 little endian), whole UMP messages only. The same as nix/agent/src/ump.rs; both check the "midi" vectors.
public struct MIDIRecord: Sendable, Equatable {
    /// An endpoint of the Mac's list: Mac → guest the source the data came from, guest → Mac the destination
    public var id: UInt32
    /// 1 = MIDI 1.0 UMP, 2 = MIDI 2.0 UMP (CoreMIDI's MIDIProtocolID)
    public var `protocol`: UInt8
    public var words: [UInt32]

    public init(id: UInt32, protocol: UInt8, words: [UInt32]) {
        self.id = id
        self.protocol = `protocol`
        self.words = words
    }

    public static let headerLength = 8

    /// Words of the UMP message starting with `first`, by its message type (UMP spec 1.1)
    public static func messageWords(_ first: UInt32) -> Int {
        switch first >> 28 {
        case 0x0, 0x1, 0x2, 0x6, 0x7: 1
        case 0x3, 0x4, 0x8, 0x9, 0xA: 2
        case 0xB, 0xC: 3
        default: 4   // 0x5, 0xD, 0xE, 0xF
        }
    }

    /// How many of `words` form whole messages (a message cut off at the end is left out)
    public static func wholeMessages<C: RandomAccessCollection>(_ words: C) -> Int where C.Element == UInt32, C.Index == Int {
        var i = words.startIndex
        while i < words.endIndex {
            let n = messageWords(words[i])
            if i + n > words.endIndex { break }
            i += n
        }
        return i - words.startIndex
    }

    /// Records as payloads of at most `maxMIDIMessage` bytes; long records split at message boundaries
    public static func encode(_ records: [MIDIRecord]) -> [Data] {
        let maxWords = min((AgentProtocol.maxMIDIMessage - headerLength) / 4, Int(UInt16.max))
        var out: [Data] = []
        var cur = Data()
        for r in records {
            let words = r.words[0..<wholeMessages(r.words)]
            var start = 0
            while start < words.count {
                var end = start
                while end < words.count, end + messageWords(words[end]) - start <= maxWords { end += messageWords(words[end]) }
                let part = words[start..<end]
                if !cur.isEmpty, cur.count + headerLength + part.count * 4 > AgentProtocol.maxMIDIMessage {
                    out.append(cur)
                    cur = Data()
                }
                withUnsafeBytes(of: r.id.littleEndian) { cur.append(contentsOf: $0) }
                cur.append(r.protocol)
                cur.append(0)
                withUnsafeBytes(of: UInt16(part.count).littleEndian) { cur.append(contentsOf: $0) }
                for w in part { withUnsafeBytes(of: w.littleEndian) { cur.append(contentsOf: $0) } }
                start = end
            }
        }
        if !cur.isEmpty { out.append(cur) }
        return out
    }

    /// The records of a payload; nil if it is malformed (too long, cut off, a protocol other than 1 or 2, or words
    /// that are not whole messages)
    public static func decode(_ payload: Data) -> [MIDIRecord]? {
        guard payload.count <= AgentProtocol.maxMIDIMessage else { return nil }
        let bytes = [UInt8](payload)
        func u32(_ at: Int) -> UInt32 {
            UInt32(bytes[at]) | UInt32(bytes[at + 1]) << 8 | UInt32(bytes[at + 2]) << 16 | UInt32(bytes[at + 3]) << 24
        }
        var records: [MIDIRecord] = []
        var at = 0
        while at < bytes.count {
            guard at + headerLength <= bytes.count else { return nil }
            let id = u32(at), proto = bytes[at + 4]
            let count = Int(UInt16(bytes[at + 6]) | UInt16(bytes[at + 7]) << 8)
            guard proto == 1 || proto == 2 else { return nil }
            at += headerLength
            guard at + count * 4 <= bytes.count else { return nil }
            let words = (0..<count).map { u32(at + $0 * 4) }
            guard wholeMessages(words) == words.count else { return nil }
            at += count * 4
            records.append(MIDIRecord(id: id, protocol: proto, words: words))
        }
        return records
    }
}

/// One endpoint of the Mac as the guest gets it (`midiEndpoints`)
public struct MIDIEndpointInfo: Codable, Sendable, Equatable {
    public var id: UInt32
    /// "in": a source on the Mac, its data come into the guest; "out": a destination, the guest plays to it
    public var direction: String
    public var name: String
    public var manufacturer: String?
    public var model: String?
    public var `protocol`: UInt8

    public init(id: UInt32, direction: String, name: String, manufacturer: String?, model: String?, protocol: UInt8) {
        self.id = id
        self.direction = direction
        self.name = name
        self.manufacturer = manufacturer
        self.model = model
        self.protocol = `protocol`
    }
}

/// What the guest reported about its MIDI endpoints, and what the Mac measured (kept in agent.json)
public struct AgentMIDIState: Codable, Sendable, Equatable {
    /// Endpoints of the Mac the guest has as virtual endpoints, and those it could not create
    public var endpoints: Int
    public var failed: [String]
    /// Round trip Mac → guest agent → Mac of the last pings, in milliseconds (transport only, without CoreMIDI)
    public var roundTripMs: RoundTrip?

    public struct RoundTrip: Codable, Sendable, Equatable {
        public var min: Double
        public var median: Double
        public var max: Double
        public var samples: Int
    }

    public init(endpoints: Int = 0, failed: [String] = [], roundTripMs: RoundTrip? = nil) {
        self.endpoints = endpoints
        self.failed = failed
        self.roundTripMs = roundTripMs
    }
}

// MARK: Messages

/// The agent's hello. Everything in it comes from the guest – untrusted, cleaned by `sanitized()`.
public struct AgentHello: Codable, Sendable, Equatable {
    public struct ProtocolRange: Codable, Sendable, Equatable {
        public var min: Int
        public var max: Int
    }
    public var agent: String
    public var `protocol`: ProtocolRange
    public var os: String?
    public var osName: String?
    public var kernel: String?
    public var hostname: String?
    public var bootId: String?
    public var features: [String]?

    func sanitized() -> AgentHello {
        var h = self
        h.agent = Untrusted.text(agent)
        h.os = os.map(Untrusted.text)
        h.osName = osName.map(Untrusted.text)
        h.kernel = kernel.map(Untrusted.text)
        h.hostname = hostname.map(Untrusted.text)
        h.bootId = bootId.map(Untrusted.text)
        h.features = features.map { Array($0.prefix(32)).map(Untrusted.text) }
        return h
    }
}

public struct AgentWelcome: Codable, Sendable, Equatable {
    public var `protocol`: Int
    public var features: [String]
    public var statusInterval: Int
    public var host: String
}

/// The agent's periodic status (guest view; untrusted, cleaned by `sanitized()`).
public struct AgentStatus: Codable, Sendable, Equatable {
    public struct Memory: Codable, Sendable, Equatable { public var total: UInt64; public var available: UInt64 }
    public struct Disk: Codable, Sendable, Equatable { public var path: String; public var total: UInt64; public var free: UInt64 }
    public struct Address: Codable, Sendable, Equatable { public var interface: String; public var family: String; public var address: String }

    public var uptime: Double?
    public var load: [Double]?
    public var memory: Memory?
    public var disk: Disk?
    public var addresses: [Address]?
    public var users: [String]?

    func sanitized() -> AgentStatus {
        var s = self
        s.uptime = uptime.flatMap(Untrusted.number)
        s.load = load.map { Array($0.prefix(3)).compactMap(Untrusted.number) }
        s.disk = disk.map { Disk(path: Untrusted.text($0.path), total: $0.total, free: $0.free) }
        s.addresses = addresses.map {
            Array($0.prefix(64)).map { Address(interface: Untrusted.text($0.interface), family: Untrusted.text($0.family),
                                               address: Untrusted.text($0.address)) }
        }
        s.users = users.map { Array($0.prefix(64)).map(Untrusted.text) }
        return s
    }
}

/// Cleaning for text and numbers from the guest before omaflake keeps or shows them
enum Untrusted {
    static func text(_ s: String) -> String {
        String(String.UnicodeScalarView(s.unicodeScalars.filter { !CharacterSet.controlCharacters.contains($0) }).prefix(256))
    }

    static func number(_ d: Double) -> Double? { d.isFinite && d >= 0 && d < 1e15 ? d : nil }
}

// MARK: State

/// What omaflake knows from a VM's agent: kept by the Mac side of the link in the instance folder (`agent.json`),
/// read by the CLI and the app. Removed when the VM starts, so a file left from an earlier run never counts.
public struct AgentState: Codable, Sendable, Equatable {
    public var hello: AgentHello?
    public var helloAt: Date?
    /// The version agreed in the welcome; nil while there is none (no hello yet, or no common version)
    public var protocolVersion: Int?
    public var status: AgentStatus?
    public var statusAt: Date?
    public var goodbyeAt: Date?
    /// Why the link does not work (no common version, too many malformed messages)
    public var problem: String?
    /// What the welcome enabled: offered by the agent and switched on for this VM
    public var features: [String]?
    /// MIDI, when the welcome enabled it: the guest's endpoints and the measured round trip
    public var midi: AgentMIDIState?

    public init() {}

    /// The agent said hello and both sides speak a common version: the VM is up
    public var isReady: Bool { hello != nil && protocolVersion != nil && goodbyeAt == nil }

    /// The agent speaks only older protocol versions than this omaflake: the image should be rebuilt (spec §11.9)
    public var agentIsOlder: Bool { (hello?.protocol.max ?? AgentProtocol.current) < AgentProtocol.current }

    public static func load(_ url: URL) -> AgentState? {
        guard let data = try? Data(contentsOf: url) else { return nil }
        return try? JSONDecoder.agent.decode(AgentState.self, from: data)
    }

    public func save(_ url: URL) throws {
        try JSONEncoder.agent.encode(self).write(to: url, options: .atomic)
    }
}

extension JSONDecoder {
    public static var agent: JSONDecoder {
        let d = JSONDecoder()
        d.dateDecodingStrategy = .iso8601
        return d
    }
}

extension JSONEncoder {
    public static var agent: JSONEncoder {
        let e = JSONEncoder()
        e.dateEncodingStrategy = .iso8601
        e.outputFormatting = [.prettyPrinted, .sortedKeys]
        return e
    }
}

// MARK: Link

/// The Mac side of one agent connection: reads frames from `fd` (QEMU: the port's Unix socket), answers the hello
/// with a welcome and keeps what the agent reports in `stateFile`. Accepts only the messages above, with limits:
/// the guest is untrusted (spec §11.9). Nothing the guest sends makes the Mac do more than update that file.
public final class AgentLink: @unchecked Sendable {
    public enum End: Sendable, Equatable { case closed, abandoned }

    let fd: Int32
    let stateFile: URL
    let host: String
    let statusInterval: Int
    let log: @Sendable (String) -> Void
    /// Bad frames, malformed JSON or unknown types tolerated before the link stops listening
    static let errorBudget = 64
    /// Messages faster than this are dropped (a guest flooding the Mac side)
    static let minInterval: TimeInterval = 0.5

    private(set) var state = AgentState()
    private var decoder = AgentDecoder()
    private var parts = AgentReassembler()
    private var errors = 0
    private var lastAccepted: [UInt16: Date] = [:]
    private var clipboardTimes: [Date] = []
    private let writeLock = NSLock()

    /// Features the Mac side allows for this VM (its switches); the welcome enables those the agent offers too
    let allowed: Set<String>
    /// What the welcome enabled – messages of other features are dropped
    public private(set) var enabled: Set<String> = []

    /// Called on the link's thread: the welcome went out with these features (also again after a new hello)
    public var onWelcome: (@Sendable (Set<String>) -> Void)?
    /// Clipboard content from the guest (text or PNG, checked), when the clipboard is enabled
    public var onClipboard: (@Sendable (String, Data) -> Void)?
    /// The guest's camera requests, when the camera is enabled
    public var onCamera: (@Sendable (CameraRequest) -> Void)?

    public enum CameraRequest: Sendable, Equatable { case start, stop, credits(Int) }

    /// MIDI the guest's apps played to the Mac's destinations, when MIDI is enabled (checked: whole UMP messages,
    /// within the rate limit); the receiver sends only to ids of its current list
    public var onMIDI: (@Sendable ([MIDIRecord]) -> Void)?
    /// MIDI from the guest: the token bucket (bytes) and when it was last filled
    private var midiTokens = Double(AgentProtocol.midiBurst)
    private var midiFilled = Date()
    private var midiDropped = 0
    private var roundTrips: [Double] = []

    /// A ping for the MIDI round trip: its payload is the send time, the pong brings it back (`midiPong`)
    @discardableResult
    public func sendMIDIPing() -> Bool {
        let now = DispatchTime.now().uptimeNanoseconds
        return send(.midiPing, payload: withUnsafeBytes(of: now.littleEndian) { Data($0) })
    }

    public init(fd: Int32, stateFile: URL, host: String = "omaflake", statusInterval: Int = 5,
                allowed: Set<String> = ["status"], log: @escaping @Sendable (String) -> Void = { _ in }) {
        self.fd = fd
        self.stateFile = stateFile
        self.host = host
        self.statusInterval = statusInterval
        self.allowed = allowed.union(["status"])
        self.log = log
    }

    /// Sends a message to the guest; thread-safe. False when the port is gone. Large messages go in chunks – except
    /// camera frames: one frame each (at most 2 MiB, so a 1280×720 NV12 frame fits), the guest takes them whole
    @discardableResult
    public func send(_ message: AgentProtocol.Message, payload: Data) -> Bool {
        let bytes = message == .cameraFrame ? AgentProtocol.encode(message.rawValue, payload: payload)
            : AgentProtocol.encodeMessage(message.rawValue, payload: payload)
        return writeLock.withLock {
            bytes.withUnsafeBytes { buf in
                var off = 0
                while off < buf.count {
                    let n = write(fd, buf.baseAddress! + off, buf.count - off)
                    if n < 0 && errno == EINTR { continue }
                    if n <= 0 { return false }
                    off += n
                }
                return true
            }
        }
    }

    /// Reads until the VM closes the port (`.closed`) or the guest misbehaves (`.abandoned`); blocking.
    public func run() -> End {
        var chunk = [UInt8](repeating: 0, count: 65536)
        while true {
            let n = read(fd, &chunk, chunk.count)
            if n < 0 && errno == EINTR { continue }
            if n <= 0 { return .closed }
            if !receive(chunk[0..<n]) { return .abandoned }
        }
    }

    /// Handles bytes from the guest; false once the error budget is spent.
    func receive(_ bytes: ArraySlice<UInt8>) -> Bool {
        decoder.feed(bytes)
        while let frame = decoder.next() { handle(frame) }
        guard errors + decoder.errors <= Self.errorBudget else {
            state.problem = "too many malformed messages from the guest – omaflake stopped listening"
            persist()
            log("agent: \(state.problem!)")
            return false
        }
        return true
    }

    func handle(_ frame: AgentFrame) {
        let now = Date()
        guard let message = AgentProtocol.Message(rawValue: frame.type) else {
            errors += 1   // unknown to this version; a newer agent only sends what the welcome enabled
            return
        }
        if [.hello, .status, .goodbye, .midiStatus].contains(message),
           let last = lastAccepted[frame.type], now.timeIntervalSince(last) < Self.minInterval { return }
        switch message {
        case .hello:
            guard let hello = try? JSONDecoder().decode(AgentHello.self, from: frame.payload).sanitized() else {
                errors += 1
                return
            }
            state = AgentState()
            state.hello = hello
            state.helloAt = now
            if let v = AgentProtocol.agree(agentMin: hello.protocol.min, agentMax: hello.protocol.max) {
                state.protocolVersion = v
                welcome(v)
                log("agent: hello from \(hello.agent) (protocol \(v)) on \(hello.osName ?? hello.os ?? "?")")
            } else {
                state.problem = "the agent speaks protocol \(hello.protocol.min)–\(hello.protocol.max), "
                    + "this omaflake \(AgentProtocol.supported.lowerBound)–\(AgentProtocol.supported.upperBound)"
                log("agent: \(state.problem!)")
            }
        case .status:
            guard state.protocolVersion != nil else { return }   // only after a welcome
            guard let status = try? JSONDecoder().decode(AgentStatus.self, from: frame.payload).sanitized() else {
                errors += 1
                return
            }
            state.status = status
            state.statusAt = now
        case .goodbye:
            state.goodbyeAt = now
            log("agent: goodbye")
        case .clipboard:
            guard enabled.contains("clipboard") else { return }
            guard let payload = parts.push(frame, limit: AgentProtocol.maxClipboardMessage) else { return }
            // At most 4 contents a second – a guest flooding the pasteboard is cut off
            clipboardTimes = clipboardTimes.filter { now.timeIntervalSince($0) < 1 } + [now]
            guard clipboardTimes.count <= 4 else { return }
            guard let (mime, data) = Self.clipboard(payload) else { errors += 1; return }
            onClipboard?(mime, data)
            return   // no state change to keep
        case .cameraControl:
            guard enabled.contains("camera") else { return }
            struct Control: Decodable { var action: String }
            switch (try? JSONDecoder().decode(Control.self, from: frame.payload))?.action {
            case "start": onCamera?(.start)
            case "stop": onCamera?(.stop)
            default: errors += 1
            }
            return
        case .cameraCredit:
            guard enabled.contains("camera") else { return }
            struct Credit: Decodable { var credits: Int }
            guard let c = try? JSONDecoder().decode(Credit.self, from: frame.payload), c.credits > 0 else { errors += 1; return }
            onCamera?(.credits(min(c.credits, AgentProtocol.maxCameraCredits)))
            return
        case .midiData:
            guard enabled.contains("midi") else { return }
            // Token bucket: `midiRate` bytes a second on average, `midiBurst` at once – beyond that dropped
            midiTokens = min(Double(AgentProtocol.midiBurst),
                             midiTokens + now.timeIntervalSince(midiFilled) * Double(AgentProtocol.midiRate))
            midiFilled = now
            guard Double(frame.payload.count) <= midiTokens else {
                midiDropped += 1
                if midiDropped == 1 || midiDropped % 1000 == 0 {
                    log("agent: MIDI from the guest beyond \(AgentProtocol.midiRate / 1024) KiB/s dropped (\(midiDropped)×)")
                }
                return
            }
            midiTokens -= Double(frame.payload.count)
            guard let records = MIDIRecord.decode(frame.payload) else { errors += 1; return }
            onMIDI?(records)
            return
        case .midiStatus:
            guard enabled.contains("midi") else { return }
            struct Status: Decodable { var endpoints: Int; var failed: [String]? }
            guard let s = try? JSONDecoder().decode(Status.self, from: frame.payload), s.endpoints >= 0 else {
                errors += 1
                return
            }
            var midi = state.midi ?? AgentMIDIState()
            midi.endpoints = min(s.endpoints, 1024)
            midi.failed = Array((s.failed ?? []).prefix(64)).map(Untrusted.text)
            state.midi = midi
        case .midiPong:
            guard enabled.contains("midi"), frame.payload.count == 8 else { return }
            let sent = frame.payload.withUnsafeBytes { UInt64(littleEndian: $0.loadUnaligned(as: UInt64.self)) }
            let nowNs = DispatchTime.now().uptimeNanoseconds
            guard sent <= nowNs, nowNs - sent < 10_000_000_000 else { return }   // a forged or stale time
            roundTrips = Array((roundTrips + [Double(nowNs - sent) / 1e6]).suffix(60))
            let sorted = roundTrips.sorted()
            var midi = state.midi ?? AgentMIDIState()
            midi.roundTripMs = .init(min: sorted.first!, median: sorted[sorted.count / 2], max: sorted.last!,
                                     samples: sorted.count)
            state.midi = midi
            // kept with the next status; no file write per pong
            return
        case .welcome, .cameraFrame, .cameraStatus, .midiEndpoints, .midiPing:
            errors += 1   // the Mac's messages, never the guest's
            return
        }
        lastAccepted[frame.type] = now
        persist()
    }

    func welcome(_ version: Int) {
        // What the agent offers and this VM allows
        let offered = Set(state.hello?.features ?? ["status"])
        enabled = allowed.intersection(offered).union(["status"])
        state.features = enabled.sorted()
        let w = AgentWelcome(protocol: version, features: enabled.sorted(), statusInterval: statusInterval, host: host)
        guard let json = try? JSONEncoder().encode(w) else { return }
        send(.welcome, payload: json)
        onWelcome?(enabled)
    }

    /// (mime, data) of a clipboard message if it is text or PNG within the size limit
    static func clipboard(_ payload: Data) -> (String, Data)? {
        guard let obj = try? JSONSerialization.jsonObject(with: payload) as? [String: String],
              let mime = obj["mime"], ["text/plain;charset=utf-8", "image/png"].contains(mime),
              let b64 = obj["data"], let data = Data(base64Encoded: b64), data.count <= AgentProtocol.maxClipboard
        else { return nil }
        return (mime, data)
    }

    func persist() {
        do { try state.save(stateFile) } catch { log("agent: cannot write \(stateFile.path): \(error)") }
    }
}
