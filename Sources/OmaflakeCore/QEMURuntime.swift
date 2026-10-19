// SPDX-License-Identifier: AGPL-3.0-or-later
import CoreGraphics
import Darwin
import Foundation

/// Starts, stops and monitors QEMU processes. QEMU runs in its own session (setsid)
/// and outlives CLI/app; state is derived only from files/sockets (plan §3).
public struct QEMURuntime: Sendable {
    public let paths: Paths
    public let qemu: String
    public let firmware: String

    public init(paths: Paths, qemu: String, firmware: String) {
        self.paths = paths
        self.qemu = qemu
        self.firmware = firmware
    }

    /// QEMU from config.json, VM_QEMU or PATH; firmware and qemu-img live next to the binary.
    public static func locate(_ paths: Paths) throws -> (runtime: QEMURuntime, qemuImg: String) {
        let settings = Settings.load(paths)
        let env = ProcessInfo.processInfo.environment["VM_QEMU"]
        let qemu = try Shell.which(env ?? settings.qemu ?? "qemu-system-aarch64")
        let bin = URL(fileURLWithPath: qemu).resolvingSymlinksInPath().deletingLastPathComponent()
        let firmware = bin.deletingLastPathComponent().appendingPathComponent("share/qemu/edk2-aarch64-code.fd").path
        guard FileManager.default.fileExists(atPath: firmware) else {
            throw VMError("UEFI firmware not found: \(firmware)")
        }
        let qemuImgNext = bin.appendingPathComponent("qemu-img").path
        let qemuImg = FileManager.default.isExecutableFile(atPath: qemuImgNext) ? qemuImgNext : try Shell.which("qemu-img")
        return (QEMURuntime(paths: paths, qemu: qemu, firmware: firmware), qemuImg)
    }

    // MARK: Status

    /// PID of the instance's running QEMU, otherwise nil (a stale pidfile is ignored).
    public func pid(_ name: String) -> pid_t? {
        let ip = paths.instance(name)
        guard let text = try? String(contentsOf: ip.pidFile, encoding: .utf8),
              let pid = pid_t(text.trimmingCharacters(in: .whitespacesAndNewlines)), pid > 0,
              kill(pid, 0) == 0 || errno == EPERM else { return nil }
        // Rule out PID reuse: the process must be QEMU
        return executablePath(of: pid)?.contains("qemu-system") == true ? pid : nil
    }

    public func status(_ name: String) -> VMStatus { pid(name) == nil ? .stopped : .running }

    // MARK: Start

    /// Capabilities of the binary: GL via `-device help`, guest_owner via the option string in the binary.
    public func capabilities() -> QEMUCapabilities {
        let devices = (try? Shell.run(qemu, ["-device", "help"]))?.stdout ?? ""
        let binary = (try? Data(contentsOf: URL(fileURLWithPath: qemu), options: .mappedIfSafe)) ?? Data()
        return QEMUCapabilities(gl: devices.contains("\"virtio-gpu-gl-pci\""),
                                guestOwner: binary.range(of: Data("guest_owner_uid".utf8)) != nil)
    }

