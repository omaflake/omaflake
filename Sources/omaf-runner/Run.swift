// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import Darwin
import Foundation
import Virtualization
import OmaflakeKit

/// Runs exactly one macOS VM until it is off (spec §6.4). One process per VM.
@MainActor
final class RunController: NSObject, NSApplicationDelegate, NSWindowDelegate, VZVirtualMachineDelegate {
    struct Request {
        var bundle: MacVMBundle
        var window: Bool
        var socket: URL?
        var pidFile: URL?
        var volumes: [(tag: String, folder: URL)] = []
        var network: Bool
        var audio = false
        var microphone = false
        var clipboard = false
        var camera = false
        var cameraTestPattern = false
        /// The omaflake agent's port, and where the link keeps what the agent reports (nil: no agent port)
        var agentState: URL?
        /// The Mac's MIDI devices in the VM over the agent (plan B161)
        var midi = false
        /// The VM's name for its MIDI pair "omaflake <name>" (nil: the instance's folder, the bundle's parent)
        var vmName: String?
        /// Cards on omaflake's networks (spec §11.12)
        var networks: [NetworkCard] = []
        /// Ports of other VMs on the guest's localhost, over the agent (plan B180)
        var forwards: [AgentForward] = []
        var recovery: Bool
        var provisioning: Provisioning?
        /// A window nobody sees (plan B184): there for keys and the screen's picture (`Robot`), shown by `show`
        var quietWindow = false
    }

    struct Provisioning {
        var username: String
        var fullName: String
        var password: String
        var autoLogin: Bool
        var remoteLogin: Bool
    }

    private let request: Request
    private var vm: VZVirtualMachine!
    private var window: NSWindow?
    /// Keys, clicks and the screen's text for the control socket (plan B184) – with the window
    private var robot: Robot?
    private var server: ControlServer?
    private var clipboard: ClipboardBridge?
    private var camera: CameraBridge?
    private var agent: AgentLink?
    /// The developer's vsock check (plan B174), when ~/.omaflake/developer/vsock-bench exists
    private var vsockBench: VsockBench?
    private var midi: MIDIBridge?

    init(_ request: Request) {
        self.request = request
    }

    func applicationDidFinishLaunching(_ notification: Notification) {
        do {
            // Clipboard: a socket pair – one end is the guest's console port, the other the bridge in this process
            var clipboardHandle: FileHandle?, cameraHandle: FileHandle?
            if request.clipboard {
                let (vmEnd, ourEnd) = try Self.portPair()
                clipboardHandle = vmEnd
                let bridge = ClipboardBridge(fd: ourEnd, onClosed: {})
                bridge.start()
                clipboard = bridge
                log("Clipboard: port \(MachineConfiguration.clipboardPort)")
            }
            if request.camera {
                let (vmEnd, ourEnd) = try Self.portPair()
                cameraHandle = vmEnd
                let bridge = CameraBridge(fd: ourEnd, testPattern: request.cameraTestPattern, onClosed: {})
                bridge.start()
                camera = bridge
                log("Camera: port \(MachineConfiguration.cameraPort)\(request.cameraTestPattern ? " (test pattern)" : "")")
            }
            // The omaflake agent (spec §11.9): hello and status into agent.json; the clipboard moves to it as soon as
            // the guest's agent offers it (the welcome enables it) – the old port stays for guests without
            var agentHandle: FileHandle?
            if let stateFile = request.agentState {
                let (vmEnd, ourEnd) = try Self.portPair()
                agentHandle = vmEnd
                try? FileManager.default.removeItem(at: stateFile)   // an earlier run's report must not count
                var allowed: Set<String> = ["status"]
                if request.clipboard { allowed.insert("clipboard") }
                // Camera: images from 0.7.5 on carry it over the agent (plan B169); older ones keep the port
                if request.camera {
                    allowed.insert("camera")
                    // H.264 where the agent offers it (macOS guests, plan B174); OMAFLAKE_CAMERA_RAW=1 keeps raw frames
                    if ProcessInfo.processInfo.environment["OMAFLAKE_CAMERA_RAW"] != "1" { allowed.insert("camera-h264") }
                }
                // MIDI: the Mac side exists from the start (its "omaflake <VM>" pair is there for the Mac's apps);
                // the guest's endpoints come once its agent's welcome enables MIDI
                if request.midi {
                    // The bundle's own folder is "vm" for every VM: the name comes from omaflake, else the instance's folder
                    let name = request.vmName ?? request.bundle.dir.deletingLastPathComponent().lastPathComponent
                    midi = MIDIBridge.start(vmName: name,
                                            log: { text in log(text) })
                    if midi != nil { allowed.insert("midi") }
                }
                let link = AgentLink(fd: ourEnd, stateFile: stateFile, host: "omaflake", allowed: allowed,
                                     log: { text in log(text) })   // runner.log, like everything else here
                link.forwards = request.forwards
                let clip = clipboard, cam = camera, midiBridge = midi
                link.onWelcome = { [weak link] (features: Set<String>) in
                    guard let link else { return }
                    if features.contains("clipboard") { clip?.useAgent(link) }
                    if features.contains("camera") { cam?.useAgent(link, compressed: features.contains("camera-h264")) }
                    midiBridge?.welcomed(features, link: link)
                }
                link.onClipboard = { mime, data in clip?.apply(mime: mime, data: data) }
                link.onCamera = { request in cam?.request(request) }
                midiBridge?.attach(link)
                Thread.detachNewThread { _ = link.run() }   // ends when the VM goes; a misbehaving guest only loses its link
                agent = link
                log("Agent: port \(AgentProtocol.portName)")
            }
            let config = try MachineConfiguration.make(request.bundle,
                                                       .init(volumes: request.volumes, network: request.network,
                                                             audio: request.audio, microphone: request.microphone,
                                                             clipboard: clipboardHandle, camera: cameraHandle,
                                                             agent: agentHandle,
                                                             networks: try request.networks.map { ($0, try $0.open()) },
                                                             vsock: VsockBench.enabled))
            vm = VZVirtualMachine(configuration: config)
            vm.delegate = self
            if VsockBench.enabled {
                let bench = VsockBench()
                bench.attach(vm)
                vsockBench = bench
            }
            if request.window || request.quietWindow { showWindow(quiet: request.quietWindow) }
            if let socket = request.socket {
                // The handler runs on the main queue (ControlServer's default); a diagnosis answers from a thread later
                server = try ControlServer(path: socket) { [weak self] request, reply in
                    MainActor.assumeIsolated {
                        guard let self else { reply(["error": "terminated"]); return }
                        self.handle(request, reply: reply)
                    }
                }
            }
            if let pidFile = request.pidFile {
                try "\(getpid())\n".write(to: pidFile, atomically: true, encoding: .utf8)
            }
            let options = VZMacOSVirtualMachineStartOptions()
            options.startUpFromMacOSRecovery = request.recovery
            if let p = request.provisioning { try provision(options, p) }
            vm.start(options: options) { [weak self] error in
                MainActor.assumeIsolated {
                    if let error { self?.finish(1, "Start failed: \(describe(error))") }
                    else { log("running (pid \(getpid()))") }
                }
            }
        } catch {
            finish(1, "Start failed: \(describe(error))")
        }
    }

