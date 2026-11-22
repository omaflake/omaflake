// SPDX-License-Identifier: AGPL-3.0-or-later
//! MIDI on a Linux host (spec §11.9, plan B161 C7) – what MIDI.swift is in `omaf-runner`: every port of the host's ALSA
//! sequencer that can be read or written (a keyboard, a synthesizer, a USB interface, another app's port) appears in
//! the guest as an endpoint of the same name, and the data go both ways over the omaflake agent as Universal MIDI
//! Packets. Nothing is captured: the host's apps keep using the ports, and ports that come or go while the VM runs come
//! and go in the guest.
//!
//! - The list: every port that can be read (subscribed: its events go to the guest) or written (the guest plays to
//!   it), except omaflake's own clients ("omaflake <VM>", also other VMs' – no loops) and the kernel's System client
//!   (timer, announce). `id` = (client << 8 | port) + 1, stable while the port lives; name "client: port"; protocol 2
//!   where the client is a UMP MIDI 2.0 client, else 1; `ump` from the client's UMP endpoint and block info. The list
//!   is sent again when the sequencer announces a port or client starting, changing or leaving.
//! - Host → guest: MIDI 1.0 events become UMP (message types 0x1, 0x2, 0x3 – `ToUmp`), UMP clients' events pass raw;
//!   each record carries the event's time (the subscription stamps it on our real-time queue), else the time it
//!   arrived.
//! - Guest → host: records go to the port of their id only (unknown ids are dropped), scheduled on our real-time queue
//!   for the time the clock mapper gives (`ClockMapper`), else at once; to a client that is not UMP as MIDI 1.0 events
//!   (`from_ump`, MIDI 2.0 channel voice messages converted down).
//! - The VM's own pair, id 0: the client "omaflake <VM>" has a writable port ("… in": what the host's apps play to it
//!   comes out of "omaflake host" in the guest) and a readable one ("… out": what the guest plays to "omaflake host").
//!
//! The pure parts – ids, the list, UMP ↔ MIDI 1.0 events, the sequencer's event data – are here for every target and
//! unit-tested on a Mac; the ALSA parts (`alsa`) exist on Linux only and load libasound.so.2 at run time (UMP-era
//! symbols, alsa-lib 1.2.10+, are optional, so an older libasound works with MIDI 1.0 only).
//! OPEN (only a Linux desktop with MIDI hardware can confirm): the whole ALSA side – subscriptions with real-time
//! stamps, the queue's time base against CLOCK_MONOTONIC, scheduled delivery, UMP clients with conversion off, the UMP
//! endpoint and block info, and the manufacturer id's byte order.

use crate::agent_protocol::{AgentSender, MidiEndpointInfo, MidiRecord, UmpBlockInfo, UmpEndpointInfo};

/// What the bridge needs of a host's MIDI system (the ALSA one, or a fake in tests)
pub trait MidiHost: Send + Sync {
    /// The guest's agent carries MIDI (its welcome enabled it, again after each new hello): it gets the whole list now
    /// and data from here on
    fn use_agent(&self, sender: AgentSender);
    /// Records the guest's apps played – to the port of each record's id (unknown ids are dropped)
    fn deliver(&self, records: Vec<MidiRecord>);
    /// The host side's state for the control socket's `midi` (plan B162): `{"endpoints", "in", "out", "agent"}` as
    /// strings, as omaf-runner answers it
    fn describe(&self) -> serde_json::Value {
        serde_json::json!({"endpoints": "0", "in": "", "out": "", "agent": "unknown"})
    }
}

/// The id of the VM's own pair in the list
pub const VM_PORT_ID: u32 = 0;
/// The VM's own pair as the guest sees it (on a Mac "omaflake Mac")
pub const VM_PAIR_NAME: &str = "omaflake host";
/// omaflake's sequencer clients are named "omaflake <VM>"; none of them is listed
pub const CLIENT_PREFIX: &str = "omaflake";

/// A port's id in the list: (client << 8 | port) + 1, never 0 (the VM's pair)
pub fn endpoint_id(client: u8, port: u8) -> u32 {
    (((client as u32) << 8) | port as u32) + 1
}

/// The (client, port) of an id; None for the VM's pair and ids no port can have
pub fn address(id: u32) -> Option<(u8, u8)> {
    if id == VM_PORT_ID || id > 0x1_0000 {
        return None;
    }
    let a = id - 1;
    Some(((a >> 8) as u8, a as u8))
}

/// Port capabilities (asequencer.h `SND_SEQ_PORT_CAP_*`)
pub mod caps {
    pub const READ: u32 = 1 << 0;
    pub const WRITE: u32 = 1 << 1;
    pub const SUBS_READ: u32 = 1 << 5;
    pub const SUBS_WRITE: u32 = 1 << 6;
    pub const NO_EXPORT: u32 = 1 << 7;
    /// Linux 6.5+: a port that is there but switched off (a UMP group without a block)
    pub const INACTIVE: u32 = 1 << 8;
}

/// A port of the host's sequencer, as `alsa` reads it
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostPort {
    pub client: u8,
    pub port: u8,
    pub client_name: String,
    pub port_name: String,
    /// `caps::*`
    pub caps: u32,
    /// The client's MIDI version: 0 legacy (MIDI 1.0 events), 1 UMP MIDI 1.0, 2 UMP MIDI 2.0
    pub midi_version: u8,
    pub ump: Option<UmpEndpointInfo>,
}

impl HostPort {
    fn usable(&self, own: u8) -> bool {
        self.client != 0 // the kernel's System client: timer, announce
            && self.client != own
            && !(self.client_name == CLIENT_PREFIX || self.client_name.starts_with("omaflake "))
            && self.caps & (caps::NO_EXPORT | caps::INACTIVE) == 0
    }

    /// Others may subscribe to read it: its events can go to the guest
    pub fn readable(&self) -> bool {
        self.caps & (caps::READ | caps::SUBS_READ) == caps::READ | caps::SUBS_READ
    }

    /// Others may write to it: the guest can play to it
    pub fn writable(&self) -> bool {
        self.caps & (caps::WRITE | caps::SUBS_WRITE) == caps::WRITE | caps::SUBS_WRITE
    }

    pub fn id(&self) -> u32 {
        endpoint_id(self.client, self.port)
    }

    fn info(&self, direction: &str) -> MidiEndpointInfo {
        let clean = |s: &str| s.chars().filter(|c| !c.is_control()).take(128).collect::<String>();
        MidiEndpointInfo {
            id: self.id(),
            direction: direction.into(),
            name: clean(&format!("{}: {}", self.client_name, self.port_name)),
            manufacturer: None,
            model: None,
            protocol: if self.midi_version == 2 { 2 } else { 1 },
            ump: self.ump.clone().map(UmpEndpointInfo::limited),
        }
    }
}

/// The list for the guest: the VM's pair first (protocol `vm_protocol`), then the ports to read ("in"), then those to
/// write ("out"), each by name and id; at most 127 ports per direction (the guest takes 256 endpoints)
pub fn endpoint_list(ports: &[HostPort], own: u8, vm_protocol: u8) -> Vec<MidiEndpointInfo> {
    let pair = |direction: &str| MidiEndpointInfo {
        id: VM_PORT_ID,
        direction: direction.into(),
        name: VM_PAIR_NAME.into(),
        manufacturer: Some("omaflake".into()),
        model: None,
        protocol: vm_protocol,
        ump: None,
    };
    let mut list = vec![pair("in"), pair("out")];
    for (direction, take) in [("in", HostPort::readable as fn(&HostPort) -> bool), ("out", HostPort::writable)] {
        let mut part: Vec<MidiEndpointInfo> =
            ports.iter().filter(|p| p.usable(own) && take(p)).map(|p| p.info(direction)).collect();
        part.sort_by(|a, b| (&a.name, a.id).cmp(&(&b.name, b.id)));
        part.truncate(127);
        list.extend(part);
    }
    list
}

/// A UMP block's direction (SND_UMP_DIR_*: 1 input, 2 output, 3 both)
pub fn block_direction(direction: u32) -> &'static str {
    match direction {
        1 => "in",
        2 => "out",
        _ => "both",
    }
}

/// A UMP block's MIDI 1.0 flags (SND_UMP_BLOCK_IS_MIDI1 = 1, SND_UMP_BLOCK_IS_LOWSPEED = 2)
pub fn block_midi1(flags: u32) -> &'static str {
    match (flags & 1 != 0, flags & 2 != 0) {
        (false, _) => "no",
        (true, false) => "yes",
        (true, true) => "low-speed",
    }
}

/// A UMP block's UI hint (SND_UMP_BLOCK_UI_HINT_*: 0 unknown, 1 receiver, 2 sender, 3 both)
pub fn block_ui_hint(hint: u32) -> &'static str {
    match hint {
        1 => "receiver",
        2 => "sender",
        3 => "both",
        _ => "unknown",
    }
}

