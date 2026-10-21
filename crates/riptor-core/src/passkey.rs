//! Spots personal tracker passkeys so they aren't shared by accident.

use anyhow::{bail, ensure, Context};

const SECRET_PARAMS: &[&str] = &[
    "passkey",
    "authkey",
    "torrent_pass",
    "torrentpass",
    "pk",
    "key",
    "uid",
    "auth",
    "token",
    "apikey",
    "api_key",
    "secure",
    "pass",
    "rsskey",
];

/// A tracker URL that carries a personal key (in the query, the path or as a login).
pub fn has_secret(tracker: &str) -> bool {
    let Ok(u) = url::Url::parse(tracker) else {
        // Can't tell, so don't share it.
        return true;
    };
    if !u.username().is_empty() || u.password().is_some() {
        return true;
    }
    if u.query_pairs()
        .any(|(k, _)| SECRET_PARAMS.contains(&k.to_ascii_lowercase().as_str()))
    {
        return true;
    }
    u.path_segments().is_some_and(|mut segs| {
        segs.any(|s| {
            s.len() >= 16
                && s.chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        })
    })
}

const MAX_DEPTH: usize = 64;

/// Where the bencoded value starting at `i` ends.
fn skip(b: &[u8], i: usize, depth: usize) -> anyhow::Result<usize> {
    ensure!(depth <= MAX_DEPTH, "nested too deeply");
    match b.get(i).context("truncated")? {
        b'i' => {
            let end = b[i..]
                .iter()
                .position(|c| *c == b'e')
                .context("truncated")?;
            Ok(i + end + 1)
        }
        b'l' | b'd' => {
            let mut j = i + 1;
            while *b.get(j).context("truncated")? != b'e' {
                j = skip(b, j, depth + 1)?;
            }
            Ok(j + 1)
        }
        b'0'..=b'9' => {
            let (start, len) = string_at(b, i)?;
            Ok(start + len)
        }
        _ => bail!("not bencode"),
    }
}

/// (start of the bytes, length) of the string at `i`.
fn string_at(b: &[u8], i: usize) -> anyhow::Result<(usize, usize)> {
    let colon = b[i..]
        .iter()
        .take(21)
        .position(|c| *c == b':')
        .context("bad string length")?;
    let digits = std::str::from_utf8(&b[i..i + colon])?;
    ensure!(
        !digits.is_empty() && digits.bytes().all(|c| c.is_ascii_digit()),
        "bad string length"
    );
    let len: usize = digits.parse()?;
    let start = i + colon + 1;
    ensure!(
        start.checked_add(len).is_some_and(|e| e <= b.len()),
        "truncated"
    );
    Ok((start, len))
}

fn string(b: &[u8], i: usize) -> anyhow::Result<&[u8]> {
    let (s, n) = string_at(b, i)?;
    Ok(&b[s..s + n])
}

fn put_str(out: &mut Vec<u8>, s: &[u8]) {
    out.extend_from_slice(s.len().to_string().as_bytes());
    out.push(b':');
    out.extend_from_slice(s);
}

/// The `announce-list` tiers, as strings.
fn tiers(b: &[u8], i: usize) -> anyhow::Result<Vec<Vec<Vec<u8>>>> {
    ensure!(b.get(i) == Some(&b'l'), "announce-list is not a list");
    let mut out = Vec::new();
    let mut j = i + 1;
    while *b.get(j).context("truncated")? != b'e' {
        let end = skip(b, j, 1)?;
        let mut tier = Vec::new();
        if b[j] == b'l' {
            let mut k = j + 1;
            while b[k] != b'e' {
                let next = skip(b, k, 2)?;
                if b[k].is_ascii_digit() {
                    tier.push(string(b, k)?.to_vec());
                }
                k = next;
            }
        }
        out.push(tier);
        j = end;
    }
    Ok(out)
}

