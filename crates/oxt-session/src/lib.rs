//! One pane's whole state: a shell task plus the terminal engine, without any UI.
//!
//! Threads: the task's reader thread parses output under the engine lock and
//! sends one `Wakeup` per batch; the UI locks the engine once per frame to draw.

pub mod input;
pub mod integration;
mod term;

use std::io;
use std::ops::DerefMut;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Instant;

pub use alacritty_terminal;
use alacritty_terminal::event::WindowSize;
use alacritty_terminal::grid::Scroll;
use alacritty_terminal::sync::FairMutex;
use alacritty_terminal::term::TermMode;
use oxt_task::{Command, Process, Size, Task};

pub use input::{Key, Mods, MouseAction, MouseButton};
pub use term::{Engine, GridSize, Listener, MAX_COPY, Palette};

/// What a session tells the UI. Arrives on the receiver from [`Session::spawn`].
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// The screen changed; draw a frame. Coalesced: at most one is pending.
    Wakeup,
    /// The program set the title (OSC 0/2), or reset it (`None`).
    Title(Option<String>),
    Bell,
    /// The program asked to copy text (OSC 52).
    Copy(String),
    /// The program asked to copy this many bytes, over the limit; nothing was copied.
    CopyBlocked(usize),
    /// The shell reported its directory (OSC 7).
    Cwd(PathBuf),
    /// A synchronized update started; call [`Session::flush_sync`] at this time
    /// in case the program never ends it.
    SyncPending(Instant),
    Exited(ExitStatus),
}

pub struct SessionOptions {
    pub command: Command,
    pub size: Size,
    pub scrollback: usize,
    pub option_as_meta: bool,
}

impl Default for SessionOptions {
    fn default() -> Self {
        SessionOptions { command: Command::login_shell(), size: Size::new(24, 80), scrollback: 10_000, option_as_meta: true }
    }
}

pub struct Session {
    engine: Arc<FairMutex<Engine>>,
    task: Task,
    listener: Listener,
    /// True while a `Wakeup` is sent and not yet handled.
    pending: Arc<AtomicBool>,
    option_as_meta: bool,
}

impl Session {
    pub fn spawn(options: SessionOptions) -> io::Result<(Session, async_channel::Receiver<SessionEvent>)> {
        let (tx, rx) = async_channel::unbounded();

        // The engine needs the task's writer (to answer queries) and the task's
        // reader needs the engine, so the writer is wired in after spawning.
        let writer: Arc<std::sync::OnceLock<oxt_task::Writer>> = Default::default();
        let reply = {
            let writer = writer.clone();
            move |bytes: &[u8]| {
                if let Some(writer) = writer.get() {
                    writer.write(bytes);
                }
            }
        };
        let listener = Listener::new(reply, tx.clone());
        let size = options.size;
        let mut engine = Engine::new(grid_size(size), options.scrollback, listener.clone());
        engine.resize(grid_size(size), window_size(size));
        let engine = Arc::new(FairMutex::new(engine));

        let pending = Arc::new(AtomicBool::new(false));
        let on_output = {
            let (engine, tx, pending) = (engine.clone(), tx.clone(), pending.clone());
            move |bytes: &[u8]| {
                let changed = engine.lock().advance(bytes);
                if changed && !pending.swap(true, Ordering::AcqRel) {
                    let _ = tx.try_send(SessionEvent::Wakeup);
                }
            }
        };
        let on_exit = move |status| {
            let _ = tx.try_send(SessionEvent::Exited(status));
        };

        let task = Task::spawn(&options.command, size, on_output, on_exit)?;
        let _ = writer.set(task.writer());
        engine.lock().term.is_focused = true;
        Ok((Session { engine, task, listener, pending, option_as_meta: options.option_as_meta }, rx))
    }

    /// Call when handling a `Wakeup`, before drawing, so the next change sends another.
    pub fn wakeup_handled(&self) {
        self.pending.store(false, Ordering::Release);
    }

