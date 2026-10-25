//! nova3 search plugins. They're third-party code: run with `python -I`, a
//! cleared environment, time and output limits, and bubblewrap on Linux.
//! The runner and helpers are our own and print one JSON object per result.

use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::time::Duration;

use anyhow::{bail, ensure, Context};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

use crate::SearchResult;

const SHIM: [(&str, &str); 3] = [
    ("nova2.py", include_str!("../assets/nova/nova2.py")),
    (
        "novaprinter.py",
        include_str!("../assets/nova/novaprinter.py"),
    ),
    ("helpers.py", include_str!("../assets/nova/helpers.py")),
];
const RESERVED: [&str; 8] = [
    "nova2",
    "novaprinter",
    "helpers",
    "nova2dl",
    "socks",
    "sgmllib3",
    "engines",
    "test",
];
pub const MAX_PLUGIN_BYTES: usize = 1024 * 1024;
const SEARCH_TIMEOUT: Duration = Duration::from_secs(45);
const CAPS_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_OUTPUT: usize = 4 * 1024 * 1024;

#[derive(Clone, Debug, Serialize)]
pub struct Python {
    pub exe: PathBuf,
    pub version: String,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PluginCaps {
    pub id: String,
    #[serde(default)]
    pub name: String,
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub categories: Vec<String>,
    #[serde(default)]
    pub error: Option<String>,
}

pub fn find_in_path(name: &str) -> Option<PathBuf> {
    let exts: Vec<String> = if cfg!(windows) {
        std::env::var("PATHEXT")
            .unwrap_or_else(|_| ".EXE;.BAT;.CMD".into())
            .split(';')
            .map(|s| s.to_ascii_lowercase())
            .collect()
    } else {
        vec![String::new()]
    };
    std::env::split_paths(&std::env::var_os("PATH")?).find_map(|dir| {
        if !dir.is_absolute() {
            return None; // never resolve through "." or relative PATH entries
        }
        exts.iter()
            .map(|e| dir.join(format!("{name}{e}")))
            .find(|p| p.is_file())
    })
}

fn quiet(cmd: &mut Command) {
    #[cfg(windows)]
    cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    cmd.stdin(Stdio::null()).kill_on_drop(true);
}

/// PATH first, then `extra`. Returns the real interpreter, not the `py` launcher.
pub async fn detect(extra: &[PathBuf]) -> Option<Python> {
    let names: &[(&str, &[&str])] = if cfg!(windows) {
        &[("py", &["-3"]), ("python3", &[]), ("python", &[])]
    } else {
        &[("python3", &[]), ("python", &[])]
    };
    let candidates = names
        .iter()
        .filter_map(|(name, pre)| Some((find_in_path(name)?, *pre)))
        .chain(
            extra
                .iter()
                .filter(|p| p.is_absolute() && p.is_file())
                .map(|p| (p.clone(), &[][..])),
        );
    for (exe, pre) in candidates {
        if let Some(py) = probe(&exe, pre).await {
            return Some(py);
        }
    }
    None
}

pub async fn probe(exe: &Path, pre: &[&str]) -> Option<Python> {
    let mut cmd = Command::new(exe);
    cmd.args(pre).args([
        "-I",
        "-c",
        "import sys;print(sys.version_info[0],sys.version_info[1]);print(sys.executable)",
    ]);
    quiet(&mut cmd);
    let Ok(Ok(out)) = tokio::time::timeout(Duration::from_secs(10), cmd.output()).await else {
        return None;
    };
    if !out.status.success() {
        return None; // e.g. the Microsoft Store "python" stub
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let mut lines = text.lines();
    let mut ver = lines
        .next()
        .unwrap_or("")
        .split_whitespace()
        .map(|n| n.parse::<u32>().unwrap_or(0));
    let (major, minor) = (ver.next().unwrap_or(0), ver.next().unwrap_or(0));
    let real = PathBuf::from(lines.next().unwrap_or("").trim());
    if major == 3 && minor >= 9 && real.is_absolute() && real.is_file() {
        return Some(Python {
            exe: real,
            version: format!("{major}.{minor}"),
        });
    }
    None
}

pub fn install_shim(dir: &Path) -> anyhow::Result<()> {
    std::fs::create_dir_all(dir.join("engines"))?;
    for (name, body) in SHIM {
        let p = dir.join(name);
        if std::fs::read_to_string(&p).ok().as_deref() != Some(body) {
            std::fs::write(&p, body)?;
        }
    }
    Ok(())
}

/// `proxy` carries the plugin's web requests (the VPN tunnel).
fn command(py: &Python, dir: &Path, args: &[&str], proxy: Option<&str>) -> Command {
    let script = dir.join("nova2.py");
    #[cfg(target_os = "linux")]
    if let Some(bwrap) = find_in_path("bwrap") {
        let mut c = Command::new(bwrap);
        c.args([
            "--ro-bind",
            "/",
            "/",
            "--dev",
            "/dev",
            "--proc",
            "/proc",
            "--tmpfs",
            "/tmp",
        ]);
        if let Some(home) = std::env::var_os("HOME") {
            c.arg("--tmpfs").arg(home);
        }
        c.arg("--ro-bind").arg(dir).arg(dir);
        c.args([
            "--unshare-all",
            "--share-net",
            "--die-with-parent",
            "--new-session",
            "--",
        ]);
        c.arg(&py.exe)
            .args(["-I", "-X", "utf8", "-B"])
            .arg(&script)
            .args(args);
        finish(&mut c, dir, proxy);
        return c;
    }
    let mut c = Command::new(&py.exe);
    c.args(["-I", "-X", "utf8", "-B"]).arg(&script).args(args);
    finish(&mut c, dir, proxy);
    c
}

fn finish(c: &mut Command, dir: &Path, proxy: Option<&str>) {
    c.current_dir(dir).env_clear();
    for var in [
        "SYSTEMROOT",
        "WINDIR",
        "TEMP",
        "TMP",
        "PATH",
        "LANG",
        "SSL_CERT_FILE",
        "SSL_CERT_DIR",
    ] {
        if let Some(v) = std::env::var_os(var) {
            c.env(var, v);
        }
    }
    c.env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONDONTWRITEBYTECODE", "1");
    if !cfg!(windows) {
        c.env("HOME", "/tmp");
    }
    if let Some(p) = proxy {
        for var in ["HTTP_PROXY", "HTTPS_PROXY", "http_proxy", "https_proxy"] {
            c.env(var, p);
        }
    }
    quiet(c);
    c.stdout(Stdio::piped()).stderr(Stdio::piped());
}

async fn tail(mut r: impl tokio::io::AsyncRead + Unpin) -> String {
    let mut buf = Vec::new();
    let _ = (&mut r).take(64 * 1024).read_to_end(&mut buf).await;
    let s = String::from_utf8_lossy(&buf);
    let last = s
        .lines()
        .rev()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_string();
    last.chars().take(300).collect()
}

pub async fn capabilities(py: &Python, dir: &Path) -> anyhow::Result<Vec<PluginCaps>> {
    let out = tokio::time::timeout(
        CAPS_TIMEOUT,
        command(py, dir, &["capabilities"], None).output(),
    )
    .await
    .context("reading plugins timed out")??;
    ensure!(out.stdout.len() <= MAX_OUTPUT, "plugin output too large");
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|l| serde_json::from_str::<PluginCaps>(l).ok())
        .map(|mut c| {
            c.name = c
                .name
                .chars()
                .filter(|ch| !ch.is_control())
                .take(80)
                .collect();
            if c.name.is_empty() {
                c.name = c.id.clone();
            }
            c.categories
                .retain(|x| crate::CATEGORIES.contains(&x.as_str()));
            c
        })
        .collect())
}

