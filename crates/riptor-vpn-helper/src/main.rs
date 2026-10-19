//! Riptor's privileged VPN helper.
//!
//! Riptor runs unelevated. Creating a tunnel adapter needs administrator /
//! root rights, so Riptor starts this helper through UAC (Windows) or pkexec
//! (Linux). The helper is deliberately tiny and does exactly one thing:
//!
//! ```text
//! riptor-vpn-helper openvpn <config-file> <management-port> <management-password-file>
//! ```
//!
//! Its inputs come from a less-privileged process, so it trusts none of them:
//! * the config is re-sanitized with the same allowlist Riptor uses (no
//!   scripts, no plugins, no file references, app-only routing), then written
//!   to a folder only administrators can write, so it can't be swapped
//!   between the check and OpenVPN reading it;
//! * OpenVPN is run from its installed location, never from PATH;
//! * the management interface listens on 127.0.0.1 only and needs a password;
//! * the helper starts OpenVPN and exits; it never runs anything else.
#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context};

/// Exit codes Riptor maps to messages (stderr is not visible through UAC).
mod code {
    pub const USAGE: i32 = 2;
    pub const BAD_CONFIG: i32 = 3;
    pub const NO_OPENVPN: i32 = 4;
    pub const START_FAILED: i32 = 5;
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("openvpn") if args.len() == 4 => openvpn(&args[1], &args[2], &args[3]),
        _ => {
            eprintln!(
                "usage: riptor-vpn-helper openvpn <config> <management-port> <password-file>"
            );
            std::process::exit(code::USAGE);
        }
    };
    if let Err((code, e)) = result {
        eprintln!("riptor-vpn-helper: {e:#}");
        std::process::exit(code);
    }
}

type Res = Result<(), (i32, anyhow::Error)>;

fn fail(code: i32) -> impl FnOnce(anyhow::Error) -> (i32, anyhow::Error) {
    move |e| (code, e)
}

fn openvpn(config: &str, port: &str, pw_file: &str) -> Res {
    let port: u16 = port
        .parse()
        .ok()
        .filter(|p| *p >= 1024)
        .ok_or_else(|| (code::USAGE, anyhow::anyhow!("bad port")))?;
    let read_small = |p: &str, max: usize| -> anyhow::Result<Vec<u8>> {
        let len = std::fs::metadata(p)?.len() as usize;
        ensure!(len <= max, "{p} is too large");
        Ok(std::fs::read(p)?)
    };
    let raw =
        read_small(config, riptor_vpn::ovpn::MAX_PROFILE_BYTES).map_err(fail(code::BAD_CONFIG))?;
    let sanitized = riptor_vpn::ovpn::sanitize(&raw).map_err(|e| (code::BAD_CONFIG, e.into()))?;
    let password = String::from_utf8(read_small(pw_file, 256).map_err(fail(code::USAGE))?)
        .map_err(|_| (code::USAGE, anyhow::anyhow!("password file is not text")))?;
    let password = password.trim();
    if password.len() < 16 || !password.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err((
            code::USAGE,
            anyhow::anyhow!("weak or malformed management password"),
        ));
    }

    let exe = openvpn_exe().map_err(fail(code::NO_OPENVPN))?;
    let dir = private_dir().map_err(fail(code::START_FAILED))?;
    let tag = format!("{}-{}", std::process::id(), port);
    let cfg_path = dir.join(format!("riptor-{tag}.ovpn"));
    let pw_path = dir.join(format!("riptor-{tag}.pw"));
    std::fs::write(&cfg_path, sanitized.config).map_err(|e| (code::START_FAILED, e.into()))?;
    std::fs::write(&pw_path, format!("{password}\n"))
        .map_err(|e| (code::START_FAILED, e.into()))?;

    let mut cmd = Command::new(&exe);
    cmd.arg("--config")
        .arg(&cfg_path)
        .args(["--management", "127.0.0.1", &port.to_string()])
        .arg(&pw_path)
        .args([
            "--management-hold",
            "--management-query-passwords",
            "--script-security",
            "1",
        ])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        // CREATE_NO_WINDOW | DETACHED_PROCESS: no console, keeps running after we exit.
        cmd.creation_flags(0x0800_0000 | 0x0000_0008);
    }
    cmd.spawn()
        .map_err(|e| (code::START_FAILED, anyhow::anyhow!("starting OpenVPN: {e}")))?;

    // Wait for the management port, then remove the files: OpenVPN has read them.
    let started = Instant::now();
    let ready = loop {
        if std::net::TcpStream::connect_timeout(
            &([127, 0, 0, 1], port).into(),
            Duration::from_millis(300),
        )
        .is_ok()
        {
            break true;
        }
        if started.elapsed() > Duration::from_secs(15) {
            break false;
        }
        std::thread::sleep(Duration::from_millis(200));
    };
    let _ = std::fs::remove_file(&pw_path);
    let _ = std::fs::remove_file(&cfg_path);
    if !ready {
        return Err((
            code::START_FAILED,
            anyhow::anyhow!("OpenVPN did not open its management port"),
        ));
    }
    Ok(())
}

