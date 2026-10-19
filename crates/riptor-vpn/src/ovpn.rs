//! OpenVPN profile sanitizing.
//!
//! A `.ovpn` file is a list of OpenVPN options, and some of them run programs
//! (`up`, `down`, `plugin`, ...) as the user that starts OpenVPN, which for a
//! VPN is an administrator. Imported profiles are therefore treated as
//! hostile. Only an allowlist of options survives:
//!
//! * options that run code or touch files are **removed** and reported (Proton's
//!   own Linux profiles contain `script-security 2` + `up`/`down` scripts, so
//!   rejecting them outright would refuse real profiles);
//! * options that reference files (`ca file.crt`, `auth-user-pass creds.txt`)
//!   are **refused**: certificates must be inline, and credentials come from
//!   the OS keyring through the management interface;
//! * routing and DNS options are **replaced** with Riptor's app-only settings:
//!   no pushed routes, no system DNS changes, and a low-priority default route
//!   on the tunnel that only adapter-pinned sockets (Riptor's) use;
//! * anything unknown is refused, so a new or unusual option is never silently
//!   trusted.
//!
//! The output is a canonical config that the privileged helper re-validates
//! with this same code before starting OpenVPN.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

/// Inline blocks we accept (PEM material only).
const INLINE_BLOCKS: &[&str] = &[
    "ca",
    "cert",
    "key",
    "tls-auth",
    "tls-crypt",
    "tls-crypt-v2",
    "extra-certs",
];

/// Options that are safe to keep as written (argument checks below).
const ALLOWED: &[&str] = &[
    "client",
    "dev",
    "dev-type",
    "proto",
    "remote",
    "remote-random",
    "remote-random-hostname",
    "resolv-retry",
    "nobind",
    "persist-key",
    "persist-tun",
    "cipher",
    "data-ciphers",
    "data-ciphers-fallback",
    "ncp-ciphers",
    "auth",
    "auth-user-pass",
    "auth-nocache",
    "remote-cert-tls",
    "verify-x509-name",
    "tls-client",
    "tls-version-min",
    "tls-version-max",
    "tls-cipher",
    "tls-ciphersuites",
    "tls-groups",
    "key-direction",
    "reneg-sec",
    "reneg-bytes",
    "verb",
    "mute",
    "mute-replay-warnings",
    "ping",
    "ping-restart",
    "ping-exit",
    "ping-timer-rem",
    "fast-io",
    "sndbuf",
    "rcvbuf",
    "mssfix",
    "tun-mtu",
    "tun-mtu-extra",
    "fragment",
    "explicit-exit-notify",
    "connect-retry",
    "connect-retry-max",
    "connect-timeout",
    "server-poll-timeout",
    "hand-window",
    "tran-window",
    "float",
    "pull",
    "allow-compression",
    "disable-dco",
    "peer-fingerprint",
];

/// Options that run programs or load code: removed, never executed.
const REMOVED_EXEC: &[&str] = &[
    "up",
    "down",
    "up-delay",
    "up-restart",
    "down-pre",
    "route-up",
    "route-pre-down",
    "ipchange",
    "tls-verify",
    "auth-user-pass-verify",
    "client-connect",
    "client-disconnect",
    "learn-address",
    "plugin",
    "script-security",
    "setenv",
    "setenv-safe",
    "pull-filter",
    "management",
    "management-client",
    "management-hold",
    "management-query-passwords",
    "management-external-key",
    "log",
    "log-append",
    "status",
    "writepid",
    "daemon",
    "cd",
    "chroot",
    "user",
    "group",
    "echo",
    "push-peer-info",
];

/// Routing/DNS options: replaced by Riptor's app-only settings.
const REMOVED_ROUTING: &[&str] = &[
    "redirect-gateway",
    "redirect-private",
    "route",
    "route-ipv6",
    "route-gateway",
    "route-metric",
    "route-delay",
    "route-method",
    "route-noexec",
    "route-nopull",
    "dhcp-option",
    "block-outside-dns",
    "register-dns",
    "ip-win32",
    "dhcp-renew",
    "dhcp-release",
    "block-ipv6",
    "ifconfig-ipv6",
    "tun-ipv6",
];

