// SPDX-License-Identifier: AGPL-3.0-or-later
// miditest – checks MIDI end to end, for omaflake's MIDI bridge (plan B161; runs in a macOS VM and on the Mac).
// Build: swiftc -O scripts/miditest.swift -o build/miditest (a developer tool, not part of the app)
//
//   miditest                    list (default)
//   miditest list               every MIDI source and destination: name, manufacturer, model, protocol
//   miditest monitor            show what arrives from every source, with the time since the previous message (Ctrl-C)
//   miditest play <name>        a C major scale to the destination whose name contains <name>
//   miditest identity <name>    ask the device <name> who it is (Universal Identity Request) and time the answer –
//                               in a VM that is the whole way VM → Mac → device → Mac → VM
//   miditest echo <name>        (on the Mac) send everything from source <name> back to destination <name>, e.g.
//                               "omaflake <VM>" – then `miditest loop "omaflake Mac"` in the VM needs no device
//   miditest loop <name> [n]    n notes (default 200), a 1,000-byte SysEx and program changes to <name> and back from
//                               <name>: lost messages, latency, data unchanged
//   miditest selftest           the loop over a virtual source and destination of this program (checks the tool)
//   miditest all                list, then identity for every device that is both a source and a destination, then
//                               loop over "omaflake Mac" when an echo answers
//   miditest vm <vm>            (on the Mac) everything at once: the Mac's devices and `omaf agent`, an echo for
//                               "omaflake <vm>", this program copied into the VM over `omaf ssh` and `miditest all`
//                               run there – all of it in a log file miditest-<vm>-<time>.log in the current folder
import CoreMIDI
import Foundation

setvbuf(stdout, nil, _IOLBF, 0)

// MARK: Endpoints

func string(_ obj: MIDIObjectRef, _ prop: CFString) -> String? {
    var s: Unmanaged<CFString>?
    guard MIDIObjectGetStringProperty(obj, prop, &s) == noErr else { return nil }
    return s?.takeRetainedValue() as String?
}
func int(_ obj: MIDIObjectRef, _ prop: CFString) -> Int32? {
    var v: Int32 = 0
    return MIDIObjectGetIntegerProperty(obj, prop, &v) == noErr ? v : nil
}
func name(_ e: MIDIEndpointRef) -> String { string(e, kMIDIPropertyDisplayName) ?? string(e, kMIDIPropertyName) ?? "?" }
var sources: [MIDIEndpointRef] { (0..<MIDIGetNumberOfSources()).map { MIDIGetSource($0) } }
var destinations: [MIDIEndpointRef] { (0..<MIDIGetNumberOfDestinations()).map { MIDIGetDestination($0) } }
func find(_ list: [MIDIEndpointRef], _ wanted: String, _ what: String) -> MIDIEndpointRef {
    if let e = list.first(where: { name($0) == wanted }) ?? list.first(where: { name($0).localizedCaseInsensitiveContains(wanted) }) {
        return e
    }
    print("No \(what) contains \"\(wanted)\" – `miditest list` shows them")
    exit(1)
}
func describe(_ e: MIDIEndpointRef) -> String {
    var parts = [name(e)]
    if let m = string(e, kMIDIPropertyManufacturer), !m.isEmpty { parts.append("maker \(m)") }
    if let m = string(e, kMIDIPropertyModel), !m.isEmpty { parts.append("model \(m)") }
    if let p = int(e, kMIDIPropertyProtocolID) { parts.append("MIDI \(p == 2 ? "2.0" : "1.0")") }
    if int(e, kMIDIPropertyOffline) == 1 { parts.append("offline") }
    return parts.joined(separator: " · ")
}

// MARK: Messages

/// MIDI 1.0 bytes as text: note on/off, CC, program change, pitch bend, SysEx (shortened)
func text(_ b: [UInt8]) -> String {
    guard let s = b.first else { return "(empty)" }
    let ch = Int(s & 0x0F) + 1
    switch s & 0xF0 {
    case 0x90 where b.count >= 3 && b[2] > 0: return "note on  ch\(ch) \(b[1]) vel \(b[2])"
    case 0x80, 0x90: return b.count >= 2 ? "note off ch\(ch) \(b[1])" : "note off"
    case 0xA0: return "poly pressure ch\(ch)"
    case 0xB0: return b.count >= 3 ? "CC ch\(ch) \(b[1]) = \(b[2])" : "CC"
    case 0xC0: return b.count >= 2 ? "program ch\(ch) \(b[1])" : "program"
    case 0xD0: return "channel pressure ch\(ch)"
    case 0xE0: return b.count >= 3 ? "pitch bend ch\(ch) \(Int(b[1]) | Int(b[2]) << 7)" : "pitch bend"
    default:
        if s == 0xF0 { return "SysEx \(b.count) bytes " + b.prefix(8).map { String(format: "%02X", $0) }.joined(separator: " ") + (b.count > 8 ? " …" : "") }
        if s == 0xF8 { return "clock" }
        if s == 0xFE { return "active sensing" }
        return b.map { String(format: "%02X", $0) }.joined(separator: " ")
    }
}

