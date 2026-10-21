// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

/// Builds a macOS image from a base and a nix-darwin recipe (spec §6.5, plan M9/M10; formerly
/// `scripts/macos-build-image.sh`):
///   build the system closure on the Mac → clone the base (new identity) → start headless → install Nix →
///   nix copy → activate → shut down → `images/<recipe>-<rev>-<hash>/` (read-only) + `current/<recipe>`
///
/// The guest user's password is needed once for sudo. It is never written to a file or passed as an argument:
/// it goes to `sudo -S` on stdin, which installs a temporary sudo rule that is removed again at the end.
public struct MacImageBuilder: Sendable {
    public let paths: Paths
    public let vz: VZBackend

    public init(paths: Paths, vz: VZBackend) {
        self.paths = paths
        self.vz = vz
    }

    public struct Request: Sendable {
        public var recipe: String
        public var flake: String
        /// Guest user of the base (the recipe's `user`)
        public var user: String
        public var password: String
        /// nil = newest `bases/mac-base-*`
        public var base: URL?
        /// Keep a failed build folder for inspection instead of deleting it
        public var keepFailed: Bool

        public init(recipe: String, flake: String, user: String, password: String, base: URL? = nil, keepFailed: Bool = false) {
            self.recipe = recipe
            self.flake = flake
            self.user = user
            self.password = password
            self.base = base
            self.keepFailed = keepFailed
        }
    }

    /// Newest base (`bases/mac-base-<os-build>` with a config.json), nil if there is none.
    public func newestBase() -> URL? { Self.newestBase(paths) }

    public static func newestBase(_ paths: Paths) -> URL? {
        let names = (try? FileManager.default.contentsOfDirectory(atPath: paths.bases.path)) ?? []
        return names.filter { $0.hasPrefix("mac-base-") }.sorted().reversed()
            .map { paths.bases.appendingPathComponent($0, isDirectory: true) }
            .first { MacBaseInstaller.isReady($0) }
    }

    /// Runs the whole pipeline; progress goes to `log`, details (guest output) to `logs/build-<name>.log`.
    public func build(_ r: Request, log: @escaping @Sendable (String) -> Void) throws -> ImageStore.Image {
        try KeepAwake.during("Building a macOS image") { try buildAwake(r, log: log) }
    }

