// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the host gives QEMU (plan B124): the accelerator (HVF on a Mac; KVM on Linux when /dev/kvm can be opened, TCG
//! otherwise), the sound server on Linux, and the UEFI firmware.
//!
//! Firmware: QEMU's own edk2 builds next to the binary (`<prefix>/share/qemu/edk2-<arch>-code.fd`, plus
//! `edk2-i386-vars.fd` as the x86_64 variable store's template – nixpkgs' `qemu` and omaflake's QEMU ship them), else the
//! firmware descriptors of QEMU's firmware interop spec (`docs/interop/firmware.json`: `<prefix>/share/qemu/firmware`,
//! `/etc/qemu/firmware`, `/usr/share/qemu/firmware`), which distributions install with OVMF/AAVMF packages.
//! OPEN: the descriptors of Debian/Ubuntu/Fedora/Arch are not tried on those systems yet (the parser is tested against
//! QEMU's own descriptors, the same format).

use std::path::{Path, PathBuf};

use serde_json::Value;

use crate::qemu_command::{Accel, Arch};

/// The UEFI firmware for one architecture: the code (read-only pflash) and, where the firmware needs a formatted
/// variable store, its template (x86_64's OVMF); None = a blank 64 MiB store works (aarch64's edk2)
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Firmware {
    pub code: PathBuf,
    pub vars: Option<PathBuf>,
}

/// Where firmware descriptors are looked for, in QEMU's order of precedence (a file name found earlier wins)
pub fn descriptor_dirs(prefix: &Path) -> Vec<PathBuf> {
    vec![prefix.join("share/qemu/firmware"), "/etc/qemu/firmware".into(), "/usr/share/qemu/firmware".into()]
}

impl Firmware {
    /// The firmware for `arch` next to a QEMU installed under `prefix` (the folder above its `bin`), else from the
    /// descriptors in `dirs`
    pub fn locate(prefix: &Path, arch: Arch, dirs: &[PathBuf]) -> Option<Firmware> {
        Self::bundled(prefix, arch).or_else(|| Self::from_descriptors(arch, dirs))
    }

    /// QEMU's own edk2 build (`share/qemu/edk2-<arch>-code.fd`); x86_64 needs `edk2-i386-vars.fd` with it
    pub fn bundled(prefix: &Path, arch: Arch) -> Option<Firmware> {
        let share = prefix.join("share/qemu");
        let code = share.join(format!("edk2-{}-code.fd", arch.as_str()));
        if !code.is_file() {
            return None;
        }
        match arch {
            Arch::Aarch64 => Some(Firmware { code, vars: None }),
            Arch::X86_64 => {
                let vars = share.join("edk2-i386-vars.fd");
                vars.is_file().then_some(Firmware { code, vars: Some(vars) })
            }
        }
    }

    /// The first usable descriptor for `arch` (file names sorted; a name in an earlier folder hides the same name later):
    /// UEFI in split flash (code + variable template, raw), for the machine omaflake uses (`virt`, `q35`), without
    /// Secure Boot, enrolled keys or SMM (omaflake's q35 has no SMM).
    pub fn from_descriptors(arch: Arch, dirs: &[PathBuf]) -> Option<Firmware> {
        let mut files: Vec<(String, PathBuf)> = Vec::new();
        for dir in dirs {
            let Ok(entries) = std::fs::read_dir(dir) else { continue };
            let mut names: Vec<(String, PathBuf)> = entries
                .flatten()
                .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
                .filter(|(n, _)| n.ends_with(".json"))
                .collect();
            names.sort();
            for (name, path) in names {
                if !files.iter().any(|(n, _)| *n == name) {
                    files.push((name, path));
                }
            }
        }
        files.sort_by(|a, b| a.0.cmp(&b.0));
        files.iter().find_map(|(_, path)| {
            let json: Value = serde_json::from_slice(&std::fs::read(path).ok()?).ok()?;
            Self::from_descriptor(&json, arch)
        })
    }

