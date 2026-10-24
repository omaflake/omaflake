// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of MacVM.swift: a macOS VM's folder and config.json, MAC addresses, the DHCP leases and the runner's control
//! socket. `clonefile` is macOS-only (`Error::unsupported` elsewhere).

use std::collections::HashMap;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::Duration;

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use std::os::unix::fs::PermissionsExt;

use crate::error::{codes, IoContext, Result};
use crate::file_format::{write_atomic, FileFormat, Step};
use crate::platform::is_writable;
use crate::sockets::connect_unix;
use crate::vm_err;

/// The VM window's size (Swift `MacVMConfig.Display`; default 1920×1200 at 80 ppi)
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacDisplay {
    pub width: u32,
    pub height: u32,
    pub ppi: u32,
}

impl Default for MacDisplay {
    fn default() -> Self {
        MacDisplay { width: 1920, height: 1200, ppi: 80 }
    }
}

/// The folder's config.json, schema `macos-vm` (plan B114). 0 → 1: `version` (always 1) gave way to `$schema`
fn macvm_v0_to_v1(o: &mut Map<String, Value>) -> Result<()> {
    o.remove("version");
    Ok(())
}

static MACVM_STEPS: [Step; 1] = [macvm_v0_to_v1];

/// Settings of a macOS VM (config.json in the VM folder, spec §5). Keys: cpus, memoryMB, diskGB, display,
/// macAddress, osBuild (None left out). Hardware model, machine identifier and auxiliary storage are separate files
/// next to it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MacVmConfig {
    pub cpus: u32,
    #[serde(rename = "memoryMB")]
    pub memory_mb: u32,
    #[serde(rename = "diskGB")]
    pub disk_gb: u32,
    pub display: MacDisplay,
    /// fixed MAC per VM (for the NAT IP from /var/db/dhcpd_leases)
    #[serde(rename = "macAddress")]
    pub mac_address: String,
    /// Build of the installed IPSW, e.g. "27A5321"
    #[serde(rename = "osBuild", default, skip_serializing_if = "Option::is_none")]
    pub os_build: Option<String>,
}

impl MacVmConfig {
    /// config.json of a macOS VM (see `macvm_v0_to_v1`)
    pub const FILE_FORMAT: FileFormat = FileFormat::new("macos-vm", "config.json of a macOS VM", 0, &MACVM_STEPS);

    pub fn new(cpus: u32, memory_mb: u32, disk_gb: u32, mac_address: &str, os_build: Option<String>) -> Self {
        MacVmConfig {
            cpus,
            memory_mb,
            disk_gb,
            display: MacDisplay::default(),
            mac_address: mac_address.into(),
            os_build,
        }
    }
}

/// Folder of a macOS VM: base, image and instance share the same layout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MacVmBundle {
    pub dir: PathBuf,
}