/// Splits a packet's bytes into messages (SysEx kept whole within the packet; running status not used by CoreMIDI)
func messages(_ bytes: [UInt8]) -> [[UInt8]] {
    var out: [[UInt8]] = []
    var i = 0
    while i < bytes.count {
        let s = bytes[i]
        if s == 0xF0 {
            var j = i + 1
            while j < bytes.count && bytes[j] != 0xF7 { j += 1 }
            out.append(Array(bytes[i...min(j, bytes.count - 1)]))
            i = j + 1
            continue
        }
        let n: Int
        switch s & 0xF0 {
        case 0xC0, 0xD0: n = 2
        case 0x80, 0x90, 0xA0, 0xB0, 0xE0: n = 3
        default: n = s == 0xF2 ? 3 : (s == 0xF1 || s == 0xF3) ? 2 : 1
        }
        out.append(Array(bytes[i..<min(i + n, bytes.count)]))
        i += n
    }
    return out
}

let now: () -> Double = { Double(clock_gettime_nsec_np(CLOCK_UPTIME_RAW)) / 1e6 }   // ms

// MARK: Client

final class Midi: @unchecked Sendable {
    var client = MIDIClientRef()
    var inPort = MIDIPortRef()
    var outPort = MIDIPortRef()
    let lock = NSLock()
    var onMessage: (MIDIEndpointRef, [UInt8], Double) -> Void = { _, _, _ in }
    var sysexBuffer: [MIDIEndpointRef: [UInt8]] = [:]

    init() {
        MIDIClientCreateWithBlock("miditest" as CFString, &client) { _ in }
        MIDIOutputPortCreate(client, "miditest out" as CFString, &outPort)
        MIDIInputPortCreateWithBlock(client, "miditest in" as CFString, &inPort) { [weak self] list, src in
            guard let self else { return }
            let t = now()
            let endpoint = MIDIEndpointRef(UInt(bitPattern: src))
            // Through pointers: a packet can be longer than MIDIPacket's 256-byte data field
            let dataOffset = MemoryLayout<MIDIPacket>.offset(of: \MIDIPacket.data)!
            for packet in list.unsafeSequence() {
                let start = UnsafeRawPointer(packet).advanced(by: dataOffset)
                let bytes = Array(UnsafeRawBufferPointer(start: start, count: Int(packet.pointee.length)))
                self.handle(endpoint, bytes, t)
            }
        }
    }

    /// SysEx may come in several packets: collect until F7
    func handle(_ src: MIDIEndpointRef, _ bytes: [UInt8], _ t: Double) {
        lock.lock(); defer { lock.unlock() }
        var rest = bytes
        if var partial = sysexBuffer[src] {
            if let end = rest.firstIndex(of: 0xF7) {
                partial += rest[...end]
                sysexBuffer[src] = nil
                onMessage(src, partial, t)
                rest = Array(rest[(end + 1)...])
            } else {
                partial += rest
                sysexBuffer[src] = partial
                return
            }
        }
        if let start = rest.firstIndex(of: 0xF0), !rest[start...].contains(0xF7) {
            for m in messages(Array(rest[..<start])) { onMessage(src, m, t) }
            sysexBuffer[src] = Array(rest[start...])
            return
        }
        for m in messages(rest) { onMessage(src, m, t) }
    }

    /// The source travels as the connection's refCon: the read block gets that, not the endpoint
    /// Ends the client and its ports: no callback may reach a freed object (a check that ended keeps nothing open)
    deinit { MIDIClientDispose(client) }

    func listen(_ src: MIDIEndpointRef) { MIDIPortConnectSource(inPort, src, UnsafeMutableRawPointer(bitPattern: UInt(src))) }
    func listenAll() { sources.forEach(listen) }