#[derive(Deserialize)]
struct Line {
    link: String,
    name: String,
    size: Option<u64>,
    seeds: Option<u32>,
    leech: Option<u32>,
    desc_link: Option<String>,
    pub_date: Option<u64>,
}

#[allow(clippy::too_many_arguments)]
pub async fn search(
    py: &Python,
    dir: &Path,
    engine: &str,
    category: &str,
    query: &str,
    proxy: Option<&str>,
    cancel: CancellationToken,
    mut emit: impl FnMut(Vec<SearchResult>),
) -> anyhow::Result<usize> {
    validate_id(engine)?;
    let mut args = vec!["search", engine, category];
    args.extend(query.split_whitespace());
    let mut child = command(py, dir, &args, proxy)
        .spawn()
        .context("could not start Python")?;
    let stdout = child.stdout.take().context("no stdout")?;
    let stderr = child.stderr.take().context("no stderr")?;
    let err_task = tokio::spawn(tail(stderr));

    let mut reader = BufReader::new(stdout).lines();
    let (mut count, mut bytes, mut batch) = (0usize, 0usize, Vec::new());
    let run = async {
        while let Some(line) = reader.next_line().await? {
            bytes += line.len();
            ensure!(bytes <= MAX_OUTPUT, "plugin produced too much output");
            if let Ok(l) = serde_json::from_str::<Line>(&line) {
                let http = !l.link.starts_with("magnet:");
                batch.push(SearchResult {
                    name: l.name,
                    link: l.link,
                    size: l.size,
                    seeders: l.seeds,
                    leechers: l.leech,
                    desc_link: l.desc_link,
                    pub_date: l.pub_date,
                    engine: engine.to_string(),
                    via_plugin: http,
                });
                count += 1;
                if batch.len() >= 25 {
                    emit(std::mem::take(&mut batch));
                }
            }
        }
        anyhow::Ok(())
    };
    let outcome = tokio::select! {
        r = tokio::time::timeout(SEARCH_TIMEOUT, run) => r.map_err(|_| anyhow::anyhow!("timed out")).and_then(|r| r),
        _ = cancel.cancelled() => Ok(()),
    };
    if !batch.is_empty() {
        emit(batch);
    }
    let _ = child.kill().await;
    let stderr_tail = err_task.await.unwrap_or_default();
    match outcome {
        Err(e) => Err(e),
        Ok(()) if count == 0 && !stderr_tail.is_empty() && !cancel.is_cancelled() => {
            bail!("{stderr_tail}")
        }
        Ok(()) => Ok(count),
    }
}

