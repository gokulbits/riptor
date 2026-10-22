//! Torrents from qBittorrent's folder of `.torrent` + `.fastresume` pairs (any libtorrent-based client works).

use std::path::{Path, PathBuf};

use librqbit_bencode::BencodeValue;
use riptor_seeding::{Limit, ShareLimits};
use serde::Serialize;

const MAX_ENTRIES: usize = 5000;
const MAX_RESUME_BYTES: u64 = 8 * 1024 * 1024;
const MAX_TAGS: usize = 32;
// Anything bigger is a forged file, not a real transfer.
const MAX_BYTES: i64 = 1 << 60;
const MAX_SECS: i64 = 100 * 365 * 24 * 3600;
const MAX_LIMIT_MINUTES: i64 = 100 * 365 * 24 * 60;
// Ratios are stored times 1000.
const MAX_RATIO_MILLIS: i64 = 10_000 * 1000;
const EARLIEST_ADDED: i64 = 978_307_200;

#[derive(Debug, PartialEq)]
pub struct Found {
    pub torrent: PathBuf,
    pub resume: Resume,
}

/// What a resume file says about a torrent, with every value checked.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Resume {
    pub save_path: PathBuf,
    pub paused: bool,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub downloaded: u64,
    pub uploaded: u64,
    /// Seconds.
    pub active_time: u64,
    /// Seconds.
    pub seeding_time: u64,
    /// Unix seconds.
    pub added_on: Option<u64>,
    pub limits: ShareLimits,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct ImportReport {
    pub added: usize,
    pub already_present: usize,
    /// Resume files without a usable .torrent or save folder.
    pub skipped: usize,
    pub failed: Vec<String>,
    /// qBittorrent's all-time totals, now part of Riptor's statistics.
    pub all_time: Option<AllTime>,
}

type Dict = std::collections::HashMap<Vec<u8>, BencodeValue<Vec<u8>>>;

fn int(top: &Dict, key: &str) -> Option<i64> {
    match top.get(key.as_bytes())? {
        BencodeValue::Integer(i) => Some(*i),
        _ => None,
    }
}

fn in_range(v: Option<i64>, max: i64) -> u64 {
    v.filter(|v| (0..=max).contains(v)).map_or(0, |v| v as u64)
}

/// The same rules as names typed in Riptor; slashes (nested categories) become dashes.
pub fn clean_label(s: &[u8]) -> Option<String> {
    let s = std::str::from_utf8(s)
        .ok()?
        .trim()
        .replace(['/', '\\'], "-");
    let ok = !s.is_empty()
        && s.len() <= 64
        && !s.chars().any(|c| {
            c.is_control() || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        });
    ok.then_some(s)
}

/// -2 means the global limit, -1 none; anything else out of range falls back to global.
fn limit(v: Option<i64>, max: i64) -> Limit<i64> {
    match v {
        Some(-1) => Limit::Unlimited,
        Some(v) if (0..=max).contains(&v) => Limit::Value(v),
        _ => Limit::UseGlobal,
    }
}

fn minutes(l: Limit<i64>) -> Limit<u64> {
    match l {
        Limit::Value(v) => Limit::Value(v as u64),
        Limit::Unlimited => Limit::Unlimited,
        Limit::UseGlobal => Limit::UseGlobal,
    }
}

