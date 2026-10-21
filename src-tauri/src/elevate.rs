//! Run the VPN helper elevated: UAC on Windows, pkexec on Linux.

use std::path::Path;

/// An elevated helper that keeps running; dropping this doesn't stop it.
pub struct Elevated {
    #[cfg(windows)]
    process: isize,
    #[cfg(not(windows))]
    child: std::process::Child,
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

#[cfg(not(windows))]
pub fn start(exe: &Path, args: &[String]) -> anyhow::Result<Elevated> {
    let child = std::process::Command::new("/usr/bin/pkexec")
        .arg(exe)
        .args(args)
        .stdin(std::process::Stdio::null())
        .spawn()?;
    Ok(Elevated { child })
}

#[cfg(not(windows))]
impl Elevated {
    /// `None` while it's still running. pkexec's own 126/127 mean the prompt was declined.
    pub fn exit_code(&mut self) -> Option<i32> {
        match self.child.try_wait() {
            Ok(Some(s)) => Some(s.code().unwrap_or(1)),
            Ok(None) => None,
            Err(_) => Some(1),
        }
    }
}

/// Linux: keeps the helper running with piped stdin/stdout (WireGuard).
#[cfg(target_os = "linux")]
pub fn spawn(exe: &Path, args: &[String]) -> std::io::Result<tokio::process::Child> {
    tokio::process::Command::new("/usr/bin/pkexec")
        .arg(exe)
        .args(args)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
}
