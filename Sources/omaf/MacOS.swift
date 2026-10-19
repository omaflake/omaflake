// SPDX-License-Identifier: AGPL-3.0-or-later
import ArgumentParser
import Foundation
import OmaflakeCore

// omaf macos – macOS bases (spec §6.5 step 1, docs/macos-base.md)

struct MacOS: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "macos", abstract: "macOS bases: install from Apple's IPSW.", subcommands: [MacInstall.self])
}

struct MacInstall: ParsableCommand {
    static let configuration = CommandConfiguration(
        commandName: "install",
        abstract: "Download macOS from Apple and build a ready base ~/.omaflake/bases/mac-base-<build> – no manual steps.",
        discussion: """
            Steps: omaf-runner install (download of the latest supported IPSW unless --ipsw) → mac-fresh-<build> \
            (never started, read-only) → APFS clone mac-base-<build> with a new identity → first boot (headless) with \
            provisioning: user, automatic login and SSH, without the Setup Assistant → your SSH public keys \
            (~/.ssh/*.pub) into the guest → shut down → read-only. Existing folders are reused, never overwritten.
            """)
    @Option(help: "Local IPSW file (default: the latest supported by this Mac, from Apple)") var ipsw: String?
    @Option(help: "CPU cores") var cpus = 4
    @Option(help: "RAM in GiB") var memory = 8
    @Option(help: "Disk size in GiB (thin-provisioned)") var disk = 80
    @Option(help: "Guest user to create (default: your Mac user name)") var user: String?
    @Option(help: "Full name of the guest user (default: yours)") var fullName: String?
    @Flag(help: "Read the guest user's password from stdin instead of prompting") var passwordStdin = false
    @Flag(help: "Stop after the clone, without the first boot") var noFirstBoot = false
    @Flag(help: "Show the first boot in a window") var window = false

    func run() throws {
        let env = Env()
        let installer = MacBaseInstaller(paths: env.paths, vz: try env.backends().vz)
        let user = user ?? NSUserName()
        var request = MacBaseInstaller.Request(
            ipsw: ipsw.map { URL(fileURLWithPath: ($0 as NSString).expandingTildeInPath).standardizedFileURL },
            cpus: cpus, memoryGB: memory, diskGB: disk, user: user, fullName: fullName ?? NSFullUserName(), password: nil,
            window: window)
        if let base = installer.existingBase(request) {
            print("\(base.lastPathComponent) exists already – nothing to do (delete \(base.path) to start over).")
            return
        }
        // Asked up front, so the long download/installation runs unattended
        if !noFirstBoot {
            request.password = try readPassword("Password for the new guest user \(user): ", stdin: passwordStdin, confirm: true)
        }
        let outcome = try installer.install(request) { line in print(line); fflush(stdout) }
        switch outcome {
        case .existing(let base):
            print("Delete \(base.path) to start over.")
        case .created(let base, let ready):
            print(ready ? """
                Base \(base.lastPathComponent) is ready. Set macUser in nix/local/settings.nix to \(user) if it differs
                from your NixOS guest user, then build an image: omaf build macos/minimal
                """ : "Base \(base.lastPathComponent) cloned without the first boot (omaf-runner run … --user … to provision it).")
        }
    }
}
