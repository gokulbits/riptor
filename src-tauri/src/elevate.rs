//! Run the VPN helper elevated: UAC on Windows, pkexec on Linux.

use std::path::Path;

/// The account password, for when the desktop can't show its own prompt; wiped when dropped.
pub type Password = zeroize::Zeroizing<String>;

/// An elevated helper that keeps running; dropping this doesn't stop it.
pub struct Elevated {
    #[cfg(windows)]
    process: isize,
    #[cfg(not(windows))]
    child: std::process::Child,
    #[cfg(not(windows))]
    stderr: Tail,
    #[cfg(target_os = "linux")]
    tty: Option<std::sync::Arc<tty::Auth>>,
}

/// pkexec found no password prompt to show; Riptor can ask for the password itself.
#[derive(Debug)]
pub struct NoPrompt;

impl std::fmt::Display for NoPrompt {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(
            "your desktop didn't show a password prompt for Riptor, so Riptor will ask for it",
        )
    }
}

impl std::error::Error for NoPrompt {}

/// No control characters: a newline would type a second answer into pkexec's prompt.
pub fn check_password(p: &str) -> anyhow::Result<()> {
    anyhow::ensure!(!p.is_empty(), "enter your password");
    anyhow::ensure!(p.len() <= 1024, "that password is too long");
    anyhow::ensure!(
        !p.chars().any(char::is_control),
        "the password can't contain control characters"
    );
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn start(exe: &Path, args: &[String], _password: Option<Password>) -> anyhow::Result<Elevated> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::UI::Shell::{
        ShellExecuteExW, SEE_MASK_NOCLOSEPROCESS, SHELLEXECUTEINFOW,
    };

    let wide = |s: &std::ffi::OsStr| s.encode_wide().chain(Some(0)).collect::<Vec<u16>>();
    let params: String = args
        .iter()
        .map(|a| {
            anyhow::ensure!(!a.contains('"'), "argument contains a quote");
            Ok(format!("\"{a}\""))
        })
        .collect::<anyhow::Result<Vec<_>>>()?
        .join(" ");
    let verb = wide("runas".as_ref());
    let file = wide(exe.as_os_str());
    let params = wide(params.as_ref());

    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = size_of::<SHELLEXECUTEINFOW>() as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = 0; // SW_HIDE

    // SAFETY: the strings outlive the call; Elevated closes the handle.
    let ok = unsafe { ShellExecuteExW(&mut info) };
    if ok == 0 {
        let e = std::io::Error::last_os_error();
        // ERROR_CANCELLED
        if e.raw_os_error() == Some(1223) {
            anyhow::bail!("administrator permission was declined");
        }
        return Err(e.into());
    }
    anyhow::ensure!(!info.hProcess.is_null(), "no process handle from UAC");
    Ok(Elevated {
        process: info.hProcess as isize,
    })
}

#[cfg(windows)]
#[allow(unsafe_code)]
impl Elevated {
    /// `None` while it's still running.
    pub fn exit_code(&mut self) -> Option<i32> {
        use windows_sys::Win32::Foundation::WAIT_OBJECT_0;
        use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
        let h = self.process as windows_sys::Win32::Foundation::HANDLE;
        let mut code = 1u32;
        // SAFETY: we own the handle until Drop.
        unsafe {
            if WaitForSingleObject(h, 0) != WAIT_OBJECT_0 {
                return None;
            }
            GetExitCodeProcess(h, &mut code);
        }
        Some(code as i32)
    }
}

#[cfg(windows)]
#[allow(unsafe_code)]
impl Drop for Elevated {
    fn drop(&mut self) {
        // SAFETY: the handle from ShellExecuteExW, closed once.
        unsafe { windows_sys::Win32::Foundation::CloseHandle(self.process as _) };
    }
}

#[cfg(windows)]
impl Elevated {
    pub fn stderr(&self) -> String {
        String::new()
    }

