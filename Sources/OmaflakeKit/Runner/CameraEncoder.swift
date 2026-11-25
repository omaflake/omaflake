// SPDX-License-Identifier: AGPL-3.0-or-later
import CoreMedia
import CoreVideo
import Foundation
import VideoToolbox

/// H.264 for the camera over the agent (plan B174): the virtio console carries ~25 raw 1280×720 NV12 frames a
/// second, the camera gives 30. With the agent's `camera-h264` each frame goes out as one H.264 access unit in
/// Annex B (start codes; the parameter sets before every key frame), which OBS in a macOS guest reads as it comes.
///
/// Real time, no frame reordering (every frame can be shown as soon as it arrives), a key frame every 2 s and on
/// request (a new reader in the guest). Frames the guest has no credit for are never encoded, so the stream never
/// misses a frame it refers to. `output` runs on VideoToolbox's thread, one frame at a time, in order.
public final class CameraEncoder: @unchecked Sendable {
    public typealias Output = @Sendable (_ accessUnit: [UInt8], _ keyFrame: Bool) -> Void

    public enum Failure: Error, CustomStringConvertible {
        case create(OSStatus)
        public var description: String {
            switch self {
            case .create(let s): "the H.264 encoder could not be created (\(s))"
            }
        }
    }

    private let session: VTCompressionSession
    private let output: Output
    private let lock = NSLock()
    private var forceKeyFrame = true

    /// `bitRate` in bits a second: 8 Mbit/s keep 720p30 sharp at a fifth of the console's capacity
    public init(width: Int, height: Int, fps: Int = 30, bitRate: Int = 8_000_000, output: @escaping Output) throws {
        self.output = output
        var created: VTCompressionSession?
        // Low-latency rate control where the encoder offers it (Apple silicon); else the encoder's default
        let lowLatency = [kVTVideoEncoderSpecification_EnableLowLatencyRateControl as String: true] as CFDictionary
        var status = VTCompressionSessionCreate(allocator: nil, width: Int32(width), height: Int32(height),
                                                codecType: kCMVideoCodecType_H264, encoderSpecification: lowLatency,
                                                imageBufferAttributes: nil, compressedDataAllocator: nil,
                                                outputCallback: nil, refcon: nil, compressionSessionOut: &created)
        if status != noErr {
            status = VTCompressionSessionCreate(allocator: nil, width: Int32(width), height: Int32(height),
                                                codecType: kCMVideoCodecType_H264, encoderSpecification: nil,
                                                imageBufferAttributes: nil, compressedDataAllocator: nil,
                                                outputCallback: nil, refcon: nil, compressionSessionOut: &created)
        }
        guard status == noErr, let created else { throw Failure.create(status) }
        session = created
        let properties: [CFString: Any] = [
            kVTCompressionPropertyKey_RealTime: true,
            kVTCompressionPropertyKey_AllowFrameReordering: false,
            kVTCompressionPropertyKey_ProfileLevel: kVTProfileLevel_H264_Main_AutoLevel,
            kVTCompressionPropertyKey_AverageBitRate: bitRate,
            kVTCompressionPropertyKey_ExpectedFrameRate: fps,
            kVTCompressionPropertyKey_MaxKeyFrameInterval: fps * 2,
            kVTCompressionPropertyKey_MaxKeyFrameIntervalDuration: 2,
        ]
        for (key, value) in properties {
            // Not every encoder takes every property (low-latency mode sets some itself): best effort
            _ = VTSessionSetProperty(session, key: key, value: value as CFTypeRef)
        }
        VTCompressionSessionPrepareToEncodeFrames(session)
    }

    deinit {
        VTCompressionSessionInvalidate(session)
    }

    /// The next frame becomes a key frame (with the parameter sets): a new reader can start there
    public func requestKeyFrame() {
        lock.withLock { forceKeyFrame = true }
    }