    func send(_ dest: MIDIEndpointRef, _ bytes: [UInt8]) {
        let size = 1024 + bytes.count * 2
        let raw = UnsafeMutableRawPointer.allocate(byteCount: size, alignment: 8)
        defer { raw.deallocate() }
        let list = raw.bindMemory(to: MIDIPacketList.self, capacity: 1)
        var p = MIDIPacketListInit(list)
        // a packet holds at most 256 bytes in some drivers: SysEx goes in chunks
        var i = 0
        while i < bytes.count {
            let chunk = Array(bytes[i..<min(i + 256, bytes.count)])
            p = MIDIPacketListAdd(list, size, p, 0, chunk.count, chunk)
            i += chunk.count
        }
        MIDISend(outPort, dest, list)
    }
}

func waitUntil(_ seconds: Double, _ done: () -> Bool) -> Bool {
    let end = now() + seconds * 1000
    while now() < end {
        if done() { return true }
        RunLoop.current.run(until: Date(timeIntervalSinceNow: 0.005))
    }
    return done()
}
func stats(_ v: [Double]) -> String {
    guard !v.isEmpty else { return "–" }
    let s = v.sorted()
    let p = { (q: Double) in s[min(s.count - 1, Int(Double(s.count - 1) * q))] }
    return String(format: "min %.2f · median %.2f · p95 %.2f · max %.2f ms", s[0], p(0.5), p(0.95), s.last!)
}

// MARK: Commands

func listText() -> String {
    var lines = ["Sources (MIDI comes from here):"]
    lines += sources.map { "  " + describe($0) }
    if sources.isEmpty { lines.append("  (none)") }
    lines.append("Destinations (MIDI goes there):")
    lines += destinations.map { "  " + describe($0) }
    if destinations.isEmpty { lines.append("  (none)") }
    return lines.joined(separator: "\n")
}
func list() { print(listText()) }

func monitor() {
    let midi = Midi()
    var last: Double?
    midi.onMessage = { src, m, t in
        if m == [0xF8] || m == [0xFE] { return }
        let gap = last.map { String(format: "+%7.1f ms", t - $0) } ?? "           "
        last = t
        print("\(gap)  \(name(src)): \(text(m))")
    }
    midi.listenAll()
    print("Listening to \(sources.count) sources – play something (Ctrl-C ends)")
    RunLoop.current.run()
}

func play(_ wanted: String) {
    let midi = Midi()
    let dest = find(destinations, wanted, "destination")
    print("Playing a C major scale to \(name(dest)) …")
    for n: UInt8 in [60, 62, 64, 65, 67, 69, 71, 72, 71, 69, 67, 65, 64, 62, 60] {
        midi.send(dest, [0x90, n, 90]); usleep(250_000)
        midi.send(dest, [0x80, n, 0]); usleep(50_000)
    }
    print("Done – did you hear it?")
}

@discardableResult
func identity(_ wanted: String, quiet: Bool = false) -> Bool {
    let midi = Midi()
    let dest = find(destinations, wanted, "destination")
    let src = find(sources, name(dest), "source")
    var reply: [UInt8]?
    var at: Double = 0
    midi.onMessage = { s, m, t in
        if s == src && m.count >= 5 && m[0] == 0xF0 && m[1] == 0x7E && m[3] == 0x06 && m[4] == 0x02 { reply = m; at = t }
    }
    midi.listen(src)
    usleep(100_000)
    let sent = now()
    midi.send(dest, [0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7])
    if waitUntil(2) { reply != nil }, let r = reply {
        let maker = r.count > 5 ? (r[5] == 0 && r.count > 7 ? String(format: "%02X %02X %02X", r[5], r[6], r[7]) : String(format: "%02X", r[5])) : "?"
        print(String(format: "  %@: answers – manufacturer id %@, %.2f ms there and back", name(dest), maker, at - sent))
        return true
    }
    if !quiet { print("  \(name(dest)): no answer to the Identity Request within 2 s (not every device answers it)") }
    else { print("  \(name(dest)): no identity answer (not every device answers)") }
    return false
}

func echo(_ wanted: String) {
    let midi = Midi()
    let src = find(sources, wanted, "source")
    let dest = find(destinations, name(src), "destination")
    var count = 0
    midi.onMessage = { _, m, _ in midi.send(dest, m); count += 1 }
    midi.listen(src)
    print("Echoing everything from \(name(src)) back to \(name(dest)) (Ctrl-C ends)")
    Timer.scheduledTimer(withTimeInterval: 5, repeats: true) { _ in if count > 0 { print("  \(count) messages echoed") } }
    RunLoop.current.run()
}

