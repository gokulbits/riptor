//! Tricks fake releases use to get malware run: disguised names and disguised contents.

use std::io::Read;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::input::is_executable_name;

const MAX_FINDINGS: usize = 20;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Level {
    Caution,
    Danger,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Finding {
    pub level: Level,
    pub message: String,
    /// Relative to the torrent's folder.
    #[serde(default)]
    pub file: Option<String>,
}

pub fn worst(findings: &[Finding]) -> Option<Level> {
    findings.iter().map(|f| f.level).max()
}

/// Characters that reorder or hide text, used to fake an extension.
fn is_hiding_char(c: char) -> bool {
    matches!(c,
        '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}' | '\u{061C}'
        | '\u{200B}' | '\u{FEFF}' | '\u{180E}')
}

const VIDEO: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "webm", "wmv", "ts", "m2ts", "mpg", "mpeg", "flv", "ogv",
];
const AUDIO: &[&str] = &["mp3", "flac", "ogg", "opus", "m4a", "wav", "aac", "wma"];
const IMAGE: &[&str] = &["jpg", "jpeg", "png", "gif", "webp", "bmp"];
const TEXT: &[&str] = &[
    "txt", "nfo", "srt", "sub", "ass", "ssa", "vtt", "md", "log", "cue",
];
const ARCHIVE: &[&str] = &["zip", "rar", "7z", "gz", "tar", "bz2", "xz", "cab"];
/// What a disguised program pretends to be.
const DECOY: &[&str] = &[
    "mkv", "mp4", "m4v", "avi", "mov", "wmv", "mpg", "mp3", "flac", "wav", "pdf", "doc", "docx",
    "xls", "xlsx", "txt", "nfo", "jpg", "jpeg", "png", "gif", "srt", "zip", "rar", "7z", "iso",
    "epub",
];
/// Run a program or a script when opened.
const SHORTCUT: &[&str] = &["lnk", "pif", "scr", "hta", "scf", "cpl", "url", "website"];
const RELEASE_TAGS: &[&str] = &[
    "720p", "1080p", "2160p", "4k", "bluray", "blu-ray", "bdrip", "brrip", "webrip", "web-dl",
    "webdl", "hdtv", "dvdrip", "hdrip", "x264", "x265", "hevc", "remux",
];

fn ext(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.trim().to_ascii_lowercase())
        .unwrap_or_default()
}

fn base_name(rel: &str) -> &str {
    rel.rsplit(['/', '\\']).next().unwrap_or(rel)
}

fn finding(level: Level, message: String, file: Option<&str>) -> Finding {
    Finding {
        level,
        message,
        file: file.map(str::to_string),
    }
}

