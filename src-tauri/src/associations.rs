//! `.torrent` and `magnet:` handling. [`register`] only adds Riptor to
//! "Open with"; [`make_default`] is the user's explicit choice. Per-user only.

use serde::Serialize;

#[derive(Clone, Copy, Debug, Default, Serialize)]
pub struct AssociationStatus {
    pub registered: bool,
    pub torrent_default: bool,
    pub magnet_default: bool,
}

fn exe() -> anyhow::Result<std::path::PathBuf> {
    let p = std::env::current_exe()?;
    Ok(dunce_canonical(&p))
}

/// Shells reject `\\?\` paths.
fn dunce_canonical(p: &std::path::Path) -> std::path::PathBuf {
    let s = p.to_string_lossy();
    std::path::PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s).to_string())
}

#[cfg(windows)]
mod imp {
    use super::*;
    use winreg::enums::*;
    use winreg::RegKey;

    const PROG_TORRENT: &str = "Riptor.TorrentFile";
    const PROG_MAGNET: &str = "Riptor.MagnetLink";
    const CAPS: &str = r"Software\Riptor\Capabilities";

    fn open_command(exe: &std::path::Path) -> String {
        format!("\"{}\" \"%1\"", exe.display())
    }

    fn write_progid(
        classes: &RegKey,
        id: &str,
        desc: &str,
        exe: &std::path::Path,
        url: bool,
    ) -> anyhow::Result<()> {
        let (k, _) = classes.create_subkey(id)?;
        k.set_value("", &desc)?;
        if url {
            k.set_value("URL Protocol", &"")?;
        }
        let (icon, _) = k.create_subkey("DefaultIcon")?;
        icon.set_value("", &format!("\"{}\",0", exe.display()))?;
        let (cmd, _) = k.create_subkey(r"shell\open\command")?;
        cmd.set_value("", &open_command(exe))?;
        Ok(())
    }

    pub fn register() -> anyhow::Result<()> {
        let exe = exe()?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (classes, _) = hkcu.create_subkey(r"Software\Classes")?;
        write_progid(&classes, PROG_TORRENT, "BitTorrent file", &exe, false)?;
        write_progid(&classes, PROG_MAGNET, "Magnet link", &exe, true)?;
        let (owp, _) = classes.create_subkey(r".torrent\OpenWithProgids")?;
        owp.set_raw_value(
            PROG_TORRENT,
            &winreg::RegValue {
                bytes: vec![],
                vtype: REG_NONE,
            },
        )?;
        let (caps, _) = hkcu.create_subkey(CAPS)?;
        caps.set_value("ApplicationName", &"Riptor")?;
        caps.set_value("ApplicationDescription", &"Lightweight BitTorrent client")?;
        let (fa, _) = caps.create_subkey("FileAssociations")?;
        fa.set_value(".torrent", &PROG_TORRENT)?;
        let (ua, _) = caps.create_subkey("URLAssociations")?;
        ua.set_value("magnet", &PROG_MAGNET)?;
        let (reg, _) = hkcu.create_subkey(r"Software\RegisteredApplications")?;
        reg.set_value("Riptor", &CAPS)?;
        notify_shell();
        Ok(())
    }

