//! What the panels ask the window to do. The panels never touch panes directly.

use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq)]
pub struct OpenRequest {
    pub path: PathBuf,
    /// 1-based line to scroll to.
    pub line: Option<u64>,
    /// Text to fill into the viewer's find box.
    pub find: Option<String>,
}

impl OpenRequest {
    pub fn path(path: impl Into<PathBuf>) -> Self {
        OpenRequest { path: path.into(), line: None, find: None }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ToolbeltEvent {
    Open(OpenRequest),
    /// Type this path into the active pane.
    InsertPath(PathBuf),
    /// `cd` the active pane here.
    CdHere(PathBuf),
    NewTabHere(PathBuf),
    /// A URL clicked in a Markdown preview. The window decides where it may go.
    OpenUrl(String),
}

/// A path being dragged from the tree. Panes accept it and type the path.
#[derive(Clone, Debug)]
pub struct DraggedPath(pub PathBuf);

/// The quoting a shell reads back. Only names with control characters differ.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shell {
    /// zsh, bash, sh, ksh.
    #[default]
    Posix,
    Fish,
}

impl Shell {
    /// The shell a process name (`zsh`, `-fish`) belongs to.
    pub fn from_process(name: &str) -> Self {
        if name.trim_start_matches('-') == "fish" { Shell::Fish } else { Shell::Posix }
    }
}

/// A path quoted for the shell, only when it needs it: `/tmp/a` or `'/tmp/a b'`.
pub fn shell_quote(path: &std::path::Path, shell: Shell) -> String {
    let s = path.to_string_lossy();
    let safe = |c: char| c.is_ascii_alphanumeric() || "/._-+~,@%:".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return s.into_owned();
    }
    if !s.chars().any(char::is_control) {
        return format!("'{}'", s.replace('\'', r"'\''"));
    }
    // A control character typed raw is a keystroke (Ctrl-U, Enter), not part of
    // the name, so it goes as an escape the shell reads back.
    let mut out = String::new();
    match shell {
        // `$'…'`, which zsh, bash, sh and ksh read. Three-digit octal, because
        // ksh's `\x` eats every hex digit after it; quotes as octal too, so a
        // shell without `$'…'` (dash) sees one plain string.
        Shell::Posix => {
            out.push_str("$'");
            for c in s.chars() {
                if c.is_control() || c == '\'' || c == '\\' {
                    c.encode_utf8(&mut [0; 4]).bytes().for_each(|b| out.push_str(&format!("\\{b:03o}")));
                } else {
                    out.push(c);
                }
            }
            out.push('\'');
        }
        // fish has no `$'…'`: quoted runs, with `\uXXXX` between them.
        Shell::Fish => {
            out.push('\'');
            for c in s.chars() {
                match c {
                    '\'' | '\\' => out.extend(['\\', c]),
                    c if c.is_control() => out.push_str(&format!("'\\u{:04x}'", c as u32)),
                    c => out.push(c),
                }
            }
            out.push('\'');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("/tmp/a.txt".as_ref(), Shell::Posix), "/tmp/a.txt");
        assert_eq!(shell_quote("/tmp/a b".as_ref(), Shell::Posix), "'/tmp/a b'");
        assert_eq!(shell_quote("/tmp/it's".as_ref(), Shell::Posix), r"'/tmp/it'\''s'");
    }

    #[test]
    fn control_characters_never_reach_the_shell_raw() {
        for shell in [Shell::Posix, Shell::Fish] {
            let quoted = shell_quote("/tmp/docs\x15cd;touch pwned\n".as_ref(), shell);
            assert!(!quoted.chars().any(char::is_control), "{quoted:?}");
            assert!(!shell_quote("/tmp/a\u{9b}b".as_ref(), shell).chars().any(char::is_control));
        }
        assert_eq!(shell_quote("/tmp/it's\x15".as_ref(), Shell::Posix), r"$'/tmp/it\047s\025'");
        assert_eq!(shell_quote("/tmp/it's\x15".as_ref(), Shell::Fish), r"'/tmp/it\'s'\u0015''");
    }

    #[test]
    fn shell_from_process_name() {
        assert_eq!(Shell::from_process("-fish"), Shell::Fish);
        assert_eq!(Shell::from_process("zsh"), Shell::Posix);
    }
}
