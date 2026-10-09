//! The writer thread. A shell that isn't reading lets the pty's input buffer
//! fill, and a write would then block; a big paste must never freeze the UI.

use std::io;
use std::os::fd::{AsRawFd, OwnedFd};
use std::sync::Arc;
use std::sync::mpsc::{self, Sender};
use std::thread;

/// Queues bytes for a task's input. Cheap to clone; usable from any thread.
#[derive(Clone)]
pub struct Writer(Sender<Vec<u8>>);

impl Writer {
    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        let bytes = bytes.into();
        if !bytes.is_empty() {
            // Fails only after the task is gone, when there's nobody to write to.
            let _ = self.0.send(bytes);
        }
    }
}

pub(crate) fn spawn(master: Arc<OwnedFd>, pid: libc::pid_t) -> io::Result<Writer> {
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    thread::Builder::new().name(format!("pty-writer-{pid}")).spawn(move || {
        for chunk in rx {
            if write_all(&master, &chunk).is_err() {
                break;
            }
        }
    })?;
    Ok(Writer(tx))
}

fn write_all(master: &OwnedFd, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        let n = unsafe { libc::write(master.as_raw_fd(), bytes.as_ptr().cast(), bytes.len()) };
        if n < 0 {
            let err = io::Error::last_os_error();
            if err.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            return Err(err);
        }
        bytes = &bytes[n as usize..];
    }
    Ok(())
}
