//! OpenVPN's management protocol. Credentials go over it from the keyring, so
//! they never touch a file.

use std::net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// `>STATE:time,STATE,desc,local_ip,remote_ip,...`
    State {
        state: String,
        local_ip: Option<Ipv4Addr>,
        remote: Option<String>,
    },
    /// `>PASSWORD:Need 'Auth' username/password`
    NeedCredentials,
    /// `>PASSWORD:Verification Failed: 'Auth'`
    AuthFailed,
    /// `>HOLD:Waiting for hold release`
    Hold,
    /// `>FATAL:...`
    Fatal(String),
    /// `ENTER PASSWORD:`
    ManagementPassword,
    Other,
}

pub fn parse(line: &str) -> Line {
    let line = line.trim_end_matches(['\r', '\n']);
    if let Some(rest) = line.strip_prefix(">STATE:") {
        let f: Vec<&str> = rest.split(',').collect();
        return Line::State {
            state: f.get(1).copied().unwrap_or("").to_string(),
            local_ip: f.get(3).and_then(|s| s.parse().ok()),
            remote: f.get(4).filter(|s| !s.is_empty()).map(|s| s.to_string()),
        };
    }
    if let Some(rest) = line.strip_prefix(">PASSWORD:") {
        if rest.starts_with("Need 'Auth'") {
            return Line::NeedCredentials;
        }
        if rest.starts_with("Verification Failed") {
            return Line::AuthFailed;
        }
    }
    if line.starts_with(">HOLD:") {
        return Line::Hold;
    }
    if let Some(rest) = line.strip_prefix(">FATAL:") {
        return Line::Fatal(rest.chars().take(300).collect());
    }
    if line.starts_with("ENTER PASSWORD:") {
        return Line::ManagementPassword;
    }
    Line::Other
}

/// Newlines are refused: they'd let a value inject another command.
pub fn quote(v: &str) -> Option<String> {
    if v.contains(['\n', '\r', '\0']) {
        return None;
    }
    Some(format!(
        "\"{}\"",
        v.replace('\\', "\\\\").replace('"', "\\\"")
    ))
}

pub fn credentials(username: &str, password: &str) -> Option<[String; 2]> {
    Some([
        format!("username \"Auth\" {}\n", quote(username)?),
        format!("password \"Auth\" {}\n", quote(password)?),
    ])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_lines() {
        assert_eq!(
            parse(">STATE:1700000000,CONNECTED,SUCCESS,10.96.0.7,185.159.157.1,1194,,\r\n"),
            Line::State {
                state: "CONNECTED".into(),
                local_ip: Some(Ipv4Addr::new(10, 96, 0, 7)),
                remote: Some("185.159.157.1".into())
            }
        );
        assert_eq!(
            parse(">STATE:1,WAIT,,,"),
            Line::State {
                state: "WAIT".into(),
                local_ip: None,
                remote: None
            }
        );
        assert_eq!(
            parse(">PASSWORD:Need 'Auth' username/password"),
            Line::NeedCredentials
        );
        assert_eq!(
            parse(">PASSWORD:Verification Failed: 'Auth'"),
            Line::AuthFailed
        );
        assert_eq!(parse(">HOLD:Waiting for hold release:0"), Line::Hold);
        assert_eq!(
            parse(">FATAL:cannot open TUN"),
            Line::Fatal("cannot open TUN".into())
        );
        assert_eq!(parse("ENTER PASSWORD:"), Line::ManagementPassword);
        assert_eq!(parse("SUCCESS: hold release succeeded"), Line::Other);
    }

    #[test]
    fn credentials_cannot_inject_commands() {
        let [u, p] = credentials(r#"us"er\x"#, "p w").unwrap();
        assert_eq!(u, "username \"Auth\" \"us\\\"er\\\\x\"\n");
        assert_eq!(p, "password \"Auth\" \"p w\"\n");
        assert!(credentials("user\nsignal SIGTERM", "x").is_none());
        assert!(credentials("u", "x\r\nhold release").is_none());
    }
}
