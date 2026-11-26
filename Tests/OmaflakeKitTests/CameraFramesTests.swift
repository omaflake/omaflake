// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation
import Testing
@testable import OmaflakeKit

/// What omaflake's camera extension does with the agent's frames (plan B184)
@Suite(.timeLimit(.minutes(1)))
struct CameraFramesTests {
    @Test func aFrameIsCopiedRowByRowIntoLongerRows() {
        let width = 4, height = 2
        // Luma rows 1 2 3 4 / 5 6 7 8, one chroma row 9 10 11 12
        let frame: [UInt8] = Array(1...12)
        #expect(CameraFrames.bytes(width: width, height: height) == 12)
        var luma = [UInt8](repeating: 0, count: 6 * 2), chroma = [UInt8](repeating: 0, count: 8)
        let copied = luma.withUnsafeMutableBytes { l in
            chroma.withUnsafeMutableBytes { c in
                frame.withUnsafeBytes {
                    CameraFrames.copy($0, width: width, height: height,
                                      luma: .init(base: l.baseAddress!, bytesPerRow: 6),
                                      chroma: .init(base: c.baseAddress!, bytesPerRow: 8))
                }
            }
        }
        #expect(copied)
        #expect(luma == [1, 2, 3, 4, 0, 0, 5, 6, 7, 8, 0, 0])
        #expect(chroma == [9, 10, 11, 12, 0, 0, 0, 0])
        // A frame too short, rows too short: nothing is touched
        var small = [UInt8](repeating: 0, count: 6)
        let refused = small.withUnsafeMutableBytes { s in
            frame.withUnsafeBytes {
                CameraFrames.copy($0, width: width, height: height, luma: .init(base: s.baseAddress!, bytesPerRow: 3),
                                  chroma: .init(base: s.baseAddress!, bytesPerRow: 3))
                    || CameraFrames.copy(UnsafeRawBufferPointer(rebasing: $0[0..<11]), width: width, height: height,
                                         luma: .init(base: s.baseAddress!, bytesPerRow: 4),
                                         chroma: .init(base: s.baseAddress!, bytesPerRow: 4))
            }
        }
        #expect(!refused && small == [0, 0, 0, 0, 0, 0])
    }

    @Test func theTestPatternIsGreyWithABarThatMoves() {
        var frame: [UInt8] = []
        CameraFrames.testPattern(into: &frame, width: 160, height: 4, count: 0)
        #expect(frame.count == 160 * 4 * 3 / 2)
        #expect(frame[0] == 220 && frame[9] == 220 && frame[10] == 90, "the bar is a sixteenth wide, at the left")
        #expect(frame[(160 * 4)...].allSatisfy { $0 == 128 }, "no colour")
        CameraFrames.testPattern(into: &frame, width: 160, height: 4, count: 5)
        #expect(frame[0] == 90 && frame[10] == 220 && frame[19] == 220 && frame[20] == 90, "two pixels a picture")
    }

    /// A listener on a free port of 127.0.0.1 that sends `bytes` to its first client and closes
    private func serve(_ bytes: [UInt8]) throws -> UInt16 {
        let s = socket(AF_INET, SOCK_STREAM, 0)
        var address = sockaddr_in()
        address.sin_len = UInt8(MemoryLayout<sockaddr_in>.size)
        address.sin_family = sa_family_t(AF_INET)
        address.sin_addr.s_addr = inet_addr("127.0.0.1")
        var length = socklen_t(MemoryLayout<sockaddr_in>.size)
        let bound = withUnsafeMutablePointer(to: &address) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { p -> Bool in
                bind(s, p, length) == 0 && listen(s, 1) == 0 && getsockname(s, p, &length) == 0
            }
        }
        try #require(bound)
        Thread.detachNewThread {
            let client = accept(s, nil, nil)
            // In two parts, the cut inside a frame: the reader must put them together
            let half = min(bytes.count, bytes.count / 2 + 1)
            _ = bytes[..<half].withUnsafeBytes { write(client, $0.baseAddress, $0.count) }
            usleep(50_000)
            _ = bytes[half...].withUnsafeBytes { write(client, $0.baseAddress, $0.count) }
            close(client)
            close(s)
        }
        return UInt16(bigEndian: address.sin_port)
    }

    @Test func theReaderGivesWholeFramesAndStopsAtACutOne() throws {
        // Two frames of 6 bytes and half of a third
        let port = try serve(Array(1...15))
        let reader = CameraStreamReader(port: port, timeout: 2)
        #expect(!reader.isConnected && reader.connect() && reader.isConnected)
        var frame = [UInt8](repeating: 0, count: 6)
        #expect(reader.read(into: &frame) && frame == [1, 2, 3, 4, 5, 6])
        #expect(reader.read(into: &frame) && frame == [7, 8, 9, 10, 11, 12])
        #expect(!reader.read(into: &frame), "the stream ended inside a frame")
        #expect(!reader.isConnected && !reader.read(into: &frame))
    }

    @Test func nothingListeningIsNoConnection() throws {
        // A port that was free a moment ago: bound, read back and closed again
        let port = try serve([])
        let first = CameraStreamReader(port: port)
        #expect(first.connect())
        var one = [UInt8](repeating: 0, count: 1)
        #expect(!first.read(into: &one), "closed without a byte")
        usleep(100_000)
        #expect(!CameraStreamReader(port: port).connect(), "the listener is gone")
    }
}
