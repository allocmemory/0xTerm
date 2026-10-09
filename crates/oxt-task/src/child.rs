//! What runs on the pty: program, arguments, directory and environment.
//!
//! Everything the child needs is turned into C strings before `fork()`,
//! because the child of a multithreaded process may not allocate.

use std::ffi::{CStr, CString, OsStr, OsString};
use std::io;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::ptr;

/// Variables from the terminal that launched 0xTerm that would lie to the new shell.
const INHERITED_NOISE: &[&str] = &[
    "TERM_SESSION_ID",
    "ITERM_SESSION_ID",
    "ITERM_PROFILE",
    "LC_TERMINAL",
    "LC_TERMINAL_VERSION",
    "COLORFGBG",
    // direnv state belongs to the launching shell; the new shell loads its own.
    "DIRENV_DIR",
    "DIRENV_FILE",
    "DIRENV_DIFF",
    "DIRENV_WATCHES",
];

#[derive(Clone, Debug)]
pub struct Command {
    program: PathBuf,
    args: Vec<OsString>,
    login: bool,
    cwd: Option<PathBuf>,
    env: Vec<(OsString, OsString)>,
}

impl Command {
    pub fn new(program: impl Into<PathBuf>) -> Self {
        Command { program: program.into(), args: Vec::new(), login: false, cwd: None, env: Vec::new() }
    }

    /// The user's shell as a login shell, the way Mac terminals start it.
    pub fn login_shell() -> Self {
        Command { login: true, ..Command::new(user_shell()) }
    }

    /// Marks argv[0] with a leading '-' so the shell runs its login scripts.
    pub fn login(mut self) -> Self {
        self.login = true;
        self
    }

    pub fn arg(mut self, arg: impl Into<OsString>) -> Self {
        self.args.push(arg.into());
        self
    }

    pub fn cwd(mut self, dir: impl Into<PathBuf>) -> Self {
        self.cwd = Some(dir.into());
        self
    }

    pub fn env(mut self, key: impl Into<OsString>, value: impl Into<OsString>) -> Self {
        self.env.push((key.into(), value.into()));
        self
    }

    pub(crate) fn prepare(&self) -> io::Result<Prepared> {
        let program = cstring(self.program.as_os_str())?;

        // A leading '-' in argv[0] is how a shell learns it's a login shell.
        let name = self.program.file_name().unwrap_or(self.program.as_os_str());
        let mut argv0 = OsString::new();
        if self.login {
            argv0.push("-");
        }
        argv0.push(name);
        let argv =
            std::iter::once(argv0.as_os_str()).chain(self.args.iter().map(OsString::as_os_str)).map(cstring).collect::<io::Result<Vec<_>>>()?;

        let home = home_dir();
        let cwd = self.cwd.clone().filter(|dir| dir.is_dir()).or_else(|| home.clone()).unwrap_or_else(|| "/".into());
        let envp = self
            .environment(&cwd)
            .into_iter()
            .map(|(key, value)| {
                let mut pair = key;
                pair.push("=");
                pair.push(value);
                cstring(&pair)
            })
            .collect::<io::Result<Vec<_>>>()?;

        let mut sigset: libc::sigset_t = unsafe { std::mem::zeroed() };
        unsafe { libc::sigemptyset(&mut sigset) };

        Ok(Prepared::new(program, argv, envp, cstring(cwd.as_os_str())?, sigset))
    }

    fn environment(&self, cwd: &Path) -> Vec<(OsString, OsString)> {
        let mut env: Vec<(OsString, OsString)> = std::env::vars_os().filter(|(key, _)| !INHERITED_NOISE.iter().any(|noise| key == noise)).collect();
        let mut set = |key: &str, value: &OsStr| {
            env.retain(|(k, _)| k != key);
            env.push((key.into(), value.into()));
        };
        set("TERM", "xterm-256color".as_ref());
        set("COLORTERM", "truecolor".as_ref());
        set("TERM_PROGRAM", "0xTerm".as_ref());
        set("TERM_PROGRAM_VERSION", env!("CARGO_PKG_VERSION").as_ref());
        set("PWD", cwd.as_os_str());
        // Apps started from the Dock get no locale; without one zsh mangles UTF-8.
        let has_locale = ["LANG", "LC_ALL", "LC_CTYPE"].iter().any(|key| std::env::var_os(key).is_some());
        if !has_locale {
            set("LANG", "en_US.UTF-8".as_ref());
        }
        for (key, value) in &self.env {
            env.retain(|(k, _)| k != key);
            env.push((key.clone(), value.clone()));
        }
        env
    }
}

/// `$SHELL` if it's usable, else the shell in the user database, else zsh.
pub fn user_shell() -> PathBuf {
    if let Some(shell) = std::env::var_os("SHELL").map(PathBuf::from)
        && shell.is_absolute()
        && shell.exists()
    {
        return shell;
    }
    unsafe {
        let pw = libc::getpwuid(libc::getuid());
        if !pw.is_null() && !(*pw).pw_shell.is_null() {
            let shell = PathBuf::from(OsStr::from_bytes(CStr::from_ptr((*pw).pw_shell).to_bytes()));
            if shell.exists() {
                return shell;
            }
        }
    }
    "/bin/zsh".into()
}

fn home_dir() -> Option<PathBuf> {
    std::env::var_os("HOME").map(PathBuf::from).filter(|home| home.is_dir())
}

fn cstring(s: &OsStr) -> io::Result<CString> {
    CString::new(s.as_bytes()).map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "NUL byte in command"))
}

/// A command flattened into the C arrays `execve` takes.
pub(crate) struct Prepared {
    program: CString,
    cwd: CString,
    sigset: libc::sigset_t,
    argv_ptrs: Vec<*const libc::c_char>,
    envp_ptrs: Vec<*const libc::c_char>,
    // Owners of the strings the pointer arrays point into.
    _argv: Vec<CString>,
    _envp: Vec<CString>,
}

impl Prepared {
    fn new(program: CString, argv: Vec<CString>, envp: Vec<CString>, cwd: CString, sigset: libc::sigset_t) -> Self {
        let argv_ptrs = argv.iter().map(|s| s.as_ptr()).chain([ptr::null()]).collect();
        let envp_ptrs = envp.iter().map(|s| s.as_ptr()).chain([ptr::null()]).collect();
        Prepared { program, cwd, sigset, argv_ptrs, envp_ptrs, _argv: argv, _envp: envp }
    }

    /// Runs in the forked child: reset signals, enter the directory, exec.
    ///
    /// # Safety
    /// Call only in a freshly forked child.
    pub(crate) unsafe fn exec(&self) -> ! {
        unsafe {
            // Rust ignores SIGPIPE and the exec'd program would inherit that; restore defaults.
            // macOS has 31 signals (NSIG = 32).
            for signal in 1..32 {
                libc::signal(signal, libc::SIG_DFL);
            }
            libc::pthread_sigmask(libc::SIG_SETMASK, &self.sigset, ptr::null_mut());
            libc::chdir(self.cwd.as_ptr());
            libc::execve(self.program.as_ptr(), self.argv_ptrs.as_ptr(), self.envp_ptrs.as_ptr());
            libc::_exit(127)
        }
    }
}
