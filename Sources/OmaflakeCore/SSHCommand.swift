// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// ssh invocation for an instance – shared by the CLI (`omaf ssh`) and the app ("Open Terminal").
public enum SSHCommand {
    /// Arguments including `/usr/bin/ssh` in first position. Default target: NixOS port forwarding on 127.0.0.1;
    /// macOS guests pass their NAT address and port 22.
    public static func arguments(_ c: InstanceConfig, paths: Paths, host: String = "127.0.0.1", port: Int? = nil,
                                 command: [String] = []) -> [String] {
        // Host keys are new per instance → own known_hosts, accept on first connect.
        // Ephemeral instances generate new keys on every start → remember nothing (127.0.0.1 only).
        let hostKeys = c.ephemeral
            ? ["-o", "UserKnownHostsFile=/dev/null", "-o", "StrictHostKeyChecking=no", "-o", "LogLevel=ERROR"]
            : ["-o", "UserKnownHostsFile=\(paths.instance(c.name).knownHosts.path)", "-o", "StrictHostKeyChecking=accept-new"]
        return ["/usr/bin/ssh", "-p", "\(port ?? c.sshPort)"] + hostKeys + ["\(c.guestUser)@\(host)"] + command
    }

    /// As a single shell line (for Terminal.app), arguments single-quoted.
    public static func shellLine(_ arguments: [String]) -> String {
        arguments.map { "'" + $0.replacingOccurrences(of: "'", with: "'\\''") + "'" }.joined(separator: " ")
    }
}
