// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of QEMUCommand.swift.
//!
//! Builds the QEMU argument list from an instance (pure function, covered by unit tests).
//! Reference: scripts/bootprobe.sh (M1) and spec §6.3.
//!
//! The host decides machine, accelerator, window and sound (`Target`, plan B124): a Mac runs aarch64 guests with HVF
//! in Cocoa windows on CoreAudio (`Target::MAC`, the arguments the Swift code wrote); a Linux host runs guests of its
//! own architecture with KVM – TCG when /dev/kvm cannot be used – in GTK windows on PipeWire or PulseAudio. Everything
//! else (disk, network, share, agent ports, QMP, serial log) is the same on every host.
use crate::agent_protocol::AgentProtocol;
use crate::models::{DisplayMode, GlBackend, InstanceConfig, NetworkMode, QemuCapabilities};
use crate::paths::InstancePaths;

/// The guest's architecture (always the host's: guests run natively, plan B114)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Arch {
    Aarch64,
    X86_64,
}

impl Arch {
    /// "aarch64-linux" → Aarch64, "x86_64-linux" → X86_64; None for other systems
    pub fn of_system(system: &str) -> Option<Arch> {
        match crate::platform::Platform::architecture(system) {
            "aarch64" => Some(Arch::Aarch64),
            "x86_64" => Some(Arch::X86_64),
            _ => None,
        }
    }

    /// QEMU's and Nix's name ("aarch64", "x86_64")
    pub fn as_str(self) -> &'static str {
        match self {
            Arch::Aarch64 => "aarch64",
            Arch::X86_64 => "x86_64",
        }
    }

    /// The QEMU binary for guests of this architecture
    pub fn qemu_binary(self) -> String {
        format!("qemu-system-{}", self.as_str())
    }
}

/// How QEMU runs the guest's CPU: Apple's Hypervisor.framework, Linux' KVM, or TCG – QEMU's own emulator, many times
/// slower, the fallback on a Linux host without a usable /dev/kvm
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Accel {
    Hvf,
    Kvm,
    Tcg,
}

impl Accel {
    /// The value for `-accel`
    pub fn as_str(self) -> &'static str {
        match self {
            Accel::Hvf => "hvf",
            Accel::Kvm => "kvm",
            Accel::Tcg => "tcg",
        }
    }

    pub fn parse(text: &str) -> Option<Accel> {
        match text.trim() {
            "hvf" => Some(Accel::Hvf),
            "kvm" => Some(Accel::Kvm),
            "tcg" => Some(Accel::Tcg),
            _ => None,
        }
    }

    /// The hardware runs the guest's CPU (not emulated)
    pub fn is_hardware(self) -> bool {
        self != Accel::Tcg
    }
}

/// QEMU's window: Cocoa on a Mac, GTK on Linux
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ui {
    Cocoa,
    Gtk,
}

/// What the host gives QEMU (plan B124): the guest's architecture, the accelerator, the window toolkit and the sound
/// backend (`-audiodev` driver; None = no sound device, e.g. a Linux host without a sound server)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Target {
    pub arch: Arch,
    pub accel: Accel,
    pub ui: Ui,
    pub audio: Option<&'static str>,
}

impl Target {
    /// A Mac with Apple silicon: aarch64 with HVF, Cocoa, CoreAudio
    pub const MAC: Target = Target { arch: Arch::Aarch64, accel: Accel::Hvf, ui: Ui::Cocoa, audio: Some("coreaudio") };

    /// A Linux host running guests of `arch` with `accel`, GTK windows, sound through `audio` (`pipewire`, `pa`)
    pub fn linux(arch: Arch, accel: Accel, audio: Option<&'static str>) -> Target {
        Target { arch, accel, ui: Ui::Gtk, audio }
    }

