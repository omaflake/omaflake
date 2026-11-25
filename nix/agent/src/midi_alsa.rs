// SPDX-License-Identifier: AGPL-3.0-or-later
//! MIDI in NixOS guests (spec §11.9, plan B161, C7): the ALSA sequencer. The agent is the sequencer client
//! "omaflake" with one port per endpoint of the Mac: a source of the Mac (a keyboard) becomes a port the guest's apps
//! read from (readable, subscribable), a destination of the Mac (a synthesizer) a port they write to.
//!
//! Where the sequencer speaks UMP (alsa-lib 1.2.10+, Linux 6.5+ with CONFIG_SND_SEQ_UMP) the client is a UMP MIDI 2.0
//! client: the Mac's packets pass unchanged and the kernel converts for legacy clients; an endpoint's `ump` becomes
//! the client's UMP endpoint and block info. Otherwise the data are converted to and from MIDI 1.0 events
//! (midi1.rs). The UMP functions are looked up at run time (dlsym), so an older libasound still works.
//!
//! Data from the Mac with a time stamp are scheduled on the client's real-time queue for the time midi_clock.rs maps
//! them to (relative to now); data played in the guest are stamped with CLOCK_MONOTONIC when the agent reads them.
//!
//! libasound is linked (package.nix); struct layouts and constants are those of alsa-lib 1.2.16's headers
//! (seq_event.h, seq.h, seqmid.h, ump.h, seq_midi_event.h). OPEN until checked in a NixOS guest: everything that needs
//! a running sequencer – the ports appearing in apps (aconnect -l), data both ways, the queue's timing, the UMP mode
//! on the guest's kernel (whether its config has SND_SEQ_UMP), the UMP endpoint and block info as `aconnect -l` / a
//! UMP-aware app shows them, and that the kernel converts between our UMP events and legacy clients.

use crate::midi::{Endpoint, Ump, clean};
use crate::midi_clock::{ClockMapper, now_ns};
use crate::midi1::{Midi1, Midi1Parser, protocol_of, ump_to_midi1};
use crate::ump::{Record, message_words};
use std::collections::HashMap;
use std::ffi::{CString, c_char, c_int, c_long, c_uint, c_void};
use std::sync::{Arc, Mutex, MutexGuard};

/// snd_seq_t, snd_midi_event_t: opaque
#[repr(C)]
struct Seq {
    _private: [u8; 0],
}
#[repr(C)]
struct MidiEvent {
    _private: [u8; 0],
}

#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Addr {
    client: u8,
    port: u8,
}

/// snd_seq_real_time_t (the union snd_seq_timestamp_t is this size)
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct RealTime {
    sec: u32,
    nsec: u32,
}

/// snd_seq_event_t: 16 bytes of header and the 12-byte data union (aligned to 4; `ext` is a packed u32 length and a
/// pointer)
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct Event {
    kind: u8,
    flags: u8,
    tag: u8,
    queue: u8,
    time: RealTime,
    source: Addr,
    dest: Addr,
    data: [u32; 3],
}

/// snd_seq_ump_event_t: the same header and up to four UMP words
#[repr(C)]
#[derive(Clone, Copy, Default)]
struct UmpEvent {
    kind: u8,
    flags: u8,
    tag: u8,
    queue: u8,
    time: RealTime,
    source: Addr,
    dest: Addr,
    ump: [u32; 4],
}

const _: () = assert!(std::mem::size_of::<Event>() == 28 && std::mem::size_of::<UmpEvent>() == 32);

