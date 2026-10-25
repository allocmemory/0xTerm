//! Shell integration: OSC sequences alacritty ignores but 0xTerm wants.
//! The scanner sits in front of the parser. It holds each OSC back until the
//! OSC ends, then passes it on whole; one too long to be real never reaches
//! the parser at all, since vte would buffer it (without limit, in its `std`
//! build) and then act on it.
//!
//! - OSC 7 `file://host/path`: the shell's directory (zsh/fish hooks emit it).
//! - OSC 1337 `CurrentDir=/path`: an older spelling of the same thing that
//!   some shell integration scripts still emit.

use std::path::PathBuf;

const MAX_OSC: usize = 4096;

/// The longest OSC passed on: a 1 MiB clipboard write (OSC 52) is about
/// 1.4 MB of base64.
const MAX_SEQUENCE: usize = 2 * 1024 * 1024;

#[derive(Debug, PartialEq)]
pub enum OscEvent {
    Cwd(PathBuf),
    /// A clipboard write (OSC 52) too long to pass on: about this many bytes.
    CopyBlocked(usize),
}

/// What [`OscScanner::scan`] found in a chunk.
pub struct Scan {
    pub events: Vec<OscEvent>,
    /// The bytes to parse instead of the chunk, when they differ from it.
    pub filtered: Option<Vec<u8>>,
}

/// Where the parser is, as far as starting an OSC goes. These follow vte's
/// own transitions (`advance_esc`, `advance_osc_string`, `anywhere`).
#[derive(Default, Clone, Copy, PartialEq)]
enum State {
    /// Anywhere ESC starts an escape: text, CSI, DCS and SOS/PM/APC strings.
    #[default]
    Ground,
    /// After ESC. vte stays here through C0 bytes, DEL, bytes of 0x80 and
    /// up, and more ESCs, so any of those may come before `]`.
    Escape,
    /// In an OSC, held until it ends.
    Osc,
    /// In an OSC too long to pass on: dropped up to its end.
    Dropping,
}

#[derive(Default)]
pub struct OscScanner {
    state: State,
    /// What's held back from the parser: an ESC, or an OSC so far.
    held: Vec<u8>,
    /// The start of the OSC, for our own OSCs (7, 1337).
    body: Vec<u8>,
    /// Bytes in the current OSC so far, dropped ones included.
    len: usize,
    /// The last OSC was dropped and ended with ESC: a `\` after it is its ST.
    after_drop: bool,
    /// In `Escape`: the ESC already went to the parser (it ended an OSC), so
    /// it isn't held.
    escape_passed: bool,
    /// In `Osc`: the parser has the ESC (not `]`), so dropping the OSC must
    /// also take the parser out of its escape.
    osc_needs_can: bool,
}

/// The bytes for the parser. Stays a view of the chunk while every byte goes
/// through in place, which is nearly always.
struct Out<'a> {
    input: &'a [u8],
    /// `input[..same]` went through unchanged.
    same: usize,
    copy: Option<Vec<u8>>,
}

impl Out<'_> {
    fn pass(&mut self, i: usize) {
        match &mut self.copy {
            None if self.same == i => self.same += 1,
            _ => {
                let byte = self.input[i];
                self.copy().push(byte);
            }
        }
    }

    fn copy(&mut self) -> &mut Vec<u8> {
        let (input, same) = (self.input, self.same);
        self.copy.get_or_insert_with(|| input[..same].to_vec())
    }

    fn finish(self) -> Option<Vec<u8>> {
        match self.copy {
            None if self.same == self.input.len() => None,
            None => Some(self.input[..self.same].to_vec()),
            copy => copy,
        }
    }
}

