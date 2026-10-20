//! Real sessions talking to each other over loopback.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use librqbit::spawn_utils::BlockingSpawner;
use librqbit::{
    create_torrent, AddTorrent, AddTorrentOptions, CreateTorrentOptions, ListenerOptions, Session,
    SessionOptions,
};
use rand::{RngExt, SeedableRng};
use riptor_core::{AddParams, Engine, EngineConfig, Event, Settings, Source, TorrentState};
use riptor_storage::{CachedStorageFactory, Shared, StorageConfig, TorrentRoot};
use sha2::{Digest, Sha256};

/// Odd sizes so pieces straddle files.
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
    // A tiny write buffer forces spills.
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
        // No firewall prompt.
        listen_address: Some(Ipv4Addr::LOCALHOST.into()),
        ..Settings::default()
    };
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

    let a_dir = tmp.path().join("install-a");
    let mut settings = quiet_settings(&downloads);
    settings.peer_limit = Some(77); // something recognisable
    settings.columns = vec![
        riptor_core::ColumnPref {
            key: "size".into(),
            visible: true,
            width: Some(120),
        },
        riptor_core::ColumnPref {
            key: "name".into(),
            visible: false,
            width: None,
        },
    ];
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
    assert_eq!(b.settings().columns, settings.columns);
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn kill_switch_blocks_everything_until_lifted() {
    use riptor_core::{NetworkMode, TorrentState};
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

    let data_dir = tmp.path().join("appdata");
    let mut settings = quiet_settings(&downloads);
    settings.vpn_required = true;
    riptor_core::state_file::save(&data_dir.join("settings.json"), &settings).unwrap();
    let engine = Engine::start(EngineConfig { data_dir }).await.unwrap();
    assert_eq!(
        engine.network(),
        NetworkMode::Blocked,
        "must start blocked when the VPN is required"
    );
    let mut events = engine.subscribe();
    let id = engine
        .add_direct(Source::File(torrent_file), AddParams::default())
        .await
        .unwrap();

    let mut ticks = 0;
    while ticks < 4 {
        if let Ok(Event::Tick { torrents, global }) = events.recv().await {
            ticks += 1;
            assert_eq!(global.network, "blocked");
            assert_eq!(global.listen_port, None, "nothing may listen while blocked");
            if let Some(t) = torrents.iter().find(|t| t.id == id) {
                assert!(
                    !matches!(
                        t.state,
                        TorrentState::Seeding
                            | TorrentState::StalledUp
                            | TorrentState::Downloading
                            | TorrentState::StalledDl
                    ),
                    "torrent ran while blocked: {:?}",
                    t.state
                );
            }
        }
    }

    // Lifting the block lets it seed.
    engine.set_network(NetworkMode::Normal).await.unwrap();
    let ran = tokio::time::timeout(Duration::from_secs(30), async {
        loop {
            if let Ok(Event::Tick { torrents, .. }) = events.recv().await {
                if torrents.iter().any(|t| {
                    t.id == id && matches!(t.state, TorrentState::Seeding | TorrentState::StalledUp)
                }) {
                    return;
                }
            }
        }
    })
    .await;
    assert!(
        ran.is_ok(),
        "torrent did not start after the block was lifted"
    );

    // Must fail rather than fall back to the normal network.
    let bad = NetworkMode::Vpn(riptor_core::VpnBinding {
        adapter: "#999999".into(),
        local_ip: Ipv4Addr::new(10, 255, 255, 1),
        forwarded_port: None,
    });
    assert!(engine.set_network(bad).await.is_err());
    engine.shutdown().await.unwrap();
}

/// While checking, progress counts checked bytes, so 100% alone isn't enough.
fn complete(t: &riptor_core::TorrentSummary) -> bool {
    t.progress >= 1.0 && !matches!(t.state, TorrentState::Checking | TorrentState::Metadata)
}

