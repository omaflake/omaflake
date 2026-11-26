// SPDX-License-Identifier: AGPL-3.0-or-later
import CoreMediaIO
import Foundation
import OmaflakeKit
import os

/// omaflake's camera extension for macOS guests (plan B184; spec §11.9): one device, "omaflake Camera", with one
/// stream – 1280×720 NV12, 30 a second. While an app uses the camera, the extension reads the agent's raw frames
/// from `127.0.0.1:47042` (`CameraStreamReader`) and sends each one on at once; the agent asks the Mac for frames
/// only while that reader is connected, so the Mac's camera is on only then. Without frames (no agent, the camera
/// off for this VM) apps get a test picture instead of a frozen one.
///
/// A CoreMediaIO system extension: sandboxed, activated by the app it ships in (Sources/omaflake-tools) and
/// approved by the user. It must be signed; ad hoc it loads only in a guest with System Integrity Protection off.
let log = Logger(subsystem: "app.omaflake.guest", category: "camera")

final class StreamSource: NSObject, CMIOExtensionStreamSource, @unchecked Sendable {
    private(set) var stream: CMIOExtensionStream!
    private let format: CMIOExtensionStreamFormat
    private let videoFormat: CMFormatDescription
    private let pool: CVPixelBufferPool
    private let frameDuration = CMTime(value: 1, timescale: 30)
    private let lock = NSLock()
    private var running = false
    private var generation = 0

    init?(localizedName: String) {
        var description: CMFormatDescription?
        CMVideoFormatDescriptionCreate(allocator: kCFAllocatorDefault,
                                       codecType: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
                                       width: Int32(CameraFrames.width), height: Int32(CameraFrames.height),
                                       extensions: nil, formatDescriptionOut: &description)
        let attributes: [CFString: Any] = [
            kCVPixelBufferWidthKey: CameraFrames.width, kCVPixelBufferHeightKey: CameraFrames.height,
            kCVPixelBufferPixelFormatTypeKey: kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange,
            kCVPixelBufferIOSurfacePropertiesKey: [:] as CFDictionary,
        ]
        var made: CVPixelBufferPool?
        CVPixelBufferPoolCreate(kCFAllocatorDefault, nil, attributes as CFDictionary, &made)
        guard let description, let made else { return nil }
        videoFormat = description
        pool = made
        format = CMIOExtensionStreamFormat(formatDescription: description, maxFrameDuration: frameDuration,
                                           minFrameDuration: frameDuration, validFrameDurations: nil)
        super.init()
        stream = CMIOExtensionStream(localizedName: localizedName, streamID: UUID(), direction: .source,
                                     clockType: .hostTime, source: self)
    }

    var formats: [CMIOExtensionStreamFormat] { [format] }
    var availableProperties: Set<CMIOExtensionProperty> { [.streamActiveFormatIndex, .streamFrameDuration] }

    func streamProperties(forProperties properties: Set<CMIOExtensionProperty>) throws -> CMIOExtensionStreamProperties {
        let p = CMIOExtensionStreamProperties(dictionary: [:])
        if properties.contains(.streamActiveFormatIndex) { p.activeFormatIndex = 0 }
        if properties.contains(.streamFrameDuration) { p.frameDuration = frameDuration }
        return p
    }

    func setStreamProperties(_ streamProperties: CMIOExtensionStreamProperties) throws {}
    func authorizedToStartStream(for client: CMIOExtensionClient) -> Bool { true }

    /// The first app that uses the camera starts the stream, the last one that leaves stops it
    func startStream() throws {
        let mine = lock.withLock { () -> Int in
            running = true
            generation += 1
            return generation
        }
        log.notice("stream started")
        Thread.detachNewThread { [self] in pump(mine) }
    }

    func stopStream() throws {
        lock.withLock { running = false }
        log.notice("stream stopped")
    }

    private func wanted(_ mine: Int) -> Bool { lock.withLock { running && generation == mine } }

    /// Until the stream stops: the agent's frames while it serves them, else the test picture
    private func pump(_ mine: Int) {
        var frame = [UInt8](repeating: 0, count: CameraFrames.bytes(width: CameraFrames.width, height: CameraFrames.height))
        let reader = CameraStreamReader()
        var pictures = 0
        while wanted(mine) {
            guard reader.connect() else {
                // Nothing to read: a third of a second of the test picture, then try again
                for _ in 0..<10 where wanted(mine) {
                    CameraFrames.testPattern(into: &frame, width: CameraFrames.width, height: CameraFrames.height,
                                             count: pictures)
                    pictures += 1
                    send(frame)
                    usleep(33_000)
                }
                continue
            }
            log.notice("reading the agent's frames")
            var received = 0
            while wanted(mine), reader.read(into: &frame) {
                send(frame)
                received += 1
            }
            reader.close()
            log.notice("the agent's stream ended after \(received) frames")
        }
    }

