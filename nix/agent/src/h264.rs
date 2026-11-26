// SPDX-License-Identifier: AGPL-3.0-or-later
//! H.264 from the Mac's camera back into raw frames, in the macOS guest (plan B183). OBS's media source decodes
//! H.264 with FFmpeg's frame threads, which hold back one picture per thread – about 300 ms in a VM with eight
//! cores, and no option of the source reaches the decoder. So the agent decodes with VideoToolbox, one picture out
//! for each one in, and OBS reads raw NV12 over the loopback as it did before plan B174; the virtio port still
//! carries the small H.264 frames.
//!
//! `split` (Annex B into its NAL units) is plain code, tested everywhere; `Decoder` is VideoToolbox, macOS only.

#![cfg_attr(not(target_os = "macos"), allow(dead_code))]

/// The NAL units of one access unit in Annex B (start codes `00 00 01`, with any zeros before them)
pub fn split(annex_b: &[u8]) -> Vec<&[u8]> {
    let mut units = Vec::new();
    let mut start = None;
    let mut i = 0;
    while i + 3 <= annex_b.len() {
        if annex_b[i] == 0 && annex_b[i + 1] == 0 && annex_b[i + 2] == 1 {
            if let Some(s) = start {
                units.push(trim(&annex_b[s..i]));
            }
            start = Some(i + 3);
            i += 3;
        } else {
            i += 1;
        }
    }
    if let Some(s) = start {
        units.push(&annex_b[s..]);
    }
    units.retain(|u| !u.is_empty());
    units
}

/// Without the zeros that belong to the next start code (`00 00 00 01`); a NAL unit never ends in a zero byte
fn trim(unit: &[u8]) -> &[u8] {
    let end = unit.iter().rposition(|&b| b != 0).map_or(0, |i| i + 1);
    &unit[..end]
}

/// What an access unit holds: the parameter sets (key frames carry them first) and the picture's slices with
/// 4-byte lengths, as VideoToolbox takes them
#[derive(Debug, Default, PartialEq, Eq)]
pub struct AccessUnit {
    pub sps: Option<Vec<u8>>,
    pub pps: Option<Vec<u8>>,
    pub slices: Vec<u8>,
}

pub fn access_unit(annex_b: &[u8]) -> AccessUnit {
    let mut unit = AccessUnit::default();
    for nal in split(annex_b) {
        match nal[0] & 0x1F {
            7 => unit.sps = Some(nal.to_vec()),
            8 => unit.pps = Some(nal.to_vec()),
            1 | 5 => {
                unit.slices.extend_from_slice(&(nal.len() as u32).to_be_bytes());
                unit.slices.extend_from_slice(nal);
            }
            _ => {} // delimiters, SEI
        }
    }
    unit
}

#[cfg(target_os = "macos")]
pub use mac::Decoder;

#[cfg(target_os = "macos")]
mod mac {
    use super::{AccessUnit, access_unit};
    use std::ffi::c_void;
    use std::ptr::{null, null_mut};

    type Ref = *const c_void;
    type OSStatus = i32;

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CMTime {
        value: i64,
        timescale: i32,
        flags: u32,
        epoch: i64,
    }

    type OutputCallback = extern "C" fn(*mut c_void, *mut c_void, OSStatus, u32, Ref, CMTime, CMTime);

    #[repr(C)]
    struct CallbackRecord {
        callback: OutputCallback,
        refcon: *mut c_void,
    }

