// SPDX-License-Identifier: AGPL-3.0-or-later
//! A V4L2 camera of the Linux host as a `FrameSource`: memory-mapped streaming capture, asked for 1280×720 in NV12,
//! YUYV or MJPEG (in that order; another size the driver picks is scaled), converted by `convert`. Plain ioctls on
//! the 64-bit kernel ABI (x86_64 and aarch64 share it), no libv4l and no bindgen. The device: OMAFLAKE_CAMERA_DEVICE,
//! else the first `/dev/video*` that can capture in one of the formats. Opening it needs read-write access – systemd
//! gives the user at the seat an ACL (`uaccess`), else membership in the group `video`.
//!
//! Struct sizes and offsets as in linux/videodev2.h on LP64, checked with a C program against the headers of the
//! nixpkgs kernel on aarch64-linux (plan B126): v4l2_capability 104, v4l2_format 208 (pix at 8), v4l2_requestbuffers
//! 20, v4l2_buffer 88 (memory at 60, m.offset at 64, length at 72).

use std::ffi::CString;
use std::os::fd::RawFd;
use std::time::Duration;

use crate::host_bridge::camera::FrameSource;
use crate::host_bridge::convert::{to_nv12, Picture, PixelFormat, HEIGHT, WIDTH};

const BUF_TYPE_VIDEO_CAPTURE: u32 = 1;
const MEMORY_MMAP: u32 = 1;
const FIELD_ANY: u32 = 0;
const CAP_VIDEO_CAPTURE: u32 = 0x0000_0001;
const CAP_STREAMING: u32 = 0x0400_0000;
const CAP_DEVICE_CAPS: u32 = 0x8000_0000;

// _IOR/_IOW/_IOWR('V', nr, size): direction << 30 | size << 16 | 'V' << 8 | nr
const VIDIOC_QUERYCAP: libc::c_ulong = 0x8068_5600; // _IOR 0, 104
const VIDIOC_S_FMT: libc::c_ulong = 0xc0d0_5605; // _IOWR 5, 208
const VIDIOC_REQBUFS: libc::c_ulong = 0xc014_5608; // _IOWR 8, 20
const VIDIOC_QUERYBUF: libc::c_ulong = 0xc058_5609; // _IOWR 9, 88
const VIDIOC_QBUF: libc::c_ulong = 0xc058_560f; // _IOWR 15, 88
const VIDIOC_DQBUF: libc::c_ulong = 0xc058_5611; // _IOWR 17, 88
const VIDIOC_STREAMON: libc::c_ulong = 0x4004_5612; // _IOW 18, int
const VIDIOC_STREAMOFF: libc::c_ulong = 0x4004_5613; // _IOW 19, int

/// Buffers the driver fills while one is converted
const BUFFERS: u32 = 4;

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
        u32::from_ne_bytes(self.0[offset..offset + 4].try_into().expect("4 bytes"))
    }
    fn text(&self, range: std::ops::Range<usize>) -> String {
        let b = &self.0[range];
        String::from_utf8_lossy(&b[..b.iter().position(|&c| c == 0).unwrap_or(b.len())]).into_owned()
    }
}

fn ioctl<T>(fd: RawFd, request: libc::c_ulong, arg: &mut T) -> std::io::Result<()> {
    loop {
        // SAFETY: `arg` is a buffer of the size the request encodes (the constants above)
        if unsafe { libc::ioctl(fd, request as _, arg as *mut T) } >= 0 {
            return Ok(());
        }
        let e = std::io::Error::last_os_error();
        if e.kind() != std::io::ErrorKind::Interrupted {
            return Err(e);
        }
    }
}

/// What a device says about itself (VIDIOC_QUERYCAP)
struct Capability {
    card: String,
    capture: bool,
}

fn query(fd: RawFd) -> std::io::Result<Capability> {
    let mut c = Raw::<104>::new();
    ioctl(fd, VIDIOC_QUERYCAP, &mut c)?;
    let caps = c.get(84);
    let device = if caps & CAP_DEVICE_CAPS != 0 { c.get(88) } else { caps };
    Ok(Capability { card: c.text(16..48), capture: device & CAP_VIDEO_CAPTURE != 0 && device & CAP_STREAMING != 0 })
}

