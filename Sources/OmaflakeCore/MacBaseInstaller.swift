// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// A macOS base from Apple's IPSW (spec §6.5 step 1, docs/macos-base.md), shared by `omaf macos install` and the app:
///   `omaf-runner install` → `bases/mac-fresh-<build>` (never started, read-only) → APFS clone `bases/mac-base-<build>`
///   with a new identity → first boot with provisioning (user, automatic login, SSH; plan BM11) → your SSH public
///   keys into the guest user's authorized_keys → shut down → read-only. No manual steps (plan BM29).
/// omaflake never distributes macOS itself: the IPSW comes from Apple, the base is built on the user's Mac.
/// Existing folders are reused, never overwritten. The password goes to the runner and ssh/sudo on stdin or via
/// an askpass helper that reads it from its environment – never into a file or an argument.
public struct MacBaseInstaller: Sendable {
    public let paths: Paths
    public let vz: VZBackend

    public init(paths: Paths, vz: VZBackend) {
        self.paths = paths
        self.vz = vz
    }

    public struct Request: Sendable {
        /// Local IPSW; nil = the latest supported by this Mac, downloaded from Apple into `~/.omaflake/ipsw/`
        public var ipsw: URL?
        public var cpus: Int
        public var memoryGB: Int
        public var diskGB: Int
        public var user: String
        public var fullName: String
        /// Password of the new guest user; nil = stop after the clone, without the first boot
        public var password: String?
        /// Public keys for the guest user's authorized_keys (default: ~/.ssh/*.pub)
        public var authorizedKeys: [String]
        /// Show the first boot in a window (headless otherwise)
        public var window: Bool

        public init(ipsw: URL? = nil, cpus: Int = 4, memoryGB: Int = 8, diskGB: Int = 80,
                    user: String, fullName: String, password: String?,
                    authorizedKeys: [String] = MacBaseInstaller.defaultPublicKeys(), window: Bool = false) {
            self.authorizedKeys = authorizedKeys
            self.window = window
            self.ipsw = ipsw
            self.cpus = cpus
            self.memoryGB = memoryGB
            self.diskGB = diskGB
            self.user = user
            self.fullName = fullName
            self.password = password
        }
    }

    public enum Outcome: Sendable, Equatable {
        /// The base for this build existed already; nothing was changed
        case existing(URL)
        /// New base; `ready` = provisioned, keys installed, shut down and read-only
        case created(URL, ready: Bool)
    }

    /// The Mac user's SSH public keys (`~/.ssh/*.pub`) – the keys `omaf ssh` and the image build log in with.
    public static func defaultPublicKeys() -> [String] {
        let dir = FileManager.default.homeDirectoryForCurrentUser.appendingPathComponent(".ssh")
        let files = (try? FileManager.default.contentsOfDirectory(atPath: dir.path)) ?? []
        return files.filter { $0.hasSuffix(".pub") }.sorted().compactMap {
            (try? String(contentsOf: dir.appendingPathComponent($0), encoding: .utf8))?.trimmingCharacters(in: .whitespacesAndNewlines)
        }.filter { $0.hasPrefix("ssh-") || $0.hasPrefix("ecdsa-") }
    }

    /// The latest macOS this Mac can run as a VM (asks Apple, no download), nil if unknown.
    public func latestRelease() -> MacOSRelease? { MacOSRelease.latest(runner: vz.runner) }

    /// Build of the latest IPSW supported by this Mac, nil if unknown.
    public func latestBuild() -> String? { latestRelease()?.build }

    /// Downloads the IPSW of `release` into `ipsw/` (continues an interrupted download); `started` gets the
    /// downloader so the caller can pause it. Progress goes to `log` in whole percent.
    @discardableResult
    public func downloadIPSW(_ release: MacOSRelease, started: (IPSWDownloader) -> Void = { _ in },
                             progress: @escaping @Sendable (Int64, Int64) -> Void = { _, _ in },
                             log: @escaping @Sendable (String) -> Void) throws -> URL {
        let file = IPSWDownloader.file(release, in: paths.ipsw)
        if FileManager.default.fileExists(atPath: file.path) { return file }
        let needed = (release.bytes ?? 27_000_000_000) + Prerequisites.macSpaceNeeded
        if let free = freeSpace(at: paths.root), free < needed {
            throw VMError("Not enough space: \(gigabytes(free)) free, about \(gigabytes(needed)) needed (IPSW, installation, image)")
        }
        let downloader = IPSWDownloader(release: release, directory: paths.ipsw)
        started(downloader)
        log("Downloading macOS \(release.version) (\(release.build)) from Apple → \(file.path)")
        let last = LastPercent()
        return try KeepAwake.during("Downloading macOS") {
            try downloader.run { done, total in
                progress(done, total)
                if total > 0, let pct = last.next(Int(done * 100 / total)) { log("Download: \(pct) %") }
            }
        }
    }

