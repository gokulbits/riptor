//! Content checks run by a real engine on loopback-only settings.

use std::net::Ipv4Addr;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use riptor_core::{Engine, EngineConfig, Event, Settings, TorrentSummary};

fn quiet_settings(downloads: &Path) -> Settings {
    Settings {
        download_dir: downloads.to_path_buf(),
        dht: false,
        lsd: false,
        upnp: false,
        listen_address: Some(Ipv4Addr::LOCALHOST.into()),
        ..Settings::default()
    }
}

async fn engine(data_dir: &Path, settings: Settings) -> Arc<Engine> {
    riptor_core::state_file::save(&data_dir.join("settings.json"), &settings).unwrap();
    Engine::start(EngineConfig {
        data_dir: data_dir.to_path_buf(),
    })
    .await
    .unwrap()
}

async fn wait_for(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    id: &str,
    what: &str,
    pred: impl Fn(&TorrentSummary) -> bool,
) -> TorrentSummary {
    tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match events.recv().await {
                Ok(Event::Tick { torrents, .. }) => {
                    if let Some(t) = torrents.iter().find(|t| t.id == id) {
                        if pred(t) {
                            return t.clone();
                        }
                    }
                }
                Ok(_) | Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("{e}"),
            }
        }
    })
    .await
    .unwrap_or_else(|_| panic!("timed out waiting for: {what}"))
}

