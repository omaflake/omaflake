// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation
import Testing
@testable import OmaflakeKit

/// Writing into a guest port whose writes time out: whole messages or none (plan B168).
/// A test that hangs fails after a minute instead of holding a CI runner for six hours (plan B176)
@Suite(.timeLimit(.minutes(1)))
struct PortWriterTests {
    /// A socket pair as the runner makes it: our end with a send timeout (here short) and a small send buffer
    static func pair(timeout: Int32 = 100_000, buffer: Int32 = 8192) -> (ours: Int32, guest: Int32) {
        var fds: [Int32] = [0, 0]
        #expect(socketpair(AF_UNIX, SOCK_STREAM, 0, &fds) == 0)
        var tv = timeval(tv_sec: 0, tv_usec: timeout)
        setsockopt(fds[0], SOL_SOCKET, SO_SNDTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
        var size = buffer
        setsockopt(fds[0], SOL_SOCKET, SO_SNDBUF, &size, socklen_t(MemoryLayout<Int32>.size))
        setsockopt(fds[1], SOL_SOCKET, SO_RCVBUF, &size, socklen_t(MemoryLayout<Int32>.size))
        var on: Int32 = 1
        setsockopt(fds[0], SOL_SOCKET, SO_NOSIGPIPE, &on, socklen_t(MemoryLayout<Int32>.size))
        return (fds[0], fds[1])
    }

    /// Everything the guest end holds right now
    static func drain(_ fd: Int32) -> [UInt8] {
        let flags = fcntl(fd, F_GETFL)
        _ = fcntl(fd, F_SETFL, flags | O_NONBLOCK)
        defer { _ = fcntl(fd, F_SETFL, flags) }
        var out = [UInt8](), chunk = [UInt8](repeating: 0, count: 65536)
        while true {
            let n = read(fd, &chunk, chunk.count)
            if n <= 0 { return out }
            out += chunk[0..<n]
        }
    }

    /// Fills our end until the socket takes no more
    static func fill(_ fd: Int32) {
        let flags = fcntl(fd, F_GETFL)
        _ = fcntl(fd, F_SETFL, flags | O_NONBLOCK)
        defer { _ = fcntl(fd, F_SETFL, flags) }
        let junk = [UInt8](repeating: 0xEE, count: 4096)
        while write(fd, junk, junk.count) > 0 {}
    }

    /// The guest reading `count` bytes on another thread, starting after `delay` seconds
    final class Reader: @unchecked Sendable {
        private var bytes = [UInt8]()
        private let done = DispatchSemaphore(value: 0)

        init(_ fd: Int32, count: Int, after delay: TimeInterval) {
            DispatchQueue.global().asyncAfter(deadline: .now() + delay) { [self] in
                var chunk = [UInt8](repeating: 0, count: 65536)
                while bytes.count < count {
                    let n = Darwin.read(fd, &chunk, chunk.count)
                    if n <= 0 { break }
                    bytes += chunk[0..<n]
                }
                done.signal()
            }
        }

        func wait() -> [UInt8] {
            done.wait()
            return bytes
        }
    }

    static func message(_ count: Int) -> [UInt8] { (0..<count).map { UInt8($0 % 251) } }

    @Test func aWholeMessageWhileTheGuestReads() {
        let (ours, guest) = Self.pair()
        defer { close(ours); close(guest) }
        let sent = Self.message(1_382_400)   // one 1280×720 NV12 frame
        let received = Reader(guest, count: sent.count, after: 0)
        #expect(PortWriter(fd: ours).write(sent) == .written)
        #expect(received.wait() == sent)
    }

    @Test func aFullPortDropsTheMessageWithoutWritingAnyOfIt() {
        let (ours, guest) = Self.pair()
        defer { close(ours); close(guest) }
        Self.fill(ours)
        #expect(PortWriter(fd: ours).write(Self.message(100)) == .dropped)
        // Only the filling is in the port – not a byte of the dropped message
        #expect(Self.drain(guest).allSatisfy { $0 == 0xEE })
    }

    @Test func aBegunMessageIsFinishedWhenTheGuestReadsLate() {
        let (ours, guest) = Self.pair()
        defer { close(ours); close(guest) }
        let sent = Self.message(200_000)   // far more than the buffers: it stalls past the 0.1 s timeout
        let received = Reader(guest, count: sent.count, after: 0.5)
        #expect(PortWriter(fd: ours, finishWithin: 5).write(sent) == .written)
        #expect(received.wait() == sent)
    }

    @Test func aMessageStuckHalfwayFails() {
        let (ours, guest) = Self.pair()
        defer { close(ours); close(guest) }
        #expect(PortWriter(fd: ours, finishWithin: 0.3).write(Self.message(200_000)) == .failed)
    }

    @Test func aClosedPortFails() {
        let (ours, guest) = Self.pair()
        defer { close(ours) }
        close(guest)
        #expect(PortWriter(fd: ours).write(Self.message(100)) == .failed)
    }
}