/// `None` unless the resume file has a usable absolute save folder.
pub fn parse_fastresume(bytes: &[u8]) -> Option<Resume> {
    let BencodeValue::Dict(top) = librqbit_bencode::dyn_from_bytes::<Vec<u8>>(bytes).ok()? else {
        return None;
    };
    let save = match top.get(&b"save_path"[..])? {
        BencodeValue::Bytes(b) => String::from_utf8(b.clone()).ok()?,
        _ => return None,
    };
    let save_path = PathBuf::from(save.trim());
    if !save_path.is_absolute() || save_path.as_os_str().len() > 4096 {
        return None;
    }
    // The other client's own keys sit beside the standard resume fields.
    let paused = [
        int(&top, "paused"),
        int(&top, "qBt-stopped"),
        int(&top, "qBt-paused"),
    ]
    .contains(&Some(1));
    let category = match top.get(&b"qBt-category"[..]) {
        Some(BencodeValue::Bytes(b)) => clean_label(b),
        _ => None,
    };
    let mut tags = Vec::new();
    if let Some(BencodeValue::List(list)) = top.get(&b"qBt-tags"[..]) {
        for t in list {
            if let BencodeValue::Bytes(b) = t {
                if let Some(t) = clean_label(b) {
                    if !tags.contains(&t) && tags.len() < MAX_TAGS {
                        tags.push(t);
                    }
                }
            }
        }
    }
    let seeding = int(&top, "seeding_time").or_else(|| int(&top, "finished_time"));
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(i64::MAX, |d| d.as_secs() as i64);
    let added_on = int(&top, "added_time")
        .filter(|t| (EARLIEST_ADDED..=now.saturating_add(86_400)).contains(t))
        .map(|t| t as u64);
    let ratio = match limit(int(&top, "qBt-ratioLimit"), MAX_RATIO_MILLIS) {
        Limit::Value(v) => Limit::Value(v as f64 / 1000.0),
        Limit::Unlimited => Limit::Unlimited,
        Limit::UseGlobal => Limit::UseGlobal,
    };
    Some(Resume {
        save_path,
        paused,
        category,
        tags,
        downloaded: in_range(int(&top, "total_downloaded"), MAX_BYTES),
        uploaded: in_range(int(&top, "total_uploaded"), MAX_BYTES),
        active_time: in_range(int(&top, "active_time"), MAX_SECS),
        seeding_time: in_range(seeding, MAX_SECS),
        added_on,
        limits: ShareLimits {
            ratio,
            seeding_time: minutes(limit(int(&top, "qBt-seedingTimeLimit"), MAX_LIMIT_MINUTES)),
            inactive_seeding_time: minutes(limit(
                int(&top, "qBt-inactiveSeedingTimeLimit"),
                MAX_LIMIT_MINUTES,
            )),
        },
    })
}

/// What an import would bring in, before anything is added.
#[derive(Clone, Debug, Default, Serialize)]
pub struct ImportScan {
    /// `None` when no folder was given and none was found.
    pub dir: Option<PathBuf>,
    pub found: usize,
    pub skipped: usize,
    /// qBittorrent's own all-time totals, when its statistics file was found.
    pub all_time: Option<AllTime>,
}

/// qBittorrent's all-time totals (its Statistics window).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub struct AllTime {
    pub downloaded: u64,
    pub uploaded: u64,
}

const STATS_NAME: &str = "qBittorrent-data";
const MAX_STATS_BYTES: u64 = 1024 * 1024;

/// qBittorrent's usual installs: `(BT_backup folder, statistics file)`, most likely first.
fn profiles() -> Vec<(PathBuf, PathBuf)> {
    let mut out = Vec::new();
    #[cfg(windows)]
    {
        let dir = |var: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
        };
        // Torrents under Local, settings (and statistics) under Roaming.
        if let (Some(local), Some(roaming)) = (dir("LOCALAPPDATA"), dir("APPDATA")) {
            out.push((
                local.join("qBittorrent").join("BT_backup"),
                roaming
                    .join("qBittorrent")
                    .join(format!("{STATS_NAME}.ini")),
            ));
        }
    }
    #[cfg(not(windows))]
    if let Some(home) = std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
    {
        let xdg = |var: &str, fallback: &str| {
            std::env::var_os(var)
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .unwrap_or_else(|| home.join(fallback))
        };
        out.push((
            xdg("XDG_DATA_HOME", ".local/share").join("qBittorrent/BT_backup"),
            xdg("XDG_CONFIG_HOME", ".config").join(format!("qBittorrent/{STATS_NAME}.conf")),
        ));
        // The Flatpak build keeps both in its sandbox.
        let flatpak = home.join(".var/app/org.qbittorrent.qBittorrent");
        out.push((
            flatpak.join("data/qBittorrent/BT_backup"),
            flatpak.join(format!("config/qBittorrent/{STATS_NAME}.conf")),
        ));
    }
    out
}