// seq.h, seq_event.h
const SND_SEQ_OPEN_DUPLEX: c_int = 3;
const SND_SEQ_NONBLOCK: c_int = 1;
const SND_SEQ_PORT_CAP_READ: c_uint = 1 << 0;
const SND_SEQ_PORT_CAP_WRITE: c_uint = 1 << 1;
const SND_SEQ_PORT_CAP_SUBS_READ: c_uint = 1 << 5;
const SND_SEQ_PORT_CAP_SUBS_WRITE: c_uint = 1 << 6;
const SND_SEQ_PORT_TYPE_MIDI_GENERIC: c_uint = 1 << 1;
const SND_SEQ_PORT_TYPE_APPLICATION: c_uint = 1 << 20;
const SND_SEQ_TIME_STAMP_REAL: u8 = 1 << 0;
const SND_SEQ_TIME_MODE_REL: u8 = 1 << 1;
const SND_SEQ_EVENT_LENGTH_VARIABLE: u8 = 1 << 2;
const SND_SEQ_EVENT_LENGTH_MASK: u8 = 3 << 2;
const SND_SEQ_EVENT_UMP: u8 = 1 << 5;
const SND_SEQ_EVENT_START: c_int = 30;
const SND_SEQ_EVENT_SYSEX: u8 = 130;
const SND_SEQ_EVENT_NONE: u8 = 255;
const SND_SEQ_ADDRESS_SUBSCRIBERS: u8 = 254;
const SND_SEQ_ADDRESS_UNKNOWN: u8 = 253;
const SND_SEQ_QUEUE_DIRECT: u8 = 253;
const SND_SEQ_CLIENT_UMP_MIDI_2_0: c_int = 2;
// ump.h
const SND_UMP_EP_INFO_STATIC_BLOCKS: c_uint = 0x01;
const SND_UMP_EP_INFO_PROTO_MIDI1: c_uint = 0x0100;
const SND_UMP_EP_INFO_PROTO_MIDI2: c_uint = 0x0200;
const SND_UMP_EP_INFO_DEFAULT_VERSION: c_uint = 0x0101;
const SND_UMP_BLOCK_IS_MIDI1: c_uint = 1 << 0;
const SND_UMP_BLOCK_IS_LOWSPEED: c_uint = 1 << 1;
/// ALSA port names: 64 bytes with the NUL
const PORT_NAME_MAX: usize = 63;

#[link(name = "asound")]
unsafe extern "C" {
    fn snd_seq_open(handle: *mut *mut Seq, name: *const c_char, streams: c_int, mode: c_int) -> c_int;
    fn snd_seq_set_client_name(seq: *mut Seq, name: *const c_char) -> c_int;
    fn snd_seq_create_simple_port(seq: *mut Seq, name: *const c_char, caps: c_uint, kind: c_uint) -> c_int;
    fn snd_seq_delete_simple_port(seq: *mut Seq, port: c_int) -> c_int;
    fn snd_seq_alloc_named_queue(seq: *mut Seq, name: *const c_char) -> c_int;
    fn snd_seq_control_queue(seq: *mut Seq, q: c_int, kind: c_int, value: c_int, ev: *mut Event) -> c_int;
    fn snd_seq_drain_output(seq: *mut Seq) -> c_int;
    fn snd_seq_event_output_direct(seq: *mut Seq, ev: *mut Event) -> c_int;
    fn snd_seq_event_input(seq: *mut Seq, ev: *mut *mut Event) -> c_int;
    fn snd_seq_poll_descriptors_count(seq: *mut Seq, events: i16) -> c_int;
    fn snd_seq_poll_descriptors(seq: *mut Seq, pfds: *mut libc::pollfd, space: c_uint, events: i16) -> c_int;
    fn snd_midi_event_new(bufsize: usize, dev: *mut *mut MidiEvent) -> c_int;
    fn snd_midi_event_reset_encode(dev: *mut MidiEvent);
    fn snd_midi_event_no_status(dev: *mut MidiEvent, on: c_int);
    fn snd_midi_event_encode(dev: *mut MidiEvent, buf: *const u8, count: c_long, ev: *mut Event) -> c_long;
    fn snd_midi_event_decode(dev: *mut MidiEvent, buf: *mut u8, count: c_long, ev: *const Event) -> c_long;
}

/// The UMP part of alsa-lib (1.2.10+), resolved at run time
struct UmpApi {
    set_client_midi_version: unsafe extern "C" fn(*mut Seq, c_int) -> c_int,
    info: Option<UmpInfoApi>,
}