    /// A socket pair for a guest port: the VM's end, and ours with a 1 s send timeout (a guest that does not read
    /// must not block a bridge forever)
    private static func portPair() throws -> (FileHandle, Int32) {
        var fds: [Int32] = [0, 0]
        guard socketpair(AF_UNIX, SOCK_STREAM, 0, &fds) == 0 else { throw VMError("socketpair failed") }
        var timeout = timeval(tv_sec: 1, tv_usec: 0)
        setsockopt(fds[1], SOL_SOCKET, SO_SNDTIMEO, &timeout, socklen_t(MemoryLayout<timeval>.size))
        return (FileHandle(fileDescriptor: fds[0], closeOnDealloc: true), fds[1])
    }

    /// First start after installation: user, auto-login, SSH without Setup Assistant (macOS 27 and later, plan BM11).
    private func provision(_ options: VZMacOSVirtualMachineStartOptions, _ p: Provisioning) throws {
        let prov = VZMacGuestProvisioningOptions()
        prov.username = p.username
        prov.fullName = p.fullName
        prov.password = p.password
        prov.logsInAutomatically = p.autoLogin
        prov.enablesRemoteLogin = p.remoteLogin
        try options.setGuestProvisioning(prov)
        log("Provisioning: user \(p.username), auto-login \(p.autoLogin ? "on" : "off"), SSH \(p.remoteLogin ? "on" : "off")")
    }

    private func showWindow(quiet: Bool = false) {
        let cfg = (try? request.bundle.loadConfig())?.display ?? MacVMConfig.Display()
        let view = VZVirtualMachineView()
        view.virtualMachine = vm
        view.capturesSystemKeys = false
        view.automaticallyReconfiguresDisplay = true
        let scale = NSScreen.main?.backingScaleFactor ?? 2
        let w = NSWindow(contentRect: NSRect(x: 0, y: 0, width: CGFloat(cfg.width) / scale, height: CGFloat(cfg.height) / scale),
                         styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
        w.title = request.bundle.dir.lastPathComponent
        w.contentView = view
        w.delegate = self
        w.center()
        if quiet {
            // On screen for the window server (so the view keeps drawing – OPEN whether it does at alpha 0), but
            // invisible, not clickable and not activating the app
            w.alphaValue = 0
            w.ignoresMouseEvents = true
            w.orderBack(nil)
        } else {
            w.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
        }
        window = w
        robot = Robot(view: view, window: w)
    }

    // Closing the window = let the guest shut down cleanly (no forced power-off)
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        if vm.canRequestStop { try? vm.requestStop() }
        return false
    }