fn open_device(path: &str) -> std::io::Result<RawFd> {
    let c = CString::new(path).map_err(std::io::Error::other)?;
    // SAFETY: a NUL-terminated path
    let fd = unsafe { libc::open(c.as_ptr(), libc::O_RDWR | libc::O_NONBLOCK | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(fd)
}

fn close(fd: RawFd) {
    // SAFETY: a descriptor this module opened
    unsafe { libc::close(fd) };
}

/// The format the device agreed to (VIDIOC_S_FMT): the first of `PixelFormat::PREFERRED` it takes as asked
fn set_format(fd: RawFd) -> std::io::Result<(PixelFormat, usize, usize, usize)> {
    for format in PixelFormat::PREFERRED {
        let mut f = Raw::<208>::new();
        f.set(0, BUF_TYPE_VIDEO_CAPTURE);
        f.set(8, WIDTH as u32);
        f.set(12, HEIGHT as u32);
        f.set(16, format.fourcc());
        f.set(20, FIELD_ANY);
        if ioctl(fd, VIDIOC_S_FMT, &mut f).is_err() || f.get(16) != format.fourcc() {
            continue;
        }
        return Ok((format, f.get(8) as usize, f.get(12) as usize, f.get(24) as usize));
    }
    Err(std::io::Error::other("none of NV12, YUYV, MJPEG"))
}

/// The cameras of this host, in the order they are tried: OMAFLAKE_CAMERA_DEVICE, else /dev/video0, 1, …
pub fn candidates() -> Vec<String> {
    if let Ok(device) = std::env::var("OMAFLAKE_CAMERA_DEVICE") {
        return vec![device];
    }
    let mut numbers: Vec<u32> = std::fs::read_dir("/dev")
        .map(|d| d.flatten().filter_map(|e| e.file_name().to_str()?.strip_prefix("video")?.parse().ok()).collect())
        .unwrap_or_default();
    numbers.sort_unstable();
    numbers.into_iter().map(|n| format!("/dev/video{n}")).collect()
}

struct Mapped {
    ptr: *mut libc::c_void,
    len: usize,
}

/// One open, streaming camera
struct Stream {
    fd: RawFd,
    format: PixelFormat,
    width: usize,
    height: usize,
    stride: usize,
    buffers: Vec<Mapped>,
}

impl Stream {
    fn open(path: &str) -> Result<(Stream, String), String> {
        let fd = open_device(path).map_err(|e| format!("{path}: {e}"))?;
        let mut stream = Stream { fd, format: PixelFormat::Nv12, width: 0, height: 0, stride: 0, buffers: vec![] };
        let card = stream.setup().map_err(|e| format!("{path}: {e}"))?;
        Ok((stream, card))
    }

    fn setup(&mut self) -> std::io::Result<String> {
        let cap = query(self.fd)?;
        if !cap.capture {
            return Err(std::io::Error::other("not a streaming capture device"));
        }
        (self.format, self.width, self.height, self.stride) = set_format(self.fd)?;
        let mut req = Raw::<20>::new();
        req.set(0, BUFFERS);
        req.set(4, BUF_TYPE_VIDEO_CAPTURE);
        req.set(8, MEMORY_MMAP);
        ioctl(self.fd, VIDIOC_REQBUFS, &mut req)?;
        for index in 0..req.get(0) {
            let mut b = Raw::<88>::new();
            b.set(0, index);
            b.set(4, BUF_TYPE_VIDEO_CAPTURE);
            b.set(60, MEMORY_MMAP);
            ioctl(self.fd, VIDIOC_QUERYBUF, &mut b)?;
            let (offset, len) = (b.get(64), b.get(72) as usize);
            // SAFETY: mapping the driver's buffer as it described it
            let ptr = unsafe {
                libc::mmap(std::ptr::null_mut(), len, libc::PROT_READ, libc::MAP_SHARED, self.fd, offset as libc::off_t)
            };
            if ptr == libc::MAP_FAILED {
                return Err(std::io::Error::last_os_error());
            }
            self.buffers.push(Mapped { ptr, len });
            ioctl(self.fd, VIDIOC_QBUF, &mut b)?;
        }
        if self.buffers.is_empty() {
            return Err(std::io::Error::other("no buffers"));
        }
        let mut kind = BUF_TYPE_VIDEO_CAPTURE as libc::c_int;
        ioctl(self.fd, VIDIOC_STREAMON, &mut kind)?;
        let (w, h) = (self.width, self.height);
        Ok(format!("{} {w}×{h} {}", query(self.fd).map(|c| c.card).unwrap_or_default(), self.format.name()))
    }

    fn frame(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, String> {
        let mut pfd = libc::pollfd { fd: self.fd, events: libc::POLLIN, revents: 0 };
        // SAFETY: one valid pollfd
        let n = unsafe { libc::poll(&mut pfd, 1, timeout.as_millis().min(i32::MAX as u128) as i32) };
        if n <= 0 {
            return Ok(None);
        }
        if pfd.revents & (libc::POLLERR | libc::POLLHUP) != 0 && pfd.revents & libc::POLLIN == 0 {
            return Err("the camera went away".into());
        }
        let mut b = Raw::<88>::new();
        b.set(4, BUF_TYPE_VIDEO_CAPTURE);
        b.set(60, MEMORY_MMAP);
        if let Err(e) = ioctl(self.fd, VIDIOC_DQBUF, &mut b) {
            return if e.kind() == std::io::ErrorKind::WouldBlock { Ok(None) } else { Err(format!("capture: {e}")) };
        }
        let index = b.get(0) as usize;
        let used = b.get(8) as usize;
        let frame = self.buffers.get(index).and_then(|m| {
            // SAFETY: the driver handed this buffer to us until it is queued again below
            let data = unsafe { std::slice::from_raw_parts(m.ptr as *const u8, used.min(m.len)) };
            let p = Picture { format: self.format, width: self.width, height: self.height, stride: self.stride, data };
            to_nv12(&p)
        });
        ioctl(self.fd, VIDIOC_QBUF, &mut b).map_err(|e| format!("capture: {e}"))?;
        Ok(frame)
    }
}

impl Drop for Stream {
    fn drop(&mut self) {
        let mut kind = BUF_TYPE_VIDEO_CAPTURE as libc::c_int;
        let _ = ioctl(self.fd, VIDIOC_STREAMOFF, &mut kind);
        for m in &self.buffers {
            // SAFETY: mapped in `setup` with this length
            unsafe { libc::munmap(m.ptr, m.len) };
        }
        close(self.fd);
    }
}

// SAFETY: the mappings belong to the stream and are used only by the thread that owns it
unsafe impl Send for Stream {}

/// The host's camera, opened on "start" and closed on "stop" (the camera's light is on only between them)
#[derive(Default)]
pub struct V4l2Camera {
    stream: Option<Stream>,
}

impl V4l2Camera {
    pub fn new() -> V4l2Camera {
        V4l2Camera::default()
    }
}

impl FrameSource for V4l2Camera {
    fn open(&mut self) -> Result<String, String> {
        let mut problems = vec![];
        for path in candidates() {
            match Stream::open(&path) {
                Ok((stream, detail)) => {
                    self.stream = Some(stream);
                    return Ok(format!("{path}: {detail}"));
                }
                Err(e) => problems.push(e),
            }
        }
        Err(if problems.is_empty() { "no camera found (/dev/video*)".to_string() } else { problems.join("; ") })
    }

    fn frame(&mut self, timeout: Duration) -> Result<Option<Vec<u8>>, String> {
        let Some(stream) = self.stream.as_mut() else { return Ok(None) };
        let result = stream.frame(timeout);
        if result.is_err() {
            self.stream = None;
        }
        result
    }

    fn close(&mut self) {
        self.stream = None;
    }
}