/// UMP endpoint and block info (snd_ump_endpoint_info_t, snd_ump_block_info_t: opaque, malloc'ed by alsa-lib)
struct UmpInfoApi {
    set_endpoint_info: unsafe extern "C" fn(*mut Seq, *const c_void) -> c_int,
    set_block_info: unsafe extern "C" fn(*mut Seq, c_int, *const c_void) -> c_int,
    ep_malloc: unsafe extern "C" fn(*mut *mut c_void) -> c_int,
    ep_free: unsafe extern "C" fn(*mut c_void),
    ep_set_name: unsafe extern "C" fn(*mut c_void, *const c_char),
    ep_set_product_id: unsafe extern "C" fn(*mut c_void, *const c_char),
    ep_set_flags: unsafe extern "C" fn(*mut c_void, c_uint),
    ep_set_protocol_caps: unsafe extern "C" fn(*mut c_void, c_uint),
    ep_set_protocol: unsafe extern "C" fn(*mut c_void, c_uint),
    ep_set_num_blocks: unsafe extern "C" fn(*mut c_void, c_uint),
    ep_set_version: unsafe extern "C" fn(*mut c_void, c_uint),
    ep_set_manufacturer_id: unsafe extern "C" fn(*mut c_void, c_uint),
    ep_set_family_id: unsafe extern "C" fn(*mut c_void, c_uint),
    ep_set_model_id: unsafe extern "C" fn(*mut c_void, c_uint),
    ep_set_sw_revision: unsafe extern "C" fn(*mut c_void, *const u8),
    blk_malloc: unsafe extern "C" fn(*mut *mut c_void) -> c_int,
    blk_free: unsafe extern "C" fn(*mut c_void),
    blk_set_block_id: unsafe extern "C" fn(*mut c_void, c_uint),
    blk_set_active: unsafe extern "C" fn(*mut c_void, c_uint),
    blk_set_flags: unsafe extern "C" fn(*mut c_void, c_uint),
    blk_set_direction: unsafe extern "C" fn(*mut c_void, c_uint),
    blk_set_first_group: unsafe extern "C" fn(*mut c_void, c_uint),
    blk_set_num_groups: unsafe extern "C" fn(*mut c_void, c_uint),
    blk_set_ui_hint: unsafe extern "C" fn(*mut c_void, c_uint),
    blk_set_name: unsafe extern "C" fn(*mut c_void, *const c_char),
}

/// A function of the loaded libasound as the function-pointer type `F`; None if this libasound lacks it
///
/// # Safety
/// `F` must be the C signature of the function named (alsa-lib's headers)
unsafe fn lookup<F: Copy>(name: &str) -> Option<F> {
    assert_eq!(std::mem::size_of::<F>(), std::mem::size_of::<*mut c_void>());
    let c = CString::new(name).ok()?;
    // SAFETY: dlsym with a NUL-terminated name in the process's global symbols (libasound is linked)
    let p = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c.as_ptr()) };
    // SAFETY: a found symbol is the function of that name; the caller vouches for its signature
    (!p.is_null()).then(|| unsafe { std::mem::transmute_copy::<*mut c_void, F>(&p) })
}

macro_rules! sym {
    ($name:literal) => {
        // SAFETY: each field the result is assigned to declares the function's signature from alsa-lib's headers
        unsafe { lookup($name) }
    };
}

fn ump_api() -> Option<UmpApi> {
    let info = (|| {
        Some(UmpInfoApi {
            set_endpoint_info: sym!("snd_seq_set_ump_endpoint_info")?,
            set_block_info: sym!("snd_seq_set_ump_block_info")?,
            ep_malloc: sym!("snd_ump_endpoint_info_malloc")?,
            ep_free: sym!("snd_ump_endpoint_info_free")?,
            ep_set_name: sym!("snd_ump_endpoint_info_set_name")?,
            ep_set_product_id: sym!("snd_ump_endpoint_info_set_product_id")?,
            ep_set_flags: sym!("snd_ump_endpoint_info_set_flags")?,
            ep_set_protocol_caps: sym!("snd_ump_endpoint_info_set_protocol_caps")?,
            ep_set_protocol: sym!("snd_ump_endpoint_info_set_protocol")?,
            ep_set_num_blocks: sym!("snd_ump_endpoint_info_set_num_blocks")?,
            ep_set_version: sym!("snd_ump_endpoint_info_set_version")?,
            ep_set_manufacturer_id: sym!("snd_ump_endpoint_info_set_manufacturer_id")?,
            ep_set_family_id: sym!("snd_ump_endpoint_info_set_family_id")?,
            ep_set_model_id: sym!("snd_ump_endpoint_info_set_model_id")?,
            ep_set_sw_revision: sym!("snd_ump_endpoint_info_set_sw_revision")?,
            blk_malloc: sym!("snd_ump_block_info_malloc")?,
            blk_free: sym!("snd_ump_block_info_free")?,
            blk_set_block_id: sym!("snd_ump_block_info_set_block_id")?,
            blk_set_active: sym!("snd_ump_block_info_set_active")?,
            blk_set_flags: sym!("snd_ump_block_info_set_flags")?,
            blk_set_direction: sym!("snd_ump_block_info_set_direction")?,
            blk_set_first_group: sym!("snd_ump_block_info_set_first_group")?,
            blk_set_num_groups: sym!("snd_ump_block_info_set_num_groups")?,
            blk_set_ui_hint: sym!("snd_ump_block_info_set_ui_hint")?,
            blk_set_name: sym!("snd_ump_block_info_set_name")?,
        })
    })();
    Some(UmpApi { set_client_midi_version: sym!("snd_seq_set_client_midi_version")?, info })
}