    public func start(_ config: InstanceConfig, display: DisplayMode, gl: GLBackend = .on) throws -> pid_t {
        let ip = paths.instance(config.name)
        guard pid(config.name) == nil else { throw VMError("\"\(config.name)\" is already running") }
        guard InstanceStore(paths: paths, qemuImg: "qemu-img").backingImage(config) != nil else {
            throw VMError("Base image missing: \(config.image) – rebuild it (omaf build \(config.recipe)) or import it, then omaf reset")
        }
        try FileManager.default.createDirectory(at: ip.run, withIntermediateDirectories: true)
        if config.network != .none, !Ports.isFree(config.sshPort) {
            throw VMError("SSH port \(config.sshPort) is in use")
        }
        let caps = capabilities()
        if display == .gpu, !caps.gl {
            throw VMError("\(qemu) does not support virtio-gpu-gl – use a custom build (qemu/build.sh, omaf config --qemu …) or --no-gl")
        }
        var share: String?
        if config.share {
            // The default folder is opened up for 9p (plan B18); a folder the user chose keeps its permissions
            share = try paths.prepareShare(config, mode: 0o777).path
        }
        var cmd = QEMUCommand(firmware: firmware, paths: ip, sharePath: share, display: display,
                              gl: gl, capabilities: caps)
        if display != .none { cmd.initialResolution = Self.mainScreenResolution() }
        let args = cmd.arguments(for: config)
        // Dock icon for the window (patch qemu-cocoa-dock-icon; older builds ignore the variable)
        var extraEnv: [String: String] = [:]
        if display != .none, let icon = try? Brand.dockIconFile(paths) { extraEnv["OMAFLAKE_DOCK_ICON"] = icon.path }
        // What an earlier run's agent reported must not count for this one
        try? FileManager.default.removeItem(at: ip.agentState)
        let child = try spawnDetached(qemu, args, log: ip.qemuLog, environment: extraEnv)

        // Ready as soon as QMP answers; if QEMU dies first, show the log
        let deadline = Date().addingTimeInterval(15)
        while Date() < deadline {
            var st: Int32 = 0
            if waitpid(child, &st, WNOHANG) == child {
                throw VMError("QEMU exited during startup:\n\(tail(ip.qemuLog, lines: 15))")
            }
            if let qmp = try? QMPClient(socket: ip.qmpSocket, timeout: 1), (try? qmp.status()) != nil {
                startBridge(ip, clipboard: config.clipboardOn, camera: config.cameraOn)
                return pid(config.name) ?? child
            }
            usleep(200_000)
        }
        throw VMError("QEMU does not answer over QMP (\(ip.qmpSocket.path))")
    }

    /// The Mac side of the agent, clipboard and camera (omaf-runner bridge), detached like QEMU: it ends when the VM
    /// closes a port. Without omaf-runner the VM simply runs without them. OMAFLAKE_CAMERA_TEST_PATTERN=1 sends a test
    /// pattern instead of the camera (tests without camera permission).
    func startBridge(_ ip: InstancePaths, clipboard: Bool, camera: Bool) {
        guard let runner = try? VZBackend.locateRunner(paths) else { return }
        var args = ["bridge", "--agent", ip.agentSocket.path, "--agent-state", ip.agentState.path]
        if clipboard { args += ["--clipboard", ip.clipSocket.path] }
        if camera { args += ["--camera", ip.cameraSocket.path] }
        if camera, ProcessInfo.processInfo.environment["OMAFLAKE_CAMERA_TEST_PATTERN"] == "1" { args.append("--camera-test-pattern") }
        _ = try? spawnDetached(runner, args, log: ip.bridgeLog)
    }

    /// Without the guest_owner patch, 9p shows Mac files in the guest with the Mac UID/permissions (502 ≠ guest user/1000).
    /// So that the guest user may create files in the share, the share folder itself is 0777 (only visible locally via 9p);
    /// files created on the Mac then stay read-only in the guest (plan B18).
    /// Initial resolution from the main screen (points × Retina factor), nil without a screen.
    static func mainScreenResolution() -> (width: Int, height: Int)? {
        guard let mode = CGDisplayCopyDisplayMode(CGMainDisplayID()), mode.width > 0 else { return nil }
        let scale = Double(mode.pixelWidth) / Double(mode.width)
        return QEMUCommand.initialResolution(screenPoints: (Double(mode.width), Double(mode.height)), scale: scale)
    }

    // MARK: Stop

    /// ACPI powerdown → after `timeout` QMP `quit` → SIGKILL.
    public func stop(_ name: String, timeout: TimeInterval = 60) throws -> StopResult {
        guard let pid = pid(name) else { return .alreadyStopped }
        let socket = paths.instance(name).qmpSocket
        if let qmp = try? QMPClient(socket: socket) {
            _ = try? qmp.execute("system_powerdown")
            if waitForExit(pid, timeout: timeout) { return .powerdown }
            _ = try? qmp.execute("quit")
            if waitForExit(pid, timeout: 5) { return .quit }
        }
        kill(pid, SIGKILL)
        _ = waitForExit(pid, timeout: 5)
        return .killed
    }

    /// What the instance's agent last reported – nil while the VM is stopped or before the agent's hello.
    public func agentState(_ name: String) -> AgentState? {
        guard pid(name) != nil else { return nil }
        return AgentState.load(paths.instance(name).agentState)
    }

