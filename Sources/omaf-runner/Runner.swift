// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import Darwin
import ArgumentParser
import Foundation
import Virtualization
import OmaflakeCore

// omaf-runner – runner for macOS guests on Virtualization.framework (spec §6.4, plan §7.4).
// Requires the entitlement com.apple.security.virtualization (App/omaf-runner.entitlements, scripts/sign-runner.sh).

@main
struct VMRunner: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "omaf-runner",
        abstract: "Install and run macOS VMs (one process per VM).",
        subcommands: [Latest.self, Install.self, Clone.self, Run.self, Control.self, IP.self, Bridge.self]
    )
}

struct Latest: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "Show the latest IPSW supported by this Mac (without downloading).")

    func run() throws {
        _ = MainActor.assumeIsolated {
            Task { @MainActor in
                do {
                    let image = try await VZMacOSRestoreImage.latestSupported
                    var head = URLRequest(url: image.url)
                    head.httpMethod = "HEAD"
                    let (_, response) = try await URLSession.shared.data(for: head)
                    let size = (response as? HTTPURLResponse)?.expectedContentLength ?? -1
                    print("macOS \(Installer.version(image.operatingSystemVersion)) (\(image.buildVersion))")
                    print("URL:   \(image.url.absoluteString)")
                    print("Size:  \(size > 0 ? String(format: "%.1f GB", Double(size) / 1e9) : "unknown")")
                    Darwin.exit(0)
                } catch {
                    log("Error: \(describe(error))")
                    Darwin.exit(1)
                }
            }
        }
        dispatchMain()
    }
}

struct Install: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "Install macOS from an IPSW into a new VM folder.")
    @Argument(help: "New VM folder, e.g. ~/.omaflake/bases/mac-base-test") var target: String
    @Option(help: "Local IPSW file") var ipsw: String?
    @Flag(help: "Download the latest supported IPSW from Apple (stored in ~/.omaflake/ipsw/)") var latest = false
    @Option(help: "CPU cores") var cpus = 4
    @Option(help: "Memory in GiB") var memoryGb = 8
    @Option(help: "Disk size in GiB (thin-provisioned)") var diskGb = 80

    func run() throws {
        let request = Installer.Request(
            target: url(target), ipsw: ipsw.map(url), latest: latest, cpus: cpus, memoryGB: memoryGb,
            diskGB: diskGb, ipswDir: Paths.standard().root.appendingPathComponent("ipsw", isDirectory: true))
        _ = MainActor.assumeIsolated {
            Task { @MainActor in
                do {
                    try await Installer.run(request)
                    Darwin.exit(0)
                } catch {
                    log("Error: \(describe(error))")
                    Darwin.exit(1)
                }
            }
        }
        dispatchMain()
    }
}

struct Clone: ParsableCommand {
    static let configuration = CommandConfiguration(
        abstract: "APFS clone of a stopped VM with a new identity (machine identifier, MAC address).")
    @Argument(help: "Source VM folder, e.g. ~/.omaflake/bases/mac-base-26A434") var source: String
    @Argument(help: "New VM folder") var target: String

    func run() throws {
        let src = MacVMBundle(url(source))
        // A running VM keeps writing to its disk – a clone would not be consistent
        let pidFile = src.dir.appendingPathComponent("runner.pid")
        if let text = try? String(contentsOf: pidFile, encoding: .utf8), let pid = pid_t(text.trimmingCharacters(in: .whitespacesAndNewlines)),
           kill(pid, 0) == 0 {
            throw VMError("\(src.dir.lastPathComponent) is running (PID \(pid)) – shut it down first")
        }
        let copy = try src.clone(to: url(target))
        try VZMacMachineIdentifier().dataRepresentation.write(to: copy.machineID)
        let cfg = try copy.loadConfig()
        print("\(copy.dir.path) (MAC \(cfg.macAddress), new machine identifier)")
    }
}