    /// "macOS 27.0.1 (26A434)" in the first line → "26A434"
    static func build(fromLatest output: String) -> String? {
        guard let line = output.split(separator: "\n").first,
              let open = line.lastIndex(of: "("), let close = line.lastIndex(of: ")"), open < close else { return nil }
        return String(line[line.index(after: open)..<close])
    }

    public func fresh(build: String) -> URL { paths.bases.appendingPathComponent("mac-fresh-\(build)", isDirectory: true) }
    public func base(build: String) -> URL { paths.bases.appendingPathComponent("mac-base-\(build)", isDirectory: true) }

    /// The base that `install` would end at, if it exists already (only known up front without `ipsw`).
    public func existingBase(_ r: Request) -> URL? {
        guard r.ipsw == nil, let build = latestBuild() else { return nil }
        let dir = base(build: build)
        return Self.isReady(dir) ? dir : nil
    }

    /// Runs all steps; progress (including the runner's) goes to `log` line by line. Blocks until the
    /// first-boot VM is shut down.
    public func install(_ r: Request, started: (IPSWDownloader) -> Void = { _ in },
                        progress: @escaping @Sendable (Int64, Int64) -> Void = { _, _ in },
                        log: @escaping @Sendable (String) -> Void) throws -> Outcome {
        try KeepAwake.during("Creating a macOS base") { try installAwake(r, started: started, progress: progress, log: log) }
    }

    private func installAwake(_ r: Request, started: (IPSWDownloader) -> Void, progress: @escaping @Sendable (Int64, Int64) -> Void,
                              log: @escaping @Sendable (String) -> Void) throws -> Outcome {
        let fm = FileManager.default
        try vz.checkEntitlement()
        try fm.createDirectory(at: paths.bases, withIntermediateDirectories: true)

        // 1. Fresh installation, named after the installed build; the IPSW from Apple unless one was given
        var freshDir: URL?
        var ipsw = r.ipsw
        if ipsw == nil {
            guard let release = latestRelease() else { throw VMError("Apple's macOS catalog is not reachable – online? (or choose an IPSW file)") }
            if fm.fileExists(atPath: fresh(build: release.build).path) {
                log("\(fresh(build: release.build).lastPathComponent) exists – reusing it")
                freshDir = fresh(build: release.build)
            } else {
                ipsw = try downloadIPSW(release, started: started, progress: progress, log: log)
            }
        }
        let freshVM = try freshDir ?? installFresh(r, ipsw: ipsw!, log: log)
        let build = try MacVMBundle(freshVM).loadConfig().osBuild ?? "unknown"

        // 2. Base = APFS clone with a new machine identifier and MAC
        let baseDir = base(build: build)
        if Self.isReady(baseDir) {
            log("\(baseDir.lastPathComponent) exists already – nothing to do")
            return .existing(baseDir)
        }
        if fm.fileExists(atPath: baseDir.path) {
            // An unfinished base (e.g. no SSH on its first boot): provisioning is done, continue after it
            guard let password = r.password else { return .created(baseDir, ready: false) }
            log("\(baseDir.lastPathComponent) is unfinished – continuing (same password as before)")
            try finish(baseDir, r: r, password: password, provision: false, log: log)
            return .created(baseDir, ready: true)
        }
        try Shell.check(vz.runner, ["clone", freshVM.path, baseDir.path])
        log("Base \(baseDir.path) cloned from \(freshVM.lastPathComponent)")
        guard let password = r.password else { return .created(baseDir, ready: false) }
        guard !r.authorizedKeys.isEmpty else {
            throw VMError("No SSH public key in ~/.ssh – create one first: ssh-keygen -t ed25519")
        }
        try finish(baseDir, r: r, password: password, provision: true, log: log)
        return .created(baseDir, ready: true)
    }

    /// Marker of a finished base (keys installed, shut down, read-only); not one of the VM files, so clones skip it.
    public static let readyMarker = ".omaflake-base-ready"

    public static func isReady(_ dir: URL) -> Bool {
        FileManager.default.fileExists(atPath: dir.appendingPathComponent(readyMarker).path)
    }

    /// Guest user of a base: written into its ready marker; for older bases from the log of its first boot.
    public static func user(of dir: URL, logs: URL) -> String? {
        if let text = try? String(contentsOf: dir.appendingPathComponent(readyMarker), encoding: .utf8) {
            let user = text.trimmingCharacters(in: .whitespacesAndNewlines)
            if !user.isEmpty { return user }
        }
        let log = (try? String(contentsOf: logs.appendingPathComponent("first-boot-\(dir.lastPathComponent).log"), encoding: .utf8)) ?? ""
        return userFromFirstBootLog(log)
    }

