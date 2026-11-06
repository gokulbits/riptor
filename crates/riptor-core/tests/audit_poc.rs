//! Proof-of-concept tests for the 2026-11-06 audit (audit/AUDIT-2026-11-06.md).
//!
//! These assert the DEFECTIVE behaviour that exists today, so they pass now and
//! must be inverted when each finding is fixed. Each one names its finding.

use std::collections::BTreeMap;
use std::path::Path;

use riptor_core::backup::{self, BackupPayload};
use riptor_core::{Engine, EngineConfig, RestoreOptions, Settings};

/// A1: a forged, password-less backup turns remote control on with authentication
/// bypassed for every address, and asks the router to forward the port.
///
/// `Settings::validate()` only checks the shape of `web_ui`, and `restore_backup`
/// applies it as soon as validation passes, so nothing stops this.
#[tokio::test]
async fn a1_forged_backup_enables_unauthenticated_remote_control() {
    let d = tempfile::tempdir().unwrap();
    let dl = d.path().join("downloads");
    std::fs::create_dir_all(&dl).unwrap();

    let mut hostile = Settings::default();
    hostile.download_dir = dl.clone();
    hostile.web_ui.enabled = true;
    hostile.web_ui.bypass_localhost = true;
    hostile.web_ui.bypass_subnets = true;
    hostile.web_ui.subnet_whitelist = vec!["0.0.0.0/0".into(), "::/0".into()];
    hostile.web_ui.allow_public = true;
    hostile.web_ui.upnp = true;
    hostile.web_ui.csrf_protection = false;

    // Nothing rejects it, and no password is needed to seal it.
    hostile.validate().expect("FINDING A1: hostile web_ui validates");
    let sealed = backup::seal(
        &BackupPayload {
            created: 0,
            app_version: "0.1.8".into(),
            settings: hostile,
            categories: BTreeMap::new(),
            torrents: vec![],
            statistics: Default::default(),
        },
        None,
    )
    .unwrap();
    assert!(
        !backup::is_password_protected(&sealed).unwrap(),
        "the app key seals it, so anyone can forge one"
    );

    let file = d.path().join("forged.riptorbackup");
    std::fs::write(&file, &sealed).unwrap();

    let engine = Engine::start(EngineConfig {
        data_dir: d.path().join("data"),
    })
    .await
    .unwrap();
    assert!(!engine.settings().web_ui.enabled, "off before the restore");

    let report = engine
        .restore_backup(
            &file,
            None,
            RestoreOptions {
                settings: true,
                torrents: false,
                statistics: false,
                resume_torrents: false,
            },
        )
        .await
        .unwrap();

    assert!(report.settings_restored, "the forged settings were applied");
    let w = engine.settings().web_ui;
    assert!(
        w.enabled && w.bypass_localhost && w.bypass_subnets && w.allow_public && w.upnp,
        "FINDING A1: forged backup enabled remote control with auth bypassed: {w:?}"
    );
    assert!(
        w.subnet_whitelist.iter().any(|s| s == "0.0.0.0/0"),
        "FINDING A1: every IPv4 client bypasses authentication"
    );
}

/// A2: `check_save_root` compares paths textually, so Windows spellings of a
/// protected folder other than the one it looked for are accepted. Asserts the
/// defect, so it fails once the check canonicalises its input.
#[test]
#[cfg(windows)]
fn a2_check_save_root_misses_windows_path_aliases() {
    use riptor_core::paths::check_save_root;

    // The spelling it does catch.
    assert!(
        check_save_root(Path::new(r"C:\Windows\System32"), None).is_err(),
        "the plain spelling is refused"
    );

    // Verbatim prefix: the same folder, accepted today.
    assert!(
        check_save_root(Path::new(r"\\?\C:\Windows\System32"), None).is_ok(),
        "FINDING A2 is fixed: the verbatim prefix is now refused"
    );

    // 8.3 short name, where the volume keeps them.
    if Path::new(r"C:\PROGRA~1").is_dir() {
        assert!(
            check_save_root(Path::new(r"C:\PROGRA~1\Riptor"), None).is_ok(),
            "FINDING A2 is fixed: the 8.3 short name is now refused"
        );
    }
}

