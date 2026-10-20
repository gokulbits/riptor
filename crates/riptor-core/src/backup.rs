//! `.riptorbackup`: `"RIPTRBK\0" | version | mode | salt[16] | nonce[24] | ciphertext`.
//! XChaCha20-Poly1305 over deflated JSON, header as AAD. Mode 0 uses a key
//! built into the app (tamper-evident, not forgery-proof); mode 1 is Argon2id.

use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::path::PathBuf;

use anyhow::{bail, ensure, Context};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};

use crate::model::Record;
use crate::settings::Settings;
use crate::stats::Statistics;

pub const EXTENSION: &str = "riptorbackup";
const MAGIC: &[u8; 8] = b"RIPTRBK\0";
const VERSION: u8 = 1;
const HEADER_LEN: usize = 8 + 1 + 1 + 16 + 24;
pub const MAX_FILE_BYTES: u64 = 256 * 1024 * 1024;
const MAX_PLAINTEXT_BYTES: u64 = 512 * 1024 * 1024;
pub const MIN_PASSWORD_LEN: usize = 8;

const APP_KEY: [u8; 32] = [
    0x05, 0x5d, 0x84, 0xcf, 0x72, 0x93, 0x56, 0x27, 0xda, 0xa5, 0x62, 0xe0, 0x19, 0x6d, 0x27, 0xf9,
    0xcf, 0x58, 0xe7, 0xcb, 0x80, 0xa2, 0x44, 0xb3, 0x59, 0xdf, 0x44, 0xe2, 0x8c, 0xe5, 0x84, 0xa8,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    AppKey = 0,
    Password = 1,
}

