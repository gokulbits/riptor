//! Elevated VPN helper: `openvpn <config> <port|unix> <password-file>` or `wireguard <config>` (Linux).
#![forbid(unsafe_code)]

#[cfg(target_os = "linux")]
mod linux;

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use anyhow::{bail, ensure};

/// stderr isn't visible through UAC, so errors are exit codes.
mod code {
    pub const USAGE: i32 = 2;
    pub const BAD_CONFIG: i32 = 3;
    pub const NO_OPENVPN: i32 = 4;
    pub const START_FAILED: i32 = 5;
    #[cfg_attr(not(target_os = "linux"), allow(dead_code))]
    pub const NO_WIREGUARD: i32 = 6;
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("openvpn") if args.len() == 4 => openvpn(&args[1], &args[2], &args[3]),
        #[cfg(target_os = "linux")]
        Some("wireguard") if args.len() == 2 => wireguard(&args[1]),
        _ => {
            eprintln!(
                "usage: riptor-vpn-helper openvpn <config> <management-port | unix> <password-file>"
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

fn read_small(p: &str, max: usize) -> anyhow::Result<Vec<u8>> {
    let len = std::fs::metadata(p)?.len() as usize;
    ensure!(len <= max, "{p} is too large");
    Ok(std::fs::read(p)?)
}

/// Where OpenVPN's management interface listens.
enum Mgmt {
    Tcp(u16),
    #[cfg(target_os = "linux")]
    Unix {
        uid: u32,
        path: PathBuf,
    },
}

#[cfg(target_os = "linux")]
fn wireguard(config: &str) -> Res {
    let raw =
        read_small(config, riptor_vpn::wg::MAX_PROFILE_BYTES).map_err(fail(code::BAD_CONFIG))?;
    let sanitized = riptor_vpn::wg::sanitize(&raw).map_err(|e| (code::BAD_CONFIG, e.into()))?;
    let exe = linux::wg_quick().ok_or_else(|| {
        (
            code::NO_WIREGUARD,
            anyhow::anyhow!("wg-quick is not installed"),
        )
    })?;
    linux::wireguard(&sanitized.config, &exe).map_err(|e| (code::START_FAILED, anyhow::anyhow!(e)))
}

fn openvpn(config: &str, port: &str, pw_file: &str) -> Res {
    let mgmt = match port {
        #[cfg(target_os = "linux")]
        "unix" => {
            let uid = linux::caller_uid().map_err(|e| (code::USAGE, anyhow::anyhow!(e)))?;
            linux::socket_dir(uid).map_err(|e| (code::START_FAILED, e.into()))?;
            Mgmt::Unix {
                uid,
                path: linux::socket_path(uid),
            }
        }
        p => Mgmt::Tcp(
            p.parse()
                .ok()
                .filter(|p| *p >= 1024)
                .ok_or_else(|| (code::USAGE, anyhow::anyhow!("bad port")))?,
        ),
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
    stop_previous(&dir, &exe);
    let tag = std::process::id().to_string();
    let cfg_path = dir.join(format!("riptor-{tag}.ovpn"));
    let pw_path = dir.join(format!("riptor-{tag}.pw"));
    std::fs::write(&cfg_path, sanitized.config).map_err(|e| (code::START_FAILED, e.into()))?;
    std::fs::write(&pw_path, format!("{password}\n"))
        .map_err(|e| (code::START_FAILED, e.into()))?;

    let mut cmd = Command::new(&exe);
    cmd.arg("--config").arg(&cfg_path).arg("--management");
    match &mgmt {
        Mgmt::Tcp(port) => {
            cmd.args(["127.0.0.1", &port.to_string()]);
        }
        #[cfg(target_os = "linux")]
        Mgmt::Unix { path, .. } => {
            cmd.arg(path).arg("unix");
        }
    }
    cmd.arg(&pw_path)
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
        // CREATE_NO_WINDOW | DETACHED_PROCESS
        cmd.creation_flags(0x0800_0000 | 0x0000_0008);
    }
    let child = cmd
        .spawn()
        .map_err(|e| (code::START_FAILED, anyhow::anyhow!("starting OpenVPN: {e}")))?;
    let _ = std::fs::write(dir.join(PID_FILE), child.id().to_string());

    // Once the management interface is up OpenVPN has read the files.
    let ready = match &mgmt {
        Mgmt::Tcp(port) => {
            let started = Instant::now();
            loop {
                if std::net::TcpStream::connect_timeout(
                    &([127, 0, 0, 1], *port).into(),
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
            }
        }
        #[cfg(target_os = "linux")]
        Mgmt::Unix { uid, path } => {
            linux::claim_socket(path, *uid, Duration::from_secs(15)).is_ok()
        }
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

const PID_FILE: &str = "openvpn.pid";

/// The OpenVPN we started last time, if Riptor never got to stop it.
fn stop_previous(dir: &Path, exe: &Path) {
    let pid_file = dir.join(PID_FILE);
    let pid = std::fs::read_to_string(&pid_file)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok());
    let _ = std::fs::remove_file(&pid_file);
    let Some(pid) = pid else { return };
    #[cfg(windows)]
    {
        let _ = exe;
        let sys =
            Path::new(&std::env::var_os("SystemRoot").unwrap_or_else(|| r"C:\Windows".into()))
                .join("System32");
        let listed = Command::new(sys.join("tasklist.exe"))
            .args(["/FI", &format!("PID eq {pid}"), "/FO", "CSV", "/NH"])
            .stderr(Stdio::null())
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).to_ascii_lowercase())
            .unwrap_or_default();
        if listed.starts_with("\"openvpn.exe\"") {
            let _ = Command::new(sys.join("taskkill.exe"))
                .args(["/PID", &pid.to_string(), "/F"])
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status();
        }
    }
    #[cfg(not(windows))]
    {
        if std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|p| p == exe) {
            let _ = Command::new("/bin/kill")
                .arg(pid.to_string())
                .stderr(Stdio::null())
                .status();
        }
    }
}

/// Never PATH.
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

fn private_dir() -> anyhow::Result<PathBuf> {
    #[cfg(windows)]
    {
        let base = std::env::var_os("ProgramData")
            .map(PathBuf::from)
            .unwrap_or_else(|| r"C:\ProgramData".into());
        let dir = base.join("Riptor").join("vpn");
        std::fs::create_dir_all(&dir)?;
        // SYSTEM and Administrators only.
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
            .map_err(|e| anyhow::anyhow!("running icacls: {e}"))?
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