impl MacVmBundle {
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        MacVmBundle { dir: dir.into() }
    }

    pub fn config(&self) -> PathBuf {
        self.dir.join("config.json")
    }
    /// VZMacHardwareModel.dataRepresentation
    pub fn hardware_model(&self) -> PathBuf {
        self.dir.join("hardware-model")
    }
    /// VZMacMachineIdentifier.dataRepresentation (new per instance)
    pub fn machine_id(&self) -> PathBuf {
        self.dir.join("machine-id")
    }
    /// VZMacAuxiliaryStorage
    pub fn aux_storage(&self) -> PathBuf {
        self.dir.join("aux.img")
    }
    /// System disk, RAW (VZDiskImageStorageDeviceAttachment only supports RAW, plan BM9)
    pub fn disk(&self) -> PathBuf {
        self.dir.join("disk.img")
    }

    /// The files that make up a VM (runtime files like sockets and logs are not part of it).
    pub fn files(&self) -> [PathBuf; 5] {
        [self.config(), self.hardware_model(), self.machine_id(), self.aux_storage(), self.disk()]
    }

    /// config.json; "Not a macOS VM (config.json missing): <dir>". Bases and images are read-only: those are brought
    /// up to date at every read instead of written back.
    pub fn load_config(&self) -> Result<MacVmConfig> {
        let Ok(data) = std::fs::read(self.config()) else {
            return Err(vm_err!("Not a macOS VM (config.json missing): {}", self.dir.display()));
        };
        let (config, migrated) = MacVmConfig::FILE_FORMAT.decode::<MacVmConfig>(&data)?;
        // Bases and images are read-only: those are brought up to date at every read instead
        if migrated && is_writable(&self.dir) {
            let _ = self.save(&config);
        }
        Ok(config)
    }

    pub fn save(&self, c: &MacVmConfig) -> Result<()> {
        MacVmConfig::FILE_FORMAT.check_writable(&self.config())?;
        write_atomic(&self.config(), &MacVmConfig::FILE_FORMAT.encode(c)?)
    }

    /// APFS clone of the VM files into a new folder (no extra space until blocks change), writable (aux.img 0600, the
    /// rest 0644), with a new MAC address so that clone and source get different NAT addresses. The machine
    /// identifier needs Virtualization.framework – `omaf-runner clone` replaces it. Swift `clone(to:)`.
    pub fn clone_to(&self, target: &Path) -> Result<MacVmBundle> {
        if target.exists() {
            return Err(vm_err!(codes::EXISTS, "Target exists already: {}", target.display()));
        }
        self.load_config()?;
        std::fs::create_dir_all(target).with_path(target)?;
        let copy = MacVmBundle::new(target);
        for (src, dst) in self.files().iter().zip(copy.files().iter()) {
            clone_file(src, dst)?;
            let mode = if *src == self.aux_storage() { 0o600 } else { 0o644 };
            std::fs::set_permissions(dst, std::fs::Permissions::from_mode(mode)).with_path(dst)?;
        }
        let mut cfg = copy.load_config()?;
        cfg.mac_address = MacAddress::random_locally_administered();
        copy.save(&cfg)?;
        Ok(copy)
    }

    /// Creates an empty, sparse RAW disk (size a multiple of 512).
    pub fn create_disk(&self, gigabytes: u64) -> Result<()> {
        let disk = self.disk();
        let file = std::fs::File::create(&disk).map_err(|_| vm_err!("Cannot create disk: {}", disk.display()))?;
        file.set_len(gigabytes * 1024 * 1024 * 1024).with_path(&disk)
    }
}

/// `clonefile(2)`: an APFS clone (copy-on-write) of one file
#[cfg(target_os = "macos")]
fn clone_file(src: &Path, dst: &Path) -> Result<()> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let name = src.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let (Ok(s), Ok(d)) = (CString::new(src.as_os_str().as_bytes()), CString::new(dst.as_os_str().as_bytes())) else {
        return Err(vm_err!("clonefile {name} failed: invalid path"));
    };
    // SAFETY: both arguments are valid NUL-terminated strings that live across the call
    if unsafe { libc::clonefile(s.as_ptr(), d.as_ptr(), 0) } != 0 {
        let e = std::io::Error::last_os_error();
        return Err(vm_err!("clonefile {name} failed: {} (APFS required)", crate::error::os_text(&e)));
    }
    Ok(())
}

#[cfg(not(target_os = "macos"))]
fn clone_file(_src: &Path, _dst: &Path) -> Result<()> {
    Err(Error::unsupported("clonefile"))
}

/// Random bytes from /dev/urandom (Swift `UInt8.random`); the clock as a last resort
pub(crate) fn random_bytes<const N: usize>() -> [u8; N] {
    let mut bytes = [0u8; N];
    let filled = std::fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut bytes));
    if filled.is_err() {
        use sha2::{Digest, Sha256};
        let seed = format!("{:?}{}", std::time::SystemTime::now(), std::process::id());
        let digest = Sha256::digest(seed.as_bytes());
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = digest[i % digest.len()];
        }
    }
    bytes
}

/// MAC addresses for VM network cards.
pub enum MacAddress {}

