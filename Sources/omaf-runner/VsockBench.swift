// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation
import Virtualization

/// Plan B174: how fast vsock carries data from the Mac into a guest, against the agent's virtio console (~35 MB/s,
/// ~25 raw camera frames a second). A developer's check, off unless `~/.omaflake/developer/vsock-bench` exists: then
/// the VM gets a virtio socket device and the runner listens on port 47100. A guest that connects
/// (scripts/vsock-bench.py) gets raw 1280×720 NV12 frames' worth of bytes for 10 s, as fast as it reads.
final class VsockBench: NSObject, VZVirtioSocketListenerDelegate, @unchecked Sendable {
    static let port: UInt32 = 47100
    static let seconds: TimeInterval = 10

    static var enabled: Bool {
        FileManager.default.fileExists(atPath: NSHomeDirectory() + "/.omaflake/developer/vsock-bench")
    }

    private let listener = VZVirtioSocketListener()
    /// Kept while the runner runs (a connection closes its socket when it goes); a check makes a few at most
    private var connections: [VZVirtioSocketConnection] = []

    override init() {
        super.init()
        listener.delegate = self
    }

    /// On the VM's queue (the main queue), after the VM was created with a socket device
    @MainActor
    func attach(_ vm: VZVirtualMachine) {
        guard let device = vm.socketDevices.first as? VZVirtioSocketDevice else {
            return log("vsock bench: the VM has no socket device")
        }
        device.setSocketListener(listener, forPort: Self.port)
        log("vsock bench: listening on port \(Self.port)")
    }

    func listener(_ listener: VZVirtioSocketListener, shouldAcceptNewConnection connection: VZVirtioSocketConnection,
                  from socketDevice: VZVirtioSocketDevice) -> Bool {
        let fd = dup(connection.fileDescriptor)
        guard fd >= 0 else { return false }
        connections.append(connection)   // on the VM's queue, as every call of this delegate
        Thread.detachNewThread {
            do {
                let frame = [UInt8](repeating: 0x80, count: 1280 * 720 * 3 / 2)
                let start = Date()
                var total = 0
                var failed = false
                while Date().timeIntervalSince(start) < Self.seconds, !failed {
                    var off = 0
                    while off < frame.count {
                        let n = frame.withUnsafeBytes { write(fd, $0.baseAddress! + off, frame.count - off) }
                        if n < 0 && errno == EINTR { continue }
                        if n <= 0 { failed = true; break }
                        off += n
                    }
                    total += off
                }
                close(fd)
                let elapsed = Date().timeIntervalSince(start)
                log(String(format: "vsock bench: %.1f MB/s (%.1f raw frames a second) over %.1f s",
                           Double(total) / elapsed / 1e6, Double(total) / elapsed / Double(frame.count), elapsed))
            }
        }
        return true
    }
}