struct Run: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "Start a VM; returns when it is off.")
    @Argument(help: "VM folder") var bundle: String
    @Flag(help: "Show a window (headless otherwise)") var window = false
    @Option(help: "Control socket (status/stop/kill/show)") var socket: String?
    @Option(help: "PID file") var pidFile: String?
    @Option(help: "Share a folder with the guest (appears under /Volumes/My Shared Files)") var share: String?
    @Flag(help: "Without a network card") var noNetwork = false
    @Flag(help: "Sound from the VM to the Mac") var audio = false
    @Flag(help: "The Mac's microphone in the VM (with --audio; macOS asks for permission the first time)") var microphone = false
    @Flag(help: "Clipboard shared with the Mac (guest agent omaflake-clipboard)") var clipboard = false
    @Flag(help: "Camera port org.omaflake.camera (macOS guests: the guest side comes after M12)") var camera = false
    @Option(help: "Attach the omaflake agent's port and keep what it reports in this file (spec §11.9)") var agentState: String?
    @Flag(help: "With --camera: a test pattern instead of the camera") var cameraTestPattern = false
    @Flag(help: "Start into macOS Recovery") var recovery = false
    @Option(help: "First start after installation only: create a user (macOS 27 guest)") var user: String?
    @Option(help: "Full name for --user") var fullName: String?
    @Flag(help: "Read the password for --user from stdin (one line)") var passwordStdin = false
    @Flag(help: "With --user: automatic login") var autoLogin = false
    @Flag(help: "With --user: enable Remote Login (SSH)") var ssh = false

    func validate() throws {
        if user != nil && !passwordStdin { throw ValidationError("--user requires --password-stdin") }
        if user == nil && (autoLogin || ssh || fullName != nil) { throw ValidationError("--auto-login/--ssh/--full-name only with --user") }
    }

    func run() throws {
        var provisioning: RunController.Provisioning?
        if let user {
            guard let password = readLine(strippingNewline: true), !password.isEmpty else {
                throw ValidationError("No password on stdin")
            }
            provisioning = .init(username: user, fullName: fullName ?? user, password: password,
                                 autoLogin: autoLogin, remoteLogin: ssh)
        }
        let request = RunController.Request(
            bundle: MacVMBundle(url(bundle)), window: window, socket: socket.map(url), pidFile: pidFile.map(url),
            share: share.map(url), network: !noNetwork, audio: audio, microphone: audio && microphone,
            clipboard: clipboard, camera: camera, cameraTestPattern: cameraTestPattern, agentState: agentState.map(url),
            recovery: recovery, provisioning: provisioning)
        MainActor.assumeIsolated {
            let app = NSApplication.shared
            app.setActivationPolicy(window ? .regular : .prohibited)
            app.applicationIconImage = Brand.appIcon()   // a bare binary has no icon of its own
            let controller = RunController(request)
            app.delegate = controller
            withExtendedLifetime(controller) { app.run() }
        }
    }
}

struct Control: ParsableCommand {
    static let configuration = CommandConfiguration(commandName: "ctl", abstract: "Control a running omaf-runner.")
    @Argument(help: "Control socket") var socket: String
    @Argument(help: "status | stop | kill | show") var command: String

    func run() throws {
        let reply = try RunnerClient.send(command, socket: url(socket))
        for (k, v) in reply.sorted(by: { $0.key < $1.key }) { print("\(k): \(v)") }
        if reply["error"] != nil { throw ExitCode(1) }
    }
}

struct IP: ParsableCommand {
    static let configuration = CommandConfiguration(abstract: "IP address of the VM (NAT, from /var/db/dhcpd_leases).")
    @Argument(help: "VM folder") var bundle: String

    func run() throws {
        let cfg = try MacVMBundle(url(bundle)).loadConfig()
        guard let ip = DHCPLeases.ip(for: cfg.macAddress) else {
            throw VMError("No IP for \(cfg.macAddress) yet (is the VM running? is networking active in the guest?)")
        }
        print(ip)
    }
}

func url(_ path: String) -> URL {
    URL(fileURLWithPath: (path as NSString).expandingTildeInPath).standardizedFileURL
}