    private func buildAwake(_ r: Request, log: @escaping @Sendable (String) -> Void) throws -> ImageStore.Image {
        let images = ImageStore(paths: paths)
        guard let base = r.base ?? newestBase(), FileManager.default.fileExists(atPath: MacVMBundle(base).config.path) else {
            throw VMError("No base found (expected \(paths.bases.path)/mac-base-<os-build>) – create one with: omaf macos install")
        }
        try vz.checkEntitlement()
        if let baseUser = MacBaseInstaller.user(of: base, logs: paths.logs), baseUser != r.user {
            throw VMError("\(base.lastPathComponent) has the guest user \(baseUser), but the recipe expects \(r.user) "
                          + "(macUser in your recipes' local settings) – make them match")
        }
        let flakeRef = ImageStore.flakeRef(r.flake)

        // 1. System closure on the Mac – fails early, before a VM is started
        log("Building darwinConfigurations.\(r.recipe) on the Mac")
        let built = try Shell.stream("nix", ["build", "\(flakeRef)#darwinConfigurations.\"\(r.recipe)\".system",
                                             "--no-link", "--print-out-paths", "-L"] + ImageStore(paths: paths).guestArguments(flake: r.flake), onLine: log)
        if Cancellation.current?.isCancelled == true { throw VMError("Build cancelled") }
        guard built.status == 0,
              let system = built.stdout.split(separator: "\n").last.map(String.init), system.hasPrefix("/nix/store/") else {
            throw VMError("nix build of darwinConfigurations.\(r.recipe) failed (\(built.status)) – see log")
        }
        let name = ImageStore.imageName(recipe: r.recipe, storePath: system)
        let image = paths.imageDir(name)
        if FileManager.default.fileExists(atPath: image.path) {
            log("Image exists already: \(image.path)")
            try images.setCurrent(imageName: name)
            return try images.image(named: name)
        }

        // 2. Clone the base, start it headless
        let work = paths.images.appendingPathComponent(".build-\(name)", isDirectory: true)
        // One build per image at a time (app and CLI): the lock lives next to the folder and ends with this build
        let lock = try FileLock(paths.images.appendingPathComponent(".build-\(name).lock"),
                                busy: "A build of \(name) is already running (app or CLI) – wait for it or cancel it",
                                removeWhenDone: true)
        defer { withExtendedLifetime(lock) {} }
        if FileManager.default.fileExists(atPath: work.path) {
            // Not locked, so no build is using it: left over from a crash or kept with --keep-failed
            log("Deleting the leftover build folder \(work.lastPathComponent)")
            try Disposal.delete([work])
        }
        try FileManager.default.createDirectory(at: paths.logs, withIntermediateDirectories: true)
        let logFile = paths.logs.appendingPathComponent("build-\(name).log")
        log("Cloning \(base.lastPathComponent) (log: \(logFile.path))")
        try Shell.check(vz.runner, ["clone", base.path, work.path])
        let socket = work.appendingPathComponent("runner.sock"), pidFile = work.appendingPathComponent("runner.pid")
        let child = try spawnDetached(vz.runner, ["run", work.path, "--socket", socket.path, "--pid-file", pidFile.path], log: logFile)
        // Cancelling forces the build VM off at once (its runner is detached and would outlive omaf otherwise)
        Cancellation.current?.onCancel {
            if (try? RunnerClient.send("kill", socket: socket)) == nil { kill(child, SIGTERM) }
        }

        var vm = BuildVM(child: child, socket: socket, user: r.user, logFile: logFile)
        do {
            try provision(&vm, r: r, system: system, log: log)
        } catch {
            vm.forceOff()
            let cancelled = Cancellation.current?.isCancelled == true
            if r.keepFailed && !cancelled {
                log("Kept for inspection: \(work.path)")
            } else if FileManager.default.fileExists(atPath: work.path) {
                try? Disposal.delete([work])
                log(cancelled ? "Build cancelled – its folder was deleted"
                              : "Failed build deleted (log: \(logFile.path); --keep-failed keeps it for inspection)")
            }
            throw cancelled ? VMError("Build cancelled") : error
        }

        // 7. Store as image: without the runner's runtime files, read-only, current/<recipe>
        try? Disposal.delete([socket, pidFile])
        let recipeSource = RecipeLibrary(paths: paths).available(Settings.load(paths)).first { $0.id == r.recipe }?.source
        try ImageStore.writeInfo(ImageStore.Info(recipe: r.recipe, recipeSource: recipeSource, kind: .macos,
                                                 system: Platform.hostSystem(for: .macos) ?? "", source: r.flake,
                                                 revision: images.recipeRevision(r.recipe, flake: r.flake), storePath: system), to: work)
        try FileManager.default.moveItem(at: work, to: image)
        try Shell.check("/bin/chmod", ["-R", "a-w", image.path])
        try images.setCurrent(imageName: name)
        let library = RecipeLibrary(paths: paths)
        library.keep(r.recipe, settings: Settings.load(paths))
        library.pruneVersions()
        log("Image ready: \(image.path)")
        return try images.image(named: name)
    }

    /// Nix's macOS release tarball in <data>/cache, checked against the SHA-256 that nixos.org publishes next to it.
    func nixTarball(version: String, log: @escaping @Sendable (String) -> Void) throws -> URL {
        let name = "nix-\(version)-aarch64-darwin.tar.xz"
        let base = "https://releases.nixos.org/nix/nix-\(version)/\(name)"
        let cache = paths.root.appendingPathComponent("cache", isDirectory: true)
        let file = cache.appendingPathComponent(name)
        try FileManager.default.createDirectory(at: cache, withIntermediateDirectories: true)
        let expected = try Shell.check("/usr/bin/curl", ["-fsSL", base + ".sha256"]).trimmingCharacters(in: .whitespacesAndNewlines)
        func sum() -> String? {
            (try? Shell.check("/usr/bin/shasum", ["-a", "256", file.path]))?.split(separator: " ").first.map(String.init)
        }
        if sum() != expected {
            log("Downloading \(name)")
            try Shell.check("/usr/bin/curl", ["-fsSL", "-o", file.path, base])
            guard sum() == expected else {
                try? Disposal.delete([file])
                throw VMError("\(name): checksum does not match nixos.org's")
            }
        }
        return file
    }

