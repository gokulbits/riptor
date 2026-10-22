//! Hostile input must be refused, never panic. Seeded, so failures reproduce.

use std::net::Ipv4Addr;

use rand::{RngExt, SeedableRng};
use riptor_core::create::{create, CreateOptions, Progress};
use riptor_core::{Engine, EngineConfig, Settings, Source};

/// Runs `f` on a thread with a small stack, like a busy worker thread.
fn on_small_stack(f: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(256 * 1024)
        .spawn(f)
        .unwrap()
        .join()
        .expect("parser panicked or overflowed");
}

fn nested(depth: usize) -> Vec<u8> {
    let mut b = b"d1:x".to_vec();
    b.extend(std::iter::repeat_n(b'l', depth));
    b.extend(std::iter::repeat_n(b'e', depth));
    b.extend_from_slice(b"4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:");
    b.extend_from_slice(&[0u8; 20]);
    b.extend_from_slice(b"ee");
    b
}

#[test]
fn deep_nesting_is_refused() {
    on_small_stack(|| {
        // Legit torrents nest a few levels; this is far past the cap.
        for depth in [100, 10_000, 1_000_000] {
            let b = nested(depth);
            assert!(librqbit::torrent_from_bytes(&b).is_err(), "depth {depth}");
            assert!(librqbit_bencode::dyn_from_bytes::<Vec<u8>>(&b).is_err());
        }
        // A shallow one still parses.
        assert!(librqbit::torrent_from_bytes(&nested(3)).is_ok());
    });
}

fn sample_torrent() -> Vec<u8> {
    let d = tempfile::tempdir().unwrap();
    let src = d.path().join("files");
    std::fs::create_dir_all(src.join("sub")).unwrap();
    std::fs::write(src.join("a.bin"), vec![1u8; 50_000]).unwrap();
    std::fs::write(src.join("sub").join("b.txt"), b"hello").unwrap();
    create(
        &src,
        &CreateOptions {
            trackers: vec!["https://t.example/announce".into()],
            web_seeds: vec!["https://w.example/".into()],
            piece_length: Some(16 * 1024),
            ..Default::default()
        },
        &Progress::default(),
    )
    .unwrap()
}

fn mutate(rng: &mut rand::rngs::StdRng, base: &[u8]) -> Vec<u8> {
    let mut b = base.to_vec();
    for _ in 0..rng.random_range(1..6) {
        if b.is_empty() {
            break;
        }
        let i = rng.random_range(0..b.len());
        match rng.random_range(0..5) {
            0 => b[i] = rng.random(),
            1 => b.truncate(i),
            2 => b.insert(i, b"0123456789:ldie-"[rng.random_range(0..16)]),
            3 => {
                b.remove(i);
            }
            _ => {
                // Huge length prefixes and integers.
                let digits = b"99999999999999999999";
                b.splice(i..i, digits.iter().copied());
            }
        }
    }
    b
}

#[test]
fn mutated_torrents_never_panic() {
    let base = sample_torrent();
    on_small_stack(move || {
        let mut rng = rand::rngs::StdRng::seed_from_u64(7);
        for _ in 0..5_000 {
            let b = mutate(&mut rng, &base);
            let _ = librqbit::torrent_from_bytes(&b);
            let _ = librqbit_bencode::dyn_from_bytes::<Vec<u8>>(&b);
        }
    });
}

#[test]
fn passkey_stripping_survives_mutated_torrents() {
    let base = sample_torrent();
    on_small_stack(move || {
        let mut rng = rand::rngs::StdRng::seed_from_u64(19);
        for _ in 0..5_000 {
            let b = mutate(&mut rng, &base);
            if let Ok((out, _)) = riptor_core::passkey::strip_secret_trackers(&b) {
                // Whatever comes out must still be one well-formed dictionary.
                assert_eq!(out.first(), Some(&b'd'));
            }
        }
        let deep = nested(1_000_000);
        assert!(riptor_core::passkey::strip_secret_trackers(&deep).is_err());
    });
}

#[test]
fn checksum_files_and_file_names_from_hostile_torrents_never_panic() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(23);
    let alphabet: Vec<char> = "0123456789abcdef *()=/\\.:;#\n\r\t\u{202e}\u{feff}é 漢 SHA256 MD5"
        .chars()
        .collect();
    for _ in 0..3_000 {
        let len = rng.random_range(0..400);
        let text: String = (0..len)
            .map(|_| alphabet[rng.random_range(0..alphabet.len())])
            .collect();
        for name in ["SHA256SUMS", "x.sfv", "a.md5", "sums.txt"] {
            for e in riptor_core::checksums::parse(name, &text) {
                assert!(!e.name.split('/').any(|p| p == ".." || p.is_empty()));
            }
        }
        let files = vec![(text.clone(), len as u64), (format!("{text}.mp4.exe"), 1)];
        let _ = riptor_core::suspicion::check_names(&text, &files);
    }
}