impl OscScanner {
    /// Scans a chunk. A sequence split across chunks is completed by the next call.
    pub fn scan(&mut self, bytes: &[u8]) -> Scan {
        let mut events = Vec::new();
        let mut out = Out { input: bytes, same: 0, copy: None };
        // Where `held` starts in this chunk, while it is exactly `bytes[at..i]`.
        let mut held_at: Option<usize> = None;
        for (i, &byte) in bytes.iter().enumerate() {
            match (self.state, byte) {
                (State::Ground, 0x1b) => self.hold_escape(i, &mut held_at),
                (State::Ground, _) => out.pass(i),

                (State::Escape, b']') => {
                    self.after_drop = false;
                    if self.escape_passed {
                        self.held.clear();
                        held_at = Some(i);
                    }
                    self.osc_needs_can = self.escape_passed;
                    self.held.push(byte);
                    self.body.clear();
                    self.len = 0;
                    self.state = State::Osc;
                }
                (State::Escape, b'\\') if self.after_drop && self.held == [0x1b] => {
                    // The ST of a dropped OSC goes with it.
                    self.held.clear();
                    held_at = None;
                    self.after_drop = false;
                    self.state = State::Ground;
                }
                // vte stays in its escape for these: C0 bytes run, the rest
                // are ignored. They're held with the ESC, in order, since a
                // control byte means something else in the state before it.
                (State::Escape, 0x00..=0x17 | 0x19 | 0x1b..=0x1f | 0x7f | 0x80..=0xff) => {
                    if self.escape_passed {
                        out.pass(i);
                    } else {
                        self.held.push(byte);
                        if self.held.len() > MAX_OSC {
                            // Too many to hold; the parser has the escape from here.
                            self.release(&mut out, held_at.take());
                            self.escape_passed = true;
                        }
                    }
                }
                // Anything else ends the escape (CAN and SUB too, which vte runs).
                (State::Escape, _) => {
                    self.after_drop = false;
                    self.release(&mut out, held_at.take());
                    out.pass(i);
                    self.state = State::Ground;
                }

                // BEL, CAN and SUB end an OSC; vte acts on it for each.
                (State::Osc, 0x07 | 0x18 | 0x1a) => {
                    self.held.push(byte);
                    self.release(&mut out, held_at.take());
                    events.extend(self.dispatch());
                    self.state = State::Ground;
                }
                // So does ESC, which the parser gets now: it ends the OSC
                // there and starts the next escape (`\` for ST).
                (State::Osc, 0x1b) => {
                    self.release(&mut out, held_at.take());
                    out.pass(i);
                    events.extend(self.dispatch());
                    (self.escape_passed, self.state) = (true, State::Escape);
                }
                (State::Osc, _) => {
                    self.held.push(byte);
                    self.len += 1;
                    if self.body.len() < MAX_OSC {
                        self.body.push(byte);
                    }
                    if self.held.len() > MAX_SEQUENCE {
                        self.held = Vec::new();
                        held_at = None;
                        if self.osc_needs_can {
                            // vte leaves its escape on CAN without dispatching anything.
                            out.copy().push(0x18);
                        }
                        self.state = State::Dropping;
                    }
                }

                (State::Dropping, 0x07 | 0x18 | 0x1a | 0x1b) => {
                    if self.body.starts_with(b"52;") {
                        // Base64: four bytes of it are three of text.
                        events.push(OscEvent::CopyBlocked(self.len / 4 * 3));
                    }
                    self.state = State::Ground;
                    if byte == 0x1b {
                        self.hold_escape(i, &mut held_at);
                        self.after_drop = true;
                    }
                }
                (State::Dropping, _) => self.len += 1,
            }
        }
        Scan { events, filtered: out.finish() }
    }

    fn hold_escape(&mut self, i: usize, held_at: &mut Option<usize>) {
        self.escape_passed = false;
        self.held.clear();
        self.held.push(0x1b);
        *held_at = Some(i);
        self.state = State::Escape;
    }

    /// Passes on what's held, in place when it is still the chunk's own bytes.
    fn release(&mut self, out: &mut Out, held_at: Option<usize>) {
        if self.held.is_empty() {
            return;
        }
        if out.copy.is_none() && held_at == Some(out.same) {
            out.same += self.held.len();
        } else {
            out.copy().extend_from_slice(&self.held);
        }
        if self.held.capacity() > 64 * 1024 {
            self.held = Vec::new();
        } else {
            self.held.clear();
        }
    }

    fn dispatch(&mut self) -> Option<OscEvent> {
        let body = std::str::from_utf8(&self.body).ok()?;
        let (code, payload) = body.split_once(';')?;
        match code {
            "7" => file_url_path(payload).map(OscEvent::Cwd),
            "1337" => payload.strip_prefix("CurrentDir=").map(|dir| OscEvent::Cwd(dir.into())),
            _ => None,
        }
    }
}

