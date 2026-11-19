//! Reading a file for the viewer and deciding how to show it.

use std::path::Path;

pub const MAX_SIZE: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Content {
    /// Source or plain text. `language` is a highlighter name, if one fits.
    Text {
        text: String,
        language: Option<&'static str>,
    },
    Markdown(String),
    Image,
    Binary,
    TooLarge(u64),
}

/// A file as read for the viewer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Loaded {
    pub content: Content,
    /// The bytes were valid UTF-8, so the text is exactly what's on disk.
    /// Lossy text must never be saved back: it would replace bytes with U+FFFD.
    pub exact: bool,
}

impl Loaded {
    /// Text the viewer may edit and write back without changing other bytes.
    pub fn editable(&self) -> bool {
        self.exact && matches!(self.content, Content::Text { .. } | Content::Markdown(_))
    }
}

pub fn load(path: &Path) -> std::io::Result<Content> {
    load_file(path).map(|loaded| loaded.content)
}

pub fn load_file(path: &Path) -> std::io::Result<Loaded> {
    let other = |content| Ok(Loaded { content, exact: false });
    let ext = path.extension().and_then(|e| e.to_str()).map(str::to_ascii_lowercase).unwrap_or_default();
    if matches!(ext.as_str(), "png" | "jpg" | "jpeg" | "gif" | "webp" | "bmp" | "ico" | "svg") {
        return other(Content::Image);
    }
    let size = std::fs::metadata(path)?.len();
    if size > MAX_SIZE {
        return other(Content::TooLarge(size));
    }
    let bytes = std::fs::read(path)?;
    if bytes[..bytes.len().min(8192)].contains(&0) {
        return other(Content::Binary);
    }
    let (text, exact) = match String::from_utf8(bytes) {
        Ok(text) => (text, true),
        Err(e) => (String::from_utf8_lossy(e.as_bytes()).into_owned(), false),
    };
    let content = if matches!(ext.as_str(), "md" | "markdown" | "mdx") {
        Content::Markdown(text)
    } else {
        Content::Text { text, language: language(path, &ext) }
    };
    Ok(Loaded { content, exact })
}

/// File → highlighter language. Names match gpui-component's highlighter.
fn language(path: &Path, ext: &str) -> Option<&'static str> {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    Some(match (name, ext) {
        ("Makefile" | "makefile" | "GNUmakefile", _) => "make",
        ("Dockerfile", _) => "dockerfile",
        (".zshrc" | ".bashrc" | ".zprofile" | ".bash_profile" | ".profile", _) => "bash",
        (_, "rs") => "rust",
        (_, "c" | "h") => "c",
        // No Objective-C grammar ships; C and C++ highlight it well enough.
        (_, "cc" | "cpp" | "cxx" | "hpp" | "hh" | "mm") => "cpp",
        (_, "m") => "c",
        (_, "swift") => "swift",
        (_, "go") => "go",
        (_, "py") => "python",
        (_, "rb") => "ruby",
        (_, "js" | "mjs" | "cjs") => "javascript",
        (_, "jsx") => "javascript",
        (_, "ts" | "mts" | "cts") => "typescript",
        (_, "tsx") => "tsx",
        (_, "json") => "json",
        (_, "toml") => "toml",
        (_, "yml" | "yaml") => "yaml",
        (_, "html" | "htm") => "html",
        (_, "css") => "css",
        (_, "sh" | "bash" | "zsh") => "bash",
        (_, "sql") => "sql",
        (_, "java") => "java",
        (_, "kt" | "kts") => "kotlin",
        (_, "cs") => "csharp",
        (_, "lua") => "lua",
        (_, "zig") => "zig",
        (_, "diff" | "patch") => "diff",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds() {
        let dir = std::env::temp_dir().join(format!("oxt-load-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let write = |name: &str, bytes: &[u8]| {
            let p = dir.join(name);
            std::fs::write(&p, bytes).unwrap();
            p
        };
        assert_eq!(load(&write("a.md", b"# hi")).unwrap(), Content::Markdown("# hi".into()));
        assert_eq!(load(&write("a.rs", b"fn x(){}")).unwrap(), Content::Text { text: "fn x(){}".into(), language: Some("rust") });
        assert_eq!(load(&write("a.bin", b"a\0b")).unwrap(), Content::Binary);
        assert_eq!(load(&write("a.png", b"")).unwrap(), Content::Image);
        assert_eq!(load(&write("notes", b"plain")).unwrap(), Content::Text { text: "plain".into(), language: None });
        assert!(load_file(&write("ok.rs", b"fn x(){}")).unwrap().editable());
        assert!(load_file(&write("ok.md", b"# hi")).unwrap().editable());
        // Latin-1: shown with U+FFFD, so saving it would corrupt the file.
        assert!(!load_file(&write("latin1.txt", b"caf\xe9")).unwrap().editable());
        assert!(!load_file(&write("b.png", b"")).unwrap().editable());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
