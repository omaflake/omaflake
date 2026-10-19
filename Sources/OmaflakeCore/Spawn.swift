// SPDX-License-Identifier: AGPL-3.0-or-later
import Darwin
import Foundation

// Detached child processes (QEMU, omaf-runner): own session, outlive the CLI/app (plan §3).

/// posix_spawn with POSIX_SPAWN_SETSID: own session, no SIGHUP when the terminal closes.
/// `environment` is added to this process's environment.
func spawnDetached(_ exe: String, _ args: [String], log: URL, environment: [String: String] = [:]) throws -> pid_t {
    var fa: posix_spawn_file_actions_t?
    posix_spawn_file_actions_init(&fa)
    defer { posix_spawn_file_actions_destroy(&fa) }
    posix_spawn_file_actions_addopen(&fa, 0, "/dev/null", O_RDONLY, 0)
    posix_spawn_file_actions_addopen(&fa, 1, log.path, O_WRONLY | O_CREAT | O_APPEND, 0o644)
    posix_spawn_file_actions_adddup2(&fa, 1, 2)

    var attr: posix_spawnattr_t?
    posix_spawnattr_init(&attr)
    defer { posix_spawnattr_destroy(&attr) }
    posix_spawnattr_setflags(&attr, Int16(POSIX_SPAWN_SETSID | POSIX_SPAWN_CLOEXEC_DEFAULT))

    let argv = ([exe] + args).map { strdup($0) } + [nil]
    defer { argv.forEach { free($0) } }
    var env = ProcessInfo.processInfo.environment
    env.merge(environment) { _, new in new }
    let envp = env.map { strdup("\($0.key)=\($0.value)") } + [nil]
    defer { envp.forEach { free($0) } }
    var pid: pid_t = 0
    let rc = posix_spawn(&pid, exe, &fa, &attr, argv, envp)
    guard rc == 0 else { throw VMError("Start of \((exe as NSString).lastPathComponent) failed: \(String(cString: strerror(rc)))") }
    return pid
}

/// True once the process is gone (polls every 0.2 s).
func waitForExit(_ pid: pid_t, timeout: TimeInterval) -> Bool {
    let deadline = Date().addingTimeInterval(timeout)
    while Date() < deadline {
        if kill(pid, 0) != 0 && errno == ESRCH { return true }
        usleep(200_000)
    }
    return false
}

/// Executable path of a running process (to rule out PID reuse), nil if it does not exist.
func executablePath(of pid: pid_t) -> String? {
    var buf = [CChar](repeating: 0, count: Int(MAXPATHLEN))
    guard proc_pidpath(pid, &buf, UInt32(buf.count)) > 0 else { return nil }
    return String(decoding: buf.prefix { $0 != 0 }.map { UInt8(bitPattern: $0) }, as: UTF8.self)
}

/// PID from a pid file if that process is alive and its executable path contains `executable`.
func livePid(file: URL, executable: String) -> pid_t? {
    guard let text = try? String(contentsOf: file, encoding: .utf8),
          let pid = pid_t(text.trimmingCharacters(in: .whitespacesAndNewlines)), pid > 0,
          kill(pid, 0) == 0 || errno == EPERM else { return nil }
    return executablePath(of: pid)?.contains(executable) == true ? pid : nil
}
