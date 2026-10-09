use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use oxt_task::{Command, Size, Task};

/// Output bytes so far, and the exit code once there is one.
type State = (Mutex<(Vec<u8>, Option<i32>)>, Condvar);

/// Collects a task's output so a test can wait for some text to show up.
#[derive(Clone, Default)]
struct Output {
    inner: Arc<State>,
}

impl Output {
    fn spawn(command: Command) -> (Task, Output) {
        let output = Output::default();
        let (on_bytes, on_exit) = (output.clone(), output.clone());
        let task = Task::spawn(
            &command,
            Size::new(24, 80),
            move |bytes| {
                let (lock, cvar) = &*on_bytes.inner;
                lock.lock().unwrap().0.extend_from_slice(bytes);
                cvar.notify_all();
            },
            move |status| {
                let (lock, cvar) = &*on_exit.inner;
                lock.lock().unwrap().1 = Some(status.code().unwrap_or(-1));
                cvar.notify_all();
            },
        )
        .expect("spawn");
        (task, output)
    }

    fn wait_for(&self, needle: &str) -> String {
        self.wait_until(|text, _| text.contains(needle)).unwrap_or_else(|text| panic!("never saw {needle:?}; got {text:?}"))
    }

    fn wait_exit(&self) -> i32 {
        let (lock, _) = &*self.inner;
        self.wait_until(|_, exit| exit.is_some()).expect("never exited");
        lock.lock().unwrap().1.unwrap()
    }

    fn wait_until(&self, done: impl Fn(&str, Option<i32>) -> bool) -> Result<String, String> {
        let (lock, cvar) = &*self.inner;
        let deadline = Instant::now() + Duration::from_secs(5);
        let mut state = lock.lock().unwrap();
        loop {
            let text = String::from_utf8_lossy(&state.0).into_owned();
            if done(&text, state.1) {
                return Ok(text);
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(text);
            }
            state = cvar.wait_timeout(state, left).unwrap().0;
        }
    }
}

/// Needles are typed as `echo re""ady`, so the kernel's echo of the typed
/// line never matches; only the command's own output does.
fn sh() -> Command {
    Command::new("/bin/sh").env("PS1", "$ ")
}

#[test]
fn echoes_what_the_shell_prints() {
    let (task, out) = Output::spawn(sh());
    task.write("echo hi-$((40+2))\n");
    out.wait_for("hi-42");
}

#[test]
fn resize_reaches_the_shell() {
    let (task, out) = Output::spawn(sh());
    task.resize(Size::new(30, 100)).unwrap();
    task.write("stty size\n");
    out.wait_for("30 100");
}

#[test]
fn environment_says_who_we_are() {
    let (task, out) = Output::spawn(sh());
    task.write("echo \"<$TERM|$COLORTERM|$TERM_PROGRAM>\"\n");
    out.wait_for("<xterm-256color|truecolor|0xTerm>");
}

#[test]
fn starts_in_the_requested_directory() {
    let dir = std::env::temp_dir().canonicalize().unwrap();
    let (task, out) = Output::spawn(sh().cwd(&dir));
    task.write("echo re\"\"ady\n");
    out.wait_for("ready");
    assert_eq!(task.cwd().unwrap(), dir);
    task.write("cd /usr/bin && echo mo\"\"ved\n");
    out.wait_for("moved");
    assert_eq!(task.cwd().unwrap(), std::path::Path::new("/usr/bin"));
}

#[test]
fn knows_the_foreground_job() {
    let (task, out) = Output::spawn(sh());
    task.write("echo re\"\"ady\n");
    out.wait_for("ready");
    assert!(!task.is_busy());
    task.write("sleep 3\n");
    let deadline = Instant::now() + Duration::from_secs(3);
    while !task.is_busy() && Instant::now() < deadline {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(task.foreground().unwrap().name, "sleep");
    task.write([0x03]); // ^C, turned into SIGINT by the line discipline
    task.write("echo ba\"\"ck\n");
    out.wait_for("back");
}

#[test]
fn login_shell_gets_a_dash() {
    let (_task, out) = Output::spawn(Command::new("/bin/sh").login().arg("-c").arg("echo \"argv0=$0\""));
    out.wait_for("argv0=-sh");
}

#[test]
fn reports_exit_status() {
    let (_task, out) = Output::spawn(Command::new("/bin/sh").arg("-c").arg("exit 7"));
    assert_eq!(out.wait_exit(), 7);
}

#[test]
fn hangup_ends_the_shell() {
    let (task, out) = Output::spawn(sh());
    task.write("echo re\"\"ady\n");
    out.wait_for("ready");
    drop(task);
    out.wait_exit();
}