/// Notes, program changes and a SysEx to `dest`, expected back from `src`
@discardableResult
func loop(dest: MIDIEndpointRef, src: MIDIEndpointRef, notes: Int, midi: Midi = Midi()) -> Bool {
    var sent: [Int: Double] = [:]
    var latency: [Double] = []
    var sysexBack: [UInt8]?
    var unexpected = 0
    let sysex: [UInt8] = [0xF0, 0x7D] + (0..<997).map { UInt8($0 % 128) } + [0xF7]   // 1,000 bytes, non-commercial id
    midi.onMessage = { s, m, t in
        guard s == src else { return }
        if m.first == 0xF0 { sysexBack = m; return }
        if m.count == 3 && m[0] == 0x90, let t0 = sent.removeValue(forKey: Int(m[1]) << 7 | Int(m[2])) {
            latency.append(t - t0)
        } else if m.count == 3 && m[0] == 0x80 {
        } else if m.count == 2 && m[0] & 0xF0 == 0xC0 {
        } else { unexpected += 1 }
    }
    midi.listen(src)
    usleep(200_000)
    print("  \(notes) notes to \(name(dest)), expected back from \(name(src)) …")
    for i in 0..<notes {
        let key = 36 + i % 60, vel = 1 + (i / 60) % 126
        sent[key << 7 | vel] = now()
        midi.send(dest, [0x90, UInt8(key), UInt8(vel)])
        midi.send(dest, [0x80, UInt8(key), 0])
        _ = waitUntil(0.01) { false }
    }
    midi.send(dest, [0xC0, 5])
    midi.send(dest, sysex)
    _ = waitUntil(3) { sent.isEmpty && sysexBack != nil }
    let back = notes - sent.count
    print("    notes back: \(back)/\(notes), latency \(stats(latency))")
    let sysexOK = sysexBack == sysex
    print("    SysEx 1,000 bytes: " + (sysexBack == nil ? "did not come back" : sysexOK ? "unchanged" : "CHANGED (\(sysexBack!.count) bytes)"))
    if unexpected > 0 { print("    \(unexpected) unexpected messages") }
    return back == notes && sysexOK
}

func selftest() {
    let midi = Midi()
    var vsrc = MIDIEndpointRef(), vdest = MIDIEndpointRef()
    MIDISourceCreate(midi.client, "miditest self" as CFString, &vsrc)
    MIDIDestinationCreateWithBlock(midi.client, "miditest self" as CFString, &vdest) { list, _ in
        MIDIReceived(vsrc, list)   // what arrives at the destination goes out of the source
    }
    usleep(300_000)
    print("Self-test over a virtual pair of this program:")
    let ok = loop(dest: vdest, src: vsrc, notes: 200, midi: midi)
    print(ok ? "Self-test passed" : "Self-test FAILED")
    exit(ok ? 0 : 1)
}

func all() {
    list()
    let both = destinations.filter { d in sources.contains { name($0) == name(d) } && !name(d).hasPrefix("omaflake") }
    print("\nIdentity of every device that sends and receives:")
    if both.isEmpty { print("  (none)") }
    for d in both { identity(name(d), quiet: true) }
    if let d = destinations.first(where: { name($0) == "omaflake Mac" }), let s = sources.first(where: { name($0) == "omaflake Mac" }) {
        print("\nLoop over \"omaflake Mac\" (needs `miditest echo \"omaflake <VM>\"` running on the Mac):")
        let midi = Midi()
        var answered = false
        midi.onMessage = { src, m, _ in if src == s && m == [0xB0, 0x77, 0x55] { answered = true } }
        midi.listen(s)
        usleep(100_000)
        midi.send(d, [0xB0, 0x77, 0x55])
        if waitUntil(1) { answered } {
            loop(dest: d, src: s, notes: 200, midi: midi)
        } else {
            print("  no echo answers – start it on the Mac, or skip this part")
        }
    }
}

// MARK: The whole test from the Mac

/// Runs a program, its output (stdout and stderr) as text; `input`: a file for its stdin
func run(_ program: String, _ arguments: [String], input: String? = nil) -> (Int32, String) {
    let p = Process()
    p.executableURL = URL(fileURLWithPath: program)
    p.arguments = arguments
    let pipe = Pipe()
    p.standardOutput = pipe
    p.standardError = pipe
    if let input { p.standardInput = FileHandle(forReadingAtPath: input) }
    do { try p.run() } catch { return (-1, "\(program): \(error.localizedDescription)") }
    let data = pipe.fileHandleForReading.readDataToEndOfFile()
    p.waitUntilExit()
    return (p.terminationStatus, String(decoding: data, as: UTF8.self))
}

