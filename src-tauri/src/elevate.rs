//! Run the VPN helper elevated: UAC on Windows, pkexec on Linux.

use std::path::Path;

#[cfg(windows)]
#[allow(unsafe_code)]
pub fn run(exe: &Path, args: &[String]) -> anyhow::Result<i32> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Foundation::{CloseHandle, WAIT_OBJECT_0};
    use windows_sys::Win32::System::Threading::{
        GetExitCodeProcess, WaitForSingleObject, INFINITE,
    };
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

    // SAFETY: the strings outlive the call; the handle is closed below.
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
    let mut code = 1u32;
    // SAFETY: we own hProcess until CloseHandle.
    unsafe {
        if WaitForSingleObject(info.hProcess, INFINITE) == WAIT_OBJECT_0 {
            GetExitCodeProcess(info.hProcess, &mut code);
        }
        CloseHandle(info.hProcess);
    }
    Ok(code as i32)
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

#[cfg(not(windows))]
pub fn run(exe: &Path, args: &[String]) -> anyhow::Result<i32> {
    let status = std::process::Command::new("/usr/bin/pkexec")
        .arg(exe)
        .args(args)
        .status()?;
    match status.code() {
        Some(126) | Some(127) => anyhow::bail!("administrator permission was declined"),
        Some(c) => Ok(c),
        None => anyhow::bail!("helper was killed"),
    }
}