/// Creates a torrent of `content` and seeds it from there.
async fn seed_in_place(engine: &Engine, content: &Path, out: &Path) -> String {
    engine
        .create_torrent(
            content.to_path_buf(),
            out.to_path_buf(),
            Default::default(),
            true,
        )
        .await
        .unwrap()
        .expect("added")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn finished_torrents_are_checked_against_their_checksum_files() {
    let tmp = tempfile::tempdir().unwrap();
    let content = tmp.path().join("shared").join("Release");
    std::fs::create_dir_all(&content).unwrap();
    std::fs::write(content.join("good.iso"), vec![7u8; 300_000]).unwrap();
    std::fs::write(content.join("bad.iso"), vec![9u8; 200_000]).unwrap();
    let hex = |b: &[u8]| -> String {
        use sha2::Digest;
        sha2::Sha256::digest(b)
            .iter()
            .map(|x| format!("{x:02x}"))
            .collect()
    };
    std::fs::write(
        content.join("SHA256SUMS"),
        format!(
            "{}  good.iso\n{}  bad.iso\n{}  ../../outside.iso\n",
            hex(&[7u8; 300_000]),
            hex(b"what the publisher shipped"),
            hex(b"x"),
        ),
    )
    .unwrap();
    let engine = engine(
        &tmp.path().join("data"),
        quiet_settings(&tmp.path().join("downloads")),
    )
    .await;
    let mut events = engine.subscribe();
    let id = seed_in_place(&engine, &content, &tmp.path().join("Release.torrent")).await;
    let t = wait_for(&mut events, &id, "the checksum check", |t| {
        t.checksum.is_some()
    })
    .await;
    assert_eq!(t.checksum, Some(false));
    let report = engine.safety(&id).unwrap().checksums.unwrap();
    assert_eq!(report.verified, 1);
    assert_eq!(report.mismatched, ["bad.iso"]);
    // Names that leave the torrent's folder are never looked up.
    assert_eq!(report.not_found, 0);

    // Fixing the file and asking again clears the warning.
    std::fs::write(
        content.join("SHA256SUMS"),
        format!("{}  good.iso\n", hex(&[7u8; 300_000])),
    )
    .unwrap();
    engine.verify_checksums(std::slice::from_ref(&id)).unwrap();
    // The manifest changed on disk, so its piece no longer verifies; only the report matters here.
    let t = wait_for(&mut events, &id, "a clean re-check", |t| {
        t.checksum == Some(true)
    })
    .await;
    assert_eq!(t.checksum, Some(true));
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn passkeys_stay_out_of_shared_links_and_files() {
    let tmp = tempfile::tempdir().unwrap();
    let content = tmp.path().join("shared").join("Album");
    std::fs::create_dir_all(&content).unwrap();
    std::fs::write(content.join("track.flac"), vec![1u8; 100_000]).unwrap();
    let engine = engine(
        &tmp.path().join("data"),
        quiet_settings(&tmp.path().join("downloads")),
    )
    .await;
    let secret = "https://tracker.example/announce?passkey=0123456789abcdef";
    let public = "udp://open.example:6969/announce";
    let out = tmp.path().join("Album.torrent");
    let id = engine
        .create_torrent(
            content.clone(),
            out.clone(),
            riptor_core::create::CreateOptions {
                trackers: vec![secret.into(), public.into()],
                ..Default::default()
            },
            true,
        )
        .await
        .unwrap()
        .unwrap();

    let m = engine.magnet_link(&id).unwrap();
    assert_eq!(m.withheld, 1);
    assert!(!m.link.contains("passkey") && m.link.contains("open.example"));
    assert_eq!(engine.safety(&id).unwrap().secret_trackers, 1);

    let kept = tmp.path().join("kept.torrent");
    let clean = tmp.path().join("clean.torrent");
    engine.export_torrent(&id, &kept, false).unwrap();
    engine.export_torrent(&id, &clean, true).unwrap();
    let kept = std::fs::read(kept).unwrap();
    let clean = std::fs::read(clean).unwrap();
    assert!(String::from_utf8_lossy(&kept).contains("passkey"));
    assert!(!String::from_utf8_lossy(&clean).contains("passkey"));
    let hash = |b: &[u8]| {
        librqbit_core::torrent_metainfo::torrent_from_bytes(b)
            .unwrap()
            .info_hash
            .as_string()
    };
    assert_eq!(hash(&clean), id);
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn fake_releases_are_added_stopped_and_disguised_files_flagged() {
    let tmp = tempfile::tempdir().unwrap();
    let downloads = tmp.path().join("downloads");
    let engine = engine(&tmp.path().join("data"), quiet_settings(&downloads)).await;
    let mut events = engine.subscribe();

    // A "film" that ships a codec installer.
    let film = tmp.path().join("made").join("Film.2024.1080p");
    std::fs::create_dir_all(&film).unwrap();
    std::fs::write(film.join("Film.mkv"), vec![0x1au8; 2_000_000]).unwrap();
    std::fs::write(film.join("Codec Setup.exe"), b"MZ fake installer").unwrap();
    let out = tmp.path().join("Film.torrent");
    engine
        .create_torrent(film.clone(), out.clone(), Default::default(), false)
        .await
        .unwrap();
    let preview = engine
        .preview(riptor_core::Source::File(out))
        .await
        .unwrap();
    assert_eq!(
        riptor_core::suspicion::worst(&preview.findings),
        Some(riptor_core::Level::Danger)
    );
    let id = engine
        .add(riptor_core::AddParams {
            info_hash: preview.info_hash,
            save_path: Some(downloads.clone()),
            ..Default::default()
        })
        .await
        .unwrap();
    let t = wait_for(&mut events, &id, "the fake release to show up", |t| {
        t.suspicion.is_some()
    })
    .await;
    assert_eq!(t.state, riptor_core::TorrentState::Stopped);
    assert_eq!(t.suspicion, Some(riptor_core::Level::Danger));

    // An "mp3" that is really a program, found once it's complete.
    let album = tmp.path().join("shared").join("Album");
    std::fs::create_dir_all(&album).unwrap();
    let mut mz = b"MZ\x90\x00".to_vec();
    mz.resize(500_000, 0);
    std::fs::write(album.join("Song.mp3"), mz).unwrap();
    let id = seed_in_place(&engine, &album, &tmp.path().join("Album.torrent")).await;
    let t = wait_for(&mut events, &id, "the content scan", |t| {
        t.suspicion.is_some()
    })
    .await;
    assert_eq!(t.suspicion, Some(riptor_core::Level::Danger));
    let safety = engine.safety(&id).unwrap();
    assert_eq!(safety.findings[0].file.as_deref(), Some("Song.mp3"));
    assert!(engine.flagged_dangerous(&id, 0));
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_used_up_data_cap_holds_torrents() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let mut stats = riptor_core::Statistics::default();
    stats.record_transfer(3 << 30, 0, riptor_core::stats::unix_now());
    riptor_core::state_file::save(&data.join("statistics.json"), &stats).unwrap();
    let mut settings = quiet_settings(&tmp.path().join("downloads"));
    settings.data_cap = Some(riptor_core::settings::DataCap {
        gib: 2,
        counts: riptor_core::settings::CapCounts::Both,
        reset_day: 1,
    });
    let engine = engine(&data, settings).await;
    let mut events = engine.subscribe();
    let content = tmp.path().join("shared").join("Big");
    std::fs::create_dir_all(&content).unwrap();
    std::fs::write(content.join("a.bin"), vec![1u8; 50_000]).unwrap();
    let id = seed_in_place(&engine, &content, &tmp.path().join("Big.torrent")).await;
    let global = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(Event::Tick { global, torrents }) = events.recv().await {
                let t = torrents.iter().find(|t| t.id == id);
                if global.hold.is_some()
                    && t.is_some_and(|t| t.state == riptor_core::TorrentState::Queued)
                {
                    return global;
                }
            }
        }
    })
    .await
    .expect("the torrent should wait for the cap");
    let cap = global.data_cap.unwrap();
    assert!(cap.used >= cap.limit && cap.limit == 2 << 30);
    assert!(global.hold.unwrap().contains("data cap"));

    // Raising the cap lets it run.
    let mut s = engine.settings();
    s.data_cap.as_mut().unwrap().gib = 100;
    engine.update_settings(s).await.unwrap();
    wait_for(&mut events, &id, "seeding after the cap is raised", |t| {
        matches!(
            t.state,
            riptor_core::TorrentState::Seeding | riptor_core::TorrentState::StalledUp
        )
    })
    .await;
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_data_check_finds_damage_and_rechecks() {
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine(
        &tmp.path().join("data"),
        quiet_settings(&tmp.path().join("downloads")),
    )
    .await;
    let mut events = engine.subscribe();
    let content = tmp.path().join("shared").join("Archive");
    std::fs::create_dir_all(&content).unwrap();
    let file = content.join("photos.bin");
    std::fs::write(&file, vec![5u8; 600_000]).unwrap();
    let id = seed_in_place(&engine, &content, &tmp.path().join("Archive.torrent")).await;
    wait_for(&mut events, &id, "seeding", |t| {
        t.progress >= 1.0 && t.completed_on.is_some()
    })
    .await;

    // An intact torrent passes and records when.
    engine.check_data(std::slice::from_ref(&id)).unwrap();
    wait_for(&mut events, &id, "a clean check", |t| {
        t.last_scrub.is_some()
    })
    .await;

    // Bit rot behind Riptor's back.
    let mut bytes = std::fs::read(&file).unwrap();
    bytes[300_000] ^= 0x40;
    std::fs::write(&file, bytes).unwrap();
    engine.check_data(std::slice::from_ref(&id)).unwrap();
    let warned = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(Event::Notice { message, .. }) = events.recv().await {
                if message.contains("damaged") {
                    return message;
                }
            }
        }
    })
    .await
    .expect("a damage warning");
    assert!(warned.contains("1 piece"));
    // The recheck drops the bad piece, so it's no longer complete.
    wait_for(&mut events, &id, "the recheck", |t| {
        t.progress < 1.0 && t.state != riptor_core::TorrentState::Checking
    })
    .await;
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn untrusted_networks_hold_torrents_until_trusted() {
    use riptor_core::NetworkMode;
    let tmp = tempfile::tempdir().unwrap();
    let mut settings = quiet_settings(&tmp.path().join("downloads"));
    settings.require_vpn_untrusted = true;
    let engine = engine(&tmp.path().join("data"), settings).await;
    // Until the OS says where we are, the network counts as untrusted.
    assert_eq!(engine.network(), NetworkMode::Blocked);

    let here = riptor_core::netinfo::current().names;
    if !here.is_empty() {
        let mut s = engine.settings();
        s.trusted_networks = here.clone();
        engine.update_settings(s).await.unwrap();
        tokio::time::timeout(Duration::from_secs(30), async {
            while engine.network() != NetworkMode::Normal {
                tokio::time::sleep(Duration::from_millis(200)).await;
            }
        })
        .await
        .expect("a trusted network lifts the hold");
        assert!(!engine.kill_switch_on());
    }

    // Untrusting it again holds traffic.
    let mut s = engine.settings();
    s.trusted_networks = vec!["Somewhere else".into()];
    engine.update_settings(s).await.unwrap();
    assert_eq!(engine.network(), NetworkMode::Blocked);
    // Turning the rule off frees it.
    let mut s = engine.settings();
    s.require_vpn_untrusted = false;
    engine.update_settings(s).await.unwrap();
    assert_eq!(engine.network(), NetworkMode::Normal);
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn finished_images_and_text_preview_in_place() {
    use riptor_core::preview::FilePreview;
    let tmp = tempfile::tempdir().unwrap();
    let engine = engine(
        &tmp.path().join("data"),
        quiet_settings(&tmp.path().join("downloads")),
    )
    .await;
    let mut events = engine.subscribe();
    let content = tmp.path().join("shared").join("Docs");
    std::fs::create_dir_all(&content).unwrap();
    let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
    png.resize(5_000, 7);
    std::fs::write(content.join("cover.png"), &png).unwrap();
    std::fs::write(content.join("notes.txt"), b"read me").unwrap();
    std::fs::write(content.join("fake.jpg"), b"MZ not a picture").unwrap();
    let id = seed_in_place(&engine, &content, &tmp.path().join("Docs.torrent")).await;
    wait_for(&mut events, &id, "seeding", |t| {
        t.progress >= 1.0 && t.completed_on.is_some()
    })
    .await;
    let files = engine.files(&id).unwrap();
    let index = |name: &str| files.iter().find(|f| f.path.ends_with(name)).unwrap().index;
    assert!(files.iter().all(|f| f.previewable));
    assert!(matches!(
        engine.preview_file(&id, index("cover.png")).await.unwrap(),
        FilePreview::Image {
            mime: "image/png",
            ..
        }
    ));
    assert_eq!(
        engine.preview_file(&id, index("notes.txt")).await.unwrap(),
        FilePreview::Text {
            text: "read me".into(),
            truncated: false
        }
    );
    assert!(matches!(
        engine.preview_file(&id, index("fake.jpg")).await,
        Err(_) | Ok(FilePreview::Unsupported { .. })
    ));
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn files_another_torrent_has_are_reused() {
    let tmp = tempfile::tempdir().unwrap();
    let downloads = tmp.path().join("downloads");
    let engine = engine(&tmp.path().join("data"), quiet_settings(&downloads)).await;
    let mut events = engine.subscribe();
    let big: Vec<u8> = (0..2 * 1024 * 1024u32)
        .map(|i| (i * 7 + i / 3) as u8)
        .collect();

    // Already seeding: a folder with the big file.
    let first = tmp.path().join("shared").join("Collection");
    std::fs::create_dir_all(&first).unwrap();
    std::fs::write(first.join("movie.bin"), &big).unwrap();
    let a = seed_in_place(&engine, &first, &tmp.path().join("Collection.torrent")).await;
    wait_for(&mut events, &a, "the first torrent seeding", |t| {
        t.completed_on.is_some()
    })
    .await;

    // Another torrent of the same file plus a note, made elsewhere.
    let other = tmp.path().join("made").join("Release");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("movie.bin"), &big).unwrap();
    std::fs::write(other.join("readme.txt"), b"hello").unwrap();
    let out = tmp.path().join("Release.torrent");
    engine
        .create_torrent(other.clone(), out.clone(), Default::default(), false)
        .await
        .unwrap();
    std::fs::remove_dir_all(&other).unwrap();

    let preview = engine
        .preview(riptor_core::Source::File(out))
        .await
        .unwrap();
    assert_eq!(preview.duplicates.len(), 1);
    assert_eq!(preview.duplicate_bytes, big.len() as u64);
    assert!(preview.duplicates[0].existing.ends_with("movie.bin"));
    let b = engine
        .add(riptor_core::AddParams {
            info_hash: preview.info_hash,
            save_path: Some(downloads.clone()),
            reuse_existing: true,
            ..Default::default()
        })
        .await
        .unwrap();
    // Nothing to download it from, yet the movie is already there and verified.
    let t = wait_for(&mut events, &b, "the reused file to verify", |t| {
        t.done >= big.len() as u64 && t.state != riptor_core::TorrentState::Checking
    })
    .await;
    assert!(t.progress < 1.0);
    let placed = downloads.join("Release").join("movie.bin");
    assert_eq!(std::fs::read(&placed).unwrap(), big);
    // The original is untouched.
    assert_eq!(std::fs::read(first.join("movie.bin")).unwrap(), big);
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_country_list_loads_only_when_turned_on() {
    use std::io::Write;
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    // A copy already downloaded, so nothing goes to the network.
    let cache = riptor_core::geoip::cache_file(&data);
    std::fs::create_dir_all(cache.parent().unwrap()).unwrap();
    let mut gz = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::fast());
    gz.write_all(b"1.0.0.0,1.0.0.255,AU\n").unwrap();
    std::fs::write(&cache, gz.finish().unwrap()).unwrap();

    let engine = engine(&data, quiet_settings(&tmp.path().join("downloads"))).await;
    tokio::time::sleep(Duration::from_secs(5)).await;
    assert!(!engine.country_status().ready, "off by default");

    let mut s = engine.settings();
    s.peer_countries = true;
    engine.update_settings(s).await.unwrap();
    tokio::time::timeout(Duration::from_secs(30), async {
        while !engine.country_status().ready {
            tokio::time::sleep(Duration::from_millis(200)).await;
        }
    })
    .await
    .expect("the cached list loads");
    assert!(engine.country_status().attribution.contains("DB-IP"));

    // Off again: gone from memory and disk.
    let mut s = engine.settings();
    s.peer_countries = false;
    engine.update_settings(s).await.unwrap();
    assert!(!engine.country_status().ready);
    assert!(!cache.exists());
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn state_is_saved_when_it_changes_and_not_otherwise() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let engine = engine(&data, quiet_settings(&tmp.path().join("dl"))).await;
    let file = data.join("torrents.json");

    engine.set_category_def("tv", None).unwrap();
    assert!(std::fs::read_to_string(&file).unwrap().contains("\"tv\""));
    let modified = std::fs::metadata(&file).unwrap().modified().unwrap();
    let writes = engine.state_writes();

    engine.set_category_def("tv", None).unwrap();
    assert_eq!(
        std::fs::metadata(&file).unwrap().modified().unwrap(),
        modified,
        "an unchanged torrents.json was rewritten"
    );
    // Only the statistics can have moved on (uptime).
    assert!(engine.state_writes() - writes <= 1);

    engine.set_category_def("films", None).unwrap();
    assert!(std::fs::read_to_string(&file)
        .unwrap()
        .contains("\"films\""));
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn imports_carry_labels_history_and_limits() {
    use riptor_core::create::{create, CreateOptions, Progress};
    use riptor_core::{Limit, Source};
    let tmp = tempfile::tempdir().unwrap();
    let downloads = tmp.path().join("downloads");
    let content = downloads.join("content");
    std::fs::create_dir_all(&content).unwrap();
    std::fs::write(content.join("a.bin"), vec![3u8; 100_000]).unwrap();
    let torrent = create(
        &content,
        &CreateOptions {
            piece_length: Some(16 * 1024),
            ..Default::default()
        },
        &Progress::default(),
    )
    .unwrap();

    // A portable qBittorrent profile, with its all-time totals (1234567890123 down, 42 up).
    let profile = tmp.path().join("portable").join("qBittorrent");
    let from = profile.join("data").join("BT_backup");
    std::fs::create_dir_all(&from).unwrap();
    std::fs::create_dir_all(profile.join("config")).unwrap();
    std::fs::write(
        profile.join("config").join("qBittorrent-data.ini"),
        r"[Stats]
AllStats=@Variant(\0\0\0\x1c\0\0\0\x2\0\0\0\x12\0\x41\0l\0l\0t\0i\0m\0\x65\0\x44\0L\0\0\0\x4\0\0\x1\x1fq\xfb\x4\xcb\0\0\0\x12\0\x41\0l\0l\0t\0i\0m\0\x65\0U\0L\0\0\0\x4\0\0\0\0\0\0\0*)
",
    )
    .unwrap();
    std::fs::write(from.join("abc.torrent"), &torrent).unwrap();
    let s = |v: &str| format!("{}:{v}", v.len());
    let save = downloads.to_string_lossy().into_owned();
    let resume = format!(
        "d{}{}{}i1e{}{}{}l{}e{}i5000e{}i12345e{}i600e{}i300e{}i1700000000e{}i2500e{}i-1ee",
        s("save_path"),
        s(&save),
        s("paused"),
        s("qBt-category"),
        s("Linux/ISOs"),
        s("qBt-tags"),
        s("debian"),
        s("total_downloaded"),
        s("total_uploaded"),
        s("active_time"),
        s("seeding_time"),
        s("added_time"),
        s("qBt-ratioLimit"),
        s("qBt-seedingTimeLimit"),
    );
    std::fs::write(from.join("abc.fastresume"), resume).unwrap();

    let engine = engine(&tmp.path().join("data"), quiet_settings(&downloads)).await;
    let mut events = engine.subscribe();
    let report = engine.import_from(&from).await.unwrap();
    assert_eq!(
        (report.added, report.skipped),
        (1, 0),
        "{:?}",
        report.failed
    );
    let id = engine
        .preview(Source::File(from.join("abc.torrent")))
        .await
        .unwrap()
        .info_hash;

    // A tick may land between adding and copying the history over.
    let t = wait_for(&mut events, &id, "the imported history", |t| t.uploaded > 0).await;
    assert_eq!(t.category.as_deref(), Some("Linux-ISOs"));
    assert!(engine.categories().contains_key("Linux-ISOs"));
    assert_eq!(t.tags, ["debian"]);
    assert_eq!((t.downloaded, t.uploaded), (5000, 12345));
    assert_eq!((t.seeding_time, t.added_on), (300, 1_700_000_000));
    assert_eq!(t.limits.ratio, Limit::Value(2.5));
    assert_eq!(t.limits.seeding_time, Limit::Unlimited);
    assert_eq!(t.limits.inactive_seeding_time, Limit::UseGlobal);
    assert_eq!(Path::new(&t.save_path), content);

    // Importing again leaves what Riptor already has alone.
    let all = engine.statistics().all_time;
    assert_eq!(report.all_time.map(|t| t.uploaded), Some(42));
    assert_eq!(
        engine
            .import_scan(Some(from.clone()))
            .await
            .unwrap()
            .all_time,
        report.all_time
    );
    assert!(all.all_time_download >= 1_234_567_890_123 && all.all_time_upload >= 42);

    // Importing again leaves what Riptor already has alone, and doesn't count the totals twice.
    let again = engine.import_from(&from).await.unwrap();
    assert_eq!((again.added, again.already_present), (0, 1));
    let after = engine.statistics().all_time;
    assert!(
        after.all_time_download < 2 * 1_234_567_890_123,
        "counted twice"
    );
    assert_eq!(after.all_time_upload, all.all_time_upload);
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn transfer_history_is_sealed_and_edits_are_rejected() {
    let tmp = tempfile::tempdir().unwrap();
    let data = tmp.path().join("data");
    let downloads = tmp.path().join("downloads");
    let content = downloads.join("content");
    std::fs::create_dir_all(&content).unwrap();
    std::fs::write(content.join("a.bin"), vec![1u8; 50_000]).unwrap();

    // An older install's plain statistics are sealed on first start.
    let old = riptor_core::Statistics {
        all_time_upload: 777,
        all_time_download: 555,
        ..Default::default()
    };
    riptor_core::state_file::save(&data.join("statistics.json"), &old).unwrap();
    let e = engine(&data, quiet_settings(&downloads)).await;
    assert_eq!(e.statistics().all_time.all_time_upload, 777);
    assert!(e.statistics().history_reset.is_none());
    assert!(data.join("history.dat").exists());
    assert!(
        !data.join("statistics.json").exists(),
        "the plain file stays behind"
    );
    let id = seed_in_place(&e, &content, &tmp.path().join("c.torrent")).await;
    e.shutdown().await.unwrap();
    let sealed = std::fs::read(data.join("history.dat")).unwrap();
    assert!(!String::from_utf8_lossy(&sealed).contains("777"));

    // Editing torrents.json doesn't change a torrent's totals.
    let path = data.join("torrents.json");
    let text = std::fs::read_to_string(&path).unwrap();
    let forged = text.replace("\"all_time_upload\": 0", "\"all_time_upload\": 999999");
    assert_ne!(forged, text);
    std::fs::write(&path, forged).unwrap();
    let e = engine(&data, quiet_settings(&downloads)).await;
    let mut events = e.subscribe();
    let t = wait_for(&mut events, &id, "the torrent", |_| true).await;
    assert_eq!(t.uploaded, 0, "an edited total was trusted");
    assert_eq!(e.statistics().all_time.all_time_upload, 777);
    e.shutdown().await.unwrap();

    // Any change to the sealed file resets the history, says so, and keeps the torrent.
    let mut bytes = std::fs::read(data.join("history.dat")).unwrap();
    let n = bytes.len();
    bytes[n / 2] ^= 0x20;
    std::fs::write(data.join("history.dat"), bytes).unwrap();
    let e = engine(&data, quiet_settings(&downloads)).await;
    let report = e.statistics();
    assert_eq!(report.all_time.all_time_upload, 0);
    assert!(report.history_reset.is_some());
    assert!(std::fs::read_dir(&data).unwrap().flatten().any(|f| f
        .file_name()
        .to_string_lossy()
        .starts_with("history.dat.rejected-")));
    let mut events = e.subscribe();
    wait_for(&mut events, &id, "the torrent after a reset", |_| true).await;
    e.shutdown().await.unwrap();

    // The fresh history is sealed and accepted next time.
    let e = engine(&data, quiet_settings(&downloads)).await;
    assert!(e.statistics().history_reset.is_none());
    e.shutdown().await.unwrap();
}
