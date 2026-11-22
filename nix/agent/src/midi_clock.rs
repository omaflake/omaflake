// SPDX-License-Identifier: AGPL-3.0-or-later
//! The sender's clock on ours (spec §11.9 "Timestamps", plan B161, C7): a MIDI record may carry the nanoseconds of
//! the Mac's monotonic clock when the data arrived there. The two clocks have different origins, so the receiver
//! learns the offset: the smallest `own_now − time` seen over the last 30 s – the record that crossed the transport
//! fastest – in a sliding window that keeps the newest minimum when it slides. The data are handed on for
//! `time + offset + 2 ms`: records that took longer than the fastest are played earlier by that much (or now), so the
//! transport's jitter is smoothed by up to 2 ms and notes keep their spacing.

use std::collections::VecDeque;

/// How far back the minimum is taken
pub const WINDOW_NS: u64 = 30_000_000_000;
/// The delay every timestamped record gets on top of the fastest transport seen
pub const LATENCY_NS: u64 = 2_000_000;
/// A delivery time further ahead than this is not trusted: delivered now
pub const MAX_AHEAD_NS: u64 = 1_000_000_000;

#[derive(Debug, Default)]
pub struct ClockMapper {
    /// (own time seen, own_now − time), the differences increasing from front to back: the front is the window's
    /// minimum; a newer sample with a smaller or equal difference replaces the older ones behind it
    samples: VecDeque<(u64, i128)>,
}

impl ClockMapper {
    pub fn new() -> ClockMapper {
        ClockMapper::default()
    }

    /// One record: its sender time `time`, seen at our `now`
    pub fn observe(&mut self, time: u64, now: u64) {
        let diff = now as i128 - time as i128;
        while self.samples.back().is_some_and(|&(_, d)| d >= diff) {
            self.samples.pop_back();
        }
        self.samples.push_back((now, diff));
        while self.samples.front().is_some_and(|&(seen, _)| seen.saturating_add(WINDOW_NS) < now) {
            self.samples.pop_front();
        }
    }

    /// The current offset (own clock minus the sender's), None before the first record
    pub fn offset(&self) -> Option<i128> {
        self.samples.front().map(|&(_, d)| d)
    }

    /// When a record stamped `time` and arriving at `now` should be played, on our clock; None: now (the time already
    /// past, or more than MAX_AHEAD_NS ahead). Observes the record first – so the offset is at most `now − time` and
    /// the result at most LATENCY_NS ahead; the MAX_AHEAD_NS check guards the spec's rule all the same.
    pub fn deliver_at(&mut self, time: u64, now: u64) -> Option<u64> {
        self.observe(time, now);
        let at = time as i128 + self.offset()? + LATENCY_NS as i128;
        if at <= now as i128 || at > now as i128 + MAX_AHEAD_NS as i128 {
            return None;
        }
        Some(at as u64)
    }
}

/// Our monotonic clock in nanoseconds – on macOS mach_absolute_time (the clock of CoreMIDI's time stamps, also
/// CLOCK_UPTIME_RAW), on Linux CLOCK_MONOTONIC (the clock of the ALSA sequencer's real-time queue)
#[cfg(target_os = "macos")]
pub fn now_ns() -> u64 {
    // SAFETY: mach_absolute_time has no preconditions
    ticks_to_ns(unsafe { mach::mach_absolute_time() })
}

#[cfg(not(target_os = "macos"))]
pub fn now_ns() -> u64 {
    let mut ts = libc::timespec { tv_sec: 0, tv_nsec: 0 };
    // SAFETY: `ts` is a valid timespec to write to; CLOCK_MONOTONIC always exists on Linux
    unsafe { libc::clock_gettime(libc::CLOCK_MONOTONIC, &mut ts) };
    ts.tv_sec as u64 * 1_000_000_000 + ts.tv_nsec as u64
}

#[cfg(target_os = "macos")]
mod mach {
    #[repr(C)]
    pub struct TimebaseInfo {
        pub numer: u32,
        pub denom: u32,
    }
    unsafe extern "C" {
        pub fn mach_absolute_time() -> u64;
        pub fn mach_timebase_info(info: *mut TimebaseInfo) -> i32;
    }
}

