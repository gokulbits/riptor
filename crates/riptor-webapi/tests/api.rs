//! The Web API end to end over loopback, the way qBitController uses it.

use std::net::{Ipv4Addr, SocketAddr};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use riptor_core::create::{create, CreateOptions, Progress};
use riptor_core::{Engine, EngineConfig, Settings, WebUi};
use serde_json::Value;

const PASSWORD: &str = "correct horse battery";

async fn engine(dir: &Path) -> Arc<Engine> {
    let settings = Settings {
        download_dir: dir.join("downloads"),
        dht: false,
        lsd: false,
        upnp: false,
        listen_address: Some(Ipv4Addr::LOCALHOST.into()),
        ..Settings::default()
    };
    riptor_core::state_file::save(&dir.join("data").join("settings.json"), &settings).unwrap();
    Engine::start(EngineConfig {
        data_dir: dir.join("data"),
    })
    .await
    .unwrap()
}

fn torrent_of(dir: &Path, name: &str, byte: u8) -> Vec<u8> {
    let content = dir.join("downloads").join(name);
    std::fs::create_dir_all(&content).unwrap();
    std::fs::write(content.join("a.bin"), vec![byte; 40_000]).unwrap();
    create(
        &content,
        &CreateOptions {
            piece_length: Some(16 * 1024),
            ..Default::default()
        },
        &Progress::default(),
    )
    .unwrap()
}

struct Api {
    base: String,
    http: reqwest::Client,
    sid: Option<String>,
}

impl Api {
    fn new(addr: SocketAddr, https: bool) -> Self {
        let scheme = if https { "https" } else { "http" };
        Self {
            base: format!("{scheme}://{addr}/api/v2/"),
            http: reqwest::Client::builder()
                .danger_accept_invalid_certs(true)
                .timeout(Duration::from_secs(20))
                .build()
                .unwrap(),
            sid: None,
        }
    }

    fn with_sid(&self, r: reqwest::RequestBuilder) -> reqwest::RequestBuilder {
        match &self.sid {
            Some(sid) => r.header("Cookie", format!("SID={sid}")),
            None => r,
        }
    }

    async fn login(&mut self, user: &str, pass: &str) -> (u16, String) {
        let r = self
            .http
            .post(format!("{}auth/login", self.base))
            .form(&[("username", user), ("password", pass)])
            .send()
            .await
            .unwrap();
        let status = r.status().as_u16();
        if let Some(c) = r.headers().get("set-cookie") {
            let c = c.to_str().unwrap();
            assert!(
                c.contains("HttpOnly") && c.contains("SameSite=Strict"),
                "{c}"
            );
            self.sid = c
                .strip_prefix("SID=")
                .and_then(|v| v.split(';').next())
                .map(String::from);
        }
        (status, r.text().await.unwrap())
    }

