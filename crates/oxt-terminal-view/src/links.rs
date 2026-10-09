//! ⌘-click targets in terminal text: URLs, and file paths with an optional
//! `:line[:col]`, the way compilers and grep print them.

use std::path::{Path, PathBuf};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    Url(String),
    File { path: PathBuf, line: Option<u64> },
}

/// Characters that can't be part of a path or URL token.
fn is_break(c: char) -> bool {
    c.is_whitespace() || matches!(c, '"' | '\'' | '`' | '<' | '>' | '|' | '(' | ')' | '[' | ']' | '{' | '}' | ',' | ';')
}

/// The token under `col` (a char index into `line`), trimmed of trailing punctuation.
fn token_at(line: &str, col: usize) -> Option<&str> {
    let chars: Vec<(usize, char)> = line.char_indices().collect();
    if col >= chars.len() || is_break(chars[col].1) {
        return None;
    }
    let mut start = col;
    while start > 0 && !is_break(chars[start - 1].1) {
        start -= 1;
    }
    let mut end = col;
    while end + 1 < chars.len() && !is_break(chars[end + 1].1) {
        end += 1;
    }
    let from = chars[start].0;
    let to = chars.get(end + 1).map(|(i, _)| *i).unwrap_or(line.len());
    Some(line[from..to].trim_end_matches(['.', ':', '!', '?']))
}

/// Finds what was clicked. Paths resolve against `cwd` and must exist.
pub fn link_at(line: &str, col: usize, cwd: Option<&Path>, home: Option<&Path>) -> Option<Link> {
    let token = token_at(line, col)?;
    if let Some(i) = token.find("://") {
        let scheme = &token[..i];
        if matches!(scheme, "http" | "https" | "file" | "ftp") {
            return Some(Link::Url(token.to_owned()));
        }
    }

    // path, path:12, path:12:5 — try the longest form first.
    let mut parts = token.splitn(3, ':');
    let raw = parts.next()?;
    let line_no = parts.next().and_then(|n| n.parse::<u64>().ok());
    let raw = raw.strip_prefix("a/").or_else(|| raw.strip_prefix("b/")).filter(|_| !Path::new(raw).exists()).unwrap_or(raw);
    let path = if let Some(rest) = raw.strip_prefix("~/") {
        home?.join(rest)
    } else if Path::new(raw).is_absolute() {
        PathBuf::from(raw)
    } else {
        cwd?.join(raw)
    };
    path.is_file().then_some(Link::File { path, line: line_no })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urls() {
        let line = "see https://example.com/a?b=1. for more";
        assert_eq!(link_at(line, 8, None, None), Some(Link::Url("https://example.com/a?b=1".into())));
        assert_eq!(link_at(line, 0, None, None), None);
    }

    #[test]
    fn paths_with_lines() {
        let dir = std::env::temp_dir().join(format!("oxt-links-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src/main.rs"), "").unwrap();
        let file = dir.join("src/main.rs");

        let grep = "src/main.rs:12:    let x = 1;";
        assert_eq!(link_at(grep, 3, Some(&dir), None), Some(Link::File { path: file.clone(), line: Some(12) }));
        let rustc = "  --> src/main.rs:7:5";
        assert_eq!(link_at(rustc, 8, Some(&dir), None), Some(Link::File { path: file.clone(), line: Some(7) }));
        let diff = "+++ b/src/main.rs";
        assert_eq!(link_at(diff, 6, Some(&dir), None), Some(Link::File { path: file.clone(), line: None }));
        assert_eq!(link_at("missing.rs:3", 2, Some(&dir), None), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
