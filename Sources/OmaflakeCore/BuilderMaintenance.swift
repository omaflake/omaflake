// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Admin tasks around the Linux builder: apply the image flake's nix-darwin host (`#linux-builder`) and clean up
/// the builder's Nix store. Both need root – the app asks through macOS' administrator dialog (`AdminShell`), the CLI
/// through sudo. After Developer ID signing a privileged helper replaces the dialog (spec §11.5).
public struct BuilderMaintenance: Sendable {
    public let paths: Paths

    public init(paths: Paths) { self.paths = paths }

    /// Written by nix/hosts/linux-builder.nix: the running nix-darwin system comes from omaflake's flake
    public static let marker = "/etc/omaflake/linux-builder"

    /// Is the current nix-darwin system omaflake's `#linux-builder`? If not, applying replaces the user's own
    /// nix-darwin configuration – the app says so before it asks.
    public var systemIsOmaflakes: Bool { FileManager.default.fileExists(atPath: Self.marker) }

    public var log: URL { paths.logs.appendingPathComponent("linux-builder.log") }

    /// Step 1 of `darwin-rebuild switch`, as the user: builds the image flake's builder host and returns the system's
    /// store path. Root cannot do this – macOS' privacy protection keeps it out of ~/Documents etc. (plan B40), and it
    /// would write a root-owned flake.lock into the checkout.
    public func buildSystem(host: String = "linux-builder", onLine: @escaping @Sendable (String) -> Void = { _ in }) throws -> String {
        let settings = Settings.load(paths)
        let flake = try settings.flakePath()
        guard let nix = Prerequisites.locateNix(settings) else { throw VMError("nix not found") }
        let r = try Shell.stream(nix, ["build", "--no-link", "--print-out-paths", "--no-write-lock-file",
                                       "--extra-experimental-features", "nix-command flakes",
                                       "path:\(flake)#darwinConfigurations.\(host).system"], onLine: onLine)
        let out = r.stdout.trimmingCharacters(in: .whitespacesAndNewlines)
        guard r.status == 0, Self.isSystem(out) else {
            throw VMError("Building the Linux builder host failed:\n\(r.stderr.suffix(1500))")
        }
        return out
    }

    /// A nix-darwin system in the store (the only thing root gets to activate)
    static func isSystem(_ path: String) -> Bool {
        path.hasPrefix("/nix/store/") && !path.contains("\n") && !path.contains("'")
            && FileManager.default.isExecutableFile(atPath: path + "/activate")
    }

    /// Step 2 of `darwin-rebuild switch`, as root: set the system profile and activate (as darwin-rebuild does).
    public static func activateCommand(system: String) -> String {
        let s = shellQuote(system)
        return "nix-env -p /nix/var/nix/profiles/system --set \(s) && \(s)/activate"
    }

    // MARK: Setup by click (spec §11.5)

    /// Nix with the official installer of the NixOS community (github.com/NixOS/nix-installer): creates the /nix
    /// volume, the build users and the daemon; `--no-confirm` because the app has asked already. Uninstall:
    /// `/nix/nix-installer uninstall`. Pinned: one release, its macOS binary checked against the release's SHA-256
    /// before it runs (plan B66). `OMAFLAKE_NIX_INSTALLER_URL` points to a mirror of the same file.
    public static let nixInstallerVersion = "2.35.3"
    public static let nixInstallerSHA256 = "c4991562a71a7ef2dc52d9760545c8541db4a468bbcc91dcafd57175638bf17e"
    public static var nixInstallerURL: String {
        ProcessInfo.processInfo.environment["OMAFLAKE_NIX_INSTALLER_URL"]
            ?? "https://github.com/NixOS/nix-installer/releases/download/\(nixInstallerVersion)/nix-installer-aarch64-darwin"
    }
    public static var installNixCommand: String {
        "set -e; d=$(mktemp -d); curl -fsSL -o \"$d/nix-installer\" \(shellQuote(nixInstallerURL)); "
            + "echo \"\(nixInstallerSHA256)  $d/nix-installer\" | shasum -a 256 -c -; "
            + "chmod +x \"$d/nix-installer\"; \"$d/nix-installer\" install --no-confirm"
    }

