//! Save folders must never be somewhere the OS would run or trust files from.

use std::path::{Component, Path, PathBuf};

use anyhow::{bail, ensure};

fn env_path(var: &str) -> Option<PathBuf> {
    std::env::var_os(var)
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

fn home() -> Option<PathBuf> {
    env_path(if cfg!(windows) { "USERPROFILE" } else { "HOME" })
}

/// Lexical, so paths that don't exist yet can be checked.
fn normalize(p: &Path) -> PathBuf {
    let mut out = PathBuf::new();
    for c in p.components() {
        match c {
            Component::ParentDir => {
                out.pop();
            }
            Component::CurDir => {}
            other => out.push(other.as_os_str()),
        }
    }
    out
}

fn starts_with(p: &Path, base: &Path) -> bool {
    if cfg!(windows) {
        let p = strip_verbatim(p).to_lowercase().replace('/', "\\");
        let b = strip_verbatim(base).to_lowercase().replace('/', "\\");
        let b = b.trim_end_matches('\\');
        p == b || p.starts_with(&format!("{b}\\"))
    } else {
        p.starts_with(base)
    }
}

/// `canonicalize` hands back `\\?\C:\...`, which no protected root is spelled as.
fn strip_verbatim(p: &Path) -> String {
    let s = p.to_string_lossy();
    match s.strip_prefix(r"\\?\UNC\") {
        Some(rest) => format!(r"\\{rest}"),
        None => s.strip_prefix(r"\\?\").unwrap_or(&s).to_string(),
    }
}

/// Deepest ancestor that exists, so a folder not created yet can still be
/// resolved through the links above it.
fn resolved(p: &Path) -> Option<PathBuf> {
    let mut tail = Vec::new();
    let mut cur = p;
    loop {
        if let Ok(real) = cur.canonicalize() {
            let mut out = real;
            for part in tail.iter().rev() {
                out.push(part);
            }
            return Some(out);
        }
        tail.push(cur.file_name()?.to_os_string());
        cur = cur.parent()?;
    }
}

/// Only a plain drive or a network share. Verbatim and device spellings name the
/// same folders by another string, which the checks below would not recognise.
#[cfg(windows)]
fn check_prefix(p: &Path) -> anyhow::Result<()> {
    use std::path::Prefix;
    let Some(Component::Prefix(pre)) = p.components().next() else {
        bail!("save folder must be an absolute path");
    };
    match pre.kind() {
        Prefix::Disk(_) => Ok(()),
        Prefix::UNC(_, share) => {
            // \\host\C$ is the whole of someone's drive.
            let s = share.to_string_lossy();
            ensure!(
                !(s.len() == 2 && s.ends_with('$')),
                "an administrative share cannot be used as a save folder"
            );
            Ok(())
        }
        _ => bail!("write the save folder as a normal drive or network path"),
    }
}

#[cfg(not(windows))]
fn check_prefix(_p: &Path) -> anyhow::Result<()> {
    Ok(())
}

fn protected_roots(data_dir: Option<&Path>) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = Vec::new();
    if let Some(d) = data_dir {
        v.push(d.to_path_buf());
    }
    if cfg!(windows) {
        for var in [
            "SystemRoot",
            "windir",
            "ProgramFiles",
            "ProgramFiles(x86)",
            "ProgramW6432",
            "ProgramData",
        ] {
            v.extend(env_path(var));
        }
        if let Some(app) = env_path("APPDATA") {
            v.push(app.join("Microsoft").join("Windows").join("Start Menu"));
        }
        v.push(PathBuf::from(r"C:\Windows"));
        v.push(PathBuf::from(r"C:\Program Files"));
        v.push(PathBuf::from(r"C:\Program Files (x86)"));
        v.push(PathBuf::from(r"C:\ProgramData"));
        if let Some(h) = home() {
            v.push(h.join(".ssh"));
        }
    } else {
        for p in [
            "/etc", "/usr", "/bin", "/sbin", "/lib", "/lib32", "/lib64", "/boot", "/root",
            "/var/lib", "/opt", "/proc", "/sys", "/dev",
        ] {
            v.push(PathBuf::from(p));
        }
        if let Some(h) = home() {
            for rel in [
                ".ssh",
                ".gnupg",
                ".config/autostart",
                ".config/systemd",
                ".local/share/applications",
                ".local/bin",
                ".bashrc.d",
                ".profile.d",
            ] {
                v.push(h.join(rel));
            }
            if let Some(x) = env_path("XDG_CONFIG_HOME") {
                v.push(x.join("autostart"));
                v.push(x.join("systemd"));
            }
        }
    }
    v
}

pub fn check_save_root(p: &Path, data_dir: Option<&Path>) -> anyhow::Result<()> {
    ensure!(p.is_absolute(), "save folder must be an absolute path");
    check_prefix(p)?;
    let n = normalize(p);
    ensure!(
        n.parent().is_some(),
        "a drive or filesystem root cannot be used as a save folder"
    );
    // Both spellings: the text given, and what links and short names resolve to.
    let mine = [Some(normalize(p)), resolved(&n)];
    for root in protected_roots(data_dir) {
        let lexical = normalize(&root);
        let theirs = [Some(lexical.clone()), resolved(&lexical)];
        let hit = mine
            .iter()
            .flatten()
            .any(|m| theirs.iter().flatten().any(|t| starts_with(m, t)));
        if hit {
            bail!(
                "{} is a protected system location and cannot be used as a save folder",
                n.display()
            );
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_dangerous_folders() {
        let data = std::env::temp_dir().join("riptor-data");
        assert!(check_save_root(Path::new("relative/dir"), None).is_err());
        assert!(check_save_root(&data.join("session"), Some(&data)).is_err());
        assert!(check_save_root(&std::env::temp_dir().join("dl"), Some(&data)).is_ok());
        if cfg!(windows) {
            assert!(check_save_root(Path::new(r"C:\"), None).is_err());
            assert!(check_save_root(Path::new(r"c:\windows\system32"), None).is_err());
            assert!(check_save_root(Path::new(r"C:\Program Files\x"), None).is_err());
            assert!(
                check_save_root(Path::new(r"C:\Users\x\Downloads\..\..\..\Windows"), None).is_err()
            );
            if let Some(app) = env_path("APPDATA") {
                let startup = app.join(r"Microsoft\Windows\Start Menu\Programs\Startup");
                assert!(check_save_root(&startup, None).is_err());
            }
        } else {
            assert!(check_save_root(Path::new("/"), None).is_err());
            assert!(check_save_root(Path::new("/etc/cron.d"), None).is_err());
            assert!(check_save_root(Path::new("/home/x/Downloads/../../../etc"), None).is_err());
            if let Some(h) = home() {
                assert!(check_save_root(&h.join(".config/autostart"), None).is_err());
                assert!(check_save_root(&h.join("Downloads"), None).is_ok());
            }
        }
    }
}
