// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation
import Virtualization
import OmaflakeCore

/// Builds the VZVirtualMachineConfiguration from a VM folder (spec §6.4).
@MainActor
enum MachineConfiguration {
    struct Options {
        var share: URL?
        var network = true
        /// Sound out to the Mac
        var audio = false
        /// The Mac's microphone into the guest (macOS asks for permission the first time)
        var microphone = false
        /// Clipboard: the host end of the virtio console port org.omaflake.clipboard (see RunController)
        var clipboard: FileHandle?
        /// Camera: the host end of the port org.omaflake.camera (the guest's camera extension: after M12, plan B49)
        var camera: FileHandle?
        /// The omaflake agent's port org.omaflake.agent (spec §11.9)
        var agent: FileHandle? = nil
    }

    /// Name of the clipboard port – the same as for NixOS guests (QEMU virtserialport)
    static let clipboardPort = "org.omaflake.clipboard"
    static let cameraPort = "org.omaflake.camera"

    static func make(_ bundle: MacVMBundle, _ options: Options = Options()) throws -> VZVirtualMachineConfiguration {
        let cfg = try bundle.loadConfig()

        let platform = VZMacPlatformConfiguration()
        guard let hwData = try? Data(contentsOf: bundle.hardwareModel),
              let hardware = VZMacHardwareModel(dataRepresentation: hwData) else {
            throw VMError("hardware-model missing or invalid: \(bundle.hardwareModel.path)")
        }
        guard hardware.isSupported else { throw VMError("Hardware model is not supported by this Mac") }
        guard let idData = try? Data(contentsOf: bundle.machineID),
              let machineID = VZMacMachineIdentifier(dataRepresentation: idData) else {
            throw VMError("machine-id missing or invalid: \(bundle.machineID.path)")
        }
        platform.hardwareModel = hardware
        platform.machineIdentifier = machineID
        platform.auxiliaryStorage = VZMacAuxiliaryStorage(url: bundle.auxStorage)

        let c = VZVirtualMachineConfiguration()
        c.platform = platform
        c.bootLoader = VZMacOSBootLoader()
        c.cpuCount = cfg.cpus
        c.memorySize = UInt64(cfg.memoryMB) * 1024 * 1024

        let graphics = VZMacGraphicsDeviceConfiguration()
        graphics.displays = [VZMacGraphicsDisplayConfiguration(
            widthInPixels: cfg.display.width, heightInPixels: cfg.display.height, pixelsPerInch: cfg.display.ppi)]
        c.graphicsDevices = [graphics]

        let disk = try VZDiskImageStorageDeviceAttachment(url: bundle.disk, readOnly: false,
                                                          cachingMode: .automatic, synchronizationMode: .full)
        c.storageDevices = [VZVirtioBlockDeviceConfiguration(attachment: disk)]

        if options.network {
            let net = VZVirtioNetworkDeviceConfiguration()
            net.attachment = VZNATNetworkDeviceAttachment()
            guard let mac = VZMACAddress(string: cfg.macAddress) else {
                throw VMError("Invalid MAC address in config.json: \(cfg.macAddress)")
            }
            net.macAddress = mac
            c.networkDevices = [net]
        }

        if let share = options.share {
            let fs = VZVirtioFileSystemDeviceConfiguration(tag: VZVirtioFileSystemDeviceConfiguration.macOSGuestAutomountTag)
            fs.share = VZSingleDirectoryShare(directory: VZSharedDirectory(url: share, readOnly: false))
            c.directorySharingDevices = [fs]
        }

        if options.audio {
            let sound = VZVirtioSoundDeviceConfiguration()
            let output = VZVirtioSoundDeviceOutputStreamConfiguration()
            output.sink = VZHostAudioOutputStreamSink()
            var streams: [VZVirtioSoundDeviceStreamConfiguration] = [output]
            if options.microphone {
                let input = VZVirtioSoundDeviceInputStreamConfiguration()
                input.source = VZHostAudioInputStreamSource()
                streams.append(input)
            }
            sound.streams = streams
            c.audioDevices = [sound]
        }

        // Guest integrations: one virtio console device, a named port each (/dev/cu.<name> in the guest)
        let ports = [(clipboardPort, options.clipboard), (cameraPort, options.camera),
                     (AgentProtocol.portName, options.agent)].compactMap { name, handle in
            handle.map { (name, $0) }
        }
        if !ports.isEmpty {
            let console = VZVirtioConsoleDeviceConfiguration()
            for (i, (name, handle)) in ports.enumerated() {
                let port = VZVirtioConsolePortConfiguration()
                port.name = name
                port.attachment = VZFileHandleSerialPortAttachment(fileHandleForReading: handle, fileHandleForWriting: handle)
                console.ports[i] = port
            }
            c.consoleDevices = [console]
        }

        c.keyboards = [VZMacKeyboardConfiguration()]
        c.pointingDevices = [VZMacTrackpadConfiguration(), VZUSBScreenCoordinatePointingDeviceConfiguration()]
        c.entropyDevices = [VZVirtioEntropyDeviceConfiguration()]
        c.memoryBalloonDevices = [VZVirtioTraditionalMemoryBalloonDeviceConfiguration()]

        try c.validate()
        return c
    }
}

extension VZVirtualMachine.State {
    var name: String {
        switch self {
        case .stopped: "stopped"
        case .running: "running"
        case .paused: "paused"
        case .error: "error"
        case .starting: "starting"
        case .pausing: "pausing"
        case .resuming: "resuming"
        case .stopping: "stopping"
        case .saving: "saving"
        case .restoring: "restoring"
        @unknown default: "unknown"
        }
    }
}

/// Error text including the VZ error code (e.g. 6 = VirtualMachineLimitExceeded, limit of 2 macOS VMs).
func describe(_ error: Error) -> String {
    let ns = error as NSError
    if ns.domain == VZErrorDomain { return "\(ns.localizedDescription) [VZErrorDomain \(ns.code)]" }
    return "\(error)"
}
