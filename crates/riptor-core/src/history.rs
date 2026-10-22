//! Transfer history only Riptor should change: the overall statistics and each torrent's totals,
//! sealed in `history.dat` (`"RIPTHIS\x01" | key kind | nonce[24] | ciphertext`, header as AAD).
//! Tamper-evident, not tamper-proof: the key lives in the OS credential store (or, without one,
//! inside Riptor), so someone who digs it out can still forge the file.

use std::collections::BTreeMap;
use std::sync::OnceLock;

use base64::Engine as _;
use chacha20poly1305::aead::rand_core::RngCore;
use chacha20poly1305::aead::{Aead, AeadCore, KeyInit, OsRng, Payload};
use chacha20poly1305::{XChaCha20Poly1305, XNonce};
use serde::{Deserialize, Serialize};
use tracing::warn;

use crate::stats::Statistics;

pub const FILE: &str = "history.dat";
const MAGIC: &[u8; 8] = b"RIPTHIS\x01";
const HEADER_LEN: usize = 8 + 1 + 24;
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
const SERVICE: &str = "Riptor";
const ACCOUNT: &str = "history-key";

// Only used where there's no credential store; distinct from the backup key.
const BUILT_IN: [u8; 32] = [
    0xa6, 0x59, 0x10, 0x8c, 0x21, 0x27, 0x18, 0x70, 0x9e, 0x00, 0x4c, 0xd9, 0x4c, 0x68, 0xca, 0x21,
    0xb4, 0x81, 0x1c, 0x38, 0xdd, 0x1c, 0x2b, 0xf2, 0xe7, 0xbc, 0xf0, 0xbd, 0xed, 0xa6, 0x5d, 0x2f,
];

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum KeyKind {
    BuiltIn = 0,
    Os = 1,
}

struct Key {
    bytes: [u8; 32],
    kind: KeyKind,
    /// The OS store had a key from an earlier run.
    existed: bool,
    /// The OS store was there but couldn't be read this time.
    unavailable: bool,
}

static KEY: OnceLock<Key> = OnceLock::new();

fn key() -> &'static Key {
    KEY.get_or_init(|| Key {
        bytes: BUILT_IN,
        kind: KeyKind::BuiltIn,
        existed: false,
        unavailable: false,
    })
}

/// Uses (creating if needed) a random key in the OS credential store. Call once, before the
/// engine starts; without a store Riptor falls back to its built-in key.
pub fn use_os_key() {
    let _ = KEY.set(load_os_key());
}

fn load_os_key() -> Key {
    let fallback = |unavailable| Key {
        bytes: BUILT_IN,
        kind: KeyKind::BuiltIn,
        existed: false,
        unavailable,
    };
    let entry = match keyring::Entry::new(SERVICE, ACCOUNT) {
        Ok(e) => e,
        Err(e) => {
            warn!("no system credential store ({e}); history uses the built-in key");
            return fallback(false);
        }
    };
    match entry.get_password() {
        Ok(stored) => match base64::engine::general_purpose::STANDARD
            .decode(stored.trim())
            .ok()
            .and_then(|b| <[u8; 32]>::try_from(b).ok())
        {
            Some(bytes) => Key {
                bytes,
                kind: KeyKind::Os,
                existed: true,
                unavailable: false,
            },
            None => {
                warn!("the history key in the credential store is malformed");
                fallback(true)
            }
        },
        Err(keyring::Error::NoEntry) => {
            let mut bytes = [0u8; 32];
            OsRng.fill_bytes(&mut bytes);
            match entry.set_password(&base64::engine::general_purpose::STANDARD.encode(bytes)) {
                Ok(()) => Key {
                    bytes,
                    kind: KeyKind::Os,
                    existed: false,
                    unavailable: false,
                },
                Err(e) => {
                    warn!("couldn't save a history key ({e}); using the built-in key");
                    fallback(false)
                }
            }
        }
        Err(e) => {
            warn!("the system credential store didn't answer ({e})");
            fallback(true)
        }
    }
}

