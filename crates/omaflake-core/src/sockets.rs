// SPDX-License-Identifier: AGPL-3.0-or-later
//! Port of Sockets.swift: small POSIX socket helpers (Unix sockets for QMP, TCP for port and SSH checks).

use std::io::Read;
use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use crate::error::{os_text, Result};
use crate::vm_err;

/// Swift `enum Ports` (namespace)
pub enum Ports {}

impl Ports {
    /// Is 127.0.0.1:<port> free (bindable)? A plain bind without SO_REUSEADDR (std's `TcpListener` would set it and
    /// call a port in use by a wildcard listener free on macOS).
    pub fn is_free(port: u16) -> bool {
        // SAFETY: libc socket calls on a descriptor owned and closed here; sockaddr_in is fully initialized
        unsafe {
            let fd = libc::socket(libc::AF_INET, libc::SOCK_STREAM, 0);
            if fd < 0 {
                return false;
            }
            let addr = loopback(port);
            let rc = libc::bind(
                fd,
                (&addr as *const libc::sockaddr_in).cast(),
                std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
            );
            libc::close(fd);
            rc == 0
        }
    }

    /// Does an SSH server answer on <host>:<port> (banner "SSH-")? Needs no login.
    /// QEMU port forwarding accepts connections even when no sshd is running in the guest yet –
    /// so only the banner counts. Swift's defaults: host "127.0.0.1", timeout 3 s.
    pub fn ssh_banner(host: &str, port: u16, timeout: Duration) -> Option<String> {
        let addr = socket_addr(host, port)?;
        let mut s = TcpStream::connect_timeout(&addr, timeout).ok()?;
        s.set_read_timeout(non_zero(timeout)).ok()?;
        let mut buf = [0u8; 256];
        let n = s.read(&mut buf).ok()?;
        if n == 0 {
            return None;
        }
        let line = String::from_utf8_lossy(&buf[..n]).into_owned();
        line.starts_with("SSH-").then(|| line.trim().to_string())
    }

    /// Does <host>:<port> accept a TCP connection at all (no matter what answers)? For diagnostics.
    pub fn is_open(host: &str, port: u16, timeout: Duration) -> bool {
        socket_addr(host, port).is_some_and(|a| TcpStream::connect_timeout(&a, timeout).is_ok())
    }
}

/// Swift's defaults for `Ports.sshBanner` and `Ports.isOpen`
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(3);

fn loopback(port: u16) -> libc::sockaddr_in {
    // SAFETY: an all-zero sockaddr_in is valid; the fields are set below
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    #[cfg(any(target_os = "macos", target_os = "ios", target_os = "freebsd"))]
    {
        addr.sin_len = std::mem::size_of::<libc::sockaddr_in>() as u8;
    }
    addr.sin_family = libc::AF_INET as libc::sa_family_t;
    addr.sin_port = port.to_be();
    addr.sin_addr.s_addr = u32::from(Ipv4Addr::LOCALHOST).to_be();
    addr
}

/// IPv4 only, as Swift's `inet_addr`
fn socket_addr(host: &str, port: u16) -> Option<SocketAddr> {
    let ip: Ipv4Addr = host.parse().ok()?;
    Some(SocketAddr::V4(SocketAddrV4::new(ip, port)))
}

/// `set_read_timeout(Some(0))` is an error in Rust; zero means "no timeout" there
fn non_zero(d: Duration) -> Option<Duration> {
    (!d.is_zero()).then_some(d)
}

/// Longest Unix socket path (sun_path minus the terminating NUL): 103 bytes on macOS, 107 on Linux
pub const MAX_SOCKET_PATH: usize = {
    #[cfg(target_os = "linux")]
    {
        107
    }
    #[cfg(not(target_os = "linux"))]
    {
        103
    }
};

/// Connects to a Unix socket; reads time out after `timeout` (Swift returned the raw descriptor – the stream closes
/// itself when dropped and is close-on-exec, plan B104).
pub fn connect_unix(path: &Path, timeout: Duration) -> Result<UnixStream> {
    let len = path.as_os_str().len();
    if len > MAX_SOCKET_PATH {
        return Err(vm_err!("Socket path too long: {}", path.display()));
    }
    let s =
        UnixStream::connect(path).map_err(|e| vm_err!("Connection to {} failed: {}", path.display(), os_text(&e)))?;
    s.set_read_timeout(non_zero(timeout))
        .map_err(|e| vm_err!("Connection to {} failed: {}", path.display(), os_text(&e)))?;
    Ok(s)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::net::TcpListener;
    use std::os::unix::net::UnixListener;

    #[test]
    fn ports_and_banners() {
        let l = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = l.local_addr().unwrap().port();
        assert!(!Ports::is_free(port));
        assert!(Ports::is_open("127.0.0.1", port, DEFAULT_TIMEOUT));
        // the connection of `is_open` above is waiting first; each one gets the banner
        let t = std::thread::spawn(move || {
            for _ in 0..2 {
                let (mut c, _) = l.accept().unwrap();
                let _ = c.write_all(b"SSH-2.0-OpenSSH_10.0\r\n");
            }
        });
        assert_eq!(Ports::ssh_banner("127.0.0.1", port, DEFAULT_TIMEOUT).as_deref(), Some("SSH-2.0-OpenSSH_10.0"));
        t.join().unwrap();
        assert!(Ports::ssh_banner("not-an-ip", port, DEFAULT_TIMEOUT).is_none());
    }

    #[test]
    fn unix_sockets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("s.sock");
        let _l = UnixListener::bind(&path).unwrap();
        assert!(connect_unix(&path, Duration::from_secs(1)).is_ok());
        let missing = connect_unix(&dir.path().join("none.sock"), Duration::from_secs(1)).unwrap_err();
        assert!(
            missing.message().starts_with("Connection to ") && missing.message().ends_with("No such file or directory")
        );
        let long = dir.path().join("x".repeat(120));
        assert!(connect_unix(&long, Duration::from_secs(1)).unwrap_err().message().starts_with("Socket path too long"));
    }
}
