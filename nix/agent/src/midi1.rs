// SPDX-License-Identifier: AGPL-3.0-or-later
//! Universal MIDI Packets ↔ MIDI 1.0 byte messages, for ALSA sequencers without UMP (midi_alsa.rs, plan B161, C7).
//! Pure functions, tested on every platform.
//!
//! UMP → MIDI 1.0: system messages (type 0x1), MIDI 1.0 channel voice (0x2) and SysEx7 (0x3) as they are; MIDI 2.0
//! channel voice (0x4) translated as the UMP spec's default translation (M2-104-UM 1.1, appendix D.3): velocities and
//! values scaled down by dropping low bits, a program change with a bank as bank select MSB/LSB first, (N)RPN as the
//! controller sequence 101/100 (99/98), 6, 38; a note on whose velocity scales to 0 gets 1 (0 means note off in
//! MIDI 1.0). MIDI 2.0's per-note messages and relative (N)RPN have no MIDI 1.0 form and are dropped, as are the other
//! message types (utility, SysEx8, flex data, stream). All groups land on the one MIDI 1.0 port.
//!
//! MIDI 1.0 → UMP: a byte-stream parser (running status, real-time bytes inside SysEx) giving type 0x1, 0x2 and 0x3
//! messages on one group; SysEx split across several inputs is joined into one SysEx7 stream.

/// One piece of MIDI 1.0 data for the sequencer
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Midi1 {
    /// A whole channel or system message (1 to 3 bytes)
    Message(Vec<u8>),
    /// A piece of SysEx: the first starts with 0xF0, the last ends with 0xF7, those in between are bare data
    Sysex(Vec<u8>),
}

/// MIDI 1.0 messages from UMP words; `sysex` tracks, per group, whether a SysEx7 stream is open (start seen, end not)
pub fn ump_to_midi1(words: &[u32], sysex: &mut [bool; 16]) -> Vec<Midi1> {
    let mut out = vec![];
    let mut i = 0;
    while i < words.len() {
        let w0 = words[i];
        let n = crate::ump::message_words(w0);
        if i + n > words.len() {
            break;
        }
        let w1 = if n > 1 { words[i + 1] } else { 0 };
        i += n;
        let status = ((w0 >> 16) & 0xFF) as u8;
        let d1 = ((w0 >> 8) & 0x7F) as u8;
        let d2 = (w0 & 0x7F) as u8;
        match w0 >> 28 {
            0x1 => match status {
                0xF1 | 0xF3 => out.push(Midi1::Message(vec![status, d1])),
                0xF2 => out.push(Midi1::Message(vec![status, d1, d2])),
                0xF6 | 0xF8..=0xFF => out.push(Midi1::Message(vec![status])),
                _ => {}
            },
            0x2 => match status {
                0xC0..=0xDF => out.push(Midi1::Message(vec![status, d1])),
                0x80..=0xEF => out.push(Midi1::Message(vec![status, d1, d2])),
                _ => {}
            },
            0x3 => {
                let group = ((w0 >> 24) & 0xF) as usize;
                let count = (((w0 >> 16) & 0xF) as usize).min(6);
                let all = [(w0 >> 8) as u8, w0 as u8, (w1 >> 24) as u8, (w1 >> 16) as u8, (w1 >> 8) as u8, w1 as u8];
                let data = all[..count].iter().map(|b| b & 0x7F);
                match (w0 >> 20) & 0xF {
                    0 => {
                        sysex[group] = false;
                        out.push(Midi1::Sysex(std::iter::once(0xF0).chain(data).chain([0xF7]).collect()));
                    }
                    1 => {
                        sysex[group] = true;
                        out.push(Midi1::Sysex(std::iter::once(0xF0).chain(data).collect()));
                    }
                    2 if sysex[group] => out.push(Midi1::Sysex(data.collect())),
                    3 if sysex[group] => {
                        sysex[group] = false;
                        out.push(Midi1::Sysex(data.chain([0xF7]).collect()));
                    }
                    _ => {} // a continue or end without a start
                }
            }
            0x4 => out.extend(midi2_to_midi1(w0, w1).into_iter().map(Midi1::Message)),
            _ => {}
        }
    }
    out
}