/// A C string for ALSA: control characters dropped (clean), at most `max` bytes, cut at a character boundary
fn c_name(s: &str, max: usize) -> CString {
    let mut s = clean(s);
    while s.len() > max {
        s.pop();
    }
    CString::new(s).unwrap_or_default()
}

/// The ALSA side of one UMP description: the values the endpoint and block info setters take
#[derive(Debug, PartialEq, Eq)]
struct UmpInfo {
    name: CString,
    product_id: CString,
    flags: c_uint,
    protocol: c_uint,
    manufacturer: c_uint,
    family: c_uint,
    model: c_uint,
    revision: [u8; 4],
    num_blocks: c_uint,
    /// (block number = the block's id, flags, direction, first group, groups, UI hint, name)
    blocks: Vec<(c_uint, c_uint, c_uint, c_uint, c_uint, c_uint, CString)>,
}

impl UmpInfo {
    /// The description for ALSA. The manufacturer's three bytes go into one number as byte 1 << 16 | byte 2 << 8 |
    /// byte 3 (OPEN: the order the kernel's snd_ump_endpoint_info expects is not documented). Block numbers are the
    /// blocks' ids and the block count the highest id + 1 (UMP numbers blocks from 0; gaps stay empty – OPEN how
    /// apps show them).
    fn new(u: &Ump, protocol: u8) -> UmpInfo {
        let m = u.manufacturer_bytes();
        let blocks: Vec<_> = u
            .sorted_blocks()
            .into_iter()
            .map(|b| {
                let flags = match b.midi1.as_str() {
                    "yes" => SND_UMP_BLOCK_IS_MIDI1,
                    "low-speed" => SND_UMP_BLOCK_IS_MIDI1 | SND_UMP_BLOCK_IS_LOWSPEED,
                    _ => 0,
                };
                let direction = match b.direction.as_str() {
                    "in" => 1, // SND_UMP_DIR_INPUT
                    "out" => 2,
                    _ => 3,
                };
                let hint = match b.ui_hint.as_str() {
                    "receiver" => 1, // SND_UMP_BLOCK_UI_HINT_RECEIVER
                    "sender" => 2,
                    "both" => 3,
                    _ => 0,
                };
                let name = c_name(&b.name, crate::midi::UMP_NAME_MAX);
                (b.id as c_uint, flags, direction, b.first_group as c_uint, b.groups as c_uint, hint, name)
            })
            .collect();
        UmpInfo {
            name: c_name(&u.name, crate::midi::UMP_NAME_MAX),
            product_id: c_name(&u.product_instance_id, crate::midi::UMP_PRODUCT_ID_MAX),
            flags: if u.static_blocks { SND_UMP_EP_INFO_STATIC_BLOCKS } else { 0 },
            protocol: if protocol == 2 { SND_UMP_EP_INFO_PROTO_MIDI2 } else { SND_UMP_EP_INFO_PROTO_MIDI1 },
            manufacturer: (m[0] as c_uint) << 16 | (m[1] as c_uint) << 8 | m[2] as c_uint,
            family: u.family.unwrap_or(0) as c_uint,
            model: u.model.unwrap_or(0) as c_uint,
            revision: u.version_bytes(),
            num_blocks: blocks.iter().map(|b| b.0 + 1).max().unwrap_or(0),
            blocks,
        }
    }

    /// No description: the client's own name, no blocks (what the client has when no endpoint carries `ump`)
    fn plain() -> UmpInfo {
        UmpInfo {
            name: c_name("omaflake", 98),
            product_id: CString::default(),
            flags: 0,
            protocol: SND_UMP_EP_INFO_PROTO_MIDI2,
            manufacturer: 0,
            family: 0,
            model: 0,
            revision: [0; 4],
            num_blocks: 0,
            blocks: vec![],
        }
    }
}

/// A writable port's Mac destination (guest → Mac), as the input thread needs it
struct InPort {
    id: u32,
    protocol: u8,
    /// MIDI 1.0 bytes → UMP, for legacy events
    parser: Midi1Parser,
}

/// What the input thread and `deliver` share (alsa-lib's handle is not thread-safe: every call under this lock)
struct Inner {
    seq: *mut Seq,
    /// MIDI 1.0 bytes ↔ events (alsa-lib's parser; no running status)
    encoder: *mut MidiEvent,
    decoder: *mut MidiEvent,
    /// port number → destination on the Mac
    inputs: HashMap<c_int, InPort>,
}

// SAFETY: the handles are only used under the Mutex that holds Inner
unsafe impl Send for Inner {}

struct Port {
    info: Endpoint,
    port: c_int,
    /// Open SysEx7 streams per group (UMP → MIDI 1.0)
    sysex: [bool; 16],
}