    /// Locks the engine. Hold it briefly: the reader thread waits while you do.
    pub fn lock(&self) -> impl DerefMut<Target = Engine> + '_ {
        self.engine.lock()
    }

    pub fn pid(&self) -> i32 {
        self.task.pid()
    }

    /// Text the user typed (or the IME committed). Jumps back to the live screen.
    pub fn type_text(&self, text: &str) {
        self.scroll_to_bottom();
        self.task.write(text.as_bytes());
    }

    /// A key that isn't plain text. Returns false if it should go to text input instead.
    pub fn key(&self, key: Key, mods: Mods) -> bool {
        let mode = self.mode();
        match input::encode_key(key, mods, mode, self.option_as_meta) {
            Some(bytes) => {
                self.scroll_to_bottom();
                self.task.write(bytes);
                true
            }
            None => false,
        }
    }

    pub fn paste(&self, text: &str) {
        let bytes = input::encode_paste(text, self.mode());
        self.scroll_to_bottom();
        self.task.write(bytes);
    }

    /// Sends a mouse report if the program asked for them. Returns whether it did.
    pub fn mouse(&self, button: MouseButton, action: MouseAction, mods: Mods, col: usize, line: usize) -> bool {
        match input::encode_mouse(button, action, mods, col, line, self.mode()) {
            Some(bytes) => {
                self.task.write(bytes);
                true
            }
            None => false,
        }
    }

    pub fn focus(&self, focused: bool) {
        let mode = {
            let mut engine = self.engine.lock();
            engine.term.is_focused = focused;
            *engine.term.mode()
        };
        if let Some(bytes) = input::encode_focus(focused, mode) {
            self.task.write(bytes);
        }
    }

    /// Raw bytes to the program, untouched.
    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        self.task.write(bytes);
    }

    pub fn resize(&self, size: Size) {
        self.engine.lock().resize(grid_size(size), window_size(size));
        let _ = self.task.resize(size);
    }

    pub fn scroll(&self, lines: i32) {
        self.engine.lock().term.scroll_display(Scroll::Delta(lines));
    }

    pub fn scroll_to_bottom(&self) {
        self.engine.lock().term.scroll_display(Scroll::Bottom);
    }

    /// ⌘K: drop the scrollback and clear the screen. An idle shell is asked
    /// to redraw its prompt (Ctrl-L); a running program is left alone.
    pub fn clear(&self) {
        {
            let mut engine = self.engine.lock();
            engine.term.grid_mut().clear_history();
            if !self.task.is_busy() {
                engine.advance(b"\x1b[H\x1b[2J");
            }
        }
        if !self.task.is_busy() {
            self.task.write([0x0c]);
        }
    }

    /// Ends a synchronized update that outlived its deadline. Returns true if
    /// the screen changed.
    pub fn flush_sync(&self) -> bool {
        self.engine.lock().flush_sync(Instant::now())
    }

    /// The shell's directory, asked from the kernel; OSC 7 if the kernel can't say.
    pub fn cwd(&self) -> Option<PathBuf> {
        self.task.cwd().or_else(|| self.listener.cwd.lock().unwrap().clone())
    }

    /// The program in front: the shell itself, or e.g. `vim`.
    pub fn foreground(&self) -> Option<Process> {
        self.task.foreground()
    }

    pub fn is_busy(&self) -> bool {
        self.task.is_busy()
    }

    pub fn set_palette(&self, palette: Palette) {
        *self.listener.palette.lock().unwrap() = palette;
    }

    pub fn mode(&self) -> TermMode {
        *self.engine.lock().term.mode()
    }

    /// Hangs up the shell, like closing a window. Also happens on drop.
    pub fn close(&self) {
        self.task.hangup();
    }
}

fn grid_size(size: Size) -> GridSize {
    GridSize { rows: size.rows.max(1) as usize, cols: size.cols.max(2) as usize }
}

fn window_size(size: Size) -> WindowSize {
    WindowSize { num_lines: size.rows, num_cols: size.cols, cell_width: size.cell_width, cell_height: size.cell_height }
}