/// One MIDI 2.0 channel voice message (type 0x4) as MIDI 1.0 messages – the UMP spec's default translation
pub fn midi2_to_midi1(w0: u32, w1: u32) -> Vec<Vec<u8>> {
    let channel = ((w0 >> 16) & 0xF) as u8;
    let index = ((w0 >> 8) & 0x7F) as u8;
    let low = (w0 & 0x7F) as u8;
    let value7 = (w1 >> 25) as u8;
    let cc = |c: u8, v: u8| vec![0xB0 | channel, c, v];
    match (w0 >> 20) & 0xF {
        0x8 => vec![vec![0x80 | channel, index, (w1 >> 25) as u8]],
        0x9 => vec![vec![0x90 | channel, index, ((w1 >> 25) as u8).max(1)]],
        0xA => vec![vec![0xA0 | channel, index, value7]],
        0xB => vec![cc(index, value7)],
        0xC => {
            let mut m = vec![];
            if w0 & 1 != 0 {
                m.push(cc(0, ((w1 >> 8) & 0x7F) as u8));
                m.push(cc(32, (w1 & 0x7F) as u8));
            }
            m.push(vec![0xC0 | channel, ((w1 >> 24) & 0x7F) as u8]);
            m
        }
        0xD => vec![vec![0xD0 | channel, value7]],
        0xE => {
            let v = w1 >> 18;
            vec![vec![0xE0 | channel, (v & 0x7F) as u8, ((v >> 7) & 0x7F) as u8]]
        }
        // registered (0x2) and assignable (0x3) controllers: bank in the index byte, index in the low byte
        s @ (0x2 | 0x3) => {
            let (msb, lsb) = if s == 0x2 { (101, 100) } else { (99, 98) };
            vec![cc(msb, index), cc(lsb, low), cc(6, value7), cc(38, ((w1 >> 18) & 0x7F) as u8)]
        }
        _ => vec![], // per-note controllers, relative (N)RPN, per-note pitch bend, per-note management
    }
}

/// MIDI 1.0 bytes → UMP words (types 0x1, 0x2, 0x3) on one group; keeps running status and an open SysEx between
/// calls
#[derive(Debug, Default)]
pub struct Midi1Parser {
    group: u8,
    running: u8,
    data: Vec<u8>,
    /// SysEx open: the bytes not yet sent (at most 6 kept back, so the end can be marked) and whether a start went out
    sysex: Option<(Vec<u8>, bool)>,
}

impl Midi1Parser {
    pub fn new(group: u8) -> Midi1Parser {
        Midi1Parser { group: group & 0xF, ..Default::default() }
    }

    fn sysex_packet(&self, status: u32, bytes: &[u8]) -> [u32; 2] {
        let mut b = [0u8; 6];
        b[..bytes.len()].copy_from_slice(bytes);
        let w0 = 0x3000_0000
            | (self.group as u32) << 24
            | status << 20
            | (bytes.len() as u32) << 16
            | (b[0] as u32) << 8
            | b[1] as u32;
        let w1 = (b[2] as u32) << 24 | (b[3] as u32) << 16 | (b[4] as u32) << 8 | b[5] as u32;
        [w0, w1]
    }

    /// Ends an open SysEx (an F7, or another status byte that cuts it short)
    fn end_sysex(&mut self, out: &mut Vec<u32>) {
        if let Some((rest, started)) = self.sysex.take() {
            out.extend(self.sysex_packet(if started { 3 } else { 0 }, &rest));
        }
    }

    pub fn feed(&mut self, bytes: &[u8]) -> Vec<u32> {
        let mut out = vec![];
        for &b in bytes {
            match b {
                0xF8..=0xFF => {
                    // real time: anywhere, even inside SysEx; 0xF9 and 0xFD are undefined
                    if b != 0xF9 && b != 0xFD {
                        out.push(self.system(b, 0, 0));
                    }
                }
                0xF0 => {
                    self.end_sysex(&mut out);
                    self.running = 0;
                    self.sysex = Some((vec![], false));
                }
                0xF7 => {
                    self.end_sysex(&mut out);
                }
                0x80..=0xF6 => {
                    self.end_sysex(&mut out);
                    self.data.clear();
                    if b >= 0xF0 {
                        self.running = 0;
                        match b {
                            0xF6 => out.push(self.system(b, 0, 0)),
                            0xF1..=0xF3 => self.running = b, // waits for its data; not kept as running status
                            _ => {}                          // 0xF4, 0xF5: undefined
                        }
                    } else {
                        self.running = b;
                    }
                }
                _ => {
                    if let Some((rest, started)) = self.sysex.as_mut() {
                        rest.push(b);
                        if rest.len() > 6 {
                            let chunk: Vec<u8> = rest.drain(..6).collect();
                            let status = if *started { 2 } else { 1 };
                            *started = true;
                            out.extend(self.sysex_packet(status, &chunk));
                        }
                        continue;
                    }
                    if self.running == 0 {
                        continue; // data without a status
                    }
                    self.data.push(b);
                    let s = self.running;
                    let need = match s {
                        0xC0..=0xDF | 0xF1 | 0xF3 => 1,
                        _ => 2,
                    };
                    if self.data.len() == need {
                        let d1 = self.data[0];
                        let d2 = if need == 2 { self.data[1] } else { 0 };
                        self.data.clear();
                        if s >= 0xF0 {
                            out.push(self.system(s, d1, d2));
                            self.running = 0;
                        } else {
                            out.push(
                                0x2000_0000
                                    | (self.group as u32) << 24
                                    | (s as u32) << 16
                                    | (d1 as u32) << 8
                                    | d2 as u32,
                            );
                        }
                    }
                }
            }
        }
        out
    }

