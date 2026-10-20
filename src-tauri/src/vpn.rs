//! App-only VPN through the elevated helper.

use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, Context};
use parking_lot::Mutex;
use riptor_core::{Engine, NetworkMode, VpnBinding};
use riptor_vpn::mgmt::{self, Line};
use riptor_vpn::natpmp;
use riptor_vpn::profiles::{Kind, Profile, ProfileStore};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader};
#[cfg(not(target_os = "linux"))]
use tokio::net::TcpStream;
use tracing::{debug, info, warn};

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
    /// NAT-PMP, if the provider supports it.
    pub port_forwarding: bool,
}

type Reader = BufReader<Box<dyn AsyncRead + Send + Unpin>>;
type Writer = Box<dyn AsyncWrite + Send + Unpin>;

struct Conn {
    /// OpenVPN's management interface.
    writer: Option<tokio::sync::Mutex<Writer>>,
    tasks: Vec<tokio::task::JoinHandle<()>>,
    /// The WireGuard helper; closing its stdin takes the tunnel down.
    keeper: Option<tokio::process::Child>,
}

impl Conn {
    async fn close(mut self) {
        if let Some(w) = &self.writer {
            let _ = w.lock().await.write_all(b"signal SIGTERM\n").await;
        }
        for t in self.tasks.drain(..) {
            t.abort();
        }
        if let Some(mut child) = self.keeper.take() {
            drop(child.stdin.take());
            let _ = tokio::time::timeout(Duration::from_secs(15), child.wait()).await;
        }
    }
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

/// One at a time: OpenVPN 2.7 can drop or mix up commands sent together.
#[derive(Default)]
struct Commands {
    queue: std::collections::VecDeque<String>,
    waiting: bool,
}

impl Commands {
    fn push(&mut self, c: String) {
        self.queue.push_back(c);
    }

    fn on_line(&mut self, line: &str) {
        if line.starts_with("SUCCESS:") || line.starts_with("ERROR:") {
            self.waiting = false;
        }
    }

