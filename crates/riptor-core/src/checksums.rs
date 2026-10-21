//! Checks downloads against checksum files (SHA256SUMS, .sfv, .md5…) shipped in the torrent.

use std::collections::HashMap;
use std::io::Read;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

const MAX_MANIFEST_BYTES: u64 = 4 * 1024 * 1024;
const MAX_MANIFESTS: usize = 20;
const MAX_ENTRIES: usize = 100_000;
const MAX_LISTED: usize = 50;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Algo {
    Crc32,
    Md5,
    Sha1,
    Sha256,
    Sha512,
}

impl Algo {
    fn hex_len(self) -> usize {
        match self {
            Algo::Crc32 => 8,
            Algo::Md5 => 32,
            Algo::Sha1 => 40,
            Algo::Sha256 => 64,
            Algo::Sha512 => 128,
        }
    }

    fn from_hex_len(n: usize) -> Option<Self> {
        [Algo::Md5, Algo::Sha1, Algo::Sha256, Algo::Sha512]
            .into_iter()
            .find(|a| a.hex_len() == n)
    }

    fn from_tag(tag: &str) -> Option<Self> {
        match tag.to_ascii_uppercase().as_str() {
            "MD5" => Some(Algo::Md5),
            "SHA1" => Some(Algo::Sha1),
            "SHA256" => Some(Algo::Sha256),
            "SHA512" => Some(Algo::Sha512),
            "CRC32" => Some(Algo::Crc32),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Entry {
    pub algo: Algo,
    /// Relative to the checksum file's folder, `/` between folders.
    pub name: String,
    pub expected: Vec<u8>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChecksumReport {
    pub checked_at: u64,
    /// The checksum files used, relative to the torrent.
    pub manifests: Vec<String>,
    pub verified: u32,
    /// Files whose contents don't match (first few).
    pub mismatched: Vec<String>,
    pub mismatched_count: u32,
    /// Listed but not downloaded (deselected or unfinished).
    pub skipped: u32,
    /// Listed but not part of this torrent.
    pub not_found: u32,
}

impl ChecksumReport {
    pub fn ok(&self) -> bool {
        self.mismatched_count == 0 && self.verified > 0
    }
}

/// One file of the torrent, as it is on disk.
pub struct FileRef {
    /// Relative to the torrent's folder, `/` between folders.
    pub rel: String,
    pub path: PathBuf,
    pub complete: bool,
}

pub fn is_manifest(rel: &str) -> bool {
    let name = rel.rsplit(['/', '\\']).next().unwrap_or(rel);
    let lower = name.to_ascii_lowercase();
    let ext = lower.rsplit_once('.').map(|(_, e)| e);
    let base = lower.strip_suffix(".txt").unwrap_or(&lower);
    matches!(ext, Some("sfv" | "md5" | "sha1" | "sha256" | "sha512"))
        || ["md5", "sha1", "sha256", "sha512"]
            .iter()
            .any(|a| base == format!("{a}sums") || base == format!("{a}sum"))
}

fn algo_for_manifest(name: &str) -> Option<Algo> {
    let lower = name.to_ascii_lowercase();
    if lower.ends_with(".sfv") {
        Some(Algo::Crc32)
    } else if lower.contains("sha512") {
        Some(Algo::Sha512)
    } else if lower.contains("sha256") {
        Some(Algo::Sha256)
    } else if lower.contains("sha1") {
        Some(Algo::Sha1)
    } else if lower.contains("md5") {
        Some(Algo::Md5)
    } else {
        None
    }
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    if !s.len().is_multiple_of(2) || !s.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

/// A listed name made relative and plain, or `None` if it tries to leave its folder.
fn clean_name(name: &str) -> Option<String> {
    let name = name.trim_end_matches(['\r', '\n']);
    if name.starts_with(['/', '\\']) || name.get(1..2) == Some(":") {
        return None;
    }
    let mut parts = Vec::new();
    for p in name.split(['/', '\\']) {
        match p {
            "" | "." => continue,
            ".." => return None,
            p if p.chars().any(|c| c.is_control()) => return None,
            p => parts.push(p),
        }
    }
    (!parts.is_empty()).then(|| parts.join("/"))
}

fn entry(algo: Algo, hex: &str, name: &str) -> Option<Entry> {
    if hex.len() != algo.hex_len() {
        return None;
    }
    Some(Entry {
        algo,
        name: clean_name(name)?,
        expected: unhex(hex)?,
    })
}

/// Reads GNU (`hash  name`, `hash *name`), BSD (`SHA256 (name) = hash`) and SFV (`name crc`) lines.
pub fn parse(manifest_name: &str, text: &str) -> Vec<Entry> {
    let default = algo_for_manifest(manifest_name);
    let sfv = default == Some(Algo::Crc32);
    let mut out = Vec::new();
    for line in text.lines() {
        if out.len() >= MAX_ENTRIES {
            break;
        }
        let line = line.trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
            continue;
        }
        // BSD style.
        if let Some((tag, rest)) = line.split_once(" (") {
            if let (Some(algo), Some((name, hex))) = (Algo::from_tag(tag), rest.rsplit_once(") = "))
            {
                out.extend(entry(algo, hex.trim(), name));
                continue;
            }
        }
        if sfv {
            if let Some((name, crc)) = line.rsplit_once(char::is_whitespace) {
                out.extend(entry(Algo::Crc32, crc.trim(), name.trim()));
            }
            continue;
        }
        // GNU style; a leading backslash means the name is escaped.
        let (escaped, line) = match line.strip_prefix('\\') {
            Some(l) => (true, l),
            None => (false, line),
        };
        let Some((hex, name)) = line.split_once(char::is_whitespace) else {
            continue;
        };
        let name = name.trim_start_matches([' ', '\t']);
        let name = name.strip_prefix('*').unwrap_or(name);
        let name = if escaped {
            name.replace("\\n", "\n").replace("\\\\", "\\")
        } else {
            name.to_string()
        };
        let Some(algo) = default.or_else(|| Algo::from_hex_len(hex.len())) else {
            continue;
        };
        out.extend(entry(algo, hex, &name));
    }
    out
}

enum Hasher {
    Crc32(crc32fast::Hasher),
    Md5(md5::Md5),
    Sha1(sha1::Sha1),
    Sha256(sha2::Sha256),
    Sha512(sha2::Sha512),
}

impl Hasher {
    fn new(a: Algo) -> Self {
        match a {
            Algo::Crc32 => Self::Crc32(crc32fast::Hasher::new()),
            Algo::Md5 => Self::Md5(md5::Digest::new()),
            Algo::Sha1 => Self::Sha1(sha1::Digest::new()),
            Algo::Sha256 => Self::Sha256(sha2::Digest::new()),
            Algo::Sha512 => Self::Sha512(sha2::Digest::new()),
        }
    }

    fn update(&mut self, b: &[u8]) {
        match self {
            Self::Crc32(h) => h.update(b),
            Self::Md5(h) => md5::Digest::update(h, b),
            Self::Sha1(h) => sha1::Digest::update(h, b),
            Self::Sha256(h) => sha2::Digest::update(h, b),
            Self::Sha512(h) => sha2::Digest::update(h, b),
        }
    }

    fn finish(self) -> Vec<u8> {
        match self {
            Self::Crc32(h) => h.finalize().to_be_bytes().to_vec(),
            Self::Md5(h) => md5::Digest::finalize(h).to_vec(),
            Self::Sha1(h) => sha1::Digest::finalize(h).to_vec(),
            Self::Sha256(h) => sha2::Digest::finalize(h).to_vec(),
            Self::Sha512(h) => sha2::Digest::finalize(h).to_vec(),
        }
    }
}

pub fn hash_file(path: &std::path::Path, algo: Algo) -> std::io::Result<Vec<u8>> {
    let mut f = std::fs::File::open(path)?;
    let mut h = Hasher::new(algo);
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf)?;
        if n == 0 {
            return Ok(h.finish());
        }
        h.update(&buf[..n]);
    }
}

fn read_manifest(f: &FileRef) -> Option<String> {
    let file = std::fs::File::open(&f.path).ok()?;
    let mut bytes = Vec::new();
    file.take(MAX_MANIFEST_BYTES + 1)
        .read_to_end(&mut bytes)
        .ok()?;
    if bytes.len() as u64 > MAX_MANIFEST_BYTES {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

/// `None` when the torrent has no usable checksum files.
pub fn verify(files: &[FileRef], now: u64) -> Option<ChecksumReport> {
    let by_rel: HashMap<&str, &FileRef> = files.iter().map(|f| (f.rel.as_str(), f)).collect();
    let by_lower: HashMap<String, &FileRef> =
        files.iter().map(|f| (f.rel.to_lowercase(), f)).collect();
    let mut report = ChecksumReport {
        checked_at: now,
        ..Default::default()
    };
    let mut done = std::collections::HashSet::new();
    for m in files
        .iter()
        .filter(|f| f.complete && is_manifest(&f.rel))
        .take(MAX_MANIFESTS)
    {
        let Some(text) = read_manifest(m) else {
            continue;
        };
        let dir = m.rel.rsplit_once('/').map_or("", |(d, _)| d);
        let entries = parse(&m.rel, &text);
        if entries.is_empty() {
            continue;
        }
        report.manifests.push(m.rel.clone());
        for e in entries {
            let rel = if dir.is_empty() {
                e.name.clone()
            } else {
                format!("{dir}/{}", e.name)
            };
            let target = by_rel
                .get(rel.as_str())
                .or_else(|| by_lower.get(&rel.to_lowercase()));
            let Some(target) = target else {
                report.not_found += 1;
                continue;
            };
            if !done.insert((target.rel.clone(), e.algo as u8)) {
                continue;
            }
            if !target.complete {
                report.skipped += 1;
                continue;
            }
            match hash_file(&target.path, e.algo) {
                Ok(h) if h == e.expected => report.verified += 1,
                Ok(_) => {
                    report.mismatched_count += 1;
                    if report.mismatched.len() < MAX_LISTED {
                        report.mismatched.push(target.rel.clone());
                    }
                }
                Err(_) => report.skipped += 1,
            }
        }
    }
    (!report.manifests.is_empty()).then_some(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manifests_are_recognised() {
        for yes in [
            "SHA256SUMS",
            "iso/SHA256SUMS.txt",
            "Release.sfv",
            "a\\b\\movie.MD5",
            "file.sha512",
            "md5sum.txt",
        ] {
            assert!(is_manifest(yes), "{yes}");
        }
        for no in ["SHA256SUMS.gpg2x", "sums.txt", "movie.mkv", "md5sumsx"] {
            assert!(!is_manifest(no), "{no}");
        }
    }

    #[test]
    fn gnu_bsd_and_sfv_lines_parse() {
        let sha = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";
        let text = format!(
            "{sha}  debian.iso\n{sha} *bin/tool.img\n# comment\n\nSHA256 (bsd.iso) = {sha}\nbad line\n"
        );
        let e = parse("SHA256SUMS", &text);
        let names: Vec<_> = e.iter().map(|e| e.name.as_str()).collect();
        assert_eq!(names, ["debian.iso", "bin/tool.img", "bsd.iso"]);
        assert!(e.iter().all(|e| e.algo == Algo::Sha256));

        let sfv = parse("x.sfv", "; made by hand\nPart 1.rar  0A1B2C3D\nshort 123\n");
        assert_eq!(sfv.len(), 1);
        assert_eq!(sfv[0].name, "Part 1.rar");
        assert_eq!(sfv[0].expected, vec![0x0a, 0x1b, 0x2c, 0x3d]);

        // Unknown file name: the hash length decides.
        let md5 = parse("checks.txt", "d41d8cd98f00b204e9800998ecf8427e  empty\n");
        assert_eq!(md5[0].algo, Algo::Md5);
    }

    #[test]
    fn names_cannot_leave_the_folder() {
        let sha1 = "da39a3ee5e6b4b0d3255bfef95601890afd80709";
        for bad in [
            "../../etc/passwd",
            "/etc/shadow",
            "\\Windows\\win.ini",
            "C:\\Windows\\win.ini",
            "a/../../b",
        ] {
            assert!(
                parse("SHA1SUMS", &format!("{sha1}  {bad}\n")).is_empty(),
                "{bad}"
            );
        }
        assert_eq!(
            parse("SHA1SUMS", &format!("{sha1}  ./a\\b.txt\n"))[0].name,
            "a/b.txt"
        );
    }

    #[test]
    fn hostile_manifests_do_not_panic() {
        let long = "a".repeat(100_000);
        for text in [
            long.as_str(),
            "SHA256 (",
            "SHA256 (x) = zz",
            "\u{feff}\u{202e}",
            "0 \u{0}",
            "  *",
        ] {
            let _ = parse("SHA256SUMS", text);
            let _ = parse("x.sfv", text);
        }
        let many = "d41d8cd98f00b204e9800998ecf8427e  f\n".repeat(MAX_ENTRIES + 10);
        assert_eq!(parse("MD5SUMS", &many).len(), MAX_ENTRIES);
    }

    #[test]
    fn files_are_verified_against_their_manifest() {
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        std::fs::create_dir_all(root.join("iso")).unwrap();
        std::fs::write(root.join("iso/good.iso"), b"hello").unwrap();
        std::fs::write(root.join("iso/bad.iso"), b"tampered").unwrap();
        // sha256("hello") and sha256("world").
        let manifest =
            "2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  good.iso\n\
             486ea46224d1bb4fb680f34f7c9ad96a8f24ec88be73ea8e5a6c65260e9cb8a7  bad.iso\n\
             2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  notes.txt\n\
             2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  partial.iso\n\
             2cf24dba5fb0a30e26e83b2ac5b9e29e1b161e5c1fa7425e73043362938b9824  ../outside\n";
        std::fs::write(root.join("iso/SHA256SUMS"), manifest).unwrap();
        let f = |rel: &str, complete| FileRef {
            rel: rel.into(),
            path: root.join(rel),
            complete,
        };
        let files = [
            f("iso/SHA256SUMS", true),
            f("iso/good.iso", true),
            f("iso/bad.iso", true),
            f("iso/partial.iso", false),
        ];
        let r = verify(&files, 7).unwrap();
        assert_eq!(r.manifests, ["iso/SHA256SUMS"]);
        assert_eq!(r.verified, 1);
        assert_eq!(r.mismatched, ["iso/bad.iso"]);
        assert_eq!((r.skipped, r.not_found), (1, 1));
        assert!(!r.ok());

        // A manifest that isn't downloaded yet isn't read.
        assert!(verify(&[f("iso/SHA256SUMS", false), f("iso/good.iso", true)], 0).is_none());
    }

    #[test]
    fn every_algorithm_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("x");
        std::fs::write(&p, b"abc").unwrap();
        let hex = |a| {
            hash_file(&p, a)
                .unwrap()
                .iter()
                .map(|b| format!("{b:02x}"))
                .collect::<String>()
        };
        assert_eq!(hex(Algo::Crc32), "352441c2");
        assert_eq!(hex(Algo::Md5), "900150983cd24fb0d6963f7d28e17f72");
        assert_eq!(hex(Algo::Sha1), "a9993e364706816aba3e25717850c26c9cd0d89d");
        assert_eq!(
            hex(Algo::Sha256),
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(hex(Algo::Sha512).starts_with("ddaf35a193617aba"));
    }
}