    private func send(_ frame: [UInt8]) {
        var buffer: CVPixelBuffer?
        guard CVPixelBufferPoolCreatePixelBuffer(kCFAllocatorDefault, pool, &buffer) == kCVReturnSuccess,
              let buffer else { return }
        CVPixelBufferLockBaseAddress(buffer, [])
        var copied = false
        if let luma = CVPixelBufferGetBaseAddressOfPlane(buffer, 0),
           let chroma = CVPixelBufferGetBaseAddressOfPlane(buffer, 1) {
            copied = frame.withUnsafeBytes {
                CameraFrames.copy($0, width: CameraFrames.width, height: CameraFrames.height,
                                  luma: .init(base: luma, bytesPerRow: CVPixelBufferGetBytesPerRowOfPlane(buffer, 0)),
                                  chroma: .init(base: chroma, bytesPerRow: CVPixelBufferGetBytesPerRowOfPlane(buffer, 1)))
            }
        }
        CVPixelBufferUnlockBaseAddress(buffer, [])
        guard copied else { return }
        let now = CMClockGetTime(CMClockGetHostTimeClock())
        var timing = CMSampleTimingInfo(duration: frameDuration, presentationTimeStamp: now, decodeTimeStamp: .invalid)
        var sample: CMSampleBuffer?
        guard CMSampleBufferCreateForImageBuffer(allocator: kCFAllocatorDefault, imageBuffer: buffer,
                                                 dataReady: true, makeDataReadyCallback: nil, refcon: nil,
                                                 formatDescription: videoFormat, sampleTiming: &timing,
                                                 sampleBufferOut: &sample) == noErr, let sample else { return }
        stream.send(sample, discontinuity: [], hostTimeInNanoseconds: UInt64(now.seconds * 1_000_000_000))
    }
}

final class DeviceSource: NSObject, CMIOExtensionDeviceSource, @unchecked Sendable {
    private(set) var device: CMIOExtensionDevice!
    private let streamSource: StreamSource
    static let name = "omaflake Camera"

    init?(localizedName: String = DeviceSource.name) {
        guard let source = StreamSource(localizedName: "\(localizedName) video") else { return nil }
        streamSource = source
        super.init()
        device = CMIOExtensionDevice(localizedName: localizedName, deviceID: UUID(), legacyDeviceID: nil, source: self)
        do { try device.addStream(streamSource.stream) } catch {
            log.error("addStream: \(error.localizedDescription)")
            return nil
        }
    }

    var availableProperties: Set<CMIOExtensionProperty> { [.deviceTransportType, .deviceModel] }

    func deviceProperties(forProperties properties: Set<CMIOExtensionProperty>) throws -> CMIOExtensionDeviceProperties {
        let p = CMIOExtensionDeviceProperties(dictionary: [:])
        if properties.contains(.deviceTransportType) { p.transportType = 0x7669_7274 } // 'virt': a virtual device
        if properties.contains(.deviceModel) { p.model = DeviceSource.name }
        return p
    }

    func setDeviceProperties(_ deviceProperties: CMIOExtensionDeviceProperties) throws {}
}

final class ProviderSource: NSObject, CMIOExtensionProviderSource, @unchecked Sendable {
    private(set) var provider: CMIOExtensionProvider!
    private var deviceSource: DeviceSource?

    override init() {
        super.init()
        provider = CMIOExtensionProvider(source: self, clientQueue: nil)
        deviceSource = DeviceSource()
        guard let deviceSource else {
            log.error("no device: the stream's format or buffers could not be made")
            return
        }
        do { try provider.addDevice(deviceSource.device) } catch { log.error("addDevice: \(error.localizedDescription)") }
    }

    func connect(to client: CMIOExtensionClient) throws {}
    func disconnect(from client: CMIOExtensionClient) {}
    var availableProperties: Set<CMIOExtensionProperty> { [.providerManufacturer] }

    func providerProperties(forProperties properties: Set<CMIOExtensionProperty>) throws -> CMIOExtensionProviderProperties {
        let p = CMIOExtensionProviderProperties(dictionary: [:])
        if properties.contains(.providerManufacturer) { p.manufacturer = "omaflake" }
        return p
    }

    func setProviderProperties(_ providerProperties: CMIOExtensionProviderProperties) throws {}
}

@main
enum CameraExtension {
    static func main() {
        log.notice("omaflake camera extension starting")
        let source = ProviderSource()
        CMIOExtensionProvider.startService(provider: source.provider)
        CFRunLoopRun()
    }
}
