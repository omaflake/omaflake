// SPDX-License-Identifier: AGPL-3.0-or-later
//! The Mac's camera in macOS guests (plan B169): frames from the Mac (spec §11.9, types 0x02xx) as a raw NV12
//! 1280×720 stream on `127.0.0.1:47042`, for one reader in the guest. Until omaflake's own camera extension (M12)
//! that reader is OBS (a media source, `rawvideo` – obs.rs writes its scene), whose virtual camera the guest's apps
//! use; later the extension reads the same endpoint. The agent asks for frames only while a reader is connected,
//! so the Mac's camera and its light are on only then.
//!
//! Frames go to the reader on a thread of their own, the newest only; a credit goes back to the Mac for each frame
//! the reader took (or that a newer one replaced) – the reader's pace is the Mac's.

use std::io::{self, ErrorKind, Write};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::os::fd::{AsRawFd, RawFd};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

pub const ADDRESS: &str = "127.0.0.1:47042";
/// Where the frames are, as the diagnosis and the docs name it
pub const DEVICE: &str = "tcp://127.0.0.1:47042";
pub const WIDTH: u32 = 1280;
pub const HEIGHT: u32 = 720;
pub const FRAME_BYTES: usize = (WIDTH * HEIGHT * 3 / 2) as usize;
/// Frames the Mac may have on the way (spec §11.9, credits)
pub const CREDITS: u32 = 2;

/// Whether this guest can show the Mac's camera at all (macOS: always – the endpoint is the agent's own)
pub fn available() -> bool {
    true
}

/// The frame waiting for the writer thread, and the reader it goes to
#[derive(Default)]
struct Slot {
    frame: Option<Vec<u8>>,
    reader: Option<TcpStream>,
    /// Bumped with every new reader: a write that began for an earlier one returns no credit
    generation: u64,
}

pub struct Camera {
    listener: TcpListener,
    /// The connected reader (our handle: hang-ups and closing; the writer thread writes to a clone)
    reader: Option<TcpStream>,
    shared: Arc<(Mutex<Slot>, Condvar)>,
    /// A reader is connected
    pub active: bool,
    /// When the last frame went to a reader (the diagnosis)
    pub last_frame: Option<Instant>,
}

impl Camera {
    /// Listens on `ADDRESS`; the writer thread sends credits back with `credit`
    pub fn open(credit: fn(u32)) -> io::Result<Camera> {
        Self::open_at(ADDRESS, credit)
    }

    fn open_at(address: &str, credit: fn(u32)) -> io::Result<Camera> {
        let listener = TcpListener::bind(address)?;
        listener.set_nonblocking(true)?;
        let shared = Arc::new((Mutex::new(Slot::default()), Condvar::new()));
        let s = shared.clone();
        std::thread::Builder::new().name("camera".into()).spawn(move || writer(&s, credit))?;
        Ok(Camera { listener, reader: None, shared, active: false, last_frame: None })
    }

    /// What `poll` should watch: new readers, and the reader hanging up (it never sends anything)
    pub fn poll_fds(&self) -> Vec<(RawFd, i16)> {
        let mut fds = vec![(self.listener.as_raw_fd(), libc::POLLIN)];
        if let Some(r) = &self.reader {
            fds.push((r.as_raw_fd(), libc::POLLIN));
        }
        fds
    }