    /// "[00:12:48] Provisioning: user alice, auto-login on, SSH on" → "alice" (the last such line)
    static func userFromFirstBootLog(_ log: String) -> String? {
        guard let line = log.split(separator: "\n").last(where: { $0.contains("Provisioning: user ") }),
              let range = line.range(of: "Provisioning: user ") else { return nil }
        let user = line[range.upperBound...].prefix { $0 != "," && $0 != " " }
        return user.isEmpty ? nil : String(user)
    }

    /// 3. Boot (the first one with provisioning), keys, shutdown, read-only. The first boot after an installation
    /// sometimes brings SSH up only much later (BM30); then the VM is restarted once – provisioning is done by then.
    private func finish(_ dir: URL, r: Request, password: String, provision: Bool, log: @escaping @Sendable (String) -> Void) throws {
        let fm = FileManager.default
        try fm.createDirectory(at: paths.logs, withIntermediateDirectories: true)
        let logFile = paths.logs.appendingPathComponent("first-boot-\(dir.lastPathComponent).log")
        let socket = dir.appendingPathComponent("runner.sock"), pidFile = dir.appendingPathComponent("runner.pid")
        if !fm.fileExists(atPath: logFile.path) { fm.createFile(atPath: logFile.path, contents: nil) }
        let out = try FileHandle(forWritingTo: logFile)
        defer { try? out.close() }
        _ = try? out.seekToEnd()

        func boot(provisioning: Bool) throws -> Process {
            log(provisioning ? "First boot\(r.window ? " in a window" : " (headless)") with provisioning: user \(r.user), automatic login, SSH (log: \(logFile.path))"
                             : "Booting \(dir.lastPathComponent)\(r.window ? " in a window" : " (headless)")")
            var args = ["run", dir.path, "--socket", socket.path, "--pid-file", pidFile.path]
            if provisioning {
                args += ["--user", r.user, "--full-name", r.fullName, "--password-stdin", "--auto-login", "--ssh"]
            }
            if r.window { args.append("--window") }
            let p = Process()
            p.executableURL = URL(fileURLWithPath: vz.runner)
            p.arguments = args
            let stdin = Pipe()
            p.standardInput = stdin
            p.standardOutput = out
            p.standardError = out
            try p.run()
            if provisioning { stdin.fileHandleForWriting.write(Data((password + "\n").utf8)) }
            try? stdin.fileHandleForWriting.close()
            return p
        }
        func forceOff(_ p: Process) {
            guard p.isRunning else { return }
            if (try? RunnerClient.send("kill", socket: socket)) == nil { p.terminate() }
            p.waitUntilExit()
        }
        let mac = try MacVMBundle(dir).loadConfig().macAddress
        /// Address from the DHCP leases once its SSH server answers, nil after `seconds`
        func waitForSSH(_ p: Process, seconds: TimeInterval) throws -> String? {
            let deadline = Date().addingTimeInterval(seconds)
            while Date() < deadline {
                guard p.isRunning else { throw VMError("omaf-runner exited:\n\(tail(logFile, lines: 3))") }
                if let a = DHCPLeases.ip(for: mac), Ports.sshBanner(host: a, port: 22) != nil { return a }
                sleep(3)
            }
            return nil
        }

        var runner = try boot(provisioning: provision)
        do {
            var ip = try waitForSSH(runner, seconds: provision ? 480 : 300)
            if ip == nil && provision {
                log("No SSH yet – restarting the VM once (provisioning is done)")
                forceOff(runner)
                runner = try boot(provisioning: false)
                ip = try waitForSSH(runner, seconds: 300)
            }
            guard ip != nil else { throw VMError("No SSH from \(dir.lastPathComponent) (log: \(logFile.path)) – try Create Base again, it continues") }
            log("Guest at \(ip!), installing \(r.authorizedKeys.count) SSH key(s)")

            // Keys with password login (askpass), then check the key login
            let keys = Data((r.authorizedKeys.joined(separator: "\n") + "\n").utf8)
            var installed = false
            for _ in 0..<20 where !installed {
                installed = (try? passwordSSH(ip!, user: r.user, password: password,
                                              command: "umask 077; mkdir -p ~/.ssh && cat >> ~/.ssh/authorized_keys", input: keys))?.status == 0
                if !installed { sleep(3) }
            }
            guard installed else { throw VMError("Could not log in as \(r.user) with the password to install the SSH keys") }
            let keyLogin = try Shell.run("/usr/bin/ssh", MacImageBuilder.BuildVM.sshOptions + ["\(r.user)@\(ip!)", "true"])
            guard keyLogin.status == 0 else { throw VMError("SSH key login as \(r.user) failed after installing the keys") }

            // Shut down (sudo with the password on stdin), then read-only
            log("Shutting down")
            _ = try? Shell.run("/usr/bin/ssh", MacImageBuilder.BuildVM.sshOptions + ["\(r.user)@\(ip!)", "sudo -S -p '' /sbin/shutdown -h now"],
                               input: Data((password + "\n").utf8))
            let off = Date().addingTimeInterval(180)
            while runner.isRunning {
                guard Date() < off else { throw VMError("The base did not shut down") }
                sleep(2)
            }
        } catch {
            forceOff(runner)
            log("The base is left as it is for inspection: \(dir.path)")
            throw error
        }
        try? Disposal.delete([socket, pidFile])
        // The marker carries the guest user: image builds log in as this user (and must match the recipes' macUser)
        fm.createFile(atPath: dir.appendingPathComponent(Self.readyMarker).path, contents: Data((r.user + "\n").utf8))
        try Shell.check("/bin/chmod", ["-R", "a-w", dir.path])
        log("Base ready: \(dir.path) (read-only)")
    }