/// A2 (Unix side): a symlinked save folder is not resolved either, so a link to
/// /etc passes. Asserts the defect.
#[test]
#[cfg(unix)]
fn a2_check_save_root_does_not_resolve_symlinks() {
    use riptor_core::paths::check_save_root;
    let d = tempfile::tempdir().unwrap();
    let link = d.path().join("downloads");
    std::os::unix::fs::symlink("/etc", &link).unwrap();
    assert!(
        check_save_root(&link, None).is_ok(),
        "FINDING A2 is fixed: a symlink to /etc is now refused"
    );
}

/// A3: a `.torrent` whose `piece length` is 0 divides by zero in
/// `librqbit_core::lengths::Lengths::new` (no guard before the `div_ceil`).
/// Release builds use `panic = "abort"`, so this aborts the process.
///
/// Runs the parse on its own thread and asserts it panicked, so the suite stays
/// green; invert it once the metadata is rejected with an error instead.
#[test]
fn a3_zero_piece_length_panics_instead_of_erroring() {
    // d8:announce..4:infod6:lengthi1e4:name1:x12:piece lengthi0e6:pieces20:<20 zeros>ee
    let mut b = b"d4:infod6:lengthi1e4:name1:x12:piece lengthi0e6:pieces20:".to_vec();
    b.extend_from_slice(&[0u8; 20]);
    b.extend_from_slice(b"ee");

    let bytes = b.clone();
    let outcome = std::thread::Builder::new()
        .spawn(move || {
            // Quiet: the panic hook would otherwise print a scary backtrace.
            let prev = std::panic::take_hook();
            if std::env::var_os("AUDIT_SHOW_PANIC").is_none() {
                std::panic::set_hook(Box::new(|_| {}));
            }
            let r = std::panic::catch_unwind(|| {
                librqbit::torrent_from_bytes(&bytes)
                    .map_err(|e| format!("{e:?}"))
                    .and_then(|t| t.info.data.validate().map(|_| ()).map_err(|e| format!("{e:?}")))
            });
            std::panic::set_hook(prev);
            r
        })
        .unwrap()
        .join()
        .unwrap();

    assert!(
        outcome.is_err(),
        "FINDING A3 is fixed: zero piece length now returns an error, not a panic ({:?})",
        outcome.map(|r| r.is_ok())
    );
}

/// A3, reachability: the same metadata through `Engine::preview`, the call every
/// add path uses (file, URL, magnet, RSS pick, remote-control upload, import).
/// Runs on its own thread; if it aborts rather than unwinding the test harness
/// reports a signal, which is the point.
#[tokio::test]
async fn a3_zero_piece_length_reaches_engine_preview() {
    let d = tempfile::tempdir().unwrap();
    let mut b = b"d4:infod6:lengthi1e4:name1:x12:piece lengthi0e6:pieces20:".to_vec();
    b.extend_from_slice(&[0u8; 20]);
    b.extend_from_slice(b"ee");
    let file = d.path().join("zero.torrent");
    std::fs::write(&file, &b).unwrap();

    let engine = Engine::start(EngineConfig {
        data_dir: d.path().join("data"),
    })
    .await
    .unwrap();

    let prev = std::panic::take_hook();
    std::panic::set_hook(Box::new(|_| {}));
    let got = std::panic::AssertUnwindSafe(engine.preview(riptor_core::Source::File(file)))
        .catch_unwind_here()
        .await;
    std::panic::set_hook(prev);

    assert!(
        got.is_err(),
        "FINDING A3 is fixed: preview now returns an error for zero piece length"
    );
    engine.shutdown().await.unwrap();
}

/// Small helper: catch_unwind around a future.
trait CatchUnwindHere: Sized {
    #[allow(async_fn_in_trait)]
    async fn catch_unwind_here(self) -> Result<(), ()>;
}

impl<F: std::future::Future> CatchUnwindHere for std::panic::AssertUnwindSafe<F> {
    async fn catch_unwind_here(self) -> Result<(), ()> {
        use futures::FutureExt;
        self.catch_unwind().await.map(|_| ()).map_err(|_| ())
    }
}