#[derive(Debug, thiserror::Error)]
pub enum BackupError {
    #[error("password_required")]
    PasswordRequired,
    #[error("The password is wrong, or the backup file has been modified or damaged.")]
    WrongPasswordOrTampered,
    #[error("This file is not a Riptor backup.")]
    NotABackup,
    #[error("This backup was made by a newer version of Riptor.")]
    TooNew,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupTorrent {
    pub record: Record,
    /// The `.torrent` file, base64.
    pub torrent: String,
    #[serde(default)]
    pub only_files: Option<Vec<usize>>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupPayload {
    pub created: u64,
    pub app_version: String,
    pub settings: Settings,
    #[serde(default)]
    pub categories: BTreeMap<String, Option<PathBuf>>,
    #[serde(default)]
    pub torrents: Vec<BackupTorrent>,
    #[serde(default)]
    pub statistics: Statistics,
}

fn derive_password_key(password: &str, salt: &[u8; 16]) -> anyhow::Result<[u8; 32]> {
    // OWASP baseline.
    let params = Params::new(64 * 1024, 3, 1, Some(32)).map_err(|e| anyhow::anyhow!("{e}"))?;
    let mut key = [0u8; 32];
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password.as_bytes(), salt, &mut key)
        .map_err(|e| anyhow::anyhow!("key derivation failed: {e}"))?;
    Ok(key)
}

pub fn seal(payload: &BackupPayload, password: Option<&str>) -> anyhow::Result<Vec<u8>> {
    let mode = if password.is_some() {
        Mode::Password
    } else {
        Mode::AppKey
    };
    if let Some(p) = password {
        ensure!(
            p.chars().count() >= MIN_PASSWORD_LEN,
            "use a password of at least {MIN_PASSWORD_LEN} characters"
        );
    }
    let mut salt = [0u8; 16];
    OsRng.fill_bytes(&mut salt);
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let key = match password {
        Some(p) => derive_password_key(p, &salt)?,
        None => APP_KEY,
    };

    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.push(VERSION);
    header.push(mode as u8);
    header.extend_from_slice(&salt);
    header.extend_from_slice(&nonce);
    debug_assert_eq!(header.len(), HEADER_LEN);

    let json = serde_json::to_vec(payload)?;
    let mut z = flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
    z.write_all(&json)?;
    let compressed = z.finish()?;

    let cipher = XChaCha20Poly1305::new(&key.into());
    let ct = cipher
        .encrypt(
            &nonce,
            Payload {
                msg: &compressed,
                aad: &header,
            },
        )
        .map_err(|_| anyhow::anyhow!("encryption failed"))?;
    let mut out = header;
    out.extend_from_slice(&ct);
    Ok(out)
}

pub fn is_password_protected(bytes: &[u8]) -> anyhow::Result<bool> {
    Ok(parse_header(bytes)? == Mode::Password)
}

fn parse_header(bytes: &[u8]) -> anyhow::Result<Mode> {
    if bytes.len() < HEADER_LEN + 16 || &bytes[..8] != MAGIC {
        bail!(BackupError::NotABackup);
    }
    if bytes[8] > VERSION {
        bail!(BackupError::TooNew);
    }
    match bytes[9] {
        0 => Ok(Mode::AppKey),
        1 => Ok(Mode::Password),
        _ => bail!(BackupError::NotABackup),
    }
}

pub fn open(bytes: &[u8], password: Option<&str>) -> anyhow::Result<BackupPayload> {
    ensure!(
        bytes.len() as u64 <= MAX_FILE_BYTES,
        "backup file is too large"
    );
    let mode = parse_header(bytes)?;
    let (header, ct) = bytes.split_at(HEADER_LEN);
    let salt: [u8; 16] = header[10..26].try_into().expect("fixed size");
    let nonce = XNonce::from_slice(&header[26..50]);
    let key = match mode {
        Mode::AppKey => APP_KEY,
        Mode::Password => match password {
            Some(p) if !p.is_empty() => derive_password_key(p, &salt)?,
            _ => bail!(BackupError::PasswordRequired),
        },
    };
    let compressed = XChaCha20Poly1305::new(&key.into())
        .decrypt(
            nonce,
            Payload {
                msg: ct,
                aad: header,
            },
        )
        .map_err(|_| BackupError::WrongPasswordOrTampered)?;

    // Bounded, so a hostile file can't balloon memory.
    let mut json = Vec::new();
    flate2::read::DeflateDecoder::new(&compressed[..])
        .take(MAX_PLAINTEXT_BYTES + 1)
        .read_to_end(&mut json)
        .context("backup contents are damaged")?;
    ensure!(
        json.len() as u64 <= MAX_PLAINTEXT_BYTES,
        "backup contents are too large"
    );
    serde_json::from_slice(&json).context("backup contents are damaged")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn payload() -> BackupPayload {
        BackupPayload {
            created: 1,
            app_version: "0.1.0".into(),
            settings: Settings::default(),
            categories: BTreeMap::from([("linux".into(), None)]),
            torrents: vec![],
            statistics: Statistics {
                all_time_upload: 42,
                ..Default::default()
            },
        }
    }

    #[test]
    fn roundtrip_without_password() {
        let b = seal(&payload(), None).unwrap();
        assert!(!is_password_protected(&b).unwrap());
        let p = open(&b, None).unwrap();
        assert_eq!(p.statistics.all_time_upload, 42);
        assert!(p.categories.contains_key("linux"));
        assert!(!String::from_utf8_lossy(&b).contains("linux"));
    }

    #[test]
    fn any_modified_byte_is_detected() {
        let b = seal(&payload(), None).unwrap();
        let positions: Vec<usize> = (8..HEADER_LEN)
            .chain((HEADER_LEN..b.len()).step_by(7))
            .collect();
        for i in positions {
            let mut t = b.clone();
            t[i] ^= 0x01;
            assert!(open(&t, None).is_err(), "flip at byte {i} went unnoticed");
        }
        assert!(open(&b[..b.len() - 1], None).is_err());
        let mut longer = b.clone();
        longer.push(0);
        assert!(open(&longer, None).is_err());
    }

    #[test]
    fn password_mode() {
        let b = seal(&payload(), Some("correct horse")).unwrap();
        assert!(is_password_protected(&b).unwrap());
        let e = open(&b, None).unwrap_err();
        assert!(matches!(
            e.downcast_ref::<BackupError>(),
            Some(BackupError::PasswordRequired)
        ));
        let e = open(&b, Some("wrong password")).unwrap_err();
        assert!(matches!(
            e.downcast_ref::<BackupError>(),
            Some(BackupError::WrongPasswordOrTampered)
        ));
        assert_eq!(
            open(&b, Some("correct horse"))
                .unwrap()
                .statistics
                .all_time_upload,
            42
        );
        // Switching the mode byte to "app key" must not bypass the password.
        let mut t = b.clone();
        t[9] = 0;
        assert!(open(&t, None).is_err());
        assert!(seal(&payload(), Some("short")).is_err());
    }

    #[test]
    fn rejects_non_backups() {
        assert!(open(b"hello world", None).is_err());
        assert!(open(&[0u8; 200], None).is_err());
        let mut b = seal(&payload(), None).unwrap();
        b[8] = VERSION + 1;
        let e = open(&b, None).unwrap_err();
        assert!(matches!(
            e.downcast_ref::<BackupError>(),
            Some(BackupError::TooNew)
        ));
    }
}