    /// ssh with password authentication: an askpass helper (no secret in it) prints the password from its
    /// environment, which only this ssh process and its helper get.
    private func passwordSSH(_ host: String, user: String, password: String, command: String, input: Data) throws -> Shell.Result {
        let askpass = paths.root.appendingPathComponent("cache/askpass.sh")
        if !FileManager.default.fileExists(atPath: askpass.path) {
            try FileManager.default.createDirectory(at: askpass.deletingLastPathComponent(), withIntermediateDirectories: true)
            try "#!/bin/sh\nprintf '%s\\n' \"$OMAFLAKE_ASKPASS\"\n".write(to: askpass, atomically: true, encoding: .utf8)
            try FileManager.default.setAttributes([.posixPermissions: 0o700], ofItemAtPath: askpass.path)
        }
        let opts = ["-o", "PubkeyAuthentication=no", "-o", "PreferredAuthentications=password,keyboard-interactive",
                    "-o", "NumberOfPasswordPrompts=1", "-o", "ConnectTimeout=10", "-o", "UserKnownHostsFile=/dev/null",
                    "-o", "StrictHostKeyChecking=no", "-o", "LogLevel=ERROR"]
        return try Shell.run("/usr/bin/ssh", opts + ["\(user)@\(host)", command], input: input,
                             environment: ["SSH_ASKPASS": askpass.path, "SSH_ASKPASS_REQUIRE": "force",
                                           "DISPLAY": ":0", "OMAFLAKE_ASKPASS": password])
    }

    /// `omaf-runner install` into `bases/.install-<time>`, renamed to mac-fresh-<build> and made read-only.
    private func installFresh(_ r: Request, ipsw: URL, log: @escaping @Sendable (String) -> Void) throws -> URL {
        let fm = FileManager.default
        let tmp = paths.bases.appendingPathComponent(".install-\(Int(Date().timeIntervalSince1970))", isDirectory: true)
        var args = ["install", tmp.path, "--cpus", "\(r.cpus)", "--memory-gb", "\(r.memoryGB)", "--disk-gb", "\(r.diskGB)"]
        args += ["--ipsw", ipsw.path]
        let result = try Shell.stream(vz.runner, args, mergeStdout: true, onLine: log)
        guard result.status == 0 else {
            if fm.fileExists(atPath: tmp.path) {
                try? Disposal.delete([tmp])
                throw VMError("Installation failed (\(result.status)); the incomplete folder was deleted")
            }
            throw VMError("Installation failed (\(result.status))")
        }
        let build = try MacVMBundle(tmp).loadConfig().osBuild ?? "unknown"
        let dir = fresh(build: build)
        if fm.fileExists(atPath: dir.path) {
            try Disposal.delete([tmp])
            log("\(dir.lastPathComponent) existed already – using it; the new installation was deleted")
        } else {
            try fm.moveItem(at: tmp, to: dir)
            try Shell.check("/bin/chmod", ["-R", "a-w", dir.path])
            log("Installed: \(dir.path) (read-only, never started)")
        }
        return dir
    }
}

/// Reports a percentage only when it changed (log lines in whole percent).
final class LastPercent: @unchecked Sendable {
    private let lock = NSLock()
    private var last = -1

    func next(_ pct: Int) -> Int? {
        lock.withLock {
            guard pct != last else { return nil }
            last = pct
            return pct
        }
    }
}