    async fn get(&self, path: &str) -> (u16, String) {
        let r = self
            .with_sid(self.http.get(format!("{}{path}", self.base)))
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    async fn post(&self, path: &str, form: &[(&str, &str)]) -> (u16, String) {
        let r = self
            .with_sid(self.http.post(format!("{}{path}", self.base)))
            .form(form)
            .send()
            .await
            .unwrap();
        (r.status().as_u16(), r.text().await.unwrap())
    }

    async fn json(&self, path: &str) -> Value {
        let (code, body) = self.get(path).await;
        assert_eq!(code, 200, "{path}: {body}");
        serde_json::from_str(&body).unwrap()
    }
}

async fn serve(engine: &Arc<Engine>, data: &Path, https: bool) -> riptor_webapi::Running {
    riptor_webapi::set_password(data, PASSWORD).unwrap();
    let cfg = WebUi {
        enabled: true,
        https,
        ..WebUi::default()
    };
    riptor_webapi::start(engine.clone(), data, &cfg, (Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap()
}

async fn wait_for(api: &Api, what: &str, pred: impl Fn(&Value) -> bool) -> Value {
    for _ in 0..100 {
        let v = api.json("sync/maindata").await;
        if pred(&v) {
            return v;
        }
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    panic!("timed out waiting for {what}");
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn controls_riptor_like_a_remote_app() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path()).await;
    let data = tmp.path().join("data");
    let server = serve(&e, &data, false).await;
    let mut api = Api::new(server.addr, false);

    // Nothing without signing in.
    assert_eq!(api.get("app/version").await.0, 403);
    assert_eq!(api.login("admin", "wrong").await, (200, "Fails.".into()));
    assert_eq!(api.login("admin", PASSWORD).await, (200, "Ok.".into()));
    assert_eq!(api.get("app/version").await, (200, "v5.0.0".into()));

    // Adding a torrent file the way the app uploads it.
    let bytes = torrent_of(tmp.path(), "content", 7);
    let form = reqwest::multipart::Form::new()
        .part(
            "torrents",
            reqwest::multipart::Part::bytes(bytes)
                .file_name("content.torrent")
                .mime_str("application/x-bittorrent")
                .unwrap(),
        )
        .text(
            "savepath",
            tmp.path().join("downloads").to_string_lossy().into_owned(),
        )
        .text("stopped", "true")
        .text("tags", "linux,iso");
    let r = api
        .with_sid(api.http.post(format!("{}torrents/add", api.base)))
        .multipart(form)
        .send()
        .await
        .unwrap();
    assert_eq!(r.text().await.unwrap(), "Ok.");

    let main = wait_for(&api, "the torrent", |v| {
        v["torrents"].as_object().is_some_and(|t| !t.is_empty())
    })
    .await;
    assert_eq!(main["full_update"], true);
    let (hash, t) = main["torrents"].as_object().unwrap().iter().next().unwrap();
    assert_eq!(t["name"], "content");
    assert_eq!(t["tags"], "linux, iso");
    assert!(t["magnet_uri"].as_str().unwrap().starts_with("magnet:?"));
    assert!(main["server_state"]["dl_info_speed"].is_u64());

    // Details the app opens.
    let files = api.json(&format!("torrents/files?hash={hash}")).await;
    assert_eq!(files[0]["name"], "a.bin");
    let props = api.json(&format!("torrents/properties?hash={hash}")).await;
    assert_eq!(props["save_path"], t["save_path"]);
    assert!(api
        .json(&format!("torrents/pieceStates?hash={hash}"))
        .await
        .is_array());
    let info = api.json("torrents/info").await;
    assert_eq!(info.as_array().unwrap().len(), 1);

    // Actions are POST only and change the torrent.
    assert_eq!(
        api.get(&format!("torrents/start?hashes={hash}")).await.0,
        405
    );
    assert_eq!(
        api.post("torrents/createCategory", &[("category", "OS")])
            .await
            .0,
        200
    );
    assert_eq!(
        api.post(
            "torrents/setCategory",
            &[("hashes", hash), ("category", "OS")]
        )
        .await
        .0,
        200
    );
    assert_eq!(
        api.post("torrents/removeTags", &[("hashes", hash), ("tags", "iso")])
            .await
            .0,
        200
    );
    assert_eq!(
        api.post(
            "torrents/setShareLimits",
            &[
                ("hashes", hash),
                ("ratioLimit", "1.5"),
                ("seedingTimeLimit", "-1")
            ]
        )
        .await
        .0,
        200
    );
    assert_eq!(
        api.post(
            "torrents/setUploadLimit",
            &[("hashes", hash), ("limit", "102400")]
        )
        .await
        .0,
        200
    );
    let t = wait_for(&api, "the changes", |v| {
        v["torrents"][hash]["category"] == "OS"
    })
    .await;
    let t = &t["torrents"][hash];
    assert_eq!(t["tags"], "linux");
    assert_eq!(t["ratio_limit"], 1.5);
    assert_eq!(t["seeding_time_limit"], -1);
    assert_eq!(t["up_limit"], 102400);
    assert!(api.json("torrents/categories").await["OS"].is_object());

    // Refused on purpose, and unknown torrents.
    assert_eq!(api.post("app/shutdown", &[]).await.0, 403);
    assert_eq!(
        api.post("torrents/toggleSequentialDownload", &[("hashes", hash)])
            .await
            .0,
        409
    );
    assert_eq!(
        api.post("torrents/stop", &[("hashes", &"ab".repeat(20))])
            .await
            .0,
        404
    );

    // Another site can't drive it from a browser.
    let r = api
        .with_sid(api.http.post(format!("{}torrents/stop", api.base)))
        .header("Origin", "https://evil.example")
        .form(&[("hashes", hash.as_str())])
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 401);

    assert_eq!(
        api.post(
            "torrents/delete",
            &[("hashes", hash), ("deleteFiles", "false")]
        )
        .await
        .0,
        200
    );
    wait_for(&api, "the removal", |v| {
        v["torrents"].as_object().is_some_and(|t| t.is_empty())
    })
    .await;

    // Logging out ends the session.
    assert_eq!(api.post("auth/logout", &[]).await.0, 200);
    assert_eq!(api.get("app/version").await.0, 403);
    drop(server);
    e.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn https_and_bans() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path()).await;
    let data = tmp.path().join("data");
    let server = serve(&e, &data, true).await;
    assert!(server.fingerprint.is_some());

    // Plain HTTP to the HTTPS port gets nowhere.
    let plain = Api::new(server.addr, false);
    assert!(plain
        .http
        .get(format!("{}app/version", plain.base))
        .send()
        .await
        .is_err());

    let mut api = Api::new(server.addr, true);
    for _ in 0..riptor_webapi::auth::MAX_FAILURES {
        assert_eq!(api.login("admin", "nope").await, (200, "Fails.".into()));
    }
    // Banned: even the right password is refused now.
    assert_eq!(api.login("admin", PASSWORD).await.0, 403);
    drop(server);
    e.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn refuses_to_start_without_a_password() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path()).await;
    let cfg = WebUi {
        enabled: true,
        ..WebUi::default()
    };
    let r = riptor_webapi::start(
        e.clone(),
        &tmp.path().join("data"),
        &cfg,
        (Ipv4Addr::LOCALHOST, 0).into(),
    )
    .await;
    assert!(r.is_err());
    e.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn options_from_the_settings() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path()).await;
    let data = tmp.path().join("data");
    riptor_webapi::set_password(&data, PASSWORD).unwrap();
    let key = riptor_webapi::new_api_key(&data).unwrap();
    let cfg = WebUi {
        enabled: true,
        https: false,
        max_failures: 2,
        server_domains: "riptor.lan;*.home.example".into(),
        custom_headers_enabled: true,
        custom_headers: "X-Test: yes".into(),
        reverse_proxy: true,
        trusted_proxies: "127.0.0.1".into(),
        ..WebUi::default()
    };
    let server = riptor_webapi::start(e.clone(), &data, &cfg, (Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap();
    let base = format!("http://{}/api/v2/", server.addr);
    let http = reqwest::Client::new();
    let get = |path: &str| http.get(format!("{base}{path}"));

    // The API key works without logging in; a wrong one doesn't.
    let r = get("app/version").bearer_auth(&key).send().await.unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(r.headers()["x-frame-options"], "SAMEORIGIN");
    assert_eq!(r.headers()["x-test"], "yes");
    assert_eq!(
        get("app/version")
            .bearer_auth("rpt_nope")
            .send()
            .await
            .unwrap()
            .status()
            .as_u16(),
        403
    );

    // Host validation: listed names and IP addresses pass, others don't.
    for (host, code) in [
        ("riptor.lan", 200),
        ("box.home.example", 200),
        ("evil.example", 401),
    ] {
        let r = get("app/version")
            .bearer_auth(&key)
            .header("Host", host)
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), code, "{host}");
    }

    // Behind the trusted proxy, bans follow the real client, not the proxy.
    let login = |client: &'static str, pass: &'static str| {
        http.post(format!("{base}auth/login"))
            .header("X-Forwarded-For", client)
            .form(&[("username", "admin"), ("password", pass)])
            .send()
    };
    for _ in 0..2 {
        login("198.51.100.7", "wrong").await.unwrap();
    }
    assert_eq!(
        login("198.51.100.7", PASSWORD)
            .await
            .unwrap()
            .status()
            .as_u16(),
        403
    );
    assert_eq!(
        login("198.51.100.8", PASSWORD)
            .await
            .unwrap()
            .text()
            .await
            .unwrap(),
        "Ok."
    );
    drop(server);

    // Bypassing authentication for localhost.
    let cfg = WebUi {
        enabled: true,
        https: false,
        bypass_localhost: true,
        ..WebUi::default()
    };
    let server = riptor_webapi::start(e.clone(), &data, &cfg, (Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap();
    let r = http
        .get(format!("http://{}/api/v2/app/version", server.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(r.text().await.unwrap(), "v5.0.0");
    drop(server);
    e.shutdown().await.unwrap();
}
