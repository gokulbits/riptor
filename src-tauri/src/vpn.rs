//! App-only VPN: start OpenVPN through the privileged helper, drive it over
//! the management interface, and switch the engine's network mode.
//!
//! Only Riptor's sockets use the tunnel (they're pinned to its adapter); the
//! rest of the computer keeps its normal connection. A watchdog blocks the
//! engine the moment the tunnel or its adapter goes away.

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context};
use parking_lot::Mutex;
use riptor_core::{Engine, NetworkMode, VpnBinding};
use riptor_vpn::mgmt::{self, Line};
use riptor_vpn::natpmp;
use riptor_vpn::profiles::{Profile, ProfileStore};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::tcp::OwnedWriteHalf;
use tokio::net::TcpStream;
use tracing::{info, warn};

#[derive(Clone, Debug, Default, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Phase {
    #[default]
    Disconnected,
    Connecting,
    Connected,
    Reconnecting,
    Error,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct VpnStatus {
    pub phase: Phase,
    pub profile_id: Option<String>,
    pub profile_name: Option<String>,
    pub local_ip: Option<String>,
    pub forwarded_port: Option<u16>,
    pub message: Option<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct VpnPrefs {
    /// Ask Proton's gateway to forward a port (needs a P2P server, paid plan).
    pub port_forwarding: bool,
}

struct Conn {
    writer: tokio::sync::Mutex<OwnedWriteHalf>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
}

pub struct VpnManager {
    app: AppHandle,
    engine: Arc<Engine>,
    store: ProfileStore,
    dir: PathBuf,
    status: Mutex<VpnStatus>,
    conn: tokio::sync::Mutex<Option<Conn>>,
    prefs: Mutex<VpnPrefs>,
}

fn random_password() -> anyhow::Result<String> {
    const ABC: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(b.iter()
        .map(|x| ABC[*x as usize % ABC.len()] as char)
        .collect())
}

fn helper_path() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().context("no exe folder")?;
    let name = if cfg!(windows) {
        "riptor-vpn-helper.exe"
    } else {
        "riptor-vpn-helper"
    };
    let p = dir.join(name);
    anyhow::ensure!(
        p.is_file(),
        "the VPN helper is missing next to Riptor ({})",
        p.display()
    );
    Ok(p)
}

/// Is OpenVPN installed where the helper looks for it?
pub fn openvpn_installed() -> bool {
    if cfg!(windows) {
        let pf = std::env::var_os("ProgramFiles")
            .map(PathBuf::from)
            .unwrap_or_else(|| r"C:\Program Files".into());
        pf.join("OpenVPN").join("bin").join("openvpn.exe").is_file()
    } else {
        ["/usr/sbin/openvpn", "/usr/bin/openvpn"]
            .iter()
            .any(|p| std::path::Path::new(p).is_file())
    }
}

impl VpnManager {
    pub fn new(
        app: AppHandle,
        engine: Arc<Engine>,
        data_dir: &std::path::Path,
    ) -> anyhow::Result<Arc<Self>> {
        let dir = data_dir.join("vpn");
        let store = ProfileStore::new(data_dir)?;
        let prefs = std::fs::read(dir.join("prefs.json"))
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default();
        Ok(Arc::new(Self {
            app,
            engine,
            store,
            dir,
            status: Mutex::new(VpnStatus::default()),
            conn: tokio::sync::Mutex::new(None),
            prefs: Mutex::new(prefs),
        }))
    }

    pub fn store(&self) -> &ProfileStore {
        &self.store
    }

    pub fn status(&self) -> VpnStatus {
        self.status.lock().clone()
    }

    pub fn prefs(&self) -> VpnPrefs {
        self.prefs.lock().clone()
    }

    pub fn set_prefs(&self, p: VpnPrefs) -> anyhow::Result<()> {
        std::fs::write(self.dir.join("prefs.json"), serde_json::to_vec_pretty(&p)?)?;
        *self.prefs.lock() = p;
        Ok(())
    }

    fn set_status(&self, f: impl FnOnce(&mut VpnStatus)) {
        let s = {
            let mut s = self.status.lock();
            f(&mut s);
            s.clone()
        };
        let _ = self.app.emit("riptor://vpn", s);
    }

    /// Where the engine goes when the tunnel isn't usable.
    async fn fall_back(&self) {
        let mode = if self.engine.settings().vpn_required {
            NetworkMode::Blocked
        } else {
            NetworkMode::Normal
        };
        if let Err(e) = self.engine.set_network(mode).await {
            warn!("vpn fallback: {e:#}");
        }
    }

