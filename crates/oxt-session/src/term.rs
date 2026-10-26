//! The terminal engine for one pane: alacritty's parser + grid, plus our OSC
//! scanner, all behind one lock.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Instant;

use alacritty_terminal::event::{Event, EventListener, WindowSize};
use alacritty_terminal::grid::Dimensions;
use alacritty_terminal::term::{Config, Term};
use alacritty_terminal::vte::ansi::{self, Rgb};

use crate::SessionEvent;
use crate::integration::{OscEvent, OscScanner};

/// Grid size in cells, as alacritty wants it.
#[derive(Clone, Copy, Debug)]
pub struct GridSize {
    pub rows: usize,
    pub cols: usize,
}

impl Dimensions for GridSize {
    fn total_lines(&self) -> usize {
        self.rows
    }
    fn screen_lines(&self) -> usize {
        self.rows
    }
    fn columns(&self) -> usize {
        self.cols
    }
}

/// Parser, grid and shell-integration state. Lives behind the session's `FairMutex`.
pub struct Engine {
    pub term: Term<Listener>,
    parser: ansi::Processor,
    osc: OscScanner,
    listener: Listener,
    sync_armed: bool,
}

impl Engine {
    pub fn new(size: GridSize, scrollback: usize, listener: Listener) -> Self {
        // Kitty keyboard protocol: programs that ask for it (Claude Code, Neovim, Helix)
        // can tell Shift+Enter from Enter, and Esc from the start of a sequence.
        let config = Config { scrolling_history: scrollback, kitty_keyboard: true, ..Config::default() };
        Engine {
            term: Term::new(config, &size, listener.clone()),
            parser: ansi::Processor::new(),
            osc: OscScanner::default(),
            listener,
            sync_armed: false,
        }
    }

    /// Feeds program output through the parser. Returns true if the screen changed.
    ///
    /// Bytes inside a synchronized update (`CSI ? 2026 h`) are held by the
    /// parser until the update ends, so a half-drawn frame is never shown.
    pub fn advance(&mut self, bytes: &[u8]) -> bool {
        let scan = self.osc.scan(bytes);
        for event in scan.events {
            match event {
                OscEvent::Cwd(dir) => self.listener.emit(SessionEvent::Cwd(dir)),
                OscEvent::CopyBlocked(bytes) => self.listener.emit(SessionEvent::CopyBlocked(bytes)),
            }
        }
        let bytes = scan.filtered.as_deref().unwrap_or(bytes);
        let held_before = self.parser.sync_bytes_count();
        self.parser.advance(&mut self.term, bytes);
        let deadline = self.parser.sync_timeout().sync_timeout();
        if let Some(deadline) = deadline
            && !self.sync_armed
        {
            self.listener.emit(SessionEvent::SyncPending(deadline));
        }
        self.sync_armed = deadline.is_some();
        self.parser.sync_bytes_count() < held_before + bytes.len()
    }

    /// Ends a synchronized update whose program never closed it.
    pub fn flush_sync(&mut self, now: Instant) -> bool {
        match self.parser.sync_timeout().sync_timeout() {
            Some(deadline) if deadline <= now => {
                self.parser.stop_sync(&mut self.term);
                self.sync_armed = false;
                true
            }
            _ => false,
        }
    }

    pub fn resize(&mut self, size: GridSize, window: WindowSize) {
        *self.listener.window.lock().unwrap() = window;
        self.term.resize(size);
    }
}

/// The colors the terminal answers with when a program asks (OSC 10/11/4),
/// and the renderer draws with.
#[derive(Clone, Debug, PartialEq)]
pub struct Palette {
    pub ansi: [Rgb; 16],
    pub foreground: Rgb,
    pub background: Rgb,
    pub cursor: Rgb,
}

