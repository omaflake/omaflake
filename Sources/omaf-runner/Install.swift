// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation
import Virtualization
import OmaflakeCore

/// Fresh macOS installation from an IPSW into a new VM folder (spec §6.5 step 1).
@MainActor
enum Installer {
    struct Request {
        var target: URL
        var ipsw: URL?
        var latest: Bool
        var cpus: Int
        var memoryGB: Int
        var diskGB: Int
        var ipswDir: URL
    }

    static func run(_ r: Request) async throws {
        let fm = FileManager.default
        if fm.fileExists(atPath: r.target.path) {
            throw VMError("Target folder already exists: \(r.target.path) (nothing is overwritten)")
        }

        let ipswURL: URL
        if let ipsw = r.ipsw {
            ipswURL = ipsw
        } else if r.latest {
            ipswURL = try await downloadLatest(into: r.ipswDir)
        } else {
            throw VMError("Specify --ipsw <file> or --latest")
        }

        log("Loading restore image \(ipswURL.lastPathComponent) …")
        let image = try await VZMacOSRestoreImage.image(from: ipswURL)
        guard let req = image.mostFeaturefulSupportedConfiguration, req.hardwareModel.isSupported else {
            throw VMError("This IPSW (\(image.buildVersion)) is not supported by this Mac")
        }
        let cpus = max(r.cpus, req.minimumSupportedCPUCount)
        let memoryMB = max(r.memoryGB * 1024, Int(req.minimumSupportedMemorySize / 1024 / 1024))
        log("macOS \(version(image.operatingSystemVersion)) (\(image.buildVersion)); at least \(req.minimumSupportedCPUCount) CPU, \(req.minimumSupportedMemorySize / 1_073_741_824) GiB")

        let bundle = MacVMBundle(r.target)
        try fm.createDirectory(at: r.target, withIntermediateDirectories: true)
        do {
            try req.hardwareModel.dataRepresentation.write(to: bundle.hardwareModel)
            try VZMacMachineIdentifier().dataRepresentation.write(to: bundle.machineID)
            _ = try VZMacAuxiliaryStorage(creatingStorageAt: bundle.auxStorage, hardwareModel: req.hardwareModel, options: [])
            try bundle.createDisk(gigabytes: r.diskGB)
            try bundle.save(MacVMConfig(cpus: cpus, memoryMB: memoryMB, diskGB: r.diskGB,
                                        macAddress: VZMACAddress.randomLocallyAdministered().string,
                                        osBuild: image.buildVersion))

            let vm = VZVirtualMachine(configuration: try MachineConfiguration.make(bundle))
            let installer = VZMacOSInstaller(virtualMachine: vm, restoringFromImageAt: ipswURL)
            log("Installing to \(r.target.path) …")
            let reporter = ProgressReporter(installer.progress, label: "Install")
            defer { reporter.stop() }
            try await installer.install()
            log("Installation finished. First start: omaf-runner run \(r.target.path) --window")
        } catch {
            log("Aborted – incomplete folder is left in place (not deleted): \(r.target.path)")
            throw error
        }
    }

    /// Download the latest IPSW supported by this Mac (stored as ~/.omaflake/ipsw/<build>.ipsw, existing ones are reused).
    static func downloadLatest(into dir: URL) async throws -> URL {
        log("Asking Apple for the latest supported IPSW …")
        let latest = try await VZMacOSRestoreImage.latestSupported
        let dest = dir.appendingPathComponent("\(latest.buildVersion).ipsw")
        if FileManager.default.fileExists(atPath: dest.path) {
            log("Already present: \(dest.path)")
            return dest
        }
        try FileManager.default.createDirectory(at: dir, withIntermediateDirectories: true)
        log("Downloading \(latest.url.absoluteString) → \(dest.path)")
        return try await withCheckedThrowingContinuation { cont in
            let task = URLSession.shared.downloadTask(with: latest.url) { tmp, _, error in
                if let error { cont.resume(throwing: error); return }
                guard let tmp else { cont.resume(throwing: VMError("Download produced no file")); return }
                do {
                    try FileManager.default.moveItem(at: tmp, to: dest)
                    cont.resume(returning: dest)
                } catch { cont.resume(throwing: error) }
            }
            let reporter = ProgressReporter(task.progress, label: "Download")
            task.resume()
            Task { @MainActor in
                while task.state == .running { try? await Task.sleep(for: .seconds(2)) }
                reporter.stop()
            }
        }
    }

    static func version(_ v: OperatingSystemVersion) -> String {
        "\(v.majorVersion).\(v.minorVersion)" + (v.patchVersion > 0 ? ".\(v.patchVersion)" : "")
    }
}

/// Prints progress in whole percent (only on change – readable in a log too).
/// DispatchSource instead of Timer: `install` runs with dispatchMain(), without a RunLoop.
@MainActor
final class ProgressReporter {
    private let timer: DispatchSourceTimer
    private var last = -1

    init(_ progress: Progress, label: String) {
        timer = DispatchSource.makeTimerSource(queue: .main)
        timer.schedule(deadline: .now() + 2, repeating: 2)
        timer.setEventHandler { [weak self] in
            MainActor.assumeIsolated {
                guard let self else { return }
                let pct = Int(progress.fractionCompleted * 100)
                if pct != self.last {
                    self.last = pct
                    log("\(label): \(pct) %")
                }
            }
        }
        timer.resume()
    }

    func stop() { timer.cancel() }
}

func log(_ s: String) {
    let ts = ISO8601DateFormatter.string(from: Date(), timeZone: .current, formatOptions: [.withTime, .withColonSeparatorInTime])
    print("[\(ts)] \(s)")
    fflush(stdout)
}