pub struct Midi {
    inner: Arc<Mutex<Inner>>,
    /// (id, is a source on the Mac) → our port
    ports: HashMap<(u32, bool), Port>,
    /// The real-time queue for scheduled data (None: everything goes out at once)
    queue: Option<u8>,
    /// The client speaks UMP MIDI 2.0
    ump_mode: bool,
    ump_info_api: Option<UmpInfoApi>,
    /// The UMP description the client has now (None: never set)
    ump_info: Option<UmpInfo>,
    clock: ClockMapper,
}

fn lock(inner: &Mutex<Inner>) -> MutexGuard<'_, Inner> {
    inner.lock().unwrap_or_else(|e| e.into_inner())
}

impl Midi {
    /// The sequencer client "omaflake"; fails without /dev/snd/seq (module snd-seq) or libasound
    pub fn new(out: fn(Record)) -> Result<Midi, String> {
        let mut seq: *mut Seq = std::ptr::null_mut();
        // SAFETY (all calls below): a handle opened here and used only by this thread until the input thread starts;
        // names are NUL-terminated
        let rc = unsafe { snd_seq_open(&mut seq, c"default".as_ptr(), SND_SEQ_OPEN_DUPLEX, SND_SEQ_NONBLOCK) };
        if rc < 0 || seq.is_null() {
            return Err(format!("ALSA sequencer: error {rc} (module snd-seq loaded?)"));
        }
        unsafe { snd_seq_set_client_name(seq, c"omaflake".as_ptr()) };
        let q = unsafe { snd_seq_alloc_named_queue(seq, c"omaflake".as_ptr()) };
        let queue = if q >= 0
            && unsafe { snd_seq_control_queue(seq, q, SND_SEQ_EVENT_START, 0, std::ptr::null_mut()) } >= 0
            && unsafe { snd_seq_drain_output(seq) } >= 0
        {
            Some(q as u8)
        } else {
            crate::log(&format!("MIDI: no real-time queue ({q}) – data from the Mac are played at once"));
            None
        };
        let api = ump_api();
        let ump_mode =
            api.as_ref().is_some_and(|a| unsafe { (a.set_client_midi_version)(seq, SND_SEQ_CLIENT_UMP_MIDI_2_0) } == 0);
        crate::log(if ump_mode {
            "MIDI: ALSA sequencer client with UMP (MIDI 2.0)"
        } else {
            "MIDI: ALSA sequencer client with MIDI 1.0 events (no UMP in this kernel or libasound)"
        });
        let mut encoder = std::ptr::null_mut();
        let mut decoder = std::ptr::null_mut();
        if unsafe { snd_midi_event_new(256, &mut encoder) } < 0 || unsafe { snd_midi_event_new(256, &mut decoder) } < 0
        {
            return Err("ALSA MIDI event parser: out of memory".into());
        }
        unsafe { snd_midi_event_no_status(decoder, 1) };
        let count = unsafe { snd_seq_poll_descriptors_count(seq, libc::POLLIN) }.max(0) as usize;
        let mut fds = vec![libc::pollfd { fd: -1, events: 0, revents: 0 }; count];
        let n = unsafe { snd_seq_poll_descriptors(seq, fds.as_mut_ptr(), count as c_uint, libc::POLLIN) };
        fds.truncate(n.max(0) as usize);
        if fds.is_empty() {
            return Err("ALSA sequencer: no poll descriptor".into());
        }
        let inner = Arc::new(Mutex::new(Inner { seq, encoder, decoder, inputs: HashMap::new() }));
        let shared = inner.clone();
        std::thread::Builder::new()
            .name("midi-in".into())
            .spawn(move || input_thread(shared, fds, out))
            .map_err(|e| format!("MIDI input thread: {e}"))?;
        Ok(Midi {
            inner,
            ports: HashMap::new(),
            queue,
            ump_mode,
            ump_info_api: api.and_then(|a| a.info),
            ump_info: None,
            clock: ClockMapper::new(),
        })
    }