    pub async fn connect(self: &Arc<Self>, profile_id: &str) -> anyhow::Result<()> {
        self.disconnect().await;
        let profile: Profile = self.store.get(profile_id)?;
        if !profile.app_only_supported() {
            bail!("This WireGuard profile can't run app-only on Windows yet. Use Proton's OpenVPN config instead.");
        }
        if profile.needs_credentials && !self.store.has_credentials(profile_id) {
            bail!("Enter the profile's OpenVPN username and password first.");
        }
        if !openvpn_installed() {
            bail!("OpenVPN isn't installed. Install OpenVPN Community, then try again.");
        }
        self.set_status(|s| {
            *s = VpnStatus {
                phase: Phase::Connecting,
                profile_id: Some(profile.id.clone()),
                profile_name: Some(profile.name.clone()),
                message: Some("Waiting for administrator permission…".into()),
                ..Default::default()
            }
        });
        match self.start(&profile).await {
            Ok(()) => Ok(()),
            Err(e) => {
                self.set_status(|s| {
                    s.phase = Phase::Error;
                    s.message = Some(format!("{e:#}"));
                });
                self.fall_back().await;
                Err(e)
            }
        }
    }

    async fn start(self: &Arc<Self>, profile: &Profile) -> anyhow::Result<()> {
        let config = self.store.materialize(&profile.id)?;
        let port = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
            .local_addr()?
            .port();
        let password = random_password()?;
        // Hand-off files in our own folder; the helper copies them somewhere only admins can write.
        let run = self.dir.join("run");
        std::fs::create_dir_all(&run)?;
        let cfg_path = run.join("connect.ovpn");
        let pw_path = run.join("connect.pw");
        std::fs::write(&cfg_path, &config)?;
        std::fs::write(&pw_path, &password)?;
        let helper = helper_path()?;
        let args = vec![
            "openvpn".to_string(),
            cfg_path.to_string_lossy().into_owned(),
            port.to_string(),
            pw_path.to_string_lossy().into_owned(),
        ];
        let code =
            tokio::task::spawn_blocking(move || crate::elevate::run(&helper, &args)).await??;
        let _ = std::fs::remove_file(&cfg_path);
        let _ = std::fs::remove_file(&pw_path);
        match code {
            0 => {}
            3 => bail!("the profile was rejected by the VPN helper"),
            4 => bail!("OpenVPN isn't installed"),
            5 => bail!("OpenVPN failed to start (is another VPN using the adapter?)"),
            c => bail!("the VPN helper failed (code {c})"),
        }

        let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
            .await
            .context("connecting to OpenVPN")?;
        let (read, write) = stream.into_split();
        let me = self.clone();
        let profile_id = profile.id.clone();
        let reader =
            tokio::spawn(async move { me.drive(BufReader::new(read), password, profile_id).await });
        *self.conn.lock().await = Some(Conn {
            writer: tokio::sync::Mutex::new(write),
            tasks: vec![reader],
        });
        self.set_status(|s| s.message = Some("Connecting…".into()));
        Ok(())
    }

    async fn send(&self, line: &str) {
        if let Some(c) = self.conn.lock().await.as_ref() {
            let _ = c.writer.lock().await.write_all(line.as_bytes()).await;
        }
    }

