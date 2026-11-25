// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation
import XCTest
@testable import OmaflakeKit

/// The runner's end of a network card against a stand-in for the switch: the switch answers to the address it got
/// the datagram from, so that address must name the card's socket exactly (spec §11.12)
final class NetworkCardTests: XCTestCase {
    func testTheSwitchCanAnswerTheCard() throws {
        let dir = URL(fileURLWithPath: "/tmp").appendingPathComponent("omc-\(getpid())-\(UInt32.random(in: 0...99999))")
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        defer { try? FileManager.default.removeItem(at: dir) }
        let switchPath = dir.appendingPathComponent("vz.sock").path
        let server = Darwin.socket(AF_UNIX, SOCK_DGRAM, 0)
        defer { close(server) }
        try withUnixAddress(switchPath) { addr, len in XCTAssertEqual(Darwin.bind(server, addr, len), 0) }
        // A test that cannot get its datagram fails instead of waiting for ever
        var timeout = timeval(tv_sec: 2, tv_usec: 0)
        setsockopt(server, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))

        let card = NetworkCard(mac: "02:6f:6d:01:0b:00", socket: dir.appendingPathComponent("lan0.sock"),
                               switchSocket: URL(fileURLWithPath: switchPath))
        let handle = try card.open()
        setsockopt(handle.fileDescriptor, SOL_SOCKET, SO_RCVTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
        let frame = Data([UInt8](repeating: 0xab, count: 60))
        try handle.write(contentsOf: frame)

        var buf = [UInt8](repeating: 0, count: 2048)
        var from = sockaddr_un()
        var fromLength = socklen_t(MemoryLayout<sockaddr_un>.size)
        let n = withUnsafeMutablePointer(to: &from) { p in
            p.withMemoryRebound(to: sockaddr.self, capacity: 1) { recvfrom(server, &buf, buf.count, 0, $0, &fromLength) }
        }
        XCTAssertEqual(n, 60, "errno \(errno)")
        guard n == 60 else { return }
        // Answer exactly the address the datagram came from, as the switch does
        let reply = [UInt8]("reply".utf8)
        let sent = withUnsafePointer(to: &from) { p in
            p.withMemoryRebound(to: sockaddr.self, capacity: 1) { sendto(server, reply, reply.count, 0, $0, fromLength) }
        }
        XCTAssertEqual(sent, reply.count)
        var answer = [UInt8](repeating: 0, count: 64)
        let got = read(handle.fileDescriptor, &answer, answer.count)
        XCTAssertEqual(got, reply.count, "errno \(errno)")
        XCTAssertEqual(Array(answer.prefix(max(got, 0))), reply)
    }

    func testCardsComeInThrees() throws {
        let cards = try NetworkCard.parse(macs: ["02:6f:6d:01:0a:00"], sockets: ["/r/lan0.sock"], switches: ["/n/vz.sock"])
        XCTAssertEqual(cards, [NetworkCard(mac: "02:6f:6d:01:0a:00", socket: URL(fileURLWithPath: "/r/lan0.sock"),
                                           switchSocket: URL(fileURLWithPath: "/n/vz.sock"))])
        XCTAssertThrowsError(try NetworkCard.parse(macs: ["a"], sockets: [], switches: []))
    }
}
