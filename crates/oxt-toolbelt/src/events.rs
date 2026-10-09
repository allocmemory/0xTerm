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
}

/// A path being dragged from the tree. Panes accept it and type the path.
#[derive(Clone, Debug)]
pub struct DraggedPath(pub PathBuf);

/// A path quoted for the shell, only when it needs it: `/tmp/a` or `'/tmp/a b'`.
pub fn shell_quote(path: &std::path::Path) -> String {
    let s = path.to_string_lossy();
    let safe = |c: char| c.is_ascii_alphanumeric() || "/._-+~,@%:".contains(c);
    if !s.is_empty() && s.chars().all(safe) { s.into_owned() } else { format!("'{}'", s.replace('\'', r"'\''")) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting() {
        assert_eq!(shell_quote("/tmp/a.txt".as_ref()), "/tmp/a.txt");
        assert_eq!(shell_quote("/tmp/a b".as_ref()), "'/tmp/a b'");
        assert_eq!(shell_quote("/tmp/it's".as_ref()), r"'/tmp/it'\''s'");
    }
}
