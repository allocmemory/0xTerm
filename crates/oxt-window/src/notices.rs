//! What the window tells you about something a program did behind your back.

/// When a path's name can't be typed safely into the shell in the pane.
pub const UNSAFE_PATH: &str = "This name has characters this shell can't be given safely, so it wasn't typed in.";

/// After a program copied text to the clipboard (OSC 52).
pub fn copied(chars: usize) -> String {
    let unit = if chars == 1 { "character" } else { "characters" };
    format!("A program copied {} {unit} to the clipboard.", thousands(chars))
}

/// After a program tried to copy more than the limit.
pub fn copy_blocked(bytes: usize, limit: usize) -> String {
    format!("A program tried to copy {} to the clipboard. Blocked: the limit is {}.", megabytes(bytes), megabytes(limit))
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn megabytes(bytes: usize) -> String {
    let mb = bytes as f64 / 1_048_576.;
    if mb.fract() == 0. { format!("{mb:.0} MB") } else { format!("{mb:.1} MB") }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn notice_text() {
        assert_eq!(copied(1), "A program copied 1 character to the clipboard.");
        assert_eq!(copied(1234), "A program copied 1,234 characters to the clipboard.");
        assert_eq!(copy_blocked(8 * 1_048_576, 1_048_576), "A program tried to copy 8 MB to the clipboard. Blocked: the limit is 1 MB.");
        assert_eq!(copy_blocked(1_572_864, 1_048_576), "A program tried to copy 1.5 MB to the clipboard. Blocked: the limit is 1 MB.");
    }
}
