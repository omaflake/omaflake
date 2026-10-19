// SPDX-License-Identifier: AGPL-3.0-or-later
//! What the agent reports about the guest: hello facts and the periodic status – Linux from /proc and logind,
//! macOS from sysctl, getloadavg and utmpx (plan B106).

use serde::Serialize;
use std::ffi::CStr;
#[cfg(target_os = "linux")]
use std::fs;

#[derive(Serialize)]
pub struct ProtocolRange {
    pub min: u32,
    pub max: u32,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Hello {
    pub agent: &'static str,
    pub protocol: ProtocolRange,
    pub os: &'static str,
    pub os_name: Option<String>,
    pub kernel: Option<String>,
    pub hostname: Option<String>,
    pub boot_id: Option<String>,
    /// What this agent can do; the Mac enables a subset in its welcome
    pub features: Vec<&'static str>,
}

#[derive(Serialize)]
pub struct Memory {
    pub total: u64,
    pub available: u64,
}

#[derive(Serialize)]
pub struct Disk {
    pub path: &'static str,
    pub total: u64,
    pub free: u64,
}

#[derive(Serialize)]
pub struct Address {
    pub interface: String,
    pub family: &'static str,
    pub address: String,
}

#[derive(Serialize)]
pub struct Status {
    pub uptime: Option<f64>,
    pub load: Option<[f64; 3]>,
    pub memory: Option<Memory>,
    pub disk: Option<Disk>,
    pub addresses: Vec<Address>,
    pub users: Vec<String>,
}

#[cfg(target_os = "linux")]
fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string()).filter(|s| !s.is_empty())
}

#[cfg(target_os = "linux")]
pub fn hello(features: Vec<&'static str>) -> Hello {
    Hello {
        agent: env!("CARGO_PKG_VERSION"),
        protocol: ProtocolRange { min: crate::proto::PROTOCOL_MIN, max: crate::proto::PROTOCOL_MAX },
        os: "linux",
        os_name: os_release_name(),
        kernel: read_trimmed("/proc/sys/kernel/osrelease"),
        hostname: read_trimmed("/proc/sys/kernel/hostname"),
        boot_id: read_trimmed("/proc/sys/kernel/random/boot_id"),
        features,
    }
}

#[cfg(target_os = "macos")]
pub fn hello(features: Vec<&'static str>) -> Hello {
    Hello {
        agent: env!("CARGO_PKG_VERSION"),
        protocol: ProtocolRange { min: crate::proto::PROTOCOL_MIN, max: crate::proto::PROTOCOL_MAX },
        os: "macos",
        os_name: mac::sysctl_string("kern.osproductversion").map(|v| format!("macOS {v}")),
        kernel: mac::sysctl_string("kern.osrelease"),
        hostname: mac::hostname(),
        boot_id: mac::sysctl_string("kern.bootsessionuuid"),
        features,
    }
}

#[cfg(target_os = "linux")]
fn os_release_name() -> Option<String> {
    let text = fs::read_to_string("/etc/os-release").ok()?;
    let line = text.lines().find(|l| l.starts_with("PRETTY_NAME="))?;
    Some(line["PRETTY_NAME=".len()..].trim_matches('"').to_string())
}

#[cfg(target_os = "linux")]
pub fn status() -> Status {
    Status {
        uptime: read_trimmed("/proc/uptime").and_then(|s| s.split_whitespace().next()?.parse().ok()),
        load: load(),
        memory: memory(),
        disk: disk("/"),
        addresses: addresses(),
        users: users(),
    }
}

#[cfg(target_os = "macos")]
pub fn status() -> Status {
    Status {
        uptime: mac::uptime(),
        load: mac::load(),
        memory: mac::memory(),
        disk: disk("/"),
        addresses: addresses(),
        users: mac::users(),
    }
}

#[cfg(target_os = "linux")]
fn load() -> Option<[f64; 3]> {
    let s = read_trimmed("/proc/loadavg")?;
    let v: Vec<f64> = s.split_whitespace().take(3).filter_map(|x| x.parse().ok()).collect();
    (v.len() == 3).then(|| [v[0], v[1], v[2]])
}

#[cfg(target_os = "linux")]
fn memory() -> Option<Memory> {
    let text = fs::read_to_string("/proc/meminfo").ok()?;
    let kib = |key: &str| -> Option<u64> {
        let line = text.lines().find(|l| l.starts_with(key))?;
        line[key.len()..].split_whitespace().next()?.parse::<u64>().ok().map(|k| k * 1024)
    };
    Some(Memory { total: kib("MemTotal:")?, available: kib("MemAvailable:")? })
}

fn disk(path: &'static str) -> Option<Disk> {
    let c = std::ffi::CString::new(path).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let block = st.f_frsize as u64;
    Some(Disk { path, total: st.f_blocks as u64 * block, free: st.f_bavail as u64 * block })
}

