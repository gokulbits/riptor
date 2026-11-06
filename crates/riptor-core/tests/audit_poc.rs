//! Proof-of-concept tests for the 2026-11-06 audit (audit/AUDIT-2026-11-06.md).
//!
//! These assert the DEFECTIVE behaviour that exists today, so they pass now and
//! must be inverted when each finding is fixed. Each one names its finding.

use std::collections::BTreeMap;
use std::path::Path;

use riptor_core::backup::{self, BackupPayload};
use riptor_core::{Engine, EngineConfig, RestoreOptions, Settings};

/// A1: a forged, password-less backup must not be able to turn remote control
/// on or bypass its authentication. A password-protected one still may, since
/// it is not forgeable.
#[tokio::test]
async fn a1_forged_backup_cannot_enable_remote_control() {
    let d = tempfile::tempdir().unwrap();
    let dl = d.path().join("downloads");
    std::fs::create_dir_all(&dl).unwrap();

    let hostile = |dl: &Path| {
        let mut s = Settings {
            download_dir: dl.to_path_buf(),
            ..Default::default()
        };
        s.web_ui.enabled = true;
        s.web_ui.bypass_localhost = true;
        s.web_ui.bypass_subnets = true;
        s.web_ui.subnet_whitelist = vec!["0.0.0.0/0".into(), "::/0".into()];
        s.web_ui.allow_public = true;
        s.web_ui.upnp = true;
        s.web_ui.csrf_protection = false;
        s
    };
    let seal = |s: Settings, password: Option<&str>| {
        backup::seal(
            &BackupPayload {
                created: 0,
                app_version: "0.1.8".into(),
                settings: s,
                categories: BTreeMap::new(),
                torrents: vec![],
                statistics: Default::default(),
            },
            password,
        )
        .unwrap()
    };

    let forged = d.path().join("forged.riptorbackup");
    std::fs::write(&forged, seal(hostile(&dl), None)).unwrap();

    let engine = Engine::start(EngineConfig {
        data_dir: d.path().join("data"),
    })
    .await
    .unwrap();
    assert!(!engine.settings().web_ui.enabled, "off before the restore");

    let opts = RestoreOptions {
        settings: true,
        torrents: false,
        statistics: false,
        resume_torrents: false,
    };
    let report = engine.restore_backup(&forged, None, opts).await.unwrap();

    assert!(
        report.settings_restored,
        "the rest of the settings came back"
    );
    assert_eq!(
        engine.settings().download_dir,
        dl,
        "a harmless setting is still restored"
    );
    let w = engine.settings().web_ui;
    assert!(
        !w.enabled && !w.bypass_localhost && !w.bypass_subnets && !w.allow_public && !w.upnp,
        "a forged backup must not open remote control: {w:?}"
    );
    assert!(
        w.subnet_whitelist.is_empty() && w.csrf_protection,
        "nor weaken it: {w:?}"
    );
    assert!(
        report
            .skipped
            .iter()
            .any(|s| s.contains("remote control settings")),
        "the user is told: {:?}",
        report.skipped
    );

    // The owner's own password-protected backup is not forgeable, so it still applies.
    let mine = d.path().join("mine.riptorbackup");
    std::fs::write(&mine, seal(hostile(&dl), Some("correct horse"))).unwrap();
    engine
        .restore_backup(&mine, Some("correct horse".into()), opts)
        .await
        .unwrap();
    assert!(
        engine.settings().web_ui.enabled,
        "a password-protected backup still restores remote control"
    );
}

/// A2: `check_save_root` used to compare paths textually, so other spellings of
/// a protected folder got through. Every spelling must now be refused.
#[test]
#[cfg(windows)]
fn a2_windows_path_aliases_cannot_reach_protected_folders() {
    use riptor_core::paths::check_save_root;

    for spelling in [
        r"C:\Windows\System32",
        r"\\?\C:\Windows\System32",
        r"\\.\C:\Windows\System32",
        r"C:\Windows\..\Windows\System32",
        r"C:/Windows/System32",
    ] {
        assert!(
            check_save_root(Path::new(spelling), None).is_err(),
            "accepted {spelling}"
        );
    }

    // 8.3 short name, where the volume keeps them.
    if Path::new(r"C:\PROGRA~1").is_dir() {
        assert!(
            check_save_root(Path::new(r"C:\PROGRA~1\Riptor"), None).is_err(),
            "accepted the 8.3 short name for Program Files"
        );
    }

    // An administrative share is the whole drive.
    assert!(check_save_root(Path::new(r"\\127.0.0.1\C$\Windows"), None).is_err());

    // A normal folder, and a normal network share, still work.
    let ok = std::env::temp_dir().join("riptor-a2-ok");
    assert!(check_save_root(&ok, None).is_ok(), "{ok:?} refused");
    assert!(check_save_root(Path::new(r"\\nas\media\torrents"), None).is_ok());
}

