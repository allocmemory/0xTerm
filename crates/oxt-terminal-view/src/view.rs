//! One terminal pane as a GPUI view: owns the session, routes keyboard, mouse
//! and IME input into it, and redraws when the shell prints.

use std::ops::Range;
use std::path::PathBuf;
use std::time::Instant;

use gpui_kit::*;
use oxt_session::alacritty_terminal::grid::Dimensions;
use oxt_session::alacritty_terminal::index::{Column, Point as GridPoint, Side};
use oxt_session::alacritty_terminal::selection::{Selection, SelectionType};
use oxt_session::alacritty_terminal::term::{TermMode, viewport_to_point};
use oxt_session::{Key, Mods, MouseAction, Session, SessionEvent, SessionOptions};
use oxt_task::Size;

use oxt_session::alacritty_terminal::index::Line;
use oxt_session::alacritty_terminal::term::cell::Flags;

use crate::element::{Frame, TerminalElement, build_frame};
use crate::links::{self, Link};
use crate::style::{CellSize, TerminalStyle, hsla};

actions!(terminal, [Copy, Paste, Clear, SelectAll, ScrollPageUp, ScrollPageDown, ScrollToTop, ScrollToBottom]);

pub const KEY_CONTEXT: &str = "Terminal";

pub fn bind_keys(cx: &mut App) {
    let ctx = Some(KEY_CONTEXT);
    cx.bind_keys([
        KeyBinding::new("cmd-c", Copy, ctx),
        KeyBinding::new("cmd-v", Paste, ctx),
        KeyBinding::new("cmd-k", Clear, ctx),
        KeyBinding::new("cmd-a", SelectAll, ctx),
        KeyBinding::new("shift-pageup", ScrollPageUp, ctx),
        KeyBinding::new("shift-pagedown", ScrollPageDown, ctx),
        KeyBinding::new("cmd-home", ScrollToTop, ctx),
        KeyBinding::new("cmd-end", ScrollToBottom, ctx),
    ]);
}

/// What a pane tells its window.
#[derive(Clone, Debug)]
pub enum TerminalEvent {
    TitleChanged,
    /// The shell reported a new directory (OSC 7).
    CwdChanged(PathBuf),
    Bell,
    Focused,
    Exited,
    /// ⌘-click on a file path in the output.
    OpenFile {
        path: PathBuf,
        line: Option<u64>,
    },
}

pub struct TerminalView {
    session: Session,
    focus: FocusHandle,
    style: TerminalStyle,
    title: Option<String>,
    exited: bool,
    /// Last laid-out geometry, for mapping the mouse to cells.
    cell: Option<CellSize>,
    origin: Point<Pixels>,
    bounds: Bounds<Pixels>,
    grid: Size,
    /// Where the IME candidate window should appear.
    ime_bounds: Bounds<Pixels>,
    /// IME composition in progress (e.g. typing Japanese), not yet sent.
    marked: Option<String>,
    /// Trackpad scrolling arrives in pixels; whole lines are taken from this.
    scroll_remainder: f32,
    selecting: bool,
}

impl EventEmitter<TerminalEvent> for TerminalView {}

impl Focusable for TerminalView {
    fn focus_handle(&self, _: &App) -> FocusHandle {
        self.focus.clone()
    }
}

impl TerminalView {
    /// Starts a shell and wraps it in a new pane.
    pub fn spawn(options: SessionOptions, style: TerminalStyle, window: &mut Window, cx: &mut App) -> std::io::Result<Entity<Self>> {
        let grid = options.size;
        let (session, events) = Session::spawn(options)?;
        Ok(cx.new(|cx| Self::new(session, events, grid, style, window, cx)))
    }

    fn new(
        session: Session,
        events: async_channel::Receiver<SessionEvent>,
        grid: Size,
        style: TerminalStyle,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        session.set_palette(style.palette.clone());

        cx.spawn(async move |this, cx| {
            while let Ok(event) = events.recv().await {
                if this.update(cx, |view, cx| view.on_session_event(event, cx)).is_err() {
                    break;
                }
            }
        })
        .detach();

        let focus = cx.focus_handle();
        cx.on_focus(&focus, window, |view, _, cx| {
            view.session.focus(true);
            cx.emit(TerminalEvent::Focused);
            cx.notify();
        })
        .detach();
        cx.on_blur(&focus, window, |view, _, cx| {
            view.session.focus(false);
            cx.notify();
        })
        .detach();

        TerminalView {
            session,
            focus,
            style,
            title: None,
            exited: false,
            cell: None,
            origin: Point::default(),
            bounds: Bounds::default(),
            grid,
            ime_bounds: Bounds::default(),
            marked: None,
            scroll_remainder: 0.,
            selecting: false,
        }
    }

