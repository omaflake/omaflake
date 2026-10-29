// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Mac's camera as the V4L2 device /dev/video42 ("Mac Camera", v4l2loopback), fed with frames from the Mac
//! (spec §11.9, types 0x02xx). The agent asks for frames only while a Linux application reads the device, so the
//! Mac's camera and its indicator are on only then. Replaced the former camera-agent.py (same ioctls).

use std::fs::{File, OpenOptions};
use std::io::{self, Write};
use std::os::fd::{AsRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::time::{Duration, Instant};

pub const DEVICE: &str = "/dev/video42";
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;
pub const FRAME_BYTES: usize = (WIDTH * HEIGHT * 3 / 2) as usize;
/// Frames the Mac may have on the way: the guest's free buffers (spec §11.9, credits)
pub const CREDITS: u32 = 2;

// Linux UAPI (videodev2.h) and v4l2loopback 0.15.x – sizes for aarch64, as in camera-agent.py
const V4L2_BUF_TYPE_VIDEO_OUTPUT: u32 = 2;
const V4L2_FIELD_NONE: u32 = 1;
const V4L2_COLORSPACE_REC709: u32 = 3;
const V4L2_EVENT_PRI_CLIENT_USAGE: u32 = 0x0800_0000 + 0x08E0_0000 + 1;
const V4L2_EVENT_SUB_FL_SEND_INITIAL: u32 = 1;
const VIDIOC_S_FMT: libc::c_ulong = 0xc0d0_5605; // _IOWR('V', 5, struct v4l2_format), 208 bytes
const VIDIOC_DQEVENT: libc::c_ulong = 0x8088_5659; // _IOR('V', 89, struct v4l2_event), 136 bytes
const VIDIOC_SUBSCRIBE_EVENT: libc::c_ulong = 0x4020_565a; // _IOW('V', 90, struct v4l2_event_subscription), 32 bytes

#[repr(C, align(8))]
struct Raw<const N: usize>([u8; N]);

impl<const N: usize> Raw<N> {
    fn new() -> Self {
        Raw([0; N])
    }
    fn set(&mut self, offset: usize, value: u32) {
        self.0[offset..offset + 4].copy_from_slice(&value.to_ne_bytes());
    }
    fn get(&self, offset: usize) -> u32 {
        u32::from_ne_bytes(self.0[offset..offset + 4].try_into().unwrap())
    }
}

fn ioctl<const N: usize>(fd: RawFd, request: libc::c_ulong, arg: &mut Raw<N>) -> io::Result<()> {
    if unsafe { libc::ioctl(fd, request as _, arg.0.as_mut_ptr()) } < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// While no application reads, the black frame is written again this often (B130): a reader's first buffer is the
/// one written last before it started streaming, stamped when it was written – a frame from minutes ago made mpv and
/// ffmpeg wait those minutes for the next one
pub const IDLE_REFRESH: Duration = Duration::from_secs(1);

pub struct Camera {
    device: File,
    /// A Linux application has the camera open
    pub active: bool,
    /// When the last frame went to the device
    written: Instant,
}

impl Camera {
    /// Opens the loopback device, sets 1280×720 NV12 and listens for applications opening it
    pub fn open() -> io::Result<Camera> {
        let device = OpenOptions::new().read(true).write(true).custom_flags(libc::O_CLOEXEC).open(DEVICE)?;
        let fd = device.as_raw_fd();
        let mut f = Raw::<208>::new();
        f.set(0, V4L2_BUF_TYPE_VIDEO_OUTPUT);
        for (offset, value) in [
            (8, WIDTH),
            (12, HEIGHT),
            (16, u32::from_le_bytes(*b"NV12")),
            (20, V4L2_FIELD_NONE),
            (24, WIDTH),
            (28, FRAME_BYTES as u32),
            (32, V4L2_COLORSPACE_REC709),
        ] {
            f.set(offset, value);
        }
        ioctl(fd, VIDIOC_S_FMT, &mut f)?;
        if (f.get(8), f.get(12)) != (WIDTH, HEIGHT) {
            return Err(io::Error::other("v4l2loopback refused 1280x720 NV12"));
        }
        let mut camera = Camera { device, active: false, written: Instant::now() };
        // A first frame makes the exclusive-caps device announce itself as a capture device (browsers find it)
        camera.black();
        let mut s = Raw::<32>::new();
        s.set(0, V4L2_EVENT_PRI_CLIENT_USAGE);
        s.set(8, V4L2_EVENT_SUB_FL_SEND_INITIAL);
        ioctl(fd, VIDIOC_SUBSCRIBE_EVENT, &mut s)?;
        Ok(camera)
    }

    pub fn fd(&self) -> RawFd {
        self.device.as_raw_fd()
    }

    /// After POLLPRI: whether an application has the device open now
    pub fn readers(&mut self) -> io::Result<bool> {
        let mut e = Raw::<136>::new();
        ioctl(self.device.as_raw_fd(), VIDIOC_DQEVENT, &mut e)?;
        Ok(e.get(0) == V4L2_EVENT_PRI_CLIENT_USAGE && e.get(8) > 0)
    }

    pub fn write_frame(&mut self, frame: &[u8]) -> io::Result<()> {
        if frame.len() != FRAME_BYTES {
            return Err(io::Error::other("frame of the wrong size"));
        }
        self.written = Instant::now();
        self.device.write_all(frame)
    }

    /// A black frame – shown while the Mac sends nothing
    pub fn black(&mut self) {
        let mut frame = vec![16u8; (WIDTH * HEIGHT) as usize];
        frame.resize(FRAME_BYTES, 128);
        self.written = Instant::now();
        let _ = self.device.write_all(&frame);
    }

    /// No reader: the black frame again once `IDLE_REFRESH` has passed, so a new reader starts with a fresh one
    pub fn keep_fresh(&mut self, now: Instant) {
        if !self.active && now.duration_since(self.written) >= IDLE_REFRESH {
            self.black();
        }
    }
}
