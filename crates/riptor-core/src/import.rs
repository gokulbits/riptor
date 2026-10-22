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
}

/// Where qBittorrent keeps its resume files, most likely first.
pub fn default_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    #[cfg(windows)]
    if let Some(local) = std::env::var_os("LOCALAPPDATA").map(PathBuf::from) {
        if local.is_absolute() {
            dirs.push(local.join("qBittorrent").join("BT_backup"));
        }
    }
    #[cfg(not(windows))]
    if let Some(home) = std::env::var_os("HOME").map(PathBuf::from) {
        let data = std::env::var_os("XDG_DATA_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| home.join(".local").join("share"));
        dirs.push(data.join("qBittorrent").join("BT_backup"));
        // The Flatpak build keeps its data in its sandbox.
        dirs.push(home.join(".var/app/org.qbittorrent.qBittorrent/data/qBittorrent/BT_backup"));
    }
    dirs
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
