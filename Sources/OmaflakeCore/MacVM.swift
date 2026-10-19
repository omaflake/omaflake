// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

/// Settings of a macOS VM (config.json in the VM folder, spec §5).
/// Hardware model, machine identifier and auxiliary storage are separate files next to it.
public struct MacVMConfig: Codable, Sendable, Equatable {
    public struct Display: Codable, Sendable, Equatable {
        public var width: Int
        public var height: Int
        public var ppi: Int

        public init(width: Int = 1920, height: Int = 1200, ppi: Int = 80) {
            self.width = width
            self.height = height
            self.ppi = ppi
        }
    }

    /// Format version of the folder
    public var version: Int
    public var cpus: Int
    public var memoryMB: Int
    public var diskGB: Int
    public var display: Display
    /// fixed MAC per VM (for the NAT IP from /var/db/dhcpd_leases)
    public var macAddress: String
    /// Build of the installed IPSW, e.g. "27A5321"
    public var osBuild: String?

    public init(cpus: Int, memoryMB: Int, diskGB: Int, display: Display = Display(),
                macAddress: String, osBuild: String? = nil) {
        self.version = 1
        self.cpus = cpus
        self.memoryMB = memoryMB
        self.diskGB = diskGB
        self.display = display
        self.macAddress = macAddress
        self.osBuild = osBuild
    }
}

/// Folder of a macOS VM: base, image and instance share the same layout.
public struct MacVMBundle: Sendable {
    public let dir: URL

    public init(_ dir: URL) { self.dir = dir }

    public var config: URL { dir.appendingPathComponent("config.json") }
    /// VZMacHardwareModel.dataRepresentation
    public var hardwareModel: URL { dir.appendingPathComponent("hardware-model") }
    /// VZMacMachineIdentifier.dataRepresentation (new per instance)
    public var machineID: URL { dir.appendingPathComponent("machine-id") }
    /// VZMacAuxiliaryStorage
    public var auxStorage: URL { dir.appendingPathComponent("aux.img") }
    /// System disk, RAW (VZDiskImageStorageDeviceAttachment only supports RAW, plan BM9)
    public var disk: URL { dir.appendingPathComponent("disk.img") }

    public func loadConfig() throws -> MacVMConfig {
        guard let data = try? Data(contentsOf: config) else {
            throw VMError("Not a macOS VM (config.json missing): \(dir.path)")
        }
        return try JSONDecoder().decode(MacVMConfig.self, from: data)
    }

    public func save(_ c: MacVMConfig) throws {
        let enc = JSONEncoder()
        enc.outputFormatting = [.prettyPrinted, .sortedKeys]
        try enc.encode(c).write(to: config, options: .atomic)
    }

    /// The files that make up a VM (runtime files like sockets and logs are not part of it).
    public var files: [URL] { [config, hardwareModel, machineID, auxStorage, disk] }

    /// APFS clone of the VM files into a new folder (no extra space until blocks change), writable,
    /// with a new MAC address so that clone and source get different NAT addresses. The machine
    /// identifier needs Virtualization.framework – `omaf-runner clone` replaces it.
    @discardableResult
    public func clone(to target: URL) throws -> MacVMBundle {
        let fm = FileManager.default
        guard !fm.fileExists(atPath: target.path) else { throw VMError("Target exists already: \(target.path)") }
        _ = try loadConfig()
        try fm.createDirectory(at: target, withIntermediateDirectories: true)
        let copy = MacVMBundle(target)
        for (src, dst) in zip(files, copy.files) {
            guard clonefile(src.path, dst.path, 0) == 0 else {
                throw VMError("clonefile \(src.lastPathComponent) failed: \(String(cString: strerror(errno))) (APFS required)")
            }
            try fm.setAttributes([.posixPermissions: src == auxStorage ? 0o600 : 0o644], ofItemAtPath: dst.path)
        }
        var cfg = try copy.loadConfig()
        cfg.macAddress = MACAddress.randomLocallyAdministered()
        try copy.save(cfg)
        return copy
    }