    /// Encodes one frame; false: the encoder refused it (no output follows, the caller keeps its credit)
    @discardableResult
    public func encode(_ pixelBuffer: CVPixelBuffer, at time: CMTime) -> Bool {
        let key = lock.withLock { () -> Bool in
            defer { forceKeyFrame = false }
            return forceKeyFrame
        }
        let options = key ? [kVTEncodeFrameOptionKey_ForceKeyFrame: true] as CFDictionary : nil
        let output = output
        let status = VTCompressionSessionEncodeFrame(session, imageBuffer: pixelBuffer, presentationTimeStamp: time,
                                                     duration: .invalid, frameProperties: options,
                                                     infoFlagsOut: nil) { status, _, sample in
            guard status == noErr, let sample, let unit = Self.accessUnit(sample) else {
                output([], false)   // dropped by the encoder: the caller gives the credit back
                return
            }
            output(unit.bytes, unit.keyFrame)
        }
        if status != noErr, key { requestKeyFrame() }
        return status == noErr
    }

    /// Waits for every frame handed in so far (tests)
    public func flush() {
        VTCompressionSessionCompleteFrames(session, untilPresentationTimeStamp: .invalid)
    }

    /// One encoded frame as Annex B: the parameter sets first when it is a key frame
    static func accessUnit(_ sample: CMSampleBuffer) -> (bytes: [UInt8], keyFrame: Bool)? {
        guard let block = CMSampleBufferGetDataBuffer(sample), let format = CMSampleBufferGetFormatDescription(sample)
        else { return nil }
        let attachments = CMSampleBufferGetSampleAttachmentsArray(sample, createIfNecessary: false) as? [[CFString: Any]]
        let keyFrame = !(attachments?.first?[kCMSampleAttachmentKey_NotSync] as? Bool ?? false)
        var avcc = [UInt8](repeating: 0, count: CMBlockBufferGetDataLength(block))
        guard CMBlockBufferCopyDataBytes(block, atOffset: 0, dataLength: avcc.count, destination: &avcc) == noErr
        else { return nil }
        var count = 0, lengthSize: Int32 = 4
        CMVideoFormatDescriptionGetH264ParameterSetAtIndex(format, parameterSetIndex: 0, parameterSetPointerOut: nil,
                                                           parameterSetSizeOut: nil, parameterSetCountOut: &count,
                                                           nalUnitHeaderLengthOut: &lengthSize)
        var sets: [[UInt8]] = []
        if keyFrame {
            for i in 0..<count {
                var pointer: UnsafePointer<UInt8>?
                var size = 0
                guard CMVideoFormatDescriptionGetH264ParameterSetAtIndex(
                    format, parameterSetIndex: i, parameterSetPointerOut: &pointer, parameterSetSizeOut: &size,
                    parameterSetCountOut: nil, nalUnitHeaderLengthOut: nil) == noErr, let pointer else { return nil }
                sets.append(Array(UnsafeBufferPointer(start: pointer, count: size)))
            }
        }
        guard let bytes = annexB(avcc: avcc, lengthSize: Int(lengthSize), parameterSets: sets) else { return nil }
        return (bytes, keyFrame)
    }

    /// NAL units with big-endian length prefixes (`lengthSize` bytes) → with start codes, `parameterSets` first;
    /// nil when a length runs past the end
    public static func annexB(avcc: [UInt8], lengthSize: Int, parameterSets: [[UInt8]] = []) -> [UInt8]? {
        guard (1...4).contains(lengthSize) else { return nil }
        let start: [UInt8] = [0, 0, 0, 1]
        var out: [UInt8] = []
        out.reserveCapacity(avcc.count + parameterSets.reduce(0) { $0 + $1.count + 4 })
        for set in parameterSets { out += start + set }
        var i = 0
        while i < avcc.count {
            guard i + lengthSize <= avcc.count else { return nil }
            let length = avcc[i..<(i + lengthSize)].reduce(0) { $0 << 8 | Int($1) }
            i += lengthSize
            guard length > 0, i + length <= avcc.count else { return nil }
            out += start
            out += avcc[i..<(i + length)]
            i += length
        }
        return out
    }
}