    /// One descriptor, when it fits (see `from_descriptors`)
    pub fn from_descriptor(json: &Value, arch: Arch) -> Option<Firmware> {
        let strings = |v: Option<&Value>| -> Vec<String> {
            v.and_then(Value::as_array)
                .map(|a| a.iter().filter_map(Value::as_str).map(String::from).collect())
                .unwrap_or_default()
        };
        if !strings(json.get("interface-types")).iter().any(|t| t == "uefi") {
            return None;
        }
        let features = strings(json.get("features"));
        if features.iter().any(|f| matches!(f.as_str(), "requires-smm" | "secure-boot" | "enrolled-keys")) {
            return None;
        }
        let machine = match arch {
            Arch::Aarch64 => "virt",
            Arch::X86_64 => "pc-q35",
        };
        let targets = json.get("targets")?.as_array()?;
        let fits = targets.iter().any(|t| {
            t.get("architecture").and_then(Value::as_str) == Some(arch.as_str())
                && strings(t.get("machines")).iter().any(|m| m.starts_with(machine))
        });
        if !fits {
            return None;
        }
        let mapping = json.get("mapping")?;
        if mapping.get("device").and_then(Value::as_str) != Some("flash") {
            return None;
        }
        if mapping.get("mode").and_then(Value::as_str).is_some_and(|m| m != "split") {
            return None;
        }
        let file = |key: &str| -> Option<PathBuf> {
            let entry = mapping.get(key)?;
            if entry.get("format").and_then(Value::as_str).is_some_and(|f| f != "raw") {
                return None;
            }
            let path = PathBuf::from(entry.get("filename")?.as_str()?);
            path.is_file().then_some(path)
        };
        let code = file("executable")?;
        let vars = file("nvram-template");
        // x86_64's OVMF needs its formatted variable store
        if arch == Arch::X86_64 && vars.is_none() {
            return None;
        }
        Some(Firmware { code, vars })
    }
}

/// A Linux host's /dev/kvm: usable (opened read-write), there but not ours, or missing
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KvmState {
    Usable,
    NoAccess,
    Missing,
}

impl KvmState {
    pub const DEVICE: &'static str = "/dev/kvm";

    /// Opens `device` read-write (what QEMU does); EACCES/EPERM = not ours, else missing
    pub fn of(device: &Path) -> KvmState {
        match std::fs::OpenOptions::new().read(true).write(true).open(device) {
            Ok(_) => KvmState::Usable,
            Err(e) if matches!(e.kind(), std::io::ErrorKind::PermissionDenied) => KvmState::NoAccess,
            Err(_) if device.exists() => KvmState::NoAccess,
            Err(_) => KvmState::Missing,
        }
    }

    /// This host's /dev/kvm (Missing on a Mac)
    pub fn detect() -> KvmState {
        if cfg!(target_os = "linux") { Self::of(Path::new(Self::DEVICE)) } else { KvmState::Missing }
    }
}

/// The accelerator for this host: HVF on a Mac; on Linux KVM when /dev/kvm can be opened, else TCG.
/// `OMAFLAKE_ACCEL=tcg` forces TCG (to compare, or when KVM misbehaves).
pub fn host_accel() -> Accel {
    if cfg!(target_os = "macos") {
        return Accel::Hvf;
    }
    if std::env::var("OMAFLAKE_ACCEL").as_deref() == Ok("tcg") {
        return Accel::Tcg;
    }
    if KvmState::detect() == KvmState::Usable { Accel::Kvm } else { Accel::Tcg }
}