    /// After `poll`, with the revents of `poll_fds` in order: `Some(active)` when a reader came or went
    pub fn events(&mut self, revents: &[i16]) -> io::Result<Option<bool>> {
        let before = self.active;
        if revents.get(1).is_some_and(|&r| r != 0) && self.reader_gone() {
            self.drop_reader();
        }
        if revents.first().is_some_and(|&r| r & libc::POLLIN != 0) {
            loop {
                match self.listener.accept() {
                    Ok((stream, _)) => self.take_reader(stream)?,
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break,
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) => return Err(e),
                }
            }
        }
        // The writer thread found the reader gone (a failed write)
        if self.active && self.shared.0.lock().unwrap_or_else(|e| e.into_inner()).reader.is_none() {
            self.drop_reader();
        }
        Ok((self.active != before).then_some(self.active))
    }

    /// A reader sends nothing: readable means it hung up (or sent something – read and dropped). Reads with
    /// MSG_DONTWAIT: the socket itself stays blocking for the writer thread (both handles share its flags)
    fn reader_gone(&mut self) -> bool {
        let Some(r) = self.reader.as_ref() else { return false };
        let mut buf = [0u8; 256];
        loop {
            // SAFETY: buf is valid for its length; the fd belongs to `r`, open for this call
            let n = unsafe { libc::recv(r.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len(), libc::MSG_DONTWAIT) };
            if n > 0 {
                continue;
            }
            if n == 0 {
                return true;
            }
            match io::Error::last_os_error().kind() {
                ErrorKind::WouldBlock => return false,
                ErrorKind::Interrupted => {}
                _ => return true,
            }
        }
    }

    /// A new reader replaces the one before
    fn take_reader(&mut self, stream: TcpStream) -> io::Result<()> {
        if let Some(old) = self.reader.take() {
            let _ = old.shutdown(Shutdown::Both);
        }
        // Blocking: the writer thread waits for a slow reader. On macOS an accepted socket inherits the listener's
        // O_NONBLOCK, and a clone shares it – a non-blocking writer would give up on any reader that falls behind
        stream.set_nonblocking(false)?;
        let _ = stream.set_nodelay(true);
        let writer_end = stream.try_clone()?;
        {
            let mut slot = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
            slot.frame = None;
            slot.reader = Some(writer_end);
            slot.generation += 1;
        }
        self.reader = Some(stream);
        self.active = true;
        Ok(())
    }

    fn drop_reader(&mut self) {
        if let Some(r) = self.reader.take() {
            let _ = r.shutdown(Shutdown::Both);
        }
        let mut slot = self.shared.0.lock().unwrap_or_else(|e| e.into_inner());
        slot.frame = None;
        slot.reader = None;
        slot.generation += 1;
        self.active = false;
    }

    /// A frame from the Mac; returns the credits to give back at once (a replaced frame's)
    pub fn write_frame(&mut self, frame: Vec<u8>) -> io::Result<u32> {
        if frame.len() != FRAME_BYTES {
            return Err(io::Error::other("frame of the wrong size"));
        }
        let (lock, wake) = &*self.shared;
        let mut slot = lock.lock().unwrap_or_else(|e| e.into_inner());
        if slot.reader.is_none() {
            return Ok(0); // late frame after the reader went: the Mac stops anyway
        }
        let replaced = slot.frame.replace(frame).is_some();
        wake.notify_one();
        self.last_frame = Some(Instant::now());
        Ok(u32::from(replaced))
    }

    /// Linux keeps its device fresh while unused; here there is nothing to do
    pub fn keep_fresh(&mut self, _now: Instant) {}
}

/// Writes the newest frame to the reader; one credit back per frame written
fn writer(shared: &(Mutex<Slot>, Condvar), credit: fn(u32)) {
    let (lock, wake) = shared;
    loop {
        let (frame, mut reader, generation) = {
            let mut slot = lock.lock().unwrap_or_else(|e| e.into_inner());
            loop {
                if slot.frame.is_some()
                    && let Some(r) = slot.reader.as_ref().and_then(|r| r.try_clone().ok())
                {
                    break (slot.frame.take().expect("checked"), r, slot.generation);
                }
                slot = wake.wait(slot).unwrap_or_else(|e| e.into_inner());
            }
        };
        let ok = reader.write_all(&frame).is_ok();
        let mut slot = lock.lock().unwrap_or_else(|e| e.into_inner());
        if slot.generation != generation {
            continue; // the reader changed meanwhile: the Mac's credits started anew
        }
        if ok {
            drop(slot);
            credit(1);
        } else {
            slot.reader = None; // the main loop sees it and tells the Mac to stop
            slot.frame = None;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Read;
    use std::sync::atomic::{AtomicU32, Ordering};
    use std::time::Duration;

    static CREDITED: AtomicU32 = AtomicU32::new(0);
    fn count(n: u32) {
        CREDITED.fetch_add(n, Ordering::SeqCst);
    }

    fn events(cam: &mut Camera) -> Option<bool> {
        let mut fds: Vec<libc::pollfd> =
            cam.poll_fds().into_iter().map(|(fd, events)| libc::pollfd { fd, events, revents: 0 }).collect();
        // SAFETY: fds is a valid array of pollfd for its length
        unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 1000) };
        cam.events(&fds.iter().map(|f| f.revents).collect::<Vec<_>>()).unwrap()
    }

    #[test]
    fn a_reader_gets_whole_frames_and_returns_credits() {
        let mut cam = Camera::open_at("127.0.0.1:0", count).unwrap(); // a free port: an agent may run here
        let address = cam.listener.local_addr().unwrap();
        assert!(!cam.active);
        assert_eq!(cam.write_frame(vec![1; FRAME_BYTES]).unwrap(), 0, "no reader: dropped, no credit");

        let mut reader = TcpStream::connect(address).unwrap();
        reader.set_read_timeout(Some(Duration::from_secs(10))).unwrap(); // a broken writer fails, never hangs
        assert_eq!(events(&mut cam), Some(true));
        assert!(cam.write_frame(vec![7; 10]).is_err());
        cam.write_frame(vec![7; FRAME_BYTES]).unwrap();
        // The reader falls behind: the writer waits for it instead of giving up (the socket's buffer is far smaller
        // than a frame)
        std::thread::sleep(Duration::from_millis(300));
        let mut got = vec![0u8; FRAME_BYTES];
        reader.read_exact(&mut got).unwrap();
        assert!(got.iter().all(|&b| b == 7));
        let deadline = Instant::now() + Duration::from_secs(2);
        while CREDITED.load(Ordering::SeqCst) < 1 && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        assert_eq!(CREDITED.load(Ordering::SeqCst), 1, "a credit for the frame the reader took");

        drop(reader);
        assert_eq!(events(&mut cam), Some(false));
        assert!(!cam.active);
    }
}
