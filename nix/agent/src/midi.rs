// SPDX-License-Identifier: AGPL-3.0-or-later
//! MIDI in the guest (spec §11.9, plan B161): every MIDI endpoint of the Mac becomes a virtual endpoint here, with the
//! Mac's name, manufacturer, model and MIDI protocol – a source of the Mac (a keyboard) a virtual source the guest's
//! apps read, a destination of the Mac (a synthesizer) a virtual destination they play to. The data are Universal
//! MIDI Packets as CoreMIDI has them, so MIDI 1.0 and MIDI 2.0 (and SysEx, MIDI-CI, …) pass unchanged.
//!
//! macOS guests: CoreMIDI's C API (virtual endpoints need no driver and no entitlement). The agent must run in the
//! logged-in user's session for the apps there to see them (darwin-guest.nix starts it over SSH, like the clipboard
//! helper – that CoreMIDI reaches the user's MIDI server from there is OPEN until checked in a guest). NixOS guests:
//! the ALSA sequencer (midi_alsa.rs).
//!
//! Timestamps (plan B161, C7): data from the Mac with a time are played at the time midi_clock.rs maps it to; data
//! played in the guest go out stamped with our monotonic clock. UMP endpoints (C7): an endpoint's `ump` – what a
//! MIDI 2.0 device says of itself – is mirrored where the guest's MIDI system can (macOS 15+, ALSA with UMP).

use serde::Deserialize;

/// One endpoint of the Mac, as its list (MSG_MIDI_ENDPOINTS) gives it
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct Endpoint {
    pub id: u32,
    /// "in": a source on the Mac, data come into the guest; "out": a destination on the Mac, data go out to it
    pub direction: String,
    pub name: String,
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub model: Option<String>,
    /// 1 = MIDI 1.0, 2 = MIDI 2.0
    pub protocol: u8,
    /// What a MIDI 2.0 device says of itself; an invalid one is ignored (None), the endpoint still mirrored
    #[serde(default, deserialize_with = "lenient_ump")]
    pub ump: Option<Ump>,
}

/// A UMP endpoint's self-description (spec §11.9 "UMP endpoints")
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Ump {
    pub name: String,
    pub product_instance_id: String,
    /// The 3-byte SysEx manufacturer id, as lowercase hex (6 digits, e.g. "00213b")
    #[serde(default)]
    pub manufacturer: Option<String>,
    #[serde(default)]
    pub family: Option<u16>,
    #[serde(default)]
    pub model: Option<u16>,
    /// The 4-byte software revision, as lowercase hex (8 digits, e.g. "01020304")
    #[serde(default)]
    pub version: Option<String>,
    pub static_blocks: bool,
    pub blocks: Vec<Block>,
}

/// A function block of a UMP endpoint
#[derive(Debug, Clone, PartialEq, Eq, Hash, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Block {
    pub id: u8,
    pub name: String,
    /// "in" | "out" | "both" – as the device sees it (UMP spec: input = the block receives); the Mac reports an unknown
    /// direction as "both"
    pub direction: String,
    pub first_group: u8,
    pub groups: u8,
    /// "no" | "yes" | "low-speed"
    pub midi1: String,
    /// "unknown" | "receiver" | "sender" | "both"
    pub ui_hint: String,
}

/// UMP limits: an endpoint name and a block name at most 98 bytes (the UMP stream messages carry that much), the
/// product instance id at most 42 (UMP spec; CoreMIDI's initializer says the same)
pub const UMP_NAME_MAX: usize = 98;
pub const UMP_PRODUCT_ID_MAX: usize = 42;
pub const UMP_BLOCKS_MAX: usize = 32;

/// Lowercase hex without separators, exactly N bytes (the format agreed with the Mac side, e.g. "00213b")
fn hex_bytes<const N: usize>(s: &str) -> Option<[u8; N]> {
    if s.len() != 2 * N || !s.bytes().all(|c| c.is_ascii_digit() || (b'a'..=b'f').contains(&c)) {
        return None;
    }
    let mut out = [0u8; N];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[2 * i..2 * i + 2], 16).ok()?;
    }
    Some(out)
}

impl Ump {
    /// The manufacturer id's three bytes (absent: zeros); each a 7-bit SysEx byte
    pub fn manufacturer_bytes(&self) -> [u8; 3] {
        self.manufacturer.as_deref().and_then(hex_bytes::<3>).unwrap_or([0; 3])
    }

    /// The software revision's four bytes (absent: zeros)
    pub fn version_bytes(&self) -> [u8; 4] {
        self.version.as_deref().and_then(hex_bytes::<4>).unwrap_or([0; 4])
    }

    /// The function blocks ordered by id
    pub fn sorted_blocks(&self) -> Vec<&Block> {
        let mut b: Vec<&Block> = self.blocks.iter().collect();
        b.sort_by_key(|b| b.id);
        b
    }

