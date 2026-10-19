// SPDX-License-Identifier: AGPL-3.0-or-later
import AppKit
import Darwin
import Foundation
import Virtualization
import OmaflakeCore

/// Runs exactly one macOS VM until it is off (spec §6.4). One process per VM.
@MainActor
final class RunController: NSObject, NSApplicationDelegate, NSWindowDelegate, VZVirtualMachineDelegate {
    struct Request {
        var bundle: MacVMBundle
        var window: Bool
        var socket: URL?
        var pidFile: URL?
        var share: URL?
        var network: Bool
        var audio = false
        var microphone = false
        var clipboard = false
        var camera = false
        var cameraTestPattern = false
        /// The omaflake agent's port, and where the link keeps what the agent reports (nil: no agent port)
        var agentState: URL?
        var recovery: Bool
        var provisioning: Provisioning?
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
    private var server: ControlServer?
    private var clipboard: ClipboardBridge?
    private var camera: CameraBridge?
    private var agent: AgentLink?

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
                let link = AgentLink(fd: ourEnd, stateFile: stateFile, host: "omaflake", allowed: allowed,
                                     log: { text in log(text) })   // runner.log, like everything else here
                let clip = clipboard
                link.onWelcome = { [weak link] (features: Set<String>) in
                    if let link, features.contains("clipboard") { clip?.useAgent(link) }
                }
                link.onClipboard = { mime, data in clip?.apply(mime: mime, data: data) }
                Thread.detachNewThread { _ = link.run() }   // ends when the VM goes; a misbehaving guest only loses its link
                agent = link
                log("Agent: port \(AgentProtocol.portName)")
            }
            let config = try MachineConfiguration.make(request.bundle,
                                                       .init(share: request.share, network: request.network,
                                                             audio: request.audio, microphone: request.microphone,
                                                             clipboard: clipboardHandle, camera: cameraHandle,
                                                             agent: agentHandle))
            vm = VZVirtualMachine(configuration: config)
            vm.delegate = self
            if request.window { showWindow() }
            if let socket = request.socket {
                server = try ControlServer(path: socket) { [weak self] cmd in self?.handle(cmd) ?? ["error": "terminated"] }
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

    private func showWindow() {
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
        w.makeKeyAndOrderFront(nil)
        NSApp.activate(ignoringOtherApps: true)
        window = w
    }

    // Closing the window = let the guest shut down cleanly (no forced power-off)
    func windowShouldClose(_ sender: NSWindow) -> Bool {
        if vm.canRequestStop { try? vm.requestStop() }
        return false
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
            }
            window?.makeKeyAndOrderFront(nil)
            NSApp.activate(ignoringOtherApps: true)
            return ["ok": "window in front"]
        default:
            return ["error": "Unknown command \(command); available: \(RunnerClient.commands.joined(separator: ", "))"]
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

/// Unix socket with line-delimited JSON: {"command":"status"} → {"state":"running",…} (counterpart: OmaflakeCore.RunnerClient).
@MainActor
final class ControlServer {
    private let path: URL
    private let fd: Int32
    private let source: DispatchSourceRead

    init(path: URL, handler: @escaping @MainActor (String) -> [String: String]) throws {
        self.path = path
        unlink(path.path)   // leftover from an earlier run (socket file, no user content)
        let listener = socket(AF_UNIX, SOCK_STREAM, 0)
        guard listener >= 0 else { throw VMError("socket() failed") }
        var addr = sockaddr_un()
        addr.sun_family = sa_family_t(AF_UNIX)
        let bytes = Array(path.path.utf8)
        guard bytes.count < MemoryLayout.size(ofValue: addr.sun_path) else {
            throw VMError("Socket path too long (max. 103 bytes): \(path.path)")
        }
        withUnsafeMutableBytes(of: &addr.sun_path) { raw in
            raw.copyBytes(from: bytes)
            raw[bytes.count] = 0
        }
        addr.sun_len = UInt8(MemoryLayout<sockaddr_un>.size)
        let rc = withUnsafePointer(to: &addr) {
            $0.withMemoryRebound(to: sockaddr.self, capacity: 1) { bind(listener, $0, socklen_t(MemoryLayout<sockaddr_un>.size)) }
        }
        guard rc == 0, listen(listener, 8) == 0 else {
            Darwin.close(listener)
            throw VMError("Socket \(path.path) not usable: \(String(cString: strerror(errno)))")
        }
        fd = listener
        source = DispatchSource.makeReadSource(fileDescriptor: listener, queue: .main)
        source.setEventHandler {
            MainActor.assumeIsolated {
                let client = accept(listener, nil, nil)
                guard client >= 0 else { return }
                defer { Darwin.close(client) }
                var tv = timeval(tv_sec: 2, tv_usec: 0)
                setsockopt(client, SOL_SOCKET, SO_RCVTIMEO, &tv, socklen_t(MemoryLayout<timeval>.size))
                var data = Data()
                var buf = [UInt8](repeating: 0, count: 1024)
                while !data.contains(UInt8(ascii: "\n")) && data.count < 65536 {
                    let n = read(client, &buf, buf.count)
                    if n <= 0 { break }
                    data.append(contentsOf: buf[0..<n])
                }
                let obj = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
                let reply = handler(obj?["command"] as? String ?? "")
                let out = ((try? JSONSerialization.data(withJSONObject: reply)) ?? Data("{}".utf8)) + Data("\n".utf8)
                _ = out.withUnsafeBytes { write(client, $0.baseAddress, $0.count) }
            }
        }
        source.resume()
    }

    func close() {
        source.cancel()
        Darwin.close(fd)
        unlink(path.path)
    }
}