    pub fn auth_failure(&self) -> Option<&'static str> {
        None
    }
}

/// The last few KiB someone wrote to stderr, read on a thread so the writer never blocks.
/// Each line also goes to Riptor's log.
#[cfg(not(windows))]
#[derive(Clone, Default)]
pub struct Tail(std::sync::Arc<parking_lot::Mutex<String>>);

/// One line from pkexec or the VPN helper into Riptor's log, bounded and without control characters.
#[cfg(not(windows))]
fn log_line(from: &str, line: &[u8]) {
    let text: String = String::from_utf8_lossy(line)
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(500)
        .collect();
    let text = text.trim();
    if !text.is_empty() {
        tracing::warn!(target: "riptor::vpn", "{from}: {text}");
    }
}

#[cfg(not(windows))]
impl Tail {
    const MAX: usize = 4096;

    fn push(&self, bytes: &[u8]) {
        let mut b = self.0.lock();
        b.push_str(&String::from_utf8_lossy(bytes));
        if b.len() > Self::MAX {
            let cut = b.len() - Self::MAX;
            let cut = (cut..b.len()).find(|&i| b.is_char_boundary(i)).unwrap_or(0);
            b.drain(..cut);
        }
    }

    fn drain(&self, r: impl std::io::Read + Send + 'static) {
        use std::io::BufRead;
        let tail = self.clone();
        std::thread::spawn(move || {
            let mut r = std::io::BufReader::new(r);
            let mut line = Vec::new();
            loop {
                line.clear();
                match std::io::Read::take(&mut r, 4096).read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        log_line("pkexec/helper", &line);
                        tail.push(&line);
                    }
                }
            }
        });
    }

    pub fn text(&self) -> String {
        self.0.lock().clone()
    }
}

#[cfg(not(windows))]
pub fn start(exe: &Path, args: &[String], password: Option<Password>) -> anyhow::Result<Elevated> {
    let mut cmd = std::process::Command::new("/usr/bin/pkexec");
    cmd.arg(exe)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped());
    #[cfg(target_os = "linux")]
    let (mut child, tty) = match password {
        Some(pw) => {
            let (master, slave) = tty::open()?;
            tty::attach(&mut cmd, &slave);
            let child = cmd.spawn()?;
            (child, Some(tty::answer(master, slave, pw)))
        }
        None => (cmd.spawn()?, None),
    };
    #[cfg(not(target_os = "linux"))]
    let mut child = {
        drop(password);
        cmd.spawn()?
    };
    let stderr = Tail::default();
    if let Some(e) = child.stderr.take() {
        stderr.drain(e);
    }
    Ok(Elevated {
        child,
        stderr,
        #[cfg(target_os = "linux")]
        tty,
    })
}

#[cfg(not(windows))]
impl Elevated {
    /// `None` while it's still running. pkexec's own 126/127 mean it didn't get permission.
    pub fn exit_code(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(s)) => Some(s.code().unwrap_or(1)),
            Ok(None) => None,
            Err(_) => Some(1),
        }
    }

    /// What pkexec (or the helper) said on stderr.
    pub fn stderr(&self) -> String {
        self.stderr.text()
    }

    /// Why Riptor's own answer to pkexec's password prompt didn't work, if it gave one.
    pub fn auth_failure(&self) -> Option<&'static str> {
        #[cfg(target_os = "linux")]
        return self.tty.as_ref().and_then(|t| t.failure());
        #[cfg(not(target_os = "linux"))]
        None
    }
}

/// pkexec found no graphical prompt and couldn't fall back to a terminal one.
pub fn no_prompt(code: i32, stderr: &str) -> bool {
    !cfg!(windows)
        && code == 127
        && (stderr.contains("No authentication agent")
            || stderr.contains("textual authentication agent"))
}

