//! ⌘-click targets in terminal text: URLs, and file paths with an optional
//! `:line[:col]`, the way compilers and grep print them. Where a URL may go
//! is the window's call (`oxt-window`'s links), not the pane's.

use std::path::{Path, PathBuf};

use oxt_session::alacritty_terminal::grid::{Dimensions, Grid};
use oxt_session::alacritty_terminal::index::{Column, Line, Point};
use oxt_session::alacritty_terminal::term::cell::{Cell, Flags};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Link {
    /// A URL printed in the output.
    Url(String),
    /// An OSC 8 hyperlink: its URL, and the text it's drawn over.
    Hyperlink {
        uri: String,
        shown: String,
    },
    File {
        path: PathBuf,
        line: Option<u64>,
    },
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

/// The OSC 8 hyperlink at `point`, with the text it's drawn over: the run of
/// cells around it with the same link, across rows the text wrapped onto.
pub fn hyperlink_at(grid: &Grid<Cell>, point: Point) -> Option<Link> {
    let link = grid[point].hyperlink()?;
    let last = grid.last_column();
    let wraps = |line: Line| grid[line][last].flags.contains(Flags::WRAPLINE);
    let before = |p: Point| match p.column.0 {
        0 if p.line > grid.topmost_line() && wraps(p.line - 1) => Some(Point::new(p.line - 1, last)),
        0 => None,
        c => Some(Point::new(p.line, Column(c - 1))),
    };
    let after = |p: Point| match p.column {
        c if c < last => Some(Point::new(p.line, c + 1)),
        _ if p.line < grid.bottommost_line() && wraps(p.line) => Some(Point::new(p.line + 1, Column(0))),
        _ => None,
    };
    let same = |p: &Point| grid[*p].hyperlink().as_ref() == Some(&link);
    let mut start = point;
    while let Some(p) = before(start).filter(same) {
        start = p;
    }
    let mut shown = String::new();
    let mut at = Some(start);
    while let Some(p) = at.filter(same) {
        if !grid[p].flags.intersects(Flags::WIDE_CHAR_SPACER | Flags::LEADING_WIDE_CHAR_SPACER) {
            shown.push(grid[p].c);
        }
        at = after(p);
    }
    Some(Link::Hyperlink { uri: link.uri().to_owned(), shown })
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
    fn printed_urls_go_to_the_window_as_written() {
        // `FILE:` and `ftp:` too: routing them is the window's job.
        for url in ["file:///tmp/run.command", "ftp://example.com/a"] {
            assert_eq!(link_at(&format!("see {url} here"), 6, None, None), Some(Link::Url(url.into())));
        }
    }

    #[test]
    fn hyperlink_text_follows_a_wrap() {
        let (tx, _rx) = async_channel::unbounded();
        let mut engine = oxt_session::Engine::new(oxt_session::GridSize { rows: 3, cols: 10 }, 0, oxt_session::Listener::new(|_| {}, tx));
        engine.advance(b"\x1b]8;;https://evil.example\x1b\\github.com/org/repo\x1b]8;;\x1b\\ more");
        let expected = Some(Link::Hyperlink { uri: "https://evil.example".into(), shown: "github.com/org/repo".into() });
        for (line, col) in [(0, 3), (1, 2)] {
            assert_eq!(hyperlink_at(engine.term.grid(), Point::new(Line(line), Column(col))), expected, "clicked {line}:{col}");
        }
        assert_eq!(hyperlink_at(engine.term.grid(), Point::new(Line(1), Column(9))), None);
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
