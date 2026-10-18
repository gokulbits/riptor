//! Turning untrusted torrent path components into safe on-disk paths.
//!
//! Torrent metadata comes from strangers. librqbit rejects `..` and embedded
//! separators, but that is not enough on Windows: a component such as `C:x`
//! carries a drive prefix, and `PathBuf::push` *replaces* the whole path with
//! it. Reserved device names (`CON`, `NUL`, …), alternate data streams
//! (`a:b`), and trailing dots/spaces (silently stripped by Win32, so two
//! "different" names alias the same file) are also dangerous. Every component
//! is therefore rebuilt here, on every platform, so a torrent lands the same
//! way on Windows and Linux.

use std::collections::HashSet;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, ensure};

/// Maximum bytes per component (NTFS and ext4 both cap names at 255).
pub const MAX_COMPONENT_BYTES: usize = 255;

const RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "CLOCK$", "COM0", "COM1", "COM2", "COM3",
    "COM4", "COM5", "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2",
    "LPT3", "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

fn is_forbidden_char(c: char) -> bool {
    c.is_control()
        || matches!(c, '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*')
        // Bidi controls let a name like "photo\u{202E}gnp.exe" display as "photoexe.png".
        || matches!(c, '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
}

fn trim_win32(s: &str) -> &str {
    s.trim_start_matches(' ').trim_end_matches(['.', ' '])
}

fn is_reserved(name: &str) -> bool {
    // Windows treats "CON.txt" and "con .tar.gz" like "CON".
    let stem = name.split('.').next().unwrap_or("").trim_end();
    RESERVED.iter().any(|r| r.eq_ignore_ascii_case(stem))
}

fn truncate_component(s: String) -> String {
    if s.len() <= MAX_COMPONENT_BYTES {
        return s;
    }
    // Keep a short extension so the file type survives.
    let (stem, ext) = match s.rfind('.') {
        Some(i) if s.len() - i <= 16 && i > 0 => (&s[..i], &s[i..]),
        _ => (s.as_str(), ""),
    };
    let mut budget = MAX_COMPONENT_BYTES - ext.len();
    while !stem.is_char_boundary(budget) {
        budget -= 1;
    }
    format!("{}{}", &stem[..budget], ext)
}

/// Sanitize one path component. Never returns an empty string, `.` or `..`.
pub fn sanitize_component(raw: &str) -> String {
    let replaced: String = raw
        .chars()
        .map(|c| if is_forbidden_char(c) { '_' } else { c })
        .collect();
    // Truncation can expose a trailing dot/space or a bare reserved name
    // ("CON" + 300 spaces + "x"), so trim and check again afterwards.
    let fit = |s: String| {
        let t = trim_win32(&truncate_component(s)).to_string();
        // All-dot names ("." / ".." / "...") trim to empty.
        if t.is_empty() {
            "_".to_string()
        } else {
            t
        }
    };
    let mut s = fit(trim_win32(&replaced).to_string());
    if is_reserved(&s) {
        s.insert(0, '_');
        s = fit(s);
    }
    s
}

/// Build a relative path from raw components, sanitizing each one, and prove
/// the result cannot escape whatever it is joined onto.
pub fn sanitize_relative<'a>(
    components: impl IntoIterator<Item = &'a str>,
) -> anyhow::Result<PathBuf> {
    let mut rel = PathBuf::new();
    let mut n = 0usize;
    for c in components {
        rel.push(sanitize_component(c));
        n += 1;
    }
    ensure!(n > 0, "torrent file has no name");
    let mut seen = 0usize;
    for c in rel.components() {
        match c {
            Component::Normal(_) => seen += 1,
            other => bail!("unsafe path component {other:?} in {rel:?}"),
        }
    }
    ensure!(
        seen == n,
        "path components collapsed while sanitizing {rel:?}"
    );
    Ok(rel)
}

/// Join a sanitized relative path onto `root`, refusing anything that would
/// not stay underneath it.
pub fn safe_join(root: &Path, rel: &Path) -> anyhow::Result<PathBuf> {
    ensure!(root.is_absolute(), "save folder must be absolute: {root:?}");
    ensure!(
        rel.components().all(|c| matches!(c, Component::Normal(_))),
        "unsafe relative path {rel:?}"
    );
    let full = root.join(rel);
    ensure!(full.starts_with(root), "{full:?} escapes {root:?}");
    Ok(full)
}

