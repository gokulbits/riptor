//! Peer countries from DB-IP's free IP-to-country database (CC BY 4.0), downloaded only when turned on.

use std::io::{BufRead, BufReader, Read};
use std::net::IpAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::{bail, ensure, Context};
use futures::StreamExt;

const MAX_DOWNLOAD: usize = 32 * 1024 * 1024;
const MAX_CSV: u64 = 256 * 1024 * 1024;
const MAX_ROWS: usize = 4_000_000;
/// A newer list comes out monthly.
pub const REFRESH: Duration = Duration::from_secs(32 * 24 * 3600);
pub const ATTRIBUTION: &str = "IP geolocation by DB-IP (https://db-ip.com), CC BY 4.0";

/// Ranges sorted by start, kept as separate arrays to save memory.
#[derive(Default)]
struct Table<T> {
    starts: Vec<T>,
    ends: Vec<T>,
    countries: Vec<[u8; 2]>,
}

trait Succ: Copy {
    fn succ(self) -> Self;
}

impl Succ for u32 {
    fn succ(self) -> Self {
        self.saturating_add(1)
    }
}

impl Succ for u64 {
    fn succ(self) -> Self {
        self.saturating_add(1)
    }
}

impl<T: Succ + Ord> Table<T> {
    fn push(&mut self, start: T, end: T, cc: [u8; 2]) {
        // Touching ranges of one country become one; out-of-order rows are left for `sorted` to refuse.
        if let (Some(s), Some(e), Some(c)) = (
            self.starts.last(),
            self.ends.last_mut(),
            self.countries.last(),
        ) {
            if *c == cc && start >= *s && start <= e.succ() {
                *e = (*e).max(end);
                return;
            }
        }
        self.starts.push(start);
        self.ends.push(end);
        self.countries.push(cc);
    }

    fn get(&self, ip: T) -> Option<[u8; 2]> {
        let i = self.starts.partition_point(|s| *s <= ip).checked_sub(1)?;
        (ip <= self.ends[i]).then_some(self.countries[i])
    }

    fn sorted(&self) -> bool {
        self.starts.windows(2).all(|w| w[0] <= w[1])
    }
}

/// IPv6 ranges are kept by their first 64 bits, which is as fine as the data goes.
#[derive(Default)]
pub struct GeoDb {
    v4: Table<u32>,
    v6: Table<u64>,
}

fn v6_top(ip: std::net::Ipv6Addr) -> u64 {
    (u128::from(ip) >> 64) as u64
}

impl GeoDb {
    /// Two-letter country code, or `None` when unknown.
    pub fn lookup(&self, ip: IpAddr) -> Option<String> {
        let cc = match ip {
            IpAddr::V4(v4) => self.v4.get(u32::from(v4)),
            IpAddr::V6(v6) => match v6.to_ipv4_mapped() {
                Some(v4) => self.v4.get(u32::from(v4)),
                None => self.v6.get(v6_top(v6)),
            },
        }?;
        Some(String::from_utf8_lossy(&cc).into_owned())
    }

    pub fn ranges(&self) -> usize {
        self.v4.starts.len() + self.v6.starts.len()
    }

