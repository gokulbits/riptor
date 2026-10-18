//! End-to-end tests over loopback: real librqbit sessions talking the
//! BitTorrent protocol, with Riptor's storage layer on both sides.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use librqbit::spawn_utils::BlockingSpawner;
use librqbit::{
    create_torrent, AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerOptions, Session,
    SessionOptions,
};
use rand::{Rng, SeedableRng};
use riptor_core::{AddParams, Engine, EngineConfig, Event, Settings, Source, TorrentState};
use riptor_storage::{CachedStorageFactory, Shared, StorageConfig, TorrentRoot};
use sha2::{Digest, Sha256};

/// A few files of odd sizes so pieces straddle file boundaries.
fn make_content(dir: &Path) -> Vec<(PathBuf, usize)> {
    let mut rng = rand::rngs::StdRng::seed_from_u64(42);
    let files = [
        ("a.bin", 3 * 1024 * 1024 + 17),
        ("sub/b.bin", 9 * 1024 * 1024 + 3),
        ("sub/deeper/c.txt", 1234),
        ("d.bin", 5 * 1024 * 1024),
    ];
    files
        .iter()
        .map(|(name, len)| {
            let p = dir.join(name);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            let mut data = vec![0u8; *len];
            rng.fill(&mut data[..]);
            std::fs::write(&p, &data).unwrap();
            (PathBuf::from(name), *len)
        })
        .collect()
}

fn sha(p: &Path) -> Vec<u8> {
    Sha256::digest(std::fs::read(p).unwrap()).to_vec()
}