    /// Makes our ports match the Mac's list: new ones are created, gone or changed ones deleted. Returns how many
    /// exist and the names that failed.
    pub fn sync(&mut self, list: &[Endpoint]) -> (usize, Vec<String>) {
        let wanted: HashMap<(u32, bool), &Endpoint> = list.iter().map(|e| ((e.id, e.direction == "in"), e)).collect();
        let mut inner = lock(&self.inner);
        let stale: Vec<(u32, bool)> =
            self.ports.iter().filter(|(k, p)| wanted.get(k).is_none_or(|e| **e != p.info)).map(|(k, _)| *k).collect();
        for k in stale {
            if let Some(p) = self.ports.remove(&k) {
                inner.inputs.remove(&p.port);
                // SAFETY: a port of this client, under the lock
                unsafe { snd_seq_delete_simple_port(inner.seq, p.port) };
            }
        }
        let mut failed = vec![];
        for (k, e) in wanted {
            if self.ports.contains_key(&k) {
                continue;
            }
            let caps = if k.1 {
                SND_SEQ_PORT_CAP_READ | SND_SEQ_PORT_CAP_SUBS_READ
            } else {
                SND_SEQ_PORT_CAP_WRITE | SND_SEQ_PORT_CAP_SUBS_WRITE
            };
            let name = c_name(&e.name, PORT_NAME_MAX);
            // SAFETY: valid handle under the lock, NUL-terminated name
            let port = unsafe {
                snd_seq_create_simple_port(
                    inner.seq,
                    name.as_ptr(),
                    caps,
                    SND_SEQ_PORT_TYPE_MIDI_GENERIC | SND_SEQ_PORT_TYPE_APPLICATION,
                )
            };
            if port < 0 {
                crate::log(&format!("MIDI: cannot create port \"{}\": error {port}", clean(&e.name)));
                failed.push(clean(&e.name));
                continue;
            }
            if !k.1 {
                inner.inputs.insert(port, InPort { id: e.id, protocol: e.protocol, parser: Midi1Parser::new(0) });
            }
            self.ports.insert(k, Port { info: e.clone(), port, sysex: [false; 16] });
        }
        // The client's UMP description: the first endpoint that carries one (a sequencer client is one UMP endpoint)
        if self.ump_mode
            && let Some(api) = &self.ump_info_api
        {
            let want = match list.iter().find_map(|e| e.ump.as_ref().map(|u| (u, e.protocol))) {
                Some((u, p)) => UmpInfo::new(u, p),
                None => UmpInfo::plain(),
            };
            // Nothing set yet and nothing to say: leave the kernel's default
            let untouched = self.ump_info.is_none() && want == UmpInfo::plain();
            if !untouched && self.ump_info.as_ref() != Some(&want) {
                set_ump_info(api, inner.seq, &want);
                self.ump_info = Some(want);
            }
        }
        (self.ports.len(), failed)
    }

    /// Data from the Mac's source `r.id` come out of our port – scheduled for the time the Mac's time stamp maps
    /// to, or at once; false if there is no such port
    pub fn deliver(&mut self, r: &Record) -> bool {
        let Some(p) = self.ports.get_mut(&(r.id, true)) else { return false };
        let now = now_ns();
        let delay = r.time.and_then(|t| self.clock.deliver_at(t, now)).map(|at| at - now);
        let (flags, queue, time) = match (delay, self.queue) {
            (Some(d), Some(q)) => (
                SND_SEQ_TIME_STAMP_REAL | SND_SEQ_TIME_MODE_REL,
                q,
                RealTime { sec: (d / 1_000_000_000) as u32, nsec: (d % 1_000_000_000) as u32 },
            ),
            _ => (0, SND_SEQ_QUEUE_DIRECT, RealTime::default()),
        };
        let source = Addr { client: 0, port: p.port as u8 };
        let dest = Addr { client: SND_SEQ_ADDRESS_SUBSCRIBERS, port: SND_SEQ_ADDRESS_UNKNOWN };
        let inner = lock(&self.inner);
        let mut errors = 0;
        if self.ump_mode {
            let mut i = 0;
            while i < r.words.len() {
                let n = message_words(r.words[i]).min(r.words.len() - i);
                let mut ev = UmpEvent {
                    kind: 0,
                    flags: flags | SND_SEQ_EVENT_UMP,
                    queue,
                    time,
                    source,
                    dest,
                    ..Default::default()
                };
                ev.ump[..n].copy_from_slice(&r.words[i..i + n]);
                i += n;
                // SAFETY: a UMP event (flag set) of the size alsa-lib reads for it, under the lock
                if unsafe { snd_seq_event_output_direct(inner.seq, &mut ev as *mut UmpEvent as *mut Event) } < 0 {
                    errors += 1;
                }
            }
        } else {
            for item in ump_to_midi1(&r.words, &mut p.sysex) {
                let mut ev = Event::default();
                match &item {
                    Midi1::Message(bytes) => {
                        // SAFETY: the encoder of this client, under the lock; `ev` is written by it
                        unsafe {
                            snd_midi_event_reset_encode(inner.encoder);
                            snd_midi_event_encode(inner.encoder, bytes.as_ptr(), bytes.len() as c_long, &mut ev);
                        }
                        if ev.kind == SND_SEQ_EVENT_NONE {
                            continue;
                        }
                        ev.flags &= SND_SEQ_EVENT_LENGTH_MASK;
                    }
                    Midi1::Sysex(bytes) => {
                        ev.kind = SND_SEQ_EVENT_SYSEX;
                        ev.flags = SND_SEQ_EVENT_LENGTH_VARIABLE;
                        set_ext(&mut ev, bytes);
                    }
                }
                ev.flags |= flags;
                ev.queue = queue;
                ev.time = time;
                ev.source = source;
                ev.dest = dest;
                // SAFETY: a complete event; a SysEx one points to `bytes`, which outlives the call (alsa-lib copies
                // it into the write)
                if unsafe { snd_seq_event_output_direct(inner.seq, &mut ev) } < 0 {
                    errors += 1;
                }
            }
        }
        if errors > 0 {
            crate::log(&format!("MIDI: {errors} events to \"{}\" not sent", clean(&p.info.name)));
        }
        true
    }