    /// Read management lines until OpenVPN goes away.
    async fn drive(
        self: Arc<Self>,
        mut r: BufReader<tokio::net::tcp::OwnedReadHalf>,
        password: String,
        profile_id: String,
    ) {
        let mut line = String::new();
        let mut watchdog: Option<tokio::task::JoinHandle<()>> = None;
        loop {
            line.clear();
            match r.read_line(&mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            match mgmt::parse(&line) {
                Line::ManagementPassword => {
                    self.send(&format!("{password}\n")).await;
                    self.send("state on\n").await;
                    self.send("hold release\n").await;
                }
                Line::Hold => self.send("hold release\n").await,
                Line::NeedCredentials => match self
                    .store
                    .credentials(&profile_id)
                    .ok()
                    .and_then(|(u, p)| mgmt::credentials(&u, &p))
                {
                    Some([u, p]) => {
                        self.send(&u).await;
                        self.send(&p).await;
                    }
                    None => {
                        self.set_status(|s| {
                            s.phase = Phase::Error;
                            s.message =
                                Some("No OpenVPN username/password saved for this profile".into());
                        });
                        self.send("signal SIGTERM\n").await;
                    }
                },
                Line::AuthFailed => {
                    self.set_status(|s| {
                        s.phase = Phase::Error;
                        s.message = Some("The VPN rejected the username or password (Proton: use the OpenVPN credentials from your account page, not your login)".into());
                    });
                    self.send("signal SIGTERM\n").await;
                }
                Line::Fatal(m) => self.set_status(|s| {
                    s.phase = Phase::Error;
                    s.message = Some(m);
                }),
                Line::State {
                    state, local_ip, ..
                } => match (state.as_str(), local_ip) {
                    ("CONNECTED", Some(ip)) => {
                        if let Some(w) = watchdog.take() {
                            w.abort();
                        }
                        match self.on_connected(ip).await {
                            Ok(w) => watchdog = Some(w),
                            Err(e) => {
                                self.set_status(|s| {
                                    s.phase = Phase::Error;
                                    s.message = Some(format!("{e:#}"));
                                });
                                self.fall_back().await;
                            }
                        }
                    }
                    (
                        "RECONNECTING" | "EXITING" | "WAIT" | "AUTH" | "GET_CONFIG" | "ASSIGN_IP"
                        | "ADD_ROUTES" | "RESOLVE" | "TCP_CONNECT",
                        _,
                    ) => {
                        if matches!(self.status.lock().phase, Phase::Connected) {
                            self.fall_back().await;
                            self.set_status(|s| {
                                s.phase = Phase::Reconnecting;
                                s.message = Some(format!(
                                    "Tunnel {}: torrents are held until it's back",
                                    state.to_lowercase()
                                ));
                            });
                        }
                    }
                    _ => {}
                },
                Line::Other => {}
            }
        }
        if let Some(w) = watchdog {
            w.abort();
        }
        self.fall_back().await;
        self.set_status(|s| {
            if !matches!(s.phase, Phase::Error) {
                *s = VpnStatus::default();
            }
        });
        info!("VPN management connection closed");
    }

    /// Tunnel is up: pin the engine to it, start port forwarding and the watchdog.
    async fn on_connected(
        self: &Arc<Self>,
        ip: Ipv4Addr,
    ) -> anyhow::Result<tokio::task::JoinHandle<()>> {
        let adapter =
            riptor_core::network::adapter_for_ip(ip).context("couldn't find the VPN adapter")?;
        let mut forwarded = None;
        if self.prefs().port_forwarding {
            match natpmp::map(
                (natpmp::PROTON_GATEWAY, natpmp::PORT).into(),
                ip,
                natpmp::Proto::Tcp,
                0,
                60,
            )
            .await
            {
                Ok(m) => forwarded = Some(m.external_port),
                Err(e) => warn!("port forwarding: {e:#}"),
            }
        }
        self.engine
            .set_network(NetworkMode::Vpn(VpnBinding {
                adapter: adapter.clone(),
                local_ip: ip,
                forwarded_port: forwarded,
            }))
            .await?;
        self.set_status(|s| {
            s.phase = Phase::Connected;
            s.local_ip = Some(ip.to_string());
            s.forwarded_port = forwarded;
            s.message = None;
        });
        info!("VPN connected on {adapter} ({ip})");

        // Watchdog: adapter gone => block at once. Also renews the port mapping.
        let me = self.clone();
        Ok(tokio::spawn(async move {
            let mut tick = 0u32;
            loop {
                tokio::time::sleep(Duration::from_secs(1)).await;
                if riptor_core::network::adapter_for_ip(ip).is_none() {
                    warn!("VPN adapter disappeared; blocking");
                    me.fall_back().await;
                    me.set_status(|s| {
                        s.phase = Phase::Reconnecting;
                        s.message = Some("VPN adapter disappeared: torrents are held".into());
                    });
                    return;
                }
                tick += 1;
                if let (Some(port), true) = (forwarded, tick.is_multiple_of(45)) {
                    for proto in [natpmp::Proto::Tcp, natpmp::Proto::Udp] {
                        if let Err(e) = natpmp::map(
                            (natpmp::PROTON_GATEWAY, natpmp::PORT).into(),
                            ip,
                            proto,
                            port,
                            60,
                        )
                        .await
                        {
                            warn!("port forwarding renewal: {e:#}");
                        }
                    }
                }
            }
        }))
    }

    pub async fn disconnect(&self) {
        let conn = self.conn.lock().await.take();
        if let Some(c) = conn {
            let _ = c.writer.lock().await.write_all(b"signal SIGTERM\n").await;
            for t in c.tasks {
                t.abort();
            }
            self.fall_back().await;
        }
        self.set_status(|s| *s = VpnStatus::default());
    }
}