/// Where qBittorrent keeps its resume files, most likely first.
pub fn default_dirs() -> Vec<PathBuf> {
    profiles().into_iter().map(|(dir, _)| dir).collect()
}

fn same_dir(a: &Path, b: &Path) -> bool {
    match (std::fs::canonicalize(a), std::fs::canonicalize(b)) {
        (Ok(a), Ok(b)) => a == b,
        _ => a == b,
    }
}

/// The statistics files that belong to the profile `bt_backup` is part of.
pub fn stats_files(bt_backup: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    // A portable profile: <profile>/data/BT_backup next to <profile>/config.
    if let Some(profile) = bt_backup.parent().and_then(Path::parent) {
        let config = profile.join("config");
        out.push(config.join(format!("{STATS_NAME}.ini")));
        out.push(config.join(format!("{STATS_NAME}.conf")));
    }
    for (dir, stats) in profiles() {
        if same_dir(bt_backup, &dir) {
            out.push(stats);
        }
    }
    out
}

/// qBittorrent's all-time totals for the profile `bt_backup` belongs to.
pub fn read_all_time(bt_backup: &Path) -> Option<AllTime> {
    stats_files(bt_backup).iter().find_map(|f| {
        let meta = std::fs::symlink_metadata(f).ok()?;
        if !meta.is_file() || meta.len() > MAX_STATS_BYTES {
            return None;
        }
        parse_all_time(&std::fs::read(f).ok()?)
    })
}

/// Reads `AllStats` from the `[Stats]` section of qBittorrent's statistics file.
pub fn parse_all_time(ini: &[u8]) -> Option<AllTime> {
    let text = std::str::from_utf8(ini).ok()?;
    let mut in_stats = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_stats = line == "[Stats]";
            continue;
        }
        let Some(value) = in_stats.then(|| line.strip_prefix("AllStats=")).flatten() else {
            continue;
        };
        let value = value.trim();
        // Qt quotes the whole value when the encoded bytes contain ; , or =.
        let value = value
            .strip_prefix('"')
            .and_then(|v| v.strip_suffix('"'))
            .unwrap_or(value);
        let inner = value.strip_prefix("@Variant(")?.strip_suffix(')')?;
        return decode_hash(&qt_unescape(inner)?);
    }
    None
}

/// Undoes Qt's INI escaping of a binary value; `None` on anything Qt wouldn't write.
fn qt_unescape(s: &str) -> Option<Vec<u8>> {
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        let c = b[i];
        i += 1;
        if c != b'\\' {
            // Binary values are written byte by byte, escaping everything outside ASCII.
            if !(0x20..0x7f).contains(&c) || c == b'"' {
                return None;
            }
            out.push(c);
            continue;
        }
        let e = *b.get(i)?;
        i += 1;
        let v: u32 = match e {
            b'a' => 7,
            b'b' => 8,
            b'f' => 12,
            b'n' => 10,
            b'r' => 13,
            b't' => 9,
            b'v' => 11,
            b'"' | b'?' | b'\'' | b'\\' => e as u32,
            b'x' => {
                let start = i;
                let mut v = 0u32;
                while let Some(d) = b.get(i).and_then(|d| (*d as char).to_digit(16)) {
                    v = v * 16 + d;
                    i += 1;
                    if v > 0xff {
                        return None;
                    }
                }
                if i == start {
                    return None;
                }
                v
            }
            b'0'..=b'7' => {
                let mut v = (e - b'0') as u32;
                while let Some(d) = b.get(i).and_then(|d| (*d as char).to_digit(8)) {
                    v = v * 8 + d;
                    i += 1;
                    if v > 0xff {
                        return None;
                    }
                }
                v
            }
            _ => return None,
        };
        out.push(v as u8);
    }
    Some(out)
}

