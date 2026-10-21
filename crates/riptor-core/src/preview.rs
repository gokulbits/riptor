//! Images (checked by content, never SVG) and plain text shown inside Riptor.

use std::io::Read;
use std::path::Path;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde::Serialize;

pub const MAX_IMAGE_BYTES: u64 = 20 * 1024 * 1024;
const MAX_TEXT_BYTES: u64 = 512 * 1024;

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FilePreview {
    Image {
        mime: &'static str,
        data: String,
    },
    Text {
        text: String,
        truncated: bool,
    },
    /// Why it can't be shown.
    Unsupported {
        reason: String,
    },
}

const TEXT_EXTS: &[&str] = &[
    "txt", "nfo", "diz", "srt", "ass", "ssa", "vtt", "sub", "md", "log", "cue", "sfv", "md5",
    "sha1", "sha256", "sha512", "json", "xml", "csv", "ini", "m3u", "m3u8", "rst",
];
const IMAGE_EXTS: &[&str] = &["png", "jpg", "jpeg", "gif", "webp", "bmp"];

fn ext(name: &str) -> String {
    name.rsplit_once('.')
        .map(|(_, e)| e.to_ascii_lowercase())
        .unwrap_or_default()
}

/// Can this be previewed at all (by name)?
pub fn previewable(name: &str) -> bool {
    let e = ext(name);
    TEXT_EXTS.contains(&e.as_str())
        || IMAGE_EXTS.contains(&e.as_str())
        || crate::checksums::is_manifest(name)
}

fn image_mime(head: &[u8]) -> Option<&'static str> {
    if head.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some("image/png")
    } else if head.starts_with(b"\xff\xd8\xff") {
        Some("image/jpeg")
    } else if head.starts_with(b"GIF87a") || head.starts_with(b"GIF89a") {
        Some("image/gif")
    } else if head.len() >= 12 && &head[..4] == b"RIFF" && &head[8..12] == b"WEBP" {
        Some("image/webp")
    } else if head.starts_with(b"BM") {
        Some("image/bmp")
    } else {
        None
    }
}

/// The upper half of code page 437, which .nfo files are written in.
const CP437_HIGH: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜ¢£¥₧ƒáíóúñÑªº¿⌐¬½¼¡«»░▒▓│┤╡╢╖╕╣║╗╝╜╛┐└┴┬├─┼╞╟╚╔╩╦╠═╬╧╨╤╥╙╘╒╓╫╪┘┌█▄▌▐▀αßΓπΣσµτΦΘΩδ∞φε∩≡±≥≤⌠⌡÷≈°∙·√ⁿ²■\u{a0}";

fn cp437(bytes: &[u8]) -> String {
    let high: Vec<char> = CP437_HIGH.chars().collect();
    bytes
        .iter()
        .map(|b| match b {
            0..=0x7f => *b as char,
            _ => high[(*b - 0x80) as usize],
        })
        .collect()
}

/// Keeps line breaks and tabs; drops other control characters (including bidi tricks).
fn clean_text(s: &str) -> String {
    s.chars()
        .filter(|c| {
            matches!(c, '\n' | '\t')
                || !(c.is_control()
                    || matches!(c, '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}' | '\u{200E}' | '\u{200F}'))
        })
        .collect()
}

fn decode(bytes: &[u8], nfo: bool) -> String {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(e) if bytes.len() - e.valid_up_to() < 4 => {
            // Cut mid-character by the size limit.
            String::from_utf8_lossy(&bytes[..e.valid_up_to()]).into_owned()
        }
        Err(_) if nfo => cp437(bytes),
        Err(_) => String::from_utf8_lossy(bytes).into_owned(),
    }
}

