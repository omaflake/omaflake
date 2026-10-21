// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

/// macOS guests on Virtualization.framework (spec §6.4, plan M10): every instance is an APFS clone of the
/// image folder in `instances/<name>/vm/`, run by its own detached `omaf-runner` process.
///
/// Network: `shared` = NAT (address from /var/db/dhcpd_leases, SSH on port 22), `none` = no network card.
/// `isolated` is not available: Virtualization.framework's NAT has no restricted mode, and a vmnet host-mode
/// network (`VZVmnetNetworkDeviceAttachment`, macOS 26) needs the restricted vmnet entitlement (plan BM27).
/// Ephemeral: a fresh clone at every start; the clone of the previous run is deleted (BM27).
public struct VZBackend: VMBackend {
    public let paths: Paths
    public let store: InstanceStore
    /// Signed omaf-runner (scripts/sign-runner.sh)
    public let runner: String
    public var kind: GuestKind { .macos }

    /// Virtualization.framework runs at most two macOS VMs per Mac (plan BM16).
    public static let maxRunning = 2

    public init(paths: Paths, store: InstanceStore, runner: String) {
        self.paths = paths
        self.store = store
        self.runner = runner
    }

    /// omaf-runner from VM_RUNNER, config.json (`runner`), next to the running executable, or PATH.
    public static func locateRunner(_ paths: Paths) throws -> String {
        let fm = FileManager.default
        if let env = ProcessInfo.processInfo.environment["VM_RUNNER"], !env.isEmpty { return try Shell.which(env) }
        if let configured = Settings.load(paths).runner { return try Shell.which((configured as NSString).expandingTildeInPath) }
        if let exe = Bundle.main.executableURL?.resolvingSymlinksInPath() {
            let dir = exe.deletingLastPathComponent()
            // In omaflake.app: its own helper bundle, so macOS does not take a VM window for the app (plan B65);
            // next to omaf (CLI builds): a plain sibling
            for candidate in [dir.appendingPathComponent("../Helpers/omaf-runner.app/Contents/MacOS/omaf-runner").standardized.path,
                              dir.appendingPathComponent("omaf-runner").path] where fm.isExecutableFile(atPath: candidate) {
                return candidate
            }
        }
        return try Shell.which("omaf-runner")
    }

    // MARK: Create, reset, remove

    public func create(name: String, recipe: String, image: ImageStore.Image, settings: RecipeDefaults) throws -> InstanceConfig {
        try store.validate(name: name)
        guard !store.exists(name) else { throw VMError("Instance \"\(name)\" already exists") }
        guard image.kind == .macos else { throw VMError("\(image.name) is not a macOS image") }
        try InstanceStore.checkRunsHere(image)
        try Self.checkNetwork(settings.network)
        let ip = paths.instance(name)
        try FileManager.default.createDirectory(at: ip.dir, withIntermediateDirectories: true)
        try FileManager.default.createDirectory(at: ip.run, withIntermediateDirectories: true)
        // The SSH port of a macOS guest is 22 on its own NAT address; no port forwarding
        var config = InstanceConfig(name: name, recipe: image.recipe.isEmpty ? recipe : image.recipe, image: image.name,
                                    imagePath: image.path.path, system: image.info.system, settings: settings, sshPort: 22)
        config.identity = OmaflakeID.randomWords()
        do {
            // Ephemeral instances get their clone at every start
            if !config.ephemeral { try cloneImage(config) }
        } catch {
            try? Disposal.delete([ip.dir])
            throw error
        }
        try store.save(config)
        return config
    }

    public func reset(_ c: InstanceConfig) throws {
        let ip = paths.instance(c.name)
        try Disposal.delete([ip.vm, ip.knownHosts])
        try cloneImage(c)
    }

    public func upgrade(_ c: InstanceConfig, to image: ImageStore.Image) throws {
        guard c.image != image.name else { throw VMError("\"\(c.name)\" already uses \(image.name)") }
        guard image.kind == .macos else { throw VMError("\(image.name) is not a macOS image") }
        try InstanceStore.checkRunsHere(image)
        let ip = paths.instance(c.name)
        try Disposal.delete([ip.vm, ip.knownHosts])
        var config = c
        config.image = image.name
        config.imagePath = image.path.path
        try store.replaceEmbeddedImage(c.name, with: image)
        try cloneImage(config)
        try store.save(config)
    }

    public func remove(_ c: InstanceConfig) throws { try store.remove(c.name) }