    /// Within the spec's limits: names (no control characters) and the product instance id short enough, ids and
    /// values in range, at most 32 blocks with unique ids. Family and model are 14-bit values in UMP (MIDI-CI device
    /// identity), so a u16 above 0x3FFF is rejected too.
    pub fn valid(&self) -> bool {
        let name_ok = |n: &str| !n.is_empty() && n.len() <= UMP_NAME_MAX && !n.chars().any(char::is_control);
        let mut ids = std::collections::HashSet::new();
        name_ok(&self.name)
            && self.product_instance_id.len() <= UMP_PRODUCT_ID_MAX
            && !self.product_instance_id.chars().any(char::is_control)
            && self
                .manufacturer
                .as_deref()
                .is_none_or(|m| hex_bytes::<3>(m).is_some_and(|b| b.iter().all(|x| *x < 0x80)))
            && self.version.as_deref().is_none_or(|v| hex_bytes::<4>(v).is_some_and(|b| b.iter().all(|x| *x < 0x80)))
            && self.family.is_none_or(|f| f <= 0x3FFF)
            && self.model.is_none_or(|m| m <= 0x3FFF)
            && self.blocks.len() <= UMP_BLOCKS_MAX
            && self.blocks.iter().all(|b| {
                ids.insert(b.id)
                    && b.id <= 31
                    && name_ok(&b.name)
                    && matches!(b.direction.as_str(), "in" | "out" | "both")
                    && b.first_group <= 15
                    && (1..=16).contains(&b.groups)
                    && b.first_group as u16 + b.groups as u16 <= 16
                    && matches!(b.midi1.as_str(), "no" | "yes" | "low-speed")
                    && matches!(b.ui_hint.as_str(), "unknown" | "receiver" | "sender" | "both")
            })
    }
}

/// `ump` as given when it parses and is valid, else None – a bad description never costs the endpoint
fn lenient_ump<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<Ump>, D::Error> {
    let value = Option::<serde_json::Value>::deserialize(d)?;
    Ok(value.and_then(|v| serde_json::from_value::<Ump>(v).ok()).filter(Ump::valid))
}

#[derive(Deserialize)]
pub struct EndpointList {
    pub endpoints: Vec<Endpoint>,
}

/// Names are shown in the guest's apps: no control characters, at most 128 characters
pub fn clean(s: &str) -> String {
    s.chars().filter(|c| !c.is_control()).take(128).collect()
}

/// Whether the list is acceptable: at most 256 endpoints, ids unique, known directions and protocols
pub fn valid(list: &[Endpoint]) -> bool {
    let mut ids = std::collections::HashSet::new();
    list.len() <= 256
        && list.iter().all(|e| {
            ids.insert((e.id, e.direction == "in"))
                && (e.direction == "in" || e.direction == "out")
                && (e.protocol == 1 || e.protocol == 2)
        })
}

#[cfg(target_os = "macos")]
pub use coremidi::Midi;

#[cfg(not(target_os = "macos"))]
pub use crate::midi_alsa::Midi;

#[cfg(target_os = "macos")]
mod coremidi {
    use super::{Endpoint, Ump, clean};
    use crate::midi_clock::{ClockMapper, now_ns, ns_to_ticks, ticks_to_ns};
    use crate::ump::{Record, message_words, whole_messages};
    use std::collections::HashMap;
    use std::ffi::{c_char, c_void};
    use std::sync::OnceLock;

    type OSStatus = i32;
    type MIDIObjectRef = u32;
    type MIDIClientRef = MIDIObjectRef;
    type MIDIEndpointRef = MIDIObjectRef;
    type CFStringRef = *const c_void;
    /// An Objective-C object and a selector
    type Id = *mut c_void;
    type Sel = *mut c_void;

    /// CoreMIDI's packet structures are `#pragma pack(push, 4)`
    #[repr(C, packed(4))]
    struct MIDIEventPacket {
        time_stamp: u64,
        word_count: u32,
        words: [u32; 64],
    }

    #[repr(C, packed(4))]
    struct MIDIEventList {
        protocol: i32,
        num_packets: u32,
        packet: [MIDIEventPacket; 1],
    }

    /// The Blocks ABI (clang's Block-ABI-Apple): a block CoreMIDI calls when a guest app plays to one of our
    /// destinations (a MIDIReceiveBlock). A *global* block – CoreMIDI's copy of it is the block itself, never freed –
    /// carrying the endpoint's id behind the standard fields, read back by `received`; `forward` 0: data played to it
    /// go nowhere (a UMP endpoint's destination without a Mac destination to go to)
    #[repr(C)]
    struct BlockDescriptor {
        reserved: usize,
        size: usize,
    }

    #[repr(C)]
    struct ReceiveBlock {
        isa: *const c_void,
        flags: i32,
        reserved: i32,
        invoke: unsafe extern "C" fn(*const ReceiveBlock, *const MIDIEventList, *mut c_void),
        descriptor: *const BlockDescriptor,
        id: u32,
        forward: u32,
    }

    const BLOCK_IS_GLOBAL: i32 = 1 << 28;
    static DESCRIPTOR: BlockDescriptor = BlockDescriptor { reserved: 0, size: std::mem::size_of::<ReceiveBlock>() };

    /// A block for destination `id` that lives as long as the agent (CoreMIDI may call it until the endpoint is gone)
    fn leak_block(id: u32, forward: bool) -> *const ReceiveBlock {
        Box::leak(Box::new(ReceiveBlock {
            isa: std::ptr::addr_of!(_NSConcreteGlobalBlock) as *const c_void,
            flags: BLOCK_IS_GLOBAL,
            reserved: 0,
            invoke: received,
            descriptor: &DESCRIPTOR,
            id,
            forward: forward as u32,
        }))
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFTypeArrayCallBacks: c_void;
        fn CFStringCreateWithBytes(
            alloc: *const c_void,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            external: u8,
        ) -> CFStringRef;
        fn CFArrayCreate(
            alloc: *const c_void,
            values: *const *const c_void,
            count: isize,
            callbacks: *const c_void,
        ) -> *const c_void;
        fn CFRelease(cf: *const c_void);
    }