/// The sound server of a Linux session, as QEMU's `-audiodev` driver: PipeWire when its socket is there and QEMU has
/// the driver, else PulseAudio (also PipeWire's pulse server); None without either (no sound device then).
/// `drivers`: what `qemu-system-* -audiodev help` lists; `runtime`: `$XDG_RUNTIME_DIR`.
pub fn linux_audio(drivers: &str, runtime: Option<&Path>) -> Option<&'static str> {
    let runtime = runtime?;
    let has = |d: &str| drivers.lines().any(|l| l.trim() == d);
    if has("pipewire") && runtime.join("pipewire-0").exists() {
        return Some("pipewire");
    }
    if has("pa") && runtime.join("pulse/native").exists() {
        return Some("pa");
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn touch(p: &Path) {
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(p, b"x").unwrap();
    }

    #[test]
    fn bundled_firmware_next_to_qemu() {
        let d = tempfile::tempdir().unwrap();
        let share = d.path().join("share/qemu");
        assert_eq!(Firmware::bundled(d.path(), Arch::Aarch64), None);
        touch(&share.join("edk2-aarch64-code.fd"));
        touch(&share.join("edk2-x86_64-code.fd"));
        assert_eq!(
            Firmware::bundled(d.path(), Arch::Aarch64),
            Some(Firmware { code: share.join("edk2-aarch64-code.fd"), vars: None })
        );
        // x86_64 needs the variable store's template
        assert_eq!(Firmware::bundled(d.path(), Arch::X86_64), None);
        touch(&share.join("edk2-i386-vars.fd"));
        assert_eq!(
            Firmware::bundled(d.path(), Arch::X86_64),
            Some(Firmware { code: share.join("edk2-x86_64-code.fd"), vars: Some(share.join("edk2-i386-vars.fd")) })
        );
    }

    /// QEMU's own descriptor (share/qemu/firmware/60-edk2-x86_64.json of QEMU 11.1, paths replaced)
    fn descriptor(dir: &Path, arch: &str, machines: &str, features: &[&str]) -> Value {
        let code = dir.join(format!("{arch}-code.fd"));
        let vars = dir.join(format!("{arch}-vars.fd"));
        touch(&code);
        touch(&vars);
        json!({
            "description": format!("UEFI firmware for {arch}"),
            "interface-types": ["uefi"],
            "mapping": {
                "device": "flash",
                "executable": {"filename": code, "format": "raw"},
                "nvram-template": {"filename": vars, "format": "raw"}
            },
            "targets": [{"architecture": arch, "machines": [machines]}],
            "features": features,
            "tags": []
        })
    }

    #[test]
    fn firmware_from_descriptors() {
        let d = tempfile::tempdir().unwrap();
        let x86 = descriptor(d.path(), "x86_64", "pc-q35-*", &["acpi-s3", "amd-sev", "verbose-dynamic"]);
        let found = Firmware::from_descriptor(&x86, Arch::X86_64).unwrap();
        assert_eq!(found.code, d.path().join("x86_64-code.fd"));
        assert_eq!(found.vars, Some(d.path().join("x86_64-vars.fd")));
        assert_eq!(Firmware::from_descriptor(&x86, Arch::Aarch64), None);
        // Secure Boot / SMM builds and i440fx-only builds are left out
        let secure = descriptor(d.path(), "x86_64", "pc-q35-*", &["requires-smm", "secure-boot"]);
        assert_eq!(Firmware::from_descriptor(&secure, Arch::X86_64), None);
        let i440fx = descriptor(d.path(), "x86_64", "pc-i440fx-*", &[]);
        assert_eq!(Firmware::from_descriptor(&i440fx, Arch::X86_64), None);
        let arm = descriptor(d.path(), "aarch64", "virt-*", &[]);
        assert!(Firmware::from_descriptor(&arm, Arch::Aarch64).is_some());

        // Sorted by file name across the folders; the first that fits wins
        let dir = d.path().join("firmware");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("50-secure.json"), secure.to_string()).unwrap();
        std::fs::write(dir.join("60-x86.json"), x86.to_string()).unwrap();
        std::fs::write(dir.join("70-arm.json"), arm.to_string()).unwrap();
        std::fs::write(dir.join("10-broken.json"), "{").unwrap();
        let dirs = [d.path().join("missing"), dir];
        assert_eq!(Firmware::from_descriptors(Arch::X86_64, &dirs), Some(found));
        assert_eq!(Firmware::from_descriptors(Arch::Aarch64, &dirs).unwrap().code, d.path().join("aarch64-code.fd"));
        // Bundled files come first
        let prefix = d.path().join("qemu");
        touch(&prefix.join("share/qemu/edk2-aarch64-code.fd"));
        assert_eq!(Firmware::locate(&prefix, Arch::Aarch64, &dirs).unwrap().vars, None);
    }

    #[test]
    fn kvm_device_states() {
        let d = tempfile::tempdir().unwrap();
        assert_eq!(KvmState::of(&d.path().join("kvm")), KvmState::Missing);
        let file = d.path().join("kvm");
        std::fs::write(&file, b"").unwrap();
        assert_eq!(KvmState::of(&file), KvmState::Usable);
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o400)).unwrap();
        // root may open it anyway
        if unsafe { libc::geteuid() } != 0 {
            assert_eq!(KvmState::of(&file), KvmState::NoAccess);
        }
        if cfg!(target_os = "macos") {
            assert_eq!(host_accel(), Accel::Hvf);
        }
    }

    #[test]
    fn sound_server_of_the_session() {
        let d = tempfile::tempdir().unwrap();
        let drivers = "Available audio drivers:\nnone\nalsa\npa\npipewire\n";
        assert_eq!(linux_audio(drivers, Some(d.path())), None);
        assert_eq!(linux_audio(drivers, None), None);
        touch(&d.path().join("pulse/native"));
        assert_eq!(linux_audio(drivers, Some(d.path())), Some("pa"));
        touch(&d.path().join("pipewire-0"));
        assert_eq!(linux_audio(drivers, Some(d.path())), Some("pipewire"));
        // a QEMU without the PipeWire driver uses its pulse server
        assert_eq!(linux_audio("none\npa\n", Some(d.path())), Some("pa"));
        assert_eq!(linux_audio("none\n", Some(d.path())), None);
    }
}