    /// kCVPixelFormatType_420YpCbCr8BiPlanarVideoRange: NV12, what the stream's readers expect
    const NV12: i32 = 0x3432_3076; // '420v'
    const NUMBER_SINT32: isize = 3; // kCFNumberSInt32Type
    const LOCK_READ_ONLY: u64 = 1;

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        static kCFAllocatorNull: Ref;
        static kCFTypeDictionaryKeyCallBacks: c_void;
        static kCFTypeDictionaryValueCallBacks: c_void;
        fn CFRelease(cf: Ref);
        fn CFNumberCreate(allocator: Ref, kind: isize, value: *const c_void) -> Ref;
        fn CFDictionaryCreate(
            allocator: Ref,
            keys: *const Ref,
            values: *const Ref,
            count: isize,
            key_callbacks: *const c_void,
            value_callbacks: *const c_void,
        ) -> Ref;
    }

    #[link(name = "CoreMedia", kind = "framework")]
    unsafe extern "C" {
        fn CMVideoFormatDescriptionCreateFromH264ParameterSets(
            allocator: Ref,
            count: usize,
            sets: *const *const u8,
            sizes: *const usize,
            length_size: i32,
            out: *mut Ref,
        ) -> OSStatus;
        fn CMBlockBufferCreateWithMemoryBlock(
            allocator: Ref,
            block: *mut c_void,
            length: usize,
            block_allocator: Ref,
            source: *const c_void,
            offset: usize,
            data_length: usize,
            flags: u32,
            out: *mut Ref,
        ) -> OSStatus;
        fn CMSampleBufferCreateReady(
            allocator: Ref,
            data: Ref,
            format: Ref,
            samples: isize,
            timings: isize,
            timing: *const c_void,
            sizes: isize,
            size: *const usize,
            out: *mut Ref,
        ) -> OSStatus;
    }

    #[link(name = "CoreVideo", kind = "framework")]
    unsafe extern "C" {
        static kCVPixelBufferPixelFormatTypeKey: Ref;
        fn CVPixelBufferLockBaseAddress(buffer: Ref, flags: u64) -> i32;
        fn CVPixelBufferUnlockBaseAddress(buffer: Ref, flags: u64) -> i32;
        fn CVPixelBufferGetPlaneCount(buffer: Ref) -> usize;
        fn CVPixelBufferGetBaseAddressOfPlane(buffer: Ref, plane: usize) -> *const u8;
        fn CVPixelBufferGetBytesPerRowOfPlane(buffer: Ref, plane: usize) -> usize;
        fn CVPixelBufferGetWidthOfPlane(buffer: Ref, plane: usize) -> usize;
        fn CVPixelBufferGetHeightOfPlane(buffer: Ref, plane: usize) -> usize;
    }

    #[link(name = "VideoToolbox", kind = "framework")]
    unsafe extern "C" {
        fn VTDecompressionSessionCreate(
            allocator: Ref,
            format: Ref,
            decoder: Ref,
            image_attributes: Ref,
            callback: *const CallbackRecord,
            out: *mut Ref,
        ) -> OSStatus;
        fn VTDecompressionSessionDecodeFrame(
            session: Ref,
            sample: Ref,
            flags: u32,
            frame_refcon: *mut c_void,
            info: *mut u32,
        ) -> OSStatus;
        fn VTDecompressionSessionInvalidate(session: Ref);
    }

    /// Where the callback leaves the picture of the frame being decoded
    struct Picture {
        width: usize,
        height: usize,
        frame: Option<Vec<u8>>,
    }

    /// VideoToolbox calls this before `VTDecompressionSessionDecodeFrame` returns (no asynchronous decoding asked
    /// for): the picture's two planes, row by row without their padding, into one NV12 frame
    extern "C" fn decoded(
        _refcon: *mut c_void,
        frame_refcon: *mut c_void,
        status: OSStatus,
        _info: u32,
        image: Ref,
        _time: CMTime,
        _duration: CMTime,
    ) {
        if status != 0 || image.is_null() || frame_refcon.is_null() {
            return;
        }
        // SAFETY: frame_refcon is the `Picture` on `decode`'s stack, alive until the decode call returns; `image`
        // is a pixel buffer valid for this call, read only between lock and unlock and within each plane's rows
        unsafe {
            let picture = &mut *frame_refcon.cast::<Picture>();
            if CVPixelBufferLockBaseAddress(image, LOCK_READ_ONLY) != 0 {
                return;
            }
            let fits = CVPixelBufferGetPlaneCount(image) == 2
                && CVPixelBufferGetWidthOfPlane(image, 0) == picture.width
                && CVPixelBufferGetHeightOfPlane(image, 0) == picture.height
                && CVPixelBufferGetWidthOfPlane(image, 1) == picture.width / 2
                && CVPixelBufferGetHeightOfPlane(image, 1) == picture.height / 2;
            if fits {
                let mut frame = Vec::with_capacity(picture.width * picture.height * 3 / 2);
                for (plane, rows) in [(0, picture.height), (1, picture.height / 2)] {
                    let base = CVPixelBufferGetBaseAddressOfPlane(image, plane);
                    let stride = CVPixelBufferGetBytesPerRowOfPlane(image, plane);
                    if base.is_null() || stride < picture.width {
                        frame.clear();
                        break;
                    }
                    for row in 0..rows {
                        // Both planes are `width` bytes a row (the second holds width/2 pairs)
                        frame.extend_from_slice(std::slice::from_raw_parts(base.add(row * stride), picture.width));
                    }
                }
                if frame.len() == picture.width * picture.height * 3 / 2 {
                    picture.frame = Some(frame);
                }
            }
            CVPixelBufferUnlockBaseAddress(image, LOCK_READ_ONLY);
        }
    }

    /// One H.264 stream's decoder. The session starts with the first parameter sets and starts anew when they change
    pub struct Decoder {
        width: usize,
        height: usize,
        sets: Option<(Vec<u8>, Vec<u8>)>,
        format: Ref,
        session: Ref,
    }

    // SAFETY: the session and the format description are used by one thread at a time (`&mut self`), and
    // VideoToolbox objects may move between threads
    unsafe impl Send for Decoder {}

    impl Decoder {
        /// For pictures of exactly this size; others are dropped
        pub fn new(width: usize, height: usize) -> Decoder {
            Decoder { width, height, sets: None, format: null(), session: null() }
        }

        /// One access unit (Annex B) → its picture as NV12. None: no picture – before the first key frame, a
        /// picture of another size, or the decoder refused it (the next key frame starts anew)
        pub fn decode(&mut self, annex_b: &[u8]) -> Option<Vec<u8>> {
            let AccessUnit { sps, pps, mut slices } = access_unit(annex_b);
            if let (Some(sps), Some(pps)) = (sps, pps)
                && self.sets.as_ref().is_none_or(|(s, p)| *s != sps || *p != pps)
            {
                self.start(sps, pps);
            }
            if self.session.is_null() || slices.is_empty() {
                return None;
            }
            let mut picture = Picture { width: self.width, height: self.height, frame: None };
            // SAFETY: `slices` outlives the block buffer (released below, the decode call is synchronous) and is
            // never freed by it (kCFAllocatorNull); every out pointer is a valid local; what was created is released
            unsafe {
                let mut block: Ref = null();
                if CMBlockBufferCreateWithMemoryBlock(
                    null(),
                    slices.as_mut_ptr().cast(),
                    slices.len(),
                    kCFAllocatorNull,
                    null(),
                    0,
                    slices.len(),
                    0,
                    &mut block,
                ) != 0
                {
                    return None;
                }
                let mut sample: Ref = null();
                let size = slices.len();
                let made = CMSampleBufferCreateReady(null(), block, self.format, 1, 0, null(), 1, &size, &mut sample);
                if made == 0 {
                    let status = VTDecompressionSessionDecodeFrame(
                        self.session,
                        sample,
                        0,
                        (&raw mut picture).cast(),
                        null_mut(),
                    );
                    CFRelease(sample);
                    if status != 0 {
                        self.stop(); // a broken session: the next key frame's parameter sets start a new one
                    }
                }
                CFRelease(block);
            }
            picture.frame
        }

        fn start(&mut self, sps: Vec<u8>, pps: Vec<u8>) {
            self.stop();
            // SAFETY: the pointers are to locals valid for each call; created objects are kept in `self` (released
            // in `stop`) or released here
            unsafe {
                let pointers = [sps.as_ptr(), pps.as_ptr()];
                let sizes = [sps.len(), pps.len()];
                let mut format: Ref = null();
                if CMVideoFormatDescriptionCreateFromH264ParameterSets(
                    null(),
                    2,
                    pointers.as_ptr(),
                    sizes.as_ptr(),
                    4,
                    &mut format,
                ) != 0
                {
                    return;
                }
                let nv12 = NV12;
                let number = CFNumberCreate(null(), NUMBER_SINT32, (&raw const nv12).cast());
                let attributes = CFDictionaryCreate(
                    null(),
                    &kCVPixelBufferPixelFormatTypeKey,
                    &number,
                    1,
                    &raw const kCFTypeDictionaryKeyCallBacks,
                    &raw const kCFTypeDictionaryValueCallBacks,
                );
                let record = CallbackRecord { callback: decoded, refcon: null_mut() };
                let mut session: Ref = null();
                let status = VTDecompressionSessionCreate(null(), format, null(), attributes, &record, &mut session);
                CFRelease(attributes);
                CFRelease(number);
                if status != 0 {
                    CFRelease(format);
                    return;
                }
                self.format = format;
                self.session = session;
            }
            self.sets = Some((sps, pps));
        }

        fn stop(&mut self) {
            // SAFETY: both were created in `start` and are released exactly once (set to null after)
            unsafe {
                if !self.session.is_null() {
                    VTDecompressionSessionInvalidate(self.session);
                    CFRelease(self.session);
                }
                if !self.format.is_null() {
                    CFRelease(self.format);
                }
            }
            self.session = null();
            self.format = null();
            self.sets = None;
        }
    }

    impl Drop for Decoder {
        fn drop(&mut self) {
            self.stop();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_annex_b_with_both_start_codes() {
        let data = [0, 0, 0, 1, 9, 0x10, 0, 0, 1, 0x67, 1, 2, 0, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x65, 4, 0, 5];
        let units = split(&data);
        assert_eq!(units, vec![&[9, 0x10][..], &[0x67, 1, 2], &[0x68, 3], &[0x65, 4, 0, 5]]);
        assert!(split(&[1, 2, 3]).is_empty(), "no start code");
        assert!(split(&[0, 0, 1]).is_empty(), "nothing after it");
    }

    #[test]
    fn an_access_unit_is_parameter_sets_and_length_prefixed_slices() {
        let data =
            [0, 0, 0, 1, 9, 0x10, 0, 0, 0, 1, 0x67, 1, 0, 0, 0, 1, 0x68, 3, 0, 0, 1, 0x06, 7, 0, 0, 1, 0x65, 4, 5];
        let unit = access_unit(&data);
        assert_eq!(unit.sps.as_deref(), Some(&[0x67, 1][..]));
        assert_eq!(unit.pps.as_deref(), Some(&[0x68, 3][..]));
        assert_eq!(unit.slices, vec![0, 0, 0, 3, 0x65, 4, 5]);
        let later = access_unit(&[0, 0, 0, 1, 0x41, 9, 0, 0, 1, 0x41, 8]);
        assert_eq!((later.sps, later.pps), (None, None));
        assert_eq!(later.slices, vec![0, 0, 0, 2, 0x41, 9, 0, 0, 0, 2, 0x41, 8]);
    }

    /// VideoToolbox itself, with testdata/flip.h264 (four 1280×720 pictures: two dark, two bright; x264, one
    /// access unit a picture, each after a delimiter). Not in a build's sandbox:
    /// `cargo test -- --ignored the_decoder_gives_a_picture_for_each_frame`
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore]
    fn the_decoder_gives_a_picture_for_each_frame() {
        let data = std::fs::read(concat!(env!("CARGO_MANIFEST_DIR"), "/testdata/flip.h264")).unwrap();
        let delimiter = [0u8, 0, 0, 1, 9];
        let mut starts: Vec<usize> =
            (0..data.len() - 5).filter(|&i| data[i..i + 5] == delimiter).chain([data.len()]).collect();
        starts.dedup();
        let mut decoder = Decoder::new(1280, 720);
        assert_eq!(decoder.decode(&data[starts[1]..starts[2]]), None, "nothing before the first key frame");
        let means: Vec<u32> = starts
            .windows(2)
            .map(|w| {
                let frame = decoder.decode(&data[w[0]..w[1]]).expect("a picture for every frame");
                assert_eq!(frame.len(), 1280 * 720 * 3 / 2);
                frame[..1280 * 720].iter().map(|&b| u32::from(b)).sum::<u32>() / (1280 * 720)
            })
            .collect();
        assert_eq!(means.len(), 4);
        assert!(means[0] < 60 && means[1] < 60 && means[2] > 180 && means[3] > 180, "{means:?}");
        assert_eq!(Decoder::new(640, 360).decode(&data[starts[0]..starts[1]]), None, "a picture of another size");
    }
}