    fn system(&self, status: u8, d1: u8, d2: u8) -> u32 {
        0x1000_0000 | (self.group as u32) << 24 | (status as u32) << 16 | (d1 as u32) << 8 | d2 as u32
    }
}

/// The record protocol of UMP words that go to the Mac: 2 if they hold a MIDI 2.0 channel voice message, 1 if they
/// hold a MIDI 1.0 one, else the endpoint's
pub fn protocol_of(words: &[u32], endpoint: u8) -> u8 {
    let mut i = 0;
    while i < words.len() {
        match words[i] >> 28 {
            0x4 => return 2,
            0x2 => return 1,
            _ => {}
        }
        i += crate::ump::message_words(words[i]);
    }
    endpoint
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg(b: &[u8]) -> Midi1 {
        Midi1::Message(b.to_vec())
    }

    #[test]
    fn midi1_channel_and_system_messages_pass() {
        let mut s = [false; 16];
        let words = [0x2090_3C64, 0x20C3_0500, 0x10F8_0000, 0x10F2_1020, 0x10F3_0700, 0x20E1_0040];
        assert_eq!(
            ump_to_midi1(&words, &mut s),
            vec![
                msg(&[0x90, 0x3C, 0x64]),
                msg(&[0xC3, 0x05]),
                msg(&[0xF8]),
                msg(&[0xF2, 0x10, 0x20]),
                msg(&[0xF3, 0x07]),
                msg(&[0xE1, 0x00, 0x40])
            ]
        );
    }

    #[test]
    fn sysex7_packets_become_sysex_pieces() {
        let mut s = [false; 16];
        // complete: F0 7E 7F 06 01 F7 (identity request)
        assert_eq!(
            ump_to_midi1(&[0x3004_7E7F, 0x0601_0000], &mut s),
            vec![Midi1::Sysex(vec![0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7])]
        );
        // start, continue, end
        let words = [0x3016_0102, 0x0304_0506, 0x3022_0708, 0, 0x3031_0900, 0];
        assert_eq!(
            ump_to_midi1(&words, &mut s),
            vec![Midi1::Sysex(vec![0xF0, 1, 2, 3, 4, 5, 6]), Midi1::Sysex(vec![7, 8]), Midi1::Sysex(vec![9, 0xF7]),]
        );
        // a continue without a start is dropped
        assert_eq!(ump_to_midi1(&[0x3022_0708, 0], &mut s), Vec::<Midi1>::new());
    }

    #[test]
    fn midi2_is_translated_by_the_default_translation() {
        // note on, velocity 0xFFFF → 127; velocity 0x0100 → 0 → 1
        assert_eq!(midi2_to_midi1(0x4091_3C00, 0xFFFF_0000), vec![vec![0x91, 0x3C, 0x7F]]);
        assert_eq!(midi2_to_midi1(0x4090_3C00, 0x0100_0000), vec![vec![0x90, 0x3C, 0x01]]);
        // note off, velocity 0x8000 → 64
        assert_eq!(midi2_to_midi1(0x4080_3C00, 0x8000_0000), vec![vec![0x80, 0x3C, 0x40]]);
        // poly pressure, CC 7, channel pressure: top 7 bits of 32
        assert_eq!(midi2_to_midi1(0x40A2_4000, 0x8000_0000), vec![vec![0xA2, 0x40, 0x40]]);
        assert_eq!(midi2_to_midi1(0x40B0_0700, 0xFFFF_FFFF), vec![vec![0xB0, 0x07, 0x7F]]);
        assert_eq!(midi2_to_midi1(0x40D5_0000, 0x0200_0000), vec![vec![0xD5, 0x01]]);
        // pitch bend centre 0x8000_0000 → 0x2000 (LSB 0, MSB 0x40)
        assert_eq!(midi2_to_midi1(0x40E0_0000, 0x8000_0000), vec![vec![0xE0, 0x00, 0x40]]);
        // program 5 with bank 1/2, then without a bank
        assert_eq!(midi2_to_midi1(0x40C0_0001, 0x0500_0102), vec![vec![0xB0, 0, 1], vec![0xB0, 32, 2], vec![0xC0, 5]]);
        assert_eq!(midi2_to_midi1(0x40C0_0000, 0x0500_0102), vec![vec![0xC0, 5]]);
        // RPN 0/0 (pitch bend range) with data 0x0400_0000 → MSB 2, LSB 0; NRPN 1/2
        assert_eq!(
            midi2_to_midi1(0x4020_0000, 0x0400_0000),
            vec![vec![0xB0, 101, 0], vec![0xB0, 100, 0], vec![0xB0, 6, 2], vec![0xB0, 38, 0]]
        );
        assert_eq!(
            midi2_to_midi1(0x4030_0102, 0xFFFF_FFFF),
            vec![vec![0xB0, 99, 1], vec![0xB0, 98, 2], vec![0xB0, 6, 0x7F], vec![0xB0, 38, 0x7F]]
        );
        // per-note pitch bend: no MIDI 1.0 form
        assert_eq!(midi2_to_midi1(0x4060_3C00, 0x8000_0000), Vec::<Vec<u8>>::new());
        // through ump_to_midi1, mixed with a MIDI 1.0 message; SysEx8 (type 0x5) dropped
        let mut s = [false; 16];
        assert_eq!(
            ump_to_midi1(&[0x4090_3C00, 0xFFFF_0000, 0x5000_0000, 0, 0, 0, 0x2080_3C00], &mut s),
            vec![msg(&[0x90, 0x3C, 0x7F]), msg(&[0x80, 0x3C, 0x00])]
        );
    }

    #[test]
    fn bytes_become_ump() {
        let mut p = Midi1Parser::new(0);
        // note on, running status, program change, clock in between, song position
        assert_eq!(
            p.feed(&[0x90, 0x3C, 0x64, 0x3E, 0x00, 0xC1, 0x05, 0xF8, 0xF2, 0x10, 0x20]),
            vec![0x2090_3C64, 0x2090_3E00, 0x20C1_0500, 0x10F8_0000, 0x10F2_1020]
        );
        // a message split across calls
        assert_eq!(p.feed(&[0xE0, 0x00]), Vec::<u32>::new());
        assert_eq!(p.feed(&[0x40]), vec![0x20E0_0040]);
        // group 3
        assert_eq!(Midi1Parser::new(3).feed(&[0xB0, 7, 100]), vec![0x23B0_0764]);
    }

    #[test]
    fn sysex_bytes_become_sysex7_and_back() {
        let mut p = Midi1Parser::new(0);
        assert_eq!(p.feed(&[0xF0, 0x7E, 0x7F, 0x06, 0x01, 0xF7]), vec![0x3004_7E7F, 0x0601_0000]);
        // nine bytes in two pieces, a clock inside: start (6), end (3); the clock goes out at once
        let mut words = p.feed(&[0xF0, 1, 2, 3, 4, 0xF8, 5]);
        assert_eq!(words, vec![0x10F8_0000]);
        words = p.feed(&[6, 7, 8, 9, 0xF7]);
        assert_eq!(words, vec![0x3016_0102, 0x0304_0506, 0x3033_0708, 0x0900_0000]);
        let mut s = [false; 16];
        assert_eq!(
            ump_to_midi1(&words, &mut s),
            vec![Midi1::Sysex(vec![0xF0, 1, 2, 3, 4, 5, 6]), Midi1::Sysex(vec![7, 8, 9, 0xF7])]
        );
        // exactly six bytes: complete; twelve: start + end with six
        assert_eq!(p.feed(&[0xF0, 1, 2, 3, 4, 5, 6, 0xF7]), vec![0x3006_0102, 0x0304_0506]);
        assert_eq!(
            p.feed(&[0xF0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 0xF7]),
            vec![0x3016_0102, 0x0304_0506, 0x3036_0708, 0x090A_0B0C]
        );
        // a status byte cuts SysEx short: it ends, then the note follows
        assert_eq!(p.feed(&[0xF0, 1, 0x90, 0x3C, 0x40]), vec![0x3001_0100, 0, 0x2090_3C40]);
    }

    #[test]
    fn protocol_by_content() {
        assert_eq!(protocol_of(&[0x4090_3C00, 0xFFFF_0000], 1), 2);
        assert_eq!(protocol_of(&[0x2090_3C64], 2), 1);
        assert_eq!(protocol_of(&[0x10F8_0000], 2), 2);
    }
}