impl MacAddress {
    /// Random unicast address with the "locally administered" bit set (like VZMACAddress.randomLocallyAdministered),
    /// lowercase "xx:xx:xx:xx:xx:xx".
    pub fn random_locally_administered() -> String {
        let mut bytes = random_bytes::<6>();
        bytes[0] = (bytes[0] | 0x02) & 0xFE;
        bytes.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
    }
}

/// IP addresses from the macOS DHCP server (vmnet/NAT), /var/db/dhcpd_leases.
pub enum DhcpLeases {}

impl DhcpLeases {
    pub const FILE: &'static str = "/var/db/dhcpd_leases";

    /// MAC as written in the lease file: lowercase, leading zeros per octet removed
    /// ("e6:25:86:0f:90:4d" → "e6:25:86:f:90:4d").
    pub fn normalize(mac: &str) -> String {
        mac.to_lowercase()
            .split(':')
            .filter(|p| !p.is_empty())
            .map(|part| {
                let trimmed = part.trim_start_matches('0');
                if trimmed.is_empty() {
                    "0"
                } else {
                    trimmed
                }
            })
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Most recent IP for the MAC (entries are newest first; with several, the largest `lease` wins). Swift
    /// `ip(for:in:)`.
    pub fn ip_in(mac: &str, text: &str) -> Option<String> {
        let wanted = Self::normalize(mac);
        let mut best: Option<(String, u64)> = None;
        for block in text.split('}') {
            let mut fields: HashMap<&str, &str> = HashMap::new();
            for line in block.split('\n') {
                let t = line.trim_matches([' ', '\t']);
                if let Some((key, value)) = t.split_once('=') {
                    fields.insert(key, value);
                }
            }
            // hw_address=1,<mac>  (1 = Ethernet)
            let (Some(hw), Some(ip)) = (fields.get("hw_address"), fields.get("ip_address")) else { continue };
            let Some((_, address)) = hw.split_once(',') else { continue };
            if Self::normalize(address) != wanted {
                continue;
            }
            let lease =
                fields.get("lease").and_then(|l| u64::from_str_radix(&l.replace("0x", ""), 16).ok()).unwrap_or(0);
            if best.as_ref().is_none_or(|(_, b)| lease > *b) {
                best = Some((ip.to_string(), lease));
            }
        }
        best.map(|(ip, _)| ip)
    }

    /// From `FILE`. Swift `ip(for:)`.
    pub fn ip(mac: &str) -> Option<String> {
        let text = std::fs::read_to_string(Self::FILE).ok()?;
        Self::ip_in(mac, &text)
    }
}

/// Controls a running omaf-runner via its Unix socket (one JSON line out, one back).
pub enum RunnerClient {}

impl RunnerClient {
    pub const COMMANDS: [&'static str; 4] = ["status", "stop", "kill", "show"];
    /// Swift's default timeout
    pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(5);

    /// Sends `{"command": <command>}` + newline, reads one line back as a string map; "Invalid response from
    /// omaf-runner: <text>" otherwise.
    pub fn send(command: &str, socket: &Path, timeout: Duration) -> Result<HashMap<String, String>> {
        let mut stream = connect_unix(socket, timeout)?;
        let mut line = serde_json::to_vec(&serde_json::json!({ "command": command }))?;
        line.push(b'\n');
        let _ = stream.write_all(&line);
        let mut data = Vec::new();
        let mut buf = [0u8; 4096];
        while !data.contains(&b'\n') {
            match stream.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => data.extend_from_slice(&buf[..n]),
            }
        }
        serde_json::from_slice::<HashMap<String, String>>(&data)
            .map_err(|_| vm_err!("Invalid response from omaf-runner: {}", String::from_utf8_lossy(&data)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::paths::tests::temp_paths;
    use std::io::BufRead;
    use std::os::unix::net::UnixListener;

    #[cfg_attr(not(target_os = "macos"), allow(dead_code))] // its tests are macOS only
    fn bundle_with_files(dir: &Path, mac: &str) -> MacVmBundle {
        let b = MacVmBundle::new(dir);
        std::fs::create_dir_all(dir).unwrap();
        b.save(&MacVmConfig::new(4, 8192, 1, mac, None)).unwrap();
        for f in [b.hardware_model(), b.machine_id(), b.aux_storage()] {
            std::fs::write(f, "x").unwrap();
        }
        b.create_disk(1).unwrap();
        b
    }

    // MacCloneTests
    #[cfg(target_os = "macos")]
    #[test]
    fn clone_copies_files_with_new_mac() {
        let (_t, p) = temp_paths();
        let src = bundle_with_files(&p.root.join("base"), "c2:c1:76:60:ce:b1");
        std::fs::write(src.dir.join("runner.pid"), "sock").unwrap();
        for f in src.files() {
            std::fs::set_permissions(&f, std::fs::Permissions::from_mode(0o444)).unwrap();
        }
        let copy = src.clone_to(&p.root.join("copy")).unwrap();
        assert_ne!(copy.load_config().unwrap().mac_address, "c2:c1:76:60:ce:b1");
        assert!(is_writable(&copy.disk()));
        let aux = std::fs::metadata(copy.aux_storage()).unwrap().permissions().mode() & 0o777;
        assert_eq!(aux, 0o600);
        assert!(!copy.dir.join("runner.pid").exists());
        assert_eq!(std::fs::read(copy.hardware_model()).unwrap(), b"x");
        let again = src.clone_to(&copy.dir).unwrap_err();
        assert_eq!(again.code(), Some(codes::EXISTS));
        assert!(again.message().starts_with("Target exists already: "));
    }

    #[test]
    fn clone_of_a_folder_without_config_is_refused() {
        let (_t, p) = temp_paths();
        std::fs::create_dir_all(p.root.join("empty")).unwrap();
        let e = MacVmBundle::new(p.root.join("empty")).clone_to(&p.root.join("copy")).unwrap_err();
        assert!(e.message().starts_with("Not a macOS VM (config.json missing): "));
        assert!(!p.root.join("copy").exists());
    }

    #[test]
    fn random_mac_is_local_unicast() {
        for _ in 0..50 {
            let mac = MacAddress::random_locally_administered();
            assert_eq!(mac.len(), 17);
            let first = u8::from_str_radix(&mac[..2], 16).unwrap();
            assert_eq!(first & 0x02, 0x02);
            assert_eq!(first & 0x01, 0);
        }
    }

    // MacVMTests – excerpt as in /var/db/dhcpd_leases (2026-10-16): leading zeros per octet are missing
    const LEASES: &str = "{\n\tname=ManagedlMachine\n\tip_address=192.168.64.14\n\thw_address=1,e6:25:86:f:90:4d\n\t\
identifier=1,e6:25:86:f:90:4d\n\tlease=0x6ad236f9\n}\n{\n\tname=old\n\tip_address=192.168.64.9\n\t\
hw_address=1,e6:25:86:f:90:4d\n\tidentifier=1,e6:25:86:f:90:4d\n\tlease=0x6a000000\n}\n{\n\tname=archlinux\n\t\
ip_address=192.168.64.13\n\thw_address=ff,f1:f5:dd:7f:0:2:0:0:ab:11:ef:55:ac:1e:8d:9:12:10\n\tlease=0x6a9da38d\n}";

    #[test]
    fn normalizes_mac() {
        assert_eq!(DhcpLeases::normalize("E6:25:86:0F:90:4D"), "e6:25:86:f:90:4d");
        assert_eq!(DhcpLeases::normalize("00:0a:00:00:00:01"), "0:a:0:0:0:1");
    }

    #[test]
    fn finds_newest_lease() {
        assert_eq!(DhcpLeases::ip_in("e6:25:86:0f:90:4d", LEASES).as_deref(), Some("192.168.64.14"));
        assert_eq!(DhcpLeases::ip_in("aa:bb:cc:dd:ee:ff", LEASES), None);
        // the order in the file does not matter, the largest lease wins
        let reversed = LEASES.replace("0x6ad236f9", "0x1");
        assert_eq!(DhcpLeases::ip_in("e6:25:86:0f:90:4d", &reversed).as_deref(), Some("192.168.64.9"));
    }

    #[test]
    fn bundle_round_trip_and_sparse_disk() {
        let (_t, p) = temp_paths();
        let b = MacVmBundle::new(p.root.join("macvm"));
        std::fs::create_dir_all(&b.dir).unwrap();
        let c = MacVmConfig::new(4, 8192, 2, "e6:25:86:0f:90:4d", Some("27A1".into()));
        b.save(&c).unwrap();
        assert_eq!(b.load_config().unwrap(), c);
        let text = std::fs::read_to_string(b.config()).unwrap();
        assert!(text.contains("\"$schema\" : \"https://omaflake.dev/schema/macos-vm/v1.json\""));
        assert!(text.contains("\"memoryMB\" : 8192") && text.contains("\"osBuild\" : \"27A1\""));
        b.create_disk(2).unwrap();
        let meta = std::fs::metadata(b.disk()).unwrap();
        assert_eq!(meta.len(), 2 * 1024 * 1024 * 1024);
        use std::os::unix::fs::MetadataExt;
        assert!(meta.blocks() * 512 < 1024 * 1024); // sparse
    }

    // FileFormatTests.macVMVersionGivesWayToSchema
    #[test]
    fn mac_vm_version_gives_way_to_schema() {
        let (_t, p) = temp_paths();
        let dir = p.root.join("vm");
        std::fs::create_dir_all(&dir).unwrap();
        let old = r#"{"version":1,"cpus":4,"memoryMB":8192,"diskGB":80,"display":{"width":1920,"height":1200,"ppi":80},"macAddress":"02:00:00:00:00:01"}"#;
        std::fs::write(dir.join("config.json"), old).unwrap();
        assert_eq!(MacVmBundle::new(&dir).load_config().unwrap().cpus, 4);
        let written = std::fs::read_to_string(dir.join("config.json")).unwrap();
        assert!(!written.contains("\"version\""));
        assert!(written.contains("macos-vm/v1.json"));
        // a read-only folder (base, image) is read but not written back
        std::fs::write(dir.join("config.json"), old).unwrap();
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o555)).unwrap();
        assert_eq!(MacVmBundle::new(&dir).load_config().unwrap().disk_gb, 80);
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o755)).unwrap();
        assert!(std::fs::read_to_string(dir.join("config.json")).unwrap().contains("\"version\""));
    }

