//! The pty pair: open it, fork the child onto the slave end, keep the master.

use std::io;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd, RawFd};
use std::ptr;

use crate::child::Prepared;

/// A terminal size in cells, plus the cell size in pixels (some programs use it for images).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Size {
    pub rows: u16,
    pub cols: u16,
    pub cell_width: u16,
    pub cell_height: u16,
}

impl Size {
    pub fn new(rows: u16, cols: u16) -> Self {
        Size { rows, cols, cell_width: 0, cell_height: 0 }
    }

    fn winsize(self) -> libc::winsize {
        libc::winsize {
            ws_row: self.rows,
            ws_col: self.cols,
            ws_xpixel: self.cols.saturating_mul(self.cell_width),
            ws_ypixel: self.rows.saturating_mul(self.cell_height),
        }
    }
}

/// Opens a pty, forks, and runs the prepared program on the slave end.
/// Returns the master end and the child's pid.
///
/// This is `forkpty()` split open, so the line discipline can be configured
/// before the shell starts and reads it.
pub(crate) fn spawn(size: Size, prepared: &Prepared) -> io::Result<(OwnedFd, libc::pid_t)> {
    let (mut master, mut slave) = (-1, -1);
    let mut winsize = size.winsize();
    check(unsafe { libc::openpty(&mut master, &mut slave, ptr::null_mut(), ptr::null_mut(), &mut winsize) })?;
    let master = unsafe { OwnedFd::from_raw_fd(master) };
    let slave = unsafe { OwnedFd::from_raw_fd(slave) };

    // Panes spawned later must not inherit this pty.
    set_cloexec(master.as_raw_fd())?;
    set_cloexec(slave.as_raw_fd())?;
    enable_utf8(slave.as_raw_fd())?;

    match unsafe { libc::fork() } {
        -1 => Err(io::Error::last_os_error()),
        0 => unsafe {
            // Child. Only async-signal-safe calls from here on: no allocation, no locks.
            libc::close(master.as_raw_fd());
            // New session, slave becomes the controlling terminal and fds 0, 1, 2.
            if libc::login_tty(slave.as_raw_fd()) == -1 {
                libc::_exit(126);
            }
            prepared.exec()
        },
        pid => Ok((master, pid)),
    }
}

pub(crate) fn resize(master: &OwnedFd, size: Size) -> io::Result<()> {
    let winsize = size.winsize();
    check(unsafe { libc::ioctl(master.as_raw_fd(), libc::TIOCSWINSZ, &winsize) })
}

/// The process group that owns the terminal, from the kernel's point of view.
pub(crate) fn foreground_pgrp(master: &OwnedFd) -> Option<libc::pid_t> {
    let pgrp = unsafe { libc::tcgetpgrp(master.as_raw_fd()) };
    (pgrp > 0).then_some(pgrp)
}

/// IUTF8 makes the line discipline erase a whole UTF-8 character on backspace.
fn enable_utf8(fd: RawFd) -> io::Result<()> {
    unsafe {
        let mut termios: libc::termios = std::mem::zeroed();
        check(libc::tcgetattr(fd, &mut termios))?;
        termios.c_iflag |= libc::IUTF8;
        check(libc::tcsetattr(fd, libc::TCSANOW, &termios))
    }
}

fn set_cloexec(fd: RawFd) -> io::Result<()> {
    check(unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) })
}

fn check(ret: libc::c_int) -> io::Result<()> {
    if ret == -1 { Err(io::Error::last_os_error()) } else { Ok(()) }
}