    /// All ports gone (the Mac side went away or MIDI was switched off)
    pub fn clear(&mut self) {
        self.sync(&[]);
    }
}

/// A SysEx event's data: `ext` is a u32 length and a pointer right after it (packed)
fn set_ext(ev: &mut Event, bytes: &[u8]) {
    ev.data[0] = bytes.len() as u32;
    let p = (bytes.as_ptr() as u64).to_ne_bytes();
    ev.data[1] = u32::from_ne_bytes(p[0..4].try_into().unwrap());
    ev.data[2] = u32::from_ne_bytes(p[4..8].try_into().unwrap());
}

fn ext_len(ev: &Event) -> usize {
    ev.data[0] as usize
}

fn set_ump_info(api: &UmpInfoApi, seq: *mut Seq, info: &UmpInfo) {
    // SAFETY: alsa-lib allocates the info records, we set them with their setters and free them; the strings and
    // the revision live across the calls; `seq` is used under the caller's lock
    unsafe {
        let mut ep = std::ptr::null_mut();
        if (api.ep_malloc)(&mut ep) < 0 {
            return;
        }
        (api.ep_set_name)(ep, info.name.as_ptr());
        (api.ep_set_product_id)(ep, info.product_id.as_ptr());
        (api.ep_set_flags)(ep, info.flags);
        (api.ep_set_protocol_caps)(ep, SND_UMP_EP_INFO_PROTO_MIDI1 | SND_UMP_EP_INFO_PROTO_MIDI2);
        (api.ep_set_protocol)(ep, info.protocol);
        (api.ep_set_num_blocks)(ep, info.num_blocks);
        (api.ep_set_version)(ep, SND_UMP_EP_INFO_DEFAULT_VERSION);
        (api.ep_set_manufacturer_id)(ep, info.manufacturer);
        (api.ep_set_family_id)(ep, info.family);
        (api.ep_set_model_id)(ep, info.model);
        (api.ep_set_sw_revision)(ep, info.revision.as_ptr());
        let rc = (api.set_endpoint_info)(seq, ep);
        (api.ep_free)(ep);
        if rc < 0 {
            return crate::log(&format!("MIDI: UMP endpoint info not taken: error {rc}"));
        }
        for (id, flags, direction, first, groups, hint, name) in &info.blocks {
            let mut blk = std::ptr::null_mut();
            if (api.blk_malloc)(&mut blk) < 0 {
                return;
            }
            (api.blk_set_block_id)(blk, *id);
            (api.blk_set_active)(blk, 1);
            (api.blk_set_flags)(blk, *flags);
            (api.blk_set_direction)(blk, *direction);
            (api.blk_set_first_group)(blk, *first);
            (api.blk_set_num_groups)(blk, *groups);
            (api.blk_set_ui_hint)(blk, *hint);
            (api.blk_set_name)(blk, name.as_ptr());
            let rc = (api.set_block_info)(seq, *id as c_int, blk);
            (api.blk_free)(blk);
            if rc < 0 {
                crate::log(&format!("MIDI: UMP block {id} not taken: error {rc}"));
            }
        }
    }
}