    /// `start,end,CC` lines; anything malformed is refused, not guessed at.
    pub fn parse(reader: impl Read) -> anyhow::Result<Self> {
        let mut db = GeoDb::default();
        let mut rows = 0usize;
        let mut line = String::new();
        let mut r = BufReader::new(reader.take(MAX_CSV + 1));
        let mut read = 0u64;
        loop {
            line.clear();
            let n = r.read_line(&mut line)?;
            if n == 0 {
                break;
            }
            read += n as u64;
            ensure!(read <= MAX_CSV, "country list is too large");
            let row = line.trim_end_matches(['\r', '\n']);
            if row.is_empty() {
                continue;
            }
            rows += 1;
            ensure!(rows <= MAX_ROWS, "country list has too many rows");
            let mut parts = row.split(',');
            let (Some(a), Some(b), Some(cc), None) =
                (parts.next(), parts.next(), parts.next(), parts.next())
            else {
                bail!("bad row {rows}");
            };
            let cc = cc.as_bytes();
            ensure!(
                cc.len() == 2 && cc.iter().all(u8::is_ascii_uppercase),
                "bad country code on row {rows}"
            );
            let cc = [cc[0], cc[1]];
            let (a, b): (IpAddr, IpAddr) = (
                a.parse()
                    .with_context(|| format!("bad address on row {rows}"))?,
                b.parse()
                    .with_context(|| format!("bad address on row {rows}"))?,
            );
            if cc == *b"ZZ" {
                continue;
            }
            match (a, b) {
                (IpAddr::V4(a), IpAddr::V4(b)) if a <= b => db.v4.push(a.into(), b.into(), cc),
                (IpAddr::V6(a), IpAddr::V6(b)) if a <= b => db.v6.push(v6_top(a), v6_top(b), cc),
                _ => bail!("bad range on row {rows}"),
            }
        }
        ensure!(
            db.v4.sorted() && db.v6.sorted(),
            "country list isn't in order"
        );
        ensure!(db.ranges() > 0, "country list is empty");
        Ok(db)
    }

    pub fn open(path: &Path) -> anyhow::Result<Self> {
        let f = std::fs::File::open(path)?;
        Self::parse(flate2::read::GzDecoder::new(f))
    }
}

pub fn cache_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("geoip")
}

pub fn cache_file(data_dir: &Path) -> PathBuf {
    cache_dir(data_dir).join("dbip-country-lite.csv.gz")
}

/// The current month's list, or last month's early in a month.
pub fn urls(now_unix: u64) -> [String; 2] {
    let today = crate::stats::day_key(now_unix);
    let (y, m) = (
        today[..4].parse::<i32>().unwrap_or(1970),
        today[5..7].parse::<u32>().unwrap_or(1),
    );
    let (py, pm) = if m == 1 { (y - 1, 12) } else { (y, m - 1) };
    let url = |y: i32, m: u32| {
        format!("https://download.db-ip.com/free/dbip-country-lite-{y:04}-{m:02}.csv.gz")
    };
    [url(y, m), url(py, pm)]
}

/// Downloads, checks it parses, then replaces the cached copy.
pub async fn download(
    http: &reqwest::Client,
    data_dir: &Path,
    now_unix: u64,
) -> anyhow::Result<GeoDb> {
    let mut last = None;
    for url in urls(now_unix) {
        let got = fetch(http, &url).await;
        riptor_net::ledger::record(riptor_net::ledger::Purpose::CountryData, &url, got.is_ok());
        match got {
            Ok(bytes) => {
                let parsed = {
                    let bytes = bytes.clone();
                    tokio::task::spawn_blocking(move || {
                        GeoDb::parse(flate2::read::GzDecoder::new(&bytes[..]))
                    })
                    .await??
                };
                crate::state_file::save_bytes(&cache_file(data_dir), &bytes)?;
                return Ok(parsed);
            }
            Err(e) => last = Some(e),
        }
    }
    Err(last.unwrap_or_else(|| anyhow::anyhow!("no country list available")))
}

