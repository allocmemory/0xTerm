//! Markdown preview: the text is gpui-component's Markdown view; each
//! ```mermaid fence is cut out and drawn as a diagram in its place.

/// A piece of a Markdown document.
#[derive(Clone, Debug, PartialEq)]
pub enum Segment {
    Markdown(String),
    Mermaid(String),
}

/// Splits on top-level ```mermaid fences. Anything else stays Markdown.
pub fn segments(source: &str) -> Vec<Segment> {
    let mut out = Vec::new();
    let mut text = String::new();
    let mut lines = source.split_inclusive('\n');
    while let Some(line) = lines.next() {
        let trimmed = line.trim_start();
        let fence = trimmed.chars().take_while(|c| *c == '`' || *c == '~').count();
        if fence >= 3 && trimmed[fence..].trim().eq_ignore_ascii_case("mermaid") {
            let marker = &trimmed[..fence];
            let mut diagram = String::new();
            for inner in lines.by_ref() {
                if inner.trim_start().starts_with(marker) && inner.trim().trim_start_matches(marker.chars().next().unwrap()).is_empty() {
                    break;
                }
                diagram.push_str(inner);
            }
            if !text.trim().is_empty() {
                out.push(Segment::Markdown(std::mem::take(&mut text)));
            }
            text.clear();
            out.push(Segment::Mermaid(diagram));
        } else {
            text.push_str(line);
        }
    }
    if !text.trim().is_empty() {
        out.push(Segment::Markdown(text));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cuts_out_mermaid_fences() {
        let md = "# Title\n\nintro\n\n```mermaid\nflowchart LR\n  a --> b\n```\n\nafter\n\n```rust\nfn x() {}\n```\n";
        let s = segments(md);
        assert_eq!(s.len(), 3);
        assert_eq!(s[1], Segment::Mermaid("flowchart LR\n  a --> b\n".into()));
        assert!(matches!(&s[2], Segment::Markdown(t) if t.contains("```rust")));
    }

    #[test]
    fn plain_markdown_is_one_segment() {
        assert_eq!(segments("just text\n").len(), 1);
    }
}