pub fn read(path: &Path, name: &str) -> anyhow::Result<FilePreview> {
    let e = ext(name);
    let file = std::fs::File::open(path)?;
    let len = file.metadata()?.len();
    if IMAGE_EXTS.contains(&e.as_str()) {
        if len > MAX_IMAGE_BYTES {
            return Ok(FilePreview::Unsupported {
                reason: "This image is too large to show here.".into(),
            });
        }
        let mut bytes = Vec::with_capacity(len as usize);
        file.take(MAX_IMAGE_BYTES).read_to_end(&mut bytes)?;
        return Ok(match image_mime(&bytes) {
            Some(mime) => FilePreview::Image {
                mime,
                data: B64.encode(&bytes),
            },
            None => FilePreview::Unsupported {
                reason: "This isn't really an image, so it isn't shown.".into(),
            },
        });
    }
    if previewable(name) {
        let mut bytes = Vec::new();
        file.take(MAX_TEXT_BYTES).read_to_end(&mut bytes)?;
        if bytes.iter().take(8192).any(|b| *b == 0) {
            return Ok(FilePreview::Unsupported {
                reason: "This file isn't text.".into(),
            });
        }
        return Ok(FilePreview::Text {
            text: clean_text(&decode(&bytes, matches!(e.as_str(), "nfo" | "diz"))),
            truncated: len > MAX_TEXT_BYTES,
        });
    }
    Ok(FilePreview::Unsupported {
        reason: "Riptor only shows images and text files.".into(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(dir: &Path, name: &str, bytes: &[u8]) -> FilePreview {
        let p = dir.join(name);
        std::fs::write(&p, bytes).unwrap();
        read(&p, name).unwrap()
    }

    #[test]
    fn the_cp437_table_is_complete() {
        assert_eq!(CP437_HIGH.chars().count(), 128);
        assert_eq!(cp437(b"\xdb\xb0 A"), "█░ A");
    }

    #[test]
    fn images_must_really_be_images() {
        let d = tempfile::tempdir().unwrap();
        let png = b"\x89PNG\r\n\x1a\n\x00\x00\x00\rIHDR";
        match write(d.path(), "a.png", png) {
            FilePreview::Image { mime, data } => {
                assert_eq!(mime, "image/png");
                assert_eq!(B64.decode(data).unwrap(), png);
            }
            other => panic!("{other:?}"),
        }
        // A JPEG with the wrong extension is still a JPEG.
        assert!(matches!(
            write(d.path(), "b.png", b"\xff\xd8\xff\xe0"),
            FilePreview::Image {
                mime: "image/jpeg",
                ..
            }
        ));
        for (name, bytes) in [
            ("evil.jpg", &b"MZ\x90\x00"[..]),
            ("page.png", b"<svg onload=alert(1)>"),
            ("x.gif", b"<html><script>"),
        ] {
            assert!(
                matches!(
                    write(d.path(), name, bytes),
                    FilePreview::Unsupported { .. }
                ),
                "{name}"
            );
        }
        assert!(matches!(
            write(d.path(), "pic.svg", b"<svg/>"),
            FilePreview::Unsupported { .. }
        ));
    }

    #[test]
    fn text_is_plain_and_capped() {
        let d = tempfile::tempdir().unwrap();
        match write(
            d.path(),
            "readme.txt",
            b"\xef\xbb\xbfhello\r\n\x1b[31mred\x07 \xe2\x80\xaeevil\tok",
        ) {
            FilePreview::Text { text, truncated } => {
                assert_eq!(text, "hello\n[31mred evil\tok");
                assert!(!truncated);
            }
            other => panic!("{other:?}"),
        }
        match write(d.path(), "art.nfo", b"\xdb\xdb release \xb1") {
            FilePreview::Text { text, .. } => assert_eq!(text, "██ release ▒"),
            other => panic!("{other:?}"),
        }
        let big = vec![b'a'; (MAX_TEXT_BYTES + 10) as usize];
        match write(d.path(), "big.log", &big) {
            FilePreview::Text { text, truncated } => {
                assert!(truncated);
                assert_eq!(text.len() as u64, MAX_TEXT_BYTES);
            }
            other => panic!("{other:?}"),
        }
        assert!(matches!(
            write(d.path(), "bin.txt", b"ab\x00cd"),
            FilePreview::Unsupported { .. }
        ));
        assert!(matches!(
            write(d.path(), "SHA256SUMS", b"abc  x"),
            FilePreview::Text { .. }
        ));
        assert!(matches!(
            write(d.path(), "setup.exe", b"MZ"),
            FilePreview::Unsupported { .. }
        ));
    }
}
