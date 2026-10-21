//! The network the computer is on: its name and whether it's metered.

use serde::Serialize;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct NetInfo {
    /// `None` when the OS doesn't say.
    pub metered: Option<bool>,
    /// What the OS calls the current network (Wi-Fi name or connection name).
    pub names: Vec<String>,
}

const MAX_NAMES: usize = 8;
const MAX_NAME: usize = 128;

fn clean(name: &str) -> Option<String> {
    let n: String = name
        .chars()
        .filter(|c| !c.is_control())
        .take(MAX_NAME)
        .collect();
    let n = n.trim().to_string();
    (!n.is_empty()).then_some(n)
}

/// Blocking; call from a worker thread.
pub fn current() -> NetInfo {
    let mut info = query();
    info.names.truncate(MAX_NAMES);
    info
}

#[cfg(windows)]
fn query() -> NetInfo {
    use windows::Networking::Connectivity::{NetworkCostType, NetworkInformation};
    let Ok(p) = NetworkInformation::GetInternetConnectionProfile() else {
        return NetInfo::default();
    };
    let metered = p
        .GetConnectionCost()
        .and_then(|c| c.NetworkCostType())
        .ok()
        .and_then(|t| match t {
            NetworkCostType::Unrestricted => Some(false),
            NetworkCostType::Fixed | NetworkCostType::Variable => Some(true),
            _ => None,
        });
    let mut names: Vec<String> = p
        .GetNetworkNames()
        .map(|v| {
            v.into_iter()
                .filter_map(|n| clean(&n.to_string()))
                .collect()
        })
        .unwrap_or_default();
    if names.is_empty() {
        names.extend(p.ProfileName().ok().and_then(|n| clean(&n.to_string())));
    }
    NetInfo { metered, names }
}

#[cfg(target_os = "linux")]
fn query() -> NetInfo {
    const NM: &str = "org.freedesktop.NetworkManager";
    let metered =
        busctl(&["/org/freedesktop/NetworkManager", NM, "Metered"]).and_then(|v| parse_metered(&v));
    let names = busctl(&["/org/freedesktop/NetworkManager", NM, "PrimaryConnection"])
        .and_then(|v| parse_string(&v, 'o'))
        .filter(|p| p.starts_with("/org/freedesktop/NetworkManager/ActiveConnection/"))
        .and_then(|path| {
            busctl(&[
                path.as_str(),
                "org.freedesktop.NetworkManager.Connection.Active",
                "Id",
            ])
        })
        .and_then(|v| parse_string(&v, 's'))
        .and_then(|n| clean(&n))
        .into_iter()
        .collect();
    NetInfo { metered, names }
}

#[cfg(target_os = "linux")]
fn busctl(args: &[&str]) -> Option<String> {
    let out = std::process::Command::new("busctl")
        .args([
            "--system",
            "--timeout=2",
            "get-property",
            "org.freedesktop.NetworkManager",
        ])
        .args(args)
        .stdin(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .output()
        .ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_string())
}

#[cfg(not(any(windows, target_os = "linux")))]
fn query() -> NetInfo {
    NetInfo::default()
}

/// NetworkManager's NMMetered: 1 yes, 2 no, 3 guessed yes, 4 guessed no.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_metered(v: &str) -> Option<bool> {
    match v.strip_prefix("u ")?.trim() {
        "1" | "3" => Some(true),
        "2" | "4" => Some(false),
        _ => None,
    }
}

/// busctl prints `s "Home Wi-Fi"` or `o "/path"`.
#[cfg_attr(not(target_os = "linux"), allow(dead_code))]
fn parse_string(v: &str, kind: char) -> Option<String> {
    let rest = v.strip_prefix(kind)?.strip_prefix(' ')?;
    let inner = rest.strip_prefix('"')?.strip_suffix('"')?;
    Some(inner.replace("\\\"", "\"").replace("\\\\", "\\"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busctl_replies_parse() {
        assert_eq!(parse_metered("u 1"), Some(true));
        assert_eq!(parse_metered("u 4"), Some(false));
        assert_eq!(parse_metered("u 0"), None);
        assert_eq!(parse_metered("garbage"), None);
        assert_eq!(
            parse_string("s \"Home Wi-Fi\"", 's').as_deref(),
            Some("Home Wi-Fi")
        );
        assert_eq!(
            parse_string(
                "o \"/org/freedesktop/NetworkManager/ActiveConnection/3\"",
                'o'
            )
            .as_deref(),
            Some("/org/freedesktop/NetworkManager/ActiveConnection/3")
        );
        assert_eq!(parse_string("s Home", 's'), None);
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean("  Café\u{0}\n "), Some("Café".into()));
        assert_eq!(clean("\u{7}"), None);
        assert_eq!(clean(&"x".repeat(500)).unwrap().len(), MAX_NAME);
    }

    #[test]
    fn asking_the_os_never_panics() {
        let _ = current();
    }
}
