//! Listing one folder for the tree. Loaded lazily: a folder is read when it's opened.

use std::io;
use std::path::{Path, PathBuf};

/// Never worth showing.
const HIDDEN: &[&str] = &[".git", ".DS_Store", "__pycache__"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    pub path: PathBuf,
    pub is_dir: bool,
    /// Matched by a .gitignore. Shown, but dimmed.
    pub ignored: bool,
}

/// The folder's children: folders first, then files, each sorted by name the way Finder does.
///
/// Only `dir` itself is opened. Child folders are never opened just to list
/// this one: on macOS, opening ~/Desktop or ~/Documents asks the user for
/// permission, and a listing must not trigger that.
pub fn list(dir: &Path) -> io::Result<Vec<Entry>> {
    let rules = GitRules::for_dir(dir);
    let mut entries = Vec::new();
    for item in std::fs::read_dir(dir)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if HIDDEN.contains(&name.as_str()) {
            continue;
        }
        let path = item.path();
        // Follow symlinks so a link to a folder opens like a folder (stat, not open).
        let is_dir = std::fs::metadata(&path).map(|m| m.is_dir()).unwrap_or(false);
        let ignored = rules.is_ignored(&path, is_dir);
        entries.push(Entry { name, path, is_dir, ignored });
    }
    entries.sort_by(|a, b| b.is_dir.cmp(&a.is_dir).then_with(|| natural_cmp(&a.name, &b.name)));
    Ok(entries)
}

/// The .gitignore rules that apply in one folder: its own file and every
/// parent's up to the repository root, plus .git/info/exclude.
struct GitRules(Vec<ignore::gitignore::Gitignore>);

impl GitRules {
    fn for_dir(dir: &Path) -> Self {
        let Some(repo) = dir.ancestors().find(|a| a.join(".git").exists()) else { return GitRules(Vec::new()) };
        let mut rules = Vec::new();
        for folder in dir.ancestors().take_while(|a| a.starts_with(repo)) {
            let file = folder.join(".gitignore");
            if file.is_file() {
                let (gi, _) = ignore::gitignore::Gitignore::new(&file);
                rules.push(gi);
            }
        }
        let exclude = repo.join(".git/info/exclude");
        if exclude.is_file() {
            let mut builder = ignore::gitignore::GitignoreBuilder::new(repo);
            builder.add(&exclude);
            if let Ok(gi) = builder.build() {
                rules.push(gi);
            }
        }
        GitRules(rules)
    }

    /// Nearest folder's rules win, as in git.
    fn is_ignored(&self, path: &Path, is_dir: bool) -> bool {
        for rules in &self.0 {
            let m = rules.matched(path, is_dir);
            if m.is_ignore() {
                return true;
            }
            if m.is_whitelist() {
                return false;
            }
        }
        false
    }
}

/// Case-insensitive, with digit runs compared as numbers: file2 before file10.
pub fn natural_cmp(a: &str, b: &str) -> std::cmp::Ordering {
    let (mut a, mut b) = (a.chars().peekable(), b.chars().peekable());
    loop {
        match (a.peek().copied(), b.peek().copied()) {
            (None, None) => return std::cmp::Ordering::Equal,
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let take = |it: &mut std::iter::Peekable<std::str::Chars>| {
                    let mut n = String::new();
                    while let Some(c) = it.peek().copied().filter(char::is_ascii_digit) {
                        n.push(c);
                        it.next();
                    }
                    n
                };
                let (na, nb) = (take(&mut a), take(&mut b));
                let (ta, tb) = (na.trim_start_matches('0'), nb.trim_start_matches('0'));
                let ord = ta.len().cmp(&tb.len()).then_with(|| ta.cmp(tb));
                if ord.is_ne() {
                    return ord;
                }
            }
            (Some(x), Some(y)) => {
                let ord = x.to_lowercase().cmp(y.to_lowercase());
                if ord.is_ne() {
                    return ord;
                }
                a.next();
                b.next();
            }
        }
    }
}

/// Breadcrumb parts for the tree header: `~` for home, `/` for the root.
/// Long paths keep the first part and the last `keep` parts, with `…` between.
pub fn crumbs(dir: &Path, home: Option<&Path>, keep: usize) -> Vec<(String, PathBuf)> {
    let (mut parts, rest) = match home.and_then(|h| dir.strip_prefix(h).ok().map(|rest| (h, rest))) {
        Some((home, rest)) => (vec![("~".to_owned(), home.to_path_buf())], rest),
        None => (vec![("/".to_owned(), PathBuf::from("/"))], dir.strip_prefix("/").unwrap_or(dir)),
    };
    let mut path = parts[0].1.clone();
    for part in rest.components() {
        path.push(part);
        parts.push((part.as_os_str().to_string_lossy().into_owned(), path.clone()));
    }
    if parts.len() > keep + 2 {
        let tail = parts.split_off(parts.len() - keep);
        parts.truncate(1);
        parts.push(("…".to_owned(), tail[0].1.parent().unwrap_or(&tail[0].1).to_path_buf()));
        parts.extend(tail);
    }
    parts
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn natural_order() {
        let mut names = vec!["file10", "File2", "file1", "a"];
        names.sort_by(|a, b| natural_cmp(a, b));
        assert_eq!(names, ["a", "file1", "File2", "file10"]);
    }

    #[test]
    fn folders_first_hidden_skipped_ignored_marked() {
        let root = std::env::temp_dir().join(format!("oxt-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join(".git")).unwrap();
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("target")).unwrap();
        for f in ["b.txt", "a.txt", ".env", ".DS_Store", ".gitignore"] {
            std::fs::write(root.join(f), "").unwrap();
        }
        std::fs::write(root.join(".gitignore"), "target/\n").unwrap();

        let entries = list(&root).unwrap();
        let names: Vec<_> = entries.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["src", "target", ".env", ".gitignore", "a.txt", "b.txt"]);
        assert!(entries.iter().find(|e| e.name == "target").unwrap().ignored);
        assert!(!entries.iter().find(|e| e.name == "src").unwrap().ignored);
        std::fs::remove_dir_all(&root).unwrap();
    }

    #[test]
    fn breadcrumbs() {
        let home = Path::new("/Users/me");
        let names = |c: Vec<(String, PathBuf)>| c.into_iter().map(|(n, _)| n).collect::<Vec<_>>();
        assert_eq!(names(crumbs(Path::new("/Users/me/code/x"), Some(home), 3)), ["~", "code", "x"]);
        assert_eq!(names(crumbs(Path::new("/usr/bin"), Some(home), 3)), ["/", "usr", "bin"]);
        assert_eq!(names(crumbs(Path::new("/Users/me/a/b/c/d/e"), Some(home), 2)), ["~", "…", "d", "e"]);
        assert_eq!(crumbs(Path::new("/Users/me/a/b/c/d/e"), Some(home), 2)[1].1, Path::new("/Users/me/a/b/c"));
    }
}
