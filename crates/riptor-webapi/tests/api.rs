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
    assert!(main["server_state"]["dl_info_data"].is_u64());
    assert!(main["server_state"]["up_info_data"].is_u64());
    let transfer = api.json("transfer/info").await;
    assert!(transfer["dl_info_data"].is_u64() && transfer["up_info_data"].is_u64());

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

    // Download order flips between in order and rarest first.
    assert_eq!(
        api.post("torrents/toggleSequentialDownload", &[("hashes", hash)])
            .await
            .0,
        200
    );
    wait_for(&api, "rarest first", |v| {
        v["torrents"][hash]["seq_dl"] == false
    })
    .await;

    // Refused on purpose, and unknown torrents.
    assert_eq!(api.post("app/shutdown", &[]).await.0, 403);
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
async fn add_answers_before_metadata_arrives() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path()).await;
    let data = tmp.path().join("data");
    let server = serve(&e, &data, false).await;
    let mut api = Api::new(server.addr, false);
    assert_eq!(api.login("admin", PASSWORD).await, (200, "Ok.".into()));

    let add = |form: reqwest::multipart::Form| {
        api.with_sid(api.http.post(format!("{}torrents/add", api.base)))
            .multipart(form)
            .send()
    };

    // No DHT and no trackers, so this magnet's metadata never comes.
    let magnet = format!("magnet:?xt=urn:btih:{}&dn=nowhere", "5a".repeat(20));
    let started = std::time::Instant::now();
    let r = add(reqwest::multipart::Form::new().text("urls", magnet))
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 200);
    assert_eq!(r.text().await.unwrap(), "Ok.");
    assert!(started.elapsed() < Duration::from_secs(5));

    // Bad input is still refused up front.
    let r = add(reqwest::multipart::Form::new().text("urls", "magnet:?xt=urn:btih:nothex"))
        .await
        .unwrap();
    assert_eq!(r.text().await.unwrap(), "Fails.");
    let junk = reqwest::multipart::Part::bytes(b"not a torrent".to_vec()).file_name("x.torrent");
    let r = add(reqwest::multipart::Form::new().part("torrents", junk))
        .await
        .unwrap();
    assert_eq!(r.text().await.unwrap(), "Fails.");
    let r = add(reqwest::multipart::Form::new()
        .text("urls", "c3".repeat(20))
        .text("savepath", "relative/folder"))
    .await
    .unwrap();
    assert_eq!(r.text().await.unwrap(), "Fails.");

    // A url-encoded form works too.
    let hash = "7b".repeat(20);
    let started = std::time::Instant::now();
    assert_eq!(
        api.post("torrents/add", &[("urls", hash.as_str())]).await,
        (200, "Ok.".into())
    );
    assert!(started.elapsed() < Duration::from_secs(5));
    assert_eq!(
        api.post("torrents/add", &[("urls", "not a link")]).await,
        (200, "Fails.".into())
    );

    // An uploaded file still lands in the list.
    let bytes = torrent_of(tmp.path(), "content", 9);
    let file = reqwest::multipart::Part::bytes(bytes).file_name("content.torrent");
    let r = add(reqwest::multipart::Form::new()
        .part("torrents", file)
        .text("stopped", "true"))
    .await
    .unwrap();
    assert_eq!(r.text().await.unwrap(), "Ok.");
    wait_for(&api, "the uploaded torrent", |v| {
        v["torrents"]
            .as_object()
            .is_some_and(|t| t.values().any(|t| t["name"] == "content"))
    })
    .await;
    // Its temp copy goes once the add is done.
    let leftovers = || {
        std::fs::read_dir(&data)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|f| f.file_name().to_string_lossy().starts_with("webui-upload-"))
            .count()
    };
    for _ in 0..50 {
        if leftovers() == 0 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    assert_eq!(leftovers(), 0);

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
    // Opening the address in a browser shows the web interface; the API itself still needs a sign-in.
    let root = api
        .http
        .get(format!("https://{}/", server.addr))
        .send()
        .await
        .unwrap();
    assert_eq!(root.status().as_u16(), 200);
    assert!(root.text().await.unwrap().contains("web/app.js"));
    assert_eq!(api.get("app/version").await.0, 403);

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

    // A name rebound to 127.0.0.1 doesn't get the bypass.
    let local = format!("http://{}/api/v2/", server.addr);
    let r = http
        .get(format!("{local}app/version"))
        .header("Host", "attacker.example")
        .header("Origin", "http://attacker.example")
        .send()
        .await
        .unwrap();
    assert_eq!(r.status().as_u16(), 403);
    let r = http
        .post(format!("{local}auth/login"))
        .header("Host", "attacker.example")
        .header("Origin", "http://attacker.example")
        .form(&[("username", "admin"), ("password", "wrong")])
        .send()
        .await
        .unwrap();
    assert!(r.headers().get("set-cookie").is_none());
    assert_eq!(r.text().await.unwrap(), "Fails.");
    drop(server);

    // A name listed in the server domains does.
    let cfg = WebUi {
        enabled: true,
        https: false,
        bypass_localhost: true,
        server_domains: "riptor.example.org".into(),
        ..WebUi::default()
    };
    let server = riptor_webapi::start(e.clone(), &data, &cfg, (Ipv4Addr::LOCALHOST, 0).into())
        .await
        .unwrap();
    let r = http
        .get(format!("http://{}/api/v2/app/version", server.addr))
        .header("Host", "riptor.example.org")
        .send()
        .await
        .unwrap();
    assert_eq!(r.text().await.unwrap(), "v5.0.0");
    drop(server);
    e.shutdown().await.unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn web_pages_search_rss_log_and_options() {
    let tmp = tempfile::tempdir().unwrap();
    let e = engine(tmp.path()).await;
    let data = tmp.path().join("data");
    riptor_webapi::set_password(&data, PASSWORD).unwrap();
    let hub = Arc::new(riptor_services::Hub::default());
    let rss = Arc::new(riptor_rss::Rss::open(&data));
    hub.set_rss(rss.clone());
    let http = riptor_core::input::http_client().unwrap();
    hub.set_search(riptor_search::SearchManager::new(&data, http).unwrap());
    let cfg = WebUi {
        enabled: true,
        https: false,
        ..WebUi::default()
    };
    let server = riptor_webapi::start_with(
        e.clone(),
        hub,
        &data,
        &cfg,
        (Ipv4Addr::LOCALHOST, 0).into(),
        Arc::default(),
    )
    .await
    .unwrap();
    let mut api = Api::new(server.addr, false);

    // The pages load without signing in, under a policy that allows only this server.
    let page = api
        .http
        .get(format!("http://{}/", server.addr))
        .send()
        .await
        .unwrap();
    let csp = page.headers()["content-security-policy"]
        .to_str()
        .unwrap()
        .to_string();
    assert!(
        csp.contains("script-src 'self'") && csp.contains("default-src 'none'"),
        "{csp}"
    );
    assert!(page.text().await.unwrap().contains("web/app.js"));
    for (path, code) in [
        ("web/app.js", 200),
        ("web/app.css", 200),
        ("web/nope.js", 404),
    ] {
        let r = api
            .http
            .get(format!("http://{}/{path}", server.addr))
            .send()
            .await
            .unwrap();
        assert_eq!(r.status().as_u16(), code, "{path}");
    }
    // Nothing behind them does.
    for path in ["rss/items", "search/plugins", "log/main", "app/preferences"] {
        assert_eq!(api.get(path).await.0, 403, "{path}");
    }
    assert_eq!(api.login("admin", PASSWORD).await.1, "Ok.");

    // RSS: feeds by name, rules in qBittorrent's shape, and what Riptor can't do refused.
    let url = "http://192.0.2.10/feed.xml";
    assert_eq!(
        api.post("rss/addFeed", &[("url", url), ("path", "Linux")])
            .await
            .0,
        200
    );
    let items = api.json("rss/items?withData=true").await;
    assert_eq!(items["Linux"]["url"], url);
    assert!(items["Linux"]["articles"].is_array());
    assert_eq!(
        api.post("rss/addFolder", &[("path", "Distros")]).await.0,
        409
    );
    let def = format!(
        r#"{{"mustContain":"debian","affectedFeeds":["{url}"],"torrentParams":{{"stopped":true}}}}"#
    );
    assert_eq!(
        api.post("rss/setRule", &[("ruleName", "Debian"), ("ruleDef", &def)])
            .await
            .0,
        200
    );
    let rules = api.json("rss/rules").await;
    assert_eq!(rules["Debian"]["mustContain"], "debian");
    assert_eq!(rules["Debian"]["torrentParams"]["stopped"], true);
    let episodes = r#"{"episodeFilter":"1x1-;"}"#;
    assert_eq!(
        api.post(
            "rss/setRule",
            &[("ruleName", "Shows"), ("ruleDef", episodes)]
        )
        .await
        .0,
        409
    );
    let into_data = serde_json::json!({ "savePath": data.join("session") }).to_string();
    assert_eq!(
        api.post(
            "rss/setRule",
            &[("ruleName", "Debian"), ("ruleDef", &into_data)]
        )
        .await
        .0,
        409
    );
    assert_eq!(
        api.post(
            "rss/renameRule",
            &[("ruleName", "Debian"), ("newRuleName", "Debian ISOs")]
        )
        .await
        .0,
        200
    );
    assert_eq!(
        api.post("rss/removeRule", &[("ruleName", "Debian ISOs")])
            .await
            .0,
        200
    );
    assert_eq!(api.json("rss/rules").await, serde_json::json!({}));
    assert_eq!(
        api.post(
            "rss/moveItem",
            &[("itemPath", "Linux"), ("destPath", "Distros")]
        )
        .await
        .0,
        200
    );
    assert_eq!(
        api.post("rss/removeItem", &[("path", "Distros")]).await.0,
        200
    );
    assert!(rss.view().feeds.is_empty());

    // Search engines: listed, switched off by name, and no plugin installs unless allowed.
    let plugins = api.json("search/plugins").await;
    assert!(plugins
        .as_array()
        .unwrap()
        .iter()
        .any(|p| p["name"] == "archive"));
    assert_eq!(
        api.post(
            "search/enablePlugin",
            &[("names", "archive"), ("enable", "false")]
        )
        .await
        .0,
        200
    );
    let plugins = api.json("search/plugins").await;
    let archive = plugins
        .as_array()
        .unwrap()
        .iter()
        .find(|p| p["name"] == "archive")
        .unwrap()
        .clone();
    assert_eq!(archive["enabled"], false);
    let (code, body) = api
        .post(
            "search/installPlugin",
            &[("sources", "https://example.org/x.py")],
        )
        .await;
    assert_eq!(code, 403, "{body}");
    assert_eq!(
        api.post(
            "search/start",
            &[("pattern", "debian"), ("category", "bogus")]
        )
        .await
        .0,
        409
    );
    assert_eq!(api.get("search/results?id=999").await.0, 404);

    // The log answers in qBittorrent's shape.
    assert!(api.json("log/main?last_known_id=-1").await.is_array());

    // Options: safe ones change, remote control's own can't.
    let prefs = api.json("app/preferences").await;
    assert_eq!(prefs["web_ui_port"], cfg.port);
    let change = r#"{"dl_limit":102400,"max_ratio_enabled":true,"max_ratio":2,"rss_refresh_interval":15,"web_ui_port":8080}"#;
    let (code, body) = api.post("app/setPreferences", &[("json", change)]).await;
    assert_eq!(code, 200, "{body}");
    let s = e.settings();
    assert_eq!(s.dl_limit_kib, Some(100));
    assert_eq!(s.seeding.max_ratio, Some(2.0));
    assert_eq!(rss.view().options.refresh_minutes, 15);
    for evil in [r#"{"web_ui_port":9999}"#, r#"{"bypass_local_auth":true}"#] {
        assert_eq!(
            api.post("app/setPreferences", &[("json", evil)]).await.0,
            403,
            "{evil}"
        );
    }
    let into_data = serde_json::json!({ "save_path": data.join("session") }).to_string();
    assert_eq!(
        api.post("app/setPreferences", &[("json", &into_data)])
            .await
            .0,
        409
    );
    drop(server);
    e.shutdown().await.unwrap();
}