    /// `nix copy` into the build VM with a watchdog: a copy of ~9 GiB over ssh-ng once stood still after 632 MB with
    /// both sides idle (plan B75). No new path for 5 minutes → stop and copy again; paths already in the guest are
    /// skipped, later attempts use the older ssh:// protocol.
    func copyClosure(_ system: String, stores: [String], env: [String: String], stall: TimeInterval = 300,
                     log: @escaping @Sendable (String) -> Void) throws {
        final class Watch: @unchecked Sendable {
            let lock = NSLock()
            var last = Date(), copied = 0, stalled = false, process: Process?
        }
        for (attempt, store) in stores.enumerated() {
            try Cancellation.current?.check()
            let w = Watch()
            let timer = DispatchSource.makeTimerSource(queue: .global())
            timer.schedule(deadline: .now() + 30, repeating: 30)
            timer.setEventHandler {
                w.lock.withLock {
                    guard !w.stalled, Date().timeIntervalSince(w.last) > stall else { return }
                    w.stalled = true
                    w.process?.terminate()
                }
            }
            timer.resume()
            defer { timer.cancel() }
            let r = try Shell.stream("nix", ["copy", "-v", "--no-check-sigs", "--to", store, system], environment: env,
                                     started: { p in w.lock.withLock { w.process = p } }) { line in
                let n: Int? = w.lock.withLock {
                    w.last = Date()
                    guard line.hasPrefix("copying path") else { return nil }
                    w.copied += 1
                    return w.copied
                }
                if let n, n % 100 == 0 { log("… \(n) paths copied") }
            }
            if r.status == 0 { return }
            try Cancellation.current?.check()
            guard w.lock.withLock({ w.stalled }) else {
                throw VMError("Copying into the build VM failed (\(r.status)):\n\(r.stderr.suffix(800))")
            }
            if attempt + 1 < stores.count {
                log("No progress for \(Int(stall / 60)) minutes – copying again (attempt \(attempt + 2) of \(stores.count))")
            }
        }
        throw VMError("Copying into the build VM stood still \(stores.count) times")
    }

    /// Steps 3–6 inside the running build VM.
    private func provision(_ vm: inout BuildVM, r: Request, system: String, log: @escaping @Sendable (String) -> Void) throws {
        log("Waiting for the VM")
        let mac = try MacVMBundle(vm.socket.deletingLastPathComponent()).loadConfig().macAddress
        let deadline = Date().addingTimeInterval(600)
        while vm.ip == nil {
            try Cancellation.current?.check()
            guard !vm.exited() else { throw VMError("omaf-runner exited:\n\(tail(vm.logFile, lines: 3))") }
            let ip = DHCPLeases.ip(for: mac)
            guard Date() < deadline else {
                // What was missing – the address, the open port or SSH's greeting (plan B61)
                let port = ip.map { Ports.isOpen(host: $0, port: 22) ? "port 22 open but no SSH greeting" : "port 22 closed" } ?? "no DHCP lease for \(mac)"
                throw VMError("No SSH after 10 minutes (\(ip.map { "address \($0), " } ?? "")\(port))")
            }
            if let ip, Ports.sshBanner(host: ip, port: 22) != nil { vm.ip = ip } else { sleep(5) }
        }
        log("VM at \(vm.ip!)")
        var loggedIn = false
        for _ in 0..<30 where !loggedIn {
            try Cancellation.current?.check()
            loggedIn = (try? vm.ssh("true").status) == 0
            if !loggedIn { sleep(2) }
        }
        guard loggedIn else {
            throw VMError("SSH login as \(r.user) with your key failed (is your public key in the base? docs/macos-base.md)")
        }

        // 3. Temporary sudo without a password (removed in step 6)
        let rule = "/etc/sudoers.d/zz-omaflake-build"
        let sudo = try vm.ssh("sudo -S -p '' sh -c 'echo \"\(r.user) ALL=(ALL) NOPASSWD: ALL\" > \(rule) && chmod 440 \(rule)'",
                              input: Data((r.password + "\n").utf8))
        guard sudo.status == 0 else { throw VMError("sudo in the guest failed (wrong password?)") }

        // 4. Nix (same version as on the Mac), the guest user trusted for nix copy
        if try vm.ssh("test -x /nix/var/nix/profiles/default/bin/nix").status != 0 {
            let version = try Shell.check("nix", ["--version"]).split(separator: " ").last
                .map { $0.trimmingCharacters(in: .whitespacesAndNewlines) } ?? ""
            log("Installing Nix \(version) in the guest (a few minutes)")
            // The Mac brings the release tarball (cached, checksum from nixos.org): a fresh macOS guest downloads its
            // aerial wallpapers and assets meanwhile, and Nix's own download crawled (plan B76).
            // No channel: images use flakes; fetching nixpkgs-unstable as a channel took minutes (plan B74)
            let tarball = try nixTarball(version: version, log: log)
            let name = "nix-\(version)-aarch64-darwin"
            let put = try vm.ssh("cat > /tmp/\(name).tar.xz", input: try Data(contentsOf: tarball))
            guard put.status == 0 else { throw VMError("Copying the Nix tarball into the guest failed") }
            try vm.check("cd /tmp && tar -xJf \(name).tar.xz && cd \(name) && ./install --daemon --yes --no-channel-add",
                         what: "Nix installation")
        }
        try vm.check("grep -q '^trusted-users' /etc/nix/nix.conf || echo 'trusted-users = root \(r.user)' | sudo tee -a /etc/nix/nix.conf >/dev/null; "
                     + "sudo launchctl kickstart -k system/org.nixos.nix-daemon", what: "nix-daemon setup")
        let store = "ssh-ng://\(r.user)@\(vm.ip!)?remote-program=/nix/var/nix/profiles/default/bin/nix-daemon"
        let env = ["NIX_SSHOPTS": BuildVM.sshOptions.joined(separator: " ")]
        // The restarted daemon needs a moment; paths built on the Mac are unsigned and need a trusted user
        var trusted = false
        for _ in 0..<30 where !trusted {
            try Cancellation.current?.check()
            let info = try? Shell.run("nix", ["store", "info", "--json", "--store", store], environment: env)
            trusted = info?.stdout.contains("\"trusted\":true") == true
            if !trusted { sleep(2) }
        }
        guard trusted else { throw VMError("The guest's nix-daemon does not trust \(r.user)") }

        // 5. Copy and activate
        log("Copying the closure")
        let legacy = "ssh://\(r.user)@\(vm.ip!)?remote-program=/nix/var/nix/profiles/default/bin/nix-store"
        try copyClosure(system, stores: [store, legacy, legacy], env: env, log: log)
        log("Activating")
        // nix-darwin takes over these files; the originals stay next to them (never delete)
        try vm.check("for f in /etc/nix/nix.conf /etc/bashrc /etc/zshrc; do [ -L \"$f\" ] || [ ! -e \"$f\" ] || sudo mv \"$f\" \"$f.before-nix-darwin\"; done",
                     what: "moving /etc files aside")
        try vm.check("sudo /nix/var/nix/profiles/default/bin/nix-env -p /nix/var/nix/profiles/system --set '\(system)' && sudo '\(system)/activate'",
                     what: "Activation")

        // 6. Remove the sudo rule, shut down
        try vm.check("sudo rm -f \(rule)", what: "removing the temporary sudo rule")
        _ = try? vm.ssh("sudo -n /sbin/shutdown -h now")
        log("Shutting down")
        let off = Date().addingTimeInterval(120)
        while !vm.exited() {
            try Cancellation.current?.check()
            guard Date() < off else { throw VMError("The build VM did not shut down") }
            sleep(2)
        }
    }