/// An install that already sealed its history with its own key; old plain files aren't trusted.
pub fn install_has_sealed_history() -> bool {
    key().existed
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct TorrentHistory {
    pub downloaded: u64,
    pub uploaded: u64,
    pub active_time: u64,
    pub seeding_time: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    pub statistics: Statistics,
    /// By hex info hash.
    pub torrents: BTreeMap<String, TorrentHistory>,
}

#[derive(Debug, thiserror::Error)]
pub enum HistoryError {
    /// Edited, damaged, or sealed with a key this install doesn't have.
    #[error("the history file was changed outside Riptor or is damaged")]
    Rejected,
    /// Sealed with the OS key, which couldn't be read this time; leave the file alone.
    #[error("the system credential store couldn't be reached")]
    KeyUnavailable,
}

pub fn seal(h: &History) -> anyhow::Result<Vec<u8>> {
    let k = key();
    anyhow::ensure!(!k.unavailable, "history is read-only this session");
    let plain = serde_json::to_vec(h)?;
    let nonce = XChaCha20Poly1305::generate_nonce(&mut OsRng);
    let mut header = Vec::with_capacity(HEADER_LEN);
    header.extend_from_slice(MAGIC);
    header.push(k.kind as u8);
    header.extend_from_slice(&nonce);
    let sealed = XChaCha20Poly1305::new(&k.bytes.into())
        .encrypt(
            &nonce,
            Payload {
                msg: &plain,
                aad: &header,
            },
        )
        .map_err(|_| anyhow::anyhow!("sealing the history failed"))?;
    header.extend_from_slice(&sealed);
    Ok(header)
}

pub fn open(bytes: &[u8]) -> Result<History, HistoryError> {
    if bytes.len() < HEADER_LEN || bytes.len() as u64 > MAX_FILE_BYTES || &bytes[..8] != MAGIC {
        return Err(HistoryError::Rejected);
    }
    let k = key();
    let kind = bytes[8];
    if kind == KeyKind::Os as u8 && k.unavailable {
        return Err(HistoryError::KeyUnavailable);
    }
    if kind != k.kind as u8 {
        return Err(HistoryError::Rejected);
    }
    let (header, sealed) = bytes.split_at(HEADER_LEN);
    let plain = XChaCha20Poly1305::new(&k.bytes.into())
        .decrypt(
            XNonce::from_slice(&header[9..]),
            Payload {
                msg: sealed,
                aad: header,
            },
        )
        .map_err(|_| HistoryError::Rejected)?;
    let mut h: History = serde_json::from_slice(&plain).map_err(|_| HistoryError::Rejected)?;
    h.statistics.sanitize();
    Ok(h)
}

/// The session can't save history (its key couldn't be read), so saves leave the file alone.
pub fn read_only() -> bool {
    key().unavailable
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> History {
        let mut h = History::default();
        h.statistics.all_time_upload = 123_456;
        h.statistics.all_time_download = 7;
        h.torrents.insert(
            "aa".repeat(20),
            TorrentHistory {
                downloaded: 1,
                uploaded: 2,
                active_time: 3,
                seeding_time: 4,
            },
        );
        h
    }

    #[test]
    fn round_trips_and_hides_the_numbers() {
        let sealed = seal(&sample()).unwrap();
        assert_eq!(open(&sealed).unwrap(), sample());
        // Nothing readable to edit.
        let text = String::from_utf8_lossy(&sealed);
        assert!(!text.contains("123456") && !text.contains("all_time"));
        // A fresh nonce each time.
        assert_ne!(seal(&sample()).unwrap(), sealed);
    }

    #[test]
    fn any_change_is_rejected() {
        let sealed = seal(&sample()).unwrap();
        for i in 0..sealed.len() {
            let mut b = sealed.clone();
            b[i] ^= 0x01;
            assert!(open(&b).is_err(), "byte {i}");
        }
        assert!(open(&sealed[..sealed.len() - 1]).is_err());
        assert!(open(b"").is_err());
        assert!(open(br#"{"statistics":{"all_time_upload":999}}"#).is_err());
    }
}
