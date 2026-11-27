// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import Darwin
import Foundation
import Virtualization
import OmaflakeKit

// omaf-runner – runner for macOS guests on Virtualization.framework (spec §6.4, plan §7.4).
// Requires the entitlement com.apple.security.virtualization (App/omaf-runner.entitlements, scripts/sign-runner.sh).

@main
enum VMRunner {
    static func main() {
        CommandLineTool.main(
            [Latest.self, Install.self, Clone.self, Run.self, Control.self, IP.self, Bridge.self, MIDICheck.self, DiagnoseCheck.self],
            abstract: "Install and run macOS VMs (one process per VM).",
            argv: Array(CommandLine.arguments.dropFirst()))
    }
}

struct Latest: RunnerCommand {
    static let name = "latest"
    static let abstract = "Show the latest IPSW supported by this Mac (without downloading)."

    init(_ a: inout Arguments) throws {}

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

struct Install: RunnerCommand {
    static let name = "install"
    static let abstract = "Install macOS from an IPSW into a new VM folder."
    static let arguments = "<target>   (new VM folder, e.g. ~/.omaflake/bases/mac-base-test)"
    static let options: [Option] = [
        .value("ipsw", "Local IPSW file"),
        .flag("latest", "Download the latest supported IPSW from Apple (stored in ~/.omaflake/ipsw/)"),
        .value("cpus", "CPU cores (4)"),
        .value("memory-gb", "Memory in GiB (8)"),
        .value("disk-gb", "Disk size in GiB, thin-provisioned (80)"),
    ]
    let target: String
    let ipsw: String?
    let latest: Bool
    let cpus: Int
    let memoryGb: Int
    let diskGb: Int

    init(_ a: inout Arguments) throws {
        target = try a.argument("target")
        ipsw = a.string("ipsw")
        latest = a.flag("latest")
        cpus = try a.int("cpus", default: 4)
        memoryGb = try a.int("memory-gb", default: 8)
        diskGb = try a.int("disk-gb", default: 80)
    }