    pub fn session(&self) -> &Session {
        &self.session
    }

    /// The title a program set (OSC 0/2), if any.
    pub fn title(&self) -> Option<&str> {
        self.title.as_deref()
    }

    pub fn has_exited(&self) -> bool {
        self.exited
    }

    pub fn set_style(&mut self, style: TerminalStyle, cx: &mut Context<Self>) {
        if style != self.style {
            self.session.set_palette(style.palette.clone());
            self.style = style;
            self.cell = None;
            cx.notify();
        }
    }

    fn on_session_event(&mut self, event: SessionEvent, cx: &mut Context<Self>) {
        match event {
            SessionEvent::Wakeup => {
                self.session.wakeup_handled();
                cx.notify();
            }
            SessionEvent::Title(title) => {
                self.title = title;
                cx.emit(TerminalEvent::TitleChanged);
            }
            SessionEvent::Cwd(dir) => cx.emit(TerminalEvent::CwdChanged(dir)),
            SessionEvent::Bell => cx.emit(TerminalEvent::Bell),
            SessionEvent::Copy(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
            SessionEvent::SyncPending(deadline) => {
                cx.spawn(async move |this, cx| {
                    cx.background_executor().timer(deadline.saturating_duration_since(Instant::now())).await;
                    let _ = this.update(cx, |view, cx| {
                        if view.session.flush_sync() {
                            cx.notify();
                        }
                    });
                })
                .detach();
            }
            SessionEvent::Exited(_) => {
                self.exited = true;
                cx.emit(TerminalEvent::Exited);
                cx.notify();
            }
        }
    }

    /// Lays out the grid for `bounds`, resizes the pty if needed, and builds the frame.
    pub(crate) fn frame(&mut self, bounds: Bounds<Pixels>, focused: bool, window: &mut Window) -> Frame {
        let cell = *self.cell.get_or_insert_with(|| CellSize::measure(&self.style, window));
        let pad = self.style.padding;
        let cols = ((bounds.size.width - pad * 2.) / cell.width).floor().max(2.) as u16;
        let rows = ((bounds.size.height - pad) / cell.height).floor().max(1.) as u16;
        let grid = Size { rows, cols, cell_width: f32::from(cell.width) as u16, cell_height: f32::from(cell.height) as u16 };
        if grid != self.grid {
            self.grid = grid;
            self.session.resize(grid);
        }
        self.bounds = bounds;
        self.origin = point(bounds.origin.x + pad, bounds.origin.y + pad / 2.);

        let engine = self.session.lock();
        let (frame, ime) = build_frame(&engine, &self.style, cell, self.origin, focused, self.marked.as_deref(), window);
        self.ime_bounds = ime;
        frame
    }

    /// Mouse position → grid point (scrollback-aware), the half of the cell it's
    /// in, and the on-screen cell for mouse reports.
    fn grid_point(&self, position: Point<Pixels>) -> (GridPoint, Side, usize, usize) {
        let cell = self.cell.unwrap_or(CellSize { width: px(8.), height: px(16.) });
        let x = f32::from(position.x - self.origin.x) / f32::from(cell.width);
        let y = f32::from(position.y - self.origin.y) / f32::from(cell.height);
        let col = (x.max(0.) as usize).min(self.grid.cols.saturating_sub(1) as usize);
        let line = (y.max(0.) as usize).min(self.grid.rows.saturating_sub(1) as usize);
        let side = if x.fract() > 0.5 { Side::Right } else { Side::Left };
        let offset = self.session.lock().term.grid().display_offset();
        (viewport_to_point(offset, GridPoint::new(line, Column(col))), side, col, line)
    }

    /// The link under a grid point: an OSC 8 hyperlink, else a URL or path in the text.
    fn link_at(&self, point: GridPoint) -> Option<Link> {
        let cwd = self.session.cwd();
        let home = std::env::var_os("HOME").map(PathBuf::from);
        let engine = self.session.lock();
        let grid = engine.term.grid();
        if let Some(link) = grid[point].hyperlink() {
            let uri = link.uri().to_owned();
            drop(engine);
            return match uri.strip_prefix("file://") {
                Some(rest) => {
                    let path = PathBuf::from(&rest[rest.find('/').unwrap_or(0)..]);
                    path.is_file().then_some(Link::File { path, line: None })
                }
                None => Some(Link::Url(uri)),
            };
        }
        // A long path may wrap across rows; join the whole logical line.
        let wrapped = |line: Line| grid[line][grid.last_column()].flags.contains(Flags::WRAPLINE);
        let mut first = point.line;
        while first > grid.topmost_line() && wrapped(first - 1) {
            first -= 1;
        }
        let mut text = String::new();
        let mut col = 0;
        let mut line = first;
        loop {
            for c in 0..grid.columns() {
                let cell = &grid[line][Column(c)];
                if cell.flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
                    continue;
                }
                if line == point.line && c == point.column.0 {
                    col = text.chars().count();
                }
                text.push(cell.c);
            }
            if !wrapped(line) || line >= grid.bottommost_line() {
                break;
            }
            line += 1;
        }
        drop(engine);
        links::link_at(&text, col, cwd.as_deref(), home.as_deref())
    }