/// mach_timebase_info: host ticks × numer / denom = nanoseconds (Apple silicon: 125/3, Intel 1/1)
#[cfg(target_os = "macos")]
fn timebase() -> (u128, u128) {
    static TB: std::sync::OnceLock<(u128, u128)> = std::sync::OnceLock::new();
    *TB.get_or_init(|| {
        let mut info = mach::TimebaseInfo { numer: 0, denom: 0 };
        // SAFETY: `info` is a valid TimebaseInfo to write to
        let rc = unsafe { mach::mach_timebase_info(&mut info) };
        if rc != 0 || info.numer == 0 || info.denom == 0 { (1, 1) } else { (info.numer as u128, info.denom as u128) }
    })
}

/// Host ticks (CoreMIDI's MIDITimeStamp) in nanoseconds
#[cfg(target_os = "macos")]
pub fn ticks_to_ns(ticks: u64) -> u64 {
    let (numer, denom) = timebase();
    (ticks as u128 * numer / denom) as u64
}

/// Nanoseconds in host ticks
#[cfg(target_os = "macos")]
pub fn ns_to_ticks(ns: u64) -> u64 {
    let (numer, denom) = timebase();
    (ns as u128 * denom / numer) as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    const MS: u64 = 1_000_000;
    const S: u64 = 1_000_000_000;

    #[test]
    fn the_first_record_sets_the_offset_and_is_delayed_by_the_latency() {
        let mut c = ClockMapper::new();
        assert_eq!(c.offset(), None);
        // the sender's clock is 100 s ahead of ours
        assert_eq!(c.deliver_at(200 * S, 100 * S), Some(100 * S + 2 * MS));
        assert_eq!(c.offset(), Some(-(100 * S as i128)));
    }

    #[test]
    fn jitter_is_smoothed_and_spacing_kept() {
        let mut c = ClockMapper::new();
        // fastest transport: 1 ms; notes 10 ms apart on the sender, the second one 1.5 ms slower in transit
        assert_eq!(c.deliver_at(10 * S, 10 * S + MS), Some(10 * S + 3 * MS));
        assert_eq!(c.deliver_at(10 * S + 10 * MS, 10 * S + 12 * MS + MS / 2), Some(10 * S + 13 * MS));
        // 3 ms slower than the fastest: the time is past, delivered now
        assert_eq!(c.deliver_at(10 * S + 20 * MS, 10 * S + 24 * MS), None);
    }

    #[test]
    fn a_faster_record_lowers_the_offset_at_once() {
        let mut c = ClockMapper::new();
        c.observe(0, 5 * MS);
        c.observe(10 * MS, 12 * MS);
        assert_eq!(c.offset(), Some(2 * MS as i128));
    }

    #[test]
    fn the_window_slides_and_keeps_the_newest_minimum() {
        let mut c = ClockMapper::new();
        c.observe(0, MS); // diff 1 ms at 1 ms
        c.observe(S, S + 3 * MS); // 3 ms
        c.observe(20 * S, 20 * S + 2 * MS); // 2 ms
        assert_eq!(c.offset(), Some(MS as i128));
        // 31 s: the 1 ms sample is older than 30 s – the minimum of what is left (2 ms, the newer one) takes over
        c.observe(31 * S, 31 * S + 5 * MS);
        assert_eq!(c.offset(), Some(2 * MS as i128));
        // a later equal minimum replaces the older one, so it lasts longer
        c.observe(40 * S, 40 * S + 2 * MS);
        c.observe(51 * S, 51 * S + 9 * MS);
        assert_eq!(c.offset(), Some(2 * MS as i128));
        c.observe(71 * S, 71 * S + 9 * MS);
        assert_eq!(c.offset(), Some(9 * MS as i128));
    }

    #[test]
    fn the_clock_runs() {
        let a = now_ns();
        std::thread::sleep(std::time::Duration::from_millis(5));
        let b = now_ns();
        assert!(b > a && b - a >= 4 * MS && b - a < S);
        #[cfg(target_os = "macos")]
        assert!(ns_to_ticks(ticks_to_ns(1_000_000)).abs_diff(1_000_000) <= 1);
    }
}