    #[link(name = "CoreMIDI", kind = "framework")]
    unsafe extern "C" {
        static kMIDIPropertyManufacturer: CFStringRef;
        static kMIDIPropertyModel: CFStringRef;
        static kMIDIPropertyUniqueID: CFStringRef;
        fn MIDIClientCreate(
            name: CFStringRef,
            notify: *const c_void,
            refcon: *mut c_void,
            out: *mut MIDIClientRef,
        ) -> OSStatus;
        fn MIDISourceCreateWithProtocol(
            client: MIDIClientRef,
            name: CFStringRef,
            protocol: i32,
            out: *mut MIDIEndpointRef,
        ) -> OSStatus;
        fn MIDIDestinationCreateWithProtocol(
            client: MIDIClientRef,
            name: CFStringRef,
            protocol: i32,
            out: *mut MIDIEndpointRef,
            block: *const ReceiveBlock,
        ) -> OSStatus;
        fn MIDIEndpointDispose(endpoint: MIDIEndpointRef) -> OSStatus;
        fn MIDIObjectSetStringProperty(obj: MIDIObjectRef, key: CFStringRef, value: CFStringRef) -> OSStatus;
        fn MIDIObjectSetIntegerProperty(obj: MIDIObjectRef, key: CFStringRef, value: i32) -> OSStatus;
        fn MIDIEventListInit(list: *mut MIDIEventList, protocol: i32) -> *mut MIDIEventPacket;
        fn MIDIEventListAdd(
            list: *mut MIDIEventList,
            size: usize,
            packet: *mut MIDIEventPacket,
            time: u64,
            words: usize,
            data: *const u32,
        ) -> *mut MIDIEventPacket;
        fn MIDIReceivedEventList(source: MIDIEndpointRef, list: *const MIDIEventList) -> OSStatus;
    }

    // The Objective-C runtime, for CoreMIDI's UMP endpoint classes (macOS 15+, which have no C API)
    #[link(name = "objc")]
    unsafe extern "C" {
        static _NSConcreteGlobalBlock: [*const c_void; 32];
        fn objc_getClass(name: *const c_char) -> Id;
        fn sel_registerName(name: *const c_char) -> Sel;
        /// Never called as declared: cast to the method's exact signature first (arm64: no variadic calling)
        fn objc_msgSend();
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }

    /// A CFString, released when dropped
    struct CfString(CFStringRef);

    impl CfString {
        fn new(s: &str) -> CfString {
            // SAFETY: the bytes are valid UTF-8 of the given length; kCFStringEncodingUTF8 = 0x08000100
            CfString(unsafe { CFStringCreateWithBytes(std::ptr::null(), s.as_ptr(), s.len() as isize, 0x0800_0100, 0) })
        }
    }

    impl Drop for CfString {
        fn drop(&mut self) {
            if !self.0.is_null() {
                // SAFETY: created by CFStringCreateWithBytes above, released once
                unsafe { CFRelease(self.0) }
            }
        }
    }

    /// Our endpoints carry this property, so omaflake on the same Mac (developer checks) never mirrors them back
    pub const MIRROR_PROPERTY: &str = "app.omaflake.mirror";

    /// Where data played to our destinations go: the agent's port (set once by `Midi::new`)
    static OUT: OnceLock<fn(Record)> = OnceLock::new();

    /// The packets of an event list: (time stamp, whole UMP messages). A packet holds every message of one time
    /// stamp, so it may be longer than the 64 words MIDIEventPacket declares (CoreMIDI sizes the list for it); the
    /// word count is read as it is, only bounded by the largest MIDI data message.
    ///
    /// # Safety
    /// `list` must be a valid event list whose packets follow each other with `word_count` words each
    /// (MIDIEventPacketNext)
    unsafe fn read_event_list(list: *const MIDIEventList) -> Vec<(u64, Vec<u32>)> {
        const MAX_WORDS: usize = crate::proto::MAX_MIDI_MESSAGE / 4;
        let mut packets = vec![];
        // SAFETY: as the caller vouches; every read is unaligned (pack 4)
        unsafe {
            let count = std::ptr::read_unaligned(std::ptr::addr_of!((*list).num_packets));
            let mut packet = std::ptr::addr_of!((*list).packet) as *const MIDIEventPacket;
            for _ in 0..count {
                let stamp = std::ptr::read_unaligned(std::ptr::addr_of!((*packet).time_stamp));
                let n = std::ptr::read_unaligned(std::ptr::addr_of!((*packet).word_count)) as usize;
                if n > MAX_WORDS {
                    break; // not a packet CoreMIDI made
                }
                let first = std::ptr::addr_of!((*packet).words) as *const u32;
                let mut words: Vec<u32> = (0..n).map(|i| std::ptr::read_unaligned(first.add(i))).collect();
                words.truncate(whole_messages(&words));
                packet = first.add(n) as *const MIDIEventPacket;
                packets.push((stamp, words));
            }
        }
        packets
    }