func omafPath() -> String? {
    let home = FileManager.default.homeDirectoryForCurrentUser.path
    return ["/usr/local/bin/omaf", "\(home)/.local/bin/omaf", "/Applications/omaflake.app/Contents/MacOS/omaf",
            "\(home)/Applications/omaflake.app/Contents/MacOS/omaf"].first { FileManager.default.isExecutableFile(atPath: $0) }
}

func vmTest(_ vm: String) {
    var log = ""
    func out(_ s: String) { print(s); log += s + "\n" }
    let started = ISO8601DateFormatter().string(from: Date())
    let safe = vm.map { $0.isLetter || $0.isNumber || $0 == "-" ? $0 : "_" }
    let file = "miditest-\(String(safe))-\(started.replacingOccurrences(of: ":", with: "-")).log"
    // The log is written whatever happens – also when a step stops the run
    func save() {
        do {
            try log.write(toFile: file, atomically: true, encoding: .utf8)
            print("\nLog: \(FileManager.default.currentDirectoryPath)/\(file)")
        } catch { print("\nCould not write the log: \(error.localizedDescription)") }
    }
    out("miditest vm \(vm) – \(started) – Mac: macOS \(ProcessInfo.processInfo.operatingSystemVersionString)")
    guard let omaf = omafPath() else {
        out("omaf not found in /usr/local/bin, ~/.local/bin or omaflake.app – install the omaf command in omaflake's Doctor")
        save()
        exit(1)
    }
    out("omaf: \(omaf)")

    out("\n== The Mac's MIDI devices")
    out(listText())
    out("\n== omaf agent \(vm)")
    out(run(omaf, ["agent", vm]).1.trimmingCharacters(in: .whitespacesAndNewlines))

    // The echo for the VM's own pair, in this process
    let pair = "omaflake \(vm)"
    let midi = Midi()
    var echoed = 0
    let echoLock = NSLock()
    if let src = sources.first(where: { name($0) == pair }), let dest = destinations.first(where: { name($0) == pair }) {
        midi.onMessage = { _, m, _ in midi.send(dest, m); echoLock.lock(); echoed += 1; echoLock.unlock() }
        midi.listen(src)
        out("\nEcho on the Mac: everything from \"\(pair)\" goes back to it")
    } else {
        out("\nNo \"\(pair)\" on the Mac – is the VM running with MIDI on? (the loop in the VM is skipped then)")
    }

    out("\n== Copying miditest into the VM")
    let me = Bundle.main.executablePath ?? CommandLine.arguments[0]
    let (copied, copyOut) = run(omaf, ["ssh", vm, "cat > ~/miditest && chmod +x ~/miditest && echo copied"], input: me)
    out(copyOut.trimmingCharacters(in: .whitespacesAndNewlines))
    if copied != 0 { out("Copying failed (\(copied)) – is the VM running and does `omaf ssh \(vm)` work?") }
    else {
        out("\n== In the VM: miditest all")
        let (status, text) = run(omaf, ["ssh", vm, "~/miditest all"])
        out(text.trimmingCharacters(in: .whitespacesAndNewlines))
        if status != 0 { out("(exit status \(status))") }
    }
    echoLock.lock(); let n = echoed; echoLock.unlock()
    out("\nEcho on the Mac: \(n) messages sent back")

    save()
}

// MARK: Main

let args = Array(CommandLine.arguments.dropFirst())
switch (args.first ?? "list", args.count) {
case ("list", _): list()
case ("monitor", _): monitor()
case ("play", 2...): play(args[1])
case ("identity", 2...): exit(identity(args[1]) ? 0 : 1)
case ("echo", 2...): echo(args[1])
case ("loop", 2...):
    let dest = find(destinations, args[1], "destination")
    let src = find(sources, name(dest), "source")
    exit(loop(dest: dest, src: src, notes: args.count > 2 ? Int(args[2]) ?? 200 : 200) ? 0 : 1)
case ("selftest", _): selftest()
case ("all", _): all()
case ("vm", 2...): vmTest(args[1])
default:
    print("""
    miditest – checks MIDI end to end
      miditest list                 sources and destinations
      miditest monitor              show what arrives (Ctrl-C ends)
      miditest play <name>          a scale to a destination
      miditest identity <name>      ask a device who it is, time the answer
      miditest echo <name>          (Mac) send everything from <name> back to <name>
      miditest loop <name> [n]      notes and SysEx there and back, with latency
      miditest selftest             check this tool
      miditest all                  everything that works without help
      miditest vm <vm>              (Mac) all of it with the VM over omaf ssh, saved as a log
    """)
}
