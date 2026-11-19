//! Writing a viewer tab back to disk, and creating new files.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::MetadataExt;
use std::path::{Path, PathBuf};

/// Writes `text` to `path` so a crash never leaves a half-written file.
///
/// The text goes to a temporary file beside the target, is flushed to disk,
/// takes the old file's permissions, then is renamed over it. A symlink is
/// followed, so the link stays a link and its target gets the new text. A file
/// with more than one hard link is written in place instead, because a rename
/// would split it from its other names.
pub fn write(path: &Path, text: &str) -> io::Result<()> {
    let target = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let old = fs::metadata(&target).ok();
    if old.as_ref().is_some_and(|m| m.nlink() > 1) {
        let mut file = OpenOptions::new().write(true).truncate(true).open(&target)?;
        file.write_all(text.as_bytes())?;
        return file.sync_all();
    }
    let temp = temp_path(&target);
    let result = (|| {
        let mut file = OpenOptions::new().write(true).create_new(true).open(&temp)?;
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if let Some(old) = &old {
            fs::set_permissions(&temp, old.permissions())?;
        }
        fs::rename(&temp, &target)?;
        // The rename is durable once the folder is flushed too.
        if let Some(dir) = target.parent() {
            File::open(dir).and_then(|d| d.sync_all()).ok();
        }
        Ok(())
    })();
    if result.is_err() {
        fs::remove_file(&temp).ok();
    }
    result
}

/// Creates an empty file, never touching one that's already there.
/// Returns whether it was created.
pub fn create(path: &Path) -> io::Result<bool> {
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(_) => Ok(true),
        Err(e) if e.kind() == io::ErrorKind::AlreadyExists => Ok(false),
        Err(e) => Err(e),
    }
}

/// `.name.0xterm-<pid>.tmp` beside the target: hidden, and on the same disk
/// so the rename is atomic.
fn temp_path(target: &Path) -> PathBuf {
    let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    target.with_file_name(format!(".{name}.0xterm-{}.tmp", std::process::id()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn dir(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("oxt-save-{name}-{}", std::process::id()));
        fs::remove_dir_all(&dir).ok();
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn replaces_text_and_keeps_permissions() {
        let dir = dir("perm");
        let path = dir.join("run.sh");
        fs::write(&path, "echo old\n").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
        write(&path, "echo new\r\n").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "echo new\r\n");
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o755);
        // No temporary file left behind.
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn symlinks_stay_links() {
        let dir = dir("link");
        let real = dir.join("real.txt");
        let link = dir.join("link.txt");
        fs::write(&real, "a").unwrap();
        std::os::unix::fs::symlink(&real, &link).unwrap();
        write(&link, "b").unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(&real).unwrap(), "b");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn hard_links_share_the_new_text() {
        let dir = dir("hard");
        let a = dir.join("a.txt");
        let b = dir.join("b.txt");
        fs::write(&a, "old").unwrap();
        fs::hard_link(&a, &b).unwrap();
        write(&a, "new").unwrap();
        assert_eq!(fs::read_to_string(&b).unwrap(), "new");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn create_never_clobbers() {
        let dir = dir("create");
        let path = dir.join("notes.md");
        assert!(create(&path).unwrap());
        fs::write(&path, "keep me").unwrap();
        assert!(!create(&path).unwrap());
        assert_eq!(fs::read_to_string(&path).unwrap(), "keep me");
        fs::remove_dir_all(&dir).unwrap();
    }
}
