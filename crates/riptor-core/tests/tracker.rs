//! Tracker replies flowing into swarm sizes, the ledger and the swarm view. Its own process: those are global.

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

/// A tracker on 127.0.0.1 that always gives the same answer.
async fn fake_tracker(body: Vec<u8>) -> String {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let listener = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))
        .await
        .unwrap();
    let addr = listener.local_addr().unwrap();
    tokio::spawn(async move {
        loop {
            let Ok((mut s, _)) = listener.accept().await else {
                return;
            };
            let body = body.clone();
            tokio::spawn(async move {
                let mut buf = [0u8; 4096];
                let _ = s.read(&mut buf).await;
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = s.write_all(head.as_bytes()).await;
                let _ = s.write_all(&body).await;
            });
        }
    });
    format!("http://{addr}/announce")
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn tracker_swarm_counts_reach_the_torrent_list() {
    let tmp = tempfile::tempdir().unwrap();
    let mut body =
        b"d8:completei1e10:incompletei7e8:intervali1800e5:peers0:11:external ip4:".to_vec();
    body.extend_from_slice(&[8, 8, 4, 4]);
    body.push(b'e');
    let tracker = fake_tracker(body).await;
    let content = tmp.path().join("shared").join("Rare");
    std::fs::create_dir_all(&content).unwrap();
    std::fs::write(content.join("only-copy.bin"), vec![3u8; 100_000]).unwrap();
    let engine = engine(
        &tmp.path().join("data"),
        quiet_settings(&tmp.path().join("downloads")),
    )
    .await;
    let mut events = engine.subscribe();
    let id = engine
        .create_torrent(
            content.clone(),
            tmp.path().join("Rare.torrent"),
            riptor_core::create::CreateOptions {
                trackers: vec![tracker],
                ..Default::default()
            },
            true,
        )
        .await
        .unwrap()
        .unwrap();
    let t = wait_for(&mut events, &id, "the tracker's counts", |t| {
        t.swarm_seeds.is_some()
    })
    .await;
    assert_eq!((t.swarm_seeds, t.swarm_leechers), (Some(1), Some(7)));
    // The announce shows up in the connections ledger as a local host.
    use riptor_net::ledger::{snapshot, Purpose, Route};
    assert!(snapshot().iter().any(|e| e.purpose == Purpose::Tracker
        && e.host == "127.0.0.1"
        && e.route == Route::Local));
    // So does the address the tracker says it saw (BEP 24).
    assert!(riptor_core::seen_ip::summary()
        .iter()
        .any(|s| s.ip == "8.8.4.4" && s.trackers == 1));
    // Not on the VPN, so nothing is judged or held.
    let view = engine.swarm_view();
    assert_eq!(view.verdict, riptor_core::seen_ip::Verdict::Unknown);
    assert!(view.held.is_none());
    engine.shutdown().await.unwrap();
}