    /// The headless build VM: its runner process (our child) and SSH into it.
    struct BuildVM {
        static let sshOptions = ["-o", "BatchMode=yes", "-o", "ConnectTimeout=10", "-o", "UserKnownHostsFile=/dev/null",
                                 "-o", "StrictHostKeyChecking=no", "-o", "LogLevel=ERROR"]
        let child: pid_t
        let socket: URL
        let user: String
        let logFile: URL
        var ip: String?
        private var done = false

        init(child: pid_t, socket: URL, user: String, logFile: URL) {
            self.child = child
            self.socket = socket
            self.user = user
            self.logFile = logFile
        }

        /// True once the runner has exited (reaps it, so no zombie is left).
        mutating func exited() -> Bool {
            if !done {
                var st: Int32 = 0
                done = waitpid(child, &st, WNOHANG) == child
            }
            return done
        }

        mutating func forceOff() {
            guard !exited() else { return }
            if (try? RunnerClient.send("kill", socket: socket)) == nil { kill(child, SIGTERM) }
            let deadline = Date().addingTimeInterval(15)
            while !exited() && Date() < deadline { usleep(300_000) }
            if !exited() { kill(child, SIGKILL); _ = exited() }
        }

        /// One command in the guest (remote shell line); stdout/stderr are appended to the build log.
        func ssh(_ command: String, input: Data? = nil) throws -> Shell.Result {
            let r = try Shell.run("/usr/bin/ssh", Self.sshOptions + ["\(user)@\(ip ?? "")", command], input: input)
            appendLog(r.stdout + r.stderr)
            return r
        }

        func check(_ command: String, what: String) throws {
            guard try ssh(command).status == 0 else { throw VMError("\(what) failed – see \(logFile.path)") }
        }

        private func appendLog(_ text: String) {
            guard !text.isEmpty, let h = try? FileHandle(forWritingTo: logFile) else { return }
            defer { try? h.close() }
            _ = try? h.seekToEnd()
            h.write(Data(text.utf8))
        }
    }
}
