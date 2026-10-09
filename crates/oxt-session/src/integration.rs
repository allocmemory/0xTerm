//! Shell integration: OSC sequences alacritty ignores but 0xTerm wants.
//! The scanner watches the byte stream next to the parser; it never changes it.
//!
//! - OSC 7 `file://host/path`: the shell's directory (zsh/fish hooks emit it).
//! - OSC 1337 `CurrentDir=/path`: an older spelling of the same thing that
//!   some shell integration scripts still emit.

use std::path::PathBuf;

const MAX_OSC: usize = 4096;

#[derive(Debug, PartialEq)]
pub enum OscEvent {
    Cwd(PathBuf),
}

#[derive(Default)]
pub struct OscScanner {
    state: State,
    body: Vec<u8>,
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
    pub fn scan(&mut self, bytes: &[u8]) -> Vec<OscEvent> {
        let mut events = Vec::new();
        for &byte in bytes {
            self.state = match (self.state, byte) {
                (State::Ground, 0x1b) => State::Escape,
                (State::Ground, _) => State::Ground,
                (State::Escape, b']') => {
                    self.body.clear();
                    State::Osc
                }
                (State::Escape, 0x1b) => State::Escape,
                (State::Escape, _) => State::Ground,
                // BEL or ST (ESC \) ends an OSC.
                (State::Osc, 0x07) => {
                    events.extend(self.dispatch());
                    State::Ground
                }
                (State::Osc, 0x1b) => State::OscEscape,
                (State::Osc, 0x18 | 0x1a) => State::Ground,
                (State::Osc, _) => {
                    if self.body.len() < MAX_OSC {
                        self.body.push(byte);
                    }
                    State::Osc
                }
                (State::OscEscape, b'\\') => {
                    events.extend(self.dispatch());
                    State::Ground
                }
                (State::OscEscape, b']') => {
                    self.body.clear();
                    State::Osc
                }
                (State::OscEscape, _) => State::Ground,
            };
        }
        events
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
        assert_eq!(s.scan(b"x\x1b]7;file://mac/Users/me/a%20b\x07y"), vec![OscEvent::Cwd("/Users/me/a b".into())]);
        assert_eq!(s.scan(b"\x1b]7;file:///tmp\x1b\\"), vec![OscEvent::Cwd("/tmp".into())]);
    }

    #[test]
    fn split_across_chunks() {
        let mut s = OscScanner::default();
        assert!(s.scan(b"\x1b]7;file://h/us").is_empty());
        assert_eq!(s.scan(b"r/bin\x07"), vec![OscEvent::Cwd("/usr/bin".into())]);
    }

    #[test]
    fn osc_1337_current_dir_and_noise() {
        let mut s = OscScanner::default();
        assert_eq!(s.scan(b"\x1b]0;title\x07\x1b[31m\x1b]1337;CurrentDir=/var\x07"), vec![OscEvent::Cwd("/var".into())]);
    }
}
