//! Linux: the management socket and WireGuard.

use std::io::{BufRead, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

/// The user pkexec ran us for. pkexec sets this; the caller can't choose it.
pub fn caller_uid() -> Result<u32, String> {
    let uid: u32 = std::env::var("PKEXEC_UID")
        .map_err(|_| "not started through pkexec".to_string())?
        .parse()
        .map_err(|_| "bad PKEXEC_UID".to_string())?;
    if uid == 0 {
        return Err("refusing to run for root".into());
    }
    Ok(uid)
}

/// `/run/riptor-vpn-sock/<uid>/mgmt.sock`; Riptor computes the same path.
pub fn socket_path(uid: u32) -> PathBuf {
    PathBuf::from(format!("/run/riptor-vpn-sock/{uid}/mgmt.sock"))
}

fn plain_dir(dir: &Path, mode: u32) -> std::io::Result<()> {
    if !dir.exists() {
        std::fs::DirBuilder::new().mode(mode).create(dir)?;
    }
    let meta = std::fs::symlink_metadata(dir)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(std::io::Error::other(format!(
            "{} is not a plain folder",
            dir.display()
        )));
    }
    std::fs::set_permissions(dir, std::fs::Permissions::from_mode(mode))
}

/// Root-owned, so the user can't swap the socket for a symlink.
pub fn socket_dir(uid: u32) -> std::io::Result<PathBuf> {
    let parent = Path::new("/run/riptor-vpn-sock");
    plain_dir(parent, 0o711)?;
    let dir = parent.join(uid.to_string());
    plain_dir(&dir, 0o711)?;
    std::os::unix::fs::chown(&dir, Some(0), Some(0))?;
    let _ = std::fs::remove_file(dir.join("mgmt.sock"));
    Ok(dir)
}

/// Waits for OpenVPN to create the socket, then hands it to `uid`, mode 0600.
pub fn claim_socket(path: &Path, uid: u32, timeout: Duration) -> std::io::Result<()> {
    let started = Instant::now();
    loop {
        if let Ok(meta) = std::fs::symlink_metadata(path) {
            use std::os::unix::fs::FileTypeExt;
            if !meta.file_type().is_socket() {
                return Err(std::io::Error::other("management path is not a socket"));
            }
            std::os::unix::fs::lchown(path, Some(uid), None)?;
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
            return Ok(());
        }
        if started.elapsed() > timeout {
            return Err(std::io::Error::other(
                "OpenVPN did not create its management socket",
            ));
        }
        std::thread::sleep(Duration::from_millis(200));
    }
}

/// Never PATH.
pub fn wg_quick() -> Option<PathBuf> {
    ["/usr/bin/wg-quick", "/usr/sbin/wg-quick", "/bin/wg-quick"]
        .iter()
        .map(PathBuf::from)
        .find(|p| p.is_file() && std::fs::metadata(p).map(|m| m.uid() == 0).unwrap_or(false))
}

/// The interface is named after the file, so it's always `riptor0`.
pub const WG_CONF: &str = "/run/riptor-vpn/riptor0.conf";

fn wg(action: &str, exe: &Path) -> bool {
    Command::new(exe)
        .args([action, WG_CONF])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Stays up until stdin closes.
pub fn wireguard(sanitized: &str, exe: &Path) -> Result<(), String> {
    plain_dir(Path::new("/run/riptor-vpn"), 0o700).map_err(|e| e.to_string())?;
    // A leftover from a crash would make wg-quick refuse.
    if Path::new("/sys/class/net/riptor0").exists() {
        let _ = std::fs::write(WG_CONF, sanitized);
        wg("down", exe);
    }
    {
        let mut f = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(WG_CONF)
            .map_err(|e| e.to_string())?;
        f.write_all(sanitized.as_bytes())
            .map_err(|e| e.to_string())?;
    }
    if !wg("up", exe) {
        let _ = std::fs::remove_file(WG_CONF);
        return Err("wg-quick could not bring the tunnel up".into());
    }
    println!("up");
    let _ = std::io::stdout().flush();

    let mut line = String::new();
    let stdin = std::io::stdin();
    let mut lock = stdin.lock();
    while lock.read_line(&mut line).map(|n| n > 0).unwrap_or(false) {
        line.clear();
    }
    wg("down", exe);
    let _ = std::fs::remove_file(WG_CONF);
    Ok(())
}
