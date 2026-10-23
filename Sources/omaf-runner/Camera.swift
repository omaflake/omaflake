// SPDX-License-Identifier: AGPL-3.0-or-later
import AVFoundation
import CoreVideo
import Darwin
import Foundation
import OmaflakeKit

/// Mac side of the camera channel: captures the Mac's camera only while the guest asks for it and sends 1280×720 NV12
/// frames. Two transports: the omaflake agent (images from plan B106 on – frames only against the guest's credits,
/// spec §11.9) or the old port org.omaflake.camera ("start"/"stop" lines from the guest; messages with a 16-byte
/// header "<4sBBHII": b"OMFC", version 1, kind 1 status JSON / 2 frame, 0, payload bytes, sequence).
/// `testPattern` sends a moving pattern instead of the camera (checks without camera permission).
final class CameraBridge: NSObject, AVCaptureVideoDataOutputSampleBufferDelegate, @unchecked Sendable {
    static let width = 1280, height = 720
    static let frameBytes = width * height * 3 / 2

    enum Transport { case port(Int32), agent(AgentLink) }

    let testPattern: Bool
    let onClosed: @Sendable () -> Void
    private let writeLock = NSLock()
    private let queue = DispatchQueue(label: "omaflake.camera")
    private var transport: Transport   // on `queue` once started
    private let portFD: Int32?
    private var session: AVCaptureSession?
    private var pattern: DispatchSourceTimer?
    private var sequence: UInt32 = 0
    private var frame = [UInt8](repeating: 0, count: CameraBridge.frameBytes)
    /// Agent: frames the guest still accepts, and when it last granted some (no credit for 2 s stops the camera)
    private var credits = 0
    private var lastCredit = Date.distantPast
    /// The newest frame that found no credit – sent as soon as one comes (older ones are replaced: live video)
    private var pending: [UInt8]?

    init(transport: Transport, testPattern: Bool, onClosed: @escaping @Sendable () -> Void) {
        self.transport = transport
        if case .port(let fd) = transport { portFD = fd } else { portFD = nil }
        self.testPattern = testPattern
        self.onClosed = onClosed
    }

    convenience init(fd: Int32, testPattern: Bool, onClosed: @escaping @Sendable () -> Void) {
        self.init(transport: .port(fd), testPattern: testPattern, onClosed: onClosed)
    }

    func start() {
        // "ready": the guest repeats "start" if an app opened the camera before the bridge was connected
        queue.async { [self] in status("ready", reason: testPattern ? "test pattern" : "camera") }
        if portFD != nil { Thread.detachNewThread { [self] in readLoop() } }
    }

    /// The guest's agent carries the camera (its welcome enabled it): from now on frames go there, against credits
    func useAgent(_ link: AgentLink) {
        queue.async { [self] in
            stopCapture()
            transport = .agent(link)
            credits = 0
            status("ready", reason: testPattern ? "test pattern" : "camera")
        }
    }

    /// Start, stop and credits from the agent
    func request(_ r: AgentLink.CameraRequest) {
        queue.async { [self] in
            switch r {
            case .start: startCapture()
            case .stop: stopCapture(); credits = 0; pending = nil
            case .credits(let n):
                credits = min(credits + n, AgentProtocol.maxCameraCredits)
                lastCredit = Date()
                if case .agent(let link) = transport, let frame = pending {
                    pending = nil
                    sendAgent(link, kind: 2, frame)
                }
            }
        }
    }

    /// "start"/"stop" lines from the guest
    private func readLoop() {
        guard let fd = portFD else { return }
        var buffer = Data()
        var chunk = [UInt8](repeating: 0, count: 4096)
        while true {
            let n = read(fd, &chunk, chunk.count)
            if n < 0 && errno == EINTR { continue }
            if n <= 0 { queue.sync { stopCapture() }; onClosed(); return }
            buffer.append(contentsOf: chunk[0..<n])
            while let nl = buffer.firstIndex(of: 0x0A) {
                let line = buffer[buffer.startIndex..<nl]
                buffer.removeSubrange(buffer.startIndex...nl)
                guard let obj = try? JSONSerialization.jsonObject(with: Data(line)) as? [String: String] else { continue }
                switch obj["type"] {
                case "start": queue.async { [self] in startCapture() }
                case "stop": queue.async { [self] in stopCapture() }
                default: break
                }
            }
        }
    }

    // MARK: Capture (on `queue`)

    private func startCapture() {
        guard session == nil, pattern == nil else { return }
        if testPattern { return startPattern() }
        switch AVCaptureDevice.authorizationStatus(for: .video) {
        case .authorized: break
        case .notDetermined:
            // macOS asks the user (for the app that started the VM, or Terminal)
            AVCaptureDevice.requestAccess(for: .video) { [self] granted in
                queue.async { [self] in granted ? startCapture() : status("unavailable", reason: "camera permission denied") }
            }
            return
        default:
            return status("unavailable", reason: "camera permission denied (System Settings → Privacy & Security → Camera)")
        }
        guard let device = AVCaptureDevice.default(for: .video), let input = try? AVCaptureDeviceInput(device: device) else {
            return status("unavailable", reason: "no camera found")
        }
        let s = AVCaptureSession()
        s.beginConfiguration()
        if s.canSetSessionPreset(.hd1280x720) { s.sessionPreset = .hd1280x720 }
        guard s.canAddInput(input) else { return status("unavailable", reason: "camera busy") }
        s.addInput(input)
        let output = AVCaptureVideoDataOutput()
        // NV12 (video range) at exactly 1280×720 – AVFoundation scales when the camera delivers another size
        output.videoSettings = [
            kCVPixelBufferPixelFormatTypeKey as String: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
            kCVPixelBufferWidthKey as String: Self.width,
            kCVPixelBufferHeightKey as String: Self.height,
        ]
        output.alwaysDiscardsLateVideoFrames = true
        output.setSampleBufferDelegate(self, queue: queue)
        guard s.canAddOutput(output) else { return status("unavailable", reason: "cannot capture") }
        s.addOutput(output)
        s.commitConfiguration()
        s.startRunning()
        session = s
        status("streaming", reason: device.localizedName)
    }

