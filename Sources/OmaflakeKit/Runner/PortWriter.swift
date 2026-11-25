// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

/// Writes whole messages into a guest port's socket whose writes time out (`SO_SNDTIMEO`, the runner's port pairs:
/// a guest that does not read must not block a bridge forever). A message either goes out whole or not at all – a
/// half-written frame would leave the guest's reader out of step (plan B168).
public struct PortWriter {
    public enum Outcome: Equatable, Sendable {
        /// The whole message is in the socket
        case written
        /// Not a byte went out before the timeout: the guest is not reading right now – live video drops this one
        case dropped
        /// The port is closed, or a message stuck halfway for longer than `finishWithin`
        case failed
    }

    public let fd: Int32
    /// How long a message that has begun may take to finish
    public let finishWithin: TimeInterval

    public init(fd: Int32, finishWithin: TimeInterval = 5) {
        self.fd = fd
        self.finishWithin = finishWithin
    }

    public func write(_ bytes: [UInt8]) -> Outcome {
        bytes.withUnsafeBytes { buf in
            var off = 0
            let started = Date()
            while off < buf.count {
                let n = Darwin.write(fd, buf.baseAddress! + off, buf.count - off)
                if n > 0 {
                    off += n
                    continue
                }
                if n < 0 && errno == EINTR { continue }
                if n < 0 && (errno == EAGAIN || errno == EWOULDBLOCK) {
                    if off == 0 { return .dropped }
                    // Begun: finish it, so the next message starts where the guest expects a header
                    if Date().timeIntervalSince(started) > finishWithin { return .failed }
                    continue
                }
                return .failed
            }
            return .written
        }
    }
}
