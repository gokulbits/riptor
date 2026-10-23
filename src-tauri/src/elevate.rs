//! Run the VPN helper elevated: UAC on Windows, pkexec on Linux.

use std::path::Path;

/// An elevated helper that keeps running; dropping this doesn't stop it.
pub struct Elevated {
    #[cfg(windows)]
    process: isize,
    #[cfg(not(windows))]
    child: std::process::Child,
    #[cfg(not(windows))]
    stderr: Tail,
}

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn start(exe: &Path, args: &[String]) -> anyhow::Result<Elevated> {
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
pub fn start(exe: &Path, args: &[String]) -> anyhow::Result<Elevated> {
    let mut child = std::process::Command::new("/usr/bin/pkexec")
        .arg(exe)
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
    let stderr = Tail::default();
    if let Some(e) = child.stderr.take() {
        stderr.drain(e);
    }
    Ok(Elevated { child, stderr })
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
        127 if stderr.contains("No authentication agent") => {
            "no password prompt could be shown: no polkit authentication agent is running in this desktop session".into()
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
pub fn spawn(exe: &Path, args: &[String]) -> std::io::Result<(tokio::process::Child, Tail)> {
    let mut child = tokio::process::Command::new("/usr/bin/pkexec")
        .arg(exe)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn()?;
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
    Ok((child, stderr))
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
        .contains("no polkit authentication agent"));
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
}
