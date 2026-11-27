// SPDX-License-Identifier: AGPL-3.0-or-later
//! The few packets a network's switch answers itself (spec §11.12): ARP and ping for its own address, and DHCP – each
//! VM's address follows from its MAC (`Addressing`), so the switch keeps no lease state. Pure functions on Ethernet
//! frames; no IPv6, no IP options, no fragments (none of these are needed for ARP, DHCP and ping).

use std::net::Ipv4Addr;

// In omaflake-sdk (plan B192): part of the API's page models
pub use omaflake_sdk::networks::Mac;

pub const ETHERTYPE_IPV4: u16 = 0x0800;
pub const ETHERTYPE_ARP: u16 = 0x0806;
pub const BROADCAST: [u8; 6] = [0xff; 6];

const IPPROTO_ICMP: u8 = 1;
const IPPROTO_UDP: u8 = 17;
const DHCP_SERVER_PORT: u16 = 67;
const DHCP_CLIENT_PORT: u16 = 68;
const DHCP_MAGIC: [u8; 4] = [99, 130, 83, 99];
/// A day; a VM's address never changes, so renewing is only a formality
pub const LEASE_SECONDS: u32 = 86_400;

pub fn mac_text(m: &Mac) -> String {
    m.iter().map(|b| format!("{b:02x}")).collect::<Vec<_>>().join(":")
}

pub fn parse_mac(text: &str) -> Option<Mac> {
    let parts: Vec<&str> = text.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut m = [0u8; 6];
    for (i, p) in parts.iter().enumerate() {
        if p.len() != 2 {
            return None;
        }
        m[i] = u8::from_str_radix(p, 16).ok()?;
    }
    Some(m)
}

/// Destination, source and EtherType of an Ethernet II frame
pub fn ethernet(frame: &[u8]) -> Option<(Mac, Mac, u16)> {
    if frame.len() < 14 {
        return None;
    }
    let mut dst = [0u8; 6];
    let mut src = [0u8; 6];
    dst.copy_from_slice(&frame[0..6]);
    src.copy_from_slice(&frame[6..12]);
    Some((dst, src, u16::from_be_bytes([frame[12], frame[13]])))
}

/// Broadcast or multicast (the group bit)
pub fn is_group(mac: &Mac) -> bool {
    mac[0] & 1 == 1
}

pub(crate) fn ethernet_header(dst: Mac, src: Mac, ethertype: u16) -> Vec<u8> {
    let mut f = Vec::with_capacity(342);
    f.extend_from_slice(&dst);
    f.extend_from_slice(&src);
    f.extend_from_slice(&ethertype.to_be_bytes());
    f
}