/// Lower-case hex of `bytes`; None when they are all 0 (the device did not say)
pub fn hex_or_none(bytes: &[u8]) -> Option<String> {
    bytes.iter().any(|b| *b != 0).then(|| bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// A block as the list carries it
pub fn block_info(
    id: u8,
    name: &str,
    direction: u32,
    first_group: u32,
    groups: u32,
    flags: u32,
    ui_hint: u32,
) -> UmpBlockInfo {
    UmpBlockInfo {
        id,
        name: name.to_string(),
        direction: block_direction(direction).into(),
        first_group: first_group.min(255) as u8,
        groups: groups.min(255) as u8,
        midi1: block_midi1(flags).into(),
        ui_hint: block_ui_hint(ui_hint).into(),
    }
}

// MARK: MIDI 1.0 events

/// Sequencer event types (asequencer.h `SND_SEQ_EVENT_*`) this bridge reads or writes
pub mod event {
    pub const NOTE: u8 = 5;
    pub const NOTEON: u8 = 6;
    pub const NOTEOFF: u8 = 7;
    pub const KEYPRESS: u8 = 8;
    pub const CONTROLLER: u8 = 10;
    pub const PGMCHANGE: u8 = 11;
    pub const CHANPRESS: u8 = 12;
    pub const PITCHBEND: u8 = 13;
    pub const CONTROL14: u8 = 14;
    pub const NONREGPARAM: u8 = 15;
    pub const REGPARAM: u8 = 16;
    pub const SONGPOS: u8 = 20;
    pub const SONGSEL: u8 = 21;
    pub const QFRAME: u8 = 22;
    pub const START: u8 = 30;
    pub const CONTINUE: u8 = 31;
    pub const STOP: u8 = 32;
    pub const CLOCK: u8 = 36;
    pub const TUNE_REQUEST: u8 = 40;
    pub const RESET: u8 = 41;
    pub const SENSING: u8 = 42;
    pub const CLIENT_START: u8 = 60;
    pub const CLIENT_EXIT: u8 = 61;
    pub const CLIENT_CHANGE: u8 = 62;
    pub const PORT_START: u8 = 63;
    pub const PORT_EXIT: u8 = 64;
    pub const PORT_CHANGE: u8 = 65;
    pub const SYSEX: u8 = 130;

    /// Announcements of the System client that change the list
    pub fn announces_change(t: u8) -> bool {
        (CLIENT_START..=PORT_CHANGE).contains(&t)
    }
}

/// A MIDI 1.0 event of the sequencer (the subset that is MIDI data)
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SeqEvent {
    NoteOn {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    NoteOff {
        channel: u8,
        note: u8,
        velocity: u8,
    },
    KeyPressure {
        channel: u8,
        note: u8,
        value: u8,
    },
    Controller {
        channel: u8,
        param: u8,
        value: u8,
    },
    ProgramChange {
        channel: u8,
        value: u8,
    },
    ChannelPressure {
        channel: u8,
        value: u8,
    },
    /// −8192…8191, as ALSA has it
    PitchBend {
        channel: u8,
        value: i16,
    },
    /// A 14-bit controller (param 0–31: MSB and LSB, else one 7-bit controller)
    Control14 {
        channel: u8,
        param: u32,
        value: i32,
    },
    Rpn {
        channel: u8,
        param: u16,
        value: u16,
    },
    Nrpn {
        channel: u8,
        param: u16,
        value: u16,
    },
    SongPosition(u16),
    SongSelect(u8),
    QuarterFrame(u8),
    TuneRequest,
    Clock,
    Start,
    Continue,
    Stop,
    ActiveSensing,
    Reset,
    /// Bytes of a SysEx, possibly a piece of one (F0 on the first piece, F7 on the last)
    SysEx(Vec<u8>),
}

impl SeqEvent {
    /// The event of a sequencer event's type and its 12 data bytes (`snd_seq_event.data`: note = channel, note,
    /// velocity, off velocity, duration; ctrl = channel, 3 unused, param u32, value i32); `sysex`: its variable data.
    /// None for events that are not MIDI data.
    pub fn parse(type_: u8, d: &[u8; 12], sysex: Option<&[u8]>) -> Option<SeqEvent> {
        let channel = d[0] & 0x0F;
        let param = u32::from_le_bytes([d[4], d[5], d[6], d[7]]);
        let value = i32::from_le_bytes([d[8], d[9], d[10], d[11]]);
        let b7 = |v: i32| (v & 0x7F) as u8;
        Some(match type_ {
            event::NOTE | event::NOTEON => SeqEvent::NoteOn { channel, note: d[1] & 0x7F, velocity: d[2] & 0x7F },
            event::NOTEOFF => SeqEvent::NoteOff { channel, note: d[1] & 0x7F, velocity: d[2] & 0x7F },
            event::KEYPRESS => SeqEvent::KeyPressure { channel, note: d[1] & 0x7F, value: d[2] & 0x7F },
            event::CONTROLLER => SeqEvent::Controller { channel, param: (param & 0x7F) as u8, value: b7(value) },
            event::PGMCHANGE => SeqEvent::ProgramChange { channel, value: b7(value) },
            event::CHANPRESS => SeqEvent::ChannelPressure { channel, value: b7(value) },
            event::PITCHBEND => SeqEvent::PitchBend { channel, value: value.clamp(-8192, 8191) as i16 },
            event::CONTROL14 => SeqEvent::Control14 { channel, param, value },
            event::REGPARAM => {
                SeqEvent::Rpn { channel, param: (param & 0x3FFF) as u16, value: (value & 0x3FFF) as u16 }
            }
            event::NONREGPARAM => {
                SeqEvent::Nrpn { channel, param: (param & 0x3FFF) as u16, value: (value & 0x3FFF) as u16 }
            }
            event::SONGPOS => SeqEvent::SongPosition((value & 0x3FFF) as u16),
            event::SONGSEL => SeqEvent::SongSelect(b7(value)),
            event::QFRAME => SeqEvent::QuarterFrame(b7(value)),
            event::TUNE_REQUEST => SeqEvent::TuneRequest,
            event::CLOCK => SeqEvent::Clock,
            event::START => SeqEvent::Start,
            event::CONTINUE => SeqEvent::Continue,
            event::STOP => SeqEvent::Stop,
            event::SENSING => SeqEvent::ActiveSensing,
            event::RESET => SeqEvent::Reset,
            event::SYSEX => SeqEvent::SysEx(sysex?.to_vec()),
            _ => return None,
        })
    }

    /// The sequencer event's type and data bytes (for `SysEx` the variable data are the event's bytes, the data
    /// bytes carry only their length – the caller adds the pointer)
    pub fn encode(&self) -> (u8, [u8; 12]) {
        let mut d = [0u8; 12];
        let note = |d: &mut [u8; 12], channel: u8, note: u8, velocity: u8| {
            d[0] = channel & 0x0F;
            d[1] = note & 0x7F;
            d[2] = velocity & 0x7F;
        };
        let ctrl = |d: &mut [u8; 12], channel: u8, param: u32, value: i32| {
            d[0] = channel & 0x0F;
            d[4..8].copy_from_slice(&param.to_le_bytes());
            d[8..12].copy_from_slice(&value.to_le_bytes());
        };
        let t = match *self {
            SeqEvent::NoteOn { channel, note: n, velocity } => {
                note(&mut d, channel, n, velocity);
                event::NOTEON
            }
            SeqEvent::NoteOff { channel, note: n, velocity } => {
                note(&mut d, channel, n, velocity);
                event::NOTEOFF
            }
            SeqEvent::KeyPressure { channel, note: n, value } => {
                note(&mut d, channel, n, value);
                event::KEYPRESS
            }
            SeqEvent::Controller { channel, param, value } => {
                ctrl(&mut d, channel, param as u32, value as i32);
                event::CONTROLLER
            }
            SeqEvent::ProgramChange { channel, value } => {
                ctrl(&mut d, channel, 0, value as i32);
                event::PGMCHANGE
            }
            SeqEvent::ChannelPressure { channel, value } => {
                ctrl(&mut d, channel, 0, value as i32);
                event::CHANPRESS
            }
            SeqEvent::PitchBend { channel, value } => {
                ctrl(&mut d, channel, 0, value as i32);
                event::PITCHBEND
            }
            SeqEvent::Control14 { channel, param, value } => {
                ctrl(&mut d, channel, param, value);
                event::CONTROL14
            }
            SeqEvent::Rpn { channel, param, value } => {
                ctrl(&mut d, channel, param as u32, value as i32);
                event::REGPARAM
            }
            SeqEvent::Nrpn { channel, param, value } => {
                ctrl(&mut d, channel, param as u32, value as i32);
                event::NONREGPARAM
            }
            SeqEvent::SongPosition(v) => {
                ctrl(&mut d, 0, 0, v as i32);
                event::SONGPOS
            }
            SeqEvent::SongSelect(v) => {
                ctrl(&mut d, 0, 0, v as i32);
                event::SONGSEL
            }
            SeqEvent::QuarterFrame(v) => {
                ctrl(&mut d, 0, 0, v as i32);
                event::QFRAME
            }
            SeqEvent::TuneRequest => event::TUNE_REQUEST,
            SeqEvent::Clock => event::CLOCK,
            SeqEvent::Start => event::START,
            SeqEvent::Continue => event::CONTINUE,
            SeqEvent::Stop => event::STOP,
            SeqEvent::ActiveSensing => event::SENSING,
            SeqEvent::Reset => event::RESET,
            SeqEvent::SysEx(ref bytes) => {
                d[0..4].copy_from_slice(&(bytes.len() as u32).to_le_bytes());
                event::SYSEX
            }
        };
        (t, d)
    }
}

/// MIDI 1.0 events of one source as UMP of the MIDI 1.0 protocol, group 0 (UMP spec 1.1: system messages type 0x1,
/// channel voice type 0x2, SysEx type 0x3 in packets of up to 6 bytes). A SysEx may arrive in pieces; the state between
/// them is kept here, so each source needs its own.
#[derive(Debug, Clone, Default)]
pub struct ToUmp {
    in_sysex: bool,
    started: bool,
    pending: Vec<u8>,
}

impl ToUmp {
    pub fn new() -> Self {
        Self::default()
    }

    /// Appends the UMP words of `ev` to `out`
    pub fn push(&mut self, ev: &SeqEvent, out: &mut Vec<u32>) {
        let voice = |status: u8, channel: u8, d1: u8, d2: u8| {
            0x2000_0000
                | ((status as u32 | (channel & 0x0F) as u32) << 16)
                | ((d1 & 0x7F) as u32) << 8
                | (d2 & 0x7F) as u32
        };
        let system = |status: u8, d1: u8, d2: u8| {
            0x1000_0000 | (status as u32) << 16 | ((d1 & 0x7F) as u32) << 8 | (d2 & 0x7F) as u32
        };
        match *ev {
            SeqEvent::NoteOff { channel, note, velocity } => out.push(voice(0x80, channel, note, velocity)),
            SeqEvent::NoteOn { channel, note, velocity } => out.push(voice(0x90, channel, note, velocity)),
            SeqEvent::KeyPressure { channel, note, value } => out.push(voice(0xA0, channel, note, value)),
            SeqEvent::Controller { channel, param, value } => out.push(voice(0xB0, channel, param, value)),
            SeqEvent::ProgramChange { channel, value } => out.push(voice(0xC0, channel, value, 0)),
            SeqEvent::ChannelPressure { channel, value } => out.push(voice(0xD0, channel, value, 0)),
            SeqEvent::PitchBend { channel, value } => {
                let v = (value as i32 + 8192).clamp(0, 0x3FFF) as u16;
                out.push(voice(0xE0, channel, (v & 0x7F) as u8, (v >> 7) as u8));
            }
            SeqEvent::Control14 { channel, param, value } => {
                let v = value.clamp(0, 0x3FFF);
                if param < 32 {
                    out.push(voice(0xB0, channel, param as u8, (v >> 7) as u8));
                    out.push(voice(0xB0, channel, param as u8 + 32, (v & 0x7F) as u8));
                } else {
                    out.push(voice(0xB0, channel, (param & 0x7F) as u8, (v & 0x7F) as u8));
                }
            }
            SeqEvent::Rpn { channel, param, value } | SeqEvent::Nrpn { channel, param, value } => {
                let (msb, lsb) = if matches!(ev, SeqEvent::Rpn { .. }) { (101, 100) } else { (99, 98) };
                out.push(voice(0xB0, channel, msb, (param >> 7) as u8));
                out.push(voice(0xB0, channel, lsb, (param & 0x7F) as u8));
                out.push(voice(0xB0, channel, 6, (value >> 7) as u8));
                out.push(voice(0xB0, channel, 38, (value & 0x7F) as u8));
            }
            SeqEvent::QuarterFrame(v) => out.push(system(0xF1, v, 0)),
            SeqEvent::SongPosition(v) => out.push(system(0xF2, (v & 0x7F) as u8, (v >> 7) as u8)),
            SeqEvent::SongSelect(v) => out.push(system(0xF3, v, 0)),
            SeqEvent::TuneRequest => out.push(system(0xF6, 0, 0)),
            SeqEvent::Clock => out.push(system(0xF8, 0, 0)),
            SeqEvent::Start => out.push(system(0xFA, 0, 0)),
            SeqEvent::Continue => out.push(system(0xFB, 0, 0)),
            SeqEvent::Stop => out.push(system(0xFC, 0, 0)),
            SeqEvent::ActiveSensing => out.push(system(0xFE, 0, 0)),
            SeqEvent::Reset => out.push(system(0xFF, 0, 0)),
            SeqEvent::SysEx(ref bytes) => self.sysex(bytes, out),
        }
    }

    /// SysEx bytes: F0 starts one (ending an unterminated one before), F7 ends it, data bytes in between go out in
    /// packets of 6 – the last packet of a SysEx is held back until its end is known
    fn sysex(&mut self, bytes: &[u8], out: &mut Vec<u32>) {
        for &b in bytes {
            match b {
                0xF0 => {
                    if self.in_sysex {
                        self.finish(out);
                    }
                    self.in_sysex = true;
                }
                0xF7 => {
                    if self.in_sysex {
                        self.finish(out);
                    }
                }
                b if b & 0x80 != 0 => {} // another status inside a SysEx: not data
                b if self.in_sysex => {
                    if self.pending.len() == 6 {
                        let status = if self.started { 2 } else { 1 };
                        Self::packet(status, &self.pending, out);
                        self.started = true;
                        self.pending.clear();
                    }
                    self.pending.push(b);
                }
                _ => {} // data without a SysEx
            }
        }
    }

    fn finish(&mut self, out: &mut Vec<u32>) {
        let status = if self.started { 3 } else { 0 };
        Self::packet(status, &self.pending, out);
        *self = ToUmp::default();
    }

    /// One SysEx7 packet: status 0 complete, 1 start, 2 continue, 3 end; at most 6 bytes
    fn packet(status: u32, data: &[u8], out: &mut Vec<u32>) {
        let mut b = [0u8; 6];
        b[..data.len()].copy_from_slice(data);
        out.push(0x3000_0000 | status << 20 | (data.len() as u32) << 16 | (b[0] as u32) << 8 | b[1] as u32);
        out.push(u32::from_be_bytes([b[2], b[3], b[4], b[5]]));
    }
}

/// UMP words (whole messages) as MIDI 1.0 events, for a port of a client that is not UMP: MIDI 1.0 channel voice
/// (type 0x2) and system messages (0x1) as they are, SysEx7 (0x3) as pieces (F0 on a start or complete packet, F7 on an
/// end or complete one), MIDI 2.0 channel voice (0x4) converted down (UMP spec 1.1, "Translation"): velocities and
/// values to 7 bits, pitch bend to 14, program change with its bank as CC 0/32, RPN and NRPN as CC 101/100 or 99/98
/// with 6/38. Per-note and relative controllers, utility, SysEx8, flex data and stream messages have no MIDI 1.0 form
/// and are dropped. The group is dropped (a MIDI 1.0 port has none).
pub fn from_ump(words: &[u32]) -> Vec<SeqEvent> {
    let mut out = vec![];
    let mut i = 0;
    while i < words.len() {
        let w0 = words[i];
        let n = MidiRecord::message_words(w0);
        let Some(m) = words.get(i..i + n) else { break };
        i += n;
        let status = ((w0 >> 16) & 0xFF) as u8;
        let channel = status & 0x0F;
        let (d1, d2) = (((w0 >> 8) & 0x7F) as u8, (w0 & 0x7F) as u8);
        match w0 >> 28 {
            0x1 => out.extend(match status {
                0xF1 => Some(SeqEvent::QuarterFrame(d1)),
                0xF2 => Some(SeqEvent::SongPosition(d1 as u16 | (d2 as u16) << 7)),
                0xF3 => Some(SeqEvent::SongSelect(d1)),
                0xF6 => Some(SeqEvent::TuneRequest),
                0xF8 => Some(SeqEvent::Clock),
                0xFA => Some(SeqEvent::Start),
                0xFB => Some(SeqEvent::Continue),
                0xFC => Some(SeqEvent::Stop),
                0xFE => Some(SeqEvent::ActiveSensing),
                0xFF => Some(SeqEvent::Reset),
                _ => None,
            }),
            0x2 => out.extend(match status & 0xF0 {
                0x80 => Some(SeqEvent::NoteOff { channel, note: d1, velocity: d2 }),
                0x90 => Some(SeqEvent::NoteOn { channel, note: d1, velocity: d2 }),
                0xA0 => Some(SeqEvent::KeyPressure { channel, note: d1, value: d2 }),
                0xB0 => Some(SeqEvent::Controller { channel, param: d1, value: d2 }),
                0xC0 => Some(SeqEvent::ProgramChange { channel, value: d1 }),
                0xD0 => Some(SeqEvent::ChannelPressure { channel, value: d1 }),
                0xE0 => Some(SeqEvent::PitchBend { channel, value: (d1 as i32 | (d2 as i32) << 7) as i16 - 8192 }),
                _ => None,
            }),
            0x3 => {
                let kind = (w0 >> 20) & 0x0F;
                let len = (((w0 >> 16) & 0x0F) as usize).min(6);
                let bytes =
                    [(w0 >> 8) as u8, w0 as u8, (m[1] >> 24) as u8, (m[1] >> 16) as u8, (m[1] >> 8) as u8, m[1] as u8];
                let mut sysex = Vec::with_capacity(8);
                if kind == 0 || kind == 1 {
                    sysex.push(0xF0);
                }
                sysex.extend(bytes[..len].iter().map(|b| b & 0x7F));
                if kind == 0 || kind == 3 {
                    sysex.push(0xF7);
                }
                if kind <= 3 {
                    out.push(SeqEvent::SysEx(sysex));
                }
            }
            0x4 => midi2_down(m[0], m[1], &mut out),
            _ => {}
        }
    }
    out
}

/// One MIDI 2.0 channel voice message as MIDI 1.0 events
fn midi2_down(w0: u32, w1: u32, out: &mut Vec<SeqEvent>) {
    let status = ((w0 >> 20) & 0x0F) as u8;
    let channel = ((w0 >> 16) & 0x0F) as u8;
    let (i1, i2) = (((w0 >> 8) & 0x7F) as u8, (w0 & 0x7F) as u8);
    let cc = |param: u8, value: u8| SeqEvent::Controller { channel, param, value: value & 0x7F };
    let v7 = (w1 >> 25) as u8;
    let vel = ((w1 >> 16) >> 9) as u8;
    match status {
        0x8 => out.push(SeqEvent::NoteOff { channel, note: i1, velocity: vel }),
        // A MIDI 2.0 note on with a velocity below 1/128 must not turn into a MIDI 1.0 note off
        0x9 => out.push(SeqEvent::NoteOn { channel, note: i1, velocity: vel.max(1) }),
        0xA => out.push(SeqEvent::KeyPressure { channel, note: i1, value: v7 }),
        0xB => out.push(cc(i1, v7)),
        0xC => {
            if w0 & 1 != 0 {
                out.push(cc(0, ((w1 >> 8) & 0x7F) as u8));
                out.push(cc(32, (w1 & 0x7F) as u8));
            }
            out.push(SeqEvent::ProgramChange { channel, value: ((w1 >> 24) & 0x7F) as u8 });
        }
        0xD => out.push(SeqEvent::ChannelPressure { channel, value: v7 }),
        0xE => out.push(SeqEvent::PitchBend { channel, value: (w1 >> 18) as i16 - 8192 }),
        0x2 | 0x3 => {
            let (msb, lsb) = if status == 0x2 { (101, 100) } else { (99, 98) };
            let v14 = (w1 >> 18) as u16;
            out.extend([cc(msb, i1), cc(lsb, i2), cc(6, (v14 >> 7) as u8), cc(38, (v14 & 0x7F) as u8)]);
        }
        _ => {} // per-note controllers and management, relative controllers: no MIDI 1.0 form
    }
}

#[cfg(target_os = "linux")]
pub use alsa::{AlsaMidi, start};

/// The ALSA sequencer through libasound.so.2, loaded at run time
#[cfg(target_os = "linux")]
mod alsa {
    use super::*;
    use crate::Log;
    use crate::agent_protocol::{AgentMessage, ClockMapper, monotonic_ns};
    use std::collections::HashMap;
    use std::ffi::{CStr, CString, c_char, c_int, c_short, c_uint, c_void};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};

    const OPEN_DUPLEX: c_int = 3;
    const NONBLOCK: c_int = 1;
    const TYPE_MIDI_GENERIC: c_uint = 1 << 1;
    const TYPE_SOFTWARE: c_uint = 1 << 17;
    const TYPE_APPLICATION: c_uint = 1 << 20;
    const EVENT_START: c_int = event::START as c_int;
    /// Event flags: real-time stamp, relative time, variable length, UMP (Linux 6.5+)
    const TIME_STAMP_REAL: u8 = 1 << 0;
    const TIME_MODE_REL: u8 = 1 << 1;
    const LENGTH_MASK: u8 = 3 << 2;
    const LENGTH_VARIABLE: u8 = 1 << 2;
    const EVENT_UMP: u8 = 1 << 5;
    const QUEUE_DIRECT: u8 = 253;
    const ADDRESS_SUBSCRIBERS: u8 = 254;
    const ADDRESS_UNKNOWN: u8 = 253;
    const CLIENT_UMP_MIDI_2_0: c_int = 2;
    const UMP_EP_STATIC_BLOCKS: c_uint = 1 << 0;

    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    struct Addr {
        client: u8,
        port: u8,
    }

    /// `snd_seq_ump_event_t`: `snd_seq_event_t` (28 bytes) with 16 bytes of data instead of 12, so either fits in it
    #[repr(C)]
    #[derive(Debug, Clone, Copy, Default)]
    struct Event {
        type_: u8,
        flags: u8,
        tag: i8,
        queue: u8,
        time: [u32; 2],
        source: Addr,
        dest: Addr,
        data: [u8; 16],
    }

    const _: () = assert!(std::mem::size_of::<Event>() == 32);

    macro_rules! library {
        (required { $($name:ident: fn($($arg:ty),*) $(-> $ret:ty)?;)* }
         optional { $($oname:ident: fn($($oarg:ty),*) $(-> $oret:ty)?;)* }) => {
            #[allow(non_snake_case)]
            struct Asound {
                $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
                $($oname: Option<unsafe extern "C" fn($($oarg),*) $(-> $oret)?>,)*
            }

            impl Asound {
                fn resolve(handle: *mut c_void) -> Result<Asound, String> {
                    Ok(Asound {
                        $($name: {
                            let p = symbol(handle, concat!(stringify!($name), "\0"))
                                .ok_or(concat!("libasound has no ", stringify!($name)))?;
                            // SAFETY: the symbol is libasound's function of this signature (alsa/seq.h)
                            unsafe { std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($arg),*) $(-> $ret)?>(p) }
                        },)*
                        $($oname: symbol(handle, concat!(stringify!($oname), "\0")).map(|p| {
                            // SAFETY: as above (alsa/seq.h, alsa/ump.h of alsa-lib 1.2.10+)
                            unsafe { std::mem::transmute::<*mut c_void, unsafe extern "C" fn($($oarg),*) $(-> $oret)?>(p) }
                        }),)*
                    })
                }
            }
        };
    }

    type P = *mut c_void;
    type C = *const c_void;

    library! {
        required {
            snd_seq_open: fn(*mut P, *const c_char, c_int, c_int) -> c_int;
            snd_seq_set_client_name: fn(P, *const c_char) -> c_int;
            snd_seq_client_id: fn(P) -> c_int;
            snd_seq_create_simple_port: fn(P, *const c_char, c_uint, c_uint) -> c_int;
            snd_seq_alloc_named_queue: fn(P, *const c_char) -> c_int;
            snd_seq_control_queue: fn(P, c_int, c_int, c_int, P) -> c_int;
            snd_seq_drain_output: fn(P) -> c_int;
            snd_seq_connect_from: fn(P, c_int, c_int, c_int) -> c_int;
            snd_seq_port_subscribe_malloc: fn(*mut P) -> c_int;
            snd_seq_port_subscribe_free: fn(P);
            snd_seq_port_subscribe_set_sender: fn(P, *const Addr);
            snd_seq_port_subscribe_set_dest: fn(P, *const Addr);
            snd_seq_port_subscribe_set_queue: fn(P, c_int);
            snd_seq_port_subscribe_set_time_update: fn(P, c_int);
            snd_seq_port_subscribe_set_time_real: fn(P, c_int);
            snd_seq_subscribe_port: fn(P, P) -> c_int;
            snd_seq_unsubscribe_port: fn(P, P) -> c_int;
            snd_seq_client_info_malloc: fn(*mut P) -> c_int;
            snd_seq_client_info_free: fn(P);
            snd_seq_client_info_set_client: fn(P, c_int);
            snd_seq_client_info_get_client: fn(C) -> c_int;
            snd_seq_client_info_get_name: fn(C) -> *const c_char;
            snd_seq_query_next_client: fn(P, P) -> c_int;
            snd_seq_get_client_info: fn(P, P) -> c_int;
            snd_seq_set_client_info: fn(P, P) -> c_int;
            snd_seq_port_info_malloc: fn(*mut P) -> c_int;
            snd_seq_port_info_free: fn(P);
            snd_seq_port_info_set_client: fn(P, c_int);
            snd_seq_port_info_set_port: fn(P, c_int);
            snd_seq_port_info_get_port: fn(C) -> c_int;
            snd_seq_port_info_get_name: fn(C) -> *const c_char;
            snd_seq_port_info_get_capability: fn(C) -> c_uint;
            snd_seq_query_next_port: fn(P, P) -> c_int;
            snd_seq_event_input: fn(P, *mut *mut Event) -> c_int;
            snd_seq_event_output_direct: fn(P, *mut Event) -> c_int;
            snd_seq_poll_descriptors_count: fn(P, c_short) -> c_int;
            snd_seq_poll_descriptors: fn(P, *mut libc::pollfd, c_uint, c_short) -> c_int;
        }
        optional {
            snd_seq_set_client_midi_version: fn(P, c_int) -> c_int;
            snd_seq_client_info_get_midi_version: fn(C) -> c_int;
            snd_seq_client_info_set_ump_conversion: fn(P, c_int);
            snd_seq_ump_event_input: fn(P, *mut *mut Event) -> c_int;
            snd_seq_ump_event_output_direct: fn(P, *mut Event) -> c_int;
            snd_seq_get_ump_endpoint_info: fn(P, c_int, P) -> c_int;
            snd_seq_get_ump_block_info: fn(P, c_int, c_int, P) -> c_int;
            snd_ump_endpoint_info_malloc: fn(*mut P) -> c_int;
            snd_ump_endpoint_info_free: fn(P);
            snd_ump_endpoint_info_get_name: fn(C) -> *const c_char;
            snd_ump_endpoint_info_get_product_id: fn(C) -> *const c_char;
            snd_ump_endpoint_info_get_manufacturer_id: fn(C) -> c_uint;
            snd_ump_endpoint_info_get_family_id: fn(C) -> c_uint;
            snd_ump_endpoint_info_get_model_id: fn(C) -> c_uint;
            snd_ump_endpoint_info_get_sw_revision: fn(C) -> *const u8;
            snd_ump_endpoint_info_get_flags: fn(C) -> c_uint;
            snd_ump_endpoint_info_get_num_blocks: fn(C) -> c_uint;
            snd_ump_block_info_malloc: fn(*mut P) -> c_int;
            snd_ump_block_info_free: fn(P);
            snd_ump_block_info_get_name: fn(C) -> *const c_char;
            snd_ump_block_info_get_direction: fn(C) -> c_uint;
            snd_ump_block_info_get_first_group: fn(C) -> c_uint;
            snd_ump_block_info_get_num_groups: fn(C) -> c_uint;
            snd_ump_block_info_get_flags: fn(C) -> c_uint;
            snd_ump_block_info_get_ui_hint: fn(C) -> c_uint;
        }
    }

    fn symbol(handle: *mut c_void, name: &str) -> Option<*mut c_void> {
        // SAFETY: `handle` is from dlopen, `name` NUL-terminated (the macro appends it)
        let p = unsafe { libc::dlsym(handle, name.as_ptr() as *const c_char) };
        (!p.is_null()).then_some(p)
    }

    fn load() -> Result<&'static Asound, String> {
        // SAFETY: a NUL-terminated library name; the handle is kept for the process's life (never dlclose'd)
        let handle = unsafe { libc::dlopen(c"libasound.so.2".as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
        if handle.is_null() {
            return Err("libasound.so.2 not found (alsa-lib)".into());
        }
        Ok(Box::leak(Box::new(Asound::resolve(handle)?)))
    }

    fn text(p: *const c_char) -> String {
        if p.is_null() {
            return String::new();
        }
        // SAFETY: libasound returns NUL-terminated strings that live as long as their info object
        unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
    }

    fn c_name(s: &str) -> CString {
        let mut clean: String = s.chars().filter(|c| !c.is_control()).collect();
        let mut end = clean.len().min(63);
        while !clean.is_char_boundary(end) {
            end -= 1;
        }
        clean.truncate(end);
        CString::new(clean).unwrap_or_default()
    }

    /// An info object of libasound, freed when dropped
    struct Info {
        p: P,
        free: unsafe extern "C" fn(P),
    }

    impl Info {
        fn new(malloc: unsafe extern "C" fn(*mut P) -> c_int, free: unsafe extern "C" fn(P)) -> Option<Info> {
            let mut p: P = std::ptr::null_mut();
            // SAFETY: libasound's *_malloc writes a new object or fails
            (unsafe { malloc(&mut p) } >= 0 && !p.is_null()).then_some(Info { p, free })
        }
    }

    impl Drop for Info {
        fn drop(&mut self) {
            // SAFETY: allocated by the matching *_malloc, freed once
            unsafe { (self.free)(self.p) }
        }
    }

    /// The sequencer state, under one lock (a libasound handle is not thread-safe)
    struct Seq {
        handle: P,
        me: u8,
        /// The VM's pair: host apps play to `vm_in`, read from `vm_out`
        vm_in: u8,
        vm_out: u8,
        /// Our own port, not exported: subscribed to the host's readable ports, the source of what goes to theirs
        bridge: u8,
        queue: c_int,
        /// CLOCK_MONOTONIC when the queue started: its real time + this = our monotonic time
        queue_base: u64,
        /// Our client is UMP MIDI 2.0 with conversion off: UMP clients' events come raw, legacy ones as they are
        ump: bool,
        /// Readable ports we listen to, by id
        connected: HashMap<u32, (u8, u8)>,
        /// Writable ports by id: whether their client is UMP
        destinations: HashMap<u32, bool>,
        /// MIDI version of each client (for the protocol of their UMP events)
        versions: HashMap<u8, u8>,
        list: Vec<MidiEndpointInfo>,
        sender: Option<AgentSender>,
        clock: ClockMapper,
        sysex: HashMap<u32, ToUmp>,
    }

    // SAFETY: the handle is only used under the Mutex that owns `Seq`
    unsafe impl Send for Seq {}

    /// The host's MIDI over the ALSA sequencer (see the module's documentation)
    pub struct AlsaMidi {
        lib: &'static Asound,
        seq: Mutex<Seq>,
        log: Arc<Log<'static>>,
    }

    /// Opens the sequencer as "omaflake <VM>", creates the VM's pair and starts reading (a thread for the life of the
    /// process); Err when there is no libasound or no sequencer (no snd-seq module)
    pub fn start(vm_name: &str, log: Arc<Log<'static>>) -> Result<Arc<AlsaMidi>, String> {
        let lib = load()?;
        let mut handle: P = std::ptr::null_mut();
        // SAFETY: valid out pointer and device name
        let rc = unsafe { (lib.snd_seq_open)(&mut handle, c"default".as_ptr(), OPEN_DUPLEX, NONBLOCK) };
        if rc < 0 || handle.is_null() {
            return Err(format!("no ALSA sequencer (error {rc})"));
        }
        let name = c_name(&format!("omaflake {vm_name}"));
        // SAFETY: an open handle; NUL-terminated names
        let (me, ump) = unsafe {
            (lib.snd_seq_set_client_name)(handle, name.as_ptr());
            let me = (lib.snd_seq_client_id)(handle);
            (me, set_ump(lib, handle))
        };
        let port = |name: &str, caps: u32, type_: c_uint| -> Result<u8, String> {
            let name = c_name(name);
            // SAFETY: an open handle, a NUL-terminated name
            let p = unsafe { (lib.snd_seq_create_simple_port)(handle, name.as_ptr(), caps, type_) };
            u8::try_from(p).map_err(|_| format!("cannot create the port {name:?} (error {p})"))
        };
        let public = TYPE_MIDI_GENERIC | TYPE_SOFTWARE | TYPE_APPLICATION;
        let vm_in = port(&format!("omaflake {vm_name} in"), caps::WRITE | caps::SUBS_WRITE, public)?;
        let vm_out = port(&format!("omaflake {vm_name} out"), caps::READ | caps::SUBS_READ, public)?;
        let bridge = port("omaflake bridge", caps::READ | caps::WRITE | caps::NO_EXPORT, TYPE_APPLICATION)?;
        // SAFETY: an open handle; the queue's start goes out with the drain
        let (queue, queue_base) = unsafe {
            let q = (lib.snd_seq_alloc_named_queue)(handle, c"omaflake".as_ptr());
            if q < 0 {
                return Err(format!("cannot allocate a queue (error {q})"));
            }
            (lib.snd_seq_control_queue)(handle, q, EVENT_START, 0, std::ptr::null_mut());
            (lib.snd_seq_drain_output)(handle);
            let base = monotonic_ns();
            // Announcements (ports and clients starting, changing, leaving) come to our own port
            (lib.snd_seq_connect_from)(handle, bridge as c_int, 0, 1);
            (q, base)
        };
        let seq = Seq {
            handle,
            me: me as u8,
            vm_in,
            vm_out,
            bridge,
            queue,
            queue_base,
            ump,
            connected: HashMap::new(),
            destinations: HashMap::new(),
            versions: HashMap::new(),
            list: vec![],
            sender: None,
            clock: ClockMapper::new(),
            sysex: HashMap::new(),
        };
        let midi = Arc::new(AlsaMidi { lib, seq: Mutex::new(seq), log });
        log_ump(&midi.log, ump);
        midi.rescan(false);
        let m = midi.clone();
        std::thread::Builder::new()
            .name("midi".into())
            .spawn(move || m.read_loop())
            .map_err(|e| format!("cannot start the MIDI thread: {e}"))?;
        Ok(midi)
    }

    fn log_ump(log: &Arc<Log<'static>>, ump: bool) {
        log(if ump {
            "MIDI: ALSA sequencer with UMP (MIDI 2.0 clients pass raw)"
        } else {
            "MIDI: ALSA sequencer, MIDI 1.0 events (no UMP in this kernel or libasound)"
        });
    }

    /// Makes our client UMP MIDI 2.0 with conversion off, where kernel and libasound have it (Linux 6.5+, alsa-lib
    /// 1.2.10+); false: a legacy client
    ///
    /// SAFETY: `handle` is an open sequencer handle of `lib`
    unsafe fn set_ump(lib: &Asound, handle: P) -> bool {
        let (Some(version), Some(conversion), Some(_), Some(_)) = (
            lib.snd_seq_set_client_midi_version,
            lib.snd_seq_client_info_set_ump_conversion,
            lib.snd_seq_ump_event_input,
            lib.snd_seq_ump_event_output_direct,
        ) else {
            return false;
        };
        let Some(info) = Info::new(lib.snd_seq_client_info_malloc, lib.snd_seq_client_info_free) else { return false };
        // SAFETY: (the caller's) an open handle and a client info object
        unsafe {
            if version(handle, CLIENT_UMP_MIDI_2_0) < 0 || (lib.snd_seq_get_client_info)(handle, info.p) < 0 {
                version(handle, 0);
                return false;
            }
            conversion(info.p, 0);
            if (lib.snd_seq_set_client_info)(handle, info.p) < 0 {
                version(handle, 0);
                return false;
            }
        }
        true
    }

    impl MidiHost for AlsaMidi {
        fn use_agent(&self, sender: AgentSender) {
            self.seq.lock().unwrap_or_else(|e| e.into_inner()).sender = Some(sender);
            self.rescan(true);
        }

        fn describe(&self) -> serde_json::Value {
            let s = self.seq.lock().unwrap_or_else(|e| e.into_inner());
            let names = |direction: &str| {
                s.list
                    .iter()
                    .filter(|e| e.direction == direction)
                    .map(|e| e.name.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            };
            serde_json::json!({
                "endpoints": s.list.len().to_string(),
                "in": names("in"),
                "out": names("out"),
                "agent": if s.sender.is_some() { "connected" } else { "waiting for the guest's agent" },
            })
        }

        fn deliver(&self, records: Vec<MidiRecord>) {
            let lib = self.lib;
            let mut s = self.seq.lock().unwrap_or_else(|e| e.into_inner());
            let now = monotonic_ns();
            for r in records {
                let delay = r.time.and_then(|t| s.clock.schedule(now, t)).map(|at| at - now);
                let (source, dest, raw) = if r.id == VM_PORT_ID {
                    (s.vm_out, Addr { client: ADDRESS_SUBSCRIBERS, port: ADDRESS_UNKNOWN }, s.ump)
                } else if let (Some(&ump_client), Some((client, port))) = (s.destinations.get(&r.id), address(r.id)) {
                    (s.bridge, Addr { client, port }, s.ump && ump_client)
                } else {
                    continue; // a port that is gone (or never was)
                };
                let mut ev = Event { source: Addr { client: s.me, port: source }, dest, ..Default::default() };
                match delay {
                    Some(d) => {
                        ev.flags = TIME_STAMP_REAL | TIME_MODE_REL;
                        ev.queue = s.queue as u8;
                        ev.time = [(d / 1_000_000_000) as u32, (d % 1_000_000_000) as u32];
                    }
                    None => ev.queue = QUEUE_DIRECT,
                }
                if raw {
                    let mut i = 0;
                    while i < r.words.len() {
                        let n = MidiRecord::message_words(r.words[i]);
                        let mut e = ev;
                        e.flags |= EVENT_UMP;
                        for (k, w) in r.words[i..i + n].iter().enumerate() {
                            e.data[k * 4..k * 4 + 4].copy_from_slice(&w.to_ne_bytes());
                        }
                        s.output(lib, &mut e);
                        i += n;
                    }
                } else {
                    for m in from_ump(&r.words) {
                        let (type_, data) = m.encode();
                        let mut e = ev;
                        e.type_ = type_;
                        e.data[..12].copy_from_slice(&data);
                        if let SeqEvent::SysEx(bytes) = &m {
                            e.flags |= LENGTH_VARIABLE;
                            e.data[4..12].copy_from_slice(&(bytes.as_ptr() as u64).to_ne_bytes());
                        }
                        s.output(lib, &mut e);
                    }
                }
            }
        }
    }

    impl Seq {
        fn output(&self, lib: &Asound, ev: &mut Event) {
            // SAFETY: an open handle and a valid event (a SysEx's pointer is to bytes alive for this call)
            let rc = unsafe {
                match lib.snd_seq_ump_event_output_direct.filter(|_| self.ump) {
                    Some(f) => f(self.handle, ev),
                    None => (lib.snd_seq_event_output_direct)(self.handle, ev),
                }
            };
            let _ = rc; // a port that went away meanwhile: dropped (the announcement updates the list)
        }

        /// Subscribes our port to a readable port, its events stamped with our queue's real time
        fn subscribe(&self, lib: &Asound, client: u8, port: u8, on: bool) -> bool {
            let Some(sub) = Info::new(lib.snd_seq_port_subscribe_malloc, lib.snd_seq_port_subscribe_free) else {
                return false;
            };
            let sender = Addr { client, port };
            let dest = Addr { client: self.me, port: self.bridge };
            // SAFETY: an open handle and a subscription object
            unsafe {
                (lib.snd_seq_port_subscribe_set_sender)(sub.p, &sender);
                (lib.snd_seq_port_subscribe_set_dest)(sub.p, &dest);
                (lib.snd_seq_port_subscribe_set_queue)(sub.p, self.queue);
                (lib.snd_seq_port_subscribe_set_time_update)(sub.p, 1);
                (lib.snd_seq_port_subscribe_set_time_real)(sub.p, 1);
                if on {
                    (lib.snd_seq_subscribe_port)(self.handle, sub.p) >= 0
                } else {
                    (lib.snd_seq_unsubscribe_port)(self.handle, sub.p) >= 0
                }
            }
        }

        /// Every port of the sequencer with its client's name, MIDI version and UMP info
        fn ports(&self, lib: &Asound) -> Vec<HostPort> {
            let mut out = vec![];
            let (Some(ci), Some(pi)) = (
                Info::new(lib.snd_seq_client_info_malloc, lib.snd_seq_client_info_free),
                Info::new(lib.snd_seq_port_info_malloc, lib.snd_seq_port_info_free),
            ) else {
                return out;
            };
            // SAFETY: an open handle and info objects; the strings are copied before the next query
            unsafe {
                (lib.snd_seq_client_info_set_client)(ci.p, -1);
                while (lib.snd_seq_query_next_client)(self.handle, ci.p) >= 0 {
                    let client = (lib.snd_seq_client_info_get_client)(ci.p);
                    let Ok(client) = u8::try_from(client) else { continue };
                    let client_name = text((lib.snd_seq_client_info_get_name)(ci.p));
                    let midi_version =
                        lib.snd_seq_client_info_get_midi_version.map(|f| f(ci.p)).unwrap_or(0).clamp(0, 2) as u8;
                    let ump = if midi_version > 0 { self.ump_info(lib, client) } else { None };
                    (lib.snd_seq_port_info_set_client)(pi.p, client as c_int);
                    (lib.snd_seq_port_info_set_port)(pi.p, -1);
                    while (lib.snd_seq_query_next_port)(self.handle, pi.p) >= 0 {
                        let Ok(port) = u8::try_from((lib.snd_seq_port_info_get_port)(pi.p)) else { continue };
                        out.push(HostPort {
                            client,
                            port,
                            client_name: client_name.clone(),
                            port_name: text((lib.snd_seq_port_info_get_name)(pi.p)),
                            caps: (lib.snd_seq_port_info_get_capability)(pi.p),
                            midi_version,
                            ump: ump.clone(),
                        });
                    }
                }
            }
            out
        }

        /// A UMP client's endpoint and block info; None where libasound or the client has none
        fn ump_info(&self, lib: &Asound, client: u8) -> Option<UmpEndpointInfo> {
            let ep = Info::new(lib.snd_ump_endpoint_info_malloc?, lib.snd_ump_endpoint_info_free?)?;
            // SAFETY: an open handle and info objects of the matching kind; strings copied at once
            unsafe {
                if (lib.snd_seq_get_ump_endpoint_info?)(self.handle, client as c_int, ep.p) < 0 {
                    return None;
                }
                let revision = (lib.snd_ump_endpoint_info_get_sw_revision?)(ep.p);
                let revision: [u8; 4] = if revision.is_null() { [0; 4] } else { *(revision as *const [u8; 4]) };
                let manufacturer = (lib.snd_ump_endpoint_info_get_manufacturer_id?)(ep.p);
                let nonzero = |v: c_uint| (v != 0).then_some(v.min(u16::MAX as c_uint) as u16);
                let mut info = UmpEndpointInfo {
                    name: text((lib.snd_ump_endpoint_info_get_name?)(ep.p)),
                    product_instance_id: text((lib.snd_ump_endpoint_info_get_product_id?)(ep.p)),
                    // OPEN: the byte order of ALSA's manufacturer id (taken as the 3 SysEx id bytes, high first)
                    manufacturer: hex_or_none(&manufacturer.to_be_bytes()[1..]),
                    family: nonzero((lib.snd_ump_endpoint_info_get_family_id?)(ep.p)),
                    model: nonzero((lib.snd_ump_endpoint_info_get_model_id?)(ep.p)),
                    version: hex_or_none(&revision),
                    static_blocks: (lib.snd_ump_endpoint_info_get_flags?)(ep.p) & UMP_EP_STATIC_BLOCKS != 0,
                    blocks: vec![],
                };
                let blocks = (lib.snd_ump_endpoint_info_get_num_blocks?)(ep.p).min(32);
                let get_block = lib.snd_seq_get_ump_block_info?;
                for b in 0..blocks {
                    let Some(bi) = Info::new(lib.snd_ump_block_info_malloc?, lib.snd_ump_block_info_free?) else {
                        break;
                    };
                    if get_block(self.handle, client as c_int, b as c_int, bi.p) < 0 {
                        continue;
                    }
                    info.blocks.push(block_info(
                        b as u8,
                        &text((lib.snd_ump_block_info_get_name?)(bi.p)),
                        (lib.snd_ump_block_info_get_direction?)(bi.p),
                        (lib.snd_ump_block_info_get_first_group?)(bi.p),
                        (lib.snd_ump_block_info_get_num_groups?)(bi.p),
                        (lib.snd_ump_block_info_get_flags?)(bi.p),
                        (lib.snd_ump_block_info_get_ui_hint?)(bi.p),
                    ));
                }
                Some(info.limited())
            }
        }

        /// One event read from the sequencer as a record (None: not MIDI data, or an announcement – `true` then)
        fn record(&mut self, ev: &Event, now: u64) -> (Option<MidiRecord>, bool) {
            if ev.source.client == 0 {
                return (None, event::announces_change(ev.type_));
            }
            let id = if ev.dest.port == self.vm_in {
                VM_PORT_ID
            } else {
                let id = endpoint_id(ev.source.client, ev.source.port);
                if !self.connected.contains_key(&id) {
                    return (None, false);
                }
                id
            };
            // The subscription stamps our queue's real time; else (the VM's pair) the time it arrived
            let time = if ev.flags & TIME_STAMP_REAL != 0 && ev.queue as c_int == self.queue {
                let t = self.queue_base + ev.time[0] as u64 * 1_000_000_000 + ev.time[1] as u64;
                t.min(now)
            } else {
                now
            };
            if ev.flags & EVENT_UMP != 0 {
                let words: Vec<u32> = (0..4)
                    .map(|k| u32::from_ne_bytes(ev.data[k * 4..k * 4 + 4].try_into().unwrap_or_default()))
                    .collect();
                let n = MidiRecord::message_words(words[0]);
                let version = self.versions.get(&ev.source.client).copied().unwrap_or(1);
                let protocol = if version == 2 { 2 } else { 1 };
                return (Some(MidiRecord { id, protocol, time: Some(time), words: words[..n].to_vec() }), false);
            }
            let mut data = [0u8; 12];
            data.copy_from_slice(&ev.data[..12]);
            let sysex = if ev.flags & LENGTH_MASK == LENGTH_VARIABLE {
                let len = u32::from_ne_bytes([data[0], data[1], data[2], data[3]]) as usize;
                let ptr = u64::from_ne_bytes(data[4..12].try_into().unwrap_or_default()) as usize as *const u8;
                if ptr.is_null() || len > 1 << 20 {
                    None
                } else {
                    // SAFETY: libasound's variable data of this event, `len` bytes, valid until the next read
                    Some(unsafe { std::slice::from_raw_parts(ptr, len) }.to_vec())
                }
            } else {
                None
            };
            let Some(m) = SeqEvent::parse(ev.type_, &data, sysex.as_deref()) else { return (None, false) };
            let mut words = vec![];
            self.sysex.entry(id).or_default().push(&m, &mut words);
            if words.is_empty() {
                return (None, false);
            }
            (Some(MidiRecord { id, protocol: 1, time: Some(time), words }), false)
        }
    }

    impl AlsaMidi {
        /// Reads the list again: subscribes to new readable ports, lets go of gone ones, sends the list when it changed
        /// (or always: `force`, after a welcome)
        fn rescan(&self, force: bool) {
            let lib = self.lib;
            let mut s = self.seq.lock().unwrap_or_else(|e| e.into_inner());
            let ports = s.ports(lib);
            s.versions = ports.iter().map(|p| (p.client, p.midi_version)).collect();
            let mut list = endpoint_list(&ports, s.me, if s.ump { 2 } else { 1 });
            let wanted: HashMap<u32, (u8, u8)> =
                list.iter().filter(|e| e.direction == "in").filter_map(|e| Some((e.id, address(e.id)?))).collect();
            let gone: Vec<u32> = s.connected.keys().filter(|id| !wanted.contains_key(id)).copied().collect();
            for id in gone {
                if let Some((client, port)) = s.connected.remove(&id) {
                    s.subscribe(lib, client, port, false); // fails when the port is gone already: fine
                }
                s.sysex.remove(&id);
            }
            for (&id, &(client, port)) in &wanted {
                if !s.connected.contains_key(&id) {
                    if s.subscribe(lib, client, port, true) {
                        s.connected.insert(id, (client, port));
                    } else {
                        let name = list.iter().find(|e| e.id == id).map(|e| e.name.clone()).unwrap_or_default();
                        (self.log)(&format!("MIDI: cannot listen to \"{name}\""));
                    }
                }
            }
            let connected = s.connected.clone();
            list.retain(|e| e.direction != "in" || e.id == VM_PORT_ID || connected.contains_key(&e.id));
            s.destinations = ports
                .iter()
                .filter(|p| list.iter().any(|e| e.direction == "out" && e.id == p.id()))
                .map(|p| (p.id(), p.midi_version > 0))
                .collect();
            if !force && list == s.list {
                return;
            }
            if list != s.list {
                let names: Vec<String> = list
                    .iter()
                    .skip(2)
                    .map(|e| format!("{} {}", if e.direction == "in" { "→" } else { "←" }, e.name))
                    .collect();
                (self.log)(&format!("MIDI: {}", if names.is_empty() { "no devices".into() } else { names.join(", ") }));
            }
            s.list = list;
            let (Some(sender), Ok(json)) =
                (s.sender.clone(), serde_json::to_vec(&serde_json::json!({"endpoints": s.list})))
            else {
                return;
            };
            drop(s);
            sender.send(AgentMessage::MidiEndpoints, &json);
        }

        /// Waits for events (and the time for a rescan after announcements settled), sends their records to the guest
        fn read_loop(&self) {
            let lib = self.lib;
            let fds: Vec<libc::pollfd> = {
                let s = self.seq.lock().unwrap_or_else(|e| e.into_inner());
                // SAFETY: an open handle; the buffer has room for `n` descriptors
                unsafe {
                    let n = (lib.snd_seq_poll_descriptors_count)(s.handle, libc::POLLIN).max(0) as usize;
                    let mut fds = vec![libc::pollfd { fd: -1, events: 0, revents: 0 }; n];
                    let got = (lib.snd_seq_poll_descriptors)(s.handle, fds.as_mut_ptr(), n as c_uint, libc::POLLIN);
                    fds.truncate(got.max(0) as usize);
                    fds
                }
            };
            let mut rescan_at: Option<Instant> = None;
            loop {
                let timeout =
                    rescan_at.map_or(1000, |t| t.saturating_duration_since(Instant::now()).as_millis() as c_int);
                let mut ready = fds.clone();
                // SAFETY: valid descriptors from libasound
                unsafe { libc::poll(ready.as_mut_ptr(), ready.len() as libc::nfds_t, timeout) };
                let (records, sender) = {
                    let mut s = self.seq.lock().unwrap_or_else(|e| e.into_inner());
                    let mut records = vec![];
                    let now = monotonic_ns();
                    for _ in 0..4096 {
                        let mut ev: *mut Event = std::ptr::null_mut();
                        // SAFETY: an open handle; the event lives in libasound's buffer until the next read
                        let rc = unsafe {
                            match lib.snd_seq_ump_event_input.filter(|_| s.ump) {
                                Some(f) => f(s.handle, &mut ev),
                                None => (lib.snd_seq_event_input)(s.handle, &mut ev),
                            }
                        };
                        if rc == -libc::ENOSPC {
                            (self.log)("MIDI: the sequencer's input overran – events were lost");
                            continue;
                        }
                        if rc < 0 || ev.is_null() {
                            break;
                        }
                        // A legacy event is the first 28 bytes; the rest of the copy is not read for it
                        // SAFETY: libasound returned a valid event of at least 28 bytes; in UMP mode 32
                        let e = unsafe {
                            if s.ump {
                                *ev
                            } else {
                                let mut e = Event::default();
                                std::ptr::copy_nonoverlapping(ev as *const u8, &mut e as *mut Event as *mut u8, 28);
                                e
                            }
                        };
                        let (record, announce) = s.record(&e, now);
                        records.extend(record);
                        if announce && rescan_at.is_none() {
                            // Several announcements come for one device; look once they settled
                            rescan_at = Some(Instant::now() + Duration::from_millis(300));
                        }
                    }
                    (records, s.sender.clone())
                };
                if let (Some(sender), false) = (sender, records.is_empty()) {
                    sender.send_midi(&records);
                }
                if rescan_at.is_some_and(|t| Instant::now() >= t) {
                    rescan_at = None;
                    self.rescan(false);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn port(client: u8, port: u8, client_name: &str, port_name: &str, caps: u32) -> HostPort {
        HostPort {
            client,
            port,
            client_name: client_name.into(),
            port_name: port_name.into(),
            caps,
            midi_version: 0,
            ump: None,
        }
    }

    const RW: u32 = caps::READ | caps::SUBS_READ | caps::WRITE | caps::SUBS_WRITE;

    #[test]
    fn ids_and_addresses() {
        assert_eq!(endpoint_id(0, 0), 1);
        assert_eq!(endpoint_id(20, 0), 0x1401);
        assert_eq!(endpoint_id(255, 255), 0x1_0000);
        assert_eq!(address(0x1401), Some((20, 0)));
        assert_eq!(address(0x1_0000), Some((255, 255)));
        assert_eq!(address(VM_PORT_ID), None);
        assert_eq!(address(0x1_0001), None);
    }

    #[test]
    fn the_list() {
        let mut ump = port(24, 0, "MIDI 2.0 Synth", "Group 1 (Main)", RW);
        ump.midi_version = 2;
        let ports = vec![
            port(0, 0, "System", "Timer", caps::READ | caps::SUBS_READ),
            port(0, 1, "System", "Announce", caps::READ | caps::SUBS_READ),
            port(14, 0, "Midi Through", "Midi Through Port-0", RW),
            port(20, 0, "Keystation 49", "Keystation 49 MIDI 1", caps::READ | caps::SUBS_READ),
            port(128, 0, "omaflake work", "omaflake work in", caps::WRITE | caps::SUBS_WRITE),
            port(129, 0, "omaflake other", "omaflake other out", caps::READ | caps::SUBS_READ),
            port(130, 0, "Synth\u{1b}", "out", caps::WRITE), // nobody may subscribe: direct writes only
            port(131, 0, "Hidden", "x", RW | caps::NO_EXPORT),
            port(132, 0, "Off", "x", RW | caps::INACTIVE),
            ump,
        ];
        let list = endpoint_list(&ports, 128, 2);
        let short: Vec<(u32, &str, &str, u8)> =
            list.iter().map(|e| (e.id, e.direction.as_str(), e.name.as_str(), e.protocol)).collect();
        assert_eq!(
            short,
            [
                (0, "in", VM_PAIR_NAME, 2),
                (0, "out", VM_PAIR_NAME, 2),
                (0x1401, "in", "Keystation 49: Keystation 49 MIDI 1", 1),
                (0x1801, "in", "MIDI 2.0 Synth: Group 1 (Main)", 2),
                (0x0E01, "in", "Midi Through: Midi Through Port-0", 1),
                (0x1801, "out", "MIDI 2.0 Synth: Group 1 (Main)", 2),
                (0x0E01, "out", "Midi Through: Midi Through Port-0", 1),
            ]
        );
    }

    #[test]
    fn ump_blocks() {
        let b = block_info(3, "Main", 3, 0, 2, 1 | 2, 1);
        assert_eq!((b.direction.as_str(), b.midi1.as_str(), b.ui_hint.as_str()), ("both", "low-speed", "receiver"));
        assert_eq!(block_info(0, "", 1, 0, 1, 0, 9).midi1, "no");
        assert_eq!(block_info(0, "", 2, 0, 1, 1, 2).direction, "out");
        assert_eq!(hex_or_none(&[0x00, 0x20, 0x1f]).as_deref(), Some("00201f"));
        assert_eq!(hex_or_none(&[0, 0, 0, 0]), None);
    }

    /// MIDI 1.0 events of a keyboard as UMP
    #[test]
    fn events_to_ump() {
        let mut t = ToUmp::new();
        let mut out = vec![];
        for ev in [
            SeqEvent::NoteOn { channel: 0, note: 60, velocity: 100 },
            SeqEvent::NoteOff { channel: 1, note: 60, velocity: 0 },
            SeqEvent::Controller { channel: 2, param: 7, value: 127 },
            SeqEvent::ProgramChange { channel: 3, value: 5 },
            SeqEvent::PitchBend { channel: 0, value: 0 },
            SeqEvent::PitchBend { channel: 0, value: -8192 },
            SeqEvent::Clock,
            SeqEvent::SongPosition(0x1234),
        ] {
            t.push(&ev, &mut out);
        }
        assert_eq!(
            out,
            [0x2090_3C64, 0x2081_3C00, 0x20B2_077F, 0x20C3_0500, 0x20E0_0040, 0x20E0_0000, 0x10F8_0000, 0x10F2_3424]
        );
        // the same note on as the "midi" vectors' first record
        assert_eq!(out[0], 546323556);
    }

    #[test]
    fn sysex_in_pieces() {
        let mut t = ToUmp::new();
        let mut out = vec![];
        // A short one: one complete packet
        t.push(&SeqEvent::SysEx(vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]), &mut out);
        assert_eq!(out, [0x3004_7E7F, 0x0601_0000]);
        out.clear();
        // 9 data bytes in two pieces: start (6), end (3)
        t.push(&SeqEvent::SysEx(vec![0xF0, 1, 2, 3, 4]), &mut out);
        assert!(out.is_empty(), "held back until the end is known");
        t.push(&SeqEvent::SysEx(vec![5, 6, 7, 8, 9, 0xF7]), &mut out);
        assert_eq!(out, [0x3016_0102, 0x0304_0506, 0x3033_0708, 0x0900_0000]);
        out.clear();
        // exactly 12 bytes: start, end of 6
        t.push(&SeqEvent::SysEx([vec![0xF0], (1..=12).collect(), vec![0xF7]].concat()), &mut out);
        assert_eq!(out.len(), 4);
        assert_eq!((out[0] >> 16 & 0xFF, out[2] >> 16 & 0xFF), (0x16, 0x36));
        // and back: the same bytes as SysEx pieces
        let back: Vec<u8> =
            from_ump(&out).into_iter().flat_map(|e| if let SeqEvent::SysEx(b) = e { b } else { vec![] }).collect();
        assert_eq!(back, [vec![0xF0], (1..=12).collect(), vec![0xF7]].concat());
        // an empty one
        out.clear();
        t.push(&SeqEvent::SysEx(vec![0xF0, 0xF7]), &mut out);
        assert_eq!(out, [0x3000_0000, 0]);
    }

    #[test]
    fn ump_back_to_events() {
        let events = from_ump(&[0x2090_3C64, 0x20E0_0040, 0x10FA_0000, 0x3004_7E7F, 0x0601_0000]);
        assert_eq!(
            events,
            [
                SeqEvent::NoteOn { channel: 0, note: 60, velocity: 100 },
                SeqEvent::PitchBend { channel: 0, value: 0 },
                SeqEvent::Start,
                SeqEvent::SysEx(vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]),
            ]
        );
        // every MIDI 1.0 event survives the round trip
        let all = [
            SeqEvent::NoteOn { channel: 15, note: 127, velocity: 1 },
            SeqEvent::KeyPressure { channel: 4, note: 10, value: 20 },
            SeqEvent::ChannelPressure { channel: 5, value: 30 },
            SeqEvent::PitchBend { channel: 6, value: 8191 },
            SeqEvent::QuarterFrame(0x35),
            SeqEvent::SongSelect(3),
            SeqEvent::TuneRequest,
            SeqEvent::Continue,
            SeqEvent::Stop,
            SeqEvent::ActiveSensing,
            SeqEvent::Reset,
        ];
        let mut words = vec![];
        let mut t = ToUmp::new();
        for e in &all {
            t.push(e, &mut words);
        }
        assert_eq!(from_ump(&words), all);
    }

    /// MIDI 2.0 channel voice messages for a MIDI 1.0 port (UMP spec 1.1, translation)
    #[test]
    fn midi2_down_to_midi1() {
        let events = from_ump(&[
            0x4090_3C00,
            0xFFFF_0000, // note on, full velocity
            0x4090_3C00,
            0x0100_0000, // note on, velocity too small for 7 bits: 1, not a note off
            0x4081_3C00,
            0x8000_0000, // note off, velocity 64
            0x40B2_0700,
            0x8000_0000, // CC 7 = 64
            0x40C3_0001,
            0x0500_0203, // program 5, bank 2/3
            0x40E0_0000,
            0x8000_0000, // pitch bend centre
            0x4020_0001,
            0x8000_0000, // RPN 0/1 = 8192
            0x40F0_0000,
            0, // per-note management: dropped
        ]);
        let cc = |channel, param, value| SeqEvent::Controller { channel, param, value };
        assert_eq!(
            events,
            [
                SeqEvent::NoteOn { channel: 0, note: 60, velocity: 127 },
                SeqEvent::NoteOn { channel: 0, note: 60, velocity: 1 },
                SeqEvent::NoteOff { channel: 1, note: 60, velocity: 64 },
                cc(2, 7, 64),
                cc(3, 0, 2),
                cc(3, 32, 3),
                SeqEvent::ProgramChange { channel: 3, value: 5 },
                SeqEvent::PitchBend { channel: 0, value: 0 },
                cc(0, 101, 0),
                cc(0, 100, 1),
                cc(0, 6, 64),
                cc(0, 38, 0),
            ]
        );
        // utility, SysEx8, stream messages and a cut-off message: nothing
        assert!(from_ump(&[0x0000_0000, 0x5000_0000, 0, 0, 0, 0xF000_0000, 0, 0, 0, 0x4090_3C00]).is_empty());
    }

    /// The sequencer's event data both ways
    #[test]
    fn sequencer_event_data() {
        let all = [
            SeqEvent::NoteOn { channel: 9, note: 36, velocity: 90 },
            SeqEvent::NoteOff { channel: 9, note: 36, velocity: 0 },
            SeqEvent::Controller { channel: 1, param: 64, value: 127 },
            SeqEvent::ProgramChange { channel: 2, value: 10 },
            SeqEvent::PitchBend { channel: 3, value: -100 },
            SeqEvent::Rpn { channel: 0, param: 0, value: 0x3FFF },
            SeqEvent::SongPosition(1000),
            SeqEvent::Clock,
        ];
        for e in all {
            let (t, d) = e.encode();
            assert_eq!(SeqEvent::parse(t, &d, None), Some(e.clone()), "{e:?}");
        }
        let (t, d) = SeqEvent::SysEx(vec![0xF0, 1, 0xF7]).encode();
        assert_eq!((t, &d[..4]), (event::SYSEX, &3u32.to_le_bytes()[..]));
        assert_eq!(SeqEvent::parse(t, &d, Some(&[0xF0, 1, 0xF7])), Some(SeqEvent::SysEx(vec![0xF0, 1, 0xF7])));
        // ALSA's note with a duration counts as a note on; announcements and others are not data
        let (_, d) = SeqEvent::NoteOn { channel: 0, note: 1, velocity: 2 }.encode();
        assert_eq!(SeqEvent::parse(event::NOTE, &d, None), Some(SeqEvent::NoteOn { channel: 0, note: 1, velocity: 2 }));
        assert_eq!(SeqEvent::parse(event::PORT_START, &d, None), None);
        assert!(event::announces_change(event::PORT_EXIT) && !event::announces_change(event::SYSEX));
        // a 14-bit controller becomes MSB and LSB
        let mut words = vec![];
        ToUmp::new().push(&SeqEvent::Control14 { channel: 0, param: 1, value: 0x2001 }, &mut words);
        assert_eq!(words, [0x20B0_0140, 0x20B0_2101]);
    }
}