/// IPv4 and IPv6 addresses of all interfaces except loopback
fn addresses() -> Vec<Address> {
    let mut out = vec![];
    let mut list: *mut libc::ifaddrs = std::ptr::null_mut();
    if unsafe { libc::getifaddrs(&mut list) } != 0 {
        return out;
    }
    let mut cur = list;
    while !cur.is_null() {
        let ifa = unsafe { &*cur };
        cur = ifa.ifa_next;
        if ifa.ifa_addr.is_null() || ifa.ifa_flags as u32 & libc::IFF_LOOPBACK as u32 != 0 {
            continue;
        }
        let name = unsafe { CStr::from_ptr(ifa.ifa_name) }.to_string_lossy().into_owned();
        let family = unsafe { (*ifa.ifa_addr).sa_family } as i32;
        let address = match family {
            libc::AF_INET => {
                let sin = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in) };
                std::net::Ipv4Addr::from(u32::from_be(sin.sin_addr.s_addr)).to_string()
            }
            libc::AF_INET6 => {
                let sin6 = unsafe { &*(ifa.ifa_addr as *const libc::sockaddr_in6) };
                std::net::Ipv6Addr::from(sin6.sin6_addr.s6_addr).to_string()
            }
            _ => continue,
        };
        out.push(Address { interface: name, family: if family == libc::AF_INET { "ipv4" } else { "ipv6" }, address });
    }
    unsafe { libc::freeifaddrs(list) };
    out
}

/// Users with a session (systemd-logind's /run/systemd/users/<uid>, key NAME=)
#[cfg(target_os = "linux")]
fn users() -> Vec<String> {
    let mut names: Vec<String> = fs::read_dir("/run/systemd/users")
        .map(|dir| {
            dir.filter_map(|e| fs::read_to_string(e.ok()?.path()).ok())
                .filter(|text| !text.lines().any(|l| l == "STATE=closing" || l == "STATE=lingering"))
                .filter_map(|text| text.lines().find_map(|l| l.strip_prefix("NAME=").map(str::to_string)))
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names.dedup();
    names
}

/// macOS: the same facts from sysctl, getloadavg and utmpx
#[cfg(target_os = "macos")]
mod mac {
    use super::Memory;
    use std::ffi::{CStr, CString};

    fn sysctl_raw(name: &str) -> Option<Vec<u8>> {
        let c = CString::new(name).ok()?;
        let mut len: libc::size_t = 0;
        if unsafe { libc::sysctlbyname(c.as_ptr(), std::ptr::null_mut(), &mut len, std::ptr::null_mut(), 0) } != 0 {
            return None;
        }
        let mut buf = vec![0u8; len];
        if unsafe { libc::sysctlbyname(c.as_ptr(), buf.as_mut_ptr().cast(), &mut len, std::ptr::null_mut(), 0) } != 0 {
            return None;
        }
        buf.truncate(len);
        Some(buf)
    }

    pub fn sysctl_string(name: &str) -> Option<String> {
        let raw = sysctl_raw(name)?;
        let s = CStr::from_bytes_until_nul(&raw).ok()?.to_string_lossy().trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    fn sysctl_u64(name: &str) -> Option<u64> {
        let raw = sysctl_raw(name)?;
        match raw.len() {
            8 => Some(u64::from_ne_bytes(raw.try_into().ok()?)),
            4 => Some(u32::from_ne_bytes(raw.try_into().ok()?) as u64),
            _ => None,
        }
    }

    pub fn hostname() -> Option<String> {
        let mut buf = [0u8; 256];
        if unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } != 0 {
            return None;
        }
        Some(CStr::from_bytes_until_nul(&buf).ok()?.to_string_lossy().into_owned())
    }

    pub fn uptime() -> Option<f64> {
        let raw = sysctl_raw("kern.boottime")?;
        if raw.len() < std::mem::size_of::<libc::timeval>() {
            return None;
        }
        let boot: libc::timeval = unsafe { std::ptr::read_unaligned(raw.as_ptr().cast()) };
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).ok()?.as_secs_f64();
        Some(now - (boot.tv_sec as f64 + boot.tv_usec as f64 / 1e6))
    }

    pub fn load() -> Option<[f64; 3]> {
        let mut l = [0f64; 3];
        (unsafe { libc::getloadavg(l.as_mut_ptr(), 3) } == 3).then_some(l)
    }

    /// Total, and free plus inactive pages as "available" (what macOS can hand out without paging)
    pub fn memory() -> Option<Memory> {
        let total = sysctl_u64("hw.memsize")?;
        let page = sysctl_u64("hw.pagesize").unwrap_or(16384);
        let free = sysctl_u64("vm.page_free_count").unwrap_or(0) + sysctl_u64("vm.page_purgeable_count").unwrap_or(0);
        Some(Memory { total, available: (free * page).min(total) })
    }

    /// Users logged in at the console or a terminal (utmpx)
    pub fn users() -> Vec<String> {
        let mut names = vec![];
        unsafe {
            libc::setutxent();
            loop {
                let e = libc::getutxent();
                if e.is_null() {
                    break;
                }
                if (*e).ut_type == libc::USER_PROCESS {
                    let name = CStr::from_ptr((*e).ut_user.as_ptr()).to_string_lossy().into_owned();
                    if !name.is_empty() {
                        names.push(name);
                    }
                }
            }
            libc::endutxent();
        }
        names.sort();
        names.dedup();
        names
    }
}
