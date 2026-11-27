// SPDX-License-Identifier: AGPL-3.0-or-later
//! The switch of one network (`omaf network-switch <name>`, spec §11.12): a learning Ethernet switch between the VMs'
//! second cards, plus the switch's own address (`.1`) answering ARP, ping and DHCP. One process per network, single
//! threaded on `poll(2)`; it ends by itself a minute after the last VM went (and when its network is removed).
//!
//! **Ports.** QEMU connects to `qemu.sock` (`-netdev stream`: each frame behind a 4-byte big-endian length, one
//! connection per VM card). omaf-runner sends datagrams to `vz.sock` from a socket bound to a path of its own
//! (`VZFileHandleNetworkDeviceAttachment`: one frame per datagram); the switch answers to that path, so each such path
//! is a port. A frame that cannot be passed on at once is dropped – Ethernet may lose frames, TCP sends again.
//!
//! **Status.** Every two seconds `status.json` says which ports are connected, which VM (by its card's MAC) is
//! behind each, its traffic, and how much went between which two VMs – the network view in the app reads it.

use std::collections::{BTreeMap, HashMap};
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::packets::{self, BROADCAST, DhcpAnswer, mac_text};
use super::{Networks, parse_member_mac};
use crate::error::{IoContext, Result};
use crate::file_format::write_atomic;
use crate::paths::Paths;
use crate::vm_err;
use omaflake_sdk::networks::Mac;
use omaflake_sdk::networks::Network;
use omaflake_sdk::time::Date;

// In omaflake-sdk (plan B192): part of the API's page models
use omaflake_sdk::networks::{Flow, PortStatus, SwitchStatus};

/// Largest frame accepted (QEMU's stream protocol allows more; jumbo frames are not configured anywhere)
const MAX_FRAME: usize = 65_535;
/// A stream port's queue beyond which frames to it are dropped
const MAX_QUEUE: usize = 4 << 20;

enum Endpoint {
    Stream { stream: UnixStream, input: Vec<u8>, output: Vec<u8> },
    Datagram { peer: Peer },
}

/// A datagram sender's address exactly as the kernel gave it. std's `SocketAddr` is not used: on macOS it takes the
/// last byte of the path for a terminating NUL, and a sender that bound its path without one (Python, some C code)
/// came out a character short – the answers went nowhere.
#[derive(Clone, PartialEq, Eq)]
struct Peer {
    raw: Vec<u8>,
}

impl Peer {
    const PATH_OFFSET: usize = std::mem::offset_of!(libc::sockaddr_un, sun_path);

    /// The path up to its first NUL (None: an unbound sender)
    fn path(&self) -> Option<PathBuf> {
        use std::os::unix::ffi::OsStrExt;
        let bytes = self.raw.get(Self::PATH_OFFSET..)?;
        let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
        (end > 0).then(|| PathBuf::from(std::ffi::OsStr::from_bytes(&bytes[..end])))
    }
}

/// One datagram and its sender (None when nothing is waiting or on an error)
fn receive_from(socket: &UnixDatagram, buf: &mut [u8]) -> Option<(usize, Peer)> {
    // SAFETY: an all-zero sockaddr_un is valid; recvfrom writes at most `len` bytes of address and `buf.len()` of data
    unsafe {
        let mut addr: libc::sockaddr_un = std::mem::zeroed();
        let mut len = std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t;
        let n = libc::recvfrom(
            socket.as_raw_fd(),
            buf.as_mut_ptr().cast(),
            buf.len(),
            0,
            (&mut addr as *mut libc::sockaddr_un).cast(),
            &mut len,
        );
        if n < 0 {
            return None;
        }
        let len = (len as usize).min(std::mem::size_of::<libc::sockaddr_un>());
        let raw = std::slice::from_raw_parts((&addr as *const libc::sockaddr_un).cast::<u8>(), len).to_vec();
        Some((n as usize, Peer { raw }))
    }
}