    fn mods(m: &Modifiers) -> Mods {
        Mods { shift: m.shift, ctrl: m.control, alt: m.alt }
    }

    fn key_down(&mut self, event: &KeyDownEvent, _: &mut Window, cx: &mut Context<Self>) {
        let ks = &event.keystroke;
        if ks.modifiers.platform || self.exited {
            return; // ⌘ shortcuts belong to the app.
        }
        let key = match ks.key.as_str() {
            "up" => Key::Up,
            "down" => Key::Down,
            "left" => Key::Left,
            "right" => Key::Right,
            "home" => Key::Home,
            "end" => Key::End,
            "pageup" => Key::PageUp,
            "pagedown" => Key::PageDown,
            "insert" => Key::Insert,
            "delete" => Key::Delete,
            "enter" => Key::Enter,
            "tab" => Key::Tab,
            "backspace" => Key::Backspace,
            "escape" => Key::Escape,
            "space" => Key::Char(' '),
            f if f.len() > 1 && f.starts_with('f') && f[1..].parse::<u8>().is_ok() => Key::F(f[1..].parse().unwrap()),
            k if k.chars().count() == 1 => Key::Char(k.chars().next().unwrap()),
            _ => return,
        };
        // Plain characters go through the text input path so IME and dead keys work.
        if let Key::Char(_) = key
            && !ks.modifiers.control
            && !ks.modifiers.alt
        {
            return;
        }
        if self.marked.is_some() {
            return; // The IME owns keys while composing.
        }
        if self.session.key(key, Self::mods(&ks.modifiers)) {
            self.clear_selection();
            cx.stop_propagation();
            cx.notify();
        }
    }

    fn mouse_down(&mut self, event: &MouseDownEvent, window: &mut Window, cx: &mut Context<Self>) {
        window.focus(&self.focus, cx);
        let (point, side, col, line) = self.grid_point(event.position);
        if event.modifiers.platform && event.button == MouseButton::Left {
            match self.link_at(point) {
                Some(Link::Url(url)) => cx.open_url(&url),
                Some(Link::File { path, line }) => cx.emit(TerminalEvent::OpenFile { path, line }),
                None => {}
            }
            return;
        }
        if self.reports_mouse(&event.modifiers) {
            if let Some(button) = report_button(event.button) {
                self.session.mouse(button, MouseAction::Press, Self::mods(&event.modifiers), col, line);
            }
            return;
        }
        if event.button != MouseButton::Left {
            return;
        }
        let kind = match event.click_count {
            2 => SelectionType::Semantic,
            n if n >= 3 => SelectionType::Lines,
            _ if event.modifiers.alt => SelectionType::Block,
            _ => SelectionType::Simple,
        };
        let mut engine = self.session.lock();
        if event.modifiers.shift
            && let Some(selection) = engine.term.selection.as_mut()
        {
            selection.update(point, side);
        } else {
            engine.term.selection = Some(Selection::new(kind, point, side));
        }
        drop(engine);
        self.selecting = true;
        cx.notify();
    }