async fn wait_for(
    events: &mut tokio::sync::broadcast::Receiver<Event>,
    id: &str,
    what: &str,
    pred: impl Fn(&riptor_core::TorrentSummary) -> bool,
) -> riptor_core::TorrentSummary {
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

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn move_and_recheck_keep_the_data() {
    let tmp = tempfile::tempdir().unwrap();
    let downloads = tmp.path().join("downloads");
    let content = downloads.join("content");
    let rels: Vec<PathBuf> = make_content(&content).into_iter().map(|(p, _)| p).collect();
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

    let data_dir = tmp.path().join("data");
    riptor_core::state_file::save(&data_dir.join("settings.json"), &quiet_settings(&downloads))
        .unwrap();
    let engine = Engine::start(EngineConfig {
        data_dir: data_dir.clone(),
    })
    .await
    .unwrap();
    let mut events = engine.subscribe();
    let id = engine
        .add_direct(Source::File(torrent_file), AddParams::default())
        .await
        .unwrap();
    wait_for(&mut events, &id, "initial check", complete).await;
    let hashes: Vec<_> = rels.iter().map(|p| sha(&content.join(p))).collect();

    // Into Riptor's own data folder: refused before anything moves.
    assert!(engine
        .move_torrents(std::slice::from_ref(&id), &data_dir)
        .await
        .is_err());

    // Something already at the destination: nothing moves, nothing is lost.
    let blocked = tmp.path().join("blocked");
    let clash = blocked.join("content").join(&rels[0]);
    std::fs::create_dir_all(clash.parent().unwrap()).unwrap();
    std::fs::write(&clash, b"someone else's file").unwrap();
    assert!(engine
        .move_torrents(std::slice::from_ref(&id), &blocked)
        .await
        .is_err());
    assert_eq!(std::fs::read(&clash).unwrap(), b"someone else's file");
    for (p, h) in rels.iter().zip(&hashes) {
        assert_eq!(
            &sha(&content.join(p)),
            h,
            "{} changed after a failed move",
            p.display()
        );
    }
    wait_for(
        &mut events,
        &id,
        "still complete after the failed move",
        |t| complete(t) && Path::new(&t.save_path) == content,
    )
    .await;

    // A real move: the torrent's folder comes along and the old one goes.
    let elsewhere = tmp.path().join("elsewhere");
    engine
        .move_torrents(std::slice::from_ref(&id), &elsewhere)
        .await
        .unwrap();
    let moved_root = elsewhere.join("content");
    assert!(!content.exists(), "old folder should be gone");
    for (p, h) in rels.iter().zip(&hashes) {
        let new = moved_root.join(p);
        assert_eq!(&sha(&new), h, "{} differs after the move", new.display());
    }
    let t = wait_for(&mut events, &id, "verified at the new place", |t| {
        complete(t) && Path::new(&t.save_path) == moved_root
    })
    .await;
    assert_eq!(t.name, "content");

    // Recheck after damaging a piece: progress drops, the rest stays verified.
    let victim = moved_root.join(&rels[0]);
    let mut bytes = std::fs::read(&victim).unwrap();
    bytes[0] ^= 0xff;
    std::fs::write(&victim, &bytes).unwrap();
    engine.recheck(std::slice::from_ref(&id)).await.unwrap();
    let t = wait_for(&mut events, &id, "recheck finds the damage", |t| {
        t.progress < 1.0 && t.progress > 0.0
    })
    .await;
    assert_eq!(Path::new(&t.save_path), moved_root);

    // The new place survives a restart.
    engine.shutdown().await.unwrap();
    let saved = std::fs::read_to_string(data_dir.join("torrents.json")).unwrap();
    assert!(saved.contains("elsewhere"), "{saved}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn per_torrent_speed_limits_are_validated_and_kept() {
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
    let data_dir = tmp.path().join("data");
    riptor_core::state_file::save(&data_dir.join("settings.json"), &quiet_settings(&downloads))
        .unwrap();
    let engine = Engine::start(EngineConfig {
        data_dir: data_dir.clone(),
    })
    .await
    .unwrap();
    let mut events = engine.subscribe();
    let id = engine
        .add_direct(Source::File(torrent_file), AddParams::default())
        .await
        .unwrap();
    let ids = std::slice::from_ref(&id);

    assert!(engine.set_speed_limits(ids, Some(u32::MAX), None).is_err());
    engine.set_speed_limits(ids, Some(512), Some(0)).unwrap();
    let t = wait_for(&mut events, &id, "limits in the summary", |t| {
        t.dl_limit_kib.is_some()
    })
    .await;
    assert_eq!((t.dl_limit_kib, t.ul_limit_kib), (Some(512), None));

    engine.shutdown().await.unwrap();
    let saved = std::fs::read_to_string(data_dir.join("torrents.json")).unwrap();
    assert!(saved.contains("\"dl_limit_kib\": 512"), "{saved}");
}

/// Range server: `/good/` serves `root`, `/bad/` sends zeros.
async fn range_server(root: PathBuf) -> (SocketAddr, Arc<std::sync::atomic::AtomicUsize>) {
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    let good_hits = Arc::new(AtomicUsize::new(0));
    let hits = good_hits.clone();
    tokio::spawn(async move {
        loop {
            let Ok((mut sock, _)) = listener.accept().await else {
                return;
            };
            let root = root.clone();
            let hits = hits.clone();
            tokio::spawn(async move {
                let mut req = Vec::new();
                let mut buf = [0u8; 1024];
                while !req.windows(4).any(|w| w == b"\r\n\r\n") && req.len() < 8192 {
                    match sock.read(&mut buf).await {
                        Ok(0) | Err(_) => return,
                        Ok(n) => req.extend_from_slice(&buf[..n]),
                    }
                }
                let req = String::from_utf8_lossy(&req).to_string();
                let path = req.split_whitespace().nth(1).unwrap_or("/").to_string();
                let range = req
                    .lines()
                    .find_map(|l| {
                        l.to_ascii_lowercase()
                            .strip_prefix("range: bytes=")
                            .map(str::to_string)
                    })
                    .and_then(|r| {
                        let (a, b) = r.trim().split_once('-')?;
                        Some((a.parse::<u64>().ok()?, b.parse::<u64>().ok()?))
                    });
                let (good, rel) = match (path.strip_prefix("/good/"), path.strip_prefix("/bad/")) {
                    (Some(r), _) => (true, r.to_string()),
                    (_, Some(r)) => (false, r.to_string()),
                    _ => return,
                };
                let Ok(data) = std::fs::read(root.join(&rel)) else {
                    let _ = sock
                        .write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n")
                        .await;
                    return;
                };
                let (a, b) = range.unwrap_or((0, data.len() as u64 - 1));
                let body = if good {
                    hits.fetch_add(1, Ordering::Relaxed);
                    data[a as usize..=b as usize].to_vec()
                } else {
                    vec![0u8; (b - a + 1) as usize]
                };
                let head = format!(
                    "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {a}-{b}/{}\r\nConnection: close\r\n\r\n",
                    body.len(),
                    data.len()
                );
                let _ = sock.write_all(head.as_bytes()).await;
                let _ = sock.write_all(&body).await;
            });
        }
    });
    (addr, good_hits)
}

/// Adds a `url-list` to a .torrent without touching the info dict.
fn with_url_list(torrent: &[u8], urls: &[String]) -> Vec<u8> {
    let mut out = b"d8:url-listl".to_vec();
    for u in urls {
        out.extend_from_slice(format!("{}:{u}", u.len()).as_bytes());
    }
    out.push(b'e');
    out.extend_from_slice(&torrent[1..]);
    out
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn web_seed_alone_completes_a_download() {
    let tmp = tempfile::tempdir().unwrap();
    let source = tmp.path().join("source");
    let content = source.join("content");
    let rels: Vec<PathBuf> = make_content(&content).into_iter().map(|(p, _)| p).collect();
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
    let (addr, good_hits) = range_server(source.clone()).await;
    // A lying server first: its pieces fail the check and it gets dropped.
    let torrent = with_url_list(
        &created.as_bytes().unwrap(),
        &[
            format!("http://{addr}/bad/"),
            format!("http://{addr}/good/"),
        ],
    );
    let torrent_file = tmp.path().join("web.torrent");
    std::fs::write(&torrent_file, torrent).unwrap();

    let downloads = tmp.path().join("downloads");
    let data_dir = tmp.path().join("data");
    riptor_core::state_file::save(&data_dir.join("settings.json"), &quiet_settings(&downloads))
        .unwrap();
    let engine = Engine::start(EngineConfig { data_dir }).await.unwrap();
    engine.allow_local_web_seeds();
    let mut events = engine.subscribe();
    let id = engine
        .add_direct(Source::File(torrent_file), AddParams::default())
        .await
        .unwrap();
    // No peers, trackers or DHT: everything comes from the web seed.
    wait_for(&mut events, &id, "download from the web seed", |t| {
        complete(t)
    })
    .await;
    for p in &rels {
        assert_eq!(
            sha(&downloads.join("content").join(p)),
            sha(&content.join(p)),
            "{}",
            p.display()
        );
    }
    assert!(good_hits.load(std::sync::atomic::Ordering::Relaxed) > 0);
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn watched_folder_adds_and_renames() {
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
    let watch = tmp.path().join("watch");
    std::fs::create_dir_all(&watch).unwrap();
    let old = std::time::SystemTime::now() - Duration::from_secs(60);
    for (name, bytes) in [
        ("good.torrent", created.as_bytes().unwrap().to_vec()),
        ("junk.torrent", b"not a torrent".to_vec()),
    ] {
        let p = watch.join(name);
        std::fs::write(&p, bytes).unwrap();
        std::fs::File::options()
            .write(true)
            .open(&p)
            .unwrap()
            .set_modified(old)
            .unwrap();
    }

    let data_dir = tmp.path().join("data");
    let mut settings = quiet_settings(&downloads);
    // Riptor's own data folder is refused, like any protected save folder.
    let engine = Engine::start(EngineConfig {
        data_dir: data_dir.clone(),
    })
    .await
    .unwrap();
    settings.watched_folder = Some(data_dir.clone());
    assert!(engine.update_settings(settings.clone()).await.is_err());
    settings.watched_folder = Some(watch.clone());
    engine.update_settings(settings).await.unwrap();

    let mut events = engine.subscribe();
    let id = created.info_hash().as_string();
    wait_for(&mut events, &id, "added from the watched folder", |t| {
        complete(t)
    })
    .await;
    assert!(watch.join("good.torrent.added").exists());
    assert!(watch.join("junk.torrent.invalid").exists());
    assert!(!watch.join("good.torrent").exists());
    engine.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn created_torrent_seeds_in_place() {
    let tmp = tempfile::tempdir().unwrap();
    let downloads = tmp.path().join("downloads");
    let shared = tmp.path().join("shared");
    let content = shared.join("My Files");
    make_content(&content);
    let data_dir = tmp.path().join("data");
    riptor_core::state_file::save(&data_dir.join("settings.json"), &quiet_settings(&downloads))
        .unwrap();
    let engine = Engine::start(EngineConfig {
        data_dir: data_dir.clone(),
    })
    .await
    .unwrap();
    let opts = || riptor_core::create::CreateOptions {
        trackers: vec!["udp://tracker.example:6969/announce".into()],
        ..Default::default()
    };
    // Never into Riptor's own folder, never inside the shared folder.
    for bad in [
        data_dir.join("x.torrent"),
        content.join("x.torrent"),
        tmp.path().join("x.txt"),
    ] {
        assert!(engine
            .create_torrent(content.clone(), bad, opts(), false)
            .await
            .is_err());
    }
    let out = tmp.path().join("My Files.torrent");
    let mut events = engine.subscribe();
    let id = engine
        .create_torrent(content.clone(), out.clone(), opts(), true)
        .await
        .unwrap()
        .expect("added");
    assert!(out.exists());
    let t = wait_for(&mut events, &id, "seeding from the original folder", |t| {
        complete(t)
    })
    .await;
    assert_eq!(Path::new(&t.save_path), content);
    assert!(engine.create_progress().is_none());
    engine.shutdown().await.unwrap();
}