/// Sends to exactly the address a datagram came from
fn send_to(socket: &UnixDatagram, frame: &[u8], peer: &Peer) -> std::io::Result<usize> {
    // SAFETY: `peer.raw` is a sockaddr_un prefix the kernel wrote (at most its size); `frame` is valid for its length
    let n = unsafe {
        libc::sendto(
            socket.as_raw_fd(),
            frame.as_ptr().cast(),
            frame.len(),
            0,
            peer.raw.as_ptr().cast(),
            peer.raw.len() as libc::socklen_t,
        )
    };
    if n < 0 { Err(std::io::Error::last_os_error()) } else { Ok(n as usize) }
}

struct Port {
    endpoint: Endpoint,
    status: PortStatus,
    mac: Option<Mac>,
    closed: bool,
}

/// How long the switch stays without any port before it ends
pub const IDLE: Duration = Duration::from_secs(60);

pub struct Switch {
    networks: Networks,
    network: Network,
    qemu: UnixListener,
    vz: UnixDatagram,
    ports: Vec<Port>,
    /// MAC → index in `ports`
    table: HashMap<Mac, usize>,
    flows: BTreeMap<(Mac, Mac), u64>,
    started: Date,
    idle: Duration,
}

static STOP: AtomicBool = AtomicBool::new(false);

extern "C" fn on_signal(_: libc::c_int) {
    STOP.store(true, Ordering::SeqCst);
}

impl Switch {
    /// Binds the network's sockets (an older switch's leftovers are replaced; a running one is an error)
    pub fn open(paths: Paths, name: &str) -> Result<Switch> {
        let networks = Networks::new(paths);
        let network = networks.get(name)?;
        if let Some(pid) = networks.switch_pid(name)
            && pid != std::process::id() as i32
        {
            return Err(vm_err!("The switch of network \"{}\" is running already (pid {})", name, pid));
        }
        let dir = networks.run_dir(name);
        std::fs::create_dir_all(&dir).with_path(&dir)?;
        let (qemu_path, vz_path) = (networks.qemu_socket(name), networks.vz_socket(name));
        let _ = std::fs::remove_file(&qemu_path);
        let _ = std::fs::remove_file(&vz_path);
        let qemu = UnixListener::bind(&qemu_path).with_path(&qemu_path)?;
        let vz = UnixDatagram::bind(&vz_path).with_path(&vz_path)?;
        qemu.set_nonblocking(true).with_path(&qemu_path)?;
        vz.set_nonblocking(true).with_path(&vz_path)?;
        // Room for bursts (Apple's advice for VZFileHandleNetworkDeviceAttachment: receive ≥ 4 × send)
        set_buffer(vz.as_raw_fd(), libc::SO_RCVBUF, 4 << 20);
        set_buffer(vz.as_raw_fd(), libc::SO_SNDBUF, 1 << 20);
        write_atomic(&networks.pid_file(name), format!("{}\n", std::process::id()).as_bytes())?;
        Ok(Switch {
            networks,
            network,
            qemu,
            vz,
            ports: vec![],
            table: HashMap::new(),
            flows: BTreeMap::new(),
            started: omaflake_sdk::time::now(),
            idle: IDLE,
        })
    }

    /// The process: runs until SIGTERM/SIGINT, `IDLE` without ports, or the network is removed
    pub fn serve(paths: Paths, name: &str) -> Result<()> {
        let mut switch = Switch::open(paths, name)?;
        // SAFETY: the handler only stores into an atomic
        unsafe {
            libc::signal(libc::SIGTERM, on_signal as *const () as libc::sighandler_t);
            libc::signal(libc::SIGINT, on_signal as *const () as libc::sighandler_t);
            libc::signal(libc::SIGPIPE, libc::SIG_IGN);
        }
        eprintln!("network {}: switch at {} ({})", name, switch.network.switch_address(), switch.network.subnet());
        let result = switch.run(&STOP);
        switch.close();
        result
    }

    pub fn with_idle(mut self, idle: Duration) -> Switch {
        self.idle = idle;
        self
    }

    /// Removes the sockets, the pid file and the status
    pub fn close(&self) {
        let name = &self.network.name;
        for f in [
            self.networks.qemu_socket(name),
            self.networks.vz_socket(name),
            self.networks.pid_file(name),
            self.networks.status_file(name),
        ] {
            let _ = std::fs::remove_file(f);
        }
    }