impl Palette {
    /// Resolves an xterm color index: 0–15 theme, 16–231 color cube,
    /// 232–255 grays, 256 foreground, 257 background, 258 cursor.
    pub fn rgb(&self, index: usize) -> Rgb {
        match index {
            0..=15 => self.ansi[index],
            16..=231 => {
                let i = index - 16;
                let level = |v: usize| if v == 0 { 0 } else { (55 + v * 40) as u8 };
                Rgb { r: level(i / 36), g: level(i / 6 % 6), b: level(i % 6) }
            }
            232..=255 => {
                let v = (8 + (index - 232) * 10) as u8;
                Rgb { r: v, g: v, b: v }
            }
            257 => self.background,
            258 => self.cursor,
            _ => self.foreground,
        }
    }
}

impl Default for Palette {
    /// The default dark theme.
    fn default() -> Self {
        let rgb = |hex: u32| Rgb { r: (hex >> 16) as u8, g: (hex >> 8) as u8, b: hex as u8 };
        Palette {
            ansi: [
                0x1b1b1b, 0xff6b6b, 0x7ee787, 0xf2cc60, 0x79c0ff, 0xd2a8ff, 0x76e3ea, 0xd0d0d0, 0x5a5a5a, 0xff8b8b, 0x9ef0a6, 0xf8dc8a, 0xa5d6ff,
                0xe2c5ff, 0xb3f0ff, 0xffffff,
            ]
            .map(rgb),
            foreground: rgb(0xe6e6e6),
            background: rgb(0x0b0b0b),
            cursor: rgb(0xe6e6e6),
        }
    }
}

/// The most a program can put on the clipboard (OSC 52) at once.
pub const MAX_COPY: usize = 1024 * 1024;

/// Writes bytes back to the program (answers to its queries).
type Reply = Arc<dyn Fn(&[u8]) + Send + Sync>;

/// Where the engine's side effects go: replies to the pty, events to the UI.
#[derive(Clone)]
pub struct Listener {
    reply: Reply,
    events: async_channel::Sender<SessionEvent>,
    pub(crate) palette: Arc<Mutex<Palette>>,
    window: Arc<Mutex<WindowSize>>,
    pub(crate) cwd: Arc<Mutex<Option<PathBuf>>>,
}

impl Listener {
    pub fn new(reply: impl Fn(&[u8]) + Send + Sync + 'static, events: async_channel::Sender<SessionEvent>) -> Self {
        Listener {
            reply: Arc::new(reply),
            events,
            palette: Default::default(),
            window: Arc::new(Mutex::new(WindowSize { num_lines: 0, num_cols: 0, cell_width: 0, cell_height: 0 })),
            cwd: Default::default(),
        }
    }

    pub(crate) fn emit(&self, event: SessionEvent) {
        if let SessionEvent::Cwd(dir) = &event {
            *self.cwd.lock().unwrap() = Some(dir.clone());
        }
        // Fails only when the UI dropped the receiver; nobody is listening then.
        let _ = self.events.try_send(event);
    }
}

impl EventListener for Listener {
    fn send_event(&self, event: Event) {
        match event {
            // Answers to queries like "what are you?" (DA) or "where's the cursor?" (DSR).
            Event::PtyWrite(text) => (self.reply)(text.as_bytes()),
            Event::ColorRequest(index, format) => {
                let rgb = self.palette.lock().unwrap().rgb(index);
                (self.reply)(format(rgb).as_bytes());
            }
            Event::TextAreaSizeRequest(format) => {
                let window = *self.window.lock().unwrap();
                (self.reply)(format(window).as_bytes());
            }
            Event::Title(title) => self.emit(SessionEvent::Title(Some(title))),
            Event::ResetTitle => self.emit(SessionEvent::Title(None)),
            Event::Bell => self.emit(SessionEvent::Bell),
            // A program may copy (OSC 52), up to a size anyone means to copy.
            Event::ClipboardStore(_, text) if text.len() <= MAX_COPY => self.emit(SessionEvent::Copy(text)),
            Event::ClipboardStore(_, text) => self.emit(SessionEvent::CopyBlocked(text.len())),
            // OSC 52 reads stay off: a program must not read the clipboard behind your back.
            Event::ClipboardLoad(..) => {}
            Event::Wakeup | Event::MouseCursorDirty | Event::CursorBlinkingChange => {}
            Event::Exit | Event::ChildExit(_) => {}
        }
    }
}