/// A2 (Unix side): a symlinked save folder must be resolved before the check.
#[test]
#[cfg(unix)]
fn a2_symlinked_save_folders_cannot_reach_protected_folders() {
    use riptor_core::paths::check_save_root;
    let d = tempfile::tempdir().unwrap();

    let direct = d.path().join("to-etc");
    std::os::unix::fs::symlink("/etc", &direct).unwrap();
    assert!(
        check_save_root(&direct, None).is_err(),
        "a symlink to /etc was accepted"
    );

    // The link is a parent of the folder, which does not exist yet.
    let parent = d.path().join("to-usr");
    std::os::unix::fs::symlink("/usr", &parent).unwrap();
    assert!(
        check_save_root(&parent.join("share").join("dl"), None).is_err(),
        "a folder under a symlink to /usr was accepted"
    );

    // A plain folder is still fine.
    let fine = d.path().join("downloads");
    std::fs::create_dir_all(&fine).unwrap();
    assert!(check_save_root(&fine, None).is_ok());
}

/// A3: a `.torrent` whose `piece length` is 0 used to divide by zero in
/// `librqbit_core::lengths::Lengths::new`. Both the early check in
/// `riptor_core::input` and the vendored session's own bound must refuse it,
/// and it must be an error, never a panic.
#[tokio::test]
async fn a3_zero_piece_length_is_refused_not_a_panic() {
    let d = tempfile::tempdir().unwrap();
    let engine = Engine::start(EngineConfig {
        data_dir: d.path().join("data"),
    })
    .await
    .unwrap();

    for (name, piece_len) in [("zero", "0"), ("huge", "4294967295"), ("tiny", "1024")] {
        let mut b = format!("d4:infod6:lengthi1e4:name1:x12:piece lengthi{piece_len}e6:pieces20:")
            .into_bytes();
        b.extend_from_slice(&[0u8; 20]);
        b.extend_from_slice(b"ee");

        assert!(
            riptor_core::input::check_metadata(&b).is_err(),
            "{name}: check_metadata accepted it"
        );

        let file = d.path().join(format!("{name}.torrent"));
        std::fs::write(&file, &b).unwrap();
        let err = engine
            .preview(riptor_core::Source::File(file))
            .await
            .expect_err(&format!("{name}: preview accepted it"));
        assert!(
            format!("{err:#}").contains("piece length"),
            "{name}: unhelpful error {err:#}"
        );
    }

    // A normal piece length still works.
    assert!(riptor_core::input::check_metadata(&sane_torrent()).is_ok());
    engine.shutdown().await.unwrap();
}

/// A3: parsing was always safe; it was validating that divided by zero. Metadata
/// peers send over BEP 9 never passes through `riptor_core::input`, so the
/// vendored session carries its own bound at both of its validate sites.
#[test]
fn a3_parsing_is_safe_validating_was_not() {
    let mut b = b"d4:infod6:lengthi1e4:name1:x12:piece lengthi0e6:pieces20:".to_vec();
    b.extend_from_slice(&[0u8; 20]);
    b.extend_from_slice(b"ee");
    let parsed = librqbit::torrent_from_bytes(&b).expect("parses");
    assert_eq!(parsed.info.data.piece_length, 0);
}

fn sane_torrent() -> Vec<u8> {
    let mut b = b"d4:infod6:lengthi1e4:name1:x12:piece lengthi16384e6:pieces20:".to_vec();
    b.extend_from_slice(&[0u8; 20]);
    b.extend_from_slice(b"ee");
    b
}

/// A4: `move_files` used to check the destination and then let `fs::rename`
/// replace it. The move itself must refuse, with no check-then-act window.
#[test]
fn a4_moving_never_overwrites_the_destination() {
    use riptor_core::relocate::move_files;

    let d = tempfile::tempdir().unwrap();
    let from = d.path().join("src.bin");
    let to = d.path().join("sub").join("dst.bin");
    std::fs::create_dir_all(to.parent().unwrap()).unwrap();
    std::fs::write(&from, b"new").unwrap();
    std::fs::write(&to, b"precious").unwrap();

    let err = move_files(&[(from.clone(), to.clone())]).expect_err("overwrote the destination");
    assert!(
        format!("{err:#}").contains("won't overwrite"),
        "unhelpful error: {err:#}"
    );
    assert_eq!(
        std::fs::read(&to).unwrap(),
        b"precious",
        "destination changed"
    );
    assert!(from.is_file(), "source was removed anyway");

    // With the destination gone it moves, and the source is cleaned up.
    std::fs::remove_file(&to).unwrap();
    move_files(&[(from.clone(), to.clone())]).unwrap();
    assert_eq!(std::fs::read(&to).unwrap(), b"new");
    assert!(!from.exists());
}