    fn next(&mut self) -> Option<String> {
        if self.waiting {
            return None;
        }
        let c = self.queue.pop_front()?;
        self.waiting = true;
        Some(c)
    }
}

/// A new address is unusable (WSAEADDRNOTAVAIL) until duplicate-address detection ends.
async fn wait_until_bindable(ip: Ipv4Addr) -> anyhow::Result<()> {
    for _ in 0..40 {
        if std::net::UdpSocket::bind((ip, 0)).is_ok() {
            return Ok(());
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    bail!("the VPN address {ip} never became usable")
}

/// Includes OpenVPN's own retries across the profile's servers.
const CONNECT_TIMEOUT: Duration = Duration::from_secs(120);
const MAX_MGMT_LINE: usize = 64 * 1024;

/// Like `read_line`, except OpenVPN's `ENTER PASSWORD:` prompt has no newline.
async fn next_line<R: tokio::io::AsyncBufRead + Unpin>(
    r: &mut R,
    line: &mut String,
) -> std::io::Result<usize> {
    let mut bytes = Vec::new();
    loop {
        let buf = r.fill_buf().await?;
        if buf.is_empty() {
            break;
        }
        match buf.iter().position(|b| *b == b'\n') {
            Some(i) => {
                bytes.extend_from_slice(&buf[..=i]);
                r.consume(i + 1);
                break;
            }
            None => {
                let n = buf.len();
                bytes.extend_from_slice(buf);
                r.consume(n);
            }
        }
        if bytes.starts_with(b"ENTER PASSWORD:") {
            break;
        }
        if bytes.len() > MAX_MGMT_LINE {
            return Err(std::io::Error::other("management line too long"));
        }
    }
    line.push_str(&String::from_utf8_lossy(&bytes));
    Ok(bytes.len())
}

fn random_password() -> anyhow::Result<String> {
    const ABC: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789";
    let mut b = [0u8; 32];
    getrandom::fill(&mut b).map_err(|e| anyhow::anyhow!("{e}"))?;
    Ok(b.iter()
        .map(|x| ABC[*x as usize % ABC.len()] as char)
        .collect())
}

/// UAC shows this file name, so it reads as a product name.
const HELPER_EXE_WINDOWS: &str = "Riptor VPN Service.exe";

fn helper_path() -> anyhow::Result<PathBuf> {
    let exe = std::env::current_exe()?;
    let dir = exe.parent().context("no exe folder")?;
    let mut p = dir.join(if cfg!(windows) {
        HELPER_EXE_WINDOWS
    } else {
        "riptor-vpn-helper"
    });
    // Dev builds: cargo's own output name.
    if cfg!(all(windows, debug_assertions)) && !p.is_file() {
        p = dir.join("riptor-vpn-helper.exe");
    }
    anyhow::ensure!(
        p.is_file(),
        "the VPN helper is missing next to Riptor ({})",
        p.display()
    );
    Ok(p)
}

pub fn openvpn_installed() -> bool {
    #[cfg(windows)]
    {
        // Same lookup as the helper; never PATH.
        let hklm = winreg::RegKey::predef(winreg::enums::HKEY_LOCAL_MACHINE);
        let from_registry = hklm.open_subkey(r"SOFTWARE\OpenVPN").ok().and_then(|k| {
            k.get_value::<String, _>("exe_path").ok().or_else(|| {
                k.get_value::<String, _>("")
                    .ok()
                    .map(|d| format!(r"{d}\bin\openvpn.exe"))
            })
        });
        from_registry
            .into_iter()
            .chain(Some(
                r"C:\Program Files\OpenVPN\bin\openvpn.exe".to_string(),
            ))
            .any(|p| std::path::Path::new(&p).is_file())
    }
    #[cfg(not(windows))]
    {
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
            bail!("This WireGuard profile can't run app-only on Windows yet. Use the provider's OpenVPN config instead.");
        }
        if profile.kind == Kind::OpenVpn {
            if profile.needs_credentials && !self.store.has_credentials(profile_id) {
                bail!("Enter the profile's OpenVPN username and password first.");
            }
            if !openvpn_installed() {
                bail!("OpenVPN isn't installed. Install OpenVPN Community, then try again.");
            }
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
        let started = match profile.kind {
            Kind::OpenVpn => self.start(&profile).await,
            #[cfg(target_os = "linux")]
            Kind::WireGuard => self.start_wireguard(&profile).await,
            #[cfg(not(target_os = "linux"))]
            Kind::WireGuard => Err(anyhow::anyhow!("WireGuard isn't available on this system")),
        };
        match started {
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
        // Linux: a Unix socket only this user can open. Elsewhere: loopback TCP.
        let port = std::net::TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?
            .local_addr()?
            .port();
        let endpoint = if cfg!(target_os = "linux") {
            "unix".to_string()
        } else {
            port.to_string()
        };
        let password = random_password()?;
        // The helper copies these into an admin-only folder before use.
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
            endpoint,
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

        let (read, write): (Box<dyn AsyncRead + Send + Unpin>, Writer) = {
            #[cfg(target_os = "linux")]
            {
                let uid = rustix::process::getuid().as_raw();
                let path = format!("/run/riptor-vpn-sock/{uid}/mgmt.sock");
                let stream = tokio::net::UnixStream::connect(&path)
                    .await
                    .context("connecting to OpenVPN")?;
                let (r, w) = stream.into_split();
                (Box::new(r), Box::new(w))
            }
            #[cfg(not(target_os = "linux"))]
            {
                let stream = TcpStream::connect((Ipv4Addr::LOCALHOST, port))
                    .await
                    .context("connecting to OpenVPN")?;
                let (r, w) = stream.into_split();
                (Box::new(r), Box::new(w))
            }
        };
        // The password prompt arrives immediately, so the writer must exist first.
        let mut conn = self.conn.lock().await;
        *conn = Some(Conn {
            writer: Some(tokio::sync::Mutex::new(write)),
            tasks: Vec::new(),
            keeper: None,
        });
        let me = self.clone();
        let profile_id = profile.id.clone();
        let reader =
            tokio::spawn(async move { me.drive(BufReader::new(read), password, profile_id).await });
        if let Some(c) = conn.as_mut() {
            c.tasks.push(reader);
        }
        let me = self.clone();
        let timeout = tokio::spawn(async move {
            tokio::time::sleep(CONNECT_TIMEOUT).await;
            if matches!(me.status.lock().phase, Phase::Connecting) {
                warn!("VPN connect timed out");
                me.set_status(|s| {
                    s.phase = Phase::Error;
                    s.message = Some(
                        "Timed out connecting. Try another server from your VPN provider.".into(),
                    );
                });
                me.send("signal SIGTERM\n").await;
            }
        });
        if let Some(c) = conn.as_mut() {
            c.tasks.push(timeout);
        }
        drop(conn);
        self.set_status(|s| s.message = Some("Connecting…".into()));
        Ok(())
    }

    async fn send(&self, line: &str) {
        if let Some(w) = self
            .conn
            .lock()
            .await
            .as_ref()
            .and_then(|c| c.writer.as_ref())
        {
            let _ = w.lock().await.write_all(line.as_bytes()).await;
        }
    }

    async fn drive(self: Arc<Self>, mut r: Reader, password: String, profile_id: String) {
        let mut line = String::new();
        let mut watchdog: Option<tokio::task::JoinHandle<()>> = None;
        let mut cmds = Commands::default();
        loop {
            line.clear();
            match next_line(&mut r, &mut line).await {
                Ok(0) | Err(_) => break,
                Ok(_) => {}
            }
            if line.starts_with("ERROR:") {
                // It can echo back what we sent.
                debug!("openvpn: ERROR");
            } else {
                debug!("openvpn: {}", line.trim_end());
            }
            cmds.on_line(&line);
            let parsed = mgmt::parse(&line);
            if let Line::State { state, .. } = &parsed {
                info!("VPN state: {state}");
            }
            match parsed {
                Line::ManagementPassword => {
                    cmds.push(format!("{password}\n"));
                    cmds.push("state on\n".into());
                    cmds.push("hold release\n".into());
                }
                Line::Hold => cmds.push("hold release\n".into()),
                Line::NeedCredentials => match self
                    .store
                    .credentials(&profile_id)
                    .ok()
                    .and_then(|(u, p)| mgmt::credentials(&u, &p))
                {
                    Some([u, p]) => {
                        cmds.push(u);
                        cmds.push(p);
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
                        s.message = Some("The VPN rejected the username or password (many providers use separate OpenVPN credentials, not your account login)".into());
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
                                warn!("VPN connected but unusable: {e:#}");
                                self.set_status(|s| {
                                    s.phase = Phase::Error;
                                    s.message = Some(format!("{e:#}"));
                                });
                                self.fall_back().await;
                                self.send("signal SIGTERM\n").await;
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
            if let Some(c) = cmds.next() {
                self.send(&c).await;
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

    async fn on_connected(
        self: &Arc<Self>,
        ip: Ipv4Addr,
    ) -> anyhow::Result<tokio::task::JoinHandle<()>> {
        wait_until_bindable(ip).await?;
        let adapter =
            riptor_core::network::adapter_for_ip(ip).context("couldn't find the VPN adapter")?;
        let mut forwarded = None;
        if self.prefs().port_forwarding {
            match natpmp::map(
                (natpmp::DEFAULT_GATEWAY, natpmp::PORT).into(),
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

        // Watchdog. Also renews the port mapping.
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
                            (natpmp::DEFAULT_GATEWAY, natpmp::PORT).into(),
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

    /// The tunnel lasts as long as we hold the helper's stdin.
    #[cfg(target_os = "linux")]
    async fn start_wireguard(self: &Arc<Self>, profile: &Profile) -> anyhow::Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let config = self.store.materialize(&profile.id)?;
        let ip = riptor_vpn::wg::tunnel_ipv4(&config)
            .context("the WireGuard profile has no IPv4 address")?;
        let run = self.dir.join("run");
        std::fs::create_dir_all(&run)?;
        let cfg_path = run.join("connect.conf");
        std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&cfg_path)?
            .write_all(config.as_bytes())?;
        let args = vec![
            "wireguard".to_string(),
            cfg_path.to_string_lossy().into_owned(),
        ];
        let mut child = crate::elevate::spawn(&helper_path()?, &args)?;
        let stdout = child
            .stdout
            .take()
            .context("no output from the VPN helper")?;
        let mut lines = BufReader::new(stdout).lines();
        // Includes the time the user takes to approve the prompt.
        let first = tokio::time::timeout(Duration::from_secs(180), lines.next_line()).await;
        let _ = std::fs::remove_file(&cfg_path);
        if !matches!(&first, Ok(Ok(Some(l))) if l.trim() == "up") {
            drop(child.stdin.take());
            let code = tokio::time::timeout(Duration::from_secs(5), child.wait())
                .await
                .ok()
                .and_then(|r| r.ok())
                .and_then(|s| s.code());
            bail!(match code {
                Some(126) | Some(127) => "administrator permission was declined",
                Some(3) => "the profile was rejected by the VPN helper",
                Some(6) => "WireGuard tools (wg-quick) aren't installed",
                _ => "WireGuard failed to start",
            });
        }
        let watchdog = self.on_connected(ip).await?;
        *self.conn.lock().await = Some(Conn {
            writer: None,
            tasks: vec![watchdog],
            keeper: Some(child),
        });
        Ok(())
    }

    /// For quitting: no session restart.
    pub async fn shutdown(&self) {
        if let Some(c) = self.conn.lock().await.take() {
            c.close().await;
        }
    }

    pub async fn disconnect(&self) {
        let conn = self.conn.lock().await.take();
        if let Some(c) = conn {
            c.close().await;
            self.fall_back().await;
        }
        self.set_status(|s| *s = VpnStatus::default());
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    #[tokio::test]
    async fn password_prompt_without_newline_is_read() {
        let (mut ours, theirs) = tokio::io::duplex(64);
        let mut r = BufReader::new(theirs);
        // Exactly what OpenVPN 2.7 sends, then it waits.
        ours.write_all(b"ENTER PASSWORD:").await.unwrap();
        let mut line = String::new();
        tokio::time::timeout(Duration::from_secs(2), next_line(&mut r, &mut line))
            .await
            .expect("hung waiting for a newline")
            .unwrap();
        assert_eq!(mgmt::parse(&line), Line::ManagementPassword);

        ours.write_all(b"SUCCESS: password is correct\r\n>HOLD:Waiting for hold release:0\r\n")
            .await
            .unwrap();
        line.clear();
        next_line(&mut r, &mut line).await.unwrap();
        assert!(line.starts_with("SUCCESS"));
        line.clear();
        next_line(&mut r, &mut line).await.unwrap();
        assert_eq!(mgmt::parse(&line), Line::Hold);
        drop(ours);
        line.clear();
        assert_eq!(next_line(&mut r, &mut line).await.unwrap(), 0);
    }

    #[test]
    fn commands_wait_for_each_reply() {
        let mut c = Commands::default();
        for cmd in ["pw\n", "state on\n", "hold release\n"] {
            c.push(cmd.into());
        }
        assert_eq!(c.next().as_deref(), Some("pw\n"));
        assert_eq!(c.next(), None);
        c.on_line(">INFO:OpenVPN Management Interface Version 6\r\n");
        assert_eq!(c.next(), None);
        c.on_line("SUCCESS: password is correct\r\n");
        assert_eq!(c.next().as_deref(), Some("state on\n"));
        c.on_line("ERROR: whatever\r\n");
        assert_eq!(c.next().as_deref(), Some("hold release\n"));
        c.on_line("SUCCESS: hold release succeeded\r\n");
        assert_eq!(c.next(), None);
    }
}