/// Options that would make OpenVPN read or write a local file.
const FILE_ARG: &[&str] = &[
    "ca",
    "cert",
    "key",
    "tls-auth",
    "tls-crypt",
    "tls-crypt-v2",
    "extra-certs",
    "pkcs12",
    "crl-verify",
    "dh",
    "secret",
    "config",
    "askpass",
    "auth-user-pass",
    "http-proxy-user-pass",
];

/// Appended to every sanitized profile.
const APP_ONLY: &str = "\
# --- added by Riptor: app-only VPN ---
# Never run scripts or plugins.
script-security 1
# Ignore routes and DNS the server pushes; the system keeps its own.
route-nopull
pull-filter ignore \"redirect-gateway\"
pull-filter ignore \"dhcp-option\"
pull-filter ignore \"block-outside-dns\"
pull-filter ignore \"route-ipv6\"
pull-filter ignore \"ifconfig-ipv6\"
# A default route on the tunnel with a very high metric: the system keeps
# preferring its normal connection, while sockets pinned to this adapter
# (only Riptor's) are routed through the VPN.
route 0.0.0.0 0.0.0.0 vpn_gateway 9999
auth-nocache
";

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Remote {
    pub host: String,
    pub port: u16,
    pub proto: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sanitized {
    /// Canonical config to hand to OpenVPN.
    pub config: String,
    /// The profile wants a username/password (sent via the management interface).
    pub needs_credentials: bool,
    pub remotes: Vec<Remote>,
    /// Options that were dropped, for the import summary.
    pub removed: Vec<String>,
}

#[derive(Debug, thiserror::Error, PartialEq)]
pub enum OvpnError {
    #[error("profile is too large")]
    TooLarge,
    #[error("profile is not text")]
    NotText,
    #[error(
        "line {line}: \"{option}\" references a file; certificates must be inline in the profile"
    )]
    FileReference { line: usize, option: String },
    #[error("line {line}: unsupported option \"{option}\"")]
    Unsupported { line: usize, option: String },
    #[error("line {line}: invalid value for \"{option}\"")]
    BadValue { line: usize, option: String },
    #[error("inline <{0}> block is not closed")]
    UnclosedBlock(String),
    #[error("inline <{0}> blocks are not supported")]
    UnsupportedBlock(String),
    #[error("profile has no \"remote\" server")]
    NoRemote,
    #[error("profile has no inline <ca> certificate")]
    NoCa,
    #[error("only client profiles are supported")]
    NotClient,
}

pub const MAX_PROFILE_BYTES: usize = 256 * 1024;

/// Split an OpenVPN option line into words, honouring quotes.
fn words(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quote: Option<char> = None;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), c) if c == q => quote = None,
            (None, c) if c.is_whitespace() => {
                if !cur.is_empty() {
                    out.push(std::mem::take(&mut cur));
                }
            }
            (_, '\\') => {
                if let Some(n) = chars.next() {
                    cur.push(n);
                }
            }
            (_, c) => cur.push(c),
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

fn safe_token(s: &str) -> bool {
    !s.is_empty()
        && s.len() <= 255
        && s.chars()
            .all(|c| c.is_ascii_alphanumeric() || "._-:+/@=,[] ".contains(c))
}

/// Re-quote a value for the canonical config when it contains a space.
fn emit(arg: &str) -> String {
    if arg.contains(' ') {
        format!("\"{arg}\"")
    } else {
        arg.to_string()
    }
}

