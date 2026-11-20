//! Writing a viewer tab back to disk, and creating new files.

use std::ffi::CString;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{self, Write};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::{MetadataExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// Whether this process may write the file itself. Folder permissions don't
/// count: they'd let a rename replace a read-only file.
pub fn writable(path: &Path) -> bool {
    let Ok(c) = CString::new(resolve(path).as_os_str().as_bytes()) else { return false };
    // SAFETY: a valid NUL-terminated path; access() only reads it.
    unsafe { libc::access(c.as_ptr(), libc::W_OK) == 0 }
}

/// Writes `text` to `path`, and returns the file's new modified time.
///
/// Usually the text goes to a temporary file beside the target (readable only
/// by its owner while it fills), is flushed to disk, takes the old file's
/// permissions, ACLs and extended attributes, then is renamed over it, so a
/// crash never leaves a half-written file.
///
/// A rename can't keep everything, so these are written in place instead:
/// a file with more than one hard link (the rename would split it from its
/// other names), a file someone else owns (it would become ours), and a file
/// in a folder we can't write. A read-only file is refused. A symlink is
/// followed, even one whose target is gone, so the link stays a link.
pub fn write(path: &Path, text: &str) -> io::Result<Option<SystemTime>> {
    let target = resolve(path);
    let old = fs::metadata(&target).ok();
    if let Some(old) = &old {
        if !writable(&target) {
            return Err(io::Error::new(io::ErrorKind::PermissionDenied, "the file is read-only"));
        }
        // SAFETY: geteuid has no preconditions.
        if old.nlink() > 1 || old.uid() != unsafe { libc::geteuid() } {
            return write_in_place(&target, text);
        }
    }
    match write_and_swap(&target, text, old.as_ref()) {
        Err(e) if e.kind() == io::ErrorKind::PermissionDenied && old.is_some() => write_in_place(&target, text),
        result => result,
    }
}

fn write_and_swap(target: &Path, text: &str, old: Option<&Metadata>) -> io::Result<Option<SystemTime>> {
    let temp = temp_path(target);
    // Owner-only while it holds the text, when the file it replaces exists;
    // a new file starts with the usual permissions, which it keeps anyway.
    let mode = if old.is_some() { 0o600 } else { 0o666 };
    // Fails if anything is already at that name; it isn't ours to remove.
    let mut file = OpenOptions::new().write(true).create_new(true).mode(mode).open(&temp)?;
    let result = (|| {
        file.write_all(text.as_bytes())?;
        file.sync_all()?;
        if let Some(old) = old {
            copy_acl_and_xattrs(target, &file)?;
            file.set_permissions(old.permissions())?;
        }
        // A rename keeps the modified time, so this is the time the target ends up with.
        let written = file.metadata()?.modified().ok();
        fs::rename(&temp, target)?;
        // The rename is durable once the folder is flushed too.
        if let Some(dir) = target.parent() {
            File::open(dir).and_then(|d| d.sync_all()).ok();
        }
        Ok(written)
    })();
    if result.is_err() {
        fs::remove_file(&temp).ok();
    }
    result
}

/// Overwrites the file where it is: keeps its owner, links and attributes,
/// but a crash midway can leave it partly written.
fn write_in_place(target: &Path, text: &str) -> io::Result<Option<SystemTime>> {
    let mut file = OpenOptions::new().write(true).open(target)?;
    file.write_all(text.as_bytes())?;
    file.set_len(text.len() as u64)?;
    file.sync_all()?;
    Ok(file.metadata()?.modified().ok())
}

/// ACLs and extended attributes (Finder tags, quarantine) from the old file.
/// Not the timestamps: the file did change.
#[cfg(target_os = "macos")]
fn copy_acl_and_xattrs(from: &Path, to: &File) -> io::Result<()> {
    use std::os::fd::AsRawFd;
    let source = File::open(from)?;
    // SAFETY: two open descriptors and no copyfile state.
    let status = unsafe { libc::fcopyfile(source.as_raw_fd(), to.as_raw_fd(), std::ptr::null_mut(), libc::COPYFILE_ACL | libc::COPYFILE_XATTR) };
    if status < 0 { Err(io::Error::last_os_error()) } else { Ok(()) }
}

#[cfg(not(target_os = "macos"))]
fn copy_acl_and_xattrs(_: &Path, _: &File) -> io::Result<()> {
    Ok(())
}

/// The file a path names: symlinks followed, and for a link whose target is
/// gone, that target's path, so saving recreates it instead of the link.
fn resolve(path: &Path) -> PathBuf {
    if let Ok(real) = fs::canonicalize(path) {
        return real;
    }
    match fs::read_link(path) {
        Ok(target) => path.parent().map(|dir| dir.join(&target)).unwrap_or(target),
        Err(_) => path.to_path_buf(),
    }
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
    fn read_only_files_are_refused_not_replaced() {
        let dir = dir("ro");
        let path = dir.join("schema.lock");
        fs::write(&path, "locked").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o444)).unwrap();
        assert!(!writable(&path));
        let err = write(&path, "changed").unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::PermissionDenied);
        assert_eq!(fs::read_to_string(&path).unwrap(), "locked");
        assert_eq!(fs::read_dir(&dir).unwrap().count(), 1);
        fs::set_permissions(&path, fs::Permissions::from_mode(0o644)).unwrap();
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn private_files_stay_private_and_the_mtime_is_reported() {
        let dir = dir("secret");
        let path = dir.join(".env");
        fs::write(&path, "TOKEN=old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600)).unwrap();
        let written = write(&path, "TOKEN=new").unwrap();
        assert_eq!(fs::metadata(&path).unwrap().permissions().mode() & 0o777, 0o600);
        assert_eq!(written, fs::metadata(&path).unwrap().modified().ok());
        fs::remove_dir_all(&dir).unwrap();
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn extended_attributes_survive_a_save() {
        let dir = dir("xattr");
        let path = dir.join("tagged.txt");
        fs::write(&path, "old").unwrap();
        let set = std::process::Command::new("xattr").args(["-w", "com.example.oxt", "kept"]).arg(&path).status().unwrap();
        assert!(set.success());
        write(&path, "new").unwrap();
        let out = std::process::Command::new("xattr").args(["-p", "com.example.oxt"]).arg(&path).output().unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout).trim(), "kept");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_dangling_symlink_stays_a_link() {
        let dir = dir("dangling");
        let link = dir.join("notes.md");
        std::os::unix::fs::symlink("moved/notes.md", &link).unwrap();
        fs::create_dir(dir.join("moved")).unwrap();
        write(&link, "hi").unwrap();
        assert!(fs::symlink_metadata(&link).unwrap().file_type().is_symlink());
        assert_eq!(fs::read_to_string(dir.join("moved/notes.md")).unwrap(), "hi");
        fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn a_file_already_at_the_temp_name_is_left_alone() {
        let dir = dir("planted");
        let path = dir.join("a.txt");
        fs::write(&path, "old").unwrap();
        let planted = temp_path(&fs::canonicalize(&path).unwrap());
        fs::write(&planted, "not ours").unwrap();
        assert!(write(&path, "new").is_err());
        assert_eq!(fs::read_to_string(&planted).unwrap(), "not ours");
        assert_eq!(fs::read_to_string(&path).unwrap(), "old");
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