/// Why pkexec didn't run the helper, from its exit code and what it printed.
pub fn pkexec_failure(code: i32, stderr: &str) -> Option<String> {
    if cfg!(windows) {
        return None;
    }
    let said = stderr
        .lines()
        .map(str::trim)
        .find(|l| !l.is_empty())
        .unwrap_or_default();
    Some(match code {
        126 => "the administrator prompt was closed".into(),
        // With no graphical agent, pkexec falls back to a terminal prompt Riptor doesn't have.
        127 if stderr.contains("No authentication agent")
            || stderr.contains("textual authentication agent") =>
        {
            NoPrompt.to_string()
        }
        127 if stderr.contains("Not authorized") || said.is_empty() => {
            "administrator permission wasn't given (wrong password, or this account isn't an administrator)".into()
        }
        127 => format!("pkexec couldn't start the VPN helper: {}", said.chars().take(300).collect::<String>()),
        _ => return None,
    })
}

/// Linux: keeps the helper running with piped stdin/stdout (WireGuard).
#[cfg(target_os = "linux")]
pub fn spawn(
    exe: &Path,
    args: &[String],
    password: Option<Password>,
) -> std::io::Result<(
    tokio::process::Child,
    Tail,
    Option<std::sync::Arc<tty::Auth>>,
)> {
    let mut cmd = std::process::Command::new("/usr/bin/pkexec");
    cmd.arg(exe)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped());
    let pty = match password {
        Some(pw) => {
            let (m, s) = tty::open()?;
            tty::attach(&mut cmd, &s);
            Some((m, s, pw))
        }
        None => None,
    };
    let mut child = tokio::process::Command::from(cmd).spawn()?;
    let auth = pty.map(|(m, s, pw)| tty::answer(m, s, pw));
    let stderr = Tail::default();
    if let Some(e) = child.stderr.take() {
        let tail = stderr.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncBufReadExt;
            let mut r = tokio::io::BufReader::new(e);
            let mut line = Vec::new();
            loop {
                line.clear();
                match r.read_until(b'\n', &mut line).await {
                    Ok(0) | Err(_) => break,
                    Ok(_) => {
                        line.truncate(4096);
                        log_line("pkexec/helper", &line);
                        tail.push(&line);
                    }
                }
            }
        });
    }
    Ok((child, stderr, auth))
}

/// Linux: answering pkexec's own terminal prompt when the desktop has no graphical one.
#[cfg(target_os = "linux")]
pub mod tty {
    use std::collections::VecDeque;
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, BorrowedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::process::CommandExt;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use parking_lot::Mutex;
    use tracing::{info, warn};

    use super::{next_reply, Password, Reply};

    /// How long pkexec gets to ask, and then to say whether the password worked.
    const PROMPT_WAIT: Duration = Duration::from_secs(60);
    /// Terminals kept open for helpers that may still run; the oldest is closed past this.
    const KEPT: usize = 8;

    static OPEN: Mutex<VecDeque<Arc<AtomicBool>>> = Mutex::new(VecDeque::new());

    #[derive(Default)]
    pub struct Auth {
        failure: Mutex<Option<&'static str>>,
    }

