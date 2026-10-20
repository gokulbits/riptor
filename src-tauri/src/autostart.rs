//! Start Riptor in the tray when the user signs in.

pub const FLAG: &str = "--minimized";

#[cfg(windows)]
mod imp {
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE};
    use winreg::RegKey;

    const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const NAME: &str = "Riptor";

    pub fn enabled() -> bool {
        RegKey::predef(HKEY_CURRENT_USER)
            .open_subkey_with_flags(RUN, KEY_READ)
            .and_then(|k| k.get_value::<String, _>(NAME))
            .is_ok()
    }

    pub fn set(on: bool) -> anyhow::Result<()> {
        let (key, _) = RegKey::predef(HKEY_CURRENT_USER).create_subkey(RUN)?;
        if on {
            let exe = std::env::current_exe()?;
            key.set_value(NAME, &format!("\"{}\" {}", exe.display(), super::FLAG))?;
        } else {
            let key =
                RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(RUN, KEY_SET_VALUE)?;
            match key.delete_value(NAME) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => return Err(e.into()),
                _ => {}
            }
        }
        Ok(())
    }
}

#[cfg(not(windows))]
mod imp {
    use std::path::PathBuf;

    fn entry() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("autostart").join("riptor.desktop"))
    }

    pub fn enabled() -> bool {
        entry().is_some_and(|p| p.is_file())
    }

    pub fn set(on: bool) -> anyhow::Result<()> {
        let path = entry().ok_or_else(|| anyhow::anyhow!("no home folder"))?;
        if on {
            let exe = std::env::current_exe()?;
            let exe = exe.to_string_lossy();
            anyhow::ensure!(
                !exe.chars().any(|c| c.is_control() || c == '"' || c == '\\'),
                "Riptor's path can't be used in an autostart entry"
            );
            std::fs::create_dir_all(path.parent().unwrap_or(&path))?;
            std::fs::write(
                &path,
                format!(
                    "[Desktop Entry]\nType=Application\nName=Riptor\nExec=\"{exe}\" {}\nX-GNOME-Autostart-enabled=true\n",
                    super::FLAG
                ),
            )?;
        } else if path.exists() {
            std::fs::remove_file(path)?;
        }
        Ok(())
    }
}

pub use imp::{enabled, set};

pub fn started_minimized() -> bool {
    std::env::args().skip(1).any(|a| a == FLAG)
}
