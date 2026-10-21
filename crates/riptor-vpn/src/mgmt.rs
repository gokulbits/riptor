//! OpenVPN's management protocol. Credentials go over it from the keyring, so
//! they never touch a file.

use std::net::Ipv4Addr;

#[derive(Clone, Debug, PartialEq)]
pub enum Line {
    /// `>STATE:time,STATE,desc,local_ip,remote_ip,...`
    State {
        state: String,
        /// Why, e.g. `tls-error` for RECONNECTING.
        detail: String,
        local_ip: Option<Ipv4Addr>,
        remote: Option<String>,
    },
    /// `>PASSWORD:Need 'Auth' username/password`
    NeedCredentials,
    /// `>PASSWORD:Verification Failed: 'Auth' ['reason']`
    AuthFailed(Option<String>),
    /// `>HOLD:Waiting for hold release`
    Hold,
    /// `>FATAL:...`
    Fatal(String),
    /// `>LOG:time,flags,message` once `log on` is sent.
    Log {
        fatal: bool,
        msg: String,
    },
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
            detail: f.get(2).copied().unwrap_or("").to_string(),
            local_ip: f.get(3).and_then(|s| s.parse().ok()),
            remote: f.get(4).filter(|s| !s.is_empty()).map(|s| s.to_string()),
        };
    }
    if let Some(rest) = line.strip_prefix(">PASSWORD:") {
        if rest.starts_with("Need 'Auth'") {
            return Line::NeedCredentials;
        }
        if rest.starts_with("Verification Failed") {
            let reason = rest
                .split_once("['")
                .and_then(|(_, r)| r.rsplit_once("']"))
                .map(|(r, _)| short(r))
                .filter(|r| !r.is_empty());
            return Line::AuthFailed(reason);
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
    if let Some(rest) = line.strip_prefix(">LOG:") {
        let mut f = rest.splitn(3, ',');
        let (_, flags, msg) = (f.next(), f.next().unwrap_or(""), f.next().unwrap_or(""));
        return Line::Log {
            fatal: flags.contains('F'),
            msg: short(msg),
        };
    }
    Line::Other
}

/// Server text shown in the UI: no control characters, bounded.
fn short(s: &str) -> String {
    s.chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .take(200)
        .collect::<String>()
        .trim()
        .to_string()
}

pub fn auth_failed_message(reason: Option<&str>) -> String {
    let base = "The VPN rejected the username or password (many providers use separate OpenVPN credentials, not your account login)";
    match reason {
        Some(r) => format!("{base}. The server said: {r}"),
        None => base.into(),
    }
}

/// Restarts allowed while connecting before giving up on this attempt.
pub const MAX_RESTARTS: u32 = 3;

#[derive(Debug, PartialEq)]
pub enum Verdict {
    /// Stop OpenVPN and show this.
    Failed(String),
    /// Still trying; show this as progress.
    Retrying(String),
}

/// Spots a connect attempt that can't succeed, so it needn't wait for the timeout.
#[derive(Default)]
pub struct ConnectWatch {
    restarts: u32,
    last_problem: Option<String>,
}

impl ConnectWatch {
    pub fn on(&mut self, line: &Line) -> Option<Verdict> {
        match line {
            Line::AuthFailed(reason) => Some(Verdict::Failed(auth_failed_message(reason.as_deref()))),
            Line::Fatal(msg) | Line::Log { fatal: true, msg } => Some(Verdict::Failed(msg.clone())),
            Line::Log { msg, .. } if msg.contains("AUTH_FAILED") => {
                let reason = msg
                    .split_once("AUTH_FAILED,")
                    .map(|(_, r)| r.trim())
                    .filter(|r| !r.is_empty());
                Some(Verdict::Failed(auth_failed_message(reason)))
            }
            Line::Log { msg, .. } if msg.contains("VERIFY ERROR") => Some(Verdict::Failed(format!(
                "The VPN server's certificate didn't verify ({msg}). The profile may be out of date; download a new one from your VPN provider."
            ))),
            Line::Log { msg, .. } if is_problem(msg) => {
                self.last_problem = Some(msg.clone());
                Some(Verdict::Retrying(format!("Connecting… {msg}")))
            }
            Line::State { state, detail, .. } if state == "RECONNECTING" => {
                self.restarts += 1;
                let why = self.last_problem.clone().unwrap_or_else(|| detail.clone());
                Some(if self.restarts >= MAX_RESTARTS {
                    Verdict::Failed(format!(
                        "The VPN server ended the connection {} times ({why}). Check the profile's OpenVPN username and password, or try another server.",
                        self.restarts
                    ))
                } else {
                    Verdict::Retrying(format!("Connecting… retrying ({why})"))
                })
            }
            _ => None,
        }
    }
}

fn is_problem(msg: &str) -> bool {
    let m = msg.to_ascii_lowercase();
    [
        "tls error",
        "tls_error",
        "refused",
        "reset",
        "unreachable",
        "timed out",
        "failed",
    ]
    .iter()
    .any(|p| m.contains(p))
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
                detail: "SUCCESS".into(),
                local_ip: Some(Ipv4Addr::new(10, 96, 0, 7)),
                remote: Some("185.159.157.1".into())
            }
        );
        assert_eq!(
            parse(">STATE:1,WAIT,,,"),
            Line::State {
                state: "WAIT".into(),
                detail: String::new(),
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
            Line::AuthFailed(None)
        );
        assert_eq!(
            parse(">PASSWORD:Verification Failed: 'Auth' ['Subscription expired\u{1b}[0m']"),
            Line::AuthFailed(Some("Subscription expired [0m".into()))
        );
        assert_eq!(
            parse(">LOG:1700000000,F,Exiting due to fatal error"),
            Line::Log {
                fatal: true,
                msg: "Exiting due to fatal error".into()
            }
        );
        assert_eq!(parse(">HOLD:Waiting for hold release:0"), Line::Hold);
        assert_eq!(
            parse(">FATAL:cannot open TUN"),
            Line::Fatal("cannot open TUN".into())
        );
        assert_eq!(parse("ENTER PASSWORD:"), Line::ManagementPassword);
        assert_eq!(parse("SUCCESS: hold release succeeded"), Line::Other);
    }

    fn watch(lines: &[&str]) -> Vec<Option<Verdict>> {
        let mut w = ConnectWatch::default();
        lines.iter().map(|l| w.on(&parse(l))).collect()
    }

    #[test]
    fn rejected_logins_fail_at_once() {
        let v = watch(&[
            ">STATE:1,AUTH,,,",
            ">LOG:1,I,AUTH: Received control message: AUTH_FAILED,Wrong password",
        ]);
        assert_eq!(v[0], None);
        match &v[1] {
            Some(Verdict::Failed(m)) => {
                assert!(m.ends_with("The server said: Wrong password"), "{m}")
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            watch(&[">PASSWORD:Verification Failed: 'Auth'"])[0],
            Some(Verdict::Failed(_))
        ));
        assert!(matches!(
            watch(&[">LOG:1,N,VERIFY ERROR: depth=0, error=certificate has expired"])[0],
            Some(Verdict::Failed(_))
        ));
        assert!(matches!(
            watch(&[">LOG:1,F,Cannot open TUN/TAP dev"])[0],
            Some(Verdict::Failed(_))
        ));
    }

    #[test]
    fn a_server_that_keeps_dropping_the_handshake_fails_after_three_tries() {
        let v = watch(&[
            ">LOG:1,N,TLS Error: TLS handshake failed",
            ">STATE:1,RECONNECTING,tls-error,,",
            ">STATE:2,RECONNECTING,tls-error,,",
            ">LOG:2,I,UDPv4 link local: (not bound)",
            ">STATE:3,RECONNECTING,tls-error,,",
        ]);
        assert_eq!(
            v[0],
            Some(Verdict::Retrying(
                "Connecting… TLS Error: TLS handshake failed".into()
            ))
        );
        assert!(matches!(v[1], Some(Verdict::Retrying(_))));
        assert!(matches!(v[2], Some(Verdict::Retrying(_))));
        assert_eq!(v[3], None);
        match &v[4] {
            Some(Verdict::Failed(m)) => assert!(
                m.contains("3 times (TLS Error: TLS handshake failed)"),
                "{m}"
            ),
            other => panic!("{other:?}"),
        }
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