    /// First activation of nix-darwin: it refuses to take over /etc files it did not write (the Nix installer's
    /// nix.conf and shell rc files) – move them aside as docs/linux-builder.md says (rename, never delete), then
    /// set the system profile and activate.
    public static func firstActivationCommand(system: String) -> String {
        let moveAside = ["/etc/nix/nix.conf", "/etc/bashrc", "/etc/zshrc", "/etc/zshenv"].map { f in
            "if [ -f \(f) ] && [ ! -L \(f) ]; then mv \(f) \(f).before-nix-darwin; fi"
        }.joined(separator: "; ")
        return moveAside + "; " + activateCommand(system: system)
    }

    /// Unused store paths in the builder (everything can be rebuilt or downloaded again), with free space before/after.
    /// `ssh linux-builder` works for root only (the key in /etc/nix is root's).
    public static let cleanUpCommand = "ssh linux-builder 'df -h / | tail -1; nix-store --gc 2>&1 | tail -1; df -h / | tail -1'"

    /// Wraps a command for a root shell: Nix tools on PATH, root's HOME, output appended to `log` (created by the
    /// user first, so it stays theirs).
    public func rootScript(_ command: String) throws -> String {
        try FileManager.default.createDirectory(at: paths.logs, withIntermediateDirectories: true)
        if !FileManager.default.fileExists(atPath: log.path) { FileManager.default.createFile(atPath: log.path, contents: nil) }
        let header = "=== \(ISO8601DateFormatter().string(from: Date())) \(command)"
        // pipefail: the command's failure, not tee's success, is the script's result
        return "set -o pipefail; export HOME=/var/root PATH=/run/current-system/sw/bin:/nix/var/nix/profiles/default/bin:/usr/bin:/bin:/usr/sbin:/sbin; "
            + "echo \(shellQuote(header)) >> \(shellQuote(log.path)); "
            + "{ \(command) ; } 2>&1 | tee -a \(shellQuote(log.path))"
    }
}

/// Runs a shell command as root after macOS' administrator dialog (AppleScript `do shell script … with administrator
/// privileges`). The command travels as an argument, never spliced into AppleScript source.
public enum AdminShell {
    public struct Cancelled: Error {}

    /// Blocks until the command ends; returns its output. Throws `Cancelled` when the user cancels the dialog.
    public static func run(_ command: String, prompt: String) throws -> String {
        let p = Process()
        p.executableURL = URL(fileURLWithPath: "/usr/bin/osascript")
        p.arguments = ["-e", "on run argv",
                       "-e", "do shell script (item 1 of argv) with prompt (item 2 of argv) with administrator privileges",
                       "-e", "end run", command, prompt]
        let out = Pipe(), err = Pipe()
        p.standardOutput = out
        p.standardError = err
        try p.run()
        let outData = out.fileHandleForReading.readDataToEndOfFile()
        let errData = err.fileHandleForReading.readDataToEndOfFile()
        p.waitUntilExit()
        let output = String(decoding: outData, as: UTF8.self).replacingOccurrences(of: "\r", with: "\n")
        guard p.terminationStatus == 0 else {
            let message = String(decoding: errData, as: UTF8.self)
            // -128: "User canceled."
            if message.contains("(-128)") { throw Cancelled() }
            throw VMError(message.trimmingCharacters(in: .whitespacesAndNewlines))
        }
        return output
    }
}

/// Single-quotes a word for /bin/sh.
public func shellQuote(_ s: String) -> String {
    "'" + s.replacingOccurrences(of: "'", with: "'\\''") + "'"
}