struct Stream<'a>(&'a [u8]);

impl Stream<'_> {
    fn take(&mut self, n: usize) -> Option<&[u8]> {
        if self.0.len() < n {
            return None;
        }
        let (head, rest) = self.0.split_at(n);
        self.0 = rest;
        Some(head)
    }

    fn u32(&mut self) -> Option<u32> {
        Some(u32::from_be_bytes(self.take(4)?.try_into().ok()?))
    }

    fn i64(&mut self) -> Option<i64> {
        Some(i64::from_be_bytes(self.take(8)?.try_into().ok()?))
    }
}

// QVariant type ids in the Qt 4.0 stream format QSettings uses for @Variant.
const QT_LONG_LONG: u32 = 4;
const QT_ULONG_LONG: u32 = 5;
const QT_VARIANT_HASH: u32 = 28;

/// A serialized `QVariantHash` holding `AlltimeDL` and `AlltimeUL`.
fn decode_hash(bytes: &[u8]) -> Option<AllTime> {
    let mut s = Stream(bytes);
    if s.u32()? != QT_VARIANT_HASH {
        return None;
    }
    let count = s.u32()?;
    if count > 16 {
        return None;
    }
    let (mut dl, mut ul) = (None, None);
    for _ in 0..count {
        let len = s.u32()? as usize;
        if !len.is_multiple_of(2) || len > 64 {
            return None;
        }
        let units: Vec<u16> = s
            .take(len)?
            .chunks(2)
            .map(|p| u16::from_be_bytes([p[0], p[1]]))
            .collect();
        let key = String::from_utf16(&units).ok()?;
        // Only 64-bit integers are expected; anything else can't be skipped safely.
        let value = match s.u32()? {
            QT_LONG_LONG | QT_ULONG_LONG => s.i64()?,
            _ => return None,
        };
        let value = (0..=MAX_BYTES).contains(&value).then_some(value as u64)?;
        match key.as_str() {
            "AlltimeDL" => dl = Some(value),
            "AlltimeUL" => ul = Some(value),
            _ => {}
        }
    }
    if !s.0.is_empty() {
        return None;
    }
    Some(AllTime {
        downloaded: dl?,
        uploaded: ul?,
    })
}

/// Counts what's importable in `dir`, or in the first usual folder that exists.
pub fn scan_counts(dir: Option<&Path>) -> anyhow::Result<ImportScan> {
    let dir = match dir {
        Some(d) => Some(d.to_path_buf()),
        None => default_dirs().into_iter().find(|d| d.is_dir()),
    };
    let Some(dir) = dir else {
        return Ok(ImportScan::default());
    };
    let (found, skipped) = scan(&dir)?;
    Ok(ImportScan {
        all_time: read_all_time(&dir),
        dir: Some(dir),
        found: found.len(),
        skipped,
    })
}

