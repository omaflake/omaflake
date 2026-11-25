// SPDX-License-Identifier: AGPL-3.0-or-later
import CoreMedia
import CoreVideo
import Foundation
import Testing
@testable import OmaflakeKit

/// The camera's H.264 over the agent (plan B174)
struct CameraEncoderTests {
    @Test func lengthPrefixedUnitsGetStartCodesAndTheParameterSetsFirst() throws {
        let avcc: [UInt8] = [0, 0, 0, 2, 0x65, 0xAA, 0, 0, 0, 1, 0x06]
        let out = try #require(CameraEncoder.annexB(avcc: avcc, lengthSize: 4, parameterSets: [[0x67, 1], [0x68, 2]]))
        #expect(out == [0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x68, 2, 0, 0, 0, 1, 0x65, 0xAA, 0, 0, 0, 1, 0x06])
        #expect(CameraEncoder.annexB(avcc: [0, 2, 0x41, 0x9A], lengthSize: 2) == [0, 0, 0, 1, 0x41, 0x9A])
        #expect(CameraEncoder.annexB(avcc: [0, 0, 0, 9, 0x65], lengthSize: 4) == nil, "a length past the end")
        #expect(CameraEncoder.annexB(avcc: [0, 0, 0, 0], lengthSize: 4) == nil, "an empty unit")
        #expect(CameraEncoder.annexB(avcc: [0, 0], lengthSize: 4) == nil, "a cut length")
    }

    /// NAL unit types in an Annex B access unit (4-byte start codes, as `annexB` writes them)
    static func nalTypes(_ unit: [UInt8]) -> [UInt8] {
        var types: [UInt8] = []
        var i = 0
        while i + 4 < unit.count {
            if unit[i] == 0, unit[i + 1] == 0, unit[i + 2] == 0, unit[i + 3] == 1 {
                types.append(unit[i + 4] & 0x1F)
                i += 4
            } else {
                i += 1
            }
        }
        return types
    }

    final class Collected: @unchecked Sendable {
        let lock = NSLock()
        var units: [(bytes: [UInt8], key: Bool)] = []
        func add(_ b: [UInt8], _ k: Bool) { lock.withLock { units.append((b, k)) } }
        var all: [(bytes: [UInt8], key: Bool)] { lock.withLock { units } }
    }

    /// Real frames through VideoToolbox: a key frame with SPS and PPS first, then frames that refer to it, and a key
    /// frame again on request; far smaller than the raw frame
    @Test func encodesKeyFramesWithParameterSetsAndOnRequest() throws {
        let collected = Collected()
        let encoder = try CameraEncoder(width: 1280, height: 720) { bytes, key in collected.add(bytes, key) }
        var pb: CVPixelBuffer?
        CVPixelBufferCreate(nil, 1280, 720, kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                            [kCVPixelBufferIOSurfacePropertiesKey: [:]] as CFDictionary, &pb)
        let buffer = try #require(pb)
        for n in 0..<12 {
            CVPixelBufferLockBaseAddress(buffer, [])
            let y = CVPixelBufferGetBaseAddressOfPlane(buffer, 0)!, uv = CVPixelBufferGetBaseAddressOfPlane(buffer, 1)!
            let yStride = CVPixelBufferGetBytesPerRowOfPlane(buffer, 0)
            for row in 0..<720 { memset(y + row * yStride, Int32(16 + (row + n * 8) % 200), 1280) }
            memset(uv, 128, CVPixelBufferGetBytesPerRowOfPlane(buffer, 1) * 360)
            CVPixelBufferUnlockBaseAddress(buffer, [])
            if n == 8 { encoder.requestKeyFrame() }
            #expect(encoder.encode(buffer, at: CMTime(value: CMTimeValue(n), timescale: 30)))
        }
        encoder.flush()
        let units = collected.all
        #expect(units.count == 12)
        #expect(units.allSatisfy { !$0.bytes.isEmpty })
        let first = try #require(units.first)
        #expect(first.key)
        #expect(Array(Self.nalTypes(first.bytes).prefix(2)) == [7, 8], "SPS and PPS first")
        #expect(Self.nalTypes(first.bytes).contains(5), "an IDR slice")
        #expect(!units[1].key && !Self.nalTypes(units[1].bytes).contains(7))
        #expect(units[8].key && Self.nalTypes(units[8].bytes).prefix(2) == [7, 8], "the requested key frame")
        #expect(units.map(\.bytes.count).max()! < 1280 * 720 * 3 / 2 / 4)
    }
}
