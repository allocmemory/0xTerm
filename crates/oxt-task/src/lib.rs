//! Shell processes on a pty: one pty, one reader thread and one writer thread
//! per pane.
//!
//! ```text
//!   Task::write ──► writer thread ──► master fd ═══ kernel line discipline ═══ slave fd ◄─► shell
//!   on_output   ◄── reader thread ◄── master fd
//! ```

mod child;
mod proc_info;
mod pty;
mod reader;
mod writer;

use std::io;
use std::os::fd::OwnedFd;
use std::path::PathBuf;
use std::process::ExitStatus;
use std::sync::Arc;

pub use child::{Command, user_shell};
pub use proc_info::Process;
pub use pty::Size;
pub use writer::Writer;

/// A running program attached to a pty. Dropping it hangs up the program.
pub struct Task {
    pid: libc::pid_t,
    master: Arc<OwnedFd>,
    writer: Writer,
}

impl Task {
    /// Starts `command` on a new pty of `size`.
    ///
    /// `on_output` runs on the reader thread with every chunk the program prints.
    /// `on_exit` runs on the same thread once the program has exited and its
    /// output is drained.
    pub fn spawn(
        command: &Command,
        size: Size,
        on_output: impl FnMut(&[u8]) + Send + 'static,
        on_exit: impl FnOnce(ExitStatus) + Send + 'static,
    ) -> io::Result<Task> {
        let prepared = command.prepare()?;
        let (master, pid) = pty::spawn(size, &prepared)?;
        let master = Arc::new(master);
        reader::spawn(master.clone(), pid, on_output, on_exit)?;
        let writer = writer::spawn(master.clone(), pid)?;
        Ok(Task { pid, master, writer })
    }

    pub fn pid(&self) -> i32 {
        self.pid
    }

    /// Queues bytes for the program's input. Never blocks.
    pub fn write(&self, bytes: impl Into<Vec<u8>>) {
        self.writer.write(bytes);
    }

    /// A handle that can write to this task from any thread.
    pub fn writer(&self) -> Writer {
        self.writer.clone()
    }

    /// Sets the pty's window size. The kernel sends SIGWINCH to the foreground job.
    pub fn resize(&self, size: Size) -> io::Result<()> {
        pty::resize(&self.master, size)
    }

    /// The shell's current directory.
    pub fn cwd(&self) -> Option<PathBuf> {
        proc_info::cwd(self.pid)
    }

    /// The job that owns the terminal right now: the shell itself, or e.g. `vim`.
    pub fn foreground(&self) -> Option<Process> {
        let pgrp = pty::foreground_pgrp(&self.master)?;
        proc_info::process(pgrp)
    }

    /// True when something other than the shell owns the terminal.
    pub fn is_busy(&self) -> bool {
        pty::foreground_pgrp(&self.master).is_some_and(|pgrp| pgrp != self.pid)
    }

    /// Sends SIGHUP, like closing a terminal window. The shell passes it on to its jobs.
    pub fn hangup(&self) {
        unsafe { libc::kill(self.pid, libc::SIGHUP) };
    }
}

impl Drop for Task {
    fn drop(&mut self) {
        self.hangup();
    }
}
