//! Keys, paste and mouse → the bytes a program expects (xterm conventions).
//!
//! Plain text (including IME composition) skips this and is written as UTF-8.
//! This module only handles what text can't express: arrows, function keys,
//! control and Meta combinations, bracketed paste and mouse reports.

use alacritty_terminal::term::TermMode;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Key {
    /// A character key, unshifted ('a', '[', '2'): used with Ctrl or Meta.
    Char(char),
    Enter,
    Tab,
    Backspace,
    Escape,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    F(u8),
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Mods {
    pub shift: bool,
    pub ctrl: bool,
    /// Option. Sent as Meta (an ESC prefix) only when `option_as_meta` is on.
    pub alt: bool,
}

impl Mods {
    /// xterm's modifier parameter: 1 + shift + 2·alt + 4·ctrl.
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8
    }
}

/// Encodes a key press. `None` means "not ours": let the text input handle it.
pub fn encode_key(key: Key, mods: Mods, mode: TermMode, option_as_meta: bool) -> Option<Vec<u8>> {
    let meta = mods.alt && option_as_meta;
    let app_cursor = mode.contains(TermMode::APP_CURSOR);
    let modified = mods.shift || mods.ctrl || meta;
    let csi_mods = Mods { alt: meta, ..mods }.param();

    // Cursor keys: ESC [ A, or ESC O A in application mode, or ESC [ 1 ; m A with modifiers.
    let cursor = |final_byte: u8| -> Vec<u8> {
        if modified {
            format!("\x1b[1;{csi_mods}{}", final_byte as char).into_bytes()
        } else if app_cursor {
            vec![0x1b, b'O', final_byte]
        } else {
            vec![0x1b, b'[', final_byte]
        }
    };
    // Editing keys: ESC [ n ~ or ESC [ n ; m ~.
    let tilde = |n: u8| -> Vec<u8> { if modified { format!("\x1b[{n};{csi_mods}~") } else { format!("\x1b[{n}~") }.into_bytes() };
    let with_meta = |bytes: Vec<u8>| if meta { [&[0x1b], bytes.as_slice()].concat() } else { bytes };

    Some(match key {
        Key::Up => cursor(b'A'),
        Key::Down => cursor(b'B'),
        Key::Right => cursor(b'C'),
        Key::Left => cursor(b'D'),
        Key::Home => cursor(b'H'),
        Key::End => cursor(b'F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F(n @ 1..=4) => {
            let final_byte = b'P' + (n - 1);
            if modified { format!("\x1b[1;{csi_mods}{}", final_byte as char).into_bytes() } else { vec![0x1b, b'O', final_byte] }
        }
        Key::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][(n - 5) as usize]),
        Key::F(_) => return None,
        Key::Enter => with_meta(vec![b'\r']),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => with_meta(vec![b'\t']),
        Key::Backspace if mods.ctrl => with_meta(vec![0x08]),
        Key::Backspace => with_meta(vec![0x7f]),
        Key::Escape => with_meta(vec![0x1b]),
        Key::Char(c) if mods.ctrl => with_meta(vec![control(c)?]),
        Key::Char(c) if meta => with_meta(c.to_string().into_bytes()),
        Key::Char(_) => return None,
    })
}