    /// Ready once the agent says hello (also without network) or, for images without the agent, SSH answers.
    public func waitForReady(_ config: InstanceConfig, timeout: TimeInterval) -> Readiness? {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let state = agentState(config.name), state.isReady { return .agent(state) }
            if config.network != .none, Ports.sshBanner(port: config.sshPort, timeout: 1) != nil {
                // An image with the agent says hello as early as sshd: give it a moment to win
                if let state = agentState(config.name), state.isReady { return .agent(state) }
                return .ssh
            }
            if pid(config.name) == nil { return nil }
            sleep(1)
        }
        return nil
    }

    /// Waits until the instance's SSH server sends its banner.
    public func waitForSSH(_ config: InstanceConfig, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if Ports.sshBanner(port: config.sshPort) != nil { return true }
            if pid(config.name) == nil { return false }
            sleep(2)
        }
        return false
    }
}

/// Last lines of a text file.
public func tail(_ url: URL, lines: Int) -> String {
    guard let text = try? String(contentsOf: url, encoding: .utf8) else { return "" }
    return text.split(separator: "\n", omittingEmptySubsequences: false).suffix(lines).joined(separator: "\n")
}

/// Exclusive lock per instance (CLI and app at the same time, plan §6). Released when the process exits.
public final class InstanceLock {
    private let fd: Int32

    public init(_ paths: InstancePaths) throws {
        try? FileManager.default.createDirectory(at: paths.lockFile.deletingLastPathComponent(), withIntermediateDirectories: true)
        // `fd` is set only once the lock is held: an init that throws after setting every stored property still
        // runs deinit, which would close the descriptor a second time – by then perhaps another file's
        // O_CLOEXEC: processes started meanwhile (QEMU, the runner, nix, a shell) must not inherit the descriptor – a
        // flock lock lasts while any process still has the file open (plan B104)
        let fd = open(paths.lockFile.path, O_CREAT | O_RDWR | O_CLOEXEC, 0o644)
        guard fd >= 0 else { throw VMError("Cannot create lock file: \(paths.lockFile.path)") }
        guard flock(fd, LOCK_EX | LOCK_NB) == 0 else {
            let e = errno
            close(fd)
            if e == EWOULDBLOCK { throw VMError("Instance is currently busy (locked)") }
            throw VMError("Cannot lock \(paths.lockFile.path): \(String(cString: strerror(e)))")
        }
        self.fd = fd
    }

    deinit { close(fd) }
}

/// Exclusive lock on a file (flock), released when it is deinitialized or the process exits – also across app and CLI.
public final class FileLock {
    private let fd: Int32
    private let file: URL
    /// Removes the lock file when released – for per-build locks whose names are never reused (plan B84)
    private let removeWhenDone: Bool

    public init(_ file: URL, busy: String, removeWhenDone: Bool = false) throws {
        try FileManager.default.createDirectory(at: file.deletingLastPathComponent(), withIntermediateDirectories: true)
        // Stored properties are set only once the lock is held: an init that throws with every property set still
        // runs deinit – it closed the descriptor a second time (perhaps another file's by then) and, with
        // removeWhenDone, unlinked the lock file another build was holding (plan B104)
        // O_CLOEXEC: see InstanceLock – an inherited descriptor kept the lock after this one was gone
        let fd = open(file.path, O_CREAT | O_RDWR | O_CLOEXEC, 0o644)
        guard fd >= 0 else { throw VMError("Cannot create lock file: \(file.path)") }
        guard flock(fd, LOCK_EX | LOCK_NB) == 0 else {
            let e = errno
            close(fd)
            // Busy only when someone holds the lock; anything else is an error of its own (not "busy")
            if e == EWOULDBLOCK { throw VMError(busy) }
            throw VMError("Cannot lock \(file.path): \(String(cString: strerror(e)))")
        }
        self.fd = fd
        self.file = file
        self.removeWhenDone = removeWhenDone
    }

    deinit {
        // Unlink while still holding the lock: no one can take a file that is going away
        if removeWhenDone { unlink(file.path) }
        close(fd)
    }
}
