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

/// The quoting a shell reads back.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Shell {
    /// zsh, bash, sh, ksh, dash, and anything that isn't a known shell: a
    /// program like vim, or ssh to what is most likely one of these.
    #[default]
    Posix,
    Fish,
    /// Shells whose quoting differs (nushell runs `(…)` inside `$'…'`, tcsh
    /// expands `!` inside `'…'`, PowerShell ends `'…'` on curly quotes), so
    /// only names with nothing to escape are typed into them.
    Other,
}

impl Shell {
    /// The shell a process name (`zsh`, `-fish`) belongs to.
    pub fn from_process(name: &str) -> Self {
        match name.trim_start_matches('-') {
            "fish" => Shell::Fish,
            "nu" | "tcsh" | "csh" | "pwsh" | "powershell" | "xonsh" | "elvish" | "ion" | "murex" => Shell::Other,
            _ => Shell::Posix,
        }
    }
}

/// A path quoted for the shell, only when it needs it: `/tmp/a` or `'/tmp/a b'`.
/// `None` when this shell can't be given the name safely.
pub fn shell_quote(path: &std::path::Path, shell: Shell) -> Option<String> {
    let s = path.to_string_lossy();
    let safe = |c: char| c.is_ascii_alphanumeric() || "/._-+~,@%:".contains(c);
    if !s.is_empty() && s.chars().all(safe) {
        return Some(s.into_owned());
    }
    // A control character typed raw is a keystroke (Ctrl-U, Enter), not part of
    // the name, so it goes as an escape the shell reads back.
    let mut out = String::new();
    match shell {
        Shell::Posix if !s.chars().any(char::is_control) => out = format!("'{}'", s.replace('\'', r"'\''")),
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
        // In fish, `\'` and `\\` are escapes even inside single quotes, so
        // every name gets fish's own form: quoted runs, `\uXXXX` between them.
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
        Shell::Other if s.chars().any(|c| c.is_control() || matches!(c, '\'' | '\\' | '!' | '\u{2018}'..='\u{201b}')) => return None,
        Shell::Other => out = format!("'{s}'"),
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("/tmp/a.txt".as_ref(), Shell::Posix).unwrap(), "/tmp/a.txt");
        assert_eq!(shell_quote("/tmp/a b".as_ref(), Shell::Posix).unwrap(), "'/tmp/a b'");
        assert_eq!(shell_quote("/tmp/it's".as_ref(), Shell::Posix).unwrap(), r"'/tmp/it'\''s'");
    }

    #[test]
    fn control_characters_never_reach_the_shell_raw() {
        for shell in [Shell::Posix, Shell::Fish] {
            let quoted = shell_quote("/tmp/docs\x15cd;touch pwned\n".as_ref(), shell).unwrap();
            assert!(!quoted.chars().any(char::is_control), "{quoted:?}");
            assert!(!shell_quote("/tmp/a\u{9b}b".as_ref(), shell).unwrap().chars().any(char::is_control));
        }
        assert_eq!(shell_quote("/tmp/it's\x15".as_ref(), Shell::Posix).unwrap(), r"$'/tmp/it\047s\025'");
        assert_eq!(shell_quote("/tmp/it's\x15".as_ref(), Shell::Fish).unwrap(), r"'/tmp/it\'s'\u0015''");
    }

    #[test]
    fn fish_gets_its_own_quoting_for_every_name() {
        assert_eq!(shell_quote(r"/tmp/x\';touch pwned;#".as_ref(), Shell::Fish).unwrap(), r"'/tmp/x\\\';touch pwned;#'");
        assert_eq!(shell_quote("/tmp/a b".as_ref(), Shell::Fish).unwrap(), "'/tmp/a b'");
    }

    #[test]
    fn other_shells_only_get_names_with_nothing_to_escape() {
        assert_eq!(shell_quote("/tmp/a b (1)".as_ref(), Shell::Other).unwrap(), "'/tmp/a b (1)'");
        // PowerShell also closes a single quote on ‘ ’ ‚ ‛.
        for name in ["/tmp/a\x01(open -a Calculator)", "/tmp/it's", r"/tmp/a\b", "/tmp/a!b", "/tmp/a’; touch pwned; ‘", "/tmp/a‛b"] {
            assert_eq!(shell_quote(name.as_ref(), Shell::Other), None, "{name:?}");
        }
    }

    #[test]
    fn shell_from_process_name() {
        assert_eq!(Shell::from_process("-fish"), Shell::Fish);
        assert_eq!(Shell::from_process("zsh"), Shell::Posix);
        assert_eq!(Shell::from_process("nu"), Shell::Other);
        assert_eq!(Shell::from_process("tcsh"), Shell::Other);
        assert_eq!(Shell::from_process("pwsh"), Shell::Other);
        // Programs that aren't shells (vim, ssh to a remote zsh) get the usual quoting.
        for name in ["vim", "ssh", "node", "claude"] {
            assert_eq!(Shell::from_process(name), Shell::Posix, "{name}");
        }
    }
}