/// The Internet checksum (RFC 1071)
pub fn checksum(data: &[u8]) -> u16 {
    let mut sum: u32 = 0;
    for chunk in data.chunks(2) {
        let word = if chunk.len() == 2 { u16::from_be_bytes([chunk[0], chunk[1]]) } else { u16::from(chunk[0]) << 8 };
        sum += u32::from(word);
    }
    while sum > 0xffff {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

fn ipv4_header(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8, payload_len: usize) -> [u8; 20] {
    ipv4_header_ttl(src, dst, protocol, payload_len, 64)
}

pub(crate) fn ipv4_header_ttl(src: Ipv4Addr, dst: Ipv4Addr, protocol: u8, payload_len: usize, ttl: u8) -> [u8; 20] {
    let total = (20 + payload_len) as u16;
    let mut h = [0u8; 20];
    h[0] = 0x45;
    h[2..4].copy_from_slice(&total.to_be_bytes());
    h[8] = ttl;
    h[9] = protocol;
    h[12..16].copy_from_slice(&src.octets());
    h[16..20].copy_from_slice(&dst.octets());
    let sum = checksum(&h);
    h[10..12].copy_from_slice(&sum.to_be_bytes());
    h
}

/// An IPv4 packet in a frame: (source, destination, protocol, payload) – None for options-free parsing failures and
/// fragments
pub fn ipv4(frame: &[u8]) -> Option<(Ipv4Addr, Ipv4Addr, u8, &[u8])> {
    let p = frame.get(14..)?;
    if p.len() < 20 || p[0] >> 4 != 4 {
        return None;
    }
    let ihl = usize::from(p[0] & 0x0f) * 4;
    let total = usize::from(u16::from_be_bytes([p[2], p[3]]));
    let fragment = u16::from_be_bytes([p[6], p[7]]) & 0x3fff;
    if ihl < 20 || total < ihl || total > p.len() || fragment != 0 {
        return None;
    }
    let src = Ipv4Addr::new(p[12], p[13], p[14], p[15]);
    let dst = Ipv4Addr::new(p[16], p[17], p[18], p[19]);
    Some((src, dst, p[9], &p[ihl..total]))
}

// MARK: ARP

/// An ARP request for `ip` → the reply from `mac` (None when it is not a request for `ip`)
pub fn arp_reply(frame: &[u8], ip: Ipv4Addr, mac: Mac) -> Option<Vec<u8>> {
    let (_, _, ethertype) = ethernet(frame)?;
    let a = frame.get(14..42)?;
    // Ethernet/IPv4, request
    if ethertype != ETHERTYPE_ARP || a[0..8] != [0, 1, 8, 0, 6, 4, 0, 1] || a[24..28] != ip.octets() {
        return None;
    }
    let mut sender_mac = [0u8; 6];
    sender_mac.copy_from_slice(&a[8..14]);
    let mut f = ethernet_header(sender_mac, mac, ETHERTYPE_ARP);
    f.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 2]);
    f.extend_from_slice(&mac);
    f.extend_from_slice(&ip.octets());
    f.extend_from_slice(&a[8..18]); // the asker's MAC and IP
    Some(f)
}

// MARK: ICMP

/// A ping to `ip` → the answer from `mac`
pub fn echo_reply(frame: &[u8], ip: Ipv4Addr, mac: Mac) -> Option<Vec<u8>> {
    let (_, src_mac, _) = ethernet(frame)?;
    let (src, dst, protocol, payload) = ipv4(frame)?;
    if protocol != IPPROTO_ICMP || dst != ip || payload.len() < 8 || payload[0] != 8 {
        return None;
    }
    let mut icmp = payload.to_vec();
    icmp[0] = 0;
    icmp[2] = 0;
    icmp[3] = 0;
    let sum = checksum(&icmp);
    icmp[2..4].copy_from_slice(&sum.to_be_bytes());
    let mut f = ethernet_header(src_mac, mac, ETHERTYPE_IPV4);
    f.extend_from_slice(&ipv4_header(ip, src, IPPROTO_ICMP, icmp.len()));
    f.extend_from_slice(&icmp);
    Some(f)
}

// MARK: DHCP

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpKind {
    Discover,
    Request,
    Release,
    Inform,
}

/// A client's DHCP message
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DhcpMessage {
    pub kind: DhcpKind,
    pub xid: [u8; 4],
    pub flags: [u8; 2],
    pub ciaddr: Ipv4Addr,
    pub chaddr: Mac,
    /// Option 50
    pub requested: Option<Ipv4Addr>,
    /// Option 54: the server the client chose (a REQUEST meant for another server is not ours)
    pub server: Option<Ipv4Addr>,
}