    /// CoreMIDI's thread: a guest app played to destination `id` – whole UMP messages go to the Mac, one record per
    /// time stamp, stamped with our monotonic clock (an event list time of 0 means now)
    unsafe extern "C" fn received(block: *const ReceiveBlock, list: *const MIDIEventList, _refcon: *mut c_void) {
        // SAFETY: CoreMIDI passes our block and a valid event list for the duration of the call
        let (forward, id, protocol, packets) = unsafe {
            let protocol = std::ptr::read_unaligned(std::ptr::addr_of!((*list).protocol));
            ((*block).forward != 0, (*block).id, protocol, read_event_list(list))
        };
        if !forward || !(protocol == 1 || protocol == 2) {
            return;
        }
        let mut records: Vec<Record> = vec![];
        for (stamp, words) in packets {
            if words.is_empty() {
                continue;
            }
            let time = if stamp == 0 { now_ns() } else { ticks_to_ns(stamp) };
            match records.last_mut() {
                Some(last) if last.time == Some(time) => last.words.extend(words),
                _ => records.push(Record { id, protocol: protocol as u8, time: Some(time), words }),
            }
        }
        if let Some(out) = OUT.get() {
            records.into_iter().for_each(out);
        }
    }

    // --- UMP endpoints (macOS 15+): Objective-C through the runtime -------------------------------------------------
    //
    // Checked against the macOS 26 SDK headers MIDIUMPMutableEndpoint.h, MIDIUMPMutableFunctionBlock.h,
    // MIDIUMPEndpoint.h, MIDIUMPCI.h and MIDIMessages.h (selectors, argument types, enum values). OPEN until checked
    // in a macOS 15+ guest: that the endpoint appears in the guest's apps, that its source and destination take data
    // like the plain virtual ones, that CoreMIDI numbers the function blocks by their order (the initializer has no
    // id), that releasing the endpoint after disabling it disposes of its source and destination, and that it works
    // without a run loop on the main thread (the header asks for the main thread – sync runs there).

    /// MIDI2DeviceManufacturer and MIDI2DeviceRevisionLevel: small structs, passed by value
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Manufacturer([u8; 3]);
    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Revision([u8; 4]);

    fn sel(name: &str) -> Sel {
        let c = std::ffi::CString::new(name).expect("selector without NUL");
        // SAFETY: a NUL-terminated name; selectors are registered for the process's life
        unsafe { sel_registerName(c.as_ptr()) }
    }

    fn class(name: &str) -> Id {
        let c = std::ffi::CString::new(name).expect("class name without NUL");
        // SAFETY: a NUL-terminated name; nil when the class does not exist (macOS before 15)
        unsafe { objc_getClass(c.as_ptr()) }
    }

    /// objc_msgSend cast to a method's signature
    macro_rules! msg {
        ($($arg:ty),* => $ret:ty) => {
            // SAFETY (for every use): objc_msgSend is called with the exact argument and return types of the method
            // named by the selector passed, as the SDK headers declare it
            {
                #[allow(unused_unsafe)]
                let f = unsafe {
                    std::mem::transmute::<unsafe extern "C" fn(), unsafe extern "C" fn(Id, Sel $(, $arg)*) -> $ret>(
                        objc_msgSend,
                    )
                };
                f
            }
        };
    }

    struct UmpClasses {
        endpoint: Id,
        block: Id,
        device_info: Id,
    }
    // SAFETY: class objects are process-wide and immutable for our use
    unsafe impl Send for UmpClasses {}
    unsafe impl Sync for UmpClasses {}

