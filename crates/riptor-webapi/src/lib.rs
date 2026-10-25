#![recursion_limit = "256"]
//! qBittorrent's Web API v2, enough for remote apps such as qBitController. Off by default; see
//! SECURITY.md for what it exposes and how it's guarded.

mod api;
pub mod auth;
mod ddns;
pub mod net;
mod qbt;

use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Duration;

use parking_lot::Mutex;
use riptor_core::{Engine, NetworkMode, WebUi};
use serde::Serialize;
use tokio::net::TcpListener;
use tracing::{info, warn};

pub use auth::{
    ddns_password, has_api_key, has_password, new_api_key, remove_api_key, set_ddns_password,
    set_password,
};

#[derive(Clone, Debug, Default, Serialize)]
pub struct Status {
    pub running: bool,
    pub port: Option<u16>,
    pub https: bool,
    /// SHA-256 of the certificate in use, for checking it in the app.
    pub fingerprint: Option<String>,
    pub error: Option<String>,
    /// The last dynamic DNS result.
    pub ddns: Option<String>,
    /// Addresses phones on the local network can use.
    pub addresses: Vec<String>,
}

pub struct Running {
    pub addr: SocketAddr,
    pub fingerprint: Option<String>,
    ddns: Arc<Mutex<Option<String>>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

impl Drop for Running {
    fn drop(&mut self) {
        for t in &self.tasks {
            t.abort();
        }
    }
}

fn vpn_ip(engine: &Engine) -> Option<Ipv4Addr> {
    match engine.network() {
        NetworkMode::Vpn(v) => Some(v.local_ip),
        _ => None,
    }
}

/// Where the settings say to listen.
pub fn bind_addr(cfg: &WebUi) -> SocketAddr {
    let ip = cfg
        .address
        .parse::<IpAddr>()
        .unwrap_or(Ipv4Addr::UNSPECIFIED.into());
    SocketAddr::new(ip, cfg.port)
}

fn certificate(data_dir: &Path, cfg: &WebUi) -> anyhow::Result<net::Cert> {
    match (&cfg.cert_file, &cfg.key_file) {
        (Some(cert), Some(key)) => net::load_pem(cert, key),
        _ => net::load_or_create_cert(data_dir),
    }
}

/// Starts the server on `bind`; it stops when the returned value is dropped.
pub async fn start(
    engine: Arc<Engine>,
    data_dir: &Path,
    cfg: &WebUi,
    bind: SocketAddr,
) -> anyhow::Result<Running> {
    start_with(engine, data_dir, cfg, bind, Arc::default()).await
}

/// Like `start`, with failed sign-ins remembered in `throttle` across restarts.
pub async fn start_with(
    engine: Arc<Engine>,
    data_dir: &Path,
    cfg: &WebUi,
    bind: SocketAddr,
    throttle: Arc<auth::Throttle>,
) -> anyhow::Result<Running> {
    cfg.validate()?;
    anyhow::ensure!(
        has_password(data_dir),
        "set a remote control password first"
    );
    let listener = TcpListener::bind(bind).await?;
    let (tls, fingerprint) = if cfg.https {
        let cert = certificate(data_dir, cfg)?;
        (Some(net::tls_acceptor(&cert)?), Some(cert.fingerprint))
    } else {
        (None, None)
    };
    let allow_public = cfg.allow_public;
    let gate_engine = engine.clone();
    let gate = net::Gate::new(listener, tls, move |remote, local| {
        net::refusal(remote.ip(), local.ip(), allow_public, vpn_ip(&gate_engine))
    })?;
    let addr = axum::serve::Listener::local_addr(&gate)?;
    let policy = auth::Policy {
        max_failures: cfg.max_failures,
        ban: Duration::from_secs(cfg.ban_secs.into()),
        idle: Duration::from_secs(cfg.session_timeout_secs.into()),
    };
    let ctx = Arc::new(api::Ctx::new(
        engine,
        auth::Auth::with_throttle(data_dir, &cfg.username, policy, throttle),
        data_dir.to_path_buf(),
        cfg.clone(),
    ));
    let app = api::router(ctx).into_make_service_with_connect_info::<net::Client>();
    let mut tasks = vec![tokio::spawn(async move {
        if let Err(e) = axum::serve(gate, app).await {
            warn!("remote control server stopped: {e}");
        }
    })];
    if cfg.upnp {
        match librqbit_upnp::UpnpPortForwarder::new(vec![addr.port()], None, None) {
            Ok(f) => tasks.push(tokio::spawn(async move {
                f.run_forever().await;
            })),
            Err(e) => warn!("couldn't ask the router to forward the remote control port: {e:#}"),
        }
    }
    let ddns = Arc::new(Mutex::new(None));
    if cfg.ddns.enabled {
        tasks.push(tokio::spawn(ddns::run(
            cfg.ddns.clone(),
            data_dir.to_path_buf(),
            ddns.clone(),
        )));
    }
    Ok(Running {
        addr,
        fingerprint,
        ddns,
        tasks,
    })
}

/// Where the server can be reached from the local network: the bound address, or every LAN one.
fn reachable(bound: IpAddr, vpn: Option<Ipv4Addr>) -> Vec<String> {
    if !bound.is_unspecified() {
        return vec![bound.to_string()];
    }
    riptor_core::lan_addresses()
        .into_iter()
        .filter(|ip| Some(*ip) != vpn)
        .map(|ip| ip.to_string())
        .collect()
}

/// Keeps the server in step with the settings and the secrets file for as long as Riptor runs.
pub async fn supervise(engine: Arc<Engine>, data_dir: PathBuf, status: Arc<Mutex<Status>>) {
    type Want = (WebUi, Option<std::time::SystemTime>);
    let mut running: Option<(Want, Running)> = None;
    let mut failed: Option<Want> = None;
    // Changing a setting restarts the server; bans stay.
    let throttle = Arc::new(auth::Throttle::default());
    loop {
        let cfg = engine.settings().web_ui;
        let stamp = std::fs::metadata(data_dir.join(auth::FILE))
            .and_then(|m| m.modified())
            .ok();
        let want = cfg.enabled.then(|| (cfg.clone(), stamp));
        let current = running.as_ref().map(|(w, _)| w.clone());
        if want != current && want != failed {
            running = None;
            let mut st = Status::default();
            if let Some(w) = want.clone() {
                match start_with(
                    engine.clone(),
                    &data_dir,
                    &w.0,
                    bind_addr(&w.0),
                    throttle.clone(),
                )
                .await
                {
                    Ok(r) => {
                        info!("remote control listening on {}", r.addr);
                        st = Status {
                            running: true,
                            port: Some(r.addr.port()),
                            https: w.0.https,
                            fingerprint: r.fingerprint.clone(),
                            addresses: reachable(r.addr.ip(), vpn_ip(&engine)),
                            ..Default::default()
                        };
                        running = Some((w, r));
                        failed = None;
                    }
                    Err(e) => {
                        warn!("remote control couldn't start: {e:#}");
                        st.error = Some(format!("{e:#}"));
                        failed = want;
                    }
                }
            } else {
                failed = None;
            }
            *status.lock() = st;
        }
        if let Some((_, r)) = &running {
            status.lock().ddns = r.ddns.lock().clone();
        }
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}
