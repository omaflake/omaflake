// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import ArgumentParser
import Darwin
import Foundation
import OmaflakeCore

/// Mac side of the guest integration channels (one process per VM, started detached with the VM, ends with it).
/// Agent: hello and status from the omaflake agent over the port org.omaflake.agent, kept in the instance's
/// agent.json (AgentLink, spec §11.9). Clipboard and camera go over the agent as soon as its welcome enables them
/// (images from plan B106 on); older images keep their own ports: org.omaflake.clipboard (one JSON object per line,
/// {"mime": …, "data": <base64>}) and org.omaflake.camera. --clipboard / --camera (the old ports' sockets) are also
/// what switches the features on for this VM. Each side of the clipboard remembers what it last wrote, so the change
/// that causes is not sent back; nothing is sent until something is copied.
struct Bridge: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "Agent, clipboard and camera bridge between the Mac and a VM (started by omaf).")
    @Option(help: "Unix socket of the VM's agent port") var agent: String?
    @Option(help: "File for what the agent reports (with --agent)") var agentState: String?
    @Option(help: "Unix socket of the VM's clipboard port") var clipboard: String?
    @Option(help: "Unix socket of the VM's camera port") var camera: String?
    @Flag(help: "Send a test pattern instead of the camera") var cameraTestPattern = false

    func run() throws {
        // The process ends when the VM closes a port (it is going away)
        let closed: @Sendable () -> Void = { Darwin.exit(0) }
        var parts: [AnyObject] = []
        // The old ports (images before plan B106); a send timeout, so a port nobody reads in a newer image never blocks
        let clip = try clipboard.map { ClipboardBridge(fd: try connect($0, sendTimeout: 2)) }
        let cam = try camera.map { CameraBridge(transport: .port(try connect($0)), testPattern: cameraTestPattern, onClosed: closed) }
        if let agent {
            guard let agentState else { throw ValidationError("--agent needs --agent-state") }
            let fd = try connect(agent)
            var allowed: Set<String> = ["status"]
            if clip != nil { allowed.insert("clipboard") }
            if cam != nil { allowed.insert("camera") }
            let link = AgentLink(fd: fd, stateFile: URL(fileURLWithPath: agentState), host: "omaflake", allowed: allowed,
                                 log: { FileHandle.standardError.write(Data("\($0)\n".utf8)) })
            // An image whose agent carries the clipboard and the camera: they move from the old ports to the agent
            link.onWelcome = { [weak link] features in
                guard let link else { return }
                if features.contains("clipboard") { clip?.useAgent(link) }
                if features.contains("camera") { cam?.useAgent(link) }
            }
            link.onClipboard = { mime, data in clip?.apply(mime: mime, data: data) }
            link.onCamera = { request in cam?.request(request) }
            Thread.detachNewThread {
                // The VM closed the port: it is going away. A guest that misbehaves only loses its agent link.
                if link.run() == .closed { closed() }
            }
            parts.append(link)
        }
        if let clip { clip.start(); parts.append(clip) }
        if let cam { cam.start(); parts.append(cam) }
        guard !parts.isEmpty else { throw ValidationError("--agent, --clipboard and/or --camera") }
        withExtendedLifetime(parts) { dispatchMain() }
    }

    /// Connects to the socket QEMU creates; the VM may still be starting.
    /// `sendTimeout`: seconds a write may wait for the guest to read (0: forever)
    func connect(_ path: String, sendTimeout: Int = 0) throws -> Int32 {
        let deadline = Date().addingTimeInterval(60)
        while true {
            if let fd = try? connectUnix(path, timeout: 2) {
                // The connect timeout also applies to reads – the bridge waits for the guest indefinitely
                var none = timeval(tv_sec: 0, tv_usec: 0)
                var send = timeval(tv_sec: sendTimeout, tv_usec: 0)
                setsockopt(fd, SOL_SOCKET, SO_RCVTIMEO, &none, socklen_t(MemoryLayout<timeval>.size))
                setsockopt(fd, SOL_SOCKET, SO_SNDTIMEO, &send, socklen_t(MemoryLayout<timeval>.size))
                return fd
            }
            guard Date() < deadline else { throw VMError("Socket not available: \(path)") }
            usleep(500_000)
        }
    }
}

final class ClipboardBridge: @unchecked Sendable {
    static let text = "text/plain;charset=utf-8", png = "image/png"
    static let maxBytes = 32 * 1024 * 1024

    let fd: Int32
    /// The VM closed the port: the bridge process ends; inside omaf-runner (macOS guests) the bridge just stops
    let onClosed: @Sendable () -> Void
    let pasteboard = NSPasteboard.general
    /// Writes off the main queue: a guest whose agent does not read yet must not block anything
    private let writeQueue = DispatchQueue(label: "omaflake.clipboard.write")
    /// What came from the guest and was written to the pasteboard, as its change count and fingerprint
    private var writtenChangeCount = -1
    private var lastChangeCount: Int
    private var lastSent: String?
    private let lock = NSLock()
    /// Set once the guest's agent carries the clipboard: then the content goes there instead of the old port
    private var agent: AgentLink?

