//! The reader thread: blocks on the master fd and hands every chunk to the session.
//! When the slave side closes, it reaps the child and reports its exit status.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::os::unix::process::ExitStatusExt;
use std::process::ExitStatus;
use std::sync::Arc;
use std::thread;

const BUFFER_SIZE: usize = 64 * 1024;

pub(crate) fn spawn(
    master: Arc<OwnedFd>,
    pid: libc::pid_t,
    mut on_output: impl FnMut(&[u8]) + Send + 'static,
    on_exit: impl FnOnce(ExitStatus) + Send + 'static,
) -> io::Result<()> {
    thread::Builder::new().name(format!("pty-reader-{pid}")).spawn(move || {
        let mut buf = vec![0u8; BUFFER_SIZE];
        loop {
            let n = unsafe { libc::read(master.as_raw_fd(), buf.as_mut_ptr().cast(), buf.len()) };
            match n {
                n if n > 0 => on_output(&buf[..n as usize]),
                // EIO is how macOS says "the slave side is gone".
                0 => break,
                _ if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted => continue,
                _ => break,
            }
        }
        on_exit(wait(pid));
    })?;
    Ok(())
}

fn wait(pid: libc::pid_t) -> ExitStatus {
    let mut status = 0;
    loop {
        let ret = unsafe { libc::waitpid(pid, &mut status, 0) };
        if ret == -1 && io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
            continue;
        }
        return ExitStatus::from_raw(status);
    }
}