    pub fn run(&mut self, stop: &AtomicBool) -> Result<()> {
        let mut last_status = Instant::now() - Duration::from_secs(10);
        let mut last_reload = Instant::now();
        let mut empty_since = Some(Instant::now());
        let mut buf = vec![0u8; MAX_FRAME + 4];
        while !stop.load(Ordering::SeqCst) {
            self.poll_once(&mut buf, 500)?;
            self.ports.retain(|p| !p.closed);
            self.rebuild_table();
            if self.ports.is_empty() {
                let since = *empty_since.get_or_insert_with(Instant::now);
                if since.elapsed() >= self.idle {
                    eprintln!("network {}: no VM for {} s – ending", self.network.name, self.idle.as_secs());
                    break;
                }
            } else {
                empty_since = None;
            }
            if last_reload.elapsed() >= Duration::from_secs(5) {
                last_reload = Instant::now();
                match self.networks.get(&self.network.name) {
                    Ok(n) => self.network = n,
                    Err(_) => {
                        eprintln!("network {}: removed – ending", self.network.name);
                        break;
                    }
                }
            }
            if last_status.elapsed() >= Duration::from_secs(2) {
                last_status = Instant::now();
                let _ = self.write_status();
            }
        }
        Ok(())
    }

    fn poll_once(&mut self, buf: &mut [u8], timeout_ms: i32) -> Result<()> {
        let mut fds = vec![
            libc::pollfd { fd: self.qemu.as_raw_fd(), events: libc::POLLIN, revents: 0 },
            libc::pollfd { fd: self.vz.as_raw_fd(), events: libc::POLLIN, revents: 0 },
        ];
        let mut stream_ports = vec![];
        for (i, p) in self.ports.iter().enumerate() {
            if let Endpoint::Stream { stream, output, .. } = &p.endpoint {
                let events = libc::POLLIN | if output.is_empty() { 0 } else { libc::POLLOUT };
                fds.push(libc::pollfd { fd: stream.as_raw_fd(), events, revents: 0 });
                stream_ports.push(i);
            }
        }
        // SAFETY: `fds` is a valid array of pollfd for its length
        let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, timeout_ms) };
        if n < 0 {
            let e = std::io::Error::last_os_error();
            if e.kind() == ErrorKind::Interrupted {
                return Ok(());
            }
            return Err(vm_err!("poll: {}", crate::error::os_text(&e)));
        }
        if fds[0].revents & libc::POLLIN != 0 {
            self.accept();
        }
        if fds[1].revents & libc::POLLIN != 0 {
            self.receive_datagrams(buf);
        }
        for (k, i) in stream_ports.into_iter().enumerate() {
            let revents = fds[2 + k].revents;
            if revents & libc::POLLOUT != 0 {
                self.flush(i);
            }
            if revents & (libc::POLLIN | libc::POLLHUP | libc::POLLERR) != 0 {
                self.read_stream(i, buf);
            }
        }
        Ok(())
    }

    fn accept(&mut self) {
        loop {
            match self.qemu.accept() {
                Ok((stream, _)) => {
                    if stream.set_nonblocking(true).is_err() {
                        continue;
                    }
                    self.ports.push(Port {
                        endpoint: Endpoint::Stream { stream, input: vec![], output: vec![] },
                        status: PortStatus { kind: "qemu".into(), ..Default::default() },
                        mac: None,
                        closed: false,
                    });
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(_) => break,
            }
        }
    }

    fn receive_datagrams(&mut self, buf: &mut [u8]) {
        while let Some((len, peer)) = receive_from(&self.vz, buf) {
            // An unbound sender cannot be answered – not a port
            if peer.path().is_none() {
                continue;
            }
            let i = match self
                .ports
                .iter()
                .position(|p| matches!(&p.endpoint, Endpoint::Datagram { peer: q } if *q == peer) && !p.closed)
            {
                Some(i) => i,
                None => {
                    self.ports.push(Port {
                        endpoint: Endpoint::Datagram { peer },
                        status: PortStatus { kind: "vz".into(), ..Default::default() },
                        mac: None,
                        closed: false,
                    });
                    self.ports.len() - 1
                }
            };
            let frame = buf[..len].to_vec();
            self.handle(i, &frame);
        }
    }

    fn read_stream(&mut self, i: usize, buf: &mut [u8]) {
        let mut frames = vec![];
        {
            let Endpoint::Stream { stream, input, .. } = &mut self.ports[i].endpoint else { return };
            loop {
                match stream.read(buf) {
                    Ok(0) => {
                        self.ports[i].closed = true;
                        break;
                    }
                    Ok(n) => input.extend_from_slice(&buf[..n]),
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                    Err(_) => {
                        self.ports[i].closed = true;
                        break;
                    }
                }
            }
            let Endpoint::Stream { input, .. } = &mut self.ports[i].endpoint else { return };
            let mut at = 0;
            while input.len() - at >= 4 {
                let len = u32::from_be_bytes([input[at], input[at + 1], input[at + 2], input[at + 3]]) as usize;
                if len > MAX_FRAME {
                    self.ports[i].closed = true; // out of step: nothing sensible follows
                    return;
                }
                if input.len() - at - 4 < len {
                    break;
                }
                frames.push(input[at + 4..at + 4 + len].to_vec());
                at += 4 + len;
            }
            input.drain(..at);
        }
        for f in frames {
            self.handle(i, &f);
        }
    }

    fn flush(&mut self, i: usize) {
        let Endpoint::Stream { stream, output, .. } = &mut self.ports[i].endpoint else { return };
        while !output.is_empty() {
            match stream.write(output) {
                Ok(0) => break,
                Ok(n) => {
                    output.drain(..n);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                Err(e) if e.kind() == ErrorKind::Interrupted => continue,
                Err(_) => {
                    self.ports[i].closed = true;
                    break;
                }
            }
        }
    }

    fn send(&mut self, i: usize, frame: &[u8]) {
        let port = &mut self.ports[i];
        if port.closed {
            return;
        }
        let sent = match &mut port.endpoint {
            Endpoint::Stream { output, .. } => {
                if output.len() + frame.len() + 4 > MAX_QUEUE {
                    false
                } else {
                    output.extend_from_slice(&(frame.len() as u32).to_be_bytes());
                    output.extend_from_slice(frame);
                    true
                }
            }
            Endpoint::Datagram { peer } => match send_to(&self.vz, frame, peer) {
                Ok(_) => true,
                Err(e) => {
                    // Full: drop the frame; gone (the runner ended): drop the port
                    let gone = matches!(e.raw_os_error(), Some(libc::ENOENT) | Some(libc::ECONNREFUSED));
                    if gone {
                        port.closed = true;
                        let path = peer.path().unwrap_or_default();
                        eprintln!("network {}: {} is gone ({})", self.network.name, path.display(), e);
                    }
                    false
                }
            },
        };
        if sent {
            port.status.tx_frames += 1;
            port.status.tx_bytes += frame.len() as u64;
        } else {
            port.status.dropped += 1;
        }
        if matches!(self.ports[i].endpoint, Endpoint::Stream { .. }) {
            self.flush(i);
        }
    }

    /// A frame from port `from`
    fn handle(&mut self, from: usize, frame: &[u8]) {
        let Some((dst, src, _)) = packets::ethernet(frame) else { return };
        {
            let p = &mut self.ports[from];
            p.status.rx_frames += 1;
            p.status.rx_bytes += frame.len() as u64;
            if !packets::is_group(&src) && p.mac != Some(src) {
                p.mac = Some(src);
                p.status.mac = Some(mac_text(&src));
            }
        }
        if !packets::is_group(&src) {
            self.table.insert(src, from);
        }
        let own = self.network.switch_mac();
        // Names of the network's VMs (plan B179): "<vm>.<network>.local" over mDNS – the query still goes to all
        if dst == packets::MDNS_MAC
            && let Some(reply) = self.mdns(frame)
        {
            for i in 0..self.ports.len() {
                self.send(i, &reply);
            }
        }
        if dst == BROADCAST || dst == own {
            if let Some(reply) = self.answer(frame) {
                self.send(from, &reply);
            }
            if dst == own {
                return;
            }
        }
        if packets::is_group(&dst) {
            for i in 0..self.ports.len() {
                if i != from {
                    self.send(i, frame);
                }
            }
            return;
        }
        match self.table.get(&dst).copied() {
            Some(to) if to == from => {}
            Some(to) => {
                *self.flows.entry((src, dst)).or_insert(0) += frame.len() as u64;
                self.send(to, frame);
            }
            None => {
                for i in 0..self.ports.len() {
                    if i != from {
                        self.send(i, frame);
                    }
                }
            }
        }
    }

    /// What the switch's own address says to a frame: ARP, ping, DHCP
    fn answer(&self, frame: &[u8]) -> Option<Vec<u8>> {
        let (ip, mac) = (self.network.switch_address(), self.network.switch_mac());
        if let Some(r) = packets::arp_reply(frame, ip, mac) {
            return Some(r);
        }
        if let Some(r) = packets::echo_reply(frame, ip, mac) {
            return Some(r);
        }
        let m = packets::dhcp_message(frame)?;
        // Only cards of this network get an address – which follows from the MAC
        let (number, host) = parse_member_mac(&m.chaddr)?;
        if number != self.network.number || host < super::FIRST_HOST {
            return None;
        }
        let client = self.network.address(host);
        let reply = packets::dhcp_reply(&m, ip, client)?;
        let answer = DhcpAnswer {
            server_ip: ip,
            server_mac: mac,
            client_ip: client,
            mask: std::net::Ipv4Addr::new(255, 255, 255, 0),
            broadcast: self.network.address(255),
        };
        Some(answer.frame(&m, reply))
    }

    /// The switch's answer to an mDNS query for "<vm>.<network>.local" of one of the network's VMs (plan B179)
    fn mdns(&self, frame: &[u8]) -> Option<Vec<u8>> {
        let q = packets::mdns_query(frame)?;
        let suffix = format!(".{}.local", self.network.name.to_lowercase());
        let answers: Vec<(String, std::net::Ipv4Addr)> = q
            .questions
            .iter()
            .filter(|(_, kind)| packets::asks_address(*kind))
            .filter_map(|(name, _)| {
                let vm = name.strip_suffix(&suffix)?;
                let (_, host) = self.network.members.iter().find(|(m, _)| m.to_lowercase() == vm)?;
                Some((name.clone(), self.network.address(*host)))
            })
            .collect();
        if answers.is_empty() {
            return None;
        }
        Some(packets::mdns_answer(&q, &answers, self.network.switch_address(), self.network.switch_mac()))
    }

    /// Indices shift when a port goes: the table is rebuilt from the ports' own MACs
    fn rebuild_table(&mut self) {
        self.table = self.ports.iter().enumerate().filter_map(|(i, p)| p.mac.map(|m| (m, i))).collect();
    }

    fn member(&self, mac: &Mac) -> Option<String> {
        let (number, host) = parse_member_mac(mac)?;
        if number != self.network.number {
            return None;
        }
        self.network.members.iter().find(|(_, h)| **h == host).map(|(vm, _)| vm.clone())
    }

    pub fn status(&self) -> SwitchStatus {
        let ports = self
            .ports
            .iter()
            .map(|p| {
                let mut s = p.status.clone();
                if let Some(mac) = p.mac {
                    s.vm = self.member(&mac);
                    s.address = parse_member_mac(&mac)
                        .filter(|(n, _)| *n == self.network.number)
                        .map(|(_, h)| self.network.address(h).to_string());
                }
                s
            })
            .collect();
        let flows = self
            .flows
            .iter()
            .map(|((a, b), bytes)| Flow {
                from: self.member(a).unwrap_or_else(|| mac_text(a)),
                to: self.member(b).unwrap_or_else(|| mac_text(b)),
                bytes: *bytes,
            })
            .collect();
        SwitchStatus {
            network: self.network.name.clone(),
            pid: std::process::id() as i32,
            started: self.started,
            updated: omaflake_sdk::time::now(),
            ports,
            flows,
        }
    }

    fn write_status(&self) -> Result<()> {
        let data = serde_json::to_vec_pretty(&self.status())?;
        write_atomic(&self.networks.status_file(&self.network.name), &data)
    }
}

fn set_buffer(fd: libc::c_int, option: libc::c_int, size: libc::c_int) {
    // SAFETY: setsockopt on a socket this process owns, with a c_int value of the right size
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            option,
            (&size as *const libc::c_int).cast(),
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::super::packets::tests::dhcp_frame;
    use super::*;
    use std::net::Ipv4Addr;
    use std::path::Path;
    use std::sync::Arc;

    /// A VM's QEMU card: frames behind a 4-byte length
    struct QemuCard(UnixStream, Vec<u8>);

    impl QemuCard {
        fn connect(path: &Path) -> QemuCard {
            let s = UnixStream::connect(path).unwrap();
            s.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            QemuCard(s, vec![])
        }
        fn send(&mut self, f: &[u8]) {
            self.0.write_all(&(f.len() as u32).to_be_bytes()).unwrap();
            self.0.write_all(f).unwrap();
        }
        fn frames(&mut self, wait: Duration) -> Vec<Vec<u8>> {
            let end = Instant::now() + wait;
            let mut buf = [0u8; 4096];
            while Instant::now() < end {
                if let Ok(n) = self.0.read(&mut buf) {
                    self.1.extend_from_slice(&buf[..n]);
                }
            }
            let mut out = vec![];
            while self.1.len() >= 4 {
                let len = u32::from_be_bytes([self.1[0], self.1[1], self.1[2], self.1[3]]) as usize;
                if self.1.len() < 4 + len {
                    break;
                }
                out.push(self.1[4..4 + len].to_vec());
                self.1.drain(..4 + len);
            }
            out
        }
    }

    /// A macOS VM's card as omaf-runner opens it: a datagram socket bound to its own path
    struct VzCard(UnixDatagram);

    impl VzCard {
        fn connect(own: &Path, switch: &Path) -> VzCard {
            let s = UnixDatagram::bind(own).unwrap();
            s.connect(switch).unwrap();
            s.set_read_timeout(Some(Duration::from_millis(100))).unwrap();
            VzCard(s)
        }
        fn send(&self, f: &[u8]) {
            self.0.send(f).unwrap();
        }
        fn frames(&self, wait: Duration) -> Vec<Vec<u8>> {
            let end = Instant::now() + wait;
            let mut out = vec![];
            let mut buf = [0u8; 4096];
            while Instant::now() < end {
                if let Ok(n) = self.0.recv(&mut buf) {
                    out.push(buf[..n].to_vec());
                }
            }
            out
        }
    }

    fn raw(dst: Mac, src: Mac, text: &str) -> Vec<u8> {
        let mut f = dst.to_vec();
        f.extend_from_slice(&src);
        f.extend_from_slice(&[0x88, 0xb5]);
        f.extend_from_slice(text.as_bytes());
        f.resize(60, 0);
        f
    }

    fn text_of(f: &[u8]) -> String {
        String::from_utf8_lossy(&f[14..]).trim_end_matches('\0').to_string()
    }

    fn offered(frames: &[Vec<u8>], mac: Mac) -> Option<Ipv4Addr> {
        frames.iter().find_map(|f| {
            let (_, _, p, udp) = packets::ipv4(f)?;
            (p == 17 && udp.len() > 8 + 34 && udp[8 + 28..8 + 34] == mac)
                .then(|| Ipv4Addr::new(udp[8 + 16], udp[8 + 17], udp[8 + 18], udp[8 + 19]))
        })
    }

    /// Short socket paths: Unix sockets allow ~104 bytes, a test's temporary folder can be long
    fn short_dir() -> tempfile::TempDir {
        tempfile::Builder::new().prefix("omn").tempdir_in("/tmp").unwrap()
    }

    #[test]
    fn qemu_and_macos_cards_on_one_segment() {
        let dir = short_dir();
        let paths = Paths::new(dir.path(), None);
        let networks = Networks::new(paths.clone());
        networks.create("dev").unwrap();
        let docker = networks.join("dev", "docker").unwrap();
        networks.join("dev", "mac1").unwrap();
        networks.join("dev", "mac2").unwrap();
        let net = networks.get("dev").unwrap();
        let (m_docker, m_mac1, m_mac2) = (net.mac(10), net.mac(11), net.mac(12));

        let stop = Arc::new(AtomicBool::new(false));
        let mut switch = Switch::open(paths.clone(), "dev").unwrap();
        let s2 = stop.clone();
        let handle = std::thread::spawn(move || {
            switch.run(&s2).unwrap();
            switch.close();
        });

        let mut q = QemuCard::connect(&networks.qemu_socket("dev"));
        let v1 = VzCard::connect(&dir.path().join("v1.sock"), &networks.vz_socket("dev"));
        let v2 = VzCard::connect(&dir.path().join("v2.sock"), &networks.vz_socket("dev"));
        let wait = Duration::from_millis(300);

        // DHCP: every card gets the address of its MAC
        q.send(&dhcp_frame(m_docker, 1, None, None));
        assert_eq!(offered(&q.frames(wait), m_docker), Some(docker));
        v1.send(&dhcp_frame(m_mac1, 1, None, None));
        assert_eq!(offered(&v1.frames(wait), m_mac1), Some(Ipv4Addr::new(10, 231, 1, 11)));
        v2.send(&dhcp_frame(m_mac2, 3, Some(Ipv4Addr::new(10, 231, 1, 12)), None));
        assert_eq!(offered(&v2.frames(wait), m_mac2), Some(Ipv4Addr::new(10, 231, 1, 12)));
        // A card that is not of this network gets nothing
        let stranger = [0x02, 0x6f, 0x6d, 9, 10, 0];
        q.send(&dhcp_frame(stranger, 1, None, None));
        assert_eq!(offered(&q.frames(wait), stranger), None);
        let _ = (v1.frames(wait), v2.frames(wait)); // the broadcasts

        // Unicast across hypervisors, both ways, and between two macOS cards
        q.send(&raw(m_mac1, m_docker, "docker to mac1"));
        assert_eq!(v1.frames(wait).iter().map(|f| text_of(f)).collect::<Vec<_>>(), ["docker to mac1"]);
        assert!(v2.frames(Duration::from_millis(100)).is_empty(), "a known MAC goes to its port only");
        v1.send(&raw(m_docker, m_mac1, "mac1 to docker"));
        assert_eq!(q.frames(wait).iter().map(|f| text_of(f)).collect::<Vec<_>>(), ["mac1 to docker"]);
        v2.send(&raw(m_mac1, m_mac2, "mac2 to mac1"));
        assert_eq!(v1.frames(wait).iter().map(|f| text_of(f)).collect::<Vec<_>>(), ["mac2 to mac1"]);
        // Broadcast reaches everyone else
        v1.send(&raw(BROADCAST, m_mac1, "all"));
        assert_eq!(q.frames(wait).len(), 1);
        assert_eq!(v2.frames(wait).len(), 1);

        // A name over mDNS (plan B179): mac1 asks for "Docker.dev.local", the switch answers with docker's address
        let mut dns = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        dns.extend_from_slice(&packets::dns_name_bytes("Docker.dev.local"));
        dns.extend_from_slice(&[0, 1, 0, 1]);
        let mut udp = vec![0x14, 0xe9, 0x14, 0xe9];
        udp.extend_from_slice(&((8 + dns.len()) as u16).to_be_bytes());
        udp.extend_from_slice(&[0, 0]);
        udp.extend_from_slice(&dns);
        let mut query = packets::ethernet_header(packets::MDNS_MAC, m_mac1, packets::ETHERTYPE_IPV4);
        query.extend_from_slice(&packets::ipv4_header_ttl(
            Ipv4Addr::new(10, 231, 1, 11),
            packets::MDNS_GROUP,
            17,
            udp.len(),
            255,
        ));
        query.extend_from_slice(&udp);
        v1.send(&query);
        let answer = v1.frames(wait).into_iter().find(|f| f[6..12] == net.switch_mac()).expect("an answer");
        assert_eq!(&answer[answer.len() - 4..], &[10, 231, 1, 10]);
        // The others see the query and the answer; a name of no member gets none
        assert_eq!(q.frames(wait).len(), 2);
        let _ = v2.frames(wait);
        let mut other = query.clone();
        let at = 14 + 20 + 8 + 12 + 1;
        other[at..at + 6].copy_from_slice(b"nobody");
        v1.send(&other);
        assert!(v1.frames(Duration::from_millis(200)).is_empty());
        let _ = (q.frames(wait), v2.frames(wait));

        std::thread::sleep(Duration::from_millis(2100));
        // The switch runs in this test's process – its pid file names the test binary
        let status: SwitchStatus =
            serde_json::from_slice(&std::fs::read(networks.status_file("dev")).unwrap()).unwrap();
        assert_eq!(status.ports.len(), 3);
        let docker_port = status.ports.iter().find(|p| p.vm.as_deref() == Some("docker")).unwrap();
        assert_eq!((docker_port.kind.as_str(), docker_port.address.as_deref()), ("qemu", Some("10.231.1.10")));
        assert!(status.flows.iter().any(|f| f.from == "docker" && f.to == "mac1" && f.bytes == 60));

        // A QEMU card that goes is a port less
        drop(q);
        std::thread::sleep(Duration::from_millis(700));
        stop.store(true, Ordering::SeqCst);
        handle.join().unwrap();
        assert!(!networks.qemu_socket("dev").exists() && !networks.pid_file("dev").exists());
    }

    /// A sender that bound its path without a terminating NUL in the address length (as Python does)
    fn bind_without_nul(path: &Path) -> UnixDatagram {
        use std::os::fd::FromRawFd;
        use std::os::unix::ffi::OsStrExt;
        let bytes = path.as_os_str().as_bytes();
        // SAFETY: a fresh socket; the address is a zeroed sockaddr_un with the path copied in (shorter than sun_path)
        unsafe {
            let fd = libc::socket(libc::AF_UNIX, libc::SOCK_DGRAM, 0);
            let mut addr: libc::sockaddr_un = std::mem::zeroed();
            addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
            for (i, b) in bytes.iter().enumerate() {
                addr.sun_path[i] = *b as libc::c_char;
            }
            let len = std::mem::offset_of!(libc::sockaddr_un, sun_path) + bytes.len();
            assert_eq!(libc::bind(fd, (&addr as *const libc::sockaddr_un).cast(), len as libc::socklen_t), 0);
            UnixDatagram::from_raw_fd(fd)
        }
    }

    #[test]
    fn answers_reach_a_sender_bound_without_nul() {
        let dir = short_dir();
        let paths = Paths::new(dir.path(), None);
        let networks = Networks::new(paths.clone());
        networks.create("nul").unwrap();
        let mac = networks.get("nul").unwrap().mac(10);
        let stop = Arc::new(AtomicBool::new(false));
        let mut switch = Switch::open(paths, "nul").unwrap();
        let s2 = stop.clone();
        let handle = std::thread::spawn(move || {
            switch.run(&s2).unwrap();
            switch.close();
        });
        let card = bind_without_nul(&dir.path().join("lan0.sock"));
        card.connect(networks.vz_socket("nul")).unwrap();
        card.set_read_timeout(Some(Duration::from_millis(500))).unwrap();
        card.send(&dhcp_frame(mac, 1, None, None)).unwrap();
        let mut buf = [0u8; 2048];
        let n = card.recv(&mut buf).expect("the offer reaches lan0.sock, not lan0.soc");
        assert_eq!(offered(&[buf[..n].to_vec()], mac), Some(Ipv4Addr::new(10, 231, 1, 10)));
        stop.store(true, Ordering::SeqCst);
        handle.join().unwrap();
    }

    #[test]
    fn ends_when_idle_or_removed() {
        let dir = short_dir();
        let paths = Paths::new(dir.path(), None);
        let networks = Networks::new(paths.clone());
        networks.create("idle").unwrap();
        let mut switch = Switch::open(paths.clone(), "idle").unwrap().with_idle(Duration::from_millis(300));
        let start = Instant::now();
        switch.run(&AtomicBool::new(false)).unwrap();
        assert!(start.elapsed() < Duration::from_secs(3));
        switch.close();

        networks.create("gone").unwrap();
        let mut switch = Switch::open(paths.clone(), "gone").unwrap();
        let _card = QemuCard::connect(&networks.qemu_socket("gone"));
        let n2 = networks.clone();
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(200));
            let mut all = n2.list().unwrap();
            all.retain(|n| n.name != "gone");
            n2.save(all).unwrap();
        });
        let start = Instant::now();
        switch.run(&AtomicBool::new(false)).unwrap();
        assert!(start.elapsed() < Duration::from_secs(8), "ends within its 5 s reload");
        switch.close();
    }
}