    func run() throws {
        let request = Installer.Request(
            target: url(target), ipsw: ipsw.map(url), latest: latest, cpus: cpus, memoryGB: memoryGb,
            diskGB: diskGb, ipswDir: dataFolder().appendingPathComponent("ipsw", isDirectory: true))
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

struct Clone: RunnerCommand {
    static let name = "clone"
    static let abstract = "APFS clone of a stopped VM with a new identity (machine identifier, MAC address)."
    static let arguments = "<source> <target>   (a stopped VM folder, the new one)"
    let source: String
    let target: String

    init(_ a: inout Arguments) throws {
        source = try a.argument("source")
        target = try a.argument("target")
    }

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

struct Run: RunnerCommand {
    static let name = "run"
    static let abstract = "Start a VM; returns when it is off."
    static let arguments = "<bundle>   (VM folder)"
    static let options: [Option] = [
        .flag("window", "Show a window (headless otherwise)"),
        .value("socket", "Control socket (status/stop/kill/show)"),
        .value("pid-file", "PID file"),
        .value("volume", "A volume, <name>=<folder>: the guest mounts it at /Volumes/<name> (again for more, plan B187)"),
        .flag("no-network", "Without a network card"),
        .flag("audio", "Sound from the VM to the Mac"),
        .flag("microphone", "The Mac's microphone in the VM (with --audio; macOS asks for permission the first time)"),
        .flag("clipboard", "Clipboard shared with the Mac (guest agent omaflake-clipboard)"),
        .flag("camera", "Camera port org.omaflake.camera (images before 0.7.5; newer ones carry it over the agent)"),
        .value("agent-state", "Attach the omaflake agent's port and keep what it reports in this file (spec §11.9)"),
        .flag("camera-test-pattern", "With --camera: a test pattern instead of the camera"),
        .flag("midi", "With --agent-state: the Mac's MIDI devices in the VM, both ways (plan B161)"),
        .value("vm-name", "The VM's name: its own MIDI pair on the Mac is \"omaflake <name>\" (default: the bundle's folder)"),
        .value("lan-mac", "A card on one of omaflake's networks: its MAC (with --lan-socket and --lan-switch, once per network)"),
        .value("lan-socket", "The card's own datagram socket (the switch answers to it)"),
        .value("lan-switch", "The network switch's datagram socket (vz.sock)"),
        .value("forward", "A port of another VM on the guest's localhost, <port>=<address>:<port> (again for more; over the agent)"),
        .flag("recovery", "Start into macOS Recovery"),
        .flag("quiet-window", "A window nobody sees: for type/key/click/screen; `show` makes it visible (plan B184)"),
        .value("user", "First start after installation only: create a user (macOS 27 guest)"),
        .value("full-name", "Full name for --user"),
        .flag("password-stdin", "Read the password for --user from stdin (one line)"),
        .flag("auto-login", "With --user: automatic login"),
        .flag("ssh", "With --user: enable Remote Login (SSH)"),
    ]
    let bundle: String
    let window: Bool
    let socket: String?
    let pidFile: String?
    let volumes: [(tag: String, folder: String)]
    let noNetwork: Bool
    let audio: Bool
    let microphone: Bool
    let clipboard: Bool
    let camera: Bool
    let agentState: String?
    let cameraTestPattern: Bool
    let midi: Bool
    let vmName: String?
    let lanMac: [String]
    let lanSocket: [String]
    let lanSwitch: [String]
    let forwards: [AgentForward]
    let recovery: Bool
    let quietWindow: Bool
    let user: String?
    let fullName: String?
    let passwordStdin: Bool
    let autoLogin: Bool
    let ssh: Bool

    init(_ a: inout Arguments) throws {
        bundle = try a.argument("bundle")
        window = a.flag("window")
        socket = a.string("socket")
        pidFile = a.string("pid-file")
        volumes = try a.strings("volume").map { text in
            guard let eq = text.firstIndex(of: "="), eq != text.startIndex, text.index(after: eq) != text.endIndex else {
                throw ValidationError("--volume \(text): <name>=<folder>")
            }
            return (String(text[..<eq]), String(text[text.index(after: eq)...]))
        }
        noNetwork = a.flag("no-network")
        audio = a.flag("audio")
        microphone = a.flag("microphone")
        clipboard = a.flag("clipboard")
        camera = a.flag("camera")
        agentState = a.string("agent-state")
        cameraTestPattern = a.flag("camera-test-pattern")
        midi = a.flag("midi")
        vmName = a.string("vm-name")
        lanMac = a.strings("lan-mac")
        lanSocket = a.strings("lan-socket")
        lanSwitch = a.strings("lan-switch")
        forwards = try a.strings("forward").map { text in
            guard let f = AgentForward(argument: text) else { throw ValidationError("--forward \(text): <port>=<address>:<port>") }
            return f
        }
        recovery = a.flag("recovery")
        quietWindow = a.flag("quiet-window")
        user = a.string("user")
        fullName = a.string("full-name")
        passwordStdin = a.flag("password-stdin")
        autoLogin = a.flag("auto-login")
        ssh = a.flag("ssh")
        try validate()
    }

    func validate() throws {
        if user != nil && !passwordStdin { throw ValidationError("--user requires --password-stdin") }
        if user == nil && (autoLogin || ssh || fullName != nil) { throw ValidationError("--auto-login/--ssh/--full-name only with --user") }
        if lanMac.count != lanSocket.count || lanSocket.count != lanSwitch.count {
            throw ValidationError("--lan-mac, --lan-socket and --lan-switch must be given equally often")
        }
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
            volumes: volumes.map { ($0.tag, url($0.folder)) }, network: !noNetwork, audio: audio, microphone: audio && microphone,
            clipboard: clipboard, camera: camera, cameraTestPattern: cameraTestPattern, agentState: agentState.map(url),
            midi: midi, vmName: vmName,
            networks: try NetworkCard.parse(macs: lanMac, sockets: lanSocket, switches: lanSwitch),
            forwards: forwards, recovery: recovery, provisioning: provisioning, quietWindow: quietWindow)
        MainActor.assumeIsolated {
            let app = NSApplication.shared
            // A quiet window (plan B184) needs an app that may have windows, but no Dock icon
            app.setActivationPolicy(window ? .regular : quietWindow ? .accessory : .prohibited)
            app.applicationIconImage = Brand.appIcon()   // a bare binary has no icon of its own
            let controller = RunController(request)
            app.delegate = controller
            withExtendedLifetime(controller) { app.run() }
        }
    }
}

struct Control: RunnerCommand {
    static let name = "ctl"
    static let abstract = "Control a running omaf-runner."
    static let arguments = "<socket> <command>   (control socket; status | stop | kill | show | midi | diagnose | type | key | click | screen)"
    static let options: [Option] = [
        .value("check", "diagnose: the check (info, network, clipboard-read, …)"),
        .value("args", "the command's args as a JSON object, e.g. '{\"text\":\"x\"}'; \"-\" reads them from stdin (so a password typed into the guest is in no argument). type: {text, layout?: us|de, modifiers?: device|plain|keys}, key: {key, modifiers?}, click: {x, y}, screen: {png?}"),
    ]
    let socket: String
    let command: String
    let check: String?
    let args: String?

    init(_ a: inout Arguments) throws {
        socket = try a.argument("socket")
        command = try a.argument("command")
        check = a.string("check")
        args = a.string("args")
    }

    func run() throws {
        var request: [String: Any] = ["command": command]
        if let check { request["check"] = check }
        if var args {
            if args == "-" { args = String(decoding: FileHandle.standardInput.readDataToEndOfFile(), as: UTF8.self) }
            guard let obj = (try? JSONSerialization.jsonObject(with: Data(args.utf8))) as? [String: Any] else {
                throw ValidationError("--args must be a JSON object")
            }
            request["args"] = obj
        }
        // A diagnosis waits up to 15 s for the guest; reading the screen (text recognition) and typing take a while
        let timeout: TimeInterval = ["diagnose"].contains(command) ? 20 : ["screen", "type"].contains(command) ? 90 : 5
        let reply = try RunnerClient.request(request, socket: url(socket), timeout: timeout)
        for (k, v) in reply.sorted(by: { $0.key < $1.key }) {
            if let s = v as? String { print("\(k): \(s)"); continue }
            let json = JSONSerialization.isValidJSONObject([v])
                ? (try? JSONSerialization.data(withJSONObject: [v], options: [.sortedKeys, .withoutEscapingSlashes])) : nil
            print("\(k): \(json.map { String(String(decoding: $0, as: UTF8.self).dropFirst().dropLast()) } ?? "\(v)")")
        }
        if reply["error"] != nil { throw ExitCode(1) }
    }
}

struct IP: RunnerCommand {
    static let name = "ip"
    static let abstract = "IP address of the VM (NAT, from /var/db/dhcpd_leases)."
    static let arguments = "<bundle>   (VM folder)"
    let bundle: String

    init(_ a: inout Arguments) throws {
        bundle = try a.argument("bundle")
    }

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

/// The data folder as the core finds it (`Paths::standard`): VM_HOME, otherwise `home` in ~/.omaflake/config.json,
/// otherwise ~/.omaflake – here only for the IPSW folder of `omaf-runner install --latest`.
func dataFolder() -> URL {
    if let custom = ProcessInfo.processInfo.environment["VM_HOME"], !custom.isEmpty {
        return URL(fileURLWithPath: custom, isDirectory: true)
    }
    let standard = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".omaflake", isDirectory: true)
    guard let data = try? Data(contentsOf: standard.appendingPathComponent("config.json")),
          let object = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any],
          let home = object["home"] as? String, !home.isEmpty else { return standard }
    return URL(fileURLWithPath: (home as NSString).expandingTildeInPath, isDirectory: true)
}
