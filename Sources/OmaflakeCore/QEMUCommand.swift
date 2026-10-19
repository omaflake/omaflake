// SPDX-License-Identifier: AGPL-3.0-or-later
import Foundation

/// Builds the QEMU argument list from an instance (pure function, covered by unit tests).
/// Reference: scripts/bootprobe.sh (M1) and spec §6.3.
public struct QEMUCommand: Sendable {
    public var firmware: String
    public var paths: InstancePaths
    public var sharePath: String?
    public var display: DisplayMode
    public var gl: GLBackend
    public var capabilities: QEMUCapabilities
    /// Initial resolution of the virtual GPU in pixels (the window opens to match; afterwards the guest follows the window)
    public var initialResolution: (width: Int, height: Int)?

    /// UID/GID of the guest user (NixOS default for the first normal user and the "users" group)
    public static let guestUID = 1000
    public static let guestGID = 100

    public init(firmware: String, paths: InstancePaths, sharePath: String?, display: DisplayMode,
                gl: GLBackend = .on, capabilities: QEMUCapabilities = QEMUCapabilities(gl: false, guestOwner: false)) {
        self.firmware = firmware
        self.paths = paths
        self.sharePath = sharePath
        self.display = display
        self.gl = gl
        self.capabilities = capabilities
    }

    /// 75 % of the screen area in 16:9, in pixels, even width/height (Retina scale 2 stays integral).
    public static func initialResolution(screenPoints: (Double, Double), scale: Double) -> (width: Int, height: Int) {
        let unit = (min(screenPoints.0 * 0.75 / 16, screenPoints.1 * 0.75 / 9)).rounded(.down)
        let w = Int(16 * unit * scale) / 2 * 2
        let h = Int(9 * unit * scale) / 2 * 2
        return (w, h)
    }

    func gpuDevice(_ name: String) -> String {
        guard let r = initialResolution else { return name }
        return "\(name),xres=\(r.width),yres=\(r.height)"
    }