    /// `omaf-runner clone` (APFS clone, new machine identifier and MAC), then CPU/RAM from the instance settings.
    /// The disk keeps the image's size: growing a macOS system disk needs an APFS resize in the guest (OPEN).
    func cloneImage(_ c: InstanceConfig) throws {
        let ip = paths.instance(c.name)
        // The image inside the package (an exported or external VM may carry it), else the one in the data folder
        let source = [ip.embeddedImage, paths.imageDir(c.image)]
            .first { FileManager.default.fileExists(atPath: MacVMBundle($0).config.path) }
        guard let source else {
            throw VMError("Image missing: \(c.image) – rebuild it (omaf build \(c.recipe)) or import it, then omaf upgrade/reset")
        }
        try Shell.check(runner, ["clone", source.path, ip.vm.path])
        let vm = MacVMBundle(ip.vm)
        var cfg = try vm.loadConfig()
        cfg.cpus = c.cpus
        cfg.memoryMB = c.memoryGB * 1024
        try vm.save(cfg)
    }

    static func checkNetwork(_ mode: NetworkMode) throws {
        guard mode != .isolated else {
            throw VMError("Network mode \"isolated\" is not available for macOS guests (plan BM27) – use shared or none")
        }
    }

    // MARK: Start, stop, status

    public func start(_ c: InstanceConfig, options: StartOptions) throws -> pid_t {
        let ip = paths.instance(c.name)
        guard pid(c) == nil else { throw VMError("\"\(c.name)\" is already running") }
        try InstanceStore.checkRunsHere(c)
        try Self.checkNetwork(c.network)
        let running = store.list().filter { $0.guestKind == .macos && $0.name != c.name && pid($0) != nil }
        guard running.count < Self.maxRunning else {
            throw VMError("macOS allows at most \(Self.maxRunning) macOS VMs at the same time – running: "
                          + running.map(\.name).joined(separator: ", ") + " (plan BM16)")
        }
        try checkEntitlement()
        if c.ephemeral, FileManager.default.fileExists(atPath: ip.vm.path) {
            try reset(c)
        } else if !FileManager.default.fileExists(atPath: ip.vm.path) {
            try cloneImage(c)
        }
        var args = ["run", ip.vm.path, "--socket", ip.runnerSocket.path, "--pid-file", ip.runnerPid.path]
        if options.display != .none { args.append("--window") }
        if c.network == .none { args.append("--no-network") }
        if c.soundOn { args.append("--audio") }
        if c.microphoneOn { args.append("--microphone") }
        if c.clipboardOn { args.append("--clipboard") }
        // The omaflake agent's port, always (spec §11.9); the guest side for macOS starts with plan B106's follow-up
        args += ["--agent-state", ip.agentState.path]
        // Camera: the Mac side and the port are ready; the guest's camera extension comes after M12 (plan B49)
        if c.cameraOn {
            args.append("--camera")
            if ProcessInfo.processInfo.environment["OMAFLAKE_CAMERA_TEST_PATTERN"] == "1" { args.append("--camera-test-pattern") }
        }
        if c.share {
            args += ["--share", try paths.prepareShare(c).path]
        }
        let child = try spawnDetached(runner, args, log: ip.runnerLog)

        // Ready as soon as the runner reports the VM running; if it exits first, show its log
        let deadline = Date().addingTimeInterval(30)
        while Date() < deadline {
            var st: Int32 = 0
            if waitpid(child, &st, WNOHANG) == child {
                throw VMError("omaf-runner exited during startup:\n\(tail(ip.runnerLog, lines: 5))")
            }
            if let reply = try? RunnerClient.send("status", socket: ip.runnerSocket, timeout: 1), reply["state"] == "running" {
                return child
            }
            usleep(300_000)
        }
        throw VMError("omaf-runner does not report the VM as running (\(ip.runnerLog.path))")
    }

    /// Shutdown via SSH (`sudo -n /sbin/shutdown -h now`, allowed by darwin-guest.nix; a stop request from the
    /// runner hangs on the guest's confirmation dialog when a user is logged in, plan BM17) → runner stop request
    /// → after `timeout` the runner forces the VM off → SIGKILL.
    public func stop(_ c: InstanceConfig, timeout: TimeInterval) throws -> StopResult {
        guard let pid = pid(c) else { return .alreadyStopped }
        let ip = paths.instance(c.name)
        if let host = address(c) {
            let args = sshArguments(c, host: host, command: ["sudo", "-n", "/sbin/shutdown", "-h", "now"], batch: true)
            _ = try? Shell.run(args[0], Array(args.dropFirst()))
        } else {
            _ = try? RunnerClient.send("stop", socket: ip.runnerSocket)
        }
        if waitForExit(pid, timeout: timeout) { return .powerdown }
        if (try? RunnerClient.send("kill", socket: ip.runnerSocket)) != nil, waitForExit(pid, timeout: 10) { return .forcedOff }
        kill(pid, SIGKILL)
        _ = waitForExit(pid, timeout: 5)
        return .killed
    }

