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
#[cfg(not(windows))]
#[derive(Clone, Default)]
pub struct Tail(std::sync::Arc<parking_lot::Mutex<String>>);

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

    fn drain(&self, mut r: impl std::io::Read + Send + 'static) {
        let tail = self.clone();
        std::thread::spawn(move || {
            let mut chunk = [0u8; 1024];
            while let Ok(n @ 1..) = r.read(&mut chunk) {
                tail.push(&chunk[..n]);
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
            drop(slave);
            (child, Some(tty::answer(master, pw)))
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
    let mut auth = None;
    let mut slave = None;
    let mut master = None;
    if password.is_some() {
        let (m, s) = tty::open()?;
        tty::attach(&mut cmd, &s);
        master = Some(m);
        slave = Some(s);
    }
    let mut child = tokio::process::Command::from(cmd).spawn()?;
    drop(slave);
    if let (Some(m), Some(pw)) = (master, password) {
        auth = Some(tty::answer(m, pw));
    }
    let stderr = Tail::default();
    if let Some(mut e) = child.stderr.take() {
        let tail = stderr.clone();
        tokio::spawn(async move {
            use tokio::io::AsyncReadExt;
            let mut chunk = [0u8; 1024];
            while let Ok(n @ 1..) = e.read(&mut chunk).await {
                tail.push(&chunk[..n]);
            }
        });
    }
    Ok((child, stderr, auth))
}

/// Linux: answering pkexec's own terminal prompt when the desktop has no graphical one.
#[cfg(target_os = "linux")]
pub mod tty {
    use std::io::{Read, Write};
    use std::os::fd::{AsRawFd, BorrowedFd};
    use std::os::unix::ffi::OsStrExt;
    use std::os::unix::process::CommandExt;
    use std::sync::Arc;
    use std::time::{Duration, Instant};

    use parking_lot::Mutex;

    use super::{next_reply, Password, Reply};

    /// How long pkexec gets to ask, and then to say whether the password worked.
    const PROMPT_WAIT: Duration = Duration::from_secs(60);

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

    /// Answers pkexec's prompts on `master`, then keeps the terminal open for as long as the helper runs.
    pub fn answer(master: std::fs::File, password: Password) -> Arc<Auth> {
        let auth = Arc::new(Auth::default());
        let state = auth.clone();
        let user = std::env::var("USER")
            .or_else(|_| std::env::var("LOGNAME"))
            .unwrap_or_default();
        std::thread::spawn(move || {
            let why = converse(&master, &password, &user);
            drop(password);
            match why {
                // Closing the terminal now would hang up the helper, so hold it until the helper ends.
                Ok(()) => drain(&master),
                Err(why) => *state.failure.lock() = Some(why),
            }
        });
        auth
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
            let mut fds = [rustix::event::PollFd::new(
                master,
                rustix::event::PollFlags::IN,
            )];
            let tick = rustix::event::Timespec {
                tv_sec: 0,
                tv_nsec: 250_000_000,
            };
            match rustix::event::poll(&mut fds, Some(&tick)) {
                Ok(0) => continue,
                Ok(_) => {}
                Err(rustix::io::Errno::INTR) => continue,
                Err(_) => return Err("the password prompt closed"),
            }
            let n = match (&*master).read(&mut buf) {
                Ok(0) | Err(_) => {
                    // The terminal closed: pkexec has finished one way or the other.
                    return if sent {
                        Ok(())
                    } else {
                        Err("the password prompt closed")
                    };
                }
                Ok(n) => n,
            };
            seen.push_str(&String::from_utf8_lossy(&buf[..n]));
            if seen.len() > 16 * 1024 {
                seen.drain(..seen.len() - 8 * 1024);
            }
            if sent && seen.contains("AUTHENTICATION COMPLETE") {
                return Ok(());
            }
            let line = match next_reply(&seen, user, sent) {
                Reply::Wait => continue,
                Reply::Password => {
                    sent = true;
                    since = Instant::now();
                    let mut line = Password::new(String::with_capacity(password.len() + 1));
                    line.push_str(password);
                    line.push('\n');
                    line
                }
                Reply::Identity(n) => Password::new(format!("{n}\n")),
                Reply::GiveUp(why) => return Err(why),
            };
            seen.clear();
            if (&*master).write_all(line.as_bytes()).is_err() {
                return Err("the password prompt closed");
            }
        }
    }

    /// Reads and drops whatever arrives until the helper ends and the terminal closes.
    fn drain(master: &std::fs::File) {
        let mut buf = [0u8; 1024];
        while let Ok(1..) = (&*master).read(&mut buf) {}
    }
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
