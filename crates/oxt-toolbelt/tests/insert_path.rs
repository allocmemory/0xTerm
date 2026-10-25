//! A path typed into a shell on the user's behalf ("cd Here", Insert Path, a
//! dropped file) must arrive as one literal argument, whatever its name holds.

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use oxt_session::{Session, SessionEvent, SessionOptions};
use oxt_task::{Command, Size};
use oxt_toolbelt::{Shell, shell_quote};

/// What the window does for "cd Here".
fn cd_here(session: &Session, dir: &Path) {
    let shell = session.foreground().map(|p| Shell::from_process(&p.name)).unwrap_or_default();
    session.paste(&format!("cd {}", shell_quote(dir, shell)));
    session.write(*b"\r");
}

/// A shell in `home`, after it has drawn its first prompt.
fn shell(program: &str, flag: &str, home: &Path) -> (Session, async_channel::Receiver<SessionEvent>) {
    let command = Command::new(program).arg(flag).arg("-i").env("HOME", home).env("PS1", "$ ");
    let (session, rx) = Session::spawn(SessionOptions { command, size: Size::new(10, 80), ..SessionOptions::default() }).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        if let Ok(SessionEvent::Wakeup) = rx.recv_blocking() {
            session.wakeup_handled();
            if session.lock().term.grid().cursor.point.column.0 > 0 {
                break;
            }
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    (session, rx)
}

fn check(program: &str, flag: &str) {
    let home = std::env::temp_dir().join(format!("oxt-insert-{}-{}", Path::new(program).file_name().unwrap().to_str().unwrap(), std::process::id()));
    std::fs::create_dir_all(&home).unwrap();
    let home = home.canonicalize().unwrap();
    // Ctrl-U erases the half-typed `cd '…` line; the newline then runs what follows.
    let evil: PathBuf = home.join("docs\x15cd;touch pwned\n");
    std::fs::create_dir_all(&evil).unwrap();

    let (session, rx) = shell(program, flag, &home);
    cd_here(&session, &evil);
    std::thread::sleep(Duration::from_secs(2));
    let ran = home.join("pwned").exists();
    let cwd = session.cwd();
    session.close();
    // bash writes its history into `home` on the way out.
    while !matches!(rx.recv_blocking(), Ok(SessionEvent::Exited(_)) | Err(_)) {}
    std::fs::remove_dir_all(&home).unwrap();

    assert!(!ran, "{program}: a folder name ran a command");
    assert_eq!(cwd.as_deref(), Some(evil.as_path()), "{program}: cd didn't land in the folder");
}

#[test]
fn folder_name_cannot_run_a_command_in_zsh() {
    check("/bin/zsh", "-f");
}

#[test]
fn folder_name_cannot_run_a_command_in_bash() {
    check("/bin/bash", "--norc");
}

#[test]
fn folder_name_cannot_run_a_command_in_ksh() {
    check("/bin/ksh", "-E");
}

#[test]
fn folder_name_cannot_run_a_command_in_fish() {
    check("/opt/homebrew/bin/fish", "--no-config");
}