/// Make every path in `paths` unique, case-insensitively (two names that
/// differ only in case are the same file on Windows/macOS), by appending
/// " (n)" before the extension. Different raw names can also collide after
/// sanitizing ("a:b" and "a_b").
pub fn dedupe(paths: &mut [PathBuf]) {
    let key = |p: &Path| p.to_string_lossy().to_lowercase();
    let mut taken: HashSet<String> = HashSet::with_capacity(paths.len());
    for p in paths.iter_mut() {
        if taken.insert(key(p)) {
            continue;
        }
        let stem = p
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ext = p
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy()))
            .unwrap_or_default();
        for n in 1u32.. {
            let candidate = p.with_file_name(sanitize_component(&format!("{stem} ({n}){ext}")));
            if taken.insert(key(&candidate)) {
                *p = candidate;
                break;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prelude::*;

    #[test]
    fn components() {
        let cases = [
            ("normal.txt", "normal.txt"),
            ("..", "_"),
            (".", "_"),
            ("...", "_"),
            ("", "_"),
            ("C:", "C_"),
            ("C:evil", "C_evil"),
            ("a/b", "a_b"),
            ("a\\b", "a_b"),
            ("file.txt:stream", "file.txt_stream"),
            ("trailing. . ", "trailing"),
            ("  leading", "leading"),
            ("CON", "_CON"),
            ("con.txt", "_con.txt"),
            ("Con .tar.gz", "_Con .tar.gz"),
            ("COM1", "_COM1"),
            ("lpt9.log", "_lpt9.log"),
            ("CONSOLE", "CONSOLE"),
            ("nul\u{0}x", "nul_x"),
            ("bad<>|?*\"", "bad______"),
            ("photo\u{202E}gnp.exe", "photo_gnp.exe"),
            ("tab\there", "tab_here"),
            ("日本語.mkv", "日本語.mkv"),
        ];
        for (raw, want) in cases {
            assert_eq!(sanitize_component(raw), want, "raw={raw:?}");
        }
    }

    #[test]
    fn long_names_keep_extension_and_char_boundaries() {
        let s = sanitize_component(&format!("{}.mkv", "é".repeat(300)));
        assert!(s.len() <= MAX_COMPONENT_BYTES);
        assert!(s.ends_with(".mkv"));
        let s = sanitize_component(&"x".repeat(1000));
        assert_eq!(s.len(), MAX_COMPONENT_BYTES);
    }

    #[test]
    fn drive_prefix_cannot_escape() {
        let root = if cfg!(windows) {
            Path::new(r"C:\dl")
        } else {
            Path::new("/dl")
        };
        let rel = sanitize_relative(["dir", "C:evil", "x.exe"]).unwrap();
        let full = safe_join(root, &rel).unwrap();
        assert!(full.starts_with(root));
        assert_eq!(rel.components().count(), 3);
    }

    #[test]
    fn safe_join_rejects_relative_root_and_bad_rel() {
        assert!(safe_join(Path::new("relative"), Path::new("a")).is_err());
        let root = std::env::temp_dir();
        assert!(safe_join(&root, Path::new("../x")).is_err());
        assert!(safe_join(&root, Path::new("a/../../x")).is_err());
    }

    #[test]
    fn empty_component_list_is_rejected() {
        assert!(sanitize_relative(std::iter::empty()).is_err());
    }

    #[test]
    fn dedupe_collisions() {
        let mut v = vec![
            PathBuf::from("d/a_b.txt"),
            PathBuf::from("d/A_B.txt"),
            PathBuf::from("d/a_b.txt"),
            PathBuf::from("d/other"),
        ];
        dedupe(&mut v);
        assert_eq!(
            v,
            vec![
                PathBuf::from("d/a_b.txt"),
                PathBuf::from("d/A_B (1).txt"),
                PathBuf::from("d/a_b (2).txt"),
                PathBuf::from("d/other"),
            ]
        );
    }

    proptest! {
        #![proptest_config(ProptestConfig::with_cases(2000))]

        #[test]
        fn any_input_yields_a_safe_single_component(raw in any::<String>()) {
            let s = sanitize_component(&raw);
            prop_assert!(!s.is_empty() && s != "." && s != "..");
            prop_assert!(s.len() <= MAX_COMPONENT_BYTES);
            prop_assert!(!s.chars().any(is_forbidden_char));
            prop_assert!(!s.ends_with('.') && !s.ends_with(' '));
            prop_assert!(!is_reserved(&s));
            let p = Path::new(&s);
            let comps: Vec<_> = p.components().collect();
            prop_assert_eq!(comps.len(), 1);
            prop_assert!(matches!(comps[0], Component::Normal(_)));
        }

        #[test]
        fn joined_paths_stay_under_root(parts in proptest::collection::vec(any::<String>(), 1..6)) {
            let root = std::env::temp_dir().join("riptor-root");
            let rel = sanitize_relative(parts.iter().map(String::as_str)).unwrap();
            let full = safe_join(&root, &rel).unwrap();
            prop_assert!(full.starts_with(&root));
            prop_assert_eq!(rel.components().count(), parts.len());
        }
    }
}