    fn mouse_move(&mut self, event: &MouseMoveEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (point, side, col, line) = self.grid_point(event.position);
        if self.reports_mouse(&event.modifiers) {
            let held = event.pressed_button.and_then(report_button);
            self.session.mouse(held.unwrap_or(oxt_session::MouseButton::Left), MouseAction::Move(held), Self::mods(&event.modifiers), col, line);
            return;
        }
        if !self.selecting || event.pressed_button != Some(MouseButton::Left) {
            return;
        }
        // Dragging past the top or bottom edge scrolls the history.
        if event.position.y < self.bounds.top() {
            self.session.scroll(1);
        } else if event.position.y > self.bounds.bottom() {
            self.session.scroll(-1);
        }
        if let Some(selection) = self.session.lock().term.selection.as_mut() {
            selection.update(point, side);
        }
        cx.notify();
    }

    fn mouse_up(&mut self, event: &MouseUpEvent, _: &mut Window, cx: &mut Context<Self>) {
        let (_, _, col, line) = self.grid_point(event.position);
        if self.reports_mouse(&event.modifiers) {
            if let Some(button) = report_button(event.button) {
                self.session.mouse(button, MouseAction::Release, Self::mods(&event.modifiers), col, line);
            }
            return;
        }
        if self.selecting {
            self.selecting = false;
            let mut engine = self.session.lock();
            if engine.term.selection.as_ref().is_some_and(|s| s.is_empty()) {
                engine.term.selection = None;
            }
            cx.notify();
        }
    }

    fn scroll_wheel(&mut self, event: &ScrollWheelEvent, _: &mut Window, cx: &mut Context<Self>) {
        let line_height = self.cell.map(|c| f32::from(c.height)).unwrap_or(16.);
        let lines = match event.delta {
            ScrollDelta::Pixels(delta) => {
                self.scroll_remainder += f32::from(delta.y) / line_height;
                let whole = self.scroll_remainder.trunc();
                self.scroll_remainder -= whole;
                whole as i32
            }
            ScrollDelta::Lines(delta) => delta.y.round() as i32,
        };
        if lines == 0 {
            return;
        }
        let mode = self.session.mode();
        let (_, _, col, line) = self.grid_point(event.position);
        if mode.intersects(TermMode::MOUSE_MODE) && !event.modifiers.shift {
            let button = if lines > 0 { oxt_session::MouseButton::WheelUp } else { oxt_session::MouseButton::WheelDown };
            for _ in 0..lines.unsigned_abs().min(10) {
                self.session.mouse(button, MouseAction::Press, Self::mods(&event.modifiers), col, line);
            }
        } else if mode.contains(TermMode::ALT_SCREEN | TermMode::ALTERNATE_SCROLL) {
            // less, man, vim without mouse mode: the wheel becomes arrow keys.
            let key = if lines > 0 { Key::Up } else { Key::Down };
            for _ in 0..lines.unsigned_abs().min(10) {
                self.session.key(key, Mods::default());
            }
        } else {
            self.session.scroll(lines);
            cx.notify();
        }
    }

    /// Mouse goes to the program when it asked for it; Shift keeps it for selection.
    fn reports_mouse(&self, mods: &Modifiers) -> bool {
        !mods.shift && self.session.mode().intersects(TermMode::MOUSE_MODE)
    }

    fn clear_selection(&mut self) {
        self.session.lock().term.selection = None;
    }

    pub fn selected_text(&self) -> Option<String> {
        self.session.lock().term.selection_to_string().filter(|s| !s.is_empty())
    }

    fn copy(&mut self, _: &Copy, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = self.selected_text() {
            cx.write_to_clipboard(ClipboardItem::new_string(text));
        }
    }