/// `(found, skipped)`; only plain files directly in `dir` are read.
pub fn scan(dir: &Path) -> anyhow::Result<(Vec<Found>, usize)> {
    let mut found = Vec::new();
    let mut skipped = 0;
    for entry in std::fs::read_dir(dir)?.flatten().take(MAX_ENTRIES * 2) {
        let path = entry.path();
        let is_resume = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case("fastresume"));
        if !is_resume || !entry.file_type().is_ok_and(|t| t.is_file()) {
            continue;
        }
        if found.len() + skipped >= MAX_ENTRIES {
            break;
        }
        let torrent = path.with_extension("torrent");
        let usable = entry.metadata().is_ok_and(|m| m.len() <= MAX_RESUME_BYTES)
            && std::fs::symlink_metadata(&torrent).is_ok_and(|m| m.is_file());
        let parsed = usable
            .then(|| std::fs::read(&path).ok())
            .flatten()
            .and_then(|b| parse_fastresume(&b));
        match parsed {
            Some(resume) => found.push(Found { torrent, resume }),
            None => skipped += 1,
        }
    }
    Ok((found, skipped))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn abs() -> String {
        std::env::temp_dir()
            .join("dl")
            .to_string_lossy()
            .into_owned()
    }

    fn resume(save: &str, paused: i64) -> Vec<u8> {
        format!("d6:pausedi{paused}e9:save_path{}:{save}e", save.len()).into_bytes()
    }

    fn s(v: &str) -> String {
        format!("{}:{v}", v.len())
    }

    /// A resume file with `save_path` plus whatever `extra` bencode pairs say.
    fn with(extra: &str) -> Option<Resume> {
        let save = abs();
        parse_fastresume(format!("d{}{}{extra}e", s("save_path"), s(&save)).as_bytes())
    }

    #[test]
    fn reads_save_folder_and_paused_flag() {
        let save = abs();
        let r = parse_fastresume(&resume(&save, 1)).unwrap();
        assert_eq!((r.save_path, r.paused), (PathBuf::from(&save), true));
        assert!(!parse_fastresume(&resume(&save, 0)).unwrap().paused);
        assert!(with(&format!("{}i1e", s("qBt-stopped"))).unwrap().paused);
    }

    #[test]
    fn reads_history_labels_and_limits() {
        let r = with(&format!(
            "{}{}{}l{}{}e{}i5000000e{}i7000e{}i3600e{}i1800e{}i1700000000e{}i1500e{}i-1e{}i-2e",
            s("qBt-category"),
            s("Linux ISOs"),
            s("qBt-tags"),
            s("debian"),
            s("seeding"),
            s("total_downloaded"),
            s("total_uploaded"),
            s("active_time"),
            s("seeding_time"),
            s("added_time"),
            s("qBt-ratioLimit"),
            s("qBt-seedingTimeLimit"),
            s("qBt-inactiveSeedingTimeLimit"),
        ))
        .unwrap();
        assert_eq!(r.category.as_deref(), Some("Linux ISOs"));
        assert_eq!(r.tags, ["debian", "seeding"]);
        assert_eq!((r.downloaded, r.uploaded), (5_000_000, 7000));
        assert_eq!((r.active_time, r.seeding_time), (3600, 1800));
        assert_eq!(r.added_on, Some(1_700_000_000));
        assert_eq!(r.limits.ratio, Limit::Value(1.5));
        assert_eq!(r.limits.seeding_time, Limit::Unlimited);
        assert_eq!(r.limits.inactive_seeding_time, Limit::UseGlobal);
        // Nested categories keep their name, with dashes for slashes.
        let r = with(&format!("{}{}", s("qBt-category"), s("Movies/HD"))).unwrap();
        assert_eq!(r.category.as_deref(), Some("Movies-HD"));
        // Older files only have finished_time.
        let r = with(&format!("{}i42e", s("finished_time"))).unwrap();
        assert_eq!(r.seeding_time, 42);
        // Nothing extra: defaults all round.
        let r = with("").unwrap();
        assert_eq!(
            (r.category, r.tags.len(), r.uploaded, r.added_on),
            (None, 0, 0, None)
        );
        assert_eq!(r.limits, ShareLimits::default());
    }

    #[test]
    fn hostile_values_are_dropped() {
        let r = with(&format!(
            "{}i-5e{}i{}e{}i-1e{}i99999999999999e{}i4102444800000e{}i-7e{}i-3e{}i99999999999e",
            s("total_downloaded"),
            s("total_uploaded"),
            i64::MAX,
            s("active_time"),
            s("seeding_time"),
            s("added_time"),
            s("qBt-ratioLimit"),
            s("qBt-seedingTimeLimit"),
            s("qBt-inactiveSeedingTimeLimit"),
        ))
        .unwrap();
        assert_eq!(
            (r.downloaded, r.uploaded, r.active_time, r.seeding_time),
            (0, 0, 0, 0)
        );
        assert_eq!(r.added_on, None, "a date far in the future");
        assert_eq!(r.limits, ShareLimits::default());
        assert_eq!(
            with(&format!("{}i5e", s("added_time"))).unwrap().added_on,
            None
        );
        assert_eq!(
            with(&format!(
                "{}i{}e",
                s("qBt-ratioLimit"),
                MAX_RATIO_MILLIS + 1
            ))
            .unwrap()
            .limits
            .ratio,
            Limit::UseGlobal
        );

        // Wrong types are ignored, not trusted.
        let r = with(&format!(
            "{}i5e{}d{}{}e{}{}",
            s("qBt-category"),
            s("qBt-tags"),
            s("a"),
            s("b"),
            s("total_uploaded"),
            s("lots"),
        ))
        .unwrap();
        assert_eq!((r.category, r.tags.len(), r.uploaded), (None, 0, 0));

        // Bad names are dropped one by one; duplicates and extras too.
        let long = "x".repeat(65);
        let mut tags = format!(
            "{}{}{}{}{}{}",
            s("ok"),
            s("ok"),
            s("bad\ncontrol"),
            s("spoof\u{202E}gnp.exe"),
            s(&long),
            s("   "),
        );
        tags.push_str("2:\u{0}\u{1}");
        for i in 0..100 {
            tags.push_str(&s(&format!("t{i}")));
        }
        let r = with(&format!("{}l{tags}e", s("qBt-tags"))).unwrap();
        assert_eq!(r.tags.len(), MAX_TAGS);
        assert_eq!(r.tags[0], "ok");
        assert!(r
            .tags
            .iter()
            .all(|t| clean_label(t.as_bytes()).as_deref() == Some(t.as_str())));
        assert_eq!(
            with(&format!("{}{}", s("qBt-category"), s("")))
                .unwrap()
                .category,
            None
        );
        assert_eq!(clean_label(b"\xff\xfe"), None);
    }

    #[test]
    fn hostile_resume_files_are_ignored() {
        assert_eq!(parse_fastresume(b"not bencode"), None);
        assert_eq!(parse_fastresume(&resume("relative/path", 0)), None);
        assert_eq!(parse_fastresume(b"d9:save_pathi5ee"), None);
        assert_eq!(parse_fastresume(b"d9:save_path2:\xff\xfee"), None);
        let deep = format!("{}{}", "l".repeat(100_000), "e".repeat(100_000));
        assert_eq!(parse_fastresume(deep.as_bytes()), None);
    }

    #[test]
    fn usual_folders_are_absolute_and_counting_adds_nothing() {
        assert!(default_dirs()
            .iter()
            .all(|d| d.is_absolute() && d.ends_with("BT_backup")));
        let d = tempfile::tempdir().unwrap();
        std::fs::write(d.path().join("aaa.fastresume"), resume(&abs(), 0)).unwrap();
        std::fs::write(d.path().join("aaa.torrent"), b"d4:infod4:name1:xee").unwrap();
        std::fs::write(d.path().join("bbb.fastresume"), b"garbage").unwrap();
        let s = scan_counts(Some(d.path())).unwrap();
        assert_eq!((s.found, s.skipped), (1, 1));
        assert_eq!(s.dir.as_deref(), Some(d.path()));
        assert_eq!(s.all_time, None);
    }

    // What QSettings writes for {AlltimeDL: 1234567890123, AlltimeUL: 42}, escaped by hand
    // following Qt's iniEscapedString.
    const SAMPLE: &str = r"[Stats]