    func useAgent(_ link: AgentLink) {
        lock.withLock { agent = link }
        log("clipboard: over the omaflake agent")
    }

    func log(_ text: String) {
        if ProcessInfo.processInfo.environment["OMAFLAKE_DEBUG"] != nil { FileHandle.standardError.write(Data("\(text)\n".utf8)) }
    }

    init(fd: Int32, onClosed: @escaping @Sendable () -> Void = { Darwin.exit(0) }) {
        self.fd = fd
        self.onClosed = onClosed
        lastChangeCount = NSPasteboard.general.changeCount   // only what is copied from now on
    }

    private var timer: DispatchSourceTimer?

    /// Reads the guest's messages on a thread, polls the pasteboard on the main queue (dispatchMain keeps it alive).
    func start() {
        Thread.detachNewThread { [self] in readLoop() }
        let t = DispatchSource.makeTimerSource(queue: .main)
        t.schedule(deadline: .now() + 0.5, repeating: 0.4)
        t.setEventHandler { [self] in poll() }
        t.resume()
        timer = t
    }

    static func fingerprint(_ mime: String, _ data: Data) -> String {
        var hasher = Hasher()
        hasher.combine(mime)
        hasher.combine(data)
        return "\(data.count)-\(hasher.finalize())"
    }

    /// Mac → guest: a new pasteboard content (not the one this bridge just wrote)
    func poll() {
        let count = pasteboard.changeCount
        guard count != lastChangeCount else { return }
        lastChangeCount = count
        guard count != lock.withLock({ writtenChangeCount }) else { return }
        guard let (mime, data) = currentPasteboard(), data.count <= Self.maxBytes else { return }
        let fp = Self.fingerprint(mime, data)
        guard fp != lastSent else { return }
        lastSent = fp
        send(mime, data)
    }

    func currentPasteboard() -> (String, Data)? {
        let types = pasteboard.types ?? []
        if types.contains(.png), let data = pasteboard.data(forType: .png) { return (Self.png, data) }
        if types.contains(.tiff), let tiff = pasteboard.data(forType: .tiff),
           let png = NSBitmapImageRep(data: tiff)?.representation(using: .png, properties: [:]) { return (Self.png, png) }
        if let s = pasteboard.string(forType: .string), let data = s.data(using: .utf8) { return (Self.text, data) }
        return nil
    }

    func send(_ mime: String, _ data: Data) {
        guard let json = try? JSONSerialization.data(withJSONObject: ["mime": mime, "data": data.base64EncodedString()]) else { return }
        if let agent = lock.withLock({ agent }) {
            writeQueue.async { [self] in
                if agent.send(.clipboard, payload: json) { log("clipboard: sent \(mime) \(data.count) bytes (agent)") }
            }
            return
        }
        var line = json
        line.append(0x0A)
        let payload = line
        writeQueue.async { [self] in
            payload.withUnsafeBytes { buf in
                var off = 0
                while off < buf.count {
                    let n = write(fd, buf.baseAddress! + off, buf.count - off)
                    if n < 0 && errno == EINTR { continue }
                    // Send timeout: nobody reads in the guest (agent not running yet) – drop this content
                    if n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK) { return }
                    if n <= 0 { onClosed(); return }   // VM gone
                    off += n
                }
                if ProcessInfo.processInfo.environment["OMAFLAKE_DEBUG"] != nil {
                    FileHandle.standardError.write(Data("clipboard: sent \(mime) \(data.count) bytes\n".utf8))
                }
            }
        }
    }

    /// Guest → Mac: lines from the port; the process ends when the VM closes it.
    func readLoop() {
        var buffer = Data()
        var chunk = [UInt8](repeating: 0, count: 65536)
        while true {
            let n = read(fd, &chunk, chunk.count)
            if n < 0 && errno == EINTR { continue }
            if n <= 0 { timer?.cancel(); onClosed(); return }   // the VM closed the port
            buffer.append(contentsOf: chunk[0..<n])
            while let nl = buffer.firstIndex(of: 0x0A) {
                let line = buffer[buffer.startIndex..<nl]
                buffer.removeSubrange(buffer.startIndex...nl)
                applyLine(Data(line))
            }
            if buffer.count > Self.maxBytes * 2 { buffer.removeAll() }   // a runaway line
        }
    }

    func applyLine(_ line: Data) {
        guard let obj = try? JSONSerialization.jsonObject(with: line) as? [String: String],
              let mime = obj["mime"], let b64 = obj["data"], let data = Data(base64Encoded: b64) else { return }
        apply(mime: mime, data: data)
    }

    /// Guest → Mac: put the content on the pasteboard (from the old port or the agent)
    func apply(mime: String, data: Data) {
        DispatchQueue.main.async { [self] in
            let fp = Self.fingerprint(mime, data)
            pasteboard.clearContents()
            switch mime {
            case Self.png: pasteboard.setData(data, forType: .png)
            case Self.text: pasteboard.setString(String(decoding: data, as: UTF8.self), forType: .string)
            default: return
            }
            lock.withLock { writtenChangeCount = pasteboard.changeCount }
            lastSent = fp   // the guest has it already
        }
    }
}