/// Ctrl+key → C0 byte: Ctrl-A is 0x01, Ctrl-[ is ESC, Ctrl-Space is NUL.
fn control(c: char) -> Option<u8> {
    Some(match c.to_ascii_lowercase() {
        c @ 'a'..='z' => c as u8 - b'a' + 1,
        '@' | ' ' | '2' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// Paste. With bracketed paste on, the program knows it's a paste and won't
/// run it line by line. ESC is removed so a pasted text can't end the bracket.
pub fn encode_paste(text: &str, mode: TermMode) -> Vec<u8> {
    if mode.contains(TermMode::BRACKETED_PASTE) {
        let clean = text.replace('\x1b', "");
        format!("\x1b[200~{clean}\x1b[201~").into_bytes()
    } else {
        text.replace("\r\n", "\r").replace('\n', "\r").into_bytes()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseButton {
    Left,
    Middle,
    Right,
    WheelUp,
    WheelDown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseAction {
    Press,
    Release,
    /// Movement; `Some(button)` while a button is held.
    Move(Option<MouseButton>),
}

/// Encodes a mouse event for a program that asked for mouse reports
/// (modes 1000/1002/1003, SGR 1006). `col`/`line` are 0-based cells.
/// `None` when the current mode doesn't want this event.
pub fn encode_mouse(button: MouseButton, action: MouseAction, mods: Mods, col: usize, line: usize, mode: TermMode) -> Option<Vec<u8>> {
    let wheel = matches!(button, MouseButton::WheelUp | MouseButton::WheelDown);
    let wanted = match action {
        _ if !mode.intersects(TermMode::MOUSE_MODE) => false,
        MouseAction::Press | MouseAction::Release => true,
        MouseAction::Move(Some(_)) => mode.intersects(TermMode::MOUSE_DRAG | TermMode::MOUSE_MOTION),
        MouseAction::Move(None) => mode.contains(TermMode::MOUSE_MOTION),
    };
    if !wanted || (wheel && action == MouseAction::Release) {
        return None;
    }

    let base = match action {
        MouseAction::Move(None) => 3,
        MouseAction::Move(Some(b)) => code(b),
        _ => code(button),
    };
    let mut cb = base + 4 * mods.shift as u32 + 8 * mods.alt as u32 + 16 * mods.ctrl as u32;
    if matches!(action, MouseAction::Move(_)) {
        cb += 32;
    }

    if mode.contains(TermMode::SGR_MOUSE) {
        let end = if action == MouseAction::Release { 'm' } else { 'M' };
        return Some(format!("\x1b[<{cb};{};{}{end}", col + 1, line + 1).into_bytes());
    }
    // Legacy X10 encoding: one byte per value, so only the first 223 cells are reachable.
    if col >= 223 || line >= 223 {
        return None;
    }
    if action == MouseAction::Release {
        cb = 3 + (cb & !3);
    }
    Some(vec![0x1b, b'[', b'M', 32 + cb as u8, 33 + col as u8, 33 + line as u8])
}

fn code(button: MouseButton) -> u32 {
    match button {
        MouseButton::Left => 0,
        MouseButton::Middle => 1,
        MouseButton::Right => 2,
        MouseButton::WheelUp => 64,
        MouseButton::WheelDown => 65,
    }
}

/// Focus reports (mode 1004): vim uses them to reload files changed elsewhere.
pub fn encode_focus(focused: bool, mode: TermMode) -> Option<&'static [u8]> {
    mode.contains(TermMode::FOCUS_IN_OUT).then_some(if focused { b"\x1b[I" } else { b"\x1b[O" })
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods { shift: false, ctrl: false, alt: false };
    const CTRL: Mods = Mods { shift: false, ctrl: true, alt: false };
    const ALT: Mods = Mods { shift: false, ctrl: false, alt: true };
    const SHIFT: Mods = Mods { shift: true, ctrl: false, alt: false };

    fn key(k: Key, m: Mods, mode: TermMode) -> Vec<u8> {
        encode_key(k, m, mode, true).unwrap()
    }

    #[test]
    fn arrows_follow_cursor_mode() {
        assert_eq!(key(Key::Up, NONE, TermMode::NONE), b"\x1b[A");
        assert_eq!(key(Key::Up, NONE, TermMode::APP_CURSOR), b"\x1bOA");
        assert_eq!(key(Key::Left, CTRL, TermMode::APP_CURSOR), b"\x1b[1;5D");
        assert_eq!(key(Key::Right, ALT, TermMode::NONE), b"\x1b[1;3C");
    }

    #[test]
    fn control_and_meta() {
        assert_eq!(key(Key::Char('c'), CTRL, TermMode::NONE), [0x03]);
        assert_eq!(key(Key::Char('['), CTRL, TermMode::NONE), [0x1b]);
        assert_eq!(key(Key::Char('b'), ALT, TermMode::NONE), b"\x1bb");
        assert_eq!(key(Key::Backspace, ALT, TermMode::NONE), b"\x1b\x7f");
        assert_eq!(encode_key(Key::Char('b'), ALT, TermMode::NONE, false), None);
        assert_eq!(encode_key(Key::Char('a'), NONE, TermMode::NONE, true), None);
    }

    #[test]
    fn editing_and_function_keys() {
        assert_eq!(key(Key::Delete, NONE, TermMode::NONE), b"\x1b[3~");
        assert_eq!(key(Key::PageUp, SHIFT, TermMode::NONE), b"\x1b[5;2~");
        assert_eq!(key(Key::F(1), NONE, TermMode::NONE), b"\x1bOP");
        assert_eq!(key(Key::F(5), NONE, TermMode::NONE), b"\x1b[15~");
        assert_eq!(key(Key::Tab, SHIFT, TermMode::NONE), b"\x1b[Z");
    }

    #[test]
    fn paste() {
        assert_eq!(encode_paste("a\nb", TermMode::NONE), b"a\rb");
        assert_eq!(encode_paste("a\x1b[201~b", TermMode::BRACKETED_PASTE), b"\x1b[200~a[201~b\x1b[201~");
    }

    #[test]
    fn mouse() {
        let sgr = TermMode::MOUSE_REPORT_CLICK | TermMode::SGR_MOUSE;
        assert_eq!(encode_mouse(MouseButton::Left, MouseAction::Press, NONE, 4, 2, sgr).unwrap(), b"\x1b[<0;5;3M");
        assert_eq!(encode_mouse(MouseButton::Left, MouseAction::Release, NONE, 4, 2, sgr).unwrap(), b"\x1b[<0;5;3m");
        assert_eq!(encode_mouse(MouseButton::WheelDown, MouseAction::Press, CTRL, 0, 0, sgr).unwrap(), b"\x1b[<81;1;1M");
        assert_eq!(encode_mouse(MouseButton::Left, MouseAction::Move(Some(MouseButton::Left)), NONE, 0, 0, sgr), None);
        let x10 = TermMode::MOUSE_REPORT_CLICK;
        assert_eq!(encode_mouse(MouseButton::Right, MouseAction::Release, NONE, 0, 0, x10).unwrap(), b"\x1b[M#!!");
        assert_eq!(encode_mouse(MouseButton::Left, MouseAction::Press, NONE, 0, 0, TermMode::NONE), None);
    }
}