AllStats=@Variant(\0\0\0\x1c\0\0\0\x2\0\0\0\x12\0\x41\0l\0l\0t\0i\0m\0\x65\0\x44\0L\0\0\0\x4\0\0\x1\x1fq\xfb\x4\xcb\0\0\0\x12\0\x41\0l\0l\0t\0i\0m\0\x65\0U\0L\0\0\0\x4\0\0\0\0\0\0\0*)
";

    #[test]
    fn reads_qbittorrent_all_time_totals() {
        let want = AllTime {
            downloaded: 1_234_567_890_123,
            uploaded: 42,
        };
        assert_eq!(parse_all_time(SAMPLE.as_bytes()), Some(want));
        // Other sections and keys around it don't matter; CRLF line ends are fine.
        let busy = format!(
            "[General]\r\nfoo=bar\r\n{}\r\n[Other]\r\nAllStats=x\r\n",
            SAMPLE.replace('\n', "\r\n")
        );
        assert_eq!(parse_all_time(busy.as_bytes()), Some(want));
        assert_eq!(
            parse_all_time(SAMPLE.replace("[Stats]", "[Nope]").as_bytes()),
            None
        );
    }

    /// Qt's iniEscapedString for a binary value, so any bytes can be round-tripped.
    fn qt_escape(bytes: &[u8]) -> String {
        let (mut out, mut quotes, mut next_digit) = (String::new(), false, false);
        for &c in bytes {
            quotes |= matches!(c, b';' | b',' | b'=');
            if next_digit && c.is_ascii_hexdigit() {
                out += &format!("\\x{c:x}");
                continue;
            }
            next_digit = false;
            match c {
                0 => {
                    out += "\\0";
                    next_digit = true;
                }
                7 => out += "\\a",
                8 => out += "\\b",
                12 => out += "\\f",
                10 => out += "\\n",
                13 => out += "\\r",
                9 => out += "\\t",
                11 => out += "\\v",
                b'"' | b'\\' => {
                    out.push('\\');
                    out.push(c as char);
                }
                c if c <= 0x1f || c >= 0x7f => {
                    out += &format!("\\x{c:x}");
                    next_digit = true;
                }
                c => out.push(c as char),
            }
        }
        if quotes {
            format!("\"{out}\"")
        } else {
            out
        }
    }

    fn qt_hash(entries: &[(&str, u32, &[u8])]) -> Vec<u8> {
        let mut b = Vec::new();
        b.extend_from_slice(&QT_VARIANT_HASH.to_be_bytes());
        b.extend_from_slice(&(entries.len() as u32).to_be_bytes());
        for (key, ty, value) in entries {
            let utf16: Vec<u8> = key.encode_utf16().flat_map(u16::to_be_bytes).collect();
            b.extend_from_slice(&(utf16.len() as u32).to_be_bytes());
            b.extend_from_slice(&utf16);
            b.extend_from_slice(&ty.to_be_bytes());
            b.extend_from_slice(value);
        }
        b
    }

    fn stats_file(bytes: &[u8]) -> String {
        let v = qt_escape(&[b"@Variant(".as_slice(), bytes, b")"].concat());
        format!("[Stats]\nAllStats={v}\n")
    }

    #[test]
    fn every_escape_round_trips() {
        // Values whose bytes hit quotes, backslashes, ; , =, control bytes and hex digits after escapes.
        for (dl, ul) in [
            (0i64, 0i64),
            (0x0c3b_2c3d_225c_0a0d, 0x0030_0041_0066_7f80),
            (MAX_BYTES, 1),
            (0x0102_0304_0506_0708, 0x00ff_00fe_0000_0039),
        ] {
            let bytes = qt_hash(&[
                ("AlltimeUL", QT_LONG_LONG, &ul.to_be_bytes()),
                ("AlltimeDL", QT_ULONG_LONG, &dl.to_be_bytes()),
            ]);
            let got = parse_all_time(stats_file(&bytes).as_bytes());
            assert_eq!(
                got,
                Some(AllTime {
                    downloaded: dl as u64,
                    uploaded: ul as u64
                }),
                "{}",
                stats_file(&bytes)
            );
        }
    }

    #[test]
    fn hostile_statistics_files_are_refused() {
        let good = |dl: i64| {
            qt_hash(&[
                ("AlltimeDL", QT_LONG_LONG, &dl.to_be_bytes()),
                ("AlltimeUL", QT_LONG_LONG, &1i64.to_be_bytes()),
            ])
        };
        let refused = |bytes: &[u8]| assert_eq!(parse_all_time(stats_file(bytes).as_bytes()), None);
        refused(&good(-1));
        refused(&good(MAX_BYTES + 1));
        refused(&good(i64::MAX));
        // Missing a key, a string where a number belongs, trailing junk, truncation.
        refused(&qt_hash(&[(
            "AlltimeDL",
            QT_LONG_LONG,
            &5i64.to_be_bytes(),
        )]));
        refused(&qt_hash(&[
            ("AlltimeDL", 10, b"\0\0\0\x02\0x"),
            ("AlltimeUL", QT_LONG_LONG, &1i64.to_be_bytes()),
        ]));
        refused(&[good(5), vec![1, 2, 3]].concat());
        let g = good(5);
        for cut in [0, 3, 4, 8, 12, g.len() - 1] {
            refused(&g[..cut]);
        }
        // A huge entry count or key length.
        let mut huge = good(5);
        huge[4..8].copy_from_slice(&u32::MAX.to_be_bytes());
        refused(&huge);
        let mut long_key = good(5);
        long_key[8..12].copy_from_slice(&0x7fff_fff0u32.to_be_bytes());
        refused(&long_key);
        // Not a hash at all, and broken escapes.
        refused(&qt_hash(&[])[..4]);
        for bad in [
            r"@Variant(\x)",
            r"@Variant(\x100)",
            r"@Variant(\q)",
            "@Variant(\u{e9})",
            "@Variant(\\",
            "AllStats",
            "@Variant(abc",
        ] {
            assert_eq!(
                parse_all_time(format!("[Stats]\nAllStats={bad}\n").as_bytes()),
                None,
                "{bad}"
            );
        }
        assert_eq!(parse_all_time(b"\xff\xfe[Stats]"), None);
    }

    #[test]
    fn statistics_come_from_the_same_profile() {
        // A portable profile: data/BT_backup next to config/qBittorrent-data.ini.
        let root = tempfile::tempdir().unwrap();
        let bt = root
            .path()
            .join("qBittorrent")
            .join("data")
            .join("BT_backup");
        let config = root.path().join("qBittorrent").join("config");
        std::fs::create_dir_all(&bt).unwrap();
        std::fs::create_dir_all(&config).unwrap();
        assert_eq!(read_all_time(&bt), None);
        std::fs::write(config.join("qBittorrent-data.ini"), SAMPLE).unwrap();
        assert_eq!(read_all_time(&bt).map(|t| t.uploaded), Some(42));
        // An oversized file is never read.
        let big = format!("{SAMPLE}{}", "#".repeat(MAX_STATS_BYTES as usize));
        std::fs::write(config.join("qBittorrent-data.ini"), big).unwrap();
        assert_eq!(read_all_time(&bt), None);
    }

    #[test]
    fn pairs_resume_files_with_torrents() {
        let d = tempfile::tempdir().unwrap();
        let save = abs();
        std::fs::write(d.path().join("aaa.fastresume"), resume(&save, 0)).unwrap();
        std::fs::write(d.path().join("aaa.torrent"), b"d4:infod4:name1:xee").unwrap();
        // No .torrent next to it.
        std::fs::write(d.path().join("bbb.fastresume"), resume(&save, 0)).unwrap();
        std::fs::write(d.path().join("ccc.fastresume"), b"garbage").unwrap();
        std::fs::write(d.path().join("ccc.torrent"), b"x").unwrap();
        let (found, skipped) = scan(d.path()).unwrap();
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].torrent, d.path().join("aaa.torrent"));
        assert_eq!(skipped, 2);
    }
}
