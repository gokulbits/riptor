//! Recent log lines for the viewer, with URL secrets masked.

use std::collections::VecDeque;
use std::fmt::Write;
use std::time::{SystemTime, UNIX_EPOCH};

use parking_lot::Mutex;
use serde::Serialize;
use tracing::field::{Field, Visit};
use tracing::{Event, Level, Subscriber};
use tracing_subscriber::layer::Context;
use tracing_subscriber::Layer;

const MAX_LINES: usize = 2000;
const MAX_LINE_LEN: usize = 2000;

static LINES: Mutex<VecDeque<LogLine>> = Mutex::new(VecDeque::new());

#[derive(Clone, Serialize)]
pub struct LogLine {
    /// Unix milliseconds.
    pub time: u64,
    pub level: &'static str,
    pub target: String,
    pub message: String,
}

pub fn recent() -> Vec<LogLine> {
    LINES.lock().iter().cloned().collect()
}

pub struct Capture;

struct Message(String);

impl Visit for Message {
    fn record_str(&mut self, field: &Field, value: &str) {
        if field.name() == "message" {
            self.0.push_str(value);
        } else {
            let _ = write!(self.0, " {}={value}", field.name());
        }
    }

    fn record_debug(&mut self, field: &Field, value: &dyn std::fmt::Debug) {
        if field.name() == "message" {
            let _ = write!(self.0, "{value:?}");
        } else {
            let _ = write!(self.0, " {}={value:?}", field.name());
        }
    }
}

impl<S: Subscriber> Layer<S> for Capture {
    fn on_event(&self, event: &Event<'_>, _ctx: Context<'_, S>) {
        let meta = event.metadata();
        let mut m = Message(String::new());
        event.record(&mut m);
        let mut message = redact(&m.0);
        if message.len() > MAX_LINE_LEN {
            let mut cut = MAX_LINE_LEN;
            while !message.is_char_boundary(cut) {
                cut -= 1;
            }
            message.truncate(cut);
            message.push('…');
        }
        let line = LogLine {
            time: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
            level: match *meta.level() {
                Level::ERROR => "error",
                Level::WARN => "warn",
                Level::INFO => "info",
                Level::DEBUG => "debug",
                Level::TRACE => "trace",
            },
            target: meta.target().to_string(),
            message,
        };
        let mut lines = LINES.lock();
        if lines.len() == MAX_LINES {
            lines.pop_front();
        }
        lines.push_back(line);
    }
}

/// Masks URL query strings and long path tokens.
pub fn redact(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(i) = rest.find("://") {
        // Back up to the start of the scheme.
        let start = rest[..i]
            .rfind(|c: char| !(c.is_ascii_alphanumeric() || matches!(c, '+' | '-' | '.')))
            .map_or(0, |p| p + 1);
        let end = rest[i..]
            .find(|c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`'))
            .map_or(rest.len(), |p| i + p);
        out.push_str(&rest[..start]);
        out.push_str(&redact_url(&rest[start..end]));
        rest = &rest[end..];
    }
    out.push_str(rest);
    out
}

fn redact_url(u: &str) -> String {
    let (base, query) = match u.find(['?', '#']) {
        Some(q) => (&u[..q], true),
        None => (u, false),
    };
    let after_scheme = base.find("://").map_or(0, |i| i + 3);
    let (scheme_host, path) = match base[after_scheme..].find('/') {
        Some(p) => base.split_at(after_scheme + p),
        None => (base, ""),
    };
    // user:password@host
    let scheme_host = match scheme_host.rfind('@') {
        Some(at) => format!(
            "{}…@{}",
            &scheme_host[..after_scheme],
            &scheme_host[at + 1..]
        ),
        None => scheme_host.to_string(),
    };
    let path: Vec<String> = path
        .split('/')
        .map(|seg| {
            if seg.len() >= 16
                && seg
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
            {
                "…".to_string()
            } else {
                seg.to_string()
            }
        })
        .collect();
    let mut r = scheme_host + &path.join("/");
    if query {
        r.push_str("?…");
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_in_urls_are_masked() {
        let line = "announce to \"http://t.example/announce?passkey=abc123&info_hash=x\" failed";
        assert_eq!(
            redact(line),
            "announce to \"http://t.example/announce?…\" failed"
        );
        assert_eq!(
            redact("udp://t.example:80/0123456789abcdef0123456789abcdef/announce"),
            "udp://t.example:80/…/announce"
        );
        assert_eq!(
            redact("proxy socks5://user:hunter2@10.0.0.1:1080 down"),
            "proxy socks5://…@10.0.0.1:1080 down"
        );
        assert_eq!(redact("no urls here"), "no urls here");
        assert_eq!(redact("https://example.org/a/b"), "https://example.org/a/b");
    }
}