#[test]
fn mutated_country_lists_never_panic() {
    let base = b"1.0.0.0,1.0.0.255,AU\n1.0.1.0,1.0.3.255,CN\n2001:200::,2001:200:ffff::,JP\n8.8.8.0,8.8.8.255,US\n".to_vec();
    let mut rng = rand::rngs::StdRng::seed_from_u64(29);
    for _ in 0..5_000 {
        let mut b = base.clone();
        for _ in 0..rng.random_range(1..6) {
            if b.is_empty() {
                break;
            }
            let i = rng.random_range(0..b.len());
            match rng.random_range(0..4) {
                0 => b[i] = rng.random(),
                1 => b.truncate(i),
                2 => b.insert(i, b"0123456789.:,\nAZaf"[rng.random_range(0..18)]),
                _ => {
                    b.remove(i);
                }
            }
        }
        if let Ok(db) = riptor_core::geoip::GeoDb::parse(&b[..]) {
            let _ = db.lookup("8.8.8.8".parse().unwrap());
            let _ = db.lookup("2001:200::1".parse().unwrap());
        }
    }
}

/// The whole add-dialog path, file to preview, on mutated torrents.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn previews_of_mutated_torrents_fail_cleanly() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("data");
    riptor_core::state_file::save(
        &data_dir.join("settings.json"),
        &Settings {
            download_dir: tmp.path().join("downloads"),
            dht: false,
            lsd: false,
            upnp: false,
            listen_address: Some(Ipv4Addr::LOCALHOST.into()),
            ..Settings::default()
        },
    )
    .unwrap();
    let engine = Engine::start(EngineConfig { data_dir }).await.unwrap();
    let base = sample_torrent();
    let mut rng = rand::rngs::StdRng::seed_from_u64(11);
    let file = tmp.path().join("t.torrent");
    let mut ok = 0;
    for _ in 0..300 {
        std::fs::write(&file, mutate(&mut rng, &base)).unwrap();
        if let Ok(p) = engine.preview(Source::File(file.clone())).await {
            ok += 1;
            for f in &p.files {
                assert!(!f.path.contains(".."), "{}", f.path);
            }
        }
    }
    // Most mutations break the info hash or structure; a few survive.
    assert!(ok < 300);
    std::fs::write(&file, nested(50_000)).unwrap();
    assert!(engine.preview(Source::File(file)).await.is_err());
    engine.shutdown().await.unwrap();
}

#[test]
fn backups_refuse_garbage_and_tampering() {
    let mut rng = rand::rngs::StdRng::seed_from_u64(3);
    for len in [0, 1, 8, 31, 64, 4096] {
        let mut b = vec![0u8; len];
        rng.fill(&mut b[..]);
        assert!(riptor_core::backup::open(&b, None).is_err());
        assert!(riptor_core::backup::open(&b, Some("password1")).is_err());
    }
    let payload = riptor_core::backup::BackupPayload {
        created: 1,
        app_version: "test".into(),
        settings: Settings::default(),
        categories: Default::default(),
        torrents: Vec::new(),
        statistics: Default::default(),
    };
    for pw in [None, Some("correct horse")] {
        let sealed = riptor_core::backup::seal(&payload, pw).unwrap();
        assert!(riptor_core::backup::open(&sealed, pw).is_ok());
        for _ in 0..500 {
            let m = mutate(&mut rng, &sealed);
            if m != sealed {
                assert!(riptor_core::backup::open(&m, pw).is_err());
            }
        }
    }
}

#[test]
fn mutated_resume_files_never_panic_or_leak_bad_values() {
    use riptor_core::import::{clean_label, parse_fastresume};
    use riptor_core::Limit;
    let save = std::env::temp_dir()
        .join("dl")
        .to_string_lossy()
        .into_owned();
    let s = |v: &str| format!("{}:{v}", v.len());
    let base = format!(
        "d{}{}{}{}{}l{}{}e{}i5000e{}i12345e{}i600e{}i300e{}i1700000000e{}i2500e{}i-1e{}i60ee",
        s("save_path"),
        s(&save),
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
    )
    .into_bytes();
    assert!(parse_fastresume(&base).is_some());
    let mut rng = rand::rngs::StdRng::seed_from_u64(37);
    for _ in 0..10_000 {
        let b = mutate(&mut rng, &base);
        let Some(r) = parse_fastresume(&b) else {
            continue;
        };
        assert!(r.save_path.is_absolute());
        assert!(r.tags.len() <= 32);
        for label in r.tags.iter().chain(&r.category) {
            assert_eq!(
                clean_label(label.as_bytes()).as_deref(),
                Some(label.as_str())
            );
        }
        assert!(r.downloaded <= 1 << 60 && r.uploaded <= 1 << 60);
        assert!(r.active_time <= 100 * 365 * 24 * 3600 && r.seeding_time <= 100 * 365 * 24 * 3600);
        if let Limit::Value(v) = r.limits.ratio {
            assert!(v.is_finite() && (0.0..=10_000.0).contains(&v), "{v}");
        }
    }
}

#[test]
fn mutated_statistics_files_never_panic() {
    use riptor_core::import::parse_all_time;
    let base = br"[Stats]
AllStats=@Variant(\0\0\0\x1c\0\0\0\x2\0\0\0\x12\0\x41\0l\0l\0t\0i\0m\0\x65\0\x44\0L\0\0\0\x4\0\0\x1\x1fq\xfb\x4\xcb\0\0\0\x12\0\x41\0l\0l\0t\0i\0m\0\x65\0U\0L\0\0\0\x4\0\0\0\0\0\0\0*)
"
    .to_vec();
    assert!(parse_all_time(&base).is_some());
    let mut rng = rand::rngs::StdRng::seed_from_u64(41);
    for _ in 0..20_000 {
        if let Some(t) = parse_all_time(&mutate(&mut rng, &base)) {
            assert!(t.downloaded <= 1 << 60 && t.uploaded <= 1 << 60);
        }
    }
}