    /// `-machine`: `virt` for aarch64 (KVM takes the host's interrupt controller, TCG the newest it emulates – GICv2,
    /// the default, allows only 8 CPUs), `q35` for x86_64 (PCIe like `virt`)
    pub fn machine(&self) -> &'static str {
        match (self.arch, self.accel) {
            (Arch::Aarch64, Accel::Hvf) => "virt,highmem=on",
            (Arch::Aarch64, Accel::Kvm) => "virt,highmem=on,gic-version=host",
            (Arch::Aarch64, Accel::Tcg) => "virt,highmem=on,gic-version=max",
            (Arch::X86_64, _) => "q35",
        }
    }

    /// `-cpu`: the host's CPU with HVF/KVM, everything TCG can emulate otherwise
    pub fn cpu(&self) -> &'static str {
        if self.accel.is_hardware() { "host" } else { "max" }
    }

    /// The GPU device: virtio-gpu on aarch64; on x86_64 virtio-vga, which also has the VGA modes the firmware and
    /// early boot use (`-vga none` keeps QEMU's default VGA card out)
    fn gpu(&self, gl: bool) -> &'static str {
        match (self.arch, gl) {
            (Arch::X86_64, false) => "virtio-vga",
            (Arch::X86_64, true) => "virtio-vga-gl",
            (Arch::Aarch64, false) => "virtio-gpu-pci",
            (Arch::Aarch64, true) => "virtio-gpu-gl-pci",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QemuCommand {
    /// What the host gives QEMU (default `Target::MAC`)
    pub target: Target,
    pub firmware: String,
    pub paths: InstancePaths,
    pub share_path: Option<String>,
    pub display: DisplayMode,
    pub gl: GlBackend,
    pub capabilities: QemuCapabilities,
    /// Initial resolution of the virtual GPU in pixels (the window opens to match; afterwards the guest follows the
    /// window)
    pub initial_resolution: Option<(u32, u32)>,
}

/// PCI slots per device, the same with and without a window (Swift `QEMUCommand.Slot`)
pub mod slot {
    pub const DISK: &str = "0x2";
    pub const RNG: &str = "0x3";
    pub const NET: &str = "0x4";
    pub const GPU: &str = "0x5";
    pub const USB: &str = "0x6";
    pub const SHARE: &str = "0x7";
    pub const SERIAL: &str = "0x8";
    pub const SOUND: &str = "0x9";
}

impl QemuCommand {
    /// UID/GID of the guest user (NixOS default for the first normal user and the "users" group)
    pub const GUEST_UID: u32 = 1000;
    pub const GUEST_GID: u32 = 100;

    /// USB keyboard and tablet on an xHCI controller (windows only)
    pub const INPUT: [&'static str; 6] =
        ["-device", "qemu-xhci,addr=0x6", "-device", "usb-kbd", "-device", "usb-tablet"];

    /// Swift defaults: `gl` .on, capabilities without GL and guest owner, no initial resolution
    pub fn new(firmware: &str, paths: InstancePaths, share_path: Option<String>, display: DisplayMode) -> Self {
        QemuCommand {
            target: Target::MAC,
            firmware: firmware.into(),
            paths,
            share_path,
            display,
            gl: GlBackend::On,
            capabilities: QemuCapabilities::default(),
            initial_resolution: None,
        }
    }

    /// 75 % of the screen area in 16:9, in pixels, even width/height (Retina scale 2 stays integral).
    /// Swift `initialResolution(screenPoints:scale:)`.
    pub fn initial_resolution_for(screen_points: (f64, f64), scale: f64) -> (u32, u32) {
        let unit = (screen_points.0 * 0.75 / 16.0).min(screen_points.1 * 0.75 / 9.0).floor();
        let w = (16.0 * unit * scale) as u32 / 2 * 2;
        let h = (9.0 * unit * scale) as u32 / 2 * 2;
        (w, h)
    }

    /// "<name>,xres=<w>,yres=<h>" with an initial resolution, else the name
    pub(crate) fn gpu_device(&self, name: &str) -> String {
        match self.initial_resolution {
            Some((w, h)) => format!("{name},xres={w},yres={h}"),
            None => name.to_string(),
        }
    }

    /// The whole argument list (without the binary) – see QEMUCommand.swift for the order, which the tests check
    pub fn arguments(&self, c: &InstanceConfig) -> Vec<String> {
        let p = &self.paths;
        let mut a: Vec<String> = vec![
            "-name".into(),
            c.name.clone(),
            "-machine".into(),
            self.target.machine().into(),
            "-accel".into(),
            self.target.accel.as_str().into(),
            "-cpu".into(),
            self.target.cpu().into(),
            "-smp".into(),
            c.cpus.to_string(),
            "-m".into(),
            format!("{}G", c.memory_gb),
            "-drive".into(),
            format!("if=pflash,format=raw,readonly=on,file={}", self.firmware),
            "-drive".into(),
            format!("if=pflash,format=raw,file={}", p.efivars().display()),
            // No boot wait in the firmware (plan B155): EDK2 counts down "Start boot option" for its Timeout variable
            // (5 s) unless QEMU passes a splash time – then that, here 0 (QemuBootOrderLib, GetFrontPageTimeoutFromQemu)
            "-boot".into(),
            "menu=on,splash-time=0".into(),
            // Fixed PCI addresses (slot): a window adds GPU and USB devices, which used to shift the disk to another
            // slot – the UEFI boot entry stored in efivars then no longer matched and the VM fell into the UEFI
            // shell (plan B32). bootindex=0 also lets the firmware pick the disk over such a stale entry.
            "-drive".into(),
            format!("if=none,id=hd0,format=qcow2,file={}", p.disk().display()),
            "-device".into(),
            format!("virtio-blk-pci,drive=hd0,bootindex=0,addr={}", slot::DISK),
            "-device".into(),
            format!("virtio-rng-pci,addr={}", slot::RNG),
        ];
        // Ephemeral instance: writes go to a temporary file and vanish
        // on exit – nothing needs to be deleted (spec §9).
        if c.ephemeral {
            a.push("-snapshot".into());
        }

        let input = Self::INPUT.iter().map(|s| s.to_string());
        if self.target.arch == Arch::X86_64 {
            // q35 adds a VGA card of its own otherwise (at the first free slot)
            a.extend(["-vga".into(), "none".into()]);
        }
        let t = &self.target;
        match self.display {
            DisplayMode::None => {
                a.extend(["-device".into(), format!("virtio-gpu-pci,addr={}", slot::GPU)]);
                a.extend(["-display".into(), "none".into()]);
            }
            DisplayMode::Window => {
                // Cocoa default: Cmd → SUPER, Option → Alt. No swap-opt-cmd (swaps exactly that) and
                // no full-grab (needs the Accessibility permission for QEMU, plan B9/B21).
                // zoom-to-fit: window freely resizable; the window size is passed to the guest as resolution
                a.extend(["-device".into(), format!("{},addr={}", self.gpu_device(t.gpu(false)), slot::GPU)]);
                let ui = match t.ui {
                    Ui::Cocoa => "cocoa,zoom-to-fit=on",
                    Ui::Gtk => "gtk,zoom-to-fit=on",
                };
                a.extend(["-display".into(), ui.into()]);
                a.extend(input);
            }
            DisplayMode::Gpu => {
                a.extend(["-device".into(), format!("{},addr={}", self.gpu_device(t.gpu(true)), slot::GPU)]);
                // GTK knows gl=on|off only (es is Cocoa's ANGLE path)
                let ui = match t.ui {
                    Ui::Cocoa => format!("cocoa,gl={},zoom-to-fit=on", self.gl.as_str()),
                    Ui::Gtk => "gtk,gl=on,zoom-to-fit=on".into(),
                };
                a.extend(["-display".into(), ui]);
                a.extend(input);
            }
        }

        if let (true, Some(driver)) = (c.sound_on(), t.audio) {
            // Intel HD Audio as try-omarchy uses it, on CoreAudio (Mac) or PipeWire/PulseAudio (Linux). Without the
            // microphone the codec is output-only: the guest sees no input and macOS never asks for microphone access.
            a.extend([
                "-audiodev".into(),
                format!("{driver},id=snd0"),
                "-device".into(),
                format!("intel-hda,addr={}", slot::SOUND),
                "-device".into(),
                format!("{},audiodev=snd0", if c.microphone_on() { "hda-micro" } else { "hda-output" }),
            ]);
        }

        let net_device = format!("virtio-net-pci,netdev=n0,addr={}", slot::NET);
        match c.network {
            NetworkMode::Shared => a.extend([
                "-netdev".into(),
                format!("user,id=n0,hostfwd=tcp:127.0.0.1:{}-:22", c.ssh_port),
                "-device".into(),
                net_device,
            ]),
            NetworkMode::Isolated => a.extend([
                "-netdev".into(),
                format!("user,id=n0,restrict=on,hostfwd=tcp:127.0.0.1:{}-:22", c.ssh_port),
                "-device".into(),
                net_device,
            ]),
            NetworkMode::None => a.extend(["-nic".into(), "none".into()]),
        }

        if let (true, Some(share)) = (c.share, &self.share_path) {
            // With patch: Mac files appear in the guest as owned by the guest user → writable on both sides (plan B18).
            // Without patch: mapped-xattr, Mac files are read-only in the guest.
            let fsdev = if self.capabilities.guest_owner {
                format!(
                    "local,id=fs0,path={share},security_model=none,guest_owner_uid={},guest_owner_gid={}",
                    Self::GUEST_UID,
                    Self::GUEST_GID
                )
            } else {
                format!("local,id=fs0,path={share},security_model=mapped-xattr")
            };
            a.extend([
                "-fsdev".into(),
                fsdev,
                "-device".into(),
                format!("virtio-9p-pci,fsdev=fs0,mount_tag=share,addr={}", slot::SHARE),
            ]);
        }

        a.extend([
            // Guest agent (plan B14)
            "-device".into(),
            format!("virtio-serial-pci,addr={}", slot::SERIAL),
            "-chardev".into(),
            format!("socket,id=qga0,path={},server=on,wait=off", p.qga_socket().display()),
            "-device".into(),
            "virtserialport,chardev=qga0,name=org.qemu.guest_agent.0".into(),
            // omaflake agent (spec §11.9): always there – hello and status, read by the omaf-runner bridge
            "-chardev".into(),
            format!("socket,id=agent0,path={},server=on,wait=off", p.agent_socket().display()),
            "-device".into(),
            format!("virtserialport,chardev=agent0,name={}", AgentProtocol::PORT_NAME),
        ]);
        if c.clipboard_on() {
            // Clipboard with the Mac: the guest agent ↔ omaf-runner bridge (spawned by QemuRuntime::start)
            a.extend([
                "-chardev".into(),
                format!("socket,id=clip0,path={},server=on,wait=off", p.clip_socket().display()),
                "-device".into(),
                "virtserialport,chardev=clip0,name=org.omaflake.clipboard".into(),
            ]);
        }
        if c.camera_on() {
            // The Mac's camera: guest agent (v4l2loopback /dev/video42) ↔ omaf-runner bridge, frames only on demand
            a.extend([
                "-chardev".into(),
                format!("socket,id=cam0,path={},server=on,wait=off", p.camera_socket().display()),
                "-device".into(),
                "virtserialport,chardev=cam0,name=org.omaflake.camera".into(),
            ]);
        }
        a.extend([
            "-qmp".into(),
            format!("unix:{},server=on,wait=off", p.qmp_socket().display()),
            // Append instead of overwrite: old logs are kept
            "-chardev".into(),
            format!("file,id=serial0,path={},append=on", p.serial_log().display()),
            "-serial".into(),
            "chardev:serial0".into(),
            "-pidfile".into(),
            p.pid_file().display().to_string(),
        ]);
        a
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::tests::config as base_config;
    use std::collections::HashSet;
    use std::path::PathBuf;

    fn ip() -> InstancePaths {
        InstancePaths::new(PathBuf::from("/vm/instances/t1"), None)
    }

    /// Swift's `config(network:ephemeral:share:)`
    fn config(network: NetworkMode, ephemeral: bool, share: bool) -> InstanceConfig {
        let mut c = base_config("t1");
        c.network = network;
        c.ephemeral = ephemeral;
        c.share = share;
        c
    }

    fn default_config() -> InstanceConfig {
        config(NetworkMode::Shared, false, true)
    }

    fn args_with(c: &InstanceConfig, display: DisplayMode, share: Option<&str>) -> Vec<String> {
        QemuCommand::new("/fw.fd", ip(), share.map(String::from), display).arguments(c)
    }

    fn args(c: &InstanceConfig) -> Vec<String> {
        args_with(c, DisplayMode::None, Some("/vm/share/t1"))
    }

    /// Values after an option, e.g. values(a, "-netdev")
    fn values(a: &[String], opt: &str) -> Vec<String> {
        a.windows(2).filter(|w| w[0] == opt).map(|w| w[1].clone()).collect()
    }

    fn has(a: &[String], opt: &str, value: &str) -> bool {
        values(a, opt).iter().any(|v| v == value)
    }

    #[test]
    fn basics() {
        let a = args(&default_config());
        assert_eq!(values(&a, "-accel"), ["hvf"]);
        assert_eq!(values(&a, "-smp"), ["8"]);
        assert_eq!(values(&a, "-m"), ["16G"]);
        assert_eq!(
            values(&a, "-drive"),
            [
                "if=pflash,format=raw,readonly=on,file=/fw.fd",
                "if=pflash,format=raw,file=/vm/instances/t1/efivars.fd",
                "if=none,id=hd0,format=qcow2,file=/vm/instances/t1/disk.qcow2",
            ]
        );
        assert!(has(&a, "-device", "virtio-blk-pci,drive=hd0,bootindex=0,addr=0x2"));
        assert_eq!(values(&a, "-qmp"), ["unix:/vm/instances/t1/qmp.sock,server=on,wait=off"]);
        assert_eq!(values(&a, "-pidfile"), ["/vm/instances/t1/qemu.pid"]);
        assert!(!a.contains(&"-snapshot".to_string()));
        assert_eq!(values(&a, "-display"), ["none"]);
        assert_eq!(&a[..2], ["-name", "t1"]);
    }

    #[test]
    fn network_modes() {
        assert_eq!(
            values(&args(&config(NetworkMode::Shared, false, true)), "-netdev"),
            ["user,id=n0,hostfwd=tcp:127.0.0.1:2230-:22"]
        );
        assert_eq!(
            values(&args(&config(NetworkMode::Isolated, false, true)), "-netdev"),
            ["user,id=n0,restrict=on,hostfwd=tcp:127.0.0.1:2230-:22"]
        );
        let none = args(&config(NetworkMode::None, false, true));
        assert!(values(&none, "-netdev").is_empty());
        assert_eq!(values(&none, "-nic"), ["none"]);
    }

    #[test]
    fn ephemeral_uses_snapshot() {
        assert!(args(&config(NetworkMode::Shared, true, true)).contains(&"-snapshot".to_string()));
    }

    #[test]
    fn share_only_when_enabled() {
        assert_eq!(
            values(&args(&config(NetworkMode::Shared, false, true)), "-fsdev"),
            ["local,id=fs0,path=/vm/share/t1,security_model=mapped-xattr"]
        );
        assert!(values(&args(&config(NetworkMode::Shared, false, false)), "-fsdev").is_empty());
        assert!(values(&args_with(&default_config(), DisplayMode::None, None), "-fsdev").is_empty());
    }

    #[test]
    fn gpu_mode() {
        let mut cmd = QemuCommand::new("/fw.fd", ip(), None, DisplayMode::Gpu);
        cmd.gl = GlBackend::Es;
        cmd.capabilities = QemuCapabilities { gl: true, guest_owner: false };
        let a = cmd.arguments(&default_config());
        assert_eq!(values(&a, "-display"), ["cocoa,gl=es,zoom-to-fit=on"]);
        assert!(has(&a, "-device", "virtio-gpu-gl-pci,addr=0x5"));
        assert!(!values(&a, "-device").iter().any(|v| v.starts_with("virtio-gpu-pci")));
    }

    #[test]
    fn share_with_guest_owner_patch() {
        let mut cmd = QemuCommand::new("/fw.fd", ip(), Some("/vm/share/t1".into()), DisplayMode::None);
        cmd.capabilities = QemuCapabilities { gl: true, guest_owner: true };
        assert_eq!(
            values(&cmd.arguments(&default_config()), "-fsdev"),
            ["local,id=fs0,path=/vm/share/t1,security_model=none,guest_owner_uid=1000,guest_owner_gid=100"]
        );
    }

    #[test]
    fn initial_resolution() {
        // MacBook Pro 14": 1512×982 points, Retina 2 → 2240×1260 (16:9, whole points)
        assert_eq!(QemuCommand::initial_resolution_for((1512.0, 982.0), 2.0), (2240, 1260));
        // 5K display with 2560×1440 points → 3840×2160
        let k = QemuCommand::initial_resolution_for((2560.0, 1440.0), 2.0);
        assert_eq!(k, (3840, 2160));
        let mut cmd = QemuCommand::new("/fw.fd", ip(), None, DisplayMode::Gpu);
        cmd.capabilities = QemuCapabilities { gl: true, guest_owner: false };
        cmd.initial_resolution = Some(k);
        assert!(has(&cmd.arguments(&default_config()), "-device", "virtio-gpu-gl-pci,xres=3840,yres=2160,addr=0x5"));
    }

    #[test]
    fn window_mode() {
        let a = args_with(&default_config(), DisplayMode::Window, Some("/vm/share/t1"));
        assert_eq!(values(&a, "-display"), ["cocoa,zoom-to-fit=on"]);
        assert!(has(&a, "-device", "usb-tablet"));
        assert!(has(&a, "-device", "virtio-gpu-pci,addr=0x5"));
    }

    #[test]
    fn clipboard_port() {
        let mut c = default_config();
        assert!(has(&args(&c), "-device", "virtserialport,chardev=clip0,name=org.omaflake.clipboard"));
        assert!(has(&args(&c), "-chardev", "socket,id=clip0,path=/vm/instances/t1/clip.sock,server=on,wait=off"));
        c.clipboard = Some(false);
        assert!(!values(&args(&c), "-device").iter().any(|v| v.contains("org.omaflake.clipboard")));
    }

    #[test]
    fn agent_port_always() {
        let mut c = config(NetworkMode::None, false, true);
        c.clipboard = Some(false);
        assert!(has(&args(&c), "-device", "virtserialport,chardev=agent0,name=org.omaflake.agent"));
        assert!(has(&args(&c), "-chardev", "socket,id=agent0,path=/vm/instances/t1/agent.sock,server=on,wait=off"));
    }

    #[test]
    fn camera_port_only_when_on() {
        let mut c = default_config();
        assert!(!values(&args(&c), "-device").iter().any(|v| v.contains("org.omaflake.camera"))); // default off
        c.camera = Some(true);
        assert!(has(&args(&c), "-device", "virtserialport,chardev=cam0,name=org.omaflake.camera"));
        assert!(has(&args(&c), "-chardev", "socket,id=cam0,path=/vm/instances/t1/cam.sock,server=on,wait=off"));
    }

    #[test]
    fn sound_and_microphone() {
        let mut c = default_config();
        assert_eq!(values(&args(&c), "-audiodev"), ["coreaudio,id=snd0"]);
        assert!(has(&args(&c), "-device", "hda-output,audiodev=snd0")); // no input without the microphone
        c.microphone = Some(true);
        assert!(has(&args(&c), "-device", "hda-micro,audiodev=snd0"));
        c.audio = Some(false);
        assert!(values(&args(&c), "-audiodev").is_empty() && !c.microphone_on());
    }

    #[test]
    fn pci_slots_do_not_depend_on_the_window() {
        // The disk keeps its slot with and without a window (plan B32)
        fn slot(a: &[String], prefix: &str) -> Option<String> {
            values(a, "-device")
                .into_iter()
                .find(|v| v.starts_with(prefix))
                .and_then(|v| v.split("addr=").last().map(String::from))
        }
        let headless = args(&default_config());
        let window = args_with(&default_config(), DisplayMode::Window, Some("/vm/share/t1"));
        for prefix in ["virtio-blk-pci", "virtio-net-pci", "virtio-rng-pci", "virtio-serial-pci"] {
            assert_eq!(slot(&headless, prefix), slot(&window, prefix), "{prefix}");
        }
        let all: Vec<String> =
            values(&window, "-device").iter().filter_map(|v| v.split("addr=").nth(1).map(String::from)).collect();
        assert_eq!(all.iter().collect::<HashSet<_>>().len(), all.len()); // no slot used twice
    }

    /// The Mac's whole argument list, word for word as before the Linux host (plan B124): headless and with a 3D window
    #[test]
    fn mac_arguments_are_unchanged() {
        let headless = args(&default_config());
        let expected_head: Vec<&str> = vec![
            "-name",
            "t1",
            "-machine",
            "virt,highmem=on",
            "-accel",
            "hvf",
            "-cpu",
            "host",
            "-smp",
            "8",
            "-m",
            "16G",
            "-drive",
            "if=pflash,format=raw,readonly=on,file=/fw.fd",
            "-drive",
            "if=pflash,format=raw,file=/vm/instances/t1/efivars.fd",
            "-boot",
            "menu=on,splash-time=0",
            "-drive",
            "if=none,id=hd0,format=qcow2,file=/vm/instances/t1/disk.qcow2",
            "-device",
            "virtio-blk-pci,drive=hd0,bootindex=0,addr=0x2",
            "-device",
            "virtio-rng-pci,addr=0x3",
            "-device",
            "virtio-gpu-pci,addr=0x5",
            "-display",
            "none",
            "-audiodev",
            "coreaudio,id=snd0",
            "-device",
            "intel-hda,addr=0x9",
            "-device",
            "hda-output,audiodev=snd0",
            "-netdev",
            "user,id=n0,hostfwd=tcp:127.0.0.1:2230-:22",
            "-device",
            "virtio-net-pci,netdev=n0,addr=0x4",
            "-fsdev",
            "local,id=fs0,path=/vm/share/t1,security_model=mapped-xattr",
            "-device",
            "virtio-9p-pci,fsdev=fs0,mount_tag=share,addr=0x7",
            "-device",
            "virtio-serial-pci,addr=0x8",
            "-chardev",
            "socket,id=qga0,path=/vm/instances/t1/qga.sock,server=on,wait=off",
            "-device",
            "virtserialport,chardev=qga0,name=org.qemu.guest_agent.0",
            "-chardev",
            "socket,id=agent0,path=/vm/instances/t1/agent.sock,server=on,wait=off",
            "-device",
            "virtserialport,chardev=agent0,name=org.omaflake.agent",
            "-chardev",
            "socket,id=clip0,path=/vm/instances/t1/clip.sock,server=on,wait=off",
            "-device",
            "virtserialport,chardev=clip0,name=org.omaflake.clipboard",
            "-qmp",
            "unix:/vm/instances/t1/qmp.sock,server=on,wait=off",
            "-chardev",
            "file,id=serial0,path=/vm/instances/t1/serial.log,append=on",
            "-serial",
            "chardev:serial0",
            "-pidfile",
            "/vm/instances/t1/qemu.pid",
        ];
        assert_eq!(headless, expected_head);
        let mut cmd = QemuCommand::new("/fw.fd", ip(), None, DisplayMode::Gpu);
        cmd.capabilities = QemuCapabilities { gl: true, guest_owner: false };
        let gpu = cmd.arguments(&default_config());
        let window: Vec<&str> = vec![
            "-device",
            "virtio-gpu-gl-pci,addr=0x5",
            "-display",
            "cocoa,gl=on,zoom-to-fit=on",
            "-device",
            "qemu-xhci,addr=0x6",
            "-device",
            "usb-kbd",
            "-device",
            "usb-tablet",
        ];
        let at = gpu.iter().position(|a| a == "virtio-gpu-gl-pci,addr=0x5").unwrap() - 1;
        assert_eq!(&gpu[at..at + window.len()], window);
        assert!(!gpu.contains(&"-vga".to_string()));
    }

    fn linux(arch: Arch, accel: Accel, display: DisplayMode) -> QemuCommand {
        let mut cmd = QemuCommand::new("/fw.fd", ip(), Some("/vm/share/t1".into()), display);
        cmd.target = Target::linux(arch, accel, Some("pipewire"));
        cmd.capabilities = QemuCapabilities { gl: true, guest_owner: false };
        cmd
    }

    /// Every architecture × accelerator on a Linux host: machine, accelerator and CPU
    #[test]
    fn linux_machine_accelerator_and_cpu() {
        let cases = [
            (Arch::X86_64, Accel::Kvm, "q35", "kvm", "host"),
            (Arch::X86_64, Accel::Tcg, "q35", "tcg", "max"),
            (Arch::Aarch64, Accel::Kvm, "virt,highmem=on,gic-version=host", "kvm", "host"),
            (Arch::Aarch64, Accel::Tcg, "virt,highmem=on,gic-version=max", "tcg", "max"),
        ];
        for (arch, accel, machine, accel_arg, cpu) in cases {
            let a = linux(arch, accel, DisplayMode::None).arguments(&default_config());
            assert_eq!(values(&a, "-machine"), [machine], "{arch:?} {accel:?}");
            assert_eq!(values(&a, "-accel"), [accel_arg]);
            assert_eq!(values(&a, "-cpu"), [cpu]);
            // x86_64 keeps q35's own VGA card out; aarch64 has none
            assert_eq!(values(&a, "-vga"), if arch == Arch::X86_64 { vec!["none"] } else { vec![] });
            // host-independent parts as on the Mac
            assert!(has(&a, "-device", "virtio-blk-pci,drive=hd0,bootindex=0,addr=0x2"));
            assert_eq!(values(&a, "-netdev"), ["user,id=n0,hostfwd=tcp:127.0.0.1:2230-:22"]);
            assert_eq!(values(&a, "-fsdev"), ["local,id=fs0,path=/vm/share/t1,security_model=mapped-xattr"]);
            assert!(has(&a, "-device", "virtserialport,chardev=agent0,name=org.omaflake.agent"));
            assert_eq!(values(&a, "-qmp"), ["unix:/vm/instances/t1/qmp.sock,server=on,wait=off"]);
            assert_eq!(values(&a, "-serial"), ["chardev:serial0"]);
            assert_eq!(values(&a, "-display"), ["none"]);
            assert!(has(&a, "-device", "virtio-gpu-pci,addr=0x5"));
        }
    }

    /// Windows on Linux: GTK, virtio-vga(-gl) on x86_64, virtio-gpu(-gl)-pci on aarch64; GL is gl=on (no ANGLE)
    #[test]
    fn linux_windows() {
        let cases = [
            (Arch::X86_64, DisplayMode::Window, "virtio-vga,addr=0x5", "gtk,zoom-to-fit=on"),
            (Arch::X86_64, DisplayMode::Gpu, "virtio-vga-gl,addr=0x5", "gtk,gl=on,zoom-to-fit=on"),
            (Arch::Aarch64, DisplayMode::Window, "virtio-gpu-pci,addr=0x5", "gtk,zoom-to-fit=on"),
            (Arch::Aarch64, DisplayMode::Gpu, "virtio-gpu-gl-pci,addr=0x5", "gtk,gl=on,zoom-to-fit=on"),
        ];
        for (arch, display, gpu, ui) in cases {
            for accel in [Accel::Kvm, Accel::Tcg] {
                let mut cmd = linux(arch, accel, display);
                cmd.gl = GlBackend::Es;
                let a = cmd.arguments(&default_config());
                assert!(has(&a, "-device", gpu), "{arch:?} {display:?}: {a:?}");
                assert_eq!(values(&a, "-display"), [ui]);
                assert!(has(&a, "-device", "usb-tablet"));
            }
        }
        let mut cmd = linux(Arch::X86_64, Accel::Kvm, DisplayMode::Gpu);
        cmd.initial_resolution = Some((1920, 1080));
        assert!(has(&cmd.arguments(&default_config()), "-device", "virtio-vga-gl,xres=1920,yres=1080,addr=0x5"));
    }

    /// Sound on Linux through the host's sound server; none without one
    #[test]
    fn linux_sound() {
        let mut cmd = linux(Arch::X86_64, Accel::Kvm, DisplayMode::None);
        let mut c = default_config();
        assert_eq!(values(&cmd.arguments(&c), "-audiodev"), ["pipewire,id=snd0"]);
        c.microphone = Some(true);
        cmd.target.audio = Some("pa");
        assert_eq!(values(&cmd.arguments(&c), "-audiodev"), ["pa,id=snd0"]);
        assert!(has(&cmd.arguments(&c), "-device", "hda-micro,audiodev=snd0"));
        cmd.target.audio = None;
        let a = cmd.arguments(&c);
        assert!(values(&a, "-audiodev").is_empty());
        assert!(!values(&a, "-device").iter().any(|d| d.starts_with("intel-hda")));
    }

    #[test]
    fn architectures_and_accelerators() {
        assert_eq!(Arch::of_system("x86_64-linux"), Some(Arch::X86_64));
        assert_eq!(Arch::of_system("aarch64-linux"), Some(Arch::Aarch64));
        assert_eq!(Arch::of_system("riscv64-linux"), None);
        assert_eq!(Arch::X86_64.qemu_binary(), "qemu-system-x86_64");
        for a in [Accel::Hvf, Accel::Kvm, Accel::Tcg] {
            assert_eq!(Accel::parse(a.as_str()), Some(a));
        }
        assert!(!Accel::Tcg.is_hardware() && Accel::Kvm.is_hardware());
    }
}