/// The .torrent without passkey trackers (info dict untouched), and how many were dropped.
pub fn strip_secret_trackers(torrent: &[u8]) -> anyhow::Result<(Vec<u8>, usize)> {
    ensure!(torrent.first() == Some(&b'd'), "not a torrent file");
    let mut entries: Vec<(&[u8], &[u8])> = Vec::new();
    let mut j = 1;
    while *torrent.get(j).context("truncated")? != b'e' {
        let key = string(torrent, j)?;
        let vstart = skip(torrent, j, 1)?;
        let vend = skip(torrent, vstart, 1)?;
        entries.push((key, &torrent[vstart..vend]));
        j = vend;
    }
    ensure!(j + 1 == torrent.len(), "trailing data after the torrent");

    let secret = |t: &[u8]| std::str::from_utf8(t).map_or(true, has_secret);
    let mut removed = 0;
    let mut kept_tiers = Vec::new();
    if let Some((_, v)) = entries.iter().find(|(k, _)| *k == b"announce-list") {
        for tier in tiers(v, 0)? {
            let before = tier.len();
            let tier: Vec<Vec<u8>> = tier.into_iter().filter(|t| !secret(t)).collect();
            removed += before - tier.len();
            if !tier.is_empty() {
                kept_tiers.push(tier);
            }
        }
    }
    let mut announce = None;
    if let Some((_, v)) = entries.iter().find(|(k, _)| *k == b"announce") {
        let a = string(v, 0)?;
        if secret(a) {
            // Counted once if the list repeats it.
            if !entries.iter().any(|(k, _)| *k == b"announce-list") {
                removed += 1;
            }
            announce = kept_tiers.first().and_then(|t| t.first()).cloned();
        } else {
            announce = Some(a.to_vec());
        }
    }

    let mut out = Vec::with_capacity(torrent.len());
    out.push(b'd');
    for (k, v) in &entries {
        match *k {
            b"announce" => {
                if let Some(a) = &announce {
                    put_str(&mut out, k);
                    put_str(&mut out, a);
                }
            }
            b"announce-list" => {
                if !kept_tiers.is_empty() {
                    put_str(&mut out, k);
                    out.push(b'l');
                    for tier in &kept_tiers {
                        out.push(b'l');
                        for t in tier {
                            put_str(&mut out, t);
                        }
                        out.push(b'e');
                    }
                    out.push(b'e');
                }
            }
            _ => {
                put_str(&mut out, k);
                out.extend_from_slice(v);
            }
        }
    }
    out.push(b'e');
    Ok((out, removed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passkeys_are_spotted() {
        for secret in [
            "https://tracker.example/announce?passkey=0123abcd",
            "http://tracker.example/announce.php?PassKey=x&info=1",
            "http://tracker.example:2710/0123456789abcdef0123456789abcdef/announce",
            "udp://tracker.example:80/a1B2c3D4e5F6g7H8/announce",
            "https://user:pw@tracker.example/announce",
            "https://tracker.example/announce?torrent_pass=abc",
            "not a url",
        ] {
            assert!(has_secret(secret), "{secret}");
        }
        for public in [
            "udp://tracker.opentrackr.example:1337/announce",
            "https://tracker.example/announce",
            "http://tracker.example/announce.php?info_hash=x",
            "udp://open.example:6969",
        ] {
            assert!(!has_secret(public), "{public}");
        }
    }

    fn torrent(announce: &str, list: &[&[&str]]) -> Vec<u8> {
        let mut t = Vec::new();
        t.push(b'd');
        put_str(&mut t, b"announce");
        put_str(&mut t, announce.as_bytes());
        put_str(&mut t, b"announce-list");
        t.push(b'l');
        for tier in list {
            t.push(b'l');
            for u in *tier {
                put_str(&mut t, u.as_bytes());
            }
            t.push(b'e');
        }
        t.push(b'e');
        put_str(&mut t, b"comment");
        put_str(&mut t, b"hi");
        put_str(&mut t, b"info");
        t.extend_from_slice(
            b"d6:lengthi5e4:name1:x12:piece lengthi16384e6:pieces20:aaaaaaaaaaaaaaaaaaaae",
        );
        t.push(b'e');
        t
    }

    fn info_hash(t: &[u8]) -> librqbit_core::hash_id::Id20 {
        librqbit_core::torrent_metainfo::torrent_from_bytes(t)
            .unwrap()
            .info_hash
    }

    #[test]
    fn stripping_keeps_the_info_hash_and_public_trackers() {
        let private = "https://t.example/announce?passkey=deadbeef";
        let public = "udp://open.example:6969/announce";
        let t = torrent(
            private,
            &[
                &[private],
                &[public, "https://t2.example/0123456789abcdef0123/announce"],
            ],
        );
        let (out, removed) = strip_secret_trackers(&t).unwrap();
        assert_eq!(removed, 2);
        assert_eq!(info_hash(&out), info_hash(&t));
        let text = String::from_utf8_lossy(&out);
        assert!(!text.contains("passkey") && !text.contains("0123456789abcdef0123"));
        let parsed = librqbit_core::torrent_metainfo::torrent_from_bytes(&out).unwrap();
        let announces: Vec<String> = parsed
            .iter_announce()
            .map(|a| String::from_utf8_lossy(a.as_ref()).into_owned())
            .collect();
        assert_eq!(announces, [public]);
        assert!(text.contains("7:comment2:hi"));

        // Nothing secret: same bytes back.
        let clean = torrent(public, &[&[public]]);
        assert_eq!(strip_secret_trackers(&clean).unwrap(), (clean.clone(), 0));

        // Only a private tracker: no trackers at all.
        let only = torrent(private, &[&[private]]);
        let (out, removed) = strip_secret_trackers(&only).unwrap();
        assert_eq!(removed, 1);
        assert!(!String::from_utf8_lossy(&out).contains("announce"));
        assert_eq!(info_hash(&out), info_hash(&only));
    }

    #[test]
    fn hostile_torrents_are_refused_not_crashed() {
        let deep = format!("d4:info{}{}e", "l".repeat(10_000), "e".repeat(10_000));
        for bad in [
            &b""[..],
            b"d",
            b"le",
            b"d8:announce",
            b"d8:announce99999999999999999999:xe",
            b"d8:announcei5ee",
            b"d13:announce-list5:helloe",
            b"d4:infodeexx",
            deep.as_bytes(),
        ] {
            assert!(
                strip_secret_trackers(bad).is_err(),
                "{:?}",
                String::from_utf8_lossy(bad)
            );
        }
    }
}
