// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// The camera stream in a macOS guest as omaflake's camera extension reads it (plan B184; spec §11.9): raw NV12
/// frames, one after the other with nothing between them, from the agent on `127.0.0.1:47042` (camera_mac.rs).
/// What can be tested without CoreMediaIO lives here; the extension itself is Sources/omaflake-camera.
public enum CameraFrames {
    public static let width = 1280
    public static let height = 720
    public static let port: UInt16 = 47042

    /// Bytes of one NV12 frame: the luma plane and the half-height plane of chroma pairs
    public static func bytes(width: Int, height: Int) -> Int { width * height * 3 / 2 }

    /// One plane of a pixel buffer: where it starts and how long a row is there (at least the picture's width)
    public struct Plane {
        public let base: UnsafeMutableRawPointer
        public let bytesPerRow: Int
        public init(base: UnsafeMutableRawPointer, bytesPerRow: Int) {
            self.base = base
            self.bytesPerRow = bytesPerRow
        }
    }

    /// Copies a frame into a pixel buffer's two planes, row by row – a buffer's rows may be longer than the
    /// picture's. False (nothing copied) when the frame or a plane's rows are too short
    @discardableResult
    public static func copy(_ frame: UnsafeRawBufferPointer, width: Int, height: Int, luma: Plane, chroma: Plane) -> Bool {
        guard frame.count >= bytes(width: width, height: height), let source = frame.baseAddress,
              luma.bytesPerRow >= width, chroma.bytesPerRow >= width else { return false }
        var offset = 0
        for (plane, rows) in [(luma, height), (chroma, height / 2)] {
            for row in 0..<rows {
                memcpy(plane.base + row * plane.bytesPerRow, source + offset + row * width, width)
            }
            offset += rows * width
        }
        return true
    }

    /// The picture while no frames come: grey with a bright bar that moves to the right with `count`
    public static func testPattern(into frame: inout [UInt8], width: Int, height: Int, count: Int) {
        let total = bytes(width: width, height: height)
        if frame.count != total { frame = [UInt8](repeating: 0, count: total) }
        let barWidth = max(width / 16, 1)
        let bar = (count * max(width / 80, 1)) % width
        for y in 0..<height {
            for x in 0..<width { frame[y * width + x] = (x >= bar && x < bar + barWidth) ? 220 : 90 }
        }
        for i in (width * height)..<total { frame[i] = 128 }
    }
}

/// A blocking reader of the agent's stream: whole frames, or false when the stream ended or stalled
public final class CameraStreamReader {
    private var fd: Int32 = -1
    private let port: UInt16
    private let timeout: Int

    /// `timeout`: seconds without a byte after which `read` gives up, so the caller can look whether it is still
    /// wanted
    public init(port: UInt16 = CameraFrames.port, timeout: Int = 2) {
        self.port = port
        self.timeout = timeout
    }

    deinit { close() }

    public var isConnected: Bool { fd >= 0 }

    /// Connects to 127.0.0.1; false when nothing listens there (no agent, or the camera is off for this VM)
    public func connect() -> Bool {
        close()
        let s = socket(AF_INET, SOCK_STREAM, 0)
        guard s >= 0 else { return false }
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_port = port.bigEndian
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        var wait = timeval(tv_sec: timeout, tv_usec: 0)
        setsockopt(s, SOL_SOCKET, SO_RCVTIMEO, &wait, socklen_t(MemoryLayout<timeval>.size))
        let result = withUnsafePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) {
                Darwin.connect(s, $0, socklen_t(MemoryLayout<sockaddr_in>.size))
            }
        }
        guard result == 0 else {
            Darwin.close(s)
            return false
        }
        fd = s
        return true
    }

    /// Fills `frame` with the next whole frame. False: the agent closed the stream, or nothing came for the
    /// timeout – the connection is closed then (a frame cut in the middle is never handed on)
    public func read(into frame: inout [UInt8]) -> Bool {
        guard fd >= 0 else { return false }
        let total = frame.count
        var got = 0
        while got < total {
            let n = frame.withUnsafeMutableBytes { Darwin.read(fd, $0.baseAddress! + got, total - got) }
            if n < 0, errno == EINTR { continue }
            if n <= 0 {
                close()
                return false
            }
            got += n
        }
        return true
    }

    public func close() {
        if fd >= 0 { Darwin.close(fd) }
        fd = -1
    }
}