async fn session(out: &Path, factory: &CachedStorageFactory) -> Arc<Session> {
    Session::new_with_opts(
        out.to_path_buf(),
        SessionOptions {
            dht: None,
            persistence: None,
            disable_local_service_discovery: true,
            listen: Some(ListenerOptions {
                listen_addr: (Ipv4Addr::LOCALHOST, 0).into(),
                ipv4_only: true,
                ..Default::default()
            }),
            default_storage_factory: Some(factory.clone().into_boxed()),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn transfer_between_two_sessions_with_cached_storage() {
    // Second run squeezes the write buffer so pieces spill to disk mid-download.
    for (label, write_buffer) in [("normal", 64usize << 20), ("spilling", 64 * 1024)] {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("seed").join("content");
        let files = make_content(&content);
        let created = create_torrent(
            &content,
            CreateTorrentOptions {
                name: Some("content"),
                trackers: vec![],
                piece_length: Some(256 * 1024),
            },
            &BlockingSpawner::new(4),
        )
        .await
        .unwrap();
        let hash = created.info_hash();
        let bytes = created.as_bytes().unwrap();

        // Seeder: data already in place.
        let seed_factory = CachedStorageFactory::new(Shared::new(StorageConfig::default()));
        seed_factory.register(
            hash,
            TorrentRoot {
                path: content.clone(),
                owned: false,
            },
        );
        let seeder = session(tmp.path(), &seed_factory).await;
        let h = seeder
            .add_torrent(
                AddTorrent::from_bytes(bytes.clone()),
                Some(AddTorrentOptions {
                    overwrite: true,
                    output_folder: Some(content.to_str().unwrap().into()),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(60), h.wait_until_completed())
            .await
            .unwrap()
            .unwrap();
        let seed_addr: SocketAddr = seeder.listen_addr().unwrap();
        let seed_addr = SocketAddr::new(Ipv4Addr::LOCALHOST.into(), seed_addr.port());

        // Leecher: empty folder.
        let leech_root = tmp.path().join("leech").join("content");
        let leech_factory = CachedStorageFactory::new(Shared::new(StorageConfig {
            write_buffer_bytes: write_buffer,
            ..StorageConfig::default()
        }));
        leech_factory.register(
            hash,
            TorrentRoot {
                path: leech_root.clone(),
                owned: true,
            },
        );
        let leecher = session(tmp.path(), &leech_factory).await;
        let l = leecher
            .add_torrent(
                AddTorrent::from_bytes(bytes),
                Some(AddTorrentOptions {
                    overwrite: true,
                    output_folder: Some(leech_root.to_str().unwrap().into()),
                    initial_peers: Some(vec![seed_addr]),
                    ..Default::default()
                }),
            )
            .await
            .unwrap()
            .into_handle()
            .unwrap();
        tokio::time::timeout(Duration::from_secs(120), l.wait_until_completed())
            .await
            .unwrap_or_else(|_| panic!("[{label}] download timed out: {}", l.stats()))
            .unwrap();

        // Pause flushes and releases files; then compare byte-for-byte.
        leecher.pause(&l).await.unwrap();
        for (rel, len) in &files {
            let got = leech_root.join(rel);
            assert_eq!(
                std::fs::metadata(&got).unwrap().len() as usize,
                *len,
                "[{label}] {rel:?} size"
            );
            assert_eq!(
                sha(&got),
                sha(&content.join(rel)),
                "[{label}] {rel:?} content"
            );
        }
        assert!(l.stats().uploaded_bytes == 0 || l.stats().finished);
        seeder.stop().await;
        leecher.stop().await;
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn engine_seeds_then_stops_at_ratio_limit() {
    let tmp = tempfile::tempdir().unwrap();
    let data_dir = tmp.path().join("appdata");
    let downloads = tmp.path().join("downloads");
    let content = downloads.join("content");
    make_content(&content);
    let created = create_torrent(
        &content,
        CreateTorrentOptions {
            name: Some("content"),
            trackers: vec![],
            piece_length: Some(256 * 1024),
        },
        &BlockingSpawner::new(4),
    )
    .await
    .unwrap();
    let torrent_file = tmp.path().join("content.torrent");
    std::fs::write(&torrent_file, created.as_bytes().unwrap()).unwrap();

    let mut settings = Settings {
        download_dir: downloads.clone(),
        dht: false,
        lsd: false,
        upnp: false,
        // Loopback only: tests must never trigger a firewall prompt.
        listen_address: Some(Ipv4Addr::LOCALHOST.into()),
        ..Settings::default()
    };
    // Ratio 0 is reached as soon as the torrent is complete.
    settings.seeding.max_ratio = Some(0.0);
    riptor_core::state_file::save(&data_dir.join("settings.json"), &settings).unwrap();

    let engine = Engine::start(EngineConfig {
        data_dir: data_dir.clone(),
    })
    .await
    .unwrap();
    let mut events = engine.subscribe();
    let id = engine
        .add_direct(
            Source::File(torrent_file),
            AddParams {
                save_path: Some(downloads.clone()),
                ..Default::default()
            },
        )
        .await
        .unwrap();

    let mut seen_complete = false;
    let stopped = tokio::time::timeout(Duration::from_secs(60), async {
        loop {
            match events.recv().await {
                Ok(Event::Tick { torrents, .. }) => {
                    if let Some(t) = torrents.iter().find(|t| t.id == id) {
                        if t.progress >= 1.0 {
                            seen_complete = true;
                        }
                        if t.state == TorrentState::Stopped {
                            return t.clone();
                        }
                    }
                }
                Ok(_) => {}
                Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {}
                Err(e) => panic!("{e}"),
            }
        }
    })
    .await
    .expect("torrent never stopped at its ratio limit");

    assert!(seen_complete, "data on disk should verify as complete");
    assert_eq!(stopped.progress, 1.0);
    assert!(stopped.completed_on.is_some());
    // Existing files sit in the user's folder, so Riptor must not claim it.
    assert_eq!(PathBuf::from(&stopped.save_path), content);

    engine.shutdown().await.unwrap();
    let saved = std::fs::read_to_string(data_dir.join("torrents.json")).unwrap();
    assert!(saved.contains("\"intent\": \"stopped\""), "{saved}");
}

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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn backup_roundtrip_into_fresh_install() {
    use riptor_core::RestoreOptions;
    let tmp = tempfile::tempdir().unwrap();
    let downloads = tmp.path().join("downloads");
    let content = downloads.join("content");
    make_content(&content);
    let created = create_torrent(
        &content,
        CreateTorrentOptions {
            name: Some("content"),
            trackers: vec![],
            piece_length: Some(256 * 1024),
        },
        &BlockingSpawner::new(4),
    )
    .await
    .unwrap();
    let torrent_file = tmp.path().join("content.torrent");
    std::fs::write(&torrent_file, created.as_bytes().unwrap()).unwrap();

    // Install A: add a torrent, make a backup.
    let a_dir = tmp.path().join("install-a");
    let mut settings = quiet_settings(&downloads);
    settings.peer_limit = Some(77); // something recognisable
    riptor_core::state_file::save(&a_dir.join("settings.json"), &settings).unwrap();
    let a = Engine::start(EngineConfig { data_dir: a_dir })
        .await
        .unwrap();
    a.set_category_def("linux", None).unwrap();
    let id = a
        .add_direct(
            Source::File(torrent_file),
            AddParams {
                category: Some("linux".into()),
                ..Default::default()
            },
        )
        .await
        .unwrap();
    tokio::time::sleep(Duration::from_secs(2)).await;
    let backup = tmp.path().join("my.riptorbackup");
    let summary = a.create_backup(&backup, None).unwrap();
    assert_eq!(summary.torrents, 1);
    assert!(!summary.password_protected);
    // Wrong extension and overwriting a non-backup file are refused.
    assert!(a.create_backup(&tmp.path().join("x.json"), None).is_err());
    let victim = tmp.path().join("notes.riptorbackup");
    std::fs::write(&victim, b"precious").unwrap();
    assert!(a.create_backup(&victim, None).is_err());
    assert_eq!(std::fs::read(&victim).unwrap(), b"precious");
    a.shutdown().await.unwrap();

    // A tampered copy is rejected.
    let mut bytes = std::fs::read(&backup).unwrap();
    let n = bytes.len();
    bytes[n / 2] ^= 0x40;
    let tampered = tmp.path().join("tampered.riptorbackup");
    std::fs::write(&tampered, &bytes).unwrap();

    // Install B: fresh, restore everything.
    let b_dir = tmp.path().join("install-b");
    riptor_core::state_file::save(&b_dir.join("settings.json"), &quiet_settings(&downloads))
        .unwrap();
    let b = Engine::start(EngineConfig { data_dir: b_dir })
        .await
        .unwrap();
    assert!(b.inspect_backup(&tampered, None).is_err());
    let opts = RestoreOptions {
        settings: true,
        torrents: true,
        statistics: true,
        resume_torrents: false,
    };
    let report = b.restore_backup(&backup, None, opts).await.unwrap();
    assert_eq!(report.added, 1, "{:?}", report.skipped);
    assert!(report.settings_restored && report.statistics_restored);
    assert_eq!(b.settings().peer_limit, Some(77));
    assert!(b.categories().contains_key("linux"));
    assert_eq!(b.statistics().all_time.torrents_added, 1);
    // Restored stopped (password-less backup), with its files present.
    assert_eq!(b.files(&id).unwrap().len(), 4);
    // Restoring again is a no-op for torrents already present.
    let again = b.restore_backup(&backup, None, opts).await.unwrap();
    assert_eq!((again.added, again.already_present), (0, 1));
    b.shutdown().await.unwrap();
}

#[test]
fn forged_backup_cannot_target_protected_folders() {
    use riptor_core::backup::{seal, BackupPayload, BackupTorrent};
    // A forged password-less backup pointing a torrent at a protected folder.
    let protected = if cfg!(windows) {
        std::path::PathBuf::from(std::env::var("APPDATA").unwrap())
            .join(r"Microsoft\Windows\Start Menu\Programs\Startup")
    } else {
        std::path::PathBuf::from(std::env::var("HOME").unwrap()).join(".config/autostart")
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();
    rt.block_on(async {
        let tmp = tempfile::tempdir().unwrap();
        let content = tmp.path().join("c");
        make_content(&content);
        let created = create_torrent(
            &content,
            CreateTorrentOptions {
                name: Some("evil"),
                trackers: vec![],
                piece_length: None,
            },
            &BlockingSpawner::new(2),
        )
        .await
        .unwrap();
        let bytes = created.as_bytes().unwrap();
        let rec: riptor_core::Record = serde_json::from_value(serde_json::json!({
            "info_hash": created.info_hash().as_string(),
            "name": "evil",
            "root": protected,
            "owned_root": false,
            "added_on": 1,
            "intent": "started",
        }))
        .unwrap();
        use base64::Engine as _;
        let payload = BackupPayload {
            created: 1,
            app_version: "x".into(),
            settings: quiet_settings(&tmp.path().join("dl")),
            categories: Default::default(),
            torrents: vec![BackupTorrent {
                record: rec,
                torrent: base64::engine::general_purpose::STANDARD.encode(&bytes),
                only_files: None,
            }],
            statistics: Default::default(),
        };
        let path = tmp.path().join("forged.riptorbackup");
        std::fs::write(&path, seal(&payload, None).unwrap()).unwrap();

        let dir = tmp.path().join("install");
        riptor_core::state_file::save(
            &dir.join("settings.json"),
            &quiet_settings(&tmp.path().join("dl")),
        )
        .unwrap();
        let e = Engine::start(EngineConfig { data_dir: dir }).await.unwrap();
        let opts = riptor_core::RestoreOptions {
            settings: false,
            torrents: true,
            statistics: false,
            resume_torrents: true,
        };
        let report = e.restore_backup(&path, None, opts).await.unwrap();
        assert_eq!(report.added, 0);
        assert!(
            report.skipped[0].contains("protected"),
            "{:?}",
            report.skipped
        );
        e.shutdown().await.unwrap();
    });
}