    fn paste(&mut self, _: &Paste, _: &mut Window, cx: &mut Context<Self>) {
        if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
            self.clear_selection();
            self.session.paste(&text);
        }
    }

    fn clear(&mut self, _: &Clear, _: &mut Window, cx: &mut Context<Self>) {
        self.session.clear();
        cx.notify();
    }

    fn select_all(&mut self, _: &SelectAll, _: &mut Window, cx: &mut Context<Self>) {
        let mut engine = self.session.lock();
        let grid = engine.term.grid();
        let top = GridPoint::new(grid.topmost_line(), Column(0));
        let bottom = GridPoint::new(grid.bottommost_line(), grid.last_column());
        let mut selection = Selection::new(SelectionType::Simple, top, Side::Left);
        selection.update(bottom, Side::Right);
        engine.term.selection = Some(selection);
        drop(engine);
        cx.notify();
    }

    fn page(&mut self, up: bool, cx: &mut Context<Self>) {
        let rows = self.grid.rows as i32;
        self.session.scroll(if up { rows } else { -rows });
        cx.notify();
    }

    /// Writes text as if typed, e.g. a path dropped from the file tree.
    pub fn insert_text(&mut self, text: &str, cx: &mut Context<Self>) {
        self.session.type_text(text);
        cx.notify();
    }
}

fn report_button(button: MouseButton) -> Option<oxt_session::MouseButton> {
    match button {
        MouseButton::Left => Some(oxt_session::MouseButton::Left),
        MouseButton::Middle => Some(oxt_session::MouseButton::Middle),
        MouseButton::Right => Some(oxt_session::MouseButton::Right),
        _ => None,
    }
}

impl Render for TerminalView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("terminal")
            .key_context(KEY_CONTEXT)
            .track_focus(&self.focus)
            .size_full()
            .overflow_hidden()
            .bg(hsla(self.style.palette.background))
            .cursor(CursorStyle::IBeam)
            .on_action(cx.listener(Self::copy))
            .on_action(cx.listener(Self::paste))
            .on_action(cx.listener(Self::clear))
            .on_action(cx.listener(Self::select_all))
            .on_action(cx.listener(|view, _: &ScrollPageUp, _, cx| view.page(true, cx)))
            .on_action(cx.listener(|view, _: &ScrollPageDown, _, cx| view.page(false, cx)))
            .on_action(cx.listener(|view, _: &ScrollToTop, _, cx| {
                view.session.scroll(i32::MAX / 2);
                cx.notify();
            }))
            .on_action(cx.listener(|view, _: &ScrollToBottom, _, cx| {
                view.session.scroll_to_bottom();
                cx.notify();
            }))
            .on_key_down(cx.listener(Self::key_down))
            .on_mouse_down(MouseButton::Left, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Right, cx.listener(Self::mouse_down))
            .on_mouse_down(MouseButton::Middle, cx.listener(Self::mouse_down))
            .on_mouse_move(cx.listener(Self::mouse_move))
            .on_mouse_up(MouseButton::Left, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Right, cx.listener(Self::mouse_up))
            .on_mouse_up(MouseButton::Middle, cx.listener(Self::mouse_up))
            .on_scroll_wheel(cx.listener(Self::scroll_wheel))
            .child(TerminalElement::new(cx.entity(), self.focus.clone()))
    }
}

/// IME and text input. The terminal has no editable text of its own: committed
/// text goes straight to the shell, composition is shown at the cursor.
impl EntityInputHandler for TerminalView {
    fn text_for_range(&mut self, _: Range<usize>, _: &mut Option<Range<usize>>, _: &mut Window, _: &mut Context<Self>) -> Option<String> {
        None
    }

    fn selected_text_range(&mut self, _: bool, _: &mut Window, _: &mut Context<Self>) -> Option<UTF16Selection> {
        Some(UTF16Selection { range: 0..0, reversed: false })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.as_ref().map(|m| 0..m.encode_utf16().count())
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(&mut self, _: Option<Range<usize>>, text: &str, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        if !self.exited && !text.is_empty() {
            self.clear_selection();
            self.session.type_text(text);
        }
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        _: Option<Range<usize>>,
        text: &str,
        _: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.marked = (!text.is_empty()).then(|| text.to_owned());
        cx.notify();
    }

    fn bounds_for_range(&mut self, _: Range<usize>, _: Bounds<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<Bounds<Pixels>> {
        Some(self.ime_bounds)
    }

    fn character_index_for_point(&mut self, _: Point<Pixels>, _: &mut Window, _: &mut Context<Self>) -> Option<usize> {
        None
    }
}