/// For sites that need cookies. Returns a temp file path.
pub async fn download(
    py: &Python,
    dir: &Path,
    engine: &str,
    url: &str,
    proxy: Option<&str>,
) -> anyhow::Result<PathBuf> {
    validate_id(engine)?;
    let u = url::Url::parse(url)?;
    ensure!(
        matches!(u.scheme(), "http" | "https"),
        "only http(s) links can be downloaded"
    );
    let out = tokio::time::timeout(
        Duration::from_secs(60),
        command(py, dir, &["download", engine, url], proxy).output(),
    )
    .await
    .context("download timed out")??;
    let text = String::from_utf8_lossy(&out.stdout);
    let path = text
        .lines()
        .last()
        .and_then(|l| l.rsplit_once(' '))
        .map(|(p, _)| PathBuf::from(p.trim()));
    let path = path.context("the plugin did not return a file")?;
    ensure!(
        path.is_absolute() && path.extension().is_some_and(|e| e == "torrent"),
        "the plugin returned an unexpected file"
    );
    Ok(path)
}

pub fn validate_id(id: &str) -> anyhow::Result<()> {
    ensure!(
        !id.is_empty()
            && id.len() <= 64
            && id
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
        "plugin file names must be lowercase letters, digits and underscores"
    );
    ensure!(!RESERVED.contains(&id), "\"{id}\" is a reserved name");
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
pub struct PluginPreview {
    pub id: String,
    pub sha256: String,
    pub size: usize,
    pub source: String,
    pub declared_name: Option<String>,
    pub declared_url: Option<String>,
    pub replaces_existing: bool,
}

pub fn declared(text: &str, attr: &str) -> Option<String> {
    text.lines().find_map(|l| {
        let l = l.trim();
        let rest = l.strip_prefix(attr)?.trim_start().strip_prefix('=')?.trim();
        let q = rest.chars().next().filter(|c| *c == '"' || *c == '\'')?;
        let inner = &rest[1..];
        Some(inner[..inner.find(q)?].chars().take(120).collect())
    })
}

pub fn inspect(
    file_name: &str,
    bytes: &[u8],
    source: &str,
    engines_dir: &Path,
) -> anyhow::Result<PluginPreview> {
    ensure!(bytes.len() <= MAX_PLUGIN_BYTES, "plugin file is too large");
    let stem = file_name
        .strip_suffix(".py")
        .context("search plugins are .py files")?;
    validate_id(stem)?;
    let text = std::str::from_utf8(bytes).context("plugin is not UTF-8 text")?;
    ensure!(
        text.contains(&format!("class {stem}(")) || text.contains(&format!("class {stem}:")),
        "this does not look like a nova3 search plugin (no class named {stem})"
    );
    Ok(PluginPreview {
        id: stem.to_string(),
        sha256: Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect(),
        size: bytes.len(),
        source: source.chars().take(500).collect(),
        declared_name: declared(text, "name"),
        declared_url: declared(text, "url"),
        replaces_existing: engines_dir.join(file_name).exists(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const PLUGIN: &str = r#"
from novaprinter import prettyPrinter
from helpers import retrieve_url

class demo(object):
    url = "https://demo.example"
    name = 'Demo engine'
    supported_categories = {"all": "0", "movies": "1"}

    def search(self, what, cat="all"):
        for i in range(2):
            prettyPrinter({"link": "magnet:?xt=urn:btih:%040d" % i, "name": "Result | %d %s" % (i, what),
                           "size": "1.5 GB", "seeds": i, "leech": "-1", "engine_url": self.url,
                           "desc_link": self.url + "/%d" % i, "pub_date": 1700000000})
        prettyPrinter({"link": "magnet:?xt=urn:btih:%040d" % 2, "name": "Strings", "size": "-1",
                       "seeds": "1,234", "leech": " 56 ", "engine_url": self.url})
"#;

    #[test]
    fn inspect_checks_names_and_shape() {
        let d = tempfile::tempdir().unwrap();
        let p = inspect("demo.py", PLUGIN.as_bytes(), "https://x", d.path()).unwrap();
        assert_eq!(p.declared_name.as_deref(), Some("Demo engine"));
        assert_eq!(p.declared_url.as_deref(), Some("https://demo.example"));
        assert_eq!(p.sha256.len(), 64);
        assert!(inspect("Demo.py", PLUGIN.as_bytes(), "", d.path()).is_err());
        assert!(inspect("helpers.py", b"class helpers: pass", "", d.path()).is_err());
        assert!(inspect("../evil.py", b"", "", d.path()).is_err());
        assert!(inspect("demo.txt", PLUGIN.as_bytes(), "", d.path()).is_err());
        assert!(inspect("other.py", PLUGIN.as_bytes(), "", d.path()).is_err());
    }

    #[tokio::test]
    async fn runs_a_plugin_end_to_end_if_python_present() {
        // RIPTOR_TEST_PYTHON lets this run without a system Python.
        let extra: Vec<PathBuf> = std::env::var_os("RIPTOR_TEST_PYTHON")
            .map(PathBuf::from)
            .into_iter()
            .collect();
        let Some(py) = detect(&extra).await else {
            eprintln!("python not installed; skipping");
            return;
        };
        let d = tempfile::tempdir().unwrap();
        install_shim(d.path()).unwrap();
        std::fs::write(d.path().join("engines/demo.py"), PLUGIN).unwrap();
        // A plugin trying to shadow the json module must not break others.
        std::fs::write(
            d.path().join("engines/json.py"),
            "class json: pass\nraise SystemExit(3)",
        )
        .unwrap();
        let caps = capabilities(&py, d.path()).await.unwrap();
        let demo = caps.iter().find(|c| c.id == "demo").unwrap();
        assert_eq!(demo.name, "Demo engine");
        assert_eq!(demo.categories, vec!["all", "movies"]);
        let mut got = Vec::new();
        let n = search(
            &py,
            d.path(),
            "demo",
            "all",
            "big buck",
            None,
            CancellationToken::new(),
            |b| got.extend(b),
        )
        .await
        .unwrap();
        assert_eq!(n, 3);
        assert_eq!(got[1].name, "Result | 1 big+buck");
        assert_eq!(got[1].size, Some(1_610_612_736));
        assert_eq!(got[1].seeders, Some(1));
        assert_eq!(got[1].leechers, None);
        assert_eq!(got[2].seeders, Some(1234), "thousands separators");
        assert_eq!(got[2].leechers, Some(56));
        assert_eq!(got[2].size, None);
        assert!(search(
            &py,
            d.path(),
            "../x",
            "all",
            "q",
            None,
            CancellationToken::new(),
            |_| {}
        )
        .await
        .is_err());

        // Web requests go through the proxy, which also does the DNS.
        std::fs::write(d.path().join("engines/viaproxy.py"), VIA_PROXY).unwrap();
        let l = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .unwrap();
        let port = l.local_addr().unwrap().port();
        tokio::spawn(async move {
            use tokio::io::{AsyncReadExt, AsyncWriteExt};
            let (mut s, _) = l.accept().await.unwrap();
            let mut buf = vec![0; 4096];
            let _ = s.read(&mut buf).await;
            let _ = s
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 12\r\n\r\nvia the vpn!")
                .await;
        });
        let tunnel = crate::scrape::Tunnel {
            end: riptor_net::TunnelEnd::new(std::net::Ipv4Addr::LOCALHOST.into(), "lo"),
            lookup: std::sync::Arc::new(|_, port| {
                Box::pin(async move {
                    Ok(vec![std::net::SocketAddr::from((
                        std::net::Ipv4Addr::LOCALHOST,
                        port,
                    ))])
                })
            }),
        };
        let proxy = crate::proxy::Proxy::start(tunnel, true).await.unwrap();
        let mut got = Vec::new();
        search(
            &py,
            d.path(),
            "viaproxy",
            "all",
            &port.to_string(),
            Some(&proxy.url()),
            CancellationToken::new(),
            |b| got.extend(b),
        )
        .await
        .unwrap();
        assert_eq!(got[0].name, "via the vpn!");
    }

    const VIA_PROXY: &str = r#"
from novaprinter import prettyPrinter
from helpers import retrieve_url

class viaproxy(object):
    url = "https://demo.example"
    name = "Via proxy"
    supported_categories = {"all": "0"}

    def search(self, what, cat="all"):
        body = retrieve_url("http://only-the-proxy-knows.test:%s/q" % what)
        prettyPrinter({"link": "magnet:?xt=urn:btih:%040d" % 1, "name": body or "no reply", "size": "-1",
                       "seeds": 0, "leech": 0, "engine_url": self.url})
"#;
}