    /// Creates an empty, sparse RAW disk (size a multiple of 512).
    public func createDisk(gigabytes: Int) throws {
        guard FileManager.default.createFile(atPath: disk.path, contents: nil) else {
            throw VMError("Cannot create disk: \(disk.path)")
        }
        let handle = try FileHandle(forWritingTo: disk)
        defer { try? handle.close() }
        try handle.truncate(atOffset: UInt64(gigabytes) * 1024 * 1024 * 1024)
    }
}

/// IP addresses from the macOS DHCP server (vmnet/NAT), /var/db/dhcpd_leases.
/// MAC addresses for VM network cards.
public enum MACAddress {
    /// Random unicast address with the "locally administered" bit set (like VZMACAddress.randomLocallyAdministered).
    public static func randomLocallyAdministered() -> String {
        var bytes = (0..<6).map { _ in UInt8.random(in: 0...255) }
        bytes[0] = (bytes[0] | 0x02) & 0xFE
        return bytes.map { String(format: "%02x", $0) }.joined(separator: ":")
    }
}

public enum DHCPLeases {
    public static let file = URL(fileURLWithPath: "/var/db/dhcpd_leases")

    /// MAC as written in the lease file: lowercase, leading zeros per octet removed
    /// ("e6:25:86:0f:90:4d" → "e6:25:86:f:90:4d").
    public static func normalize(_ mac: String) -> String {
        mac.lowercased().split(separator: ":").map { part in
            let trimmed = part.drop(while: { $0 == "0" })
            return trimmed.isEmpty ? "0" : String(trimmed)
        }.joined(separator: ":")
    }

    /// Most recent IP for the MAC (entries are newest first; with several, the largest `lease` wins).
    public static func ip(for mac: String, in text: String) -> String? {
        let wanted = normalize(mac)
        var best: (ip: String, lease: UInt64)?
        for block in text.components(separatedBy: "}") {
            var fields: [String: String] = [:]
            for line in block.split(separator: "\n") {
                let t = line.trimmingCharacters(in: .whitespaces)
                guard let eq = t.firstIndex(of: "=") else { continue }
                fields[String(t[..<eq])] = String(t[t.index(after: eq)...])
            }
            // hw_address=1,<mac>  (1 = Ethernet)
            guard let hw = fields["hw_address"], let ip = fields["ip_address"],
                  let comma = hw.firstIndex(of: ",") else { continue }
            guard normalize(String(hw[hw.index(after: comma)...])) == wanted else { continue }
            let lease = fields["lease"].flatMap { UInt64($0.replacingOccurrences(of: "0x", with: ""), radix: 16) } ?? 0
            if best == nil || lease > best!.lease { best = (ip, lease) }
        }
        return best?.ip
    }

    public static func ip(for mac: String) -> String? {
        guard let text = try? String(contentsOf: file, encoding: .utf8) else { return nil }
        return ip(for: mac, in: text)
    }
}

/// Controls a running omaf-runner via its Unix socket (one JSON line out, one back).
public enum RunnerClient {
    public static let commands = ["status", "stop", "kill", "show"]

    @discardableResult
    public static func send(_ command: String, socket: URL, timeout: TimeInterval = 5) throws -> [String: String] {
        let fd = try connectUnix(socket.path, timeout: timeout)
        defer { close(fd) }
        let line = try JSONSerialization.data(withJSONObject: ["command": command]) + Data("\n".utf8)
        _ = line.withUnsafeBytes { write(fd, $0.baseAddress, $0.count) }
        var data = Data()
        var buf = [UInt8](repeating: 0, count: 4096)
        while !data.contains(UInt8(ascii: "\n")) {
            let n = read(fd, &buf, buf.count)
            if n <= 0 { break }
            data.append(contentsOf: buf[0..<n])
        }
        guard let obj = try? JSONSerialization.jsonObject(with: data) as? [String: String] else {
            throw VMError("Invalid response from omaf-runner: \(String(decoding: data, as: UTF8.self))")
        }
        return obj
    }
}
