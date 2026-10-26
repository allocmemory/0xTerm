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
    let quoted = shell_quote(dir, shell).expect("a shell this test knows quotes every name");
    session.paste(&format!("cd {quoted}"));
    session.write(*b"\r");
}

/// A shell in `home`, after it has drawn its first prompt.
fn shell(program: &str, flag: &str, home: &Path) -> (Session, async_channel::Receiver<SessionEvent>) {
    // Started in `home`, so a command a name smuggles in shows up there.
    let command = Command::new(program).arg(flag).arg("-i").cwd(home).env("HOME", home).env("PS1", "$ ");
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

/// Names that broke out of the quoting before: Ctrl-U erases the half-typed
/// `cd '…` line and the newline runs what follows; in fish, `\'` inside single
/// quotes is an escaped quote, so the quote closed early.
const HOSTILE: [&str; 2] = ["docs\x15cd;touch pwned\n", "x\\';touch pwned;#"];

/// The shell's path, or `None` (and the test is skipped) when it isn't installed.
fn find(name: &str) -> Option<PathBuf> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    let found =
        std::env::split_paths(&path).chain(["/opt/homebrew/bin".into(), "/usr/local/bin".into()]).map(|dir| dir.join(name)).find(|p| p.is_file());
    if found.is_none() {
        eprintln!("skipped: {name} isn't installed");
    }
    found
}

fn check(name: &str, flag: &str) {
    let Some(program) = find(name) else { return };
    let program = program.to_str().unwrap();
    for (i, hostile) in HOSTILE.iter().enumerate() {
        let home = std::env::temp_dir().join(format!("oxt-insert-{name}-{i}-{}", std::process::id()));
        std::fs::create_dir_all(&home).unwrap();
        let home = home.canonicalize().unwrap();
        let evil: PathBuf = home.join(hostile);
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

        assert!(!ran, "{program}: the folder name {hostile:?} ran a command");
        assert_eq!(cwd.as_deref(), Some(evil.as_path()), "{program}: cd didn't land in {hostile:?}");
    }
}

#[test]
fn folder_name_cannot_run_a_command_in_zsh() {
    check("zsh", "-f");
}

#[test]
fn folder_name_cannot_run_a_command_in_bash() {
    check("bash", "--norc");
}

#[test]
fn folder_name_cannot_run_a_command_in_ksh() {
    check("ksh", "-E");
}

#[test]
fn folder_name_cannot_run_a_command_in_fish() {
    check("fish", "--no-config");
}