/// OpenVPN's installed location; never resolved through PATH.
fn openvpn_exe() -> anyhow::Result<PathBuf> {
    #[cfg(windows)]
    {
        use winreg::enums::HKEY_LOCAL_MACHINE;
        let hklm = winreg::RegKey::predef(HKEY_LOCAL_MACHINE);
        let mut candidates = Vec::new();
        if let Ok(k) = hklm.open_subkey(r"SOFTWARE\OpenVPN") {
            if let Ok(p) = k.get_value::<String, _>("exe_path") {
                candidates.push(PathBuf::from(p));
            }
            if let Ok(p) = k.get_value::<String, _>("") {
                candidates.push(Path::new(&p).join("bin").join("openvpn.exe"));
            }
        }
        candidates.push(PathBuf::from(r"C:\Program Files\OpenVPN\bin\openvpn.exe"));
        for c in candidates {
            if c.is_absolute() && c.is_file() {
                return Ok(c);
            }
        }
        bail!("OpenVPN is not installed")
    }
    #[cfg(not(windows))]
    {
        for c in ["/usr/sbin/openvpn", "/usr/bin/openvpn"] {
            if Path::new(c).is_file() {
                return Ok(PathBuf::from(c));
            }
        }
        bail!("OpenVPN is not installed")
    }
}

/// A folder only administrators / root can write.
fn private_dir() -> anyhow::Result<PathBuf> {
    #[cfg(windows)]
    {
        let base = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| r"C:\ProgramData".into());
        let dir = base.join("Riptor").join("vpn");
        std::fs::create_dir_all(&dir)?;
        // Replace inherited permissions: SYSTEM and Administrators only.
        let icacls =
            Path::new(&std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into()))
                .join("System32")
                .join("icacls.exe");
        let ok = Command::new(icacls)
            .arg(&dir)
            .args([
                "/inheritance:r",
                "/grant:r",
                "*S-1-5-18:(OI)(CI)F",
                "*S-1-5-32-544:(OI)(CI)F",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .context("running icacls")?
            .success();
        ensure!(ok, "could not restrict permissions on {}", dir.display());
        Ok(dir)
    }
    #[cfg(not(windows))]
    {
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        let dir = PathBuf::from("/run/riptor-vpn");
        if !dir.exists() {
            std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        }
        let meta = std::fs::symlink_metadata(&dir)?;
        ensure!(
            meta.is_dir() && !meta.file_type().is_symlink(),
            "{} is not a plain folder",
            dir.display()
        );
        std::fs::set_permissions(&dir, std::fs::Permissions::from_mode(0o700))?;
        Ok(dir)
    }
}
