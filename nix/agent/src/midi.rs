// SPDX-License-Identifier: AGPL-3.0-or-later
//! MIDI in the guest (spec §11.9, plan B161): every MIDI endpoint of the Mac becomes a virtual endpoint here, with the
//! Mac's name, manufacturer, model and MIDI protocol – a source of the Mac (a keyboard) a virtual source the guest's
//! apps read, a destination of the Mac (a synthesizer) a virtual destination they play to. The data are Universal
//! MIDI Packets as CoreMIDI has them, so MIDI 1.0 and MIDI 2.0 (and SysEx, MIDI-CI, …) pass unchanged.
//!
//! macOS guests: CoreMIDI's C API (virtual endpoints need no driver and no entitlement). The agent must run in the
//! logged-in user's session for the apps there to see them (darwin-guest.nix starts it over SSH, like the clipboard
//! helper – that CoreMIDI reaches the user's MIDI server from there is OPEN until checked in a guest). NixOS guests:
//! ALSA sequencer – OPEN, not offered yet.

#[cfg(not(target_os = "macos"))]
use crate::ump::Record;
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
pub struct Midi;

#[cfg(not(target_os = "macos"))]
impl Midi {
    pub fn new(_out: fn(Record)) -> Result<Midi, String> {
        Err("MIDI is not available in NixOS guests yet (ALSA sequencer: OPEN)".into())
    }
    pub fn sync(&mut self, _list: &[Endpoint]) -> (usize, Vec<String>) {
        (0, vec![])
    }
    pub fn deliver(&mut self, _r: &Record) -> bool {
        false
    }
    pub fn clear(&mut self) {}
}

#[cfg(target_os = "macos")]
mod coremidi {
    use super::{Endpoint, clean};
    use crate::ump::{Record, message_words, whole_messages};
    use std::collections::HashMap;
    use std::ffi::c_void;
    use std::sync::OnceLock;

    type OSStatus = i32;
    type MIDIObjectRef = u32;
    type MIDIClientRef = MIDIObjectRef;
    type MIDIEndpointRef = MIDIObjectRef;
    type CFStringRef = *const c_void;

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
    /// destinations. A *global* block – CoreMIDI's copy of it is the block itself, never freed – carrying the
    /// endpoint's id behind the standard fields, read back by `received`.
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
    }

    const BLOCK_IS_GLOBAL: i32 = 1 << 28;
    static DESCRIPTOR: BlockDescriptor = BlockDescriptor { reserved: 0, size: std::mem::size_of::<ReceiveBlock>() };

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFStringCreateWithBytes(
            alloc: *const c_void,
            bytes: *const u8,
            len: isize,
            encoding: u32,
            external: u8,
        ) -> CFStringRef;
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

    unsafe extern "C" {
        static _NSConcreteGlobalBlock: [*const c_void; 32];
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

    /// CoreMIDI's thread: a guest app played to destination `id` – whole UMP messages go to the Mac as one record
    unsafe extern "C" fn received(block: *const ReceiveBlock, list: *const MIDIEventList, _refcon: *mut c_void) {
        // SAFETY: CoreMIDI passes our block and a valid event list for the duration of the call; packets follow each
        // other with `word_count` words each (MIDIEventPacketNext)
        unsafe {
            let id = (*block).id;
            let protocol = std::ptr::read_unaligned(std::ptr::addr_of!((*list).protocol));
            let count = std::ptr::read_unaligned(std::ptr::addr_of!((*list).num_packets));
            let mut packet = std::ptr::addr_of!((*list).packet) as *const MIDIEventPacket;
            let mut words = Vec::new();
            for _ in 0..count {
                let n = (std::ptr::read_unaligned(std::ptr::addr_of!((*packet).word_count)) as usize).min(64);
                let first = std::ptr::addr_of!((*packet).words) as *const u32;
                for i in 0..n {
                    words.push(std::ptr::read_unaligned(first.add(i)));
                }
                packet = first.add(n) as *const MIDIEventPacket;
            }
            words.truncate(whole_messages(&words));
            if words.is_empty() || !(protocol == 1 || protocol == 2) {
                return;
            }
            if let Some(out) = OUT.get() {
                out(Record { id, protocol: protocol as u8, words });
            }
        }
    }

    struct Mirror {
        info: Endpoint,
        endpoint: MIDIEndpointRef,
    }

    pub struct Midi {
        client: MIDIClientRef,
        /// (id, is a source) → our endpoint
        mirrors: HashMap<(u32, bool), Mirror>,
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
            Ok(Midi { client, mirrors: HashMap::new(), list: vec![0; 8 * 1024] })
        }

        /// Makes our endpoints match the Mac's list: new ones are created, gone or changed ones disposed. Returns how
        /// many exist and the names that failed.
        pub fn sync(&mut self, list: &[Endpoint]) -> (usize, Vec<String>) {
            let wanted: HashMap<(u32, bool), &Endpoint> = list.iter().map(|e| ((e.id, e.direction == "in"), e)).collect();
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
            (self.mirrors.len(), failed)
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
                    let block = Box::leak(Box::new(ReceiveBlock {
                        isa: std::ptr::addr_of!(_NSConcreteGlobalBlock) as *const c_void,
                        flags: BLOCK_IS_GLOBAL,
                        reserved: 0,
                        invoke: received,
                        descriptor: &DESCRIPTOR,
                        id: e.id,
                    }));
                    MIDIDestinationCreateWithProtocol(self.client, name.0, protocol, &mut endpoint, block)
                }
            };
            if rc != 0 {
                return Err(rc);
            }
            // SAFETY: an endpoint of this client; properties are best effort (a taken unique id keeps CoreMIDI's)
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
            Ok(endpoint)
        }

        /// Data from the Mac's source `r.id` come out of our virtual source; false if there is none
        pub fn deliver(&mut self, r: &Record) -> bool {
            let Some(m) = self.mirrors.get(&(r.id, true)) else { return false };
            let source = m.endpoint;
            let size = self.list.len() * 8;
            let list = self.list.as_mut_ptr() as *mut MIDIEventList;
            // SAFETY: `list` points to `size` writable bytes, aligned for MIDIEventList; each add is one whole UMP
            // message (decode checked it); a full list is sent and started again
            unsafe {
                let mut packet = MIDIEventListInit(list, r.protocol as i32);
                let mut i = 0;
                while i < r.words.len() {
                    let n = message_words(r.words[i]);
                    let next = MIDIEventListAdd(list, size, packet, 0, n, r.words.as_ptr().add(i));
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
}