/// Checks a torrent's name and file list. `files`: (path relative to the torrent, size).
pub fn check_names(name: &str, files: &[(String, u64)]) -> Vec<Finding> {
    let mut out = Vec::new();
    let mut push = |f: Finding| {
        if out.len() < MAX_FINDINGS {
            out.push(f);
        }
    };
    let total: u64 = files.iter().map(|f| f.1).sum();
    let media_bytes: u64 = files
        .iter()
        .filter(|(p, _)| {
            let e = ext(p);
            VIDEO.contains(&e.as_str()) || AUDIO.contains(&e.as_str())
        })
        .map(|f| f.1)
        .sum();
    let mostly_media = total > 0 && media_bytes * 10 >= total * 8;

    if name.chars().any(is_hiding_char) {
        push(finding(
            Level::Danger,
            "The torrent's name contains hidden characters that can disguise a file type.".into(),
            None,
        ));
    }
    for (rel, _) in files {
        let file = base_name(rel);
        let e = ext(file);
        if rel.chars().any(is_hiding_char) {
            push(finding(
                Level::Danger,
                format!("{file:?} hides its real file type with invisible characters."),
                Some(rel),
            ));
            continue;
        }
        let stem = file.rsplit_once('.').map_or("", |(s, _)| s);
        let inner = ext(stem);
        if is_executable_name(file) && DECOY.contains(&inner.as_str()) {
            push(finding(
                Level::Danger,
                format!("{file} is a program pretending to be a .{inner} file."),
                Some(rel),
            ));
            continue;
        }
        if stem.ends_with("     ") || stem.contains("          ") {
            push(finding(
                Level::Caution,
                format!("{file}: the name is padded with spaces, which can push its real type out of sight."),
                Some(rel),
            ));
        }
        if SHORTCUT.contains(&e.as_str()) {
            push(finding(
                if e == "url" || e == "website" {
                    Level::Caution
                } else {
                    Level::Danger
                },
                format!("{file} is a shortcut or screen saver: opening it runs something else."),
                Some(rel),
            ));
            continue;
        }
        if mostly_media && is_executable_name(file) {
            push(finding(
                Level::Danger,
                format!("{file}: a video or music release doesn't need programs. Fake \"codec\" or \"player\" installers are a common way to spread malware."),
                Some(rel),
            ));
        }
    }

    let has_archive = files
        .iter()
        .any(|(p, _)| ARCHIVE.contains(&ext(base_name(p)).as_str()));
    if has_archive {
        for (rel, _) in files {
            let file = base_name(rel);
            let lower = file.to_ascii_lowercase();
            let e = ext(file);
            let hint = ["password", "passwd", "pass.", "unlock", "key.txt"]
                .iter()
                .any(|w| lower.contains(w));
            if hint && (TEXT.contains(&e.as_str()) || matches!(e.as_str(), "url" | "html" | "htm"))
            {
                push(finding(
                    Level::Caution,
                    format!("{file}: an archive with its password in a separate file is a common way to hide malware from virus scanners."),
                    Some(rel),
                ));
            }
        }
    }

    let lower_name = name.to_ascii_lowercase();
    let claims_video = RELEASE_TAGS.iter().any(|t| lower_name.contains(t));
    if claims_video {
        let biggest_video = files
            .iter()
            .filter(|(p, _)| VIDEO.contains(&ext(base_name(p)).as_str()))
            .map(|f| f.1)
            .max();
        match biggest_video {
            None if files.iter().any(|(p, _)| {
                is_executable_name(base_name(p)) || ARCHIVE.contains(&ext(base_name(p)).as_str())
            }) =>
            {
                push(finding(
                    Level::Caution,
                    "Its name promises a video, but it contains no video files.".into(),
                    None,
                ))
            }
            Some(len) if len < 20 * 1024 * 1024 => push(finding(
                Level::Caution,
                "Its name promises a high-quality video, but the video file is tiny.".into(),
                None,
            )),
            _ => {}
        }
    }
    out
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Kind {
    Program,
    Archive,
    Other,
}

fn sniff(head: &[u8]) -> Kind {
    const PROGRAMS: &[&[u8]] = &[
        b"MZ",
        b"\x7fELF",
        b"#!",
        b"\xfe\xed\xfa\xce",
        b"\xfe\xed\xfa\xcf",
        b"\xce\xfa\xed\xfe",
        b"\xcf\xfa\xed\xfe",
        b"L\x00\x00\x00\x01\x14\x02\x00",
        b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1",
    ];
    const ARCHIVES: &[&[u8]] = &[
        b"PK\x03\x04",
        b"Rar!\x1a\x07",
        b"7z\xbc\xaf\x27\x1c",
        b"\x1f\x8b",
        b"MSCF",
    ];
    if PROGRAMS.iter().any(|m| head.starts_with(m)) {
        Kind::Program
    } else if ARCHIVES.iter().any(|m| head.starts_with(m)) {
        Kind::Archive
    } else {
        Kind::Other
    }
}

/// Compares a finished file's first bytes with what its name says it is.
pub fn check_content(rel: &str, path: &Path) -> Option<Finding> {
    let file = base_name(rel);
    let e = ext(file);
    let e = e.as_str();
    let plain = VIDEO.contains(&e)
        || AUDIO.contains(&e)
        || IMAGE.contains(&e)
        || TEXT.contains(&e)
        || e == "pdf";
    if !plain {
        return None;
    }
    let mut head = [0u8; 16];
    let n = std::fs::File::open(path)
        .and_then(|f| f.take(16).read(&mut head))
        .ok()?;
    match sniff(&head[..n]) {
        Kind::Program => Some(finding(
            Level::Danger,
            format!("{file} is really a program or installer, not a .{e} file. Don't open it."),
            Some(rel),
        )),
        Kind::Archive if !TEXT.contains(&e) => Some(finding(
            Level::Caution,
            format!("{file} is really an archive, not a .{e} file."),
            Some(rel),
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn files(list: &[(&str, u64)]) -> Vec<(String, u64)> {
        list.iter().map(|(p, n)| (p.to_string(), *n)).collect()
    }

    const MIB: u64 = 1024 * 1024;

    #[test]
    fn honest_releases_are_quiet() {
        for (name, list) in [
            (
                "Show.S01E01.1080p.WEB-DL",
                files(&[
                    ("Show.S01E01.1080p.mkv", 1500 * MIB),
                    ("Show.nfo", 2000),
                    ("Subs/eng.srt", 50_000),
                ]),
            ),
            (
                "debian-13.iso",
                files(&[("debian-13.iso", 700 * MIB), ("SHA256SUMS", 300)]),
            ),
            (
                "Tool 2.0",
                files(&[("setup.exe", 30 * MIB), ("readme.txt", 900)]),
            ),
            (
                "Album (2020) FLAC",
                files(&[("01 Intro.flac", 30 * MIB), ("cover.jpg", MIB)]),
            ),
        ] {
            assert_eq!(check_names(name, &list), [], "{name}");
        }
    }

    #[test]
    fn disguised_names_are_flagged() {
        let check = |name: &str, list: &[(&str, u64)]| worst(&check_names(name, &files(list)));
        // A right-to-left override shows "exe.mp4" reversed.
        assert_eq!(
            check("Movie", &[("Movie\u{202e}4pm.exe", MIB)]),
            Some(Level::Danger)
        );
        assert_eq!(
            check("Movie", &[("Movie.2024.mp4.exe", MIB)]),
            Some(Level::Danger)
        );
        assert_eq!(
            check("Movie", &[("Movie.mp4 .scr", MIB)]),
            Some(Level::Danger)
        );
        assert_eq!(
            check("Docs", &[("Invoice.pdf.js", 10_000)]),
            Some(Level::Danger)
        );
        assert_eq!(check("Pack", &[("Play me.lnk", 2000)]), Some(Level::Danger));
        assert_eq!(
            check(
                "Film 1080p",
                &[("Film.mkv", 2000 * MIB), ("Codec Installer.exe", 2 * MIB)]
            ),
            Some(Level::Danger)
        );
        assert_eq!(
            check("Film", &[("Film.rar", 900 * MIB), ("PASSWORD.txt", 30)]),
            Some(Level::Caution)
        );
        assert_eq!(
            check("Film.2024.2160p.BluRay", &[("Film.mkv", 3 * MIB)]),
            Some(Level::Caution)
        );
        assert_eq!(
            check("Film.2024.1080p.WEB-DL", &[("Film.zip", 900 * MIB)]),
            Some(Level::Caution)
        );
        assert_eq!(
            check("Film", &[("Film.mp4                    .exe", MIB)]),
            Some(Level::Danger)
        );
    }

    #[test]
    fn findings_are_capped() {
        let many: Vec<(String, u64)> = (0..500).map(|i| (format!("{i}.mp4.exe"), 1)).collect();
        assert_eq!(check_names("x", &many).len(), MAX_FINDINGS);
    }

    #[test]
    fn contents_are_compared_with_names() {
        let dir = tempfile::tempdir().unwrap();
        let w = |name: &str, bytes: &[u8]| {
            let p = dir.path().join(name);
            std::fs::write(&p, bytes).unwrap();
            check_content(name, &p).map(|f| f.level)
        };
        assert_eq!(
            w("movie.mp4", b"MZ\x90\x00rest of a PE file"),
            Some(Level::Danger)
        );
        assert_eq!(w("song.mp3", b"\x7fELF\x02\x01"), Some(Level::Danger));
        assert_eq!(w("photo.jpg", b"PK\x03\x04zip"), Some(Level::Caution));
        assert_eq!(w("readme.txt", b"#!/bin/sh\nrm -rf ~"), Some(Level::Danger));
        assert_eq!(w("movie.mkv", b"\x1a\x45\xdf\xa3matroska"), None);
        assert_eq!(w("photo.jpg", b"\xff\xd8\xff\xe0"), None);
        // Archives and documents are allowed to look like archives and documents.
        assert_eq!(w("book.epub", b"PK\x03\x04"), None);
        assert_eq!(w("old.doc", b"\xd0\xcf\x11\xe0\xa1\xb1\x1a\xe1"), None);
        assert_eq!(w("empty.mp4", b""), None);
        assert_eq!(
            check_content("gone.mp4", &dir.path().join("gone.mp4")),
            None
        );
    }
}