/// A DHCP message from a client (UDP 68 → 67) in `frame`
pub fn dhcp_message(frame: &[u8]) -> Option<DhcpMessage> {
    let (_, _, protocol, udp) = ipv4(frame)?;
    if protocol != IPPROTO_UDP || udp.len() < 8 {
        return None;
    }
    let (sport, dport) = (u16::from_be_bytes([udp[0], udp[1]]), u16::from_be_bytes([udp[2], udp[3]]));
    if sport != DHCP_CLIENT_PORT || dport != DHCP_SERVER_PORT {
        return None;
    }
    let b = &udp[8..];
    // op BOOTREQUEST, htype Ethernet, hlen 6; options after the 236-byte header and the magic cookie
    if b.len() < 240 || b[0] != 1 || b[1] != 1 || b[2] != 6 || b[236..240] != DHCP_MAGIC {
        return None;
    }
    let mut chaddr = [0u8; 6];
    chaddr.copy_from_slice(&b[28..34]);
    let (mut kind, mut requested, mut server) = (None, None, None);
    let mut i = 240;
    while i < b.len() {
        let code = b[i];
        if code == 0 {
            i += 1;
            continue;
        }
        if code == 255 || i + 1 >= b.len() {
            break;
        }
        let len = usize::from(b[i + 1]);
        let value = b.get(i + 2..i + 2 + len)?;
        match (code, len) {
            (53, 1) => {
                kind = match value[0] {
                    1 => Some(DhcpKind::Discover),
                    3 => Some(DhcpKind::Request),
                    7 => Some(DhcpKind::Release),
                    8 => Some(DhcpKind::Inform),
                    _ => None,
                }
            }
            (50, 4) => requested = Some(Ipv4Addr::new(value[0], value[1], value[2], value[3])),
            (54, 4) => server = Some(Ipv4Addr::new(value[0], value[1], value[2], value[3])),
            _ => {}
        }
        i += 2 + len;
    }
    Some(DhcpMessage {
        kind: kind?,
        xid: [b[4], b[5], b[6], b[7]],
        flags: [b[10], b[11]],
        ciaddr: Ipv4Addr::new(b[12], b[13], b[14], b[15]),
        chaddr,
        requested,
        server,
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpReply {
    Offer,
    Ack,
    Nak,
}

/// What the switch's DHCP server says to a VM: its address, the subnet mask – and no router and no DNS server: the
/// network is a second card next to the VM's own NAT, which keeps the default route (spec §11.12)
pub struct DhcpAnswer {
    pub server_ip: Ipv4Addr,
    pub server_mac: Mac,
    pub client_ip: Ipv4Addr,
    pub mask: Ipv4Addr,
    pub broadcast: Ipv4Addr,
}

impl DhcpAnswer {
    pub fn frame(&self, m: &DhcpMessage, reply: DhcpReply) -> Vec<u8> {
        let yiaddr = if reply == DhcpReply::Nak { Ipv4Addr::UNSPECIFIED } else { self.client_ip };
        let mut b = vec![0u8; 236];
        b[0] = 2; // BOOTREPLY
        b[1] = 1;
        b[2] = 6;
        b[4..8].copy_from_slice(&m.xid);
        b[10..12].copy_from_slice(&m.flags);
        if reply == DhcpReply::Ack {
            b[12..16].copy_from_slice(&m.ciaddr.octets());
        }
        b[16..20].copy_from_slice(&yiaddr.octets());
        b[20..24].copy_from_slice(&self.server_ip.octets());
        b[28..34].copy_from_slice(&m.chaddr);
        b.extend_from_slice(&DHCP_MAGIC);
        let kind = match reply {
            DhcpReply::Offer => 2,
            DhcpReply::Ack => 5,
            DhcpReply::Nak => 6,
        };
        b.extend_from_slice(&[53, 1, kind]);
        b.extend_from_slice(&[54, 4]);
        b.extend_from_slice(&self.server_ip.octets());
        if reply != DhcpReply::Nak {
            b.extend_from_slice(&[51, 4]);
            b.extend_from_slice(&LEASE_SECONDS.to_be_bytes());
            b.extend_from_slice(&[58, 4]);
            b.extend_from_slice(&(LEASE_SECONDS / 2).to_be_bytes());
            b.extend_from_slice(&[59, 4]);
            b.extend_from_slice(&(LEASE_SECONDS / 8 * 7).to_be_bytes());
            b.extend_from_slice(&[1, 4]);
            b.extend_from_slice(&self.mask.octets());
            b.extend_from_slice(&[28, 4]);
            b.extend_from_slice(&self.broadcast.octets());
        }
        b.push(255);
        while b.len() < 300 {
            b.push(0); // BOOTP's minimum size; some clients drop shorter replies
        }
        // To a client without an address: broadcast (the switch hands it to the asking port anyway)
        let unicast = reply == DhcpReply::Ack && !m.ciaddr.is_unspecified();
        let (dst_ip, dst_mac) = if unicast { (m.ciaddr, m.chaddr) } else { (Ipv4Addr::BROADCAST, BROADCAST) };
        let mut udp = Vec::with_capacity(8 + b.len());
        udp.extend_from_slice(&DHCP_SERVER_PORT.to_be_bytes());
        udp.extend_from_slice(&DHCP_CLIENT_PORT.to_be_bytes());
        udp.extend_from_slice(&((8 + b.len()) as u16).to_be_bytes());
        udp.extend_from_slice(&[0, 0]); // no UDP checksum (allowed over IPv4)
        udp.extend_from_slice(&b);
        let mut f = ethernet_header(dst_mac, self.server_mac, ETHERTYPE_IPV4);
        f.extend_from_slice(&ipv4_header(self.server_ip, dst_ip, IPPROTO_UDP, udp.len()));
        f.extend_from_slice(&udp);
        f
    }
}

/// The reply to a client's message: DISCOVER → OFFER; REQUEST for its address (or none, a renewal) → ACK, for another
/// address → NAK, meant for another server → nothing; RELEASE and INFORM → nothing
pub fn dhcp_reply(m: &DhcpMessage, server: Ipv4Addr, client: Ipv4Addr) -> Option<DhcpReply> {
    match m.kind {
        DhcpKind::Discover => Some(DhcpReply::Offer),
        DhcpKind::Request => {
            if m.server.is_some_and(|s| s != server) {
                return None;
            }
            let asked = m.requested.unwrap_or(m.ciaddr);
            if asked == client || asked.is_unspecified() { Some(DhcpReply::Ack) } else { Some(DhcpReply::Nak) }
        }
        DhcpKind::Release | DhcpKind::Inform => None,
    }
}

// MARK: mDNS (plan B179)

/// mDNS (RFC 6762): the group 224.0.0.251, its Ethernet address and port 5353
pub const MDNS_GROUP: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);
pub const MDNS_MAC: Mac = [0x01, 0x00, 0x5e, 0x00, 0x00, 0xfb];
pub const MDNS_PORT: u16 = 5353;
const DNS_TYPE_A: u16 = 1;
const DNS_TYPE_ANY: u16 = 255;

/// The questions of an mDNS query: each name (lowercase, without the final dot) and its type
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdnsQuery {
    pub id: u16,
    pub questions: Vec<(String, u16)>,
    pub src_ip: Ipv4Addr,
    pub src_port: u16,
    pub src_mac: Mac,
}

/// The name at `at` of a DNS message (labels, compression pointers) and where it ends; None for a broken one
fn dns_name(msg: &[u8], mut at: usize) -> Option<(String, usize)> {
    let mut labels: Vec<String> = Vec::new();
    let mut end = None;
    for _ in 0..64 {
        let len = *msg.get(at)? as usize;
        if len == 0 {
            return Some((labels.join("."), end.unwrap_or(at + 1)));
        }
        if len & 0xc0 == 0xc0 {
            let to = (len & 0x3f) << 8 | *msg.get(at + 1)? as usize;
            end.get_or_insert(at + 2);
            at = to;
            continue;
        }
        let label = msg.get(at + 1..at + 1 + len)?;
        labels.push(String::from_utf8_lossy(label).to_lowercase());
        at += 1 + len;
    }
    None
}

/// An mDNS query in a frame: UDP to port 5353, a query (QR 0) with questions; None for anything else
pub fn mdns_query(frame: &[u8]) -> Option<MdnsQuery> {
    let (_dst, src, ethertype) = ethernet(frame)?;
    if ethertype != ETHERTYPE_IPV4 {
        return None;
    }
    let (src_ip, _dst_ip, protocol, udp) = ipv4(frame)?;
    if protocol != IPPROTO_UDP || udp.len() < 8 + 12 {
        return None;
    }
    let src_port = u16::from_be_bytes([udp[0], udp[1]]);
    if u16::from_be_bytes([udp[2], udp[3]]) != MDNS_PORT {
        return None;
    }
    let msg = &udp[8..];
    if msg[2] & 0x80 != 0 {
        return None; // a response
    }
    let count = u16::from_be_bytes([msg[4], msg[5]]);
    let mut at = 12;
    let mut questions = Vec::new();
    for _ in 0..count {
        let (name, end) = dns_name(msg, at)?;
        let kind = u16::from_be_bytes([*msg.get(end)?, *msg.get(end + 1)?]);
        questions.push((name, kind));
        at = end + 4;
    }
    Some(MdnsQuery { id: u16::from_be_bytes([msg[0], msg[1]]), questions, src_ip, src_port, src_mac: src })
}

/// Whether a question asks for an address (A or ANY)
pub fn asks_address(kind: u16) -> bool {
    kind == DNS_TYPE_A || kind == DNS_TYPE_ANY
}

pub(crate) fn dns_name_bytes(name: &str) -> Vec<u8> {
    let mut b = Vec::with_capacity(name.len() + 2);
    for label in name.split('.').filter(|l| !l.is_empty()) {
        let l = &label.as_bytes()[..label.len().min(63)];
        b.push(l.len() as u8);
        b.extend_from_slice(l);
    }
    b.push(0);
    b
}

/// The switch's answer to `q`: an A record per (name, address), TTL 120. To the group from port 5353 (cache-flush set),
/// or – a legacy query from another port – to the asker with its ID and its questions (RFC 6762 §6.7)
pub fn mdns_answer(q: &MdnsQuery, answers: &[(String, Ipv4Addr)], server_ip: Ipv4Addr, server_mac: Mac) -> Vec<u8> {
    let legacy = q.src_port != MDNS_PORT;
    let mut m = Vec::new();
    m.extend_from_slice(&(if legacy { q.id } else { 0 }).to_be_bytes());
    m.extend_from_slice(&[0x84, 0x00]); // response, authoritative
    let asked: Vec<&(String, u16)> = if legacy { q.questions.iter().collect() } else { vec![] };
    m.extend_from_slice(&(asked.len() as u16).to_be_bytes());
    m.extend_from_slice(&(answers.len() as u16).to_be_bytes());
    m.extend_from_slice(&[0, 0, 0, 0]);
    for (name, kind) in asked {
        m.extend_from_slice(&dns_name_bytes(name));
        m.extend_from_slice(&kind.to_be_bytes());
        m.extend_from_slice(&[0, 1]);
    }
    for (name, ip) in answers {
        m.extend_from_slice(&dns_name_bytes(name));
        m.extend_from_slice(&DNS_TYPE_A.to_be_bytes());
        m.extend_from_slice(&(if legacy { 0x0001u16 } else { 0x8001 }).to_be_bytes());
        m.extend_from_slice(&120u32.to_be_bytes());
        m.extend_from_slice(&[0, 4]);
        m.extend_from_slice(&ip.octets());
    }
    let (dst_ip, dst_port, dst_mac) =
        if legacy { (q.src_ip, q.src_port, q.src_mac) } else { (MDNS_GROUP, MDNS_PORT, MDNS_MAC) };
    let mut udp = Vec::with_capacity(8 + m.len());
    udp.extend_from_slice(&MDNS_PORT.to_be_bytes());
    udp.extend_from_slice(&dst_port.to_be_bytes());
    udp.extend_from_slice(&((8 + m.len()) as u16).to_be_bytes());
    udp.extend_from_slice(&[0, 0]); // no UDP checksum (allowed over IPv4)
    udp.extend_from_slice(&m);
    let mut f = ethernet_header(dst_mac, server_mac, ETHERTYPE_IPV4);
    // mDNS packets carry TTL 255 (RFC 6762 §11)
    f.extend_from_slice(&ipv4_header_ttl(server_ip, dst_ip, IPPROTO_UDP, udp.len(), 255));
    f.extend_from_slice(&udp);
    f
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    pub(crate) const VM: Mac = [0x02, 0x6f, 0x6d, 1, 10, 0];
    const SERVER: Mac = [0x02, 0x6f, 0x6d, 1, 1, 0];

    /// A client's DHCP message as dhclient/dhcpcd/macOS send it
    pub(crate) fn dhcp_frame(mac: Mac, kind: u8, requested: Option<Ipv4Addr>, server: Option<Ipv4Addr>) -> Vec<u8> {
        let mut b = vec![0u8; 236];
        b[0] = 1;
        b[1] = 1;
        b[2] = 6;
        b[4..8].copy_from_slice(&[1, 2, 3, 4]);
        b[10] = 0x80;
        b[28..34].copy_from_slice(&mac);
        b.extend_from_slice(&DHCP_MAGIC);
        b.extend_from_slice(&[53, 1, kind]);
        if let Some(r) = requested {
            b.extend_from_slice(&[50, 4]);
            b.extend_from_slice(&r.octets());
        }
        if let Some(s) = server {
            b.extend_from_slice(&[54, 4]);
            b.extend_from_slice(&s.octets());
        }
        b.extend_from_slice(&[55, 3, 1, 3, 6, 255]);
        let mut udp = Vec::new();
        udp.extend_from_slice(&68u16.to_be_bytes());
        udp.extend_from_slice(&67u16.to_be_bytes());
        udp.extend_from_slice(&((8 + b.len()) as u16).to_be_bytes());
        udp.extend_from_slice(&[0, 0]);
        udp.extend_from_slice(&b);
        let mut f = ethernet_header(BROADCAST, mac, ETHERTYPE_IPV4);
        f.extend_from_slice(&ipv4_header(Ipv4Addr::UNSPECIFIED, Ipv4Addr::BROADCAST, IPPROTO_UDP, udp.len()));
        f.extend_from_slice(&udp);
        f
    }

    fn option(frame: &[u8], code: u8) -> Option<Vec<u8>> {
        let (_, _, _, udp) = ipv4(frame)?;
        let b = &udp[8..];
        let mut i = 240;
        while i + 1 < b.len() && b[i] != 255 {
            if b[i] == 0 {
                i += 1;
                continue;
            }
            let len = usize::from(b[i + 1]);
            if b[i] == code {
                return Some(b[i + 2..i + 2 + len].to_vec());
            }
            i += 2 + len;
        }
        None
    }

    fn answer() -> DhcpAnswer {
        DhcpAnswer {
            server_ip: Ipv4Addr::new(10, 231, 1, 1),
            server_mac: SERVER,
            client_ip: Ipv4Addr::new(10, 231, 1, 10),
            mask: Ipv4Addr::new(255, 255, 255, 0),
            broadcast: Ipv4Addr::new(10, 231, 1, 255),
        }
    }

    #[test]
    fn checksum_of_a_known_header() {
        // RFC 1071's example header from Wikipedia: 4500 0073 0000 4000 4011 [b861] c0a8 0001 c0a8 00c7
        let h = hex("45000073000040004011b861c0a80001c0a800c7");
        assert_eq!(checksum(&h), 0);
        let mut zero = h.clone();
        zero[10] = 0;
        zero[11] = 0;
        assert_eq!(checksum(&zero), 0xb861);
    }

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len()).step_by(2).map(|i| u8::from_str_radix(&s[i..i + 2], 16).unwrap()).collect()
    }

    #[test]
    fn macs_as_text() {
        assert_eq!(mac_text(&VM), "02:6f:6d:01:0a:00");
        assert_eq!(parse_mac("02:6F:6d:01:0a:00"), Some(VM));
        assert_eq!(parse_mac("02:6f:6d:01:0a"), None);
        assert_eq!(parse_mac("02:6f:6d:01:0a:zz"), None);
        assert!(is_group(&BROADCAST) && !is_group(&VM));
    }

    #[test]
    fn discover_gets_an_offer_with_address_and_mask_but_no_router() {
        let m = dhcp_message(&dhcp_frame(VM, 1, None, None)).unwrap();
        assert_eq!((m.kind, m.chaddr, m.xid), (DhcpKind::Discover, VM, [1, 2, 3, 4]));
        let server = Ipv4Addr::new(10, 231, 1, 1);
        let reply = dhcp_reply(&m, server, Ipv4Addr::new(10, 231, 1, 10)).unwrap();
        assert_eq!(reply, DhcpReply::Offer);
        let f = answer().frame(&m, reply);
        let (dst, src, _) = ethernet(&f).unwrap();
        assert_eq!((dst, src), (BROADCAST, SERVER));
        let (ip_src, ip_dst, _, udp) = ipv4(&f).unwrap();
        assert_eq!((ip_src, ip_dst), (server, Ipv4Addr::BROADCAST));
        assert_eq!(checksum(&f[14..34]), 0, "IP header checksum");
        assert_eq!(udp[8 + 16..8 + 20], [10, 231, 1, 10], "yiaddr");
        assert_eq!(option(&f, 53), Some(vec![2]));
        assert_eq!(option(&f, 1), Some(vec![255, 255, 255, 0]));
        assert_eq!(option(&f, 54), Some(vec![10, 231, 1, 1]));
        assert_eq!(option(&f, 3), None, "no router: the VM's NAT keeps the default route");
        assert_eq!(option(&f, 6), None, "no DNS server");
    }

    #[test]
    fn requests_are_acked_for_the_vms_address_only() {
        let (server, client) = (Ipv4Addr::new(10, 231, 1, 1), Ipv4Addr::new(10, 231, 1, 10));
        let ask = |requested, to| dhcp_message(&dhcp_frame(VM, 3, requested, to)).unwrap();
        assert_eq!(dhcp_reply(&ask(Some(client), Some(server)), server, client), Some(DhcpReply::Ack));
        assert_eq!(dhcp_reply(&ask(Some(client), None), server, client), Some(DhcpReply::Ack));
        let other = Ipv4Addr::new(10, 231, 1, 99);
        assert_eq!(dhcp_reply(&ask(Some(other), None), server, client), Some(DhcpReply::Nak));
        let elsewhere = Ipv4Addr::new(192, 168, 64, 1);
        assert_eq!(dhcp_reply(&ask(Some(client), Some(elsewhere)), server, client), None);
        let release = dhcp_message(&dhcp_frame(VM, 7, None, None)).unwrap();
        assert_eq!(dhcp_reply(&release, server, client), None);
        let nak = answer().frame(&ask(Some(other), None), DhcpReply::Nak);
        assert_eq!(option(&nak, 53), Some(vec![6]));
        assert_eq!(option(&nak, 1), None);
    }

    #[test]
    fn not_dhcp() {
        let mut f = dhcp_frame(VM, 1, None, None);
        f[14 + 20 + 2..14 + 20 + 4].copy_from_slice(&53u16.to_be_bytes()); // to port 53
        assert_eq!(dhcp_message(&f), None);
        assert_eq!(dhcp_message(&f[..40]), None);
        assert_eq!(dhcp_message(&[0u8; 10]), None);
    }

    #[test]
    fn arp_for_the_switch_address() {
        let ip = Ipv4Addr::new(10, 231, 1, 1);
        let mut f = ethernet_header(BROADCAST, VM, ETHERTYPE_ARP);
        f.extend_from_slice(&[0, 1, 8, 0, 6, 4, 0, 1]);
        f.extend_from_slice(&VM);
        f.extend_from_slice(&[10, 231, 1, 10]);
        f.extend_from_slice(&[0; 6]);
        f.extend_from_slice(&ip.octets());
        let r = arp_reply(&f, ip, SERVER).unwrap();
        assert_eq!(ethernet(&r).unwrap(), (VM, SERVER, ETHERTYPE_ARP));
        assert_eq!(r[14 + 6..14 + 8], [0, 2], "reply");
        assert_eq!(r[14 + 8..14 + 14], SERVER);
        assert_eq!(r[14 + 14..14 + 18], [10, 231, 1, 1]);
        assert_eq!(r[14 + 18..14 + 24], VM);
        assert_eq!(r[14 + 24..14 + 28], [10, 231, 1, 10]);
        assert_eq!(arp_reply(&f, Ipv4Addr::new(10, 231, 1, 2), SERVER), None, "another address");
    }

    #[test]
    fn ping_to_the_switch_address() {
        let ip = Ipv4Addr::new(10, 231, 1, 1);
        let mut icmp = vec![8, 0, 0, 0, 0x12, 0x34, 0, 1, b'h', b'i'];
        let sum = checksum(&icmp);
        icmp[2..4].copy_from_slice(&sum.to_be_bytes());
        let mut f = ethernet_header(SERVER, VM, ETHERTYPE_IPV4);
        f.extend_from_slice(&ipv4_header(Ipv4Addr::new(10, 231, 1, 10), ip, IPPROTO_ICMP, icmp.len()));
        f.extend_from_slice(&icmp);
        let r = echo_reply(&f, ip, SERVER).unwrap();
        let (src, dst, protocol, payload) = ipv4(&r).unwrap();
        assert_eq!((src, dst, protocol), (ip, Ipv4Addr::new(10, 231, 1, 10), IPPROTO_ICMP));
        assert_eq!(payload[0], 0, "echo reply");
        assert_eq!(checksum(payload), 0, "ICMP checksum");
        assert_eq!(&payload[4..], &icmp[4..]);
        assert_eq!(ethernet(&r).unwrap().0, VM);
    }

    /// An mDNS query for "Docker.lab.local" (A) as macOS sends it, and the switch's answer to the group
    #[test]
    fn mdns_query_and_answer() {
        let vm: Mac = [0x02, 0x6f, 0x6d, 1, 11, 0];
        let mut dns = vec![0, 0, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0];
        dns.extend_from_slice(&super::dns_name_bytes("Docker.lab.local"));
        dns.extend_from_slice(&[0, 1, 0x80, 1]); // A, IN with the unicast-response bit
        let mut udp = vec![0x14, 0xe9, 0x14, 0xe9];
        udp.extend_from_slice(&((8 + dns.len()) as u16).to_be_bytes());
        udp.extend_from_slice(&[0, 0]);
        udp.extend_from_slice(&dns);
        let mut f = super::ethernet_header(MDNS_MAC, vm, ETHERTYPE_IPV4);
        f.extend_from_slice(&super::ipv4_header_ttl(Ipv4Addr::new(10, 231, 1, 11), MDNS_GROUP, 17, udp.len(), 255));
        f.extend_from_slice(&udp);
        let q = mdns_query(&f).unwrap();
        assert_eq!(q.questions, [("docker.lab.local".to_string(), 1)]);
        assert_eq!((q.src_port, q.src_mac), (MDNS_PORT, vm));
        let docker = [("docker.lab.local".to_string(), Ipv4Addr::new(10, 231, 1, 10))];
        let a = mdns_answer(&q, &docker, Ipv4Addr::new(10, 231, 1, 1), SERVER);
        assert_eq!(&a[0..6], &MDNS_MAC);
        let (src, dst, proto, payload) = ipv4(&a).unwrap();
        assert_eq!((src, dst, proto, a[14 + 8]), (Ipv4Addr::new(10, 231, 1, 1), MDNS_GROUP, 17, 255));
        let msg = &payload[8..];
        assert_eq!(&msg[0..12], &[0, 0, 0x84, 0, 0, 0, 0, 1, 0, 0, 0, 0]);
        assert_eq!(&msg[msg.len() - 4..], &[10, 231, 1, 10]);
        assert!(mdns_query(&a).is_none(), "a response is no query");
        // A legacy query (another port): answered to the asker, with its ID and question
        let mut legacy = f.clone();
        let at = 14 + 20;
        legacy[at..at + 2].copy_from_slice(&40000u16.to_be_bytes());
        legacy[at + 8..at + 10].copy_from_slice(&0x1234u16.to_be_bytes());
        let lq = mdns_query(&legacy).unwrap();
        let la = mdns_answer(&lq, &docker, Ipv4Addr::new(10, 231, 1, 1), SERVER);
        assert_eq!(&la[0..6], &vm);
        let (_, dst, _, payload) = ipv4(&la).unwrap();
        assert_eq!((dst, u16::from_be_bytes([payload[2], payload[3]])), (Ipv4Addr::new(10, 231, 1, 11), 40000));
        assert_eq!(&payload[8..12], &[0x12, 0x34, 0x84, 0]);
        // Compression pointers in a name
        let mut c = vec![0u8; 12];
        c.extend_from_slice(&super::dns_name_bytes("lab.local"));
        let p = c.len();
        c.extend_from_slice(&[6, b'd', b'o', b'c', b'k', b'e', b'r', 0xc0, 12]);
        assert_eq!(super::dns_name(&c, p), Some(("docker.lab.local".to_string(), p + 9)));
    }
}
