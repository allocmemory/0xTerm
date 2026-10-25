use std::time::{Duration, Instant};

use alacritty_terminal::index::{Column, Line, Point};
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::{Color, NamedColor};
use oxt_session::{Engine, GridSize, Listener, Session, SessionEvent, SessionOptions};
use oxt_task::{Command, Size};

/// An engine with no pty: replies to queries are collected instead of written.
fn engine(rows: usize, cols: usize) -> (Engine, std::sync::Arc<std::sync::Mutex<Vec<u8>>>, async_channel::Receiver<SessionEvent>) {
    let replies = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
    let sink = replies.clone();
    let (tx, rx) = async_channel::unbounded();
    let listener = Listener::new(move |bytes| sink.lock().unwrap().extend_from_slice(bytes), tx);
    (Engine::new(GridSize { rows, cols }, 100, listener), replies, rx)
}

fn row(engine: &Engine, line: i32) -> String {
    let grid = engine.term.grid();
    (0..grid.columns()).map(|c| grid[Line(line)][Column(c)].c).collect::<String>().trim_end().to_owned()
}

use alacritty_terminal::grid::Dimensions;

#[test]
fn text_lands_in_the_grid() {
    let (mut e, _, _) = engine(4, 20);
    e.advance(b"hello\r\nworld");
    assert_eq!(row(&e, 0), "hello");
    assert_eq!(row(&e, 1), "world");
    assert_eq!(e.term.grid().cursor.point, Point::new(Line(1), Column(5)));
}

#[test]
fn colors_and_alt_screen() {
    let (mut e, _, _) = engine(4, 20);
    e.advance(b"\x1b[31mR\x1b[38;2;1;2;3mT\x1b[0m");
    let grid = e.term.grid();
    assert_eq!(grid[Line(0)][Column(0)].fg, Color::Named(NamedColor::Red));
    assert!(matches!(grid[Line(0)][Column(1)].fg, Color::Spec(rgb) if (rgb.r, rgb.g, rgb.b) == (1, 2, 3)));

    e.advance(b"\x1b[?1049h");
    assert!(e.term.mode().contains(TermMode::ALT_SCREEN));
    assert_eq!(row(&e, 0), "");
    e.advance(b"\x1b[?1049l");
    assert_eq!(row(&e, 0), "RT");
}

#[test]
fn scrollback_keeps_old_lines() {
    let (mut e, _, _) = engine(3, 10);
    for i in 0..10 {
        e.advance(format!("line{i}\r\n").as_bytes());
    }
    assert_eq!(e.term.grid().history_size(), 8);
    assert_eq!(row(&e, -8), "line0");
}

#[test]
fn kitty_keyboard_handshake() {
    // Claude Code's sequence: query, and only if answered, push flags 5 (disambiguate + alternate keys).
    let (mut e, replies, _) = engine(4, 20);
    e.advance(b"\x1b[?u");
    assert_eq!(&*replies.lock().unwrap(), b"\x1b[?0u");
    e.advance(b"\x1b[>5u");
    assert!(e.term.mode().contains(TermMode::DISAMBIGUATE_ESC_CODES | TermMode::REPORT_ALTERNATE_KEYS));
    e.advance(b"\x1b[<u");
    assert!(!e.term.mode().intersects(TermMode::KITTY_KEYBOARD_PROTOCOL));
}

#[test]
fn answers_queries() {
    let (mut e, replies, _) = engine(4, 20);
    e.advance(b"ab\x1b[6n"); // where is the cursor?
    assert_eq!(&*replies.lock().unwrap(), b"\x1b[1;3R");
    replies.lock().unwrap().clear();
    e.advance(b"\x1b]11;?\x07"); // what's the background? (dark/light detection)
    assert_eq!(&*replies.lock().unwrap(), b"\x1b]11;rgb:0b0b/0b0b/0b0b\x07");
}

#[test]
fn osc7_and_title_become_events() {
    let (mut e, _, rx) = engine(4, 20);
    e.advance(b"\x1b]7;file://host/tmp/a%20b\x07\x1b]2;my title\x07");
    let events: Vec<_> = std::iter::from_fn(|| rx.try_recv().ok()).collect();
    assert!(events.iter().any(|ev| matches!(ev, SessionEvent::Cwd(p) if p.to_str() == Some("/tmp/a b"))));
    assert!(events.iter().any(|ev| matches!(ev, SessionEvent::Title(Some(t)) if t == "my title")));
}