pub fn sanitize(input: &[u8]) -> Result<Sanitized, OvpnError> {
    if input.len() > MAX_PROFILE_BYTES {
        return Err(OvpnError::TooLarge);
    }
    let text = std::str::from_utf8(input).map_err(|_| OvpnError::NotText)?;
    let mut out = String::new();
    let mut removed = BTreeSet::new();
    let mut remotes = Vec::new();
    let mut needs_credentials = false;
    let mut is_client = false;
    let mut has_ca = false;
    let mut lines = text.lines().enumerate().map(|(i, l)| (i + 1, l.trim()));

    while let Some((n, line)) = lines.next() {
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        // Inline block: <tag> ... </tag>
        if let Some(tag) = line.strip_prefix('<').and_then(|l| l.strip_suffix('>')) {
            if tag.starts_with('/') {
                return Err(OvpnError::Unsupported {
                    line: n,
                    option: line.into(),
                });
            }
            if !INLINE_BLOCKS.contains(&tag) {
                return Err(OvpnError::UnsupportedBlock(tag.chars().take(40).collect()));
            }
            let end = format!("</{tag}>");
            let mut body = String::new();
            loop {
                let Some((_, l)) = lines.next() else {
                    return Err(OvpnError::UnclosedBlock(tag.into()));
                };
                if l == end {
                    break;
                }
                // PEM / static-key text only.
                if !l.chars().all(|c| c.is_ascii_graphic() || c == ' ') {
                    return Err(OvpnError::BadValue {
                        line: n,
                        option: tag.into(),
                    });
                }
                body.push_str(l);
                body.push('\n');
            }
            has_ca |= tag == "ca";
            out.push_str(&format!("<{tag}>\n{body}</{tag}>\n"));
            continue;
        }

        let w = words(line);
        let Some(opt) = w
            .first()
            .map(|s| s.trim_start_matches("--").to_ascii_lowercase())
        else {
            continue;
        };
        let args = &w[1..];

        if REMOVED_EXEC.contains(&opt.as_str()) || REMOVED_ROUTING.contains(&opt.as_str()) {
            removed.insert(opt);
            continue;
        }
        if opt == "auth-user-pass" {
            // Credentials come from the keyring via the management interface; never a file.
            needs_credentials = true;
            if !args.is_empty() {
                removed.insert("auth-user-pass file".into());
            }
            out.push_str("auth-user-pass\n");
            continue;
        }
        if FILE_ARG.contains(&opt.as_str()) {
            return Err(OvpnError::FileReference {
                line: n,
                option: opt,
            });
        }
        if !ALLOWED.contains(&opt.as_str()) {
            return Err(OvpnError::Unsupported {
                line: n,
                option: opt.chars().take(60).collect(),
            });
        }
        if !args.iter().all(|a| safe_token(a)) {
            return Err(OvpnError::BadValue {
                line: n,
                option: opt,
            });
        }
        match opt.as_str() {
            "client" => is_client = true,
            "dev" => {
                if !args.first().is_some_and(|d| d.starts_with("tun")) {
                    return Err(OvpnError::BadValue {
                        line: n,
                        option: opt,
                    });
                }
            }
            "remote" => {
                let host = args.first().ok_or(OvpnError::BadValue {
                    line: n,
                    option: opt.clone(),
                })?;
                let port = match args.get(1) {
                    Some(p) => p.parse().map_err(|_| OvpnError::BadValue {
                        line: n,
                        option: opt.clone(),
                    })?,
                    None => 1194,
                };
                remotes.push(Remote {
                    host: host.clone(),
                    port,
                    proto: args.get(2).cloned(),
                });
            }
            _ => {}
        }
        let mut canonical = opt.clone();
        for a in args {
            canonical.push(' ');
            canonical.push_str(&emit(a));
        }
        out.push_str(&canonical);
        out.push('\n');
    }

    if !is_client {
        return Err(OvpnError::NotClient);
    }
    if remotes.is_empty() {
        return Err(OvpnError::NoRemote);
    }
    if !has_ca {
        return Err(OvpnError::NoCa);
    }
    out.push_str(APP_ONLY);
    Ok(Sanitized {
        config: out,
        needs_credentials,
        remotes,
        removed: removed.into_iter().collect(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const CA: &str = "<ca>\n-----BEGIN CERTIFICATE-----\nMIIBszCCAVmgAwIBAgIUQ\n-----END CERTIFICATE-----\n</ca>\n";

    /// Shaped like a Proton OpenVPN profile for Linux.
    fn proton() -> String {
        format!(
            "# ProtonVPN config\nclient\ndev tun\nproto udp\nremote 185.159.157.1 51820\nremote 185.159.157.1 1194\nremote-random\nresolv-retry infinite\nnobind\ncipher AES-256-GCM\nsetenv CLIENT_CERT 0\ntun-mtu 1500\nmssfix 0\npersist-key\npersist-tun\nreneg-sec 0\nremote-cert-tls server\nauth-user-pass\npull\nfast-io\nscript-security 2\nup /etc/openvpn/update-resolv-conf\ndown /etc/openvpn/update-resolv-conf\n{CA}key-direction 1\n<tls-crypt>\n-----BEGIN OpenVPN Static key V1-----\nabcdef\n-----END OpenVPN Static key V1-----\n</tls-crypt>\n"
        )
    }

    #[test]
    fn proton_profile_becomes_app_only_and_script_free() {
        let s = sanitize(proton().as_bytes()).unwrap();
        assert!(s.needs_credentials);
        assert_eq!(s.remotes.len(), 2);
        assert_eq!(s.removed, vec!["down", "script-security", "setenv", "up"]);
        // No way to run anything.
        for bad in [
            "update-resolv-conf",
            "script-security 2",
            "\nup ",
            "\ndown ",
        ] {
            assert!(!s.config.contains(bad), "{bad:?} survived");
        }
        // App-only settings are enforced.
        for must in [
            "script-security 1",
            "route-nopull",
            "pull-filter ignore \"redirect-gateway\"",
            "vpn_gateway 9999",
        ] {
            assert!(s.config.contains(must), "{must:?} missing");
        }
        assert!(s.config.contains("<tls-crypt>"));
        // Re-sanitizing the output is stable (the helper re-validates it).
        let again = sanitize(s.config.as_bytes()).unwrap();
        assert_eq!(again.remotes, s.remotes);
    }

    #[test]
    fn hostile_profiles_are_refused_or_defused() {
        let base =
            |extra: &str| format!("client\ndev tun\nremote vpn.example 1194 udp\n{CA}{extra}");
        // Code execution is removed, not executed.
        for (evil, directive) in [
            ("plugin /tmp/evil.so", "plugin"),
            ("tls-verify /tmp/x.sh", "tls-verify"),
            ("route-up \"/bin/sh -c id\"", "route-up"),
            ("management 0.0.0.0 7505", "management"),
            ("up C:\\evil.bat", "up"),
        ] {
            let s = sanitize(base(evil).as_bytes()).unwrap();
            assert!(
                s.removed.contains(&directive.to_string()),
                "{evil}: not reported"
            );
            assert!(
                !s.config
                    .lines()
                    .any(|l| l.split(' ').next() == Some(directive)),
                "{evil}: survived"
            );
            assert!(
                !s.config.contains("evil") && !s.config.contains("/bin/sh"),
                "{evil}: payload survived"
            );
        }
        // Values with spaces survive a round trip, re-quoted.
        let s = sanitize(base("verify-x509-name \"CN=vpn example\" name").as_bytes()).unwrap();
        assert!(s
            .config
            .contains("verify-x509-name \"CN=vpn example\" name"));
        // Files are refused.
        assert!(matches!(
            sanitize(base("ca /etc/ssl/x.crt").as_bytes()),
            Err(OvpnError::FileReference { .. })
        ));
        assert!(matches!(
            sanitize(base("config other.ovpn").as_bytes()),
            Err(OvpnError::FileReference { .. })
        ));
        // Credentials file is dropped, never read.
        let s = sanitize(base("auth-user-pass C:\\secrets.txt").as_bytes()).unwrap();
        assert!(s.needs_credentials && !s.config.contains("secrets"));
        // Unknown options and odd values are refused.
        assert!(matches!(
            sanitize(base("tun-mtu 1500; rm -rf /").as_bytes()),
            Err(OvpnError::BadValue { .. })
        ));
        assert!(matches!(
            sanitize(base("some-new-option x").as_bytes()),
            Err(OvpnError::Unsupported { .. })
        ));
        assert!(matches!(
            sanitize(base("dev tap").as_bytes()),
            Err(OvpnError::BadValue { .. })
        ));
        assert!(matches!(
            sanitize(base("<connection>\nremote x\n</connection>").as_bytes()),
            Err(OvpnError::UnsupportedBlock(_))
        ));
        // Structure checks.
        assert_eq!(
            sanitize(b"client\ndev tun\n").unwrap_err(),
            OvpnError::NoRemote
        );
        assert_eq!(
            sanitize(b"dev tun\nremote x 1\n").unwrap_err(),
            OvpnError::NotClient
        );
        assert_eq!(
            sanitize(b"client\nremote x 1\n<ca>\nabc\n").unwrap_err(),
            OvpnError::UnclosedBlock("ca".into())
        );
        assert_eq!(
            sanitize(&vec![b'a'; MAX_PROFILE_BYTES + 1]).unwrap_err(),
            OvpnError::TooLarge
        );
        assert_eq!(sanitize(&[0xff, 0xfe]).unwrap_err(), OvpnError::NotText);
    }

    #[test]
    fn quoted_words() {
        assert_eq!(
            words(r#"verify-x509-name "CN=a b" name"#),
            vec!["verify-x509-name", "CN=a b", "name"]
        );
    }
}