    public func arguments(for c: InstanceConfig) -> [String] {
        var a: [String] = [
            "-name", c.name,
            "-machine", "virt,highmem=on", "-accel", "hvf", "-cpu", "host",
            "-smp", "\(c.cpus)", "-m", "\(c.memoryGB)G",
            "-drive", "if=pflash,format=raw,readonly=on,file=\(firmware)",
            "-drive", "if=pflash,format=raw,file=\(paths.efivars.path)",
            // Fixed PCI addresses (Slot): a window adds GPU and USB devices, which used to shift the disk to another
            // slot – the UEFI boot entry stored in efivars then no longer matched and the VM fell into the UEFI
            // shell (plan B32). bootindex=0 also lets the firmware pick the disk over such a stale entry.
            "-drive", "if=none,id=hd0,format=qcow2,file=\(paths.disk.path)",
            "-device", "virtio-blk-pci,drive=hd0,bootindex=0,addr=\(Slot.disk)",
            "-device", "virtio-rng-pci,addr=\(Slot.rng)",
        ]
        // Ephemeral instance: writes go to a temporary file and vanish
        // on exit – nothing needs to be deleted (spec §9).
        if c.ephemeral { a += ["-snapshot"] }

        switch display {
        case .none:
            a += ["-device", "virtio-gpu-pci,addr=\(Slot.gpu)", "-display", "none"]
        case .window:
            // Cocoa default: Cmd → SUPER, Option → Alt. No swap-opt-cmd (swaps exactly that) and
            // no full-grab (needs the Accessibility permission for QEMU, plan B9/B21).
            // zoom-to-fit: window freely resizable; the window size is passed to the guest as resolution
            a += ["-device", gpuDevice("virtio-gpu-pci") + ",addr=\(Slot.gpu)", "-display", "cocoa,zoom-to-fit=on"] + Self.input
        case .gpu:
            a += ["-device", gpuDevice("virtio-gpu-gl-pci") + ",addr=\(Slot.gpu)", "-display", "cocoa,gl=\(gl.rawValue),zoom-to-fit=on"] + Self.input
        }

        if c.soundOn {
            // Intel HD Audio as try-omarchy uses it, on CoreAudio. Without the microphone the codec is output-only:
            // the guest sees no input and macOS never asks for microphone access.
            a += ["-audiodev", "coreaudio,id=snd0",
                  "-device", "intel-hda,addr=\(Slot.sound)",
                  "-device", "\(c.microphoneOn ? "hda-micro" : "hda-output"),audiodev=snd0"]
        }

        switch c.network {
        case .shared:
            a += ["-netdev", "user,id=n0,hostfwd=tcp:127.0.0.1:\(c.sshPort)-:22", "-device", "virtio-net-pci,netdev=n0,addr=\(Slot.net)"]
        case .isolated:
            a += ["-netdev", "user,id=n0,restrict=on,hostfwd=tcp:127.0.0.1:\(c.sshPort)-:22", "-device", "virtio-net-pci,netdev=n0,addr=\(Slot.net)"]
        case .none:
            a += ["-nic", "none"]
        }

        if c.share, let sharePath {
            // With patch: Mac files appear in the guest as owned by the guest user → writable on both sides (plan B18).
            // Without patch: mapped-xattr, Mac files are read-only in the guest.
            let fsdev = capabilities.guestOwner
                ? "local,id=fs0,path=\(sharePath),security_model=none,guest_owner_uid=\(Self.guestUID),guest_owner_gid=\(Self.guestGID)"
                : "local,id=fs0,path=\(sharePath),security_model=mapped-xattr"
            a += ["-fsdev", fsdev, "-device", "virtio-9p-pci,fsdev=fs0,mount_tag=share,addr=\(Slot.share)"]
        }

        a += [
            // Guest agent (plan B14)
            "-device", "virtio-serial-pci,addr=\(Slot.serial)",
            "-chardev", "socket,id=qga0,path=\(paths.qgaSocket.path),server=on,wait=off",
            "-device", "virtserialport,chardev=qga0,name=org.qemu.guest_agent.0",
            // omaflake agent (spec §11.9): always there – hello and status, read by the omaf-runner bridge
            "-chardev", "socket,id=agent0,path=\(paths.agentSocket.path),server=on,wait=off",
            "-device", "virtserialport,chardev=agent0,name=\(AgentProtocol.portName)",
        ]
        if c.clipboardOn {
            // Clipboard with the Mac: the guest agent ↔ omaf-runner bridge (spawned by QEMURuntime.start)
            a += ["-chardev", "socket,id=clip0,path=\(paths.clipSocket.path),server=on,wait=off",
                  "-device", "virtserialport,chardev=clip0,name=org.omaflake.clipboard"]
        }
        if c.cameraOn {
            // The Mac's camera: guest agent (v4l2loopback /dev/video42) ↔ omaf-runner bridge, frames only on demand
            a += ["-chardev", "socket,id=cam0,path=\(paths.cameraSocket.path),server=on,wait=off",
                  "-device", "virtserialport,chardev=cam0,name=org.omaflake.camera"]
        }
        a += [
            "-qmp", "unix:\(paths.qmpSocket.path),server=on,wait=off",
            // Append instead of overwrite: old logs are kept
            "-chardev", "file,id=serial0,path=\(paths.serialLog.path),append=on",
            "-serial", "chardev:serial0",
            "-pidfile", paths.pidFile.path,
        ]
        return a
    }

    static let input = ["-device", "qemu-xhci,addr=\(Slot.usb)", "-device", "usb-kbd", "-device", "usb-tablet"]

    /// PCI slots per device, the same with and without a window
    enum Slot {
        static let disk = "0x2", rng = "0x3", net = "0x4", gpu = "0x5", usb = "0x6", share = "0x7", serial = "0x8", sound = "0x9"
    }
}
