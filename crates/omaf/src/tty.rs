// SPDX-License-Identifier: AGPL-3.0-or-later
//! Terminal helpers: a password prompt without echo (Swift `readpassphrase(…, RPP_REQUIRE_TTY)`) and Ctrl-C during a
//! build (Swift: `SIGINT` ignored, a dispatch source cancels the build's token instead).

use std::fs::{File, OpenOptions};
use std::io::{Read, Write};
use std::os::fd::AsRawFd;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Mutex;
use std::thread::JoinHandle;

use omaflake_core::{Error, Result};

/// The terminal whose echo is off while a password is read (-1: none) and its settings before, so a Ctrl-C at the
/// prompt can switch the echo back on before it quits (readpassphrase does the same).
static PROMPT_TTY: AtomicI32 = AtomicI32::new(-1);
static mut PROMPT_SAVED: std::mem::MaybeUninit<libc::termios> = std::mem::MaybeUninit::uninit();

extern "C" fn restore_and_quit(signal: libc::c_int) {
    let fd = PROMPT_TTY.load(Ordering::SeqCst);
    // SAFETY: async-signal-safe calls only; PROMPT_SAVED is written before PROMPT_TTY is set and not changed while
    // it is set
    unsafe {
        if fd >= 0 {
            libc::tcsetattr(fd, libc::TCSAFLUSH, (&raw const PROMPT_SAVED).cast());
            libc::write(fd, b"\n".as_ptr().cast(), 1);
        }
        libc::signal(signal, libc::SIG_DFL);
        libc::raise(signal);
    }
}

/// Reads one line from the terminal (`/dev/tty`) with echo off, after writing `prompt` there. Fails without a terminal.
pub fn read_passphrase(prompt: &str) -> Result<String> {
    let no_terminal = || Error::new("Cannot read a password (no terminal? use --password-stdin)");
    let mut tty: File = OpenOptions::new().read(true).write(true).open("/dev/tty").map_err(|_| no_terminal())?;
    let fd = tty.as_raw_fd();
    let mut saved = std::mem::MaybeUninit::<libc::termios>::uninit();
    // SAFETY: tcgetattr fills the termios struct for an open descriptor
    if unsafe { libc::tcgetattr(fd, saved.as_mut_ptr()) } != 0 {
        return Err(no_terminal());
    }
    // SAFETY: initialized by the successful tcgetattr above
    let saved = unsafe { saved.assume_init() };
    let mut quiet = saved;
    quiet.c_lflag &= !(libc::ECHO | libc::ECHONL);
    // SAFETY: only this function writes PROMPT_SAVED, before publishing the descriptor; the handlers only call
    // async-signal-safe functions
    let old: Vec<libc::sighandler_t> = unsafe {
        (&raw mut PROMPT_SAVED).write(std::mem::MaybeUninit::new(saved));
        PROMPT_TTY.store(fd, Ordering::SeqCst);
        [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT]
            .iter()
            .map(|&s| libc::signal(s, restore_and_quit as *const () as libc::sighandler_t))
            .collect()
    };
    let _ = tty.write_all(prompt.as_bytes());
    let _ = tty.flush();
    // SAFETY: fd is open; quiet is a valid termios
    unsafe { libc::tcsetattr(fd, libc::TCSAFLUSH, &quiet) };
    let mut bytes = Vec::new();
    let mut byte = [0u8; 1];
    while bytes.len() < 1023 {
        match tty.read(&mut byte) {
            Ok(1) if byte[0] != b'\n' && byte[0] != b'\r' => bytes.push(byte[0]),
            _ => break,
        }
    }
    // SAFETY: as above – the saved settings back, the handlers as they were
    unsafe {
        libc::tcsetattr(fd, libc::TCSAFLUSH, &saved);
        PROMPT_TTY.store(-1, Ordering::SeqCst);
        for (s, h) in [libc::SIGINT, libc::SIGTERM, libc::SIGHUP, libc::SIGQUIT].iter().zip(old) {
            libc::signal(*s, h);
        }
    }
    let _ = tty.write_all(b"\n");
    let text = String::from_utf8_lossy(&bytes).into_owned();
    bytes.iter_mut().for_each(|b| *b = 0);
    Ok(text)
}

/// Write end of the pipe the SIGINT handler signals through (-1: no handler)
static INTERRUPT_PIPE: AtomicI32 = AtomicI32::new(-1);

extern "C" fn on_interrupt(_: libc::c_int) {
    let fd = INTERRUPT_PIPE.load(Ordering::SeqCst);
    if fd >= 0 {
        // SAFETY: write is async-signal-safe; the descriptor stays open while it is published
        unsafe { libc::write(fd, [1u8].as_ptr().cast(), 1) };
    }
}

/// While it lives, Ctrl-C (SIGINT) calls `handler` on a thread of its own instead of ending the process; dropping it
/// restores the default.
pub struct Interrupts {
    write: File,
    thread: Option<JoinHandle<()>>,
}

static INTERRUPTS: Mutex<()> = Mutex::new(());

impl Interrupts {
    pub fn handle(handler: impl Fn() + Send + 'static) -> Result<Interrupts> {
        let _one = INTERRUPTS.lock().unwrap_or_else(|e| e.into_inner());
        let mut fds = [0; 2];
        // SAFETY: pipe fills two descriptors that are then owned by the Files below
        if unsafe { libc::pipe(fds.as_mut_ptr()) } != 0 {
            return Err(Error::from(std::io::Error::last_os_error()));
        }
        // SAFETY: fresh descriptors from pipe, owned from here on
        let (mut read, write) = unsafe {
            use std::os::fd::FromRawFd;
            (File::from_raw_fd(fds[0]), File::from_raw_fd(fds[1]))
        };
        let thread = std::thread::spawn(move || {
            let mut byte = [0u8; 1];
            while let Ok(1) = read.read(&mut byte) {
                if byte[0] != 1 {
                    break;
                }
                handler();
            }
        });
        INTERRUPT_PIPE.store(write.as_raw_fd(), Ordering::SeqCst);
        // SAFETY: the handler only calls write (async-signal-safe)
        unsafe { libc::signal(libc::SIGINT, on_interrupt as *const () as libc::sighandler_t) };
        Ok(Interrupts { write, thread: Some(thread) })
    }
}

impl Drop for Interrupts {
    fn drop(&mut self) {
        // SAFETY: back to the default disposition
        unsafe { libc::signal(libc::SIGINT, libc::SIG_DFL) };
        INTERRUPT_PIPE.store(-1, Ordering::SeqCst);
        let _ = self.write.write_all(&[0]);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}