    impl Auth {
        pub fn failure(&self) -> Option<&'static str> {
            *self.failure.lock()
        }
    }

    /// A new pseudo-terminal: (master, slave).
    pub fn open() -> std::io::Result<(std::fs::File, std::fs::File)> {
        use rustix::pty::{grantpt, openpt, ptsname, unlockpt, OpenptFlags};
        use std::os::unix::fs::OpenOptionsExt;
        let master = openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY | OpenptFlags::CLOEXEC)?;
        grantpt(&master)?;
        unlockpt(&master)?;
        let name = ptsname(&master, Vec::new())?;
        let path = std::path::PathBuf::from(std::ffi::OsStr::from_bytes(name.as_bytes()));
        let slave = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(rustix::fs::OFlags::NOCTTY.bits() as i32)
            .open(path)?;
        Ok((std::fs::File::from(master), slave))
    }

    /// Makes the slave pkexec's controlling terminal, where it asks for the password.
    #[allow(unsafe_code)]
    pub fn attach(cmd: &mut std::process::Command, slave: &std::fs::File) {
        let fd = slave.as_raw_fd();
        // Untranslated prompts, so they can be recognised.
        cmd.env("LC_ALL", "C");
        // SAFETY: only setsid and an ioctl run between fork and exec; the fd stays open in the child until exec.
        unsafe {
            cmd.pre_exec(move || {
                rustix::process::setsid()?;
                rustix::process::ioctl_tiocsctty(BorrowedFd::borrow_raw(fd))?;
                Ok(())
            });
        }
    }

    /// Answers pkexec's prompts, then keeps the terminal open for as long as the helper may run.
    /// Riptor holds the slave too: with no slave open the terminal reads as closed, and closing the
    /// master would hang up the helper and OpenVPN.
    pub fn answer(master: std::fs::File, slave: std::fs::File, password: Password) -> Arc<Auth> {
        let auth = Arc::new(Auth::default());
        let state = auth.clone();
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_default();
        std::thread::spawn(move || {
            info!(target: "riptor::vpn", "no system password prompt; answering pkexec's own prompt as {user:?}");
            let result = converse(&master, &password, &user);
            match result {
                Ok(()) => {
                    info!(target: "riptor::vpn", "pkexec accepted the password");
                    let retired = Arc::new(AtomicBool::new(false));
                    {
                        let mut open = OPEN.lock();
                        open.push_back(retired.clone());
                        while open.len() > KEPT {
                            if let Some(old) = open.pop_front() {
                                old.store(true, Ordering::Relaxed);
                            }
                        }
                    }
                    keep_open(&master, &password, &retired);
                }
                Err(why) => {
                    warn!(target: "riptor::vpn", "pkexec password prompt: {why}");
                    *state.failure.lock() = Some(why);
                }
            }
            drop(password);
            drop(slave);
        });
        auth
    }

    /// What pkexec printed, for the log; the password is masked in case the terminal echoed it.
    fn log_output(text: &str, password: &str) {
        for line in text.lines().filter_map(|l| super::loggable(l, password)) {
            info!(target: "riptor::vpn", "pkexec: {line}");
        }
    }

    fn wait_readable(master: &std::fs::File) -> Result<bool, &'static str> {
        let mut fds = [rustix::event::PollFd::new(
            master,
            rustix::event::PollFlags::IN,
        )];
        let tick = rustix::event::Timespec {
            tv_sec: 0,
            tv_nsec: 250_000_000,
        };
        match rustix::event::poll(&mut fds, Some(&tick)) {
            Ok(0) | Err(rustix::io::Errno::INTR) => Ok(false),
            Ok(_) => Ok(true),
            Err(_) => Err("the password prompt closed"),
        }
    }

    fn converse(
        master: &std::fs::File,
        password: &Password,
        user: &str,
    ) -> Result<(), &'static str> {
        let mut seen = String::new();
        let mut sent = false;
        let mut since = Instant::now();
        let mut buf = [0u8; 1024];
        loop {
            if since.elapsed() > PROMPT_WAIT {
                // Quiet after the password means it worked; hanging up now would stop the helper.
                return if sent {
                    Ok(())
                } else {
                    Err("the system didn't ask for the password")
                };
            }
            if !wait_readable(master)? {
                continue;
            }
            let n = match (&*master).read(&mut buf) {
                Ok(n) if n > 0 => n,
                Ok(_) | Err(_) => return Err("the password prompt closed"),
            };
            let text = String::from_utf8_lossy(&buf[..n]);
            log_output(&text, password);
            seen.push_str(&text);
            if seen.len() > 16 * 1024 {
                let cut = seen.len() - 8 * 1024;
                let cut = (cut..seen.len())
                    .find(|&i| seen.is_char_boundary(i))
                    .unwrap_or(0);
                seen.drain(..cut);
            }
            if sent && seen.contains("AUTHENTICATION COMPLETE") {
                return Ok(());
            }
            let line = match next_reply(&seen, user, sent) {
                Reply::Wait => continue,
                Reply::Password => {
                    info!(target: "riptor::vpn", "pkexec asked for the password; sending it");
                    sent = true;
                    since = Instant::now();
                    let mut line = Password::new(String::with_capacity(password.len() + 1));
                    line.push_str(password);
                    line.push('\n');
                    line
                }
                Reply::Identity(n) => {
                    info!(target: "riptor::vpn", "pkexec asked which administrator; choosing {n}");
                    Password::new(format!("{n}\n"))
                }
                Reply::GiveUp(why) => return Err(why),
            };
            seen.clear();
            if (&*master).write_all(line.as_bytes()).is_err() {
                return Err("the password prompt closed");
            }
        }
    }

    /// Logs anything else the terminal shows until Riptor no longer needs it open.
    fn keep_open(master: &std::fs::File, password: &str, retired: &AtomicBool) {
        let mut buf = [0u8; 1024];
        while !retired.load(Ordering::Relaxed) {
            match wait_readable(master) {
                Ok(true) => match (&*master).read(&mut buf) {
                    Ok(n) if n > 0 => log_output(&String::from_utf8_lossy(&buf[..n]), password),
                    _ => std::thread::sleep(Duration::from_millis(250)),
                },
                Ok(false) => {}
                Err(_) => std::thread::sleep(Duration::from_millis(250)),
            }
        }
    }
}