    public func status(_ c: InstanceConfig) -> VMStatus { pid(c) == nil ? .stopped : .running }

    public func pid(_ c: InstanceConfig) -> pid_t? { livePid(file: paths.instance(c.name).runnerPid, executable: "omaf-runner") }

    /// Without the entitlement, Virtualization.framework refuses to start – say so before spawning.
    public func checkEntitlement() throws {
        let r = try Shell.run("/usr/bin/codesign", ["-d", "--entitlements", "-", "--xml", runner])
        guard r.stdout.contains("com.apple.security.virtualization") else {
            throw VMError("\(runner) lacks the entitlement com.apple.security.virtualization – sign it with scripts/sign-runner.sh")
        }
    }

    // MARK: SSH

    /// NAT address of the running guest (from the DHCP leases, by the MAC address of its clone).
    public func address(_ c: InstanceConfig) -> String? {
        guard c.network != .none, let cfg = try? MacVMBundle(paths.instance(c.name).vm).loadConfig() else { return nil }
        return DHCPLeases.ip(for: cfg.macAddress)
    }

    /// Once SSH answers, the guest is named after the instance (plan BM28) and gets its clipboard agent (B48).
    /// What the guest's omaflake agent last reported (agent.json, written by omaf-runner) – nil while stopped
    public func agentState(_ c: InstanceConfig) -> AgentState? {
        guard status(c) == .running else { return nil }
        return AgentState.load(paths.instance(c.name).agentState)
    }

    public func waitForSSH(_ c: InstanceConfig, timeout: TimeInterval) -> Bool {
        let deadline = Date().addingTimeInterval(timeout)
        while Date() < deadline {
            if let host = address(c), Ports.sshBanner(host: host, port: 22) != nil {
                _ = applyHostName(c, host: host)
                if c.clipboardOn { _ = startClipboardAgent(c, host: host) }
                return true
            }
            if pid(c) == nil { return false }
            sleep(2)
        }
        return false
    }

    /// Sets HostName, LocalHostName and ComputerName to the instance name. The image carries its recipe's name
    /// (nix-darwin sets it only on activation), so every clone would otherwise share it. Needs the sudo rule from
    /// darwin-guest.nix; with an older image it fails and the guest keeps the recipe's name.
    @discardableResult
    public func applyHostName(_ c: InstanceConfig, host: String) -> Bool {
        let command = ["HostName", "LocalHostName", "ComputerName"]
            .map { "sudo -n /usr/sbin/scutil --set \($0) \(c.name)" }.joined(separator: " && ")
        let args = sshArguments(c, host: host, command: [command], batch: true)
        return (try? Shell.run(args[0], Array(args.dropFirst())))?.status == 0
    }

    /// Starts the guest's clipboard agent (darwin-guest.nix) – over SSH instead of a LaunchAgent, which macOS would
    /// announce as a background item in every clone (BM25). Images without it simply have no shared clipboard.
    @discardableResult
    public func startClipboardAgent(_ c: InstanceConfig, host: String) -> Bool {
        let args = sshArguments(c, host: host, command: ["/run/current-system/sw/bin/omaflake-clipboard-agent"], batch: true)
        return (try? Shell.run(args[0], Array(args.dropFirst())))?.status == 0
    }

    public func sshArguments(_ c: InstanceConfig, command: [String]) -> [String] {
        // Without a lease yet, ssh fails with a clear "could not resolve" instead of a wrong host
        sshArguments(c, host: address(c) ?? "no-address-yet.\(c.name).invalid", command: command, batch: false)
    }

    func sshArguments(_ c: InstanceConfig, host: String, command: [String], batch: Bool) -> [String] {
        var args = SSHCommand.arguments(c, paths: paths, host: host, port: 22, command: command)
        if batch { args.insert(contentsOf: ["-o", "BatchMode=yes", "-o", "ConnectTimeout=10"], at: 1) }
        return args
    }
}
