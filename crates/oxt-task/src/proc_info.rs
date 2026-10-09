//! Asking the macOS kernel about a process: its directory and its name.
//! `proc_pidinfo` is one syscall, where `lsof` would start a whole process.

use std::ffi::{CStr, OsStr};
use std::mem;
use std::os::unix::ffi::OsStrExt;
use std::path::PathBuf;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Process {
    pub pid: i32,
    pub name: String,
}

pub(crate) fn cwd(pid: libc::pid_t) -> Option<PathBuf> {
    let info: libc::proc_vnodepathinfo = pidinfo(pid, libc::PROC_PIDVNODEPATHINFO)?;
    // vip_path is a MAXPATHLEN char array, declared as [[c_char; 32]; 32].
    let raw = &info.pvi_cdir.vip_path;
    let bytes = unsafe { std::slice::from_raw_parts(raw.as_ptr().cast::<u8>(), mem::size_of_val(raw)) };
    let path = CStr::from_bytes_until_nul(bytes).ok()?.to_bytes();
    (!path.is_empty()).then(|| PathBuf::from(OsStr::from_bytes(path)))
}

pub(crate) fn process(pid: libc::pid_t) -> Option<Process> {
    let info: libc::proc_bsdinfo = pidinfo(pid, libc::PROC_PIDTBSDINFO)?;
    // pbi_name holds up to 32 chars; pbi_comm is the 16-char fallback.
    let name = c_chars(&info.pbi_name).filter(|n| !n.is_empty()).or_else(|| c_chars(&info.pbi_comm))?;
    Some(Process { pid, name })
}

fn pidinfo<T>(pid: libc::pid_t, flavor: libc::c_int) -> Option<T> {
    let mut info: T = unsafe { mem::zeroed() };
    let size = mem::size_of::<T>() as libc::c_int;
    let n = unsafe { libc::proc_pidinfo(pid, flavor, 0, (&raw mut info).cast(), size) };
    (n == size).then_some(info)
}

fn c_chars(chars: &[libc::c_char]) -> Option<String> {
    let bytes = unsafe { std::slice::from_raw_parts(chars.as_ptr().cast::<u8>(), chars.len()) };
    Some(CStr::from_bytes_until_nul(bytes).ok()?.to_string_lossy().into_owned())
}
