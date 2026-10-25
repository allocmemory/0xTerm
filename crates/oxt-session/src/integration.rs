//! Shell integration: OSC sequences alacritty ignores but 0xTerm wants.
//! The scanner watches the byte stream next to the parser. The one thing it
//! changes is an OSC too long to be real: alacritty's parser would buffer it
//! all, so the scanner cuts it off.
//!
//! - OSC 7 `file://host/path`: the shell's directory (zsh/fish hooks emit it).
//! - OSC 1337 `CurrentDir=/path`: an older spelling of the same thing that
//!   some shell integration scripts still emit.

use std::path::PathBuf;

const MAX_OSC: usize = 4096;

/// The longest OSC the parser is given: a 1 MiB clipboard write (OSC 52) is
/// about 1.4 MB of base64. Past this, the parser would hold every byte (vte
/// only bounds the buffer without `std`) until the sequence ends, if ever.
const MAX_SEQUENCE: usize = 2 * 1024 * 1024;

/// CAN: ends any escape sequence the parser is in the middle of.
const CAN: u8 = 0x18;

#[derive(Debug, PartialEq)]
pub enum OscEvent {
    Cwd(PathBuf),
    /// A clipboard write (OSC 52) so long it was cut off: about this many bytes.
    CopyBlocked(usize),
}

/// What [`OscScanner::scan`] found in a chunk.
pub struct Scan {
    pub events: Vec<OscEvent>,
    /// The bytes to parse instead of the chunk, when an OSC was cut off.
    pub filtered: Option<Vec<u8>>,
}

#[derive(Default)]
pub struct OscScanner {
    state: State,
    body: Vec<u8>,
    /// Bytes in the current OSC so far.
    len: usize,
    /// The current OSC passed `MAX_SEQUENCE`: the parser was sent CAN, and the
    /// rest of the sequence is dropped.
    dropping: bool,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum State {
    #[default]
    Ground,
    Escape,
    Osc,
    OscEscape,
}

impl OscScanner {
    /// Scans a chunk. A sequence split across chunks is completed by the next call.
    pub fn scan(&mut self, bytes: &[u8]) -> Scan {
        let mut events = Vec::new();
        let mut filtered: Option<Vec<u8>> = self.dropping.then(Vec::new);
        for (i, &byte) in bytes.iter().enumerate() {
            let before = self.state;
            // The current OSC ended (BEL, ST), or was cancelled (CAN, SUB).
            let (mut ended, mut cancelled) = (false, false);
            self.state = match (self.state, byte) {
                (State::Ground, 0x1b) => State::Escape,
                (State::Ground, _) => State::Ground,
                (State::Escape, b']') => State::Osc,
                (State::Escape, 0x1b) => State::Escape,
                (State::Escape, _) => State::Ground,
                // BEL or ST (ESC \) ends an OSC.
                (State::Osc, 0x07) => {
                    ended = true;
                    State::Ground
                }
                (State::Osc, 0x1b) => State::OscEscape,
                (State::Osc, 0x18 | 0x1a) => {
                    (ended, cancelled) = (true, true);
                    State::Ground
                }
                (State::Osc, _) => {
                    if self.body.len() < MAX_OSC {
                        self.body.push(byte);
                    }
                    State::Osc
                }
                (State::OscEscape, b'\\') => {
                    ended = true;
                    State::Ground
                }
                (State::OscEscape, b']') => State::Osc,
                (State::OscEscape, _) => State::Ground,
            };
            let started = self.state == State::Osc && matches!(before, State::Escape | State::OscEscape);
            if started {
                self.body.clear();
                self.len = 0;
            }
            if ended && !cancelled && !self.dropping {
                events.extend(self.dispatch());
            } else if ended && self.dropping && self.body.starts_with(b"52;") {
                // Base64: four bytes of it are three of text.
                events.push(OscEvent::CopyBlocked(self.len / 4 * 3));
            }
            let inside = matches!(self.state, State::Osc | State::OscEscape);
            if inside {
                self.len += 1;
            }

            if self.dropping {
                if inside && !started {
                    continue;
                }
                self.dropping = false;
                // Left by ESC and something else: that escape is the parser's.
                if !ended {
                    filtered.get_or_insert_with(Vec::new).extend([0x1b, byte]);
                }
                continue;
            }
            if inside && self.len > MAX_SEQUENCE {
                self.dropping = true;
                filtered.get_or_insert_with(|| bytes[..i].to_vec()).push(CAN);
                continue;
            }
            if let Some(out) = &mut filtered {
                out.push(byte);
            }
        }
        Scan { events, filtered }
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
    fn oversized_osc_is_cut_off_until_it_ends() {
        let mut s = OscScanner::default();
        let body = vec![b'x'; MAX_SEQUENCE];
        assert!(s.scan(b"a\x1b]0;").filtered.is_none());
        let cut = s.scan(&body).filtered.unwrap();
        // Up to the limit (`]0;` counts), then CAN for the parser; the rest is dropped.
        assert_eq!(cut.len(), MAX_SEQUENCE - 3 + 1);
        assert_eq!(cut.last(), Some(&CAN));
        assert_eq!(s.scan(&body).filtered.unwrap(), b"");
        // The terminator is dropped too, then output flows again.
        assert_eq!(s.scan(b"more\x1b\\after").filtered.unwrap(), b"after");
        assert!(s.scan(b"next").filtered.is_none());
    }

    #[test]
    fn a_new_sequence_after_a_cut_one_reaches_the_parser() {
        let mut s = OscScanner::default();
        s.scan(b"\x1b]0;");
        s.scan(&vec![b'x'; MAX_SEQUENCE + 10]);
        let scan = s.scan(b"x\x1b]7;file:///tmp\x07ok");
        assert_eq!(scan.filtered.unwrap(), b"\x1b]7;file:///tmp\x07ok");
        assert_eq!(scan.events, vec![OscEvent::Cwd("/tmp".into())]);
    }
}
