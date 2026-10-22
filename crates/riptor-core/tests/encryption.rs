//! Peer encryption (MSE/PE): both sides of the handshake, hostile peers, and real transfers.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use librqbit::mse::testing::{self, Tamper};
use librqbit::spawn_utils::BlockingSpawner;
use librqbit::{
    create_torrent, AddTorrent, AddTorrentOptions, ConnectionOptions, CreateTorrentOptions,
    EncryptionPolicy, ListenerOptions, Session, SessionOptions,
};
use librqbit_core::hash_id::Id20;
use rand::{RngExt, SeedableRng};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};

const LIMIT: Duration = Duration::from_secs(10);

async fn pair() -> (TcpStream, TcpStream) {
    let l = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).await.unwrap();
    let addr = l.local_addr().unwrap();
    let (a, b) = tokio::join!(TcpStream::connect(addr), l.accept());
    (a.unwrap(), b.unwrap().0)
}

fn hash(n: u8) -> Id20 {
    Id20::new([n; 20])
}

fn bt_handshake(info_hash: Id20) -> Vec<u8> {
    let mut h = b"\x13BitTorrent protocol".to_vec();
    h.extend_from_slice(&[0u8; 8]);
    h.extend_from_slice(&info_hash.0);
    h.extend_from_slice(&[9u8; 20]);
    h
}

fn random(len: usize, seed: u64) -> Vec<u8> {
    let mut rng = rand::rngs::StdRng::seed_from_u64(seed);
    (0..len).map(|_| rng.random()).collect()
}