async fn fetch(http: &reqwest::Client, url: &str) -> anyhow::Result<Vec<u8>> {
    let resp = http
        .get(url)
        .timeout(Duration::from_secs(120))
        .send()
        .await?
        .error_for_status()?;
    ensure!(resp.url().scheme() == "https", "redirected off https");
    if let Some(len) = resp.content_length() {
        ensure!(len as usize <= MAX_DOWNLOAD, "country list is too large");
    }
    let mut body = Vec::new();
    let mut stream = resp.bytes_stream();
    while let Some(chunk) = stream.next().await {
        body.extend_from_slice(&chunk?);
        ensure!(body.len() <= MAX_DOWNLOAD, "country list is too large");
    }
    Ok(body)
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "0.0.0.0,0.255.255.255,ZZ\n\
        1.0.0.0,1.0.0.255,AU\n\
        1.0.1.0,1.0.3.255,CN\n\
        1.0.4.0,1.0.7.255,CN\n\
        8.8.8.0,8.8.8.255,US\n\
        2001:200::,2001:200:ffff:ffff:ffff:ffff:ffff:ffff,JP\n\
        2a00:1450::,2a00:1450:ffff:ffff:ffff:ffff:ffff:ffff,IE\n";

    fn db() -> GeoDb {
        GeoDb::parse(SAMPLE.as_bytes()).unwrap()
    }

    #[test]
    fn addresses_resolve_to_countries() {
        let db = db();
        let cc = |s: &str| db.lookup(s.parse().unwrap());
        assert_eq!(cc("1.0.0.7").as_deref(), Some("AU"));
        assert_eq!(cc("1.0.5.1").as_deref(), Some("CN"));
        assert_eq!(cc("8.8.8.8").as_deref(), Some("US"));
        assert_eq!(cc("::ffff:8.8.8.8").as_deref(), Some("US"));
        assert_eq!(cc("2a00:1450:4001:80b::200e").as_deref(), Some("IE"));
        assert_eq!(cc("2001:200:dff:fff1::1").as_deref(), Some("JP"));
        // Unknown ("ZZ") and gaps.
        assert_eq!(cc("0.1.2.3"), None);
        assert_eq!(cc("9.9.9.9"), None);
        assert_eq!(cc("2600::1"), None);
        // The two CN ranges were merged.
        assert_eq!(db.ranges(), 5);
    }

    #[test]
    fn hostile_lists_are_refused() {
        for bad in [
            "",
            "1.0.0.0,1.0.0.255",
            "1.0.0.0,1.0.0.255,AU,extra",
            "1.0.0.0,1.0.0.255,au",
            "1.0.0.0,1.0.0.255,A\u{0}",
            "1.0.0.9,1.0.0.1,AU",
            "1.0.0.0,::1,AU",
            "999.0.0.0,1.0.0.1,AU",
            "2.0.0.0,2.0.0.1,AU\n1.0.0.0,1.0.0.1,AU",
            "<html>not found</html>",
        ] {
            assert!(GeoDb::parse(bad.as_bytes()).is_err(), "{bad:?}");
        }
    }

    /// `RIPTOR_TEST_GEOIP=<dbip-country-lite.csv.gz>` checks a real download.
    #[test]
    fn the_real_list_parses() {
        let Some(path) = std::env::var_os("RIPTOR_TEST_GEOIP") else {
            return;
        };
        let t = std::time::Instant::now();
        let db = GeoDb::open(Path::new(&path)).unwrap();
        eprintln!("{} ranges in {:?}", db.ranges(), t.elapsed());
        let cc = |s: &str| db.lookup(s.parse().unwrap());
        assert_eq!(cc("1.1.1.1").as_deref(), Some("AU"));
        assert!(cc("8.8.8.8").is_some());
        assert!(cc("2001:4860:4860::8888").is_some());
        assert_eq!(cc("10.0.0.1"), None);
    }

    #[test]
    fn urls_follow_the_month() {
        // 2026-01-03
        let [now, before] = urls(1_767_441_600);
        assert!(now.ends_with("dbip-country-lite-2026-01.csv.gz"));
        assert!(before.ends_with("dbip-country-lite-2025-12.csv.gz"));
        assert!(now.starts_with("https://"));
    }

    #[test]
    fn a_cached_copy_opens() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = cache_file(dir.path());
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
        gz.write_all(SAMPLE.as_bytes()).unwrap();
        std::fs::write(&path, gz.finish().unwrap()).unwrap();
        assert_eq!(GeoDb::open(&path).unwrap().ranges(), 5);
        std::fs::write(&path, b"not gzip").unwrap();
        assert!(GeoDb::open(&path).is_err());
    }
}