    fn user_choice(path: &str) -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey(path)
            .ok()?
            .get_value::<String, _>("ProgId")
            .ok()
    }

    pub fn status() -> AssociationStatus {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let exe = exe().ok();
        let registered = hkcu
            .open_subkey(format!(
                r"Software\Classes\{PROG_TORRENT}\shell\open\command"
            ))
            .and_then(|k| k.get_value::<String, _>(""))
            .is_ok_and(|v| exe.as_ref().is_some_and(|e| v == open_command(e)));
        let torrent_default = user_choice(
            r"Software\Microsoft\Windows\CurrentVersion\Explorer\FileExts\.torrent\UserChoice",
        )
        .is_some_and(|p| p == PROG_TORRENT);
        let magnet_user_choice = user_choice(
            r"Software\Microsoft\Windows\Shell\Associations\UrlAssociations\magnet\UserChoice",
        );
        let magnet_default = match magnet_user_choice {
            Some(p) => p == PROG_MAGNET,
            None => hkcu
                .open_subkey(r"Software\Classes\magnet\shell\open\command")
                .and_then(|k| k.get_value::<String, _>(""))
                .is_ok_and(|v| exe.as_ref().is_some_and(|e| v == open_command(e))),
        };
        AssociationStatus {
            registered,
            torrent_default,
            magnet_default,
        }
    }

    pub fn make_default() -> anyhow::Result<()> {
        register()?;
        let exe = exe()?;
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let (classes, _) = hkcu.create_subkey(r"Software\Classes")?;
        // Windows may still ask the user once.
        write_progid(&classes, "magnet", "URL:Magnet link", &exe, true)?;
        notify_shell();
        // Windows only lets the user pick the .torrent default.
        open_default_apps();
        Ok(())
    }

    pub fn open_default_apps() {
        let _ = std::process::Command::new("explorer.exe")
            .arg("ms-settings:defaultapps")
            .spawn();
    }

    #[allow(unsafe_code)]
    fn notify_shell() {
        use windows_sys::Win32::UI::Shell::{SHChangeNotify, SHCNE_ASSOCCHANGED, SHCNF_IDLIST};
        // SAFETY: null items are documented for SHCNE_ASSOCCHANGED.
        unsafe {
            SHChangeNotify(
                SHCNE_ASSOCCHANGED as i32,
                SHCNF_IDLIST,
                std::ptr::null(),
                std::ptr::null(),
            )
        };
    }
}

#[cfg(target_os = "linux")]
mod imp {
    use super::*;
    use std::process::Command;

    const DESKTOP: &str = "riptor.desktop";
    const MIMES: [&str; 2] = ["application/x-bittorrent", "x-scheme-handler/magnet"];

    fn apps_dir() -> anyhow::Result<std::path::PathBuf> {
        let base = std::env::var_os("XDG_DATA_HOME")
            .map(std::path::PathBuf::from)
            .or_else(|| {
                std::env::var_os("HOME").map(|h| std::path::PathBuf::from(h).join(".local/share"))
            })
            .ok_or_else(|| anyhow::anyhow!("no home folder"))?;
        Ok(base.join("applications"))
    }

    fn launcher() -> anyhow::Result<std::path::PathBuf> {
        std::env::var_os("APPIMAGE")
            .map(std::path::PathBuf::from)
            .map_or_else(exe, Ok)
    }

    pub fn register() -> anyhow::Result<()> {
        let dir = apps_dir()?;
        std::fs::create_dir_all(&dir)?;
        let exe = launcher()?;
        let quoted = format!("\"{}\"", exe.display().to_string().replace('"', "\\\""));
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=Riptor\nComment=Lightweight BitTorrent client\nExec={quoted} %U\nIcon=riptor\nTerminal=false\nCategories=Network;FileTransfer;P2P;\nMimeType={};\n",
            MIMES.join(";")
        );
        std::fs::write(dir.join(DESKTOP), entry)?;
        let _ = Command::new("update-desktop-database").arg(&dir).status();
        Ok(())
    }

    fn query(mime: &str) -> bool {
        Command::new("xdg-mime")
            .args(["query", "default", mime])
            .output()
            .is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == DESKTOP)
    }

    pub fn status() -> AssociationStatus {
        AssociationStatus {
            registered: apps_dir().is_ok_and(|d| d.join(DESKTOP).exists()),
            torrent_default: query(MIMES[0]),
            magnet_default: query(MIMES[1]),
        }
    }

    pub fn make_default() -> anyhow::Result<()> {
        register()?;
        for m in MIMES {
            let ok = Command::new("xdg-mime")
                .args(["default", DESKTOP, m])
                .status()?
                .success();
            anyhow::ensure!(ok, "xdg-mime could not set the default for {m}");
        }
        Ok(())
    }

    pub fn open_default_apps() {}
}

#[cfg(not(any(windows, target_os = "linux")))]
mod imp {
    use super::*;
    pub fn register() -> anyhow::Result<()> {
        Ok(())
    }
    pub fn status() -> AssociationStatus {
        AssociationStatus::default()
    }
    pub fn make_default() -> anyhow::Result<()> {
        anyhow::bail!("not supported on this platform")
    }
    pub fn open_default_apps() {}
}

pub use imp::{make_default, open_default_apps, register, status};