/// A terminal line fit for the log: no control characters, bounded, the password masked.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn loggable(line: &str, password: &str) -> Option<String> {
    let mut line: String = line
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if !password.is_empty() {
        line = line.replace(password, "********");
    }
    let line: String = line.trim().chars().take(500).collect();
    (!line.is_empty()).then_some(line)
}

/// What to type next into pkexec's terminal prompt, from what it has printed since the last answer.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
pub enum Reply {
    Wait,
    Password,
    Identity(usize),
    GiveUp(&'static str),
}

#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
pub fn next_reply(seen: &str, user: &str, sent_password: bool) -> Reply {
    let tail = seen.trim_end();
    if seen.contains("AUTHENTICATION FAILED") {
        return Reply::GiveUp("wrong password");
    }
    if seen.contains("Choose identity to authenticate as") {
        if !tail.ends_with(':') {
            return Reply::Wait;
        }
        // Lines like "  2.  Full Name (login)" or "  2.  login".
        let pick = seen.lines().find_map(|l| {
            let (n, who) = l.trim().split_once('.')?;
            let n: usize = n.trim().parse().ok()?;
            let who = who.trim();
            (!user.is_empty() && (who == user || who.ends_with(&format!("({user})")))).then_some(n)
        });
        return match pick {
            Some(n) => Reply::Identity(n),
            None => Reply::GiveUp("this account can't give administrator permission"),
        };
    }
    let last = tail.lines().last().unwrap_or_default();
    if tail.ends_with(':') && last.contains("assword") {
        return if sent_password {
            Reply::GiveUp("wrong password")
        } else {
            Reply::Password
        };
    }
    Reply::Wait
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pkexec_failures_say_why() {
        let why = |code, err: &str| pkexec_failure(code, err).unwrap_or_default();
        if cfg!(windows) {
            assert!(pkexec_failure(127, "Not authorized").is_none());
            return;
        }
        assert!(why(126, "").contains("closed"));
        assert!(why(
            127,
            "Error executing command as another user: No authentication agent found.\n"
        )
        .contains("didn't show a password prompt"));
        // What pkexec says when it tries a terminal prompt instead.
        assert!(why(
            127,
            "Error creating textual authentication agent: Error opening current controlling terminal for the process (`/dev/tty'): No such device or address\n"
        )
        .contains("didn't show a password prompt"));
        assert!(why(
            127,
            "Error executing command as another user: Not authorized\n"
        )
        .contains("wrong password"));
        assert!(why(127, "").contains("wrong password"));
        let other = why(
            127,
            "\n  Cannot run program /usr/bin/riptor-vpn-helper: No such file\n",
        );
        assert!(other.contains("Cannot run program"), "{other}");
        assert!(why(127, &"x".repeat(5000)).len() < 400);
        // The helper's own codes aren't pkexec's.
        assert!(pkexec_failure(3, "").is_none());
    }

    #[cfg(not(windows))]
    #[test]
    fn stderr_tail_keeps_only_the_end() {
        let t = Tail::default();
        t.push("é".repeat(3000).as_bytes());
        t.push(b"last words");
        let s = t.text();
        assert!(s.len() <= Tail::MAX && s.ends_with("last words"));
    }

    #[test]
    fn answers_pkexecs_terminal_prompts() {
        let banner = "==== AUTHENTICATING FOR app.riptor.vpn-helper ====\nRiptor VPN Service needs administrator rights to create the VPN tunnel\nAuthenticating as: Pat (pat)\n";
        assert_eq!(next_reply(banner, "pat", false), Reply::Wait);
        assert_eq!(
            next_reply(&format!("{banner}Password: "), "pat", false),
            Reply::Password
        );
        // A second prompt or a failure means the password was wrong; it isn't sent again.
        assert_eq!(
            next_reply(
                "polkit-agent-helper-1: pam_authenticate failed\nPassword: ",
                "pat",
                true
            ),
            Reply::GiveUp("wrong password")
        );
        assert_eq!(
            next_reply("==== AUTHENTICATION FAILED ====\n", "pat", true),
            Reply::GiveUp("wrong password")
        );
        // Several administrators: pick this account, or give up if it isn't one.
        let menu = "Multiple identities can be used for authentication:\n 1.  Admin One (root2)\n 2.  Pat (pat)\nChoose identity to authenticate as (1-2): ";
        assert_eq!(next_reply(menu, "pat", false), Reply::Identity(2));
        assert_eq!(
            next_reply(menu, "eve", false),
            Reply::GiveUp("this account can't give administrator permission")
        );
        assert_eq!(
            next_reply(menu, "", false),
            Reply::GiveUp("this account can't give administrator permission")
        );
        // A user name that only appears inside another one doesn't match.
        assert_eq!(
            next_reply(menu, "at", false),
            Reply::GiveUp("this account can't give administrator permission")
        );
        assert_eq!(
            next_reply(&menu[..menu.len() - 8], "pat", false),
            Reply::Wait
        );
    }

    #[test]
    fn the_password_never_reaches_the_log() {
        assert_eq!(
            loggable("Password: hunter2hunter2", "hunter2hunter2").as_deref(),
            Some("Password: ********")
        );
        assert_eq!(loggable("  [0m  ", "x"), Some("[0m".into()));
        assert_eq!(loggable("   ", "x"), None);
        assert!(loggable(&"y".repeat(2000), "").unwrap().len() <= 500);
    }

    #[test]
    fn passwords_cant_smuggle_extra_answers() {
        assert!(check_password("correct horse").is_ok());
        assert!(check_password("pässwörd with spaces").is_ok());
        for bad in [
            "",
            "two\nlines",
            "cr\rhere",
            "nul\0",
            "esc\u{1b}[A",
            &"x".repeat(1025),
        ] {
            assert!(check_password(bad).is_err(), "{bad:?}");
        }
        assert!(no_prompt(127, "Error creating textual authentication agent: x") != cfg!(windows));
        assert!(!no_prompt(
            126,
            "Error creating textual authentication agent: x"
        ));
        assert!(!no_prompt(127, "Not authorized"));
    }
}
