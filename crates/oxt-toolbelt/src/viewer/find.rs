//! ⌘F inside the open file: every case-insensitive match, as line + byte range.

use std::ops::Range;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Match {
    /// 0-based line.
    pub line: usize,
    /// Byte range within that line.
    pub range: Range<usize>,
}

pub fn find_all(text: &str, query: &str) -> Vec<Match> {
    if query.is_empty() {
        return Vec::new();
    }
    let needle = query.to_lowercase();
    let mut matches = Vec::new();
    for (line_no, line) in text.lines().enumerate() {
        let lower = line.to_lowercase();
        // Lowercasing can change byte lengths outside ASCII; map back only when it didn't.
        if lower.len() != line.len() {
            continue_with_chars(line, &needle, line_no, &mut matches);
            continue;
        }
        let mut from = 0;
        while let Some(at) = lower[from..].find(&needle) {
            let start = from + at;
            matches.push(Match { line: line_no, range: start..start + needle.len() });
            from = start + needle.len().max(1);
        }
    }
    matches
}

/// Slow path for lines whose lowercase form has a different byte length.
fn continue_with_chars(line: &str, needle: &str, line_no: usize, out: &mut Vec<Match>) {
    let starts: Vec<usize> = line.char_indices().map(|(i, _)| i).collect();
    for &start in &starts {
        let mut lowered = String::new();
        for (offset, c) in line[start..].char_indices() {
            lowered.extend(c.to_lowercase());
            if lowered.len() >= needle.len() {
                if lowered == needle {
                    out.push(Match { line: line_no, range: start..start + offset + c.len_utf8() });
                }
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_every_match() {
        let m = find_all("Pty pty\nnone\nxPTY", "pty");
        assert_eq!(m.iter().map(|m| (m.line, m.range.clone())).collect::<Vec<_>>(), [(0, 0..3), (0, 4..7), (2, 1..4)]);
        assert!(find_all("abc", "").is_empty());
    }

    #[test]
    fn unicode_lines() {
        let m = find_all("İstanbul pty", "pty");
        assert_eq!(m.len(), 1);
        let line = "İstanbul pty";
        assert_eq!(&line[m[0].range.clone()], "pty");
    }
}