    private func stopCapture() {
        session?.stopRunning()
        session = nil
        pattern?.cancel()
        pattern = nil
    }

    func captureOutput(_ output: AVCaptureOutput, didOutput sampleBuffer: CMSampleBuffer, from connection: AVCaptureConnection) {
        guard let pb = CMSampleBufferGetImageBuffer(sampleBuffer),
              CVPixelBufferGetWidth(pb) == Self.width, CVPixelBufferGetHeight(pb) == Self.height,
              CVPixelBufferGetPlaneCount(pb) == 2 else { return }
        CVPixelBufferLockBaseAddress(pb, .readOnly)
        defer { CVPixelBufferUnlockBaseAddress(pb, .readOnly) }
        // Rows may be padded (bytesPerRow > width): copy plane by plane, row by row
        var off = 0
        frame.withUnsafeMutableBytes { dst in
            for plane in 0..<2 {
                guard let base = CVPixelBufferGetBaseAddressOfPlane(pb, plane) else { return }
                let rows = CVPixelBufferGetHeightOfPlane(pb, plane), stride = CVPixelBufferGetBytesPerRowOfPlane(pb, plane)
                for r in 0..<rows {
                    memcpy(dst.baseAddress! + off, base + r * stride, Self.width)
                    off += Self.width
                }
            }
        }
        send(kind: 2, frame)
    }

    /// Moving gray bars instead of the camera, at 15 fps or OMAFLAKE_CAMERA_TEST_FPS (throughput tests)
    private func startPattern() {
        let fps = ProcessInfo.processInfo.environment["OMAFLAKE_CAMERA_TEST_FPS"].flatMap(Double.init) ?? 15
        let t = DispatchSource.makeTimerSource(queue: queue)
        t.schedule(deadline: .now(), repeating: 1.0 / max(1, min(fps, 60)))
        var tick = 0
        t.setEventHandler { [self] in
            tick += 1
            // One row of bars, copied to every row (cheap enough for a debug build)
            let w = Self.width, h = Self.height
            let row = (0..<w).map { x in UInt8(16 + ((x + tick * 8) / 40 % 2) * 200) }
            frame.withUnsafeMutableBytes { dst in
                row.withUnsafeBytes { src in for y in 0..<h { memcpy(dst.baseAddress! + y * w, src.baseAddress!, w) } }
                memset(dst.baseAddress! + w * h, 128, dst.count - w * h)
            }
            send(kind: 2, frame)
        }
        t.resume()
        pattern = t
        status("streaming", reason: "test pattern")
    }

    private func status(_ value: String, reason: String) {
        let json = (try? JSONSerialization.data(withJSONObject: ["status": value, "detail": reason])) ?? Data()
        send(kind: 1, [UInt8](json))
    }

    // MARK: Writing

    private func send(kind: UInt8, _ payload: [UInt8]) {
        if case .agent(let link) = transport {
            return sendAgent(link, kind: kind, payload)
        }
        guard case .port(let fd) = transport else { return }
        var header = [UInt8](repeating: 0, count: 16)
        header.replaceSubrange(0..<4, with: Array("OMFC".utf8))
        header[4] = 1
        header[5] = kind
        withUnsafeBytes(of: UInt32(payload.count).littleEndian) { header.replaceSubrange(8..<12, with: $0) }
        sequence &+= 1
        withUnsafeBytes(of: sequence.littleEndian) { header.replaceSubrange(12..<16, with: $0) }
        writeLock.withLock {
            guard writeAll(fd, header), writeAll(fd, payload) else {
                stopCapture()
                return
            }
        }
    }

    /// Over the agent (on `queue`): a frame only against a credit, otherwise dropped – live video is never queued;
    /// no credit for 2 s while the camera runs means the guest stopped reading: the camera goes off
    private func sendAgent(_ link: AgentLink, kind: UInt8, _ payload: [UInt8]) {
        if kind == 1 {
            link.send(.cameraStatus, payload: Data(payload))
            return
        }
        guard credits > 0 else {
            if Date().timeIntervalSince(lastCredit) > 2 {
                stopCapture()
                pending = nil
                status("stopped", reason: "no credit from the guest for 2 s")
            } else {
                pending = payload   // goes out with the next credit, unless a newer frame comes first
            }
            return
        }
        credits -= 1
        if !link.send(.cameraFrame, payload: Data(payload)) { stopCapture() }
    }

    private func writeAll(_ fd: Int32, _ bytes: [UInt8]) -> Bool {
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