    #[test]
    fn runner_client_sends_one_line_and_reads_one_back() {
        let (_t, p) = temp_paths();
        let socket = p.root.join("r.sock");
        let listener = UnixListener::bind(&socket).unwrap();
        let server = std::thread::spawn(move || {
            let mut got = Vec::new();
            for reply in [&b"{\"state\":\"running\",\"pid\":\"42\"}\n"[..], b"not json\n"] {
                let (s, _) = listener.accept().unwrap();
                let mut r = std::io::BufReader::new(s);
                let mut line = String::new();
                r.read_line(&mut line).unwrap();
                got.push(line);
                r.get_mut().write_all(reply).unwrap();
            }
            got
        });
        let reply = RunnerClient::send("status", &socket, RunnerClient::DEFAULT_TIMEOUT).unwrap();
        assert_eq!(reply.get("state").map(String::as_str), Some("running"));
        assert_eq!(reply.get("pid").map(String::as_str), Some("42"));
        let e = RunnerClient::send("kill", &socket, RunnerClient::DEFAULT_TIMEOUT).unwrap_err();
        assert_eq!(e.message(), "Invalid response from omaf-runner: not json\n");
        assert_eq!(server.join().unwrap(), vec!["{\"command\":\"status\"}\n", "{\"command\":\"kill\"}\n"]);
        assert!(RunnerClient::send("status", &p.root.join("none.sock"), Duration::from_secs(1)).is_err());
    }
}
