//! Folder search: ripgrep's engine as a library. Case-insensitive literal text,
//! .gitignore respected, hidden files and binaries skipped, like `rg -iF`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};

use grep_regex::RegexMatcherBuilder;
use grep_searcher::sinks::UTF8;
use grep_searcher::{BinaryDetection, SearcherBuilder};
use ignore::WalkState;

pub const MAX_HITS: usize = 400;
const MAX_FILE_SIZE: u64 = 2 * 1024 * 1024;
const MAX_LINE: usize = 300;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Hit {
    /// 1-based.
    pub line: u64,
    /// The line, trimmed and cut to a readable length.
    pub text: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileHits {
    pub path: PathBuf,
    pub hits: Vec<Hit>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Results {
    /// Sorted by path.
    pub files: Vec<FileHits>,
    pub total: usize,
    /// Stopped at [`MAX_HITS`].
    pub truncated: bool,
}

/// Searches `root` for `query`. Runs on every core; returns early if `cancel`
/// is set (the user typed again). Call it off the UI thread.
pub fn search(root: &Path, query: &str, cancel: &AtomicBool) -> Results {
    if query.trim().is_empty() {
        return Results::default();
    }
    let matcher = RegexMatcherBuilder::new().case_insensitive(true).build(&regex_syntax::escape(query)).expect("an escaped literal is a valid regex");

    let found = Mutex::new(Vec::<FileHits>::new());
    let total = AtomicUsize::new(0);

    ignore::WalkBuilder::new(root).max_filesize(Some(MAX_FILE_SIZE)).build_parallel().run(|| {
        let matcher = matcher.clone();
        let mut searcher = SearcherBuilder::new().binary_detection(BinaryDetection::quit(0)).line_number(true).build();
        let (found, total) = (&found, &total);
        Box::new(move |entry| {
            if cancel.load(Ordering::Relaxed) || total.load(Ordering::Relaxed) >= MAX_HITS {
                return WalkState::Quit;
            }
            let Ok(entry) = entry else { return WalkState::Continue };
            if !entry.file_type().is_some_and(|t| t.is_file()) {
                return WalkState::Continue;
            }
            let mut hits = Vec::new();
            let _ = searcher.search_path(
                &matcher,
                entry.path(),
                UTF8(|line, text| {
                    if total.fetch_add(1, Ordering::Relaxed) >= MAX_HITS {
                        return Ok(false);
                    }
                    hits.push(Hit { line, text: shorten(text) });
                    Ok(true)
                }),
            );
            if !hits.is_empty() {
                found.lock().unwrap().push(FileHits { path: entry.into_path(), hits });
            }
            WalkState::Continue
        })
    });

    let mut files = found.into_inner().unwrap();
    files.sort_by(|a, b| a.path.cmp(&b.path));
    let total_hits = files.iter().map(|f| f.hits.len()).sum();
    Results { files, total: total_hits, truncated: total.load(Ordering::Relaxed) >= MAX_HITS }
}

fn shorten(line: &str) -> String {
    let line = line.trim();
    match line.char_indices().nth(MAX_LINE) {
        Some((cut, _)) => format!("{}…", &line[..cut]),
        None => line.to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_grouped_case_insensitive_and_skips_ignored() {
        let root = std::env::temp_dir().join(format!("oxt-search-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();
        std::fs::write(root.join("src/a.rs"), "fn main() {}\nlet pty = Pty::open();\n// PTY again\n").unwrap();
        std::fs::write(root.join("src/b.rs"), "nothing here\n").unwrap();
        std::fs::write(root.join("target/c.rs"), "pty in build output\n").unwrap();
        std::fs::write(root.join("bin.dat"), b"pty\0binary").unwrap();
        std::fs::write(root.join("regex.txt"), "a.b (pty)\n").unwrap();

        let results = search(&root, "pty", &AtomicBool::new(false));
        let files: Vec<_> = results.files.iter().map(|f| f.path.strip_prefix(&root).unwrap().to_str().unwrap()).collect();
        assert_eq!(files, ["regex.txt", "src/a.rs"]);
        assert_eq!(results.files[1].hits.iter().map(|h| h.line).collect::<Vec<_>>(), [2, 3]);
        assert_eq!(results.total, 3);

        // Query text is literal, not a regex.
        assert_eq!(search(&root, "(pty)", &AtomicBool::new(false)).total, 1);
        assert_eq!(search(&root, "a.b", &AtomicBool::new(false)).total, 1);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn stops_at_the_limit() {
        let root = std::env::temp_dir().join(format!("oxt-search-limit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("many.txt"), "hit\n".repeat(1000)).unwrap();
        let results = search(&root, "hit", &AtomicBool::new(false));
        assert_eq!(results.total, MAX_HITS);
        assert!(results.truncated);
        std::fs::remove_dir_all(&root).unwrap();
    }
}