    /// A control request (`ControlServer`): the commands of `RunnerClient.commands`
    func handle(_ request: ControlMessage, reply: @escaping ControlServer.Reply) {
        let command = request.object["command"] as? String ?? ""
        if command == "diagnose" {
            ControlServer.diagnose(agent, request, reply: reply)
        } else if ["type", "key", "click", "screen"].contains(command) {
            robotCommand(command, request.object["args"] as? [String: Any] ?? [:], reply: reply)
        } else {
            reply(handle(command))
        }
    }

    /// `type {text, layout?}`, `key {key}`, `click {x, y}`, `screen {png?}` (plan B184) – only a VM with a window has
    /// them; `layout` ("us" or "de", default "us"): the keyboard the guest reads; `modifiers` ("device", "plain",
    /// "keys"): how Shift and the others are sent
    private func robotCommand(_ command: String, _ args: [String: Any], reply: @escaping ControlServer.Reply) {
        guard let robot else { return reply(["error": "this VM runs without a window (show first)"]) }
        switch command {
        case "type":
            guard let text = args["text"] as? String else { return reply(["error": "type needs args {\"text\": …}"]) }
            let name = args["layout"] as? String ?? "us"
            guard let layout = GuestKeys.Layout(rawValue: name) else {
                return reply(["error": "layout is us or de"])
            }
            guard let strokes = GuestKeys.strokes(for: text, layout: layout) else {
                return reply(["error": "the text has a character the \(name) keyboard cannot type – nothing typed"])
            }
            let mode = Robot.Modifiers(rawValue: args["modifiers"] as? String ?? "device") ?? .device
            robot.press(strokes, modifiers: mode) { reply(["ok": "typed \(strokes.count) keys"]) }
        case "key":
            guard let name = args["key"] as? String, let stroke = GuestKeys.stroke(named: name) else {
                return reply(["error": "key needs args {\"key\": \"cmd+shift+t\" | \"return\" | \"right\" | …}"])
            }
            let mode = Robot.Modifiers(rawValue: args["modifiers"] as? String ?? "device") ?? .device
            robot.press([stroke], modifiers: mode) { reply(["ok": "pressed \(name)"]) }
        case "click":
            guard let x = (args["x"] as? NSNumber)?.doubleValue, let y = (args["y"] as? NSNumber)?.doubleValue,
                  (0...1).contains(x), (0...1).contains(y) else {
                return reply(["error": "click needs args {\"x\", \"y\"} from 0 to 1 (origin top left)"])
            }
            robot.click(x: x, y: y)
            reply(["ok": "clicked"])
        default:
            robot.screen(png: args["png"] as? String, reply: reply)
        }
    }

    func handle(_ command: String) -> [String: String] {
        switch command {
        case "status":
            return ["state": vm.state.name, "pid": "\(getpid())"]
        case "stop":
            guard vm.canRequestStop else { return ["error": "Stop request not possible in state \(vm.state.name)"] }
            do { try vm.requestStop(); return ["ok": "shutdown requested"] }
            catch { return ["error": describe(error)] }
        case "kill":
            vm.stop { [weak self] error in
                MainActor.assumeIsolated { self?.finish(error == nil ? 0 : 1, "forced off") }
            }
            return ["ok": "forcing off"]
        case "show":
            // started headless: only now appear as a regular app with a window
            if window == nil {
                NSApp.setActivationPolicy(.regular)
                showWindow()
            } else if window?.alphaValue == 0 {
                // the quiet window (plan B184) becomes a normal one
                NSApp.setActivationPolicy(.regular)
                window?.alphaValue = 1
                window?.ignoresMouseEvents = false
            }
            window?.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return ["ok": "window in front"]
        case "midi":
            guard let midi else { return ["error": "this VM runs without MIDI (omaf set <name> --midi, then restart)"] }
            return midi.describe()
        default:
            return ControlServer.unknown(command, available: RunnerClient.commands)
        }
    }

    nonisolated func guestDidStop(_ virtualMachine: VZVirtualMachine) {
        MainActor.assumeIsolated { finish(0, "guest powered off") }
    }

    nonisolated func virtualMachine(_ virtualMachine: VZVirtualMachine, didStopWithError error: Error) {
        let text = describe(error)
        MainActor.assumeIsolated { finish(1, "VM stopped with error: \(text)") }
    }

    private func finish(_ code: Int32, _ message: String) {
        log(message)
        server?.close()
        if let pidFile = request.pidFile { try? FileManager.default.removeItem(at: pidFile) }
        exit(code)
    }
}