async fn within<T>(what: &str, f: impl std::future::Future<Output = T>) -> T {
    tokio::time::timeout(LIMIT, f)
        .await
        .unwrap_or_else(|_| panic!("{what} hung"))
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn encrypted_streams_carry_data_both_ways() {
    let (a, b) = pair().await;
    let (init, acc) = within("handshake", async {
        tokio::join!(
            testing::initiate(a, hash(1), true, false, &[]),
            testing::accept(b, EncryptionPolicy::Prefer, vec![hash(1)]),
        )
    })
    .await;
    let (ar, aw, a_enc) = init.unwrap();
    let (br, bw, b_enc) = acc.unwrap();
    assert!(a_enc && b_enc);

    // A megabyte each way at once, in odd-sized writes.
    let up = random(1 << 20, 1);
    let down = random(1 << 20, 2);
    let send = |mut w: testing::Writer, data: Vec<u8>| async move {
        for chunk in data.chunks(7919) {
            w.write_all(chunk).await.unwrap();
        }
        w
    };
    let recv = |mut r: testing::Reader, len: usize| async move {
        let mut got = vec![0u8; len];
        r.read_exact(&mut got).await.unwrap();
        got
    };
    let (_, _, got_b, got_a) = within("transfer", async {
        tokio::join!(
            send(aw, up.clone()),
            send(bw, down.clone()),
            recv(br, up.len()),
            recv(ar, down.len()),
        )
    })
    .await;
    assert!(got_b == up, "initiator to responder");
    assert!(got_a == down, "responder to initiator");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_bittorrent_handshake_comes_through() {
    // Empty initial payload, handshake sent afterwards through the encrypted stream.
    let (a, b) = pair().await;
    let initiator = async {
        let (_r, mut w, _) = testing::initiate(a, hash(5), true, false, &[])
            .await
            .unwrap();
        w.write_all(&bt_handshake(hash(5))).await.unwrap();
        w
    };
    let (_w, got) = within("handshake", async {
        tokio::join!(
            initiator,
            testing::accept_handshake(b, EncryptionPolicy::Require, vec![hash(4), hash(5)]),
        )
    })
    .await;
    assert_eq!(got.unwrap(), (hash(5), true));

    // The handshake as the initial payload, as many clients send it.
    let (a, b) = pair().await;
    let initiator = async {
        testing::initiate(a, hash(6), true, false, &bt_handshake(hash(6)))
            .await
            .unwrap()
    };
    let (_i, got) = within("handshake", async {
        tokio::join!(
            initiator,
            testing::accept_handshake(b, EncryptionPolicy::Prefer, vec![hash(6)]),
        )
    })
    .await;
    assert_eq!(got.unwrap(), (hash(6), true));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn the_right_torrent_is_picked_and_unknown_ones_refused() {
    let many: Vec<Id20> = (0..50).map(hash).collect();
    let (a, b) = pair().await;
    let (_, got) = within("handshake", async {
        tokio::join!(
            testing::initiate(a, hash(37), true, false, &[]),
            testing::accept(b, EncryptionPolicy::Prefer, many.clone()),
        )
    })
    .await;
    assert!(got.unwrap().2);

    let (a, b) = pair().await;
    let (_, got) = within("handshake", async {
        tokio::join!(
            testing::initiate(a, hash(200), true, false, &[]),
            testing::accept(b, EncryptionPolicy::Prefer, many),
        )
    })
    .await;
    assert!(got.is_err(), "a torrent we don't have was accepted");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn policies_decide_what_is_accepted() {
    use EncryptionPolicy::*;
    // A plain BitTorrent handshake.
    for (policy, ok) in [(Disabled, true), (Prefer, true), (Require, false)] {
        let (mut a, b) = pair().await;
        a.write_all(&bt_handshake(hash(1))).await.unwrap();
        let got = within("plain", testing::accept_handshake(b, policy, vec![hash(1)])).await;
        match got {
            Ok(g) => assert!(ok && g == (hash(1), false), "{policy:?}"),
            Err(e) => assert!(!ok, "{policy:?}: {e:#}"),
        }
    }
    // An encrypted one, offering (rc4, plaintext); the result is whether it ends up encrypted.
    let cases = [
        (Disabled, (true, false), None),
        (Prefer, (true, false), Some(true)),
        (Require, (true, false), Some(true)),
        (Prefer, (true, true), Some(true)),
        (Prefer, (false, true), Some(false)),
        (Require, (false, true), None),
    ];
    for (policy, (rc4, plain), want) in cases {
        let (a, b) = pair().await;
        let (init, acc) = within("handshake", async {
            tokio::join!(
                testing::initiate(a, hash(2), rc4, plain, &[]),
                testing::accept(b, policy, vec![hash(2)]),
            )
        })
        .await;
        match (want, acc) {
            (Some(enc), Ok((mut br, _bw, got))) => {
                assert_eq!(got, enc, "{policy:?} {rc4} {plain}");
                let (_ar, mut aw, a_enc) = init.unwrap();
                assert_eq!(a_enc, enc);
                // Plaintext after the handshake still lines up.
                aw.write_all(b"ping").await.unwrap();
                let mut got = [0u8; 4];
                within("read", br.read_exact(&mut got)).await.unwrap();
                assert_eq!(&got, b"ping");
            }
            (None, Err(_)) => {}
            (want, got) => panic!(
                "{policy:?} {rc4} {plain}: wanted {want:?}, got ok={}",
                got.is_ok()
            ),
        }
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn hostile_initiators_are_refused() {
    let p = testing::PRIME;
    let mut p_minus_1 = p;
    p_minus_1[95] -= 1;
    let mut one = [0u8; 96];
    one[95] = 1;
    let key = |k: [u8; 96]| Tamper {
        public_key: Some(k),
        ..Default::default()
    };
    let cases = [
        ("zero key", key([0; 96])),
        ("key of one", key(one)),
        ("key of p-1", key(p_minus_1)),
        ("key of p", key(p)),
        ("key above p", key([0xFF; 96])),
        (
            "padding too long",
            Tamper {
                pad_a: Some(600),
                ..Default::default()
            },
        ),
        (
            "bad verification constant",
            Tamper {
                vc: Some([1; 8]),
                ..Default::default()
            },
        ),
        (
            "huge padding claim",
            Tamper {
                pad_c_len: Some(0xFFFF),
                ..Default::default()
            },
        ),
        (
            "huge payload claim",
            Tamper {
                ia_len: Some(60_000),
                ..Default::default()
            },
        ),
        (
            "unknown torrent",
            Tamper {
                info_hash: Some(hash(99)),
                ..Default::default()
            },
        ),
    ];
    for (what, tamper) in cases {
        let (a, b) = pair().await;
        let (_, got) = within(what, async {
            tokio::join!(
                testing::initiate_tampered(a, hash(1), tamper),
                testing::accept(b, EncryptionPolicy::Prefer, vec![hash(1)]),
            )
        })
        .await;
        assert!(got.is_err(), "{what} was accepted");
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn garbage_never_hangs_or_panics() {
    for (i, len) in [20usize, 95, 96, 200, 700, 5000, 70_000]
        .into_iter()
        .enumerate()
    {
        // To our listening side.
        let (mut a, b) = pair().await;
        let junk = random(len, i as u64);
        let writer = async move {
            let _ = a.write_all(&junk).await;
            let _ = a.shutdown().await;
            a
        };
        let (_a, got) = within("garbage in", async {
            tokio::join!(
                writer,
                testing::accept_handshake(b, EncryptionPolicy::Prefer, vec![hash(1)]),
            )
        })
        .await;
        assert!(got.is_err(), "{len} random bytes were accepted");

        // To our connecting side, from a "peer" that answers with junk.
        let (a, mut b) = pair().await;
        let junk = random(len, 100 + i as u64);
        let peer = async move {
            let _ = b.write_all(&junk).await;
            let _ = b.shutdown().await;
            let mut sink = Vec::new();
            let _ = b.read_to_end(&mut sink).await;
        };
        let (got, _) = within("garbage out", async {
            tokio::join!(testing::initiate(a, hash(1), true, false, &[]), peer)
        })
        .await;
        assert!(got.is_err(), "{len} random bytes completed a handshake");
    }
    // A peer replying with a zero key.
    let (a, mut b) = pair().await;
    let peer = async move {
        let _ = b.write_all(&[0u8; 96]).await;
        let mut sink = [0u8; 4096];
        let _ = b.read(&mut sink).await;
        b
    };
    let (got, _b) = within("zero key", async {
        tokio::join!(testing::initiate(a, hash(1), true, false, &[]), peer)
    })
    .await;
    assert!(got.is_err());
}

// --- Real sessions -----------------------------------------------------------

async fn session(out: &Path, encryption: EncryptionPolicy) -> Arc<Session> {
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
            connect: Some(ConnectionOptions {
                encryption,
                ..Default::default()
            }),
            ..Default::default()
        },
    )
    .await
    .unwrap()
}

/// Seeds a 3 MiB file under `seed`'s policy and tries to fetch it under `leech`'s.
async fn transfer(seed: EncryptionPolicy, leech: EncryptionPolicy, wait: Duration) -> bool {
    let tmp = tempfile::tempdir().unwrap();
    let content = tmp.path().join("seed").join("content");
    std::fs::create_dir_all(&content).unwrap();
    let data = random(3 << 20, 7);
    std::fs::write(content.join("file.bin"), &data).unwrap();
    let created = create_torrent(
        &content,
        CreateTorrentOptions {
            name: Some("content"),
            trackers: vec![],
            piece_length: Some(256 * 1024),
        },
        &BlockingSpawner::new(2),
    )
    .await
    .unwrap();
    let bytes = created.as_bytes().unwrap();

    let seeder = session(&tmp.path().join("s"), seed).await;
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
    within("seed check", h.wait_until_completed())
        .await
        .unwrap();
    let port = seeder.listen_addr().unwrap().port();

    let leech_root = tmp.path().join("leech").join("content");
    let leecher = session(&tmp.path().join("l"), leech).await;
    let l = leecher
        .add_torrent(
            AddTorrent::from_bytes(bytes),
            Some(AddTorrentOptions {
                overwrite: true,
                output_folder: Some(leech_root.to_str().unwrap().into()),
                initial_peers: Some(vec![SocketAddr::new(Ipv4Addr::LOCALHOST.into(), port)]),
                ..Default::default()
            }),
        )
        .await
        .unwrap()
        .into_handle()
        .unwrap();
    let done = tokio::time::timeout(wait, l.wait_until_completed())
        .await
        .is_ok();
    if done {
        assert!(std::fs::read(leech_root.join("file.bin")).unwrap() == data);
    }
    seeder.stop().await;
    leecher.stop().await;
    done
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessions_transfer_when_their_policies_agree() {
    use EncryptionPolicy::*;
    let ok = Duration::from_secs(60);
    assert!(transfer(Require, Require, ok).await, "both require");
    assert!(
        transfer(Require, Prefer, ok).await,
        "prefer connects to a requiring seed"
    );
    assert!(
        transfer(Prefer, Require, ok).await,
        "require connects to a preferring seed"
    );
    // The leecher's encrypted attempt fails, then it reconnects plain.
    assert!(
        transfer(Disabled, Prefer, ok).await,
        "prefer falls back to plain"
    );
    assert!(transfer(Disabled, Disabled, ok).await, "both off");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn sessions_refuse_when_their_policies_clash() {
    use EncryptionPolicy::*;
    let briefly = Duration::from_secs(8);
    assert!(
        !transfer(Require, Disabled, briefly).await,
        "a plain peer got past require"
    );
    assert!(
        !transfer(Disabled, Require, briefly).await,
        "require fell back to plain"
    );
}