#[test]
fn synchronized_update_is_held_then_flushed() {
    let (mut e, _, rx) = engine(4, 20);
    e.advance(b"\x1b[?2026hhidden");
    assert!(!e.advance(b" more"), "bytes inside the update must not wake the UI");
    assert_eq!(row(&e, 0), "");
    assert!(rx.try_iter_pending().any(|ev| matches!(ev, SessionEvent::SyncPending(_))));
    assert!(e.flush_sync(Instant::now() + Duration::from_secs(1)));
    assert_eq!(row(&e, 0), "hidden more");
}

trait TryIterPending<T> {
    fn try_iter_pending(&self) -> std::vec::IntoIter<T>;
}
impl<T> TryIterPending<T> for async_channel::Receiver<T> {
    fn try_iter_pending(&self) -> std::vec::IntoIter<T> {
        std::iter::from_fn(|| self.try_recv().ok()).collect::<Vec<_>>().into_iter()
    }
}

#[test]
fn real_shell_end_to_end() {
    let options = SessionOptions { command: Command::new("/bin/sh").env("PS1", "$ "), size: Size::new(10, 40), ..SessionOptions::default() };
    let (session, rx) = Session::spawn(options).unwrap();
    session.type_text("printf '\\033[32m%s\\033[0m\\n' gr''een\n");
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        assert!(Instant::now() < deadline, "never drew green");
        if let Ok(SessionEvent::Wakeup) = rx.recv_blocking() {
            session.wakeup_handled();
            let engine = session.lock();
            let grid = engine.term.grid();
            let found = (0..grid.screen_lines() as i32).any(|l| {
                let line = &grid[Line(l)];
                (0..grid.columns()).any(|c| line[Column(c)].c == 'g' && line[Column(c)].fg == Color::Named(NamedColor::Green))
            });
            if found {
                break;
            }
        }
    }
    assert!(session.cwd().is_some());
    session.close();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(SessionEvent::Exited(_)) = rx.recv_blocking() {
            return;
        }
    }
    panic!("shell didn't exit");
}

#[test]
fn clipboard_writes_have_a_size_limit() {
    let (mut e, _, rx) = engine(4, 20);
    e.advance(b"\x1b]52;c;aGk=\x07"); // "hi"
    assert!(rx.try_iter_pending().any(|ev| matches!(ev, SessionEvent::Copy(t) if t == "hi")));

    // "QUFB" is base64 for "AAA": 8 MiB of text, more than anyone means to copy.
    let big = format!("\x1b]52;c;{}\x07", "QUFB".repeat(8 * 1024 * 1024 / 3));
    e.advance(big.as_bytes());
    let events: Vec<_> = rx.try_iter_pending().collect();
    assert!(!events.iter().any(|ev| matches!(ev, SessionEvent::Copy(_))), "an 8 MiB write reached the clipboard");
    // Blocked, not silently: the UI tells the user, with about the size.
    assert!(events.iter().any(|ev| matches!(ev, SessionEvent::CopyBlocked(n) if n.abs_diff(8 * 1024 * 1024) < 1024)), "{events:?}");

    // Between the limit and the length an escape sequence may have, the parser
    // sees it whole and reports the exact size.
    let over = format!("\x1b]52;c;{}\x07", "QUFB".repeat(1024 * 1024 / 3 + 1));
    e.advance(over.as_bytes());
    assert!(rx.try_iter_pending().any(|ev| matches!(ev, SessionEvent::CopyBlocked(n) if n == (1024 * 1024 / 3 + 1) * 3)));
}

/// An escape sequence that never ends, e.g. `cat` of a hostile file.
fn huge_osc(e: &mut Engine, mib: usize) {
    e.advance(b"\x1b]0;");
    let chunk = vec![b'x'; 64 * 1024];
    for _ in 0..mib * 16 {
        e.advance(&chunk);
    }
    e.advance(b"\x07after");
}

#[test]
fn oversized_escape_sequence_is_dropped_and_output_resumes() {
    let (mut e, _, _) = engine(4, 20);
    huge_osc(&mut e, 4);
    assert_eq!(row(&e, 0), "after");
    // A clipboard write near its 1 MiB limit is still a whole sequence.
    let (mut e, _, rx) = engine(4, 20);
    let copy = format!("\x1b]52;c;{}\x07", "QUFB".repeat(1024 * 1024 / 3 - 1));
    e.advance(copy.as_bytes());
    assert!(rx.try_iter_pending().any(|ev| matches!(ev, SessionEvent::Copy(_))));
}

/// Run in release under `/usr/bin/time -l` to see peak memory.
#[test]
#[ignore]
fn huge_escape_sequence_memory() {
    let (mut e, _, _) = engine(4, 20);
    huge_osc(&mut e, 512);
    assert_eq!(row(&e, 0), "after");
}