    /// The three classes, or None before macOS 15
    fn ump_classes() -> Option<&'static UmpClasses> {
        static CLASSES: OnceLock<Option<UmpClasses>> = OnceLock::new();
        CLASSES
            .get_or_init(|| {
                let c = UmpClasses {
                    endpoint: class("MIDIUMPMutableEndpoint"),
                    block: class("MIDIUMPMutableFunctionBlock"),
                    device_info: class("MIDI2DeviceInfo"),
                };
                (!c.endpoint.is_null() && !c.block.is_null() && !c.device_info.is_null()).then_some(c)
            })
            .as_ref()
    }

    fn release(obj: Id) {
        if !obj.is_null() {
            let f = msg!(=> ());
            // SAFETY: an object we own a reference to
            unsafe { f(obj, sel("release")) };
        }
    }

    /// An autorelease pool around runtime calls (the agent's thread has none)
    struct Pool(*mut c_void);
    impl Pool {
        fn new() -> Pool {
            // SAFETY: no preconditions; popped in drop on the same thread
            Pool(unsafe { objc_autoreleasePoolPush() })
        }
    }
    impl Drop for Pool {
        fn drop(&mut self) {
            // SAFETY: the pool pushed in `new`, on this thread
            unsafe { objc_autoreleasePoolPop(self.0) }
        }
    }

    /// Entries of the Mac's list that describe one UMP endpoint: the same `ump` and protocol, at most one source and
    /// one destination – mirrored as one MIDIUMPMutableEndpoint (which always has both)
    #[derive(Debug, Clone, PartialEq, Eq)]
    struct UmpGroup {
        ump: Ump,
        protocol: u8,
        /// The Mac's source ("in"): data from it come out of the endpoint's source
        source: Option<Endpoint>,
        /// The Mac's destination ("out"): data played to the endpoint's destination go to it
        destination: Option<Endpoint>,
    }

    impl UmpGroup {
        fn entries(&self) -> impl Iterator<Item = (u32, bool)> + '_ {
            self.source.iter().map(|e| (e.id, true)).chain(self.destination.iter().map(|e| (e.id, false)))
        }
    }

    fn ump_groups(list: &[Endpoint]) -> Vec<UmpGroup> {
        let mut groups: Vec<UmpGroup> = vec![];
        for e in list {
            let Some(ump) = &e.ump else { continue };
            let source = e.direction == "in";
            let slot_free = |g: &UmpGroup| if source { g.source.is_none() } else { g.destination.is_none() };
            let at = groups.iter().position(|g| &g.ump == ump && g.protocol == e.protocol && slot_free(g));
            let g = match at {
                Some(i) => &mut groups[i],
                None => {
                    groups.push(UmpGroup { ump: ump.clone(), protocol: e.protocol, source: None, destination: None });
                    groups.last_mut().unwrap()
                }
            };
            if source {
                g.source = Some(e.clone());
            } else {
                g.destination = Some(e.clone());
            }
        }
        groups
    }

    struct UmpMirror {
        group: UmpGroup,
        object: Id,
        source: MIDIEndpointRef,
    }

    /// A MIDIUMPMutableEndpoint with its function blocks, enabled; None if any step fails (all undone)
    fn create_ump(c: &UmpClasses, g: &UmpGroup) -> Option<(Id, MIDIEndpointRef, MIDIEndpointRef)> {
        let _pool = Pool::new();
        let alloc = msg!(=> Id);
        let ump = &g.ump;
        // SAFETY (the calls below): `alloc` on a class object, then the initializer of that class with its declared
        // argument types; every object we own is released on every path
        let info = unsafe {
            let init = msg!(Manufacturer, u16, u16, Revision => Id);
            init(
                alloc(c.device_info, sel("alloc")),
                sel("initWithManufacturerID:family:modelNumber:revisionLevel:"),
                Manufacturer(ump.manufacturer_bytes()),
                ump.family.unwrap_or(0),
                ump.model.unwrap_or(0),
                Revision(ump.version_bytes()),
            )
        };
        if info.is_null() {
            return None;
        }
        let name = CfString::new(&clean(&ump.name));
        let product = CfString::new(&clean(&ump.product_instance_id));
        let block = leak_block(g.destination.as_ref().map_or(0, |e| e.id), g.destination.is_some());
        let endpoint = unsafe {
            let init = msg!(CFStringRef, Id, CFStringRef, i32, *const ReceiveBlock => Id);
            init(
                alloc(c.endpoint, sel("alloc")),
                sel("initWithName:deviceInfo:productInstanceID:MIDIProtocol:destinationCallback:"),
                name.0,
                info,
                product.0,
                g.protocol as i32,
                block,
            )
        };
        release(info);
        if endpoint.is_null() {
            return None;
        }
        let mut blocks: Vec<Id> = vec![];
        for b in ump.sorted_blocks() {
            let direction: i32 = match b.direction.as_str() {
                "in" => 1,  // kMIDIUMPFunctionBlockDirectionInput
                "out" => 2, // …Output
                _ => 3,     // …Bidirectional
            };
            let midi1: i32 = match b.midi1.as_str() {
                "yes" => 1,       // kMIDIUMPFunctionBlockMIDI1InfoUnrestrictedBandwidth
                "low-speed" => 2, // …RestrictedBandwidth
                _ => 0,           // …NotMIDI1
            };
            let hint: i32 = match b.ui_hint.as_str() {
                "receiver" => 1, // kMIDIUMPFunctionBlockUIHintReceiver
                "sender" => 2,   // …Sender
                "both" => 3,     // …SenderReceiver
                _ => 0,          // …Unknown
            };
            let bname = CfString::new(&clean(&b.name));
            let fb = unsafe {
                let init = msg!(CFStringRef, i32, u8, u8, u8, i32, i32, bool => Id);
                init(
                    alloc(c.block, sel("alloc")),
                    sel(
                        "initWithName:direction:firstGroup:totalGroupsSpanned:maxSysEx8Streams:MIDI1Info:UIHint:isEnabled:",
                    ),
                    bname.0,
                    direction,
                    b.first_group,
                    b.groups,
                    0,
                    midi1,
                    hint,
                    true,
                )
            };
            if fb.is_null() {
                blocks.into_iter().for_each(release);
                release(endpoint);
                return None;
            }
            blocks.push(fb);
        }
        let mut ok = true;
        if !blocks.is_empty() {
            // SAFETY: a CFArray of the block objects (toll-free bridged to NSArray), retained by it
            unsafe {
                let array = CFArrayCreate(
                    std::ptr::null(),
                    blocks.as_ptr() as *const *const c_void,
                    blocks.len() as isize,
                    std::ptr::addr_of!(kCFTypeArrayCallBacks),
                );
                if array.is_null() {
                    ok = false;
                } else {
                    let register = msg!(*const c_void, bool, *mut Id => bool);
                    ok = register(
                        endpoint,
                        sel("registerFunctionBlocks:markAsStatic:error:"),
                        array,
                        ump.static_blocks,
                        std::ptr::null_mut(),
                    );
                    CFRelease(array);
                }
            }
        }
        blocks.into_iter().for_each(release);
        if ok {
            let enable = msg!(bool, *mut Id => bool);
            // SAFETY: setEnabled:error: of MIDIUMPMutableEndpoint, no error object asked for
            ok = unsafe { enable(endpoint, sel("setEnabled:error:"), true, std::ptr::null_mut()) };
        }
        if !ok {
            release(endpoint);
            return None;
        }
        let get = msg!(=> MIDIEndpointRef);
        // SAFETY: the MIDISource / MIDIDestination properties of MIDIUMPEndpoint (MIDIEndpointRef, a UInt32)
        let (source, destination) =
            unsafe { (get(endpoint, sel("MIDISource")), get(endpoint, sel("MIDIDestination"))) };
        Some((endpoint, source, destination))
    }

    fn dispose_ump(object: Id) {
        let _pool = Pool::new();
        let enable = msg!(bool, *mut Id => bool);
        // SAFETY: an endpoint we created and own; disabled, then our reference released
        unsafe { enable(object, sel("setEnabled:error:"), false, std::ptr::null_mut()) };
        release(object);
    }

    /// Manufacturer, model, the Mac's unique id and the mirror mark on one of our endpoints (best effort: a taken
    /// unique id keeps CoreMIDI's)
    fn set_properties(endpoint: MIDIEndpointRef, e: &Endpoint) {
        // SAFETY: an endpoint of this process; the CFStrings live across the calls
        unsafe {
            for (key, value) in [(kMIDIPropertyManufacturer, &e.manufacturer), (kMIDIPropertyModel, &e.model)] {
                if let Some(v) = value {
                    let v = CfString::new(&clean(v));
                    MIDIObjectSetStringProperty(endpoint, key, v.0);
                }
            }
            // The Mac's unique id: apps that remember endpoints by it find the same device after a restart
            // (0 is the VM's own pair, not a CoreMIDI id: that one keeps the id CoreMIDI gave it)
            if e.id != 0 {
                MIDIObjectSetIntegerProperty(endpoint, kMIDIPropertyUniqueID, e.id as i32);
            }
            let mirror = CfString::new(MIRROR_PROPERTY);
            MIDIObjectSetIntegerProperty(endpoint, mirror.0, 1);
        }
    }

    struct Mirror {
        info: Endpoint,
        endpoint: MIDIEndpointRef,
    }

    pub struct Midi {
        client: MIDIClientRef,
        /// (id, is a source) → our plain virtual endpoint
        mirrors: HashMap<(u32, bool), Mirror>,
        /// UMP endpoints (macOS 15+), each for one or two entries of the list
        umps: Vec<UmpMirror>,
        /// Groups whose UMP endpoint could not be made: their entries stay plain until the list changes them
        ump_failed: Vec<UmpGroup>,
        /// The Mac's source id → the source its data come out of
        sources: HashMap<u32, MIDIEndpointRef>,
        /// The Mac's clock on ours, for timestamped data
        clock: ClockMapper,
        /// One event-list buffer, 8-byte aligned, for data from the Mac
        list: Vec<u64>,
    }

    impl Midi {
        /// A CoreMIDI client; fails when the guest has no MIDI server reachable from this process
        pub fn new(out: fn(Record)) -> Result<Midi, String> {
            let _ = OUT.set(out);
            let name = CfString::new("omaflake");
            let mut client: MIDIClientRef = 0;
            // SAFETY: valid name, no notification callback, `client` is written on success
            let rc = unsafe { MIDIClientCreate(name.0, std::ptr::null(), std::ptr::null_mut(), &mut client) };
            if rc != 0 {
                return Err(format!("CoreMIDI client: OSStatus {rc}"));
            }
            Ok(Midi {
                client,
                mirrors: HashMap::new(),
                umps: vec![],
                ump_failed: vec![],
                sources: HashMap::new(),
                clock: ClockMapper::new(),
                list: vec![0; 8 * 1024],
            })
        }

        /// Makes our endpoints match the Mac's list: new ones are created, gone or changed ones disposed – entries
        /// with a `ump` as UMP endpoints where CoreMIDI has them, the rest (and UMP endpoints that failed) as plain
        /// virtual endpoints. Returns how many entries are mirrored and the names that failed.
        pub fn sync(&mut self, list: &[Endpoint]) -> (usize, Vec<String>) {
            let groups = match ump_classes() {
                Some(_) => ump_groups(list),
                None => vec![],
            };
            // UMP endpoints no longer wanted as they are
            let (keep, stale): (Vec<UmpMirror>, Vec<UmpMirror>) =
                std::mem::take(&mut self.umps).into_iter().partition(|u| groups.contains(&u.group));
            self.umps = keep;
            for u in stale {
                dispose_ump(u.object);
            }
            self.ump_failed.retain(|g| groups.contains(g));
            // New UMP endpoints: their entries' plain endpoints go first (so the unique ids are free)
            if let Some(c) = ump_classes() {
                for g in &groups {
                    if self.umps.iter().any(|u| &u.group == g) || self.ump_failed.contains(g) {
                        continue;
                    }
                    for k in g.entries() {
                        if let Some(m) = self.mirrors.remove(&k) {
                            // SAFETY: an endpoint this client created
                            unsafe { MIDIEndpointDispose(m.endpoint) };
                        }
                    }
                    match create_ump(c, g) {
                        Some((object, source, destination)) => {
                            if let Some(e) = &g.source {
                                set_properties(source, e);
                            }
                            if let Some(e) = &g.destination {
                                set_properties(destination, e);
                            }
                            self.umps.push(UmpMirror { group: g.clone(), object, source });
                        }
                        None => {
                            crate::log(&format!(
                                "MIDI: cannot create the UMP endpoint \"{}\" – plain endpoints instead",
                                clean(&g.ump.name)
                            ));
                            self.ump_failed.push(g.clone());
                        }
                    }
                }
            }
            // Plain endpoints for every entry no UMP endpoint covers
            let covered: std::collections::HashSet<(u32, bool)> =
                self.umps.iter().flat_map(|u| u.group.entries().collect::<Vec<_>>()).collect();
            let wanted: HashMap<(u32, bool), &Endpoint> =
                list.iter().map(|e| ((e.id, e.direction == "in"), e)).filter(|(k, _)| !covered.contains(k)).collect();
            let stale: Vec<(u32, bool)> = self
                .mirrors
                .iter()
                .filter(|(k, m)| wanted.get(k).is_none_or(|e| **e != m.info))
                .map(|(k, _)| *k)
                .collect();
            for k in stale {
                if let Some(m) = self.mirrors.remove(&k) {
                    // SAFETY: an endpoint this client created
                    unsafe { MIDIEndpointDispose(m.endpoint) };
                }
            }
            let mut failed = vec![];
            for (k, e) in wanted {
                if self.mirrors.contains_key(&k) {
                    continue;
                }
                match self.create(e, k.1) {
                    Ok(endpoint) => {
                        self.mirrors.insert(k, Mirror { info: e.clone(), endpoint });
                    }
                    Err(rc) => {
                        crate::log(&format!("MIDI: cannot create \"{}\": OSStatus {rc}", clean(&e.name)));
                        failed.push(clean(&e.name));
                    }
                }
            }
            self.sources = self
                .mirrors
                .iter()
                .filter(|(k, _)| k.1)
                .map(|(k, m)| (k.0, m.endpoint))
                .chain(self.umps.iter().filter_map(|u| u.group.source.as_ref().map(|e| (e.id, u.source))))
                .collect();
            (self.mirrors.len() + covered.len(), failed)
        }

        fn create(&self, e: &Endpoint, source: bool) -> Result<MIDIEndpointRef, OSStatus> {
            let name = CfString::new(&clean(&e.name));
            let mut endpoint: MIDIEndpointRef = 0;
            let protocol = e.protocol as i32;
            // SAFETY: valid client and name; a destination gets a global block that lives for the agent's life
            let rc = unsafe {
                if source {
                    MIDISourceCreateWithProtocol(self.client, name.0, protocol, &mut endpoint)
                } else {
                    MIDIDestinationCreateWithProtocol(
                        self.client,
                        name.0,
                        protocol,
                        &mut endpoint,
                        leak_block(e.id, true),
                    )
                }
            };
            if rc != 0 {
                return Err(rc);
            }
            set_properties(endpoint, e);
            Ok(endpoint)
        }

        /// Data from the Mac's source `r.id` come out of our source – at the time the Mac's time stamp maps to (host
        /// ticks in the event list), or now; false if there is no such source. OPEN until checked in a guest: whether
        /// the guest's apps honour a time stamp slightly in the future on data from a virtual source (CoreMIDI hands
        /// it on as it is; it schedules only what is sent to a destination).
        pub fn deliver(&mut self, r: &Record) -> bool {
            let Some(&source) = self.sources.get(&r.id) else { return false };
            let at = r.time.and_then(|t| self.clock.deliver_at(t, now_ns())).map_or(0, ns_to_ticks);
            let size = self.list.len() * 8;
            let list = self.list.as_mut_ptr() as *mut MIDIEventList;
            // SAFETY: `list` points to `size` writable bytes, aligned for MIDIEventList; each add is one whole UMP
            // message (decode checked it); a full list is sent and started again
            unsafe {
                let mut packet = MIDIEventListInit(list, r.protocol as i32);
                let mut i = 0;
                while i < r.words.len() {
                    let n = message_words(r.words[i]);
                    let next = MIDIEventListAdd(list, size, packet, at, n, r.words.as_ptr().add(i));
                    if next.is_null() {
                        MIDIReceivedEventList(source, list);
                        packet = MIDIEventListInit(list, r.protocol as i32);
                        continue; // the same message again, into the empty list
                    }
                    packet = next;
                    i += n;
                }
                if std::ptr::read_unaligned(std::ptr::addr_of!((*list).num_packets)) > 0 {
                    MIDIReceivedEventList(source, list);
                }
            }
            true
        }

        /// All endpoints gone (the Mac side went away or MIDI was switched off)
        pub fn clear(&mut self) {
            self.sync(&[]);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn entry(id: u32, direction: &str, ump: Option<Ump>) -> Endpoint {
            Endpoint {
                id,
                direction: direction.into(),
                name: format!("e{id}"),
                manufacturer: None,
                model: None,
                protocol: 2,
                ump,
            }
        }

        fn ump(name: &str) -> Ump {
            Ump {
                name: name.into(),
                product_instance_id: "1".into(),
                manufacturer: None,
                family: None,
                model: None,
                version: None,
                static_blocks: true,
                blocks: vec![],
            }
        }

        /// CoreMIDI packs every message of one time stamp into one packet – longer than 64 words for long SysEx
        #[test]
        fn packets_longer_than_64_words_are_read_whole() {
            let mut buf = vec![0u64; 2048];
            let size = buf.len() * 8;
            let list = buf.as_mut_ptr() as *mut MIDIEventList;
            // 96 SysEx7 packets (start, continues, end): 192 words with one time stamp, then a note at another
            let mut words: Vec<u32> = vec![];
            for i in 0..96u32 {
                let status = match i {
                    0 => 1,
                    95 => 3,
                    _ => 2,
                };
                words.extend([0x3006_0000 | status << 20 | i, i]);
            }
            // SAFETY: `list` points to `size` writable bytes aligned for MIDIEventList; each add is a whole message
            let packets = unsafe {
                let mut packet = MIDIEventListInit(list, 1);
                for m in words.chunks(2) {
                    packet = MIDIEventListAdd(list, size, packet, 1000, 2, m.as_ptr());
                    assert!(!packet.is_null());
                }
                packet = MIDIEventListAdd(list, size, packet, 2000, 1, [0x2090_3C64u32].as_ptr());
                assert!(!packet.is_null());
                read_event_list(list)
            };
            assert_eq!(packets[0], (1000, words));
            assert_eq!(packets.last(), Some(&(2000, vec![0x2090_3C64])));
        }

        #[test]
        fn entries_with_the_same_ump_pair_up() {
            let list = vec![
                entry(1, "in", Some(ump("A"))),
                entry(2, "out", Some(ump("A"))),
                entry(3, "in", Some(ump("A"))),
                entry(4, "in", None),
                entry(5, "out", Some(ump("B"))),
            ];
            let g = ump_groups(&list);
            assert_eq!(g.len(), 3);
            assert_eq!(
                (g[0].source.as_ref().map(|e| e.id), g[0].destination.as_ref().map(|e| e.id)),
                (Some(1), Some(2))
            );
            assert_eq!((g[1].source.as_ref().map(|e| e.id), g[1].destination.as_ref().map(|e| e.id)), (Some(3), None));
            assert_eq!((g[2].source.as_ref().map(|e| e.id), g[2].destination.as_ref().map(|e| e.id)), (None, Some(5)));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const UMP: &str = r#"{"name": "Keystage", "productInstanceId": "A1B2", "manufacturer": "00213c", "family": 1,
        "model": 2, "version": "01000000", "staticBlocks": true, "blocks": [
        {"id": 1, "name": "Ext", "direction": "out", "firstGroup": 1, "groups": 1, "midi1": "low-speed", "uiHint": "sender"},
        {"id": 0, "name": "Main", "direction": "both", "firstGroup": 0, "groups": 1, "midi1": "no", "uiHint": "both"}]}"#;

    fn list(ump: &str) -> Vec<Endpoint> {
        let json =
            format!(r#"{{"endpoints": [{{"id": 7, "direction": "in", "name": "K", "protocol": 2, "ump": {ump}}}]}}"#);
        serde_json::from_str::<EndpointList>(&json).unwrap().endpoints
    }

    #[test]
    fn a_valid_ump_is_parsed() {
        let e = list(UMP);
        let u = e[0].ump.as_ref().expect("valid");
        assert_eq!(u.manufacturer_bytes(), [0x00, 0x21, 0x3C]);
        assert_eq!(u.blocks[1].direction, "both");
        assert_eq!(u.version_bytes(), [1, 0, 0, 0]);
        assert_eq!(u.sorted_blocks().iter().map(|b| b.id).collect::<Vec<_>>(), vec![0, 1]);
        // without the optional fields
        let minimal = r#"{"name": "X", "productInstanceId": "", "staticBlocks": false, "blocks": []}"#;
        let u = list(minimal)[0].ump.clone().unwrap();
        assert_eq!((u.manufacturer_bytes(), u.version_bytes()), ([0; 3], [0; 4]));
    }

    #[test]
    fn an_invalid_ump_is_ignored_and_the_endpoint_kept() {
        let bad = [
            UMP.replace("\"00213c\"", "\"0021\""),                  // manufacturer not 3 bytes
            UMP.replace("\"00213c\"", "\"00ff3c\""),                // not a 7-bit byte
            UMP.replace("\"00213c\"", "\"00213C\""),                // not lowercase
            UMP.replace("\"00213c\"", "\"00-21-3c\""),              // separators
            UMP.replace("\"01000000\"", "\"+1000000\""),            // a sign
            UMP.replace("\"family\": 1", "\"family\": 16384"),      // over 14 bits
            UMP.replace("\"id\": 1,", "\"id\": 0,"),                // duplicate block id
            UMP.replace("\"id\": 1,", "\"id\": 32,"),               // block id out of range
            UMP.replace("\"firstGroup\": 1", "\"firstGroup\": 16"), // group out of range
            UMP.replace("\"groups\": 1, \"midi1\": \"low", "\"groups\": 16, \"midi1\": \"low"), // past group 15
            UMP.replace("\"direction\": \"out\"", "\"direction\": \"up\""),
            UMP.replace("\"midi1\": \"no\"", "\"midi1\": \"maybe\""),
            UMP.replace("\"uiHint\": \"both\"", "\"uiHint\": \"all\""),
            UMP.replace("\"Keystage\"", &format!("\"{}\"", "k".repeat(99))), // name over 98 bytes
            UMP.replace("\"A1B2\"", &format!("\"{}\"", "p".repeat(43))),     // product id over 42 bytes
            UMP.replace("\"staticBlocks\": true,", ""),                      // a required field missing
            "42".to_string(),                                                // not an object
        ];
        for b in &bad {
            let e = list(b);
            assert_eq!(e.len(), 1, "{b}");
            assert_eq!(e[0].ump, None, "{b}");
        }
        // 33 blocks
        let blocks: Vec<String> = (0..33)
            .map(|i| format!(r#"{{"id": {i}, "name": "b", "direction": "in", "firstGroup": 0, "groups": 1, "midi1": "no", "uiHint": "unknown"}}"#))
            .collect();
        let many = format!(
            r#"{{"name": "X", "productInstanceId": "", "staticBlocks": false, "blocks": [{}]}}"#,
            blocks.join(",")
        );
        assert_eq!(list(&many)[0].ump, None);
        // no ump at all, or null
        assert_eq!(list("null")[0].ump, None);
        let json = r#"{"endpoints": [{"id": 1, "direction": "out", "name": "S", "protocol": 1}]}"#;
        assert_eq!(serde_json::from_str::<EndpointList>(json).unwrap().endpoints[0].ump, None);
    }
}
