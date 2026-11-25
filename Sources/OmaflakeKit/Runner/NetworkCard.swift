// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

/// The runner's end of a VM's card on one of omaflake's networks (spec §11.12): a datagram socket bound to a path of its
/// own and connected to the network's switch (`omaf network-switch`'s `vz.sock`) – one Ethernet frame per datagram, as
/// `VZFileHandleNetworkDeviceAttachment` wants it. The switch answers to the bound path, so it is unique per card.
public struct NetworkCard: Sendable, Equatable {
    public var mac: String
    /// The runner's end (`run/<vm>/lan<i>.sock`)
    public var socket: URL
    /// The switch's `vz.sock`
    public var switchSocket: URL

    public init(mac: String, socket: URL, switchSocket: URL) {
        self.mac = mac
        self.socket = socket
        self.switchSocket = switchSocket
    }

    /// The connected socket. Buffers as Apple advises for VZFileHandleNetworkDeviceAttachment (SO_RCVBUF at least
    /// 4 × SO_SNDBUF).
    public func open() throws -> FileHandle {
        let fd = Darwin.socket(AF_UNIX, SOCK_DGRAM, 0)
        guard fd >= 0 else { throw VMError("Network card: socket() failed (errno \(errno))") }
        var ok = false
        defer { if !ok { close(fd) } }
        unlink(socket.path)   // a socket file left by an earlier run
        try withUnixAddress(socket.path) { addr, len in
            guard Darwin.bind(fd, addr, len) == 0 else {
                throw VMError("Network card: cannot bind \(socket.path) (errno \(errno))")
            }
        }
        try withUnixAddress(switchSocket.path) { addr, len in
            guard Darwin.connect(fd, addr, len) == 0 else {
                throw VMError("Network card: the switch at \(switchSocket.path) does not answer (errno \(errno))")
            }
        }
        var send: Int32 = 1 << 20, receive: Int32 = 4 << 20
        setsockopt(fd, SOL_SOCKET, SO_SNDBUF, &send, socklen_t(MemoryLayout<Int32>.size))
        setsockopt(fd, SOL_SOCKET, SO_RCVBUF, &receive, socklen_t(MemoryLayout<Int32>.size))
        ok = true
        return FileHandle(fileDescriptor: fd, closeOnDealloc: true)
    }

    /// The cards from `--lan-mac/--lan-socket/--lan-switch`, which come in threes
    public static func parse(macs: [String], sockets: [String], switches: [String]) throws -> [NetworkCard] {
        guard macs.count == sockets.count, sockets.count == switches.count else {
            throw VMError("--lan-mac, --lan-socket and --lan-switch must be given equally often")
        }
        return zip(macs, zip(sockets, switches)).map {
            NetworkCard(mac: $0.0, socket: URL(fileURLWithPath: $0.1.0), switchSocket: URL(fileURLWithPath: $0.1.1))
        }
    }
}

/// A sockaddr_un for `path` with the exact length – path and its terminating NUL (at most 103 bytes of path)
func withUnixAddress(_ path: String, _ body: (UnsafePointer<sockaddr>, socklen_t) throws -> Void) throws {
    var addr = sockaddr_un()
    addr.sun_family = sa_family_t(AF_UNIX)
    let bytes = Array(path.utf8)
    let capacity = MemoryLayout.size(ofValue: addr.sun_path)
    guard bytes.count < capacity else { throw VMError("Socket path too long (\(bytes.count) bytes): \(path)") }
    withUnsafeMutableBytes(of: &addr.sun_path) { raw in
        raw.copyBytes(from: bytes)
        raw[bytes.count] = 0
    }
    let length = MemoryLayout<sockaddr_un>.offset(of: \sockaddr_un.sun_path)! + bytes.count + 1
    addr.sun_len = UInt8(length)
    try withUnsafePointer(to: &addr) { p in
        try p.withMemoryRebound(to: sockaddr.self, capacity: 1) { try body($0, socklen_t(length)) }
    }
}