/// `file://host/Users/me/a%20b` → `/Users/me/a b`. The host is ignored.
fn file_url_path(url: &str) -> Option<PathBuf> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    let mut bytes = Vec::with_capacity(path.len());
    let mut iter = path.bytes();
    while let Some(b) = iter.next() {
        if b == b'%' {
            let hex = [iter.next()?, iter.next()?];
            bytes.push(u8::from_str_radix(std::str::from_utf8(&hex).ok()?, 16).ok()?);
        } else {
            bytes.push(b);
        }
    }
    use std::os::unix::ffi::OsStringExt;
    Some(PathBuf::from(std::ffi::OsString::from_vec(bytes)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn osc7_with_bel_and_st() {
        let mut s = OscScanner::default();
        assert_eq!(s.scan(b"x\x1b]7;file://mac/Users/me/a%20b\x07y").events, vec![OscEvent::Cwd("/Users/me/a b".into())]);
        assert_eq!(s.scan(b"\x1b]7;file:///tmp\x1b\\").events, vec![OscEvent::Cwd("/tmp".into())]);
    }

    #[test]
    fn split_across_chunks() {
        let mut s = OscScanner::default();
        assert!(s.scan(b"\x1b]7;file://h/us").events.is_empty());
        assert_eq!(s.scan(b"r/bin\x07").events, vec![OscEvent::Cwd("/usr/bin".into())]);
    }

    #[test]
    fn osc_1337_current_dir_and_noise() {
        let mut s = OscScanner::default();
        assert_eq!(s.scan(b"\x1b]0;title\x07\x1b[31m\x1b]1337;CurrentDir=/var\x07").events, vec![OscEvent::Cwd("/var".into())]);
    }

    #[test]
    fn ordinary_output_is_not_copied() {
        let mut s = OscScanner::default();
        assert!(s.scan(b"ls\r\n\x1b]0;title\x07\x1b[1mbold").filtered.is_none());
    }

    #[test]
    fn oversized_osc_never_reaches_the_parser() {
        let mut s = OscScanner::default();
        let body = vec![b'x'; MAX_SEQUENCE];
        // The OSC is held from its ESC on; the text before it goes through.
        assert_eq!(s.scan(b"a\x1b]0;").filtered.unwrap(), b"a");
        assert_eq!(s.scan(&body).filtered.unwrap(), b"");
        assert_eq!(s.scan(&body).filtered.unwrap(), b"");
        // Its ST is dropped with it; then output flows again.
        assert_eq!(s.scan(b"more\x1b\\after").filtered.unwrap(), b"after");
        assert!(s.scan(b"next").filtered.is_none());
    }

    #[test]
    fn a_held_osc_goes_through_whole_when_it_ends() {
        let mut s = OscScanner::default();
        assert_eq!(s.scan(b"ab\x1b]0;ti").filtered.unwrap(), b"ab");
        assert_eq!(s.scan(b"tle\x07cd").filtered.unwrap(), b"\x1b]0;title\x07cd");
        assert_eq!(s.scan(b"\x1b").filtered.unwrap(), b"");
        assert_eq!(s.scan(b"[1m").filtered.unwrap(), b"\x1b[1m");
    }

    #[test]
    fn st_terminated_sequences_are_not_copied() {
        let mut s = OscScanner::default();
        assert!(s.scan(b"a\x1b]0;title\x1b\\b\x1b]7;file:///tmp\x1b\\c").filtered.is_none());
    }

    #[test]
    fn an_escape_after_an_osc_that_is_then_dropped_leaves_the_parser_clean() {
        let mut s = OscScanner::default();
        // The first OSC ends at the ESC, which the parser gets; the second is
        // dropped, so the parser gets CAN to leave that escape.
        let mut cut = s.scan(b"\x1b]0;a\x1b\x1b]0;").filtered.unwrap_or_default();
        cut.extend(s.scan(&vec![b'x'; MAX_SEQUENCE]).filtered.unwrap_or_default());
        assert_eq!(cut, b"\x1b]0;a\x1b\x1b\x18");
        assert_eq!(s.scan(b"\x07after").filtered.unwrap(), b"after");
    }

    #[test]
    fn control_bytes_in_an_escape_stay_in_order() {
        let mut s = OscScanner::default();
        // vte runs the NUL and keeps the escape, so `]` still opens an OSC.
        let scan = s.scan(b"\x1b\x00]7;file:///tmp\x07");
        assert!(scan.filtered.is_none());
        assert_eq!(scan.events, vec![OscEvent::Cwd("/tmp".into())]);
        // A long run of them is passed on rather than held.
        let mut long = vec![0x1b];
        long.extend(vec![0u8; MAX_OSC + 1]);
        assert!(s.scan(&long).filtered.is_none());
    }

    #[test]
    fn a_new_sequence_after_a_dropped_one_reaches_the_parser() {
        let mut s = OscScanner::default();
        s.scan(b"\x1b]0;");
        s.scan(&vec![b'x'; MAX_SEQUENCE + 10]);
        let scan = s.scan(b"x\x1b]7;file:///tmp\x07ok");
        assert_eq!(scan.filtered.unwrap(), b"\x1b]7;file:///tmp\x07ok");
        assert_eq!(scan.events, vec![OscEvent::Cwd("/tmp".into())]);
    }

    #[test]
    fn a_dropped_clipboard_write_is_reported() {
        let mut s = OscScanner::default();
        s.scan(b"\x1b]52;c;");
        s.scan(&vec![b'A'; MAX_SEQUENCE + 4]);
        let events = s.scan(b"\x07").events;
        // About the text the base64 holds: three bytes for every four.
        assert!(matches!(events[..], [OscEvent::CopyBlocked(n)] if n.abs_diff(MAX_SEQUENCE / 4 * 3) < 16), "{events:?}");
    }
}