/// The input thread: waits on the sequencer, reads every event under the lock, and sends what guest apps played to
/// our writable ports to the Mac, stamped with when it was read
fn input_thread(inner: Arc<Mutex<Inner>>, mut fds: Vec<libc::pollfd>, out: fn(Record)) {
    let mut buf = vec![0u8; 256];
    loop {
        for f in fds.iter_mut() {
            f.revents = 0;
        }
        // SAFETY: valid pollfd array of its length
        let rc = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 1000) };
        if rc <= 0 {
            continue;
        }
        let now = now_ns();
        let mut records: Vec<Record> = vec![];
        {
            let mut g = lock(&inner);
            let Inner { seq, decoder, inputs, .. } = &mut *g;
            loop {
                let mut ev: *mut Event = std::ptr::null_mut();
                // SAFETY: the handle under the lock; `ev` points into alsa-lib's buffer until the next call
                let rc = unsafe { snd_seq_event_input(*seq, &mut ev) };
                if rc == -libc::ENOSPC {
                    crate::log("MIDI: input overrun – events lost");
                    continue;
                }
                if rc < 0 || ev.is_null() {
                    break; // -EAGAIN: all read
                }
                // SAFETY: a valid event; a UMP one (flag set) is a whole snd_seq_ump_event_t in UMP mode
                let e = unsafe { *ev };
                let Some(port) = inputs.get_mut(&(e.dest.port as c_int)) else { continue };
                let words: Vec<u32> = if e.flags & SND_SEQ_EVENT_UMP != 0 {
                    let u = unsafe { (*(ev as *const UmpEvent)).ump };
                    u[..message_words(u[0])].to_vec()
                } else {
                    let need = if e.kind == SND_SEQ_EVENT_SYSEX { ext_len(&e) + 16 } else { 16 };
                    if buf.len() < need {
                        buf.resize(need, 0);
                    }
                    // SAFETY: the decoder under the lock writes at most `buf.len()` bytes; a SysEx event's data
                    // pointer is valid until the next input call
                    let n = unsafe { snd_midi_event_decode(*decoder, buf.as_mut_ptr(), buf.len() as c_long, ev) };
                    if n <= 0 {
                        continue; // not a MIDI event (port notifications, …)
                    }
                    port.parser.feed(&buf[..n as usize])
                };
                if words.is_empty() {
                    continue;
                }
                let protocol = protocol_of(&words, port.protocol);
                match records.last_mut() {
                    Some(last) if last.id == port.id && last.protocol == protocol => last.words.extend(words),
                    _ => records.push(Record { id: port.id, protocol, time: Some(now), words }),
                }
            }
        }
        records.into_iter().for_each(out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_match_alsa_lib() {
        // offsets in snd_seq_event_t: time at 4, source at 12, dest at 14, data at 16
        let e = Event::default();
        let base = &e as *const Event as usize;
        assert_eq!(&e.time as *const RealTime as usize - base, 4);
        assert_eq!(&e.source as *const Addr as usize - base, 12);
        assert_eq!(&e.dest as *const Addr as usize - base, 14);
        assert_eq!(&e.data as *const [u32; 3] as usize - base, 16);
    }

    #[test]
    fn sysex_data_is_packed_after_the_length() {
        let bytes = [0xF0u8, 1, 0xF7];
        let mut e = Event::default();
        set_ext(&mut e, &bytes);
        assert_eq!(ext_len(&e), 3);
        let raw: [u8; 12] = unsafe { std::mem::transmute(e.data) };
        assert_eq!(u64::from_ne_bytes(raw[4..12].try_into().unwrap()), bytes.as_ptr() as u64);
    }

    #[test]
    fn ump_info_for_alsa() {
        let u: Ump = serde_json::from_str(
            r#"{"name": "Keystage", "productInstanceId": "A1", "manufacturer": "00213c", "family": 1, "model": 2,
            "version": "01020304", "staticBlocks": true, "blocks": [
            {"id": 2, "name": "Ext", "direction": "out", "firstGroup": 1, "groups": 1, "midi1": "low-speed", "uiHint": "sender"},
            {"id": 0, "name": "Main", "direction": "both", "firstGroup": 0, "groups": 1, "midi1": "no", "uiHint": "both"}]}"#,
        )
        .unwrap();
        let i = UmpInfo::new(&u, 2);
        assert_eq!(i.manufacturer, 0x21_3C);
        assert_eq!((i.flags, i.protocol, i.num_blocks, i.revision), (1, 0x200, 3, [1, 2, 3, 4]));
        assert_eq!(i.blocks[0].0, 0);
        assert_eq!((i.blocks[1].0, i.blocks[1].6.as_bytes()), (2, b"Ext".as_slice()));
        assert_eq!((i.blocks[1].1, i.blocks[1].2, i.blocks[1].5), (3, 2, 2));
        assert_eq!(UmpInfo::plain().num_blocks, 0);
        assert_eq!(c_name("ä\u{7}bc", 3).as_bytes(), "äb".as_bytes());
    }
}
