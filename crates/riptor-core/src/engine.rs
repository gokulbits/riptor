//! One librqbit session plus Riptor's storage, counters and queueing.

use std::collections::{BTreeMap, HashMap, HashSet};
use std::net::Ipv6Addr;
use std::num::NonZeroU32;
use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Weak};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use anyhow::{bail, ensure, Context};
use bytes::Bytes;
use librqbit::api::TorrentIdOrHash;
use librqbit::dht::DhtPersistenceConfig;
use librqbit::limits::LimitsConfig;
use librqbit::{
    AddTorrent, AddTorrentOptions, AddTorrentResponse, ConnectionOptions, DhtSessionConfig,
    ListenerMode, ListenerOptions, ManagedTorrent, Session, SessionOptions,
    SessionPersistenceConfig, TorrentStatsState,
};
use librqbit_core::hash_id::Id20;
use parking_lot::{Mutex, RwLock};
use riptor_seeding::{Action, Intent, Scheduler, ShareLimitAction, ShareLimits, TorrentView};
use riptor_storage::sanitize::sanitize_component;
use riptor_storage::{layout_for, CachedStorageFactory, TorrentRoot};
use tokio::sync::broadcast;
use tracing::{info, warn};

use crate::input::{self, Resolved, Source};
use crate::model::*;
use crate::network::NetworkMode;
use crate::reveal::{reveal_target, RevealTarget};
use crate::settings::{Settings, Transport};
use crate::stats::{self, Statistics, StatsReport};
use crate::{backup, motw, paths, state_file, webseed};

mod countries;
mod data_check;
mod hold;
mod reuse;

pub use countries::CountryStatus;
mod peers;
pub use peers::PeerInfo;
mod safety;
mod swarm_ip;

pub use swarm_ip::SwarmView;

const TICK: Duration = Duration::from_secs(1);
const PERSIST_EVERY_TICKS: u64 = 15;
const METADATA_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_PENDING_PREVIEWS: usize = 32;
const WATCH_EVERY_TICKS: u64 = 5;
const DISK_CHECK_EVERY: Duration = Duration::from_secs(5);
/// Resume only with this much more than the limit, so it doesn't flap.
const DISK_HYSTERESIS: u64 = 256 * 1024 * 1024;

type WebSeedInfo = (Arc<webseed::Layout>, Vec<url::Url>);

/// Slow disk work the ticker hands off; one runs at a time.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
enum Job {
    Checksums(Id20),
    ContentScan(Id20),
    /// The VPN's public address, to compare with what the swarm sees.
    VpnAddress,
    NetInfo,
    Scrub(Id20),
    Countries,
}

pub struct EngineConfig {
    pub data_dir: PathBuf,
}

/// A torrent the user is looking at in the add dialog.
struct Pending {
    bytes: Bytes,
    trackers: Vec<String>,
    name: String,
    multi_file: bool,
    file_count: usize,
    has_executables: bool,
    private: bool,
    findings: Vec<crate::suspicion::Finding>,
    reuse: Option<reuse::ReusePlan>,
    at: Instant,
}

#[derive(Default)]
struct Runtime {
    last_fetched: u64,
    last_uploaded: u64,
    /// fetched - verified, last seen (finished torrents only).
    last_waste: u64,
    dl_rate: f64,
    ul_rate: f64,
    running_since: Option<Instant>,
    /// The live state file priorities were last applied to.
    priorities_applied: Option<usize>,
    /// Held because its drive is nearly full.
    no_space: bool,
}

pub struct Engine {
    data_dir: PathBuf,
    session: RwLock<Arc<Session>>,
    network: RwLock<NetworkMode>,
    factory: CachedStorageFactory,
    storage: Arc<riptor_storage::Shared>,
    settings: RwLock<Settings>,
    records: RwLock<HashMap<Id20, Record>>,
    categories: RwLock<BTreeMap<String, Option<PathBuf>>>,
    runtime: Mutex<HashMap<Id20, Runtime>>,
    pending: Mutex<HashMap<Id20, Pending>>,
    scheduler: Mutex<Scheduler>,
    events: broadcast::Sender<Event>,
    http: reqwest::Client,
    epoch: Instant,
    last_tick: Mutex<Instant>,
    ticks: Mutex<u64>,
    dirty: AtomicBool,
    saver: crate::saver::Saver,
    /// Serialises session rebuilds and torrent removals.
    admin: tokio::sync::Mutex<()>,
    statistics: Mutex<Statistics>,
    session_tally: Mutex<SessionTally>,
    last_tick_data: Mutex<(Vec<TorrentSummary>, GlobalStats)>,
    web_seeds: Mutex<HashMap<Id20, webseed::Running>>,
    /// What each torrent's metadata says about web seeds; `None` = none usable.
    web_seed_info: Mutex<HashMap<Id20, Option<WebSeedInfo>>>,
    /// Built for the current network mode.
    web_seed_http: Mutex<Option<reqwest::Client>>,
    allow_local_web_seeds: AtomicBool,
    /// Whether the speed schedule was in its window at the last check.
    alt_window: Mutex<Option<bool>>,
    /// The torrent being created, if any.
    creating: Mutex<Option<Arc<crate::create::Progress>>>,
    /// Finished downloads leaving the incomplete folder; `queued` waits for the ticker.
    moving: Mutex<HashSet<Id20>>,
    queued_moves: Mutex<Vec<(Id20, PathBuf)>>,
    last_disk_check: Mutex<Option<Instant>>,
    /// Waiting, and everything queued or running (so nothing runs twice at once).
    jobs: Mutex<(Vec<Job>, HashSet<Job>)>,
    disk_job: tokio::sync::Mutex<()>,
    /// Public addresses with and without the VPN, once known.
    vpn_public: Mutex<(Option<std::net::IpAddr>, Option<Instant>)>,
    normal_public: Mutex<Option<std::net::IpAddr>>,
    swarm_verdict: Mutex<crate::seen_ip::Verdict>,
    /// Torrents held because the swarm sees the normal address; the user lifts it.
    leak_hold: Mutex<Option<String>>,
    net_info: Mutex<Option<crate::netinfo::NetInfo>>,
    scrubbing: Mutex<HashMap<Id20, Arc<crate::scrub::Progress>>>,
    geo: Mutex<countries::GeoState>,
}

#[derive(Default)]
struct SessionTally {
    uploaded: u64,
    downloaded: u64,
    wasted: u64,
    uptime: f64,
    started: Option<Instant>,
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn kib(v: Option<u32>) -> Option<NonZeroU32> {
    v.and_then(|k| NonZeroU32::new(k.saturating_mul(1024)))
}

fn parse_hash(s: &str) -> anyhow::Result<Id20> {
    ensure!(
        s.len() == 40 && s.bytes().all(|b| b.is_ascii_hexdigit()),
        "invalid torrent id"
    );
    Id20::from_str(s).map_err(|e| anyhow::anyhow!("invalid torrent id: {e}"))
}

fn delta(now: u64, before: u64) -> u64 {
    // librqbit's live counters restart when a torrent is resumed.
    if now >= before {
        now - before
    } else {
        now
    }
}

impl Engine {
    pub async fn start(cfg: EngineConfig) -> anyhow::Result<Arc<Self>> {
        std::fs::create_dir_all(&cfg.data_dir)?;
        crate::observe::install();
        let data_dir = cfg.data_dir;
        let mut settings: Settings = state_file::load(&data_dir.join("settings.json"))?;
        settings.migrate();
        if let Err(e) = settings.validate() {
            warn!("settings invalid ({e:#}), using defaults");
            settings = Settings::default();
        }
        let state: PersistedState = state_file::load(&data_dir.join("torrents.json"))?;
        let mut statistics: Statistics = state_file::load(&data_dir.join("statistics.json"))
            .unwrap_or_else(|e| {
                warn!("statistics unreadable ({e:#}), starting fresh");
                Statistics::default()
            });
        statistics.sanitize();
        statistics.sessions += 1;
        if statistics.first_started == 0 {
            statistics.first_started = unix_now();
        }

        let storage = riptor_storage::Shared::new(settings.storage.clone());
        let factory = CachedStorageFactory::new(storage.clone());
        let mut records = HashMap::new();
        for r in state.torrents {
            match parse_hash(&r.info_hash) {
                Ok(h) => {
                    factory.register(
                        h,
                        TorrentRoot {
                            path: r.root.clone(),
                            owned: r.owned_root,
                            renames: r.renames.clone(),
                        },
                    );
                    records.insert(h, r);
                }
                Err(e) => warn!("dropping corrupt torrent record: {e:#}"),
            }
        }

        let known: HashSet<String> = records
            .values()
            .map(|r| r.info_hash.to_ascii_lowercase())
            .collect();
        match crate::session_db::prune(&data_dir.join("session"), &known) {
            Ok(gone) => {
                for h in gone {
                    info!("removed leftover session entry {h} (no Riptor record)");
                }
            }
            Err(e) => warn!("couldn't tidy the session folder: {e:#}"),
        }

        // The network isn't known yet, so the untrusted-network rule holds too.
        let network = if settings.kill_switch(None) {
            NetworkMode::Blocked
        } else {
            NetworkMode::Normal
        };
        network.note_route();
        let session = build_session(&data_dir, &settings, &factory, &network).await?;
        let (events, _) = broadcast::channel(64);
        let engine = Arc::new(Self {
            data_dir,
            session: RwLock::new(session),
            network: RwLock::new(network),
            factory,
            storage,
            settings: RwLock::new(settings),
            records: RwLock::new(records),
            categories: RwLock::new(state.categories),
            runtime: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
            scheduler: Mutex::new(Scheduler::new()),
            events,
            http: input::http_client()?,
            epoch: Instant::now(),
            last_tick: Mutex::new(Instant::now()),
            ticks: Mutex::new(0),
            dirty: AtomicBool::new(false),
            saver: Default::default(),
            admin: tokio::sync::Mutex::new(()),
            statistics: Mutex::new(statistics),
            session_tally: Mutex::new(SessionTally {
                started: Some(Instant::now()),
                ..Default::default()
            }),
            last_tick_data: Mutex::new(Default::default()),
            web_seeds: Mutex::new(HashMap::new()),
            web_seed_info: Mutex::new(HashMap::new()),
            web_seed_http: Mutex::new(None),
            allow_local_web_seeds: AtomicBool::new(false),
            alt_window: Mutex::new(None),
            creating: Mutex::new(None),
            moving: Mutex::new(HashSet::new()),
            queued_moves: Mutex::new(Vec::new()),
            last_disk_check: Mutex::new(None),
            jobs: Mutex::new((Vec::new(), HashSet::new())),
            disk_job: tokio::sync::Mutex::new(()),
            vpn_public: Mutex::new((None, None)),
            normal_public: Mutex::new(None),
            swarm_verdict: Mutex::new(crate::seen_ip::Verdict::Unknown),
            leak_hold: Mutex::new(None),
            net_info: Mutex::new(None),
            scrubbing: Mutex::new(HashMap::new()),
            geo: Mutex::new(Default::default()),
        });
        engine.apply_limits();
        engine.forget_orphans().await;
        spawn_ticker(Arc::downgrade(&engine));
        Ok(engine)
    }

    /// The torrents and totals from the last tick.
    pub fn snapshot(&self) -> (Vec<TorrentSummary>, GlobalStats) {
        self.last_tick_data.lock().clone()
    }

    pub fn subscribe(&self) -> broadcast::Receiver<Event> {
        self.events.subscribe()
    }

    fn session(&self) -> Arc<Session> {
        self.session.read().clone()
    }

    fn handle(&self, hash: &Id20) -> anyhow::Result<Arc<ManagedTorrent>> {
        self.session()
            .get(TorrentIdOrHash::Hash(*hash))
            .context("torrent not found")
    }

    fn notice(&self, level: NoticeLevel, message: impl Into<String>) {
        let _ = self.events.send(Event::Notice {
            level,
            message: message.into(),
        });
    }

    /// No record means we don't know where its files belong. Files are kept.
    async fn forget_orphans(&self) {
        let session = self.session();
        let known: HashSet<Id20> = self.records.read().keys().copied().collect();
        let orphans: Vec<Id20> = session.with_torrents(|it| {
            it.map(|(_, h)| h.info_hash())
                .filter(|h| !known.contains(h))
                .collect()
        });
        for h in orphans {
            warn!("forgetting torrent {} with no Riptor record", h.as_string());
            let _ = session.delete(TorrentIdOrHash::Hash(h), false).await;
        }
    }

    // ---------------------------------------------------------------- adding

    /// Magnets can take a while: metadata comes from peers.
    pub async fn preview(&self, src: Source) -> anyhow::Result<Preview> {
        let http = match &src {
            Source::Url(u) => self.web_client(u)?,
            _ => self.http.clone(),
        };
        let resolved = input::resolve(&src, &http).await;
        if let Source::Url(u) = &src {
            if !u.trim_start().starts_with("magnet:") {
                riptor_net::ledger::record(
                    riptor_net::ledger::Purpose::TorrentLink,
                    u,
                    resolved.is_ok(),
                );
            }
        }
        let (add, mut trackers) = match resolved? {
            Resolved::Magnet(m) => {
                let parsed = librqbit::Magnet::parse(&m)?;
                (AddTorrent::from_url(m), parsed.trackers.clone())
            }
            Resolved::Bytes(b) => (AddTorrent::from_bytes(b), Vec::new()),
        };
        let opts = AddTorrentOptions {
            list_only: true,
            ..Default::default()
        };
        let resp = tokio::time::timeout(
            METADATA_TIMEOUT,
            self.session().add_torrent(add, Some(opts)),
        )
        .await
        .context("timed out fetching torrent metadata")??;
        let AddTorrentResponse::ListOnly(lo) = resp else {
            bail!("unexpected response while reading torrent")
        };

        let mut comment = None;
        if let Ok(t) = librqbit_core::torrent_metainfo::torrent_from_bytes(&lo.torrent_bytes) {
            comment = t.comment.as_ref().map(|c| {
                String::from_utf8_lossy(c.as_ref())
                    .chars()
                    .take(2000)
                    .collect()
            });
            for a in t.iter_announce() {
                if let Ok(s) = std::str::from_utf8(a.as_ref()) {
                    trackers.push(s.to_string());
                }
            }
        }
        trackers.retain(|t| {
            t.starts_with("http://") || t.starts_with("https://") || t.starts_with("udp://")
        });
        trackers.sort();
        trackers.dedup();

        let name = lo
            .info
            .name()
            .map(|n| n.into_owned())
            .unwrap_or_else(|| lo.info_hash.as_string());
        let lengths = lo.info.lengths();
        let raw: Vec<_> = lo
            .info
            .iter_file_details()
            .map(|fd| riptor_storage::layout::RawFile {
                components: fd
                    .filename
                    .iter_components()
                    .map(|c| c.into_owned().into())
                    .collect(),
                len: fd.len,
                padding: fd.attrs().padding,
            })
            .collect();
        let multi_file = raw.len() > 1 || lo.info.info().files.is_some();
        let base = self.settings.read().download_dir.clone();
        let layout =
            riptor_storage::layout::Layout::new(&base, false, lengths.default_piece_length(), raw)?;
        let files: Vec<FileEntry> = layout
            .files
            .iter()
            .enumerate()
            .filter(|(_, f)| !f.padding)
            .map(|(i, f)| {
                let rel = f
                    .path
                    .strip_prefix(&base)
                    .unwrap_or(&f.path)
                    .to_string_lossy()
                    .into_owned();
                FileEntry {
                    index: i,
                    executable: input::is_executable_name(&rel),
                    media: input::is_media(&rel),
                    previewable: crate::preview::previewable(&rel),
                    path: rel,
                    size: f.len,
                    done: 0,
                    selected: lo.only_files.as_ref().is_none_or(|o| o.contains(&i)),
                    priority: FilePriority::Normal,
                    ready: 0,
                }
            })
            .collect();
        let has_executables = files.iter().any(|f| f.executable);
        let already_added = self.records.read().contains_key(&lo.info_hash);
        let findings = name_findings(&lo.info, &name);
        let duplicates = if already_added {
            Vec::new()
        } else {
            self.find_duplicates(&lo.info_hash, &lo.info).await
        };
        let duplicate_bytes = duplicates
            .iter()
            .filter_map(|d| files.iter().find(|f| f.index == d.index))
            .map(|f| f.size)
            .sum();

        let preview = Preview {
            info_hash: lo.info_hash.as_string(),
            name: name.clone(),
            total_size: lengths.total_length(),
            piece_length: lengths.default_piece_length(),
            multi_file,
            private: lo.info.info().private,
            has_executables,
            already_added,
            trackers: trackers.clone(),
            comment,
            files,
            findings: findings.clone(),
            duplicate_bytes,
            duplicates: duplicates
                .iter()
                .map(|d| Duplicate {
                    index: d.index,
                    existing: d.existing.to_string_lossy().into_owned(),
                })
                .collect(),
        };

        let mut pending = self.pending.lock();
        pending.retain(|_, p| p.at.elapsed() < Duration::from_secs(3600));
        if pending.len() >= MAX_PENDING_PREVIEWS {
            if let Some(oldest) = pending.iter().min_by_key(|(_, p)| p.at).map(|(k, _)| *k) {
                pending.remove(&oldest);
            }
        }
        pending.insert(
            lo.info_hash,
            Pending {
                bytes: lo.torrent_bytes,
                trackers,
                name,
                multi_file,
                file_count: layout.files.len(),
                has_executables,
                private: lo.info.info().private,
                findings,
                reuse: (!duplicates.is_empty())
                    .then(|| reuse::ReusePlan::new(duplicates.clone(), &lo.info)),
                at: Instant::now(),
            },
        );
        Ok(preview)
    }

    pub async fn add(&self, params: AddParams) -> anyhow::Result<String> {
        let hash = parse_hash(&params.info_hash)?;
        ensure!(
            !self.records.read().contains_key(&hash),
            "this torrent is already in the list"
        );
        let p = self
            .pending
            .lock()
            .remove(&hash)
            .context("this torrent's details expired; open it again")?;

        let settings = self.settings.read().clone();
        let category = params
            .category
            .as_deref()
            .map(str::trim)
            .filter(|c| !c.is_empty())
            .map(str::to_string);
        if let Some(c) = &category {
            validate_label(c)?;
        }
        let tags = params
            .tags
            .iter()
            .map(|t| t.trim().to_string())
            .filter(|t| !t.is_empty())
            .collect::<Vec<_>>();
        for t in &tags {
            validate_label(t)?;
        }
        let category_dir = category
            .as_ref()
            .and_then(|c| self.categories.read().get(c).cloned().flatten());
        let base = params
            .save_path
            .clone()
            .or(category_dir)
            .unwrap_or(settings.download_dir.clone());
        self.check_root(&base)?;

        let subfolder =
            params.create_subfolder.unwrap_or(settings.create_subfolder) && p.multi_file;
        // Files already at the save folder (seeding what you have) stay put.
        let has_data = base.join(sanitize_component(&p.name)).exists();
        let incomplete = settings
            .incomplete_dir
            .clone()
            .filter(|d| *d != base && !has_data);
        if let Some(d) = &incomplete {
            self.check_root(d)?;
        }
        let start = incomplete.as_ref().unwrap_or(&base);
        let (root, owned) = if subfolder {
            (start.join(sanitize_component(&p.name)), true)
        } else {
            (start.clone(), false)
        };
        let root_str = root
            .to_str()
            .context("save folder path is not valid Unicode")?
            .to_string();

        if let Some(only) = &params.only_files {
            ensure!(!only.is_empty(), "select at least one file");
            ensure!(
                only.iter().all(|i| *i < p.file_count),
                "invalid file selection"
            );
        }
        if let (true, Some(plan)) = (params.reuse_existing, &p.reuse) {
            self.place_duplicates(plan, &root, owned, params.only_files.as_deref())
                .await;
        }

        self.factory.register(
            hash,
            TorrentRoot {
                path: root.clone(),
                owned,
                renames: Default::default(),
            },
        );
        let opts = AddTorrentOptions {
            paused: true, // the queue decides when it actually starts
            overwrite: true,
            output_folder: Some(root_str),
            only_files: params.only_files.clone(),
            trackers: {
                let mut t = p.trackers.clone();
                if !p.private {
                    for extra in &settings.extra_trackers {
                        if !t.contains(extra) {
                            t.push(extra.clone());
                        }
                    }
                }
                (!t.is_empty()).then_some(t)
            },
            ..Default::default()
        };
        if let Err(e) = self
            .session()
            .add_torrent(AddTorrent::from_bytes(p.bytes), Some(opts))
            .await
        {
            self.factory.unregister(&hash);
            return Err(e.context("could not add torrent"));
        }

        let now = unix_now();
        let dangerous =
            crate::suspicion::worst(&p.findings) == Some(crate::suspicion::Level::Danger);
        {
            let mut records = self.records.write();
            let next_pos = records
                .values()
                .filter_map(|r| r.queue_pos)
                .max()
                .map_or(0, |m| m + 1);
            records.insert(
                hash,
                Record {
                    info_hash: hash.as_string(),
                    name: p.name.clone(),
                    root,
                    owned_root: owned,
                    category,
                    tags,
                    added_on: now,
                    completed_on: None,
                    all_time_upload: 0,
                    all_time_download: 0,
                    seeding_time: 0,
                    active_time: 0,
                    last_activity: now,
                    limits: ShareLimits::default(),
                    limit_action: None,
                    queue_pos: Some(next_pos),
                    intent: if params.start_paused || (dangerous && !params.allow_suspicious) {
                        Intent::Stopped
                    } else {
                        Intent::Started
                    },
                    has_executables: p.has_executables,
                    dl_limit_kib: None,
                    ul_limit_kib: None,
                    move_on_complete: incomplete.map(|_| base.clone()),
                    file_priority: BTreeMap::new(),
                    renames: BTreeMap::new(),
                    checksums: None,
                    last_scrub: None,
                    findings: Some(p.findings),
                    content_findings: None,
                },
            );
        }
        if dangerous && !params.allow_suspicious && !params.start_paused {
            self.notice(
                NoticeLevel::Warning,
                format!("{} looks like a fake release, so it was added stopped. Check it before starting it.", p.name),
            );
        }
        self.statistics.lock().torrents_added += 1;
        // Checked on the next tick, before the queue can start it.
        *self.last_disk_check.lock() = None;
        self.persist().await?;
        info!("added {}", p.name);
        Ok(hash.as_string())
    }

    /// Links are fetched the way torrents connect: through the VPN when it's on.
    pub fn web_client(&self, url: &str) -> anyhow::Result<reqwest::Client> {
        let lan = url::Url::parse(url).is_ok_and(|u| riptor_net::stays_local(&u));
        match self.network() {
            _ if lan => Ok(self.http.clone()),
            NetworkMode::Normal => Ok(self.http.clone()),
            NetworkMode::Vpn(b) => webseed::client(Some(b.local_ip.into()), false),
            NetworkMode::Blocked => {
                bail!("waiting for the VPN: the kill switch holds downloads while it's down")
            }
        }
    }

    pub async fn add_direct(&self, src: Source, mut params: AddParams) -> anyhow::Result<String> {
        let preview = self.preview(src).await?;
        if preview.already_added {
            bail!("this torrent is already in the list");
        }
        params.info_hash = preview.info_hash;
        self.add(params).await
    }

    // ------------------------------------------------------------- controls

    fn set_intent(&self, ids: &[String], intent: Intent) -> anyhow::Result<Vec<Id20>> {
        let hashes = ids
            .iter()
            .map(|s| parse_hash(s))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut records = self.records.write();
        for h in &hashes {
            if let Some(r) = records.get_mut(h) {
                r.intent = intent;
            }
        }
        self.dirty.store(true, Ordering::Relaxed);
        Ok(hashes)
    }

    pub async fn stop(&self, ids: &[String]) -> anyhow::Result<()> {
        let session = self.session();
        for h in self.set_intent(ids, Intent::Stopped)? {
            if let Ok(t) = self.handle(&h) {
                if !t.is_paused() {
                    session.pause(&t).await?;
                }
            }
        }
        Ok(())
    }

    pub fn resume(&self, ids: &[String]) -> anyhow::Result<()> {
        self.set_intent(ids, Intent::Started).map(|_| ())
    }

    pub fn force_start(&self, ids: &[String]) -> anyhow::Result<()> {
        self.set_intent(ids, Intent::Forced).map(|_| ())
    }

    pub async fn remove(&self, ids: &[String], delete_files: bool) -> anyhow::Result<()> {
        for id in ids {
            self.remove_one(parse_hash(id)?, delete_files).await?;
        }
        Ok(())
    }

    async fn remove_one(&self, hash: Id20, delete_files: bool) -> anyhow::Result<()> {
        let _g = self.admin.lock().await;
        self.stop_data_check(&hash);
        self.web_seeds.lock().remove(&hash);
        let result = self
            .session()
            .delete(TorrentIdOrHash::Hash(hash), delete_files)
            .await;
        if self.records.write().remove(&hash).is_some() {
            self.statistics.lock().torrents_removed += 1;
        }
        self.runtime.lock().remove(&hash);
        self.factory.unregister(&hash);
        crate::swarm::forget(&hash);
        self.persist().await?;
        let _ = self.events.send(Event::Removed {
            id: hash.as_string(),
        });
        result
    }

    /// Moves each torrent's files under `dest`; a torrent's own folder comes along.
    pub async fn move_torrents(&self, ids: &[String], dest: &Path) -> anyhow::Result<()> {
        self.check_root(dest)?;
        // An explicit move replaces the move out of the incomplete folder.
        self.edit_records(ids, |r| r.move_on_complete = None)?;
        for id in ids {
            self.reattach(parse_hash(id)?, Some(dest), None).await?;
        }
        Ok(())
    }

    /// Moves a finished download out of the incomplete folder.
    async fn finish_move(&self, hash: Id20, dest: PathBuf) {
        let result = match self.check_root(&dest) {
            Ok(()) => self.reattach(hash, Some(&dest), None).await,
            Err(e) => Err(e),
        };
        let name = self.records.write().get_mut(&hash).map(|r| {
            r.move_on_complete = None;
            r.name.clone()
        });
        self.dirty.store(true, Ordering::Relaxed);
        self.moving.lock().remove(&hash);
        if let (Err(e), Some(name)) = (result, name) {
            warn!("moving {name} out of the incomplete folder failed: {e:#}");
            let _ = self.events.send(Event::Notice {
                level: NoticeLevel::Warning,
                message: format!(
                    "{name} finished but couldn't be moved to {}: {e:#}",
                    dest.display()
                ),
            });
        }
    }

    fn take_queued_moves(&self) -> Vec<(Id20, PathBuf)> {
        std::mem::take(&mut *self.queued_moves.lock())
    }

    /// Moves torrents into their category's folder (or the default one).
    pub async fn move_to_category_folders(&self, ids: &[String]) -> anyhow::Result<()> {
        let download_dir = self.settings.read().download_dir.clone();
        for id in ids {
            let hash = parse_hash(id)?;
            let Some(rec) = self.records.read().get(&hash).cloned() else {
                continue;
            };
            let target = rec
                .category
                .as_ref()
                .and_then(|c| self.categories.read().get(c).cloned().flatten())
                .unwrap_or_else(|| download_dir.clone());
            // Still downloading: it moves to the new folder once complete.
            if rec.move_on_complete.is_some() {
                self.check_root(&target)?;
                if let Some(r) = self.records.write().get_mut(&hash) {
                    r.move_on_complete = Some(target);
                }
                self.dirty.store(true, Ordering::Relaxed);
                continue;
            }
            let base = if rec.owned_root {
                rec.root.parent().map(Path::to_path_buf)
            } else {
                Some(rec.root.clone())
            };
            if base.as_deref() != Some(target.as_path()) {
                self.check_root(&target)?;
                self.reattach(hash, Some(&target), None).await?;
            }
        }
        Ok(())
    }

    /// Checks every piece on disk again.
    pub async fn recheck(&self, ids: &[String]) -> anyhow::Result<()> {
        for id in ids {
            self.reattach(parse_hash(id)?, None, None).await?;
        }
        Ok(())
    }

    /// Remove, maybe move, re-add. Always ends in a full recheck.
    async fn reattach(
        &self,
        hash: Id20,
        dest: Option<&Path>,
        renames: Option<BTreeMap<usize, String>>,
    ) -> anyhow::Result<()> {
        let _g = self.admin.lock().await;
        self.stop_data_check(&hash);
        let rec = self
            .records
            .read()
            .get(&hash)
            .cloned()
            .context("unknown torrent")?;
        let new_root = match dest {
            None => rec.root.clone(),
            Some(d) if rec.owned_root => {
                d.join(rec.root.file_name().context("torrent folder has no name")?)
            }
            Some(d) => d.to_path_buf(),
        };
        if dest.is_some() {
            if new_root == rec.root {
                return Ok(());
            }
            ensure!(
                !(rec.owned_root && new_root.starts_with(&rec.root)),
                "can't move a torrent's folder into itself"
            );
            self.check_root(&new_root)?;
        }

        let old = TorrentRoot {
            path: rec.root.clone(),
            owned: rec.owned_root,
            renames: rec.renames.clone(),
        };
        let new = TorrentRoot {
            path: new_root.clone(),
            owned: rec.owned_root,
            renames: renames.clone().unwrap_or_else(|| rec.renames.clone()),
        };
        let t = self.handle(&hash)?;
        let (bytes, pairs, dirs) = t.with_metadata(|m| -> anyhow::Result<_> {
            let before = layout_for(&old, m)?;
            let after = layout_for(&new, m)?;
            let pairs: Vec<(PathBuf, PathBuf)> = before
                .files
                .iter()
                .zip(after.files.iter())
                .filter(|(a, _)| !a.padding)
                .map(|(a, b)| (a.path.clone(), b.path.clone()))
                .collect();
            Ok((
                m.torrent_bytes.clone(),
                pairs,
                before.subdirs_deepest_first(),
            ))
        })??;
        let only_files = t.only_files();
        let trackers: Vec<String> = t.trackers().iter().map(|u| u.to_string()).collect();

        let session = self.session();
        self.web_seeds.lock().remove(&hash);
        if !t.is_paused() {
            session.pause(&t).await?;
        }
        drop(t);
        session
            .delete(TorrentIdOrHash::Hash(hash), false)
            .await
            .context("taking the torrent out of the session")?;
        self.runtime.lock().remove(&hash);

        let mut root = rec.root.clone();
        let mut kept = rec.renames.clone();
        let mut moved = Ok(());
        if dest.is_some() || renames.is_some() {
            let pairs: Vec<(PathBuf, PathBuf)> =
                pairs.into_iter().filter(|(a, b)| a != b).collect();
            moved = tokio::task::spawn_blocking(move || crate::relocate::move_files(&pairs))
                .await
                .context("move task failed")?
                .map(|_| ());
            if moved.is_ok() {
                let mut leftovers = dirs;
                if rec.owned_root {
                    leftovers.push(rec.root.clone());
                }
                crate::relocate::remove_empty_dirs(leftovers);
                root = new_root;
                kept = new.renames.clone();
            }
        }

        self.factory.register(
            hash,
            TorrentRoot {
                path: root.clone(),
                owned: rec.owned_root,
                renames: kept.clone(),
            },
        );
        let opts = AddTorrentOptions {
            paused: true, // the queue decides when it runs again
            overwrite: true,
            output_folder: Some(
                root.to_str()
                    .context("folder path is not valid Unicode")?
                    .to_string(),
            ),
            only_files,
            trackers: (!trackers.is_empty()).then_some(trackers),
            ..Default::default()
        };
        let added = session
            .add_torrent(AddTorrent::from_bytes(bytes), Some(opts))
            .await;
        if let Some(r) = self.records.write().get_mut(&hash) {
            r.root = root;
            r.renames = kept;
        }
        self.persist().await?;
        added.context("adding the torrent back")?;
        moved
    }

    /// Renames files (and so folders): `(index, new relative path with / between folders)`.
    pub async fn rename_files(&self, id: &str, changes: &[(usize, String)]) -> anyhow::Result<()> {
        let hash = parse_hash(id)?;
        ensure!(
            !changes.is_empty() && changes.len() <= 100_000,
            "nothing to rename"
        );
        let rec = self
            .records
            .read()
            .get(&hash)
            .cloned()
            .context("unknown torrent")?;
        let t = self.handle(&hash)?;
        let originals: Vec<(String, bool)> = t.with_metadata(|m| {
            m.info
                .iter_file_details()
                .map(|fd| {
                    let parts: Vec<String> = fd
                        .filename
                        .iter_components()
                        .map(|c| c.into_owned())
                        .collect();
                    (parts.join("/"), fd.attrs().padding)
                })
                .collect()
        })?;
        drop(t);
        let mut renames = rec.renames.clone();
        for (i, rel) in changes {
            let (original, padding) = originals.get(*i).context("no such file")?;
            ensure!(!padding, "no such file");
            let rel = clean_rel_path(rel)?;
            if rel == *original {
                renames.remove(i);
            } else {
                renames.insert(*i, rel);
            }
        }
        if renames == rec.renames {
            return Ok(());
        }
        // Every file must still land in its own place.
        let new = TorrentRoot {
            path: rec.root.clone(),
            owned: rec.owned_root,
            renames: renames.clone(),
        };
        self.handle(&hash)?
            .with_metadata(|m| -> anyhow::Result<()> {
                let layout = layout_for(&new, m)?;
                for i in renames.keys() {
                    let want = renames[i]
                        .split('/')
                        .fold(rec.root.clone(), |p, c| p.join(c));
                    ensure!(
                        layout.files.get(*i).is_some_and(|f| f.path == want),
                        "another file already has the name {}",
                        renames[i]
                    );
                }
                Ok(())
            })??;
        self.reattach(hash, None, Some(renames)).await
    }

    pub async fn set_files(&self, id: &str, only_files: Vec<usize>) -> anyhow::Result<()> {
        let hash = parse_hash(id)?;
        let t = self.handle(&hash)?;
        let count = t.with_metadata(|m| m.file_infos.len())?;
        ensure!(!only_files.is_empty(), "select at least one file");
        ensure!(
            only_files.iter().all(|i| *i < count),
            "invalid file selection"
        );
        let set: HashSet<usize> = only_files.into_iter().collect();
        self.session().update_only_files(&t, &set).await
    }

    pub fn set_file_priority(
        &self,
        id: &str,
        files: &[usize],
        priority: FilePriority,
    ) -> anyhow::Result<()> {
        let hash = parse_hash(id)?;
        let t = self.handle(&hash)?;
        let count = t.with_metadata(|m| m.file_infos.len())?;
        ensure!(
            !files.is_empty() && files.iter().all(|i| *i < count),
            "invalid file selection"
        );
        {
            let mut records = self.records.write();
            let rec = records.get_mut(&hash).context("unknown torrent")?;
            for i in files {
                if priority == FilePriority::Normal {
                    rec.file_priority.remove(i);
                } else {
                    rec.file_priority.insert(*i, priority);
                }
            }
        }
        if let Some(r) = self.runtime.lock().get_mut(&hash) {
            r.priorities_applied = None;
        }
        self.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    /// Saves the torrent's .torrent file, optionally without trackers that carry a passkey.
    pub fn export_torrent(&self, id: &str, path: &Path, strip_secrets: bool) -> anyhow::Result<()> {
        ensure!(path.is_absolute(), "path must be absolute");
        ensure!(
            path.extension()
                .is_some_and(|e| e.eq_ignore_ascii_case("torrent")),
            "the file must end in .torrent"
        );
        self.check_root(path.parent().context("no folder")?)?;
        if let Ok(existing) = std::fs::read(path) {
            ensure!(
                existing.first() == Some(&b'd'),
                "{} exists and is not a torrent file",
                path.display()
            );
        }
        let bytes = self
            .handle(&parse_hash(id)?)?
            .with_metadata(|m| m.torrent_bytes.clone())?;
        ensure!(!bytes.is_empty(), "this torrent's metadata isn't known yet");
        let bytes = if strip_secrets {
            crate::passkey::strip_secret_trackers(&bytes)?.0.into()
        } else {
            bytes
        };
        std::fs::write(path, &bytes)?;
        Ok(())
    }

    pub fn set_category(&self, ids: &[String], category: Option<String>) -> anyhow::Result<()> {
        if let Some(c) = &category {
            validate_label(c)?;
            ensure!(self.categories.read().contains_key(c), "unknown category");
        }
        self.edit_records(ids, |r| r.category = category.clone())
    }

    pub fn set_tags(&self, ids: &[String], tags: Vec<String>) -> anyhow::Result<()> {
        for t in &tags {
            validate_label(t)?;
        }
        self.edit_records(ids, |r| r.tags = tags.clone())
    }

    pub fn set_share_limits(
        &self,
        ids: &[String],
        limits: ShareLimits,
        action: Option<ShareLimitAction>,
    ) -> anyhow::Result<()> {
        self.edit_records(ids, |r| {
            r.limits = limits;
            r.limit_action = action;
        })
    }

    /// KiB/s each; `None` or 0 removes the torrent's own limit.
    pub fn set_speed_limits(
        &self,
        ids: &[String],
        dl_kib: Option<u32>,
        ul_kib: Option<u32>,
    ) -> anyhow::Result<()> {
        const MAX_KIB: u32 = 4 * 1024 * 1024;
        let norm = |v: Option<u32>| v.filter(|k| *k > 0);
        let (dl, ul) = (norm(dl_kib), norm(ul_kib));
        ensure!(
            dl.is_none_or(|k| k <= MAX_KIB) && ul.is_none_or(|k| k <= MAX_KIB),
            "speed limit too high"
        );
        self.edit_records(ids, |r| {
            r.dl_limit_kib = dl;
            r.ul_limit_kib = ul;
        })
    }

    pub fn queue_move(&self, ids: &[String], how: QueueMove) -> anyhow::Result<()> {
        use riptor_seeding::queue::order;
        let hashes = ids
            .iter()
            .map(|s| parse_hash(s))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut records = self.records.write();
        let mut q: Vec<(u32, Id20)> = records
            .iter()
            .filter_map(|(h, r)| r.queue_pos.map(|p| (p, *h)))
            .collect();
        q.sort();
        // Positions are dense, so index == position; work on small ids.
        let mut ord: Vec<usize> = (0..q.len()).collect();
        let sel: Vec<usize> = q
            .iter()
            .enumerate()
            .filter(|(_, (_, h))| hashes.contains(h))
            .map(|(i, _)| i)
            .collect();
        match how {
            QueueMove::Up => order::move_up(&mut ord, &sel),
            QueueMove::Down => order::move_down(&mut ord, &sel),
            QueueMove::Top => order::move_top(&mut ord, &sel),
            QueueMove::Bottom => order::move_bottom(&mut ord, &sel),
        }
        for (pos, idx) in ord.into_iter().enumerate() {
            if let Some(r) = records.get_mut(&q[idx].1) {
                r.queue_pos = Some(pos as u32);
            }
        }
        self.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    fn edit_records(&self, ids: &[String], f: impl Fn(&mut Record)) -> anyhow::Result<()> {
        let hashes = ids
            .iter()
            .map(|s| parse_hash(s))
            .collect::<anyhow::Result<Vec<_>>>()?;
        let mut records = self.records.write();
        for h in hashes {
            if let Some(r) = records.get_mut(&h) {
                f(r);
            }
        }
        self.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn torrent_ids(&self) -> Vec<String> {
        self.records
            .read()
            .values()
            .map(|r| r.info_hash.clone())
            .collect()
    }

    pub fn categories(&self) -> BTreeMap<String, Option<PathBuf>> {
        self.categories.read().clone()
    }

    pub fn set_category_def(&self, name: &str, save_path: Option<PathBuf>) -> anyhow::Result<()> {
        validate_label(name)?;
        if let Some(p) = &save_path {
            self.check_root(p)?;
        }
        self.categories.write().insert(name.to_string(), save_path);
        self.persist_blocking()
    }

    pub fn remove_category_def(&self, name: &str) -> anyhow::Result<()> {
        self.categories.write().remove(name);
        for r in self.records.write().values_mut() {
            if r.category.as_deref() == Some(name) {
                r.category = None;
            }
        }
        self.persist_blocking()
    }

    // ------------------------------------------------------------- details

    pub fn files(&self, id: &str) -> anyhow::Result<Vec<FileEntry>> {
        let hash = parse_hash(id)?;
        let t = self.handle(&hash)?;
        let root = self
            .factory
            .root_of(&hash)
            .context("torrent has no save folder")?;
        let stats = t.stats();
        let only = t.only_files();
        let have = t.have_pieces().unwrap_or_default();
        let priorities = self
            .records
            .read()
            .get(&hash)
            .map(|r| r.file_priority.clone())
            .unwrap_or_default();
        t.with_metadata(|m| -> anyhow::Result<Vec<FileEntry>> {
            let layout = layout_for(&root, m)?;
            let ready = ready_from_start(&have, m);
            Ok(layout
                .files
                .iter()
                .enumerate()
                .filter(|(_, f)| !f.padding)
                .map(|(i, f)| {
                    let rel = f
                        .path
                        .strip_prefix(&root.path)
                        .unwrap_or(&f.path)
                        .to_string_lossy()
                        .into_owned();
                    FileEntry {
                        index: i,
                        executable: input::is_executable_name(&rel),
                        media: input::is_media(&rel),
                        previewable: crate::preview::previewable(&rel),
                        path: rel,
                        size: f.len,
                        done: stats.file_progress.get(i).copied().unwrap_or(0),
                        selected: only.as_ref().is_none_or(|o| o.contains(&i)),
                        priority: priorities.get(&i).copied().unwrap_or_default(),
                        ready: ready.get(i).copied().unwrap_or(0),
                    }
                })
                .collect())
        })?
    }

    pub fn trackers(&self, id: &str) -> anyhow::Result<Vec<String>> {
        let t = self.handle(&parse_hash(id)?)?;
        let mut v: Vec<String> = t.trackers().iter().map(|u| u.to_string()).collect();
        v.sort();
        Ok(v)
    }

    /// Replaces a torrent's trackers; a running torrent restarts to use them.
    pub async fn set_trackers(&self, id: &str, trackers: &[String]) -> anyhow::Result<()> {
        ensure!(trackers.len() <= 200, "too many trackers");
        let mut urls = Vec::new();
        for t in trackers {
            let t = t.trim();
            ensure!(crate::settings::valid_tracker(t), "not a tracker URL: {t}");
            let u = url::Url::parse(t)?;
            if !urls.contains(&u) {
                urls.push(u);
            }
        }
        let h = self.handle(&parse_hash(id)?)?;
        h.set_trackers(urls);
        self.reannounce(&[id.to_string()]).await
    }

    /// Stops running torrents for a moment so they announce again; the queue restarts them.
    pub async fn reannounce(&self, ids: &[String]) -> anyhow::Result<()> {
        let session = self.session();
        for id in ids {
            let h = self.handle(&parse_hash(id)?)?;
            if !h.is_paused() {
                session.pause(&h).await?;
            }
        }
        self.dirty.store(true, Ordering::Relaxed);
        Ok(())
    }

    pub fn add_peer(&self, id: &str, addr: &str) -> anyhow::Result<()> {
        let addr: std::net::SocketAddr = addr
            .trim()
            .parse()
            .context("use IP:port, e.g. 203.0.113.5:6881 or [2001:db8::1]:6881")?;
        ensure!(
            addr.port() != 0 && !addr.ip().is_unspecified() && !addr.ip().is_multicast(),
            "not a peer address"
        );
        let live = self
            .handle(&parse_hash(id)?)?
            .live()
            .context("start the torrent first")?;
        live.add_peer(addr)?;
        Ok(())
    }

    /// Counts what `import_from` would add; `None` looks in qBittorrent's usual folders.
    pub async fn import_scan(
        &self,
        dir: Option<PathBuf>,
    ) -> anyhow::Result<crate::import::ImportScan> {
        if let Some(d) = &dir {
            ensure!(d.is_absolute(), "choose a folder");
        }
        tokio::task::spawn_blocking(move || crate::import::scan_counts(dir.as_deref())).await?
    }

    /// Adds torrents from qBittorrent's folder; the data is checked where it lies.
    pub async fn import_from(&self, dir: &Path) -> anyhow::Result<crate::import::ImportReport> {
        ensure!(dir.is_absolute(), "choose a folder");
        let d = dir.to_path_buf();
        let (found, skipped) =
            tokio::task::spawn_blocking(move || crate::import::scan(&d)).await??;
        let mut report = crate::import::ImportReport {
            skipped,
            ..Default::default()
        };
        let add_stopped = self.settings.read().add_stopped;
        for f in found {
            let name = f
                .torrent
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let r = &f.resume;
            let result = async {
                let p = self.preview(Source::File(f.torrent.clone())).await?;
                if p.already_added {
                    return Ok(false);
                }
                if let Some(c) = &r.category {
                    self.categories.write().entry(c.clone()).or_insert(None);
                }
                let id = self
                    .add(AddParams {
                        info_hash: p.info_hash,
                        save_path: Some(r.save_path.clone()),
                        category: r.category.clone(),
                        tags: r.tags.clone(),
                        start_paused: r.paused || add_stopped,
                        // Their layout: the torrent's own folder inside the save folder.
                        create_subfolder: Some(true),
                        ..Default::default()
                    })
                    .await?;
                // Carry the history over; the data itself is still checked.
                if let Some(rec) = self.records.write().get_mut(&parse_hash(&id)?) {
                    rec.all_time_download = r.downloaded;
                    rec.all_time_upload = r.uploaded;
                    rec.active_time = r.active_time;
                    rec.seeding_time = r.seeding_time;
                    rec.added_on = r.added_on.unwrap_or(rec.added_on);
                    rec.limits = r.limits;
                }
                self.dirty.store(true, Ordering::Relaxed);
                anyhow::Ok(true)
            }
            .await;
            match result {
                Ok(true) => report.added += 1,
                Ok(false) => report.already_present += 1,
                Err(e) => {
                    if report.failed.len() < 20 {
                        report.failed.push(format!("{name}: {e:#}"));
                    }
                }
            }
        }
        info!(
            "imported {} torrents ({} already here, {} skipped)",
            report.added, report.already_present, report.skipped
        );
        Ok(report)
    }

    pub async fn ban_peer(&self, ip: std::net::IpAddr) -> anyhow::Result<()> {
        let mut s = self.settings();
        if !s.banned_ips.contains(&ip) {
            s.banned_ips.push(ip);
        }
        self.update_settings(s).await
    }

    /// Trackers with a passkey are left out: a magnet link is for sharing.
    pub fn magnet_link(&self, id: &str) -> anyhow::Result<MagnetLink> {
        let hash = parse_hash(id)?;
        let name = self
            .records
            .read()
            .get(&hash)
            .map(|r| r.name.clone())
            .context("unknown torrent")?;
        let (secret, trackers): (Vec<String>, Vec<String>) = self
            .trackers(id)
            .unwrap_or_default()
            .into_iter()
            .partition(|t| crate::passkey::has_secret(t));
        let private = self
            .handle(&hash)
            .ok()
            .and_then(|t| t.with_metadata(|m| m.info.info().private).ok())
            .unwrap_or(false);
        Ok(MagnetLink {
            link: crate::input::magnet_link(&hash.as_string(), &name, &trackers),
            withheld: secret.len(),
            private,
        })
    }

    /// The address peers see.
    pub async fn public_ip(&self) -> anyhow::Result<PublicIp> {
        let (local, via_vpn) = match self.network() {
            NetworkMode::Normal => (None, false),
            NetworkMode::Vpn(b) => (Some(std::net::IpAddr::V4(b.local_ip)), true),
            NetworkMode::Blocked => {
                anyhow::bail!("the kill switch is holding traffic until the VPN connects")
            }
        };
        let ip = crate::public_ip::lookup(local).await?;
        self.note_public_ip(ip, via_vpn);
        Ok(PublicIp {
            ip: ip.to_string(),
            via_vpn,
        })
    }

    /// Compares the address through the tunnel with the normal one, and checks DNS goes through it.
    pub async fn leak_check(&self) -> anyhow::Result<LeakCheck> {
        let NetworkMode::Vpn(b) = self.network() else {
            bail!("connect the VPN first");
        };
        let local = std::net::IpAddr::V4(b.local_ip);
        let (vpn, normal) = tokio::join!(
            crate::public_ip::lookup(Some(local)),
            crate::public_ip::lookup(None)
        );
        let tunnel_dns = match crate::dns::TunnelResolver::new(local, &crate::dns::SERVERS) {
            Ok(r) => r
                .lookup("api.ipify.org", 443)
                .await
                .is_ok_and(|a| !a.is_empty()),
            Err(_) => false,
        };
        let kill_switch = self.kill_switch_on();
        let mut notes = Vec::new();
        let (vpn_ip, normal_ip) = (vpn.ok(), normal.ok());
        if let Some(ip) = vpn_ip {
            self.note_public_ip(ip, true);
        }
        if let Some(ip) = normal_ip {
            self.note_public_ip(ip, false);
        }
        match (&vpn_ip, &normal_ip) {
            (None, _) => notes.push("Couldn't reach the IP check service through the tunnel.".into()),
            (Some(v), Some(n)) if v == n => notes.push(
                "Riptor's address is the same as your normal one: the VPN isn't changing it.".into(),
            ),
            (Some(_), None) => notes.push(
                "Couldn't look up your normal address to compare (that's fine if your normal connection blocks it).".into(),
            ),
            _ => {}
        }
        if !tunnel_dns {
            notes.push("Name lookups through the tunnel failed.".into());
        }
        if !kill_switch {
            notes.push("Turn on the kill switch so torrents stop if the tunnel drops.".into());
        }
        let passed = tunnel_dns
            && match (&vpn_ip, &normal_ip) {
                (Some(v), Some(n)) => v != n,
                (Some(_), None) => true,
                _ => false,
            };
        Ok(LeakCheck {
            vpn_ip: vpn_ip.map(|i| i.to_string()),
            normal_ip: normal_ip.map(|i| i.to_string()),
            tunnel_dns,
            adapter: b.adapter.clone(),
            kill_switch,
            passed,
            notes,
        })
    }

    /// Free space where `folder` would be created; it may not exist yet.
    pub fn free_space(&self, folder: &Path) -> anyhow::Result<u64> {
        anyhow::ensure!(folder.is_absolute(), "not an absolute path");
        let existing = folder
            .ancestors()
            .find(|p| p.is_dir())
            .context("no such drive")?;
        Ok(riptor_storage::free_space(existing)?)
    }

    /// One file's path on disk, its length, and how much of its start is on disk.
    pub fn file_path(&self, id: &str, index: usize) -> anyhow::Result<(PathBuf, u64, u64)> {
        let hash = parse_hash(id)?;
        let root = self.factory.root_of(&hash).context("unknown torrent")?;
        let t = self.handle(&hash)?;
        let have = t.have_pieces().unwrap_or_default();
        t.with_metadata(|m| -> anyhow::Result<(PathBuf, u64, u64)> {
            let layout = layout_for(&root, m)?;
            let f = layout
                .files
                .get(index)
                .filter(|f| !f.padding)
                .context("no such file")?;
            let ready = ready_from_start(&have, m).get(index).copied().unwrap_or(0);
            Ok((f.path.clone(), f.len, ready))
        })?
    }

    pub fn piece_map(&self, id: &str, buckets: usize) -> anyhow::Result<PieceMap> {
        let t = self.handle(&parse_hash(id)?)?;
        Ok(PieceMap::new(&t.have_pieces().unwrap_or_default(), buckets))
    }

    pub fn reveal_target(&self, id: &str) -> anyhow::Result<RevealTarget> {
        let hash = parse_hash(id)?;
        let root = self.factory.root_of(&hash).context("unknown torrent")?;
        // No metadata yet for an unresolved magnet.
        let files: Vec<PathBuf> = match self.handle(&hash) {
            Ok(t) => t
                .with_metadata(|m| -> anyhow::Result<Vec<PathBuf>> {
                    Ok(layout_for(&root, m)?
                        .files
                        .iter()
                        .filter(|f| !f.padding)
                        .map(|f| f.path.clone())
                        .collect())
                })
                .ok()
                .and_then(Result::ok)
                .unwrap_or_default(),
            Err(_) => Vec::new(),
        };
        reveal_target(&root.path, &files)
    }

    // ------------------------------------------------------------ settings

    pub fn settings(&self) -> Settings {
        self.settings.read().clone()
    }

    pub async fn update_settings(&self, mut new: Settings) -> anyhow::Result<()> {
        new.migrate();
        new.validate()?;
        self.check_root(&new.download_dir)?;
        for w in &new.watched_folders {
            self.check_root(&w.path)?;
            if let Some(p) = &w.save_path {
                self.check_root(p)?;
            }
        }
        let old = self.settings();
        if old.alt_schedule != new.alt_schedule {
            *self.alt_window.lock() = None;
        }
        if old.peer_countries && !new.peer_countries {
            self.forget_countries();
        } else if !old.peer_countries && new.peer_countries {
            self.geo.lock().tried = None;
        }
        state_file::save(&self.data_dir.join("settings.json"), &new)?;
        *self.settings.write() = new.clone();
        self.session()
            .set_banned(new.banned_ips.iter().copied().collect());
        self.storage.reconfigure(new.storage.clone());
        *self.last_disk_check.lock() = None;
        self.apply_limits();
        let current = self.network();
        if self.kill_switch_on() != (current == NetworkMode::Blocked)
            && matches!(current, NetworkMode::Normal | NetworkMode::Blocked)
        {
            return self.apply_kill_switch().await;
        }
        if new.needs_restart(&old) {
            self.restart_session().await?;
        }
        Ok(())
    }

    pub fn set_alt_speed(&self, on: bool) -> anyhow::Result<()> {
        let mut s = self.settings();
        s.alt_speed_enabled = on;
        state_file::save(&self.data_dir.join("settings.json"), &s)?;
        *self.settings.write() = s;
        self.apply_limits();
        Ok(())
    }

    fn apply_limits(&self) {
        let (dl, ul) = self.settings.read().effective_limits();
        let s = self.session();
        s.ratelimits.set_download_bps(kib(dl));
        s.ratelimits.set_upload_bps(kib(ul));
    }

    pub fn network(&self) -> NetworkMode {
        self.network.read().clone()
    }

    /// Rebuilds the session so every socket gets the new binding.
    pub async fn set_network(&self, mode: NetworkMode) -> anyhow::Result<()> {
        if *self.network.read() == mode {
            return Ok(());
        }
        info!("network mode -> {}", mode.label());
        mode.note_route();
        *self.network.write() = mode;
        self.forget_swarm_view();
        self.restart_session().await
    }

    async fn restart_session(&self) -> anyhow::Result<()> {
        let _g = self.admin.lock().await;
        self.web_seeds.lock().clear();
        *self.web_seed_http.lock() = None;
        let old = self.session();
        old.stop().await;
        let settings = self.settings();
        let net = self.network.read().clone();
        let new = build_session(&self.data_dir, &settings, &self.factory, &net).await?;
        *self.session.write() = new;
        self.runtime.lock().clear();
        self.apply_limits();
        info!("session restarted with new network settings");
        Ok(())
    }

    // --------------------------------------------------------------- state

    fn snapshot_state(&self) -> PersistedState {
        let mut torrents: Vec<Record> = self.records.read().values().cloned().collect();
        torrents.sort_by_key(|r| r.added_on);
        PersistedState {
            torrents,
            categories: self.categories.read().clone(),
        }
    }

    fn queue_state(&self) -> anyhow::Result<()> {
        self.dirty.store(false, Ordering::Relaxed);
        let stats = state_file::to_bytes(&*self.statistics.lock())?;
        self.saver
            .queue(self.data_dir.join("statistics.json"), stats);
        let torrents = state_file::to_bytes(&self.snapshot_state())?;
        self.saver
            .queue(self.data_dir.join("torrents.json"), torrents);
        Ok(())
    }

    /// Saves now and waits, without blocking the runtime.
    async fn persist(&self) -> anyhow::Result<()> {
        self.queue_state()?;
        self.saver.flush().await
    }

    fn persist_blocking(&self) -> anyhow::Result<()> {
        self.queue_state()?;
        self.saver.flush_blocking()
    }

    /// State files written so far this run; unchanged ones aren't rewritten.
    #[doc(hidden)]
    pub fn state_writes(&self) -> u64 {
        self.saver.writes()
    }

    // -------------------------------------------------------------- create

    /// With `seed`, it's added and seeds from where the files are.
    pub async fn create_torrent(
        &self,
        source: PathBuf,
        output: PathBuf,
        opts: crate::create::CreateOptions,
        seed: bool,
    ) -> anyhow::Result<Option<String>> {
        opts.validate()?;
        ensure!(source.is_absolute(), "source must be an absolute path");
        self.check_root(&source)?;
        ensure!(
            output.is_absolute()
                && output
                    .extension()
                    .is_some_and(|e| e.eq_ignore_ascii_case("torrent")),
            "the new file must be a .torrent"
        );
        let out_dir = output.parent().context("no folder for the new file")?;
        self.check_root(out_dir)?;
        ensure!(
            !output.starts_with(&source),
            "save the .torrent outside the folder it describes"
        );

        let progress = Arc::new(crate::create::Progress::default());
        {
            let mut c = self.creating.lock();
            ensure!(c.is_none(), "a torrent is already being created");
            *c = Some(progress.clone());
        }
        let src = source.clone();
        let p = progress.clone();
        let made =
            tokio::task::spawn_blocking(move || crate::create::create(&src, &opts, &p)).await;
        *self.creating.lock() = None;
        let bytes = made.context("hashing stopped unexpectedly")??;
        state_file::save_bytes(&output, &bytes)?;
        info!("created {}", output.display());
        if !seed {
            return Ok(None);
        }
        let params = AddParams {
            save_path: source.parent().map(Path::to_path_buf),
            create_subfolder: Some(true),
            ..Default::default()
        };
        self.add_direct(Source::File(output), params)
            .await
            .map(Some)
    }

    /// (hashed, total) bytes of the torrent being created.
    pub fn create_progress(&self) -> Option<(u64, u64)> {
        self.creating.lock().as_ref().map(|p| {
            (
                p.done.load(Ordering::Relaxed),
                p.total.load(Ordering::Relaxed),
            )
        })
    }

    pub fn cancel_create(&self) {
        if let Some(p) = self.creating.lock().as_ref() {
            p.cancel.store(true, Ordering::Relaxed);
        }
    }

    // -------------------------------------------------------------- backup

    pub fn check_root(&self, p: &Path) -> anyhow::Result<()> {
        paths::check_save_root(p, Some(&self.data_dir))
    }

    fn check_backup_path(path: &Path) -> anyhow::Result<()> {
        let ok = path
            .extension()
            .is_some_and(|e| e.eq_ignore_ascii_case(backup::EXTENSION));
        ensure!(ok, "backup files must end in .{}", backup::EXTENSION);
        ensure!(path.is_absolute(), "backup path must be absolute");
        Ok(())
    }

    pub fn create_backup(
        &self,
        path: &Path,
        password: Option<String>,
    ) -> anyhow::Result<BackupSummary> {
        Self::check_backup_path(path)?;
        if let Ok(existing) = std::fs::read(path) {
            ensure!(
                existing.starts_with(b"RIPTRBK\0"),
                "{} exists and is not a Riptor backup",
                path.display()
            );
        }
        let session = self.session();
        let records = self.records.read().clone();
        let mut torrents = Vec::new();
        for (hash, record) in records {
            let Some(h) = session.get(TorrentIdOrHash::Hash(hash)) else {
                continue;
            };
            let Ok(bytes) = h.with_metadata(|m| m.torrent_bytes.clone()) else {
                continue;
            };
            torrents.push(backup::BackupTorrent {
                record,
                torrent: B64.encode(&bytes),
                only_files: h.only_files(),
            });
        }
        torrents.sort_by_key(|t| t.record.added_on);
        let payload = backup::BackupPayload {
            created: unix_now(),
            app_version: env!("CARGO_PKG_VERSION").to_string(),
            settings: self.settings(),
            categories: self.categories(),
            torrents,
            statistics: self.statistics.lock().clone(),
        };
        let sealed = backup::seal(&payload, password.as_deref().filter(|p| !p.is_empty()))?;
        state_file::save_bytes(path, &sealed)?;
        Ok(summarize_backup(
            &payload,
            password.is_some_and(|p| !p.is_empty()),
        ))
    }

    fn read_backup(
        path: &Path,
        password: Option<&str>,
    ) -> anyhow::Result<(backup::BackupPayload, bool)> {
        Self::check_backup_path(path)?;
        let len = std::fs::metadata(path)
            .with_context(|| format!("opening {}", path.display()))?
            .len();
        ensure!(len <= backup::MAX_FILE_BYTES, "backup file is too large");
        let bytes = std::fs::read(path)?;
        let protected = backup::is_password_protected(&bytes)?;
        Ok((backup::open(&bytes, password)?, protected))
    }

    pub fn inspect_backup(
        &self,
        path: &Path,
        password: Option<String>,
    ) -> anyhow::Result<BackupSummary> {
        let (payload, protected) = Self::read_backup(path, password.as_deref())?;
        Ok(summarize_backup(&payload, protected))
    }

    /// Backups are untrusted: a password-less one can be forged.
    pub async fn restore_backup(
        &self,
        path: &Path,
        password: Option<String>,
        opts: RestoreOptions,
    ) -> anyhow::Result<RestoreReport> {
        let (payload, _) = Self::read_backup(path, password.as_deref())?;
        let mut report = RestoreReport::default();

        for (name, folder) in &payload.categories {
            let valid = validate_label(name).is_ok()
                && folder.as_deref().is_none_or(|f| self.check_root(f).is_ok());
            if valid {
                self.categories
                    .write()
                    .entry(name.clone())
                    .or_insert(folder.clone());
            } else {
                report
                    .skipped
                    .push(format!("category \"{name}\": invalid name or folder"));
            }
        }

        if opts.settings {
            let s = payload.settings.clone();
            match s.validate().and_then(|_| self.check_root(&s.download_dir)) {
                Ok(()) => {
                    self.update_settings(s).await?;
                    report.settings_restored = true;
                }
                Err(e) => report.skipped.push(format!("settings: {e:#}")),
            }
        }

        if opts.statistics {
            let mut st = payload.statistics.clone();
            st.sanitize();
            st.sessions = st.sessions.max(1);
            *self.statistics.lock() = st;
            report.statistics_restored = true;
        }

        if opts.torrents {
            for t in payload.torrents {
                let name = t.record.name.chars().take(200).collect::<String>();
                match self.restore_torrent(t, opts.resume_torrents).await {
                    Ok(true) => report.added += 1,
                    Ok(false) => report.already_present += 1,
                    Err(e) => report.skipped.push(format!("{name}: {e:#}")),
                }
            }
        }
        self.persist().await?;
        Ok(report)
    }

    async fn restore_torrent(
        &self,
        t: backup::BackupTorrent,
        resume: bool,
    ) -> anyhow::Result<bool> {
        let hash = parse_hash(&t.record.info_hash)?;
        if self.records.read().contains_key(&hash) {
            return Ok(false);
        }
        let bytes = B64
            .decode(t.torrent.as_bytes())
            .context("damaged torrent data")?;
        ensure!(
            bytes.len() <= input::MAX_TORRENT_BYTES,
            "torrent data too large"
        );
        // The torrent must really be the one the record claims.
        let parsed = librqbit_core::torrent_metainfo::torrent_from_bytes(&bytes)
            .context("damaged torrent data")?;
        ensure!(
            parsed.info_hash == hash,
            "torrent data does not match its id"
        );
        let file_count = parsed.info.data.files.as_ref().map_or(1, |f| f.len());
        drop(parsed);

        let mut rec = t.record;
        rec.info_hash = hash.as_string();
        self.check_root(&rec.root)?;
        rec.name = rec
            .name
            .chars()
            .filter(|c| !c.is_control())
            .take(1024)
            .collect();
        if rec
            .category
            .as_ref()
            .is_some_and(|c| !self.categories.read().contains_key(c))
        {
            rec.category = None;
        }
        rec.tags.retain(|tag| validate_label(tag).is_ok());
        rec.tags.truncate(32);
        // Backups are untrusted: only names that pass the same checks as a rename.
        rec.renames
            .retain(|i, r| *i < file_count && clean_rel_path(r).is_ok_and(|c| c == *r));
        rec.file_priority.retain(|i, _| *i < file_count);
        if rec
            .move_on_complete
            .as_ref()
            .is_some_and(|p| self.check_root(p).is_err())
        {
            rec.move_on_complete = None;
        }
        if !resume {
            rec.intent = Intent::Stopped;
        }
        // A forged backup could claim files were verified or hide warnings.
        rec.checksums = None;
        rec.findings = None;
        rec.content_findings = None;
        rec.queue_pos = None;
        let only_files = t
            .only_files
            .filter(|o| !o.is_empty() && o.iter().all(|i| *i < file_count));
        let root_str = rec
            .root
            .to_str()
            .context("save folder path is not valid Unicode")?
            .to_string();

        self.factory.register(
            hash,
            TorrentRoot {
                path: rec.root.clone(),
                owned: rec.owned_root,
                renames: rec.renames.clone(),
            },
        );
        let opts = AddTorrentOptions {
            paused: true,
            overwrite: true,
            output_folder: Some(root_str),
            only_files,
            ..Default::default()
        };
        if let Err(e) = self
            .session()
            .add_torrent(AddTorrent::from_bytes(bytes), Some(opts))
            .await
        {
            self.factory.unregister(&hash);
            return Err(e);
        }
        self.records.write().insert(hash, rec);
        Ok(true)
    }

    pub fn statistics(&self) -> StatsReport {
        let (torrents, global) = self.last_tick_data.lock().clone();
        let all_time = self.statistics.lock().clone();
        let tally = self.session_tally.lock();
        let session = self.session().stats_snapshot();
        let now = unix_now();
        let count = |f: &dyn Fn(&TorrentSummary) -> bool| torrents.iter().filter(|t| f(t)).count();
        let cfg = self.storage.config.lock().clone();
        StatsReport {
            all_time_ratio: stats::ratio(all_time.all_time_upload, all_time.all_time_download),
            session_upload: tally.uploaded,
            session_download: tally.downloaded,
            session_wasted: tally.wasted,
            session_ratio: stats::ratio(tally.uploaded, tally.downloaded),
            session_uptime_secs: tally.started.map_or(0, |s| s.elapsed().as_secs()),
            today: all_time.window(now, 1),
            last_7_days: all_time.window(now, 7),
            last_30_days: all_time.window(now, 30),
            dl_rate: global.dl_rate,
            ul_rate: global.ul_rate,
            torrents_total: torrents.len(),
            torrents_downloading: count(&|t| {
                matches!(t.state, TorrentState::Downloading | TorrentState::StalledDl)
            }),
            torrents_seeding: count(&|t| {
                matches!(t.state, TorrentState::Seeding | TorrentState::StalledUp)
            }),
            torrents_completed: count(&|t| t.progress >= 1.0),
            torrents_stopped: count(&|t| t.state == TorrentState::Stopped),
            torrents_queued: count(&|t| t.state == TorrentState::Queued),
            torrents_errored: count(&|t| t.state == TorrentState::Error),
            total_size: torrents.iter().map(|t| t.size).sum(),
            total_seeding_secs: torrents.iter().map(|t| t.seeding_time).sum(),
            peers_connected: torrents.iter().map(|t| t.peers).sum(),
            peers_known: torrents.iter().map(|t| t.peers_seen).sum(),
            blocked_incoming: session.counters.blocked_incoming,
            blocked_outgoing: session.counters.blocked_outgoing,
            listen_port: global.listen_port,
            read_cache: self.storage.read_cache.stats(),
            write_buffer_used: self.storage.write_budget.used(),
            write_buffer_capacity: cfg.write_buffer_bytes,
            disk: self.storage.metrics.snapshot(),
            all_time,
        }
    }

    pub async fn shutdown(&self) -> anyhow::Result<()> {
        let r = self.persist().await;
        self.session().stop().await;
        r
    }

    // ---------------------------------------------------------------- tick

    /// Holds unfinished downloads while their drive is nearly full.
    fn check_disk_space(&self) {
        {
            let mut last = self.last_disk_check.lock();
            if last.is_some_and(|t| t.elapsed() < DISK_CHECK_EVERY) {
                return;
            }
            *last = Some(Instant::now());
        }
        let min = self.settings.read().min_free_mib.map(|m| m * 1024 * 1024);
        let targets: Vec<(Id20, PathBuf)> = self
            .records
            .read()
            .iter()
            .filter(|(_, r)| r.completed_on.is_none() && r.intent != Intent::Stopped)
            .map(|(h, r)| (*h, r.root.clone()))
            .collect();
        let mut free_by_root: HashMap<PathBuf, Option<u64>> = HashMap::new();
        let mut newly_held = Vec::new();
        let mut rt = self.runtime.lock();
        for (hash, root) in targets {
            let free = *free_by_root
                .entry(root.clone())
                .or_insert_with(|| self.free_space(&root).ok());
            let r = rt.entry(hash).or_default();
            let held = match (min, free) {
                (Some(min), Some(free)) if r.no_space => free < min + DISK_HYSTERESIS,
                (Some(min), Some(free)) => free < min,
                _ => false,
            };
            if held && !r.no_space {
                newly_held.push((root, free.unwrap_or(0)));
            }
            r.no_space = held;
        }
        drop(rt);
        if let Some((root, free)) = newly_held.first() {
            let _ = self.events.send(Event::Notice {
                level: NoticeLevel::Warning,
                message: format!(
                    "Downloads paused: only {} MiB free where {} saves. They resume once there's more room.",
                    free / (1024 * 1024),
                    root.display()
                ),
            });
        }
    }

    fn no_space(&self, hash: &Id20) -> bool {
        self.runtime.lock().get(hash).is_some_and(|r| r.no_space)
    }

    async fn tick(&self) {
        self.check_disk_space();
        let session = self.session();
        let now_unix = unix_now();
        let now = self.epoch.elapsed().as_secs();
        let dt = {
            let mut last = self.last_tick.lock();
            let d = last.elapsed().as_secs_f64().max(0.001);
            *last = Instant::now();
            d
        };
        let handles: Vec<Arc<ManagedTorrent>> =
            session.with_torrents(|it| it.map(|(_, h)| h.clone()).collect());
        let prefs = self.settings.read().seeding.clone();
        let mot = self.settings.read().mark_of_the_web;

        let mut views = Vec::with_capacity(handles.len());
        let mut summaries = Vec::with_capacity(handles.len());
        let mut by_id: HashMap<usize, (Id20, Arc<ManagedTorrent>)> = HashMap::new();
        let mut web_wanted = Vec::new();
        let mut finished = Vec::new();
        let (mut sum_dd, mut sum_ud, mut sum_waste) = (0u64, 0u64, 0u64);
        let mut global = GlobalStats {
            listen_port: session.announce_port(),
            alt_speed: self.settings.read().alt_speed_enabled,
            read_cache: self.storage.read_cache.stats(),
            write_buffer_used: self.storage.write_budget.used(),
            ..Default::default()
        };
        {
            let mut records = self.records.write();
            normalize_queue(&mut records);
            let mut rt = self.runtime.lock();
            for h in handles {
                let hash = h.info_hash();
                let Some(rec) = records.get_mut(&hash) else {
                    continue;
                };
                let stats = h.stats();
                let r = rt.entry(hash).or_default();
                if let Some(live) = h.live() {
                    apply_speed_limits(live.ratelimits(), rec.dl_limit_kib, rec.ul_limit_kib);
                    let key = Arc::as_ptr(&live) as usize;
                    if r.priorities_applied != Some(key) {
                        let len = rec.file_priority.keys().max().map_or(0, |m| m + 1);
                        let mut levels = vec![FilePriority::Normal.level(); len];
                        for (i, p) in &rec.file_priority {
                            levels[*i] = p.level();
                        }
                        live.set_file_priority_levels(&levels);
                        r.priorities_applied = Some(key);
                    }
                }
                if rec.completed_on.is_some() {
                    if let Some(dest) = &rec.move_on_complete {
                        if self.moving.lock().insert(hash) {
                            self.queued_moves.lock().push((hash, dest.clone()));
                        }
                    } else if rec.content_findings.is_none() {
                        self.queue_job(Job::ContentScan(hash));
                    }
                }
                if rec.findings.is_none() {
                    if let Ok(f) = h.with_metadata(|m| name_findings(&m.info, &rec.name)) {
                        rec.findings = Some(f);
                        self.dirty.store(true, Ordering::Relaxed);
                    }
                }
                let running = matches!(stats.state, TorrentStatsState::Live);
                let (fetched, uploaded, checked, peers, seen) = stats
                    .live
                    .as_ref()
                    .map(|l| {
                        let s = &l.snapshot;
                        (
                            s.fetched_bytes,
                            s.uploaded_bytes,
                            s.downloaded_and_checked_bytes,
                            s.peer_stats.live,
                            s.peer_stats.seen,
                        )
                    })
                    .unwrap_or_default();
                // Only exact once nothing is in flight.
                if running && stats.finished {
                    let w = fetched.saturating_sub(checked);
                    sum_waste += w.saturating_sub(r.last_waste);
                    r.last_waste = w;
                } else if !running {
                    r.last_waste = 0;
                }
                let (dd, ud) = if running {
                    (
                        delta(fetched, r.last_fetched),
                        delta(uploaded, r.last_uploaded),
                    )
                } else {
                    (0, 0)
                };
                r.last_fetched = if running { fetched } else { 0 };
                r.last_uploaded = if running { uploaded } else { 0 };
                r.dl_rate = 0.5 * r.dl_rate + 0.5 * (dd as f64 / dt);
                r.ul_rate = 0.5 * r.ul_rate + 0.5 * (ud as f64 / dt);
                if r.dl_rate < 1.0 {
                    r.dl_rate = 0.0;
                }
                if r.ul_rate < 1.0 {
                    r.ul_rate = 0.0;
                }
                match (running, r.running_since) {
                    (true, None) => r.running_since = Some(Instant::now()),
                    (false, Some(_)) => r.running_since = None,
                    _ => {}
                }

                rec.all_time_download += dd;
                rec.all_time_upload += ud;
                sum_dd += dd;
                sum_ud += ud;
                let secs = dt.round() as u64;
                if running {
                    rec.active_time += secs;
                    if stats.finished {
                        rec.seeding_time += secs;
                    }
                }
                if dd + ud > 0 {
                    rec.last_activity = now_unix;
                }
                let verified = !matches!(
                    stats.state,
                    TorrentStatsState::Initializing { .. } | TorrentStatsState::Error
                );
                if stats.finished && rec.completed_on.is_none() && stats.total_bytes > 0 && verified
                {
                    rec.completed_on = Some(now_unix);
                    rec.queue_pos = None;
                    // Not the user's own data being seeded.
                    finished.push((hash, rec.name.clone(), rec.all_time_download > 0));
                } else if !stats.finished && rec.completed_on.is_some() && running {
                    // More files were selected after completion.
                    rec.completed_on = None;
                    rec.queue_pos = Some(u32::MAX);
                }

                global.dl_rate += r.dl_rate as u64;
                global.ul_rate += r.ul_rate as u64;
                let since = rec
                    .last_activity
                    .max(rec.completed_on.unwrap_or(rec.added_on));
                let swarm = crate::swarm::get(&hash);
                views.push(TorrentView {
                    id: h.id(),
                    intent: rec.intent,
                    running,
                    finished: stats.finished,
                    busy_or_error: matches!(
                        stats.state,
                        TorrentStatsState::Initializing { .. } | TorrentStatsState::Error
                    ),
                    queue_pos: rec.queue_pos,
                    total_done: stats.progress_bytes,
                    all_time_upload: rec.all_time_upload,
                    all_time_download: rec.all_time_download,
                    seeding_time: rec.seeding_time,
                    time_since_activity: now_unix.saturating_sub(since),
                    running_for: r.running_since.map_or(0, |s| s.elapsed().as_secs()),
                    dl_rate: r.dl_rate as u64,
                    ul_rate: r.ul_rate as u64,
                    swarm_seeds: swarm.map(|s| s.seeders),
                    limits: rec.limits,
                    limit_action: rec.limit_action,
                });
                let tracker = h
                    .shared()
                    .trackers
                    .iter()
                    .filter_map(|u| u.host_str())
                    .min()
                    .map(str::to_string);
                let mut summary = summarize(rec, &stats, r, peers, seen, tracker);
                summary.swarm_seeds = swarm.map(|s| s.seeders);
                summary.swarm_leechers = swarm.map(|s| s.leechers);
                summary.scrub = self.data_check_progress(&hash);
                summaries.push(summary);
                if running && !stats.finished {
                    if let Some(live) = h.live() {
                        web_wanted.push((hash, h.clone(), live));
                    }
                }
                by_id.insert(h.id(), (hash, h.clone()));
            }
        }

        {
            let mut st = self.statistics.lock();
            st.record_transfer(sum_dd, sum_ud, now_unix);
            st.record_rates(global.dl_rate, global.ul_rate);
            st.all_time_wasted += sum_waste;
            st.torrents_completed += finished.len() as u64;
            let mut t = self.session_tally.lock();
            t.downloaded += sum_dd;
            t.uploaded += sum_ud;
            t.wasted += sum_waste;
            // Carry fractions so uptime never drifts.
            t.uptime += dt;
            let whole = t.uptime.floor();
            st.total_uptime_secs += whole as u64;
            t.uptime -= whole;
        }

        self.sync_web_seeds(&session, web_wanted);

        for (hash, name, downloaded) in finished {
            let _ = self.events.send(Event::Finished {
                id: hash.as_string(),
                name,
            });
            self.dirty.store(true, Ordering::Relaxed);
            self.queue_job(Job::Checksums(hash));
            if mot && downloaded {
                if let Ok(files) = self.files(&hash.as_string()) {
                    let root = self.factory.root_of(&hash);
                    tokio::task::spawn_blocking(move || {
                        let Some(root) = root else { return };
                        for f in files.iter().filter(|f| f.selected) {
                            let _ = motw::mark(&root.path.join(&f.path));
                        }
                    });
                }
            }
        }

        let actions = self.scheduler.lock().tick(&views, &prefs, now);
        // A confirmed leak holds torrents like the kill switch does.
        let blocked = matches!(*self.network.read(), NetworkMode::Blocked) || self.leak_held();
        let hold = self.hold();
        let finished: HashSet<usize> = views.iter().filter(|v| v.finished).map(|v| v.id).collect();
        let held =
            |id: usize| hold.all.is_some() || (hold.downloads.is_some() && !finished.contains(&id));
        for a in actions {
            if (blocked || held(a.id())) && matches!(a, Action::Start { .. }) {
                continue;
            }
            if let Action::Start { id } = &a {
                if by_id.get(id).is_some_and(|(h, _)| self.no_space(h)) {
                    continue;
                }
            }
            if let Err(e) = self.apply(a, &by_id).await {
                warn!("scheduler action failed: {e:#}");
            }
        }
        for (id, (hash, h)) in &by_id {
            if (blocked || held(*id) || self.no_space(hash)) && !h.is_paused() {
                let _ = session.pause(h).await;
            }
        }
        global.network = self.network.read().label().to_string();
        global.hold = hold.all.or(hold.downloads);
        global.data_cap = self.data_cap_usage();
        global.untrusted_network = self.untrusted_network();

        *self.last_tick_data.lock() = (summaries.clone(), global.clone());
        let _ = self.events.send(Event::Tick {
            torrents: summaries,
            global,
        });

        let n = {
            let mut t = self.ticks.lock();
            *t += 1;
            *t
        };
        if n % PERSIST_EVERY_TICKS == 0 || self.dirty.load(Ordering::Relaxed) {
            // Written off the runtime; a failed write stays queued for next time.
            match self.queue_state() {
                Ok(()) => self.saver.flush_soon(),
                Err(e) => warn!("saving torrent state failed: {e:#}"),
            }
        }
        if n % 10 == 3 {
            self.schedule_countries();
        }
        if n % 60 == 30 {
            self.schedule_data_check();
        }
        if n % 15 == 1 {
            self.queue_job(Job::NetInfo);
        }
        if n % 5 == 0 {
            self.check_swarm_view();
        }
        if n % 15 == 0 {
            self.follow_alt_schedule();
        }
        if n % WATCH_EVERY_TICKS == 0 {
            self.scan_watched_folder().await;
        }
    }

    /// Only acts when the window opens or closes, so manual toggles stick.
    fn follow_alt_schedule(&self) {
        use chrono::{Datelike, Timelike};
        let sched = self.settings.read().alt_schedule;
        if !sched.enabled {
            *self.alt_window.lock() = None;
            return;
        }
        let now = chrono::Local::now();
        let inside = sched.active(
            now.weekday().num_days_from_monday(),
            (now.hour() * 60 + now.minute()) as u16,
        );
        if self.alt_window.lock().replace(inside) == Some(inside) {
            return;
        }
        if self.settings.read().alt_speed_enabled != inside {
            match self.set_alt_speed(inside) {
                Ok(()) => info!(
                    "speed schedule: alternative limits {}",
                    if inside { "on" } else { "off" }
                ),
                Err(e) => warn!("speed schedule: {e:#}"),
            }
        }
    }

    async fn scan_watched_folder(&self) {
        let folders = self.settings.read().watched_folders.clone();
        for w in folders {
            self.scan_one(&w).await;
        }
    }

    async fn scan_one(&self, w: &crate::settings::WatchedFolder) {
        let dir = w.path.clone();
        let found = tokio::task::spawn_blocking(move || crate::watch::scan(&dir))
            .await
            .unwrap_or_default();
        for path in found {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let add_stopped = self.settings.read().add_stopped;
            let added = match self.preview(Source::File(path.clone())).await {
                Ok(p) if p.already_added => Ok(()),
                Ok(p) => self
                    .add(AddParams {
                        info_hash: p.info_hash,
                        start_paused: add_stopped,
                        category: w.category.clone(),
                        save_path: w.save_path.clone(),
                        ..Default::default()
                    })
                    .await
                    .map(|_| ()),
                Err(e) => Err(e),
            };
            match &added {
                Ok(()) => info!("added {name} from the watched folder"),
                Err(e) => {
                    warn!("watched folder: {name}: {e:#}");
                    self.notice(
                        NoticeLevel::Warning,
                        format!("Couldn't add {name} from the watched folder: {e:#}"),
                    );
                }
            }
            if let Err(e) = crate::watch::mark(&path, added.is_ok()) {
                // Without the rename it would be retried forever.
                warn!("watched folder: can't rename {name}: {e}; stopping the scan");
                self.notice(
                    NoticeLevel::Warning,
                    format!("Can't rename {name} in {}, so Riptor stopped watching it. Check the folder's permissions, then add it again in Settings.", w.path.display()),
                );
                self.settings
                    .write()
                    .watched_folders
                    .retain(|x| x.path != w.path);
                return;
            }
        }
    }

    /// Workers that gave up stay off until the torrent restarts.
    fn sync_web_seeds(
        &self,
        session: &Arc<Session>,
        wanted: Vec<(Id20, Arc<ManagedTorrent>, Arc<librqbit::TorrentStateLive>)>,
    ) {
        let net = self.network.read().clone();
        let local = match &net {
            NetworkMode::Normal => None,
            NetworkMode::Vpn(v) => Some(std::net::IpAddr::V4(v.local_ip)),
            NetworkMode::Blocked => {
                self.web_seeds.lock().clear();
                return;
            }
        };
        let mut running = self.web_seeds.lock();
        let s = self.settings.read();
        // They'd bypass the proxy, and the client name shows in requests.
        let off = !s.web_seeds || s.proxy_url.is_some() || s.anonymous_mode;
        drop(s);
        if off {
            running.clear();
            return;
        }
        let keep: HashSet<Id20> = wanted.iter().map(|w| w.0).collect();
        running.retain(|h, _| keep.contains(h));
        if wanted
            .iter()
            .all(|(h, _, live)| running.get(h).is_some_and(|r| r.is_for(live)))
        {
            return;
        }
        let allow_local = self.allow_local_web_seeds.load(Ordering::Relaxed);
        let http = {
            let mut c = self.web_seed_http.lock();
            if c.is_none() {
                match webseed::client(local, allow_local) {
                    Ok(client) => *c = Some(client),
                    Err(e) => {
                        warn!("web seeds unavailable: {e:#}");
                        return;
                    }
                }
            }
            c.clone().expect("just set")
        };
        let mut info = self.web_seed_info.lock();
        info.retain(|h, _| self.records.read().contains_key(h));
        for (hash, h, live) in wanted {
            if running.get(&hash).is_some_and(|r| r.is_for(&live)) {
                continue;
            }
            let found = info.entry(hash).or_insert_with(|| {
                h.with_metadata(|m| {
                    let seeds = webseed::url_list(&m.torrent_bytes);
                    if seeds.is_empty()
                        || m.info.lengths().default_piece_length() > webseed::MAX_PIECE
                    {
                        return None;
                    }
                    let multi = m.info.info().files.is_some();
                    webseed::Layout::new(&m.info, multi).map(|l| (Arc::new(l), seeds))
                })
                .ok()
                .flatten()
            });
            let Some((layout, seeds)) = found.clone() else {
                continue;
            };
            let job = webseed::Job {
                live,
                layout,
                http: http.clone(),
                session: session.clone(),
                allow_local,
            };
            running.insert(hash, webseed::start(job, &seeds));
        }
    }

    /// Tests only: lets web seeds on 127.0.0.1 through.
    #[doc(hidden)]
    pub fn allow_local_web_seeds(&self) {
        self.allow_local_web_seeds.store(true, Ordering::Relaxed);
        *self.web_seed_http.lock() = None;
    }

    async fn apply(
        &self,
        a: Action,
        by_id: &HashMap<usize, (Id20, Arc<ManagedTorrent>)>,
    ) -> anyhow::Result<()> {
        let session = self.session();
        match a {
            Action::Start { id } => {
                if let Some((_, h)) = by_id.get(&id) {
                    session.unpause(h).await?;
                }
            }
            Action::Queue { id } => {
                if let Some((_, h)) = by_id.get(&id) {
                    session.pause(h).await?;
                }
            }
            Action::ShareLimit { id, action, reason } => {
                let Some((hash, h)) = by_id.get(&id) else {
                    return Ok(());
                };
                let name = self
                    .records
                    .read()
                    .get(hash)
                    .map(|r| r.name.clone())
                    .unwrap_or_default();
                info!(?reason, ?action, "share limit reached for {name}");
                match action {
                    ShareLimitAction::Stop => {
                        if let Some(r) = self.records.write().get_mut(hash) {
                            r.intent = Intent::Stopped;
                        }
                        self.dirty.store(true, Ordering::Relaxed);
                        session.pause(h).await?;
                    }
                    ShareLimitAction::Remove => self.remove_one(*hash, false).await?,
                    ShareLimitAction::RemoveWithFiles => self.remove_one(*hash, true).await?,
                }
                self.notice(
                    NoticeLevel::Info,
                    format!("{name}: share limit reached ({action:?})"),
                );
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum QueueMove {
    Up,
    Down,
    Top,
    Bottom,
}

/// Shown in the UI and used in paths.
/// Unbroken bytes on disk from the start of each file.
fn ready_from_start(have: &[bool], m: &librqbit::TorrentMetadata) -> Vec<u64> {
    let piece_len = u64::from(m.info.lengths().default_piece_length());
    m.file_infos
        .iter()
        .map(|f| {
            let (start, end) = (f.piece_range.start as usize, f.piece_range.end as usize);
            let n = (start..end)
                .take_while(|p| have.get(*p).copied().unwrap_or(false))
                .count();
            if n == end - start {
                return f.len;
            }
            ((start + n) as u64 * piece_len)
                .saturating_sub(f.offset_in_torrent)
                .min(f.len)
        })
        .collect()
}

/// Fake-release checks on the raw file names (before sanitizing hides the tricks).
fn name_findings(
    info: &librqbit_core::torrent_metainfo::ValidatedTorrentMetaV1Info<impl AsRef<[u8]>>,
    name: &str,
) -> Vec<crate::suspicion::Finding> {
    let files: Vec<(String, u64)> = info
        .iter_file_details()
        .filter(|fd| !fd.attrs().padding)
        .map(|fd| {
            let parts: Vec<String> = fd
                .filename
                .iter_components()
                .map(|c| c.into_owned())
                .collect();
            (parts.join("/"), fd.len)
        })
        .collect();
    crate::suspicion::check_names(name, &files)
}

/// A user-typed relative path: real names only, nothing that sanitizing would change.
fn clean_rel_path(rel: &str) -> anyhow::Result<String> {
    let parts: Vec<&str> = rel
        .split(['/', '\\'])
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .collect();
    ensure!(!parts.is_empty(), "the name can't be empty");
    ensure!(parts.len() <= 64, "too many folders");
    for p in &parts {
        ensure!(*p != "." && *p != "..", "names can't be . or ..");
        ensure!(
            sanitize_component(p) == *p,
            "\"{p}\" can't be used as a file or folder name"
        );
    }
    Ok(parts.join("/"))
}

fn validate_label(s: &str) -> anyhow::Result<()> {
    ensure!(
        !s.trim().is_empty() && s.len() <= 64,
        "names must be 1-64 characters"
    );
    ensure!(
        !s.chars().any(|c| c.is_control() || matches!(c, '/' | '\\')),
        "names cannot contain slashes"
    );
    Ok(())
}

fn normalize_queue(records: &mut HashMap<Id20, Record>) {
    let mut q: Vec<(u32, u64, Id20)> = records
        .iter()
        .filter(|(_, r)| r.completed_on.is_none())
        .map(|(h, r)| (r.queue_pos.unwrap_or(u32::MAX), r.added_on, *h))
        .collect();
    q.sort();
    for (i, (_, _, h)) in q.into_iter().enumerate() {
        if let Some(r) = records.get_mut(&h) {
            r.queue_pos = Some(i as u32);
        }
    }
}

fn summarize(
    rec: &Record,
    stats: &librqbit::TorrentStats,
    r: &Runtime,
    peers: u32,
    seen: u32,
    tracker: Option<String>,
) -> TorrentSummary {
    let state = match stats.state {
        TorrentStatsState::Error => TorrentState::Error,
        TorrentStatsState::Initializing { .. } => TorrentState::Checking,
        TorrentStatsState::Paused => match rec.intent {
            Intent::Stopped => TorrentState::Stopped,
            _ if r.no_space => TorrentState::NoSpace,
            _ => TorrentState::Queued,
        },
        TorrentStatsState::Live if stats.finished => {
            if r.ul_rate > 0.0 {
                TorrentState::Seeding
            } else {
                TorrentState::StalledUp
            }
        }
        TorrentStatsState::Live => {
            if r.dl_rate > 0.0 {
                TorrentState::Downloading
            } else {
                TorrentState::StalledDl
            }
        }
    };
    let remaining = stats.total_bytes.saturating_sub(stats.progress_bytes);
    TorrentSummary {
        id: rec.info_hash.clone(),
        name: rec.name.clone(),
        state,
        forced: rec.intent == Intent::Forced,
        progress: if stats.total_bytes == 0 {
            0.0
        } else {
            stats.progress_bytes as f64 / stats.total_bytes as f64
        },
        size: stats.total_bytes,
        done: stats.progress_bytes,
        total_size: stats.total_bytes,
        dl_rate: r.dl_rate as u64,
        ul_rate: r.ul_rate as u64,
        eta: (r.dl_rate > 0.0 && remaining > 0).then(|| (remaining as f64 / r.dl_rate) as u64),
        ratio: riptor_seeding::real_ratio(
            rec.all_time_upload,
            rec.all_time_download,
            stats.progress_bytes,
        ),
        uploaded: rec.all_time_upload,
        downloaded: rec.all_time_download,
        peers,
        peers_seen: seen,
        category: rec.category.clone(),
        tags: rec.tags.clone(),
        save_path: rec.root.to_string_lossy().into_owned(),
        tracker,
        added_on: rec.added_on,
        completed_on: rec.completed_on,
        seeding_time: rec.seeding_time,
        queue_pos: rec.queue_pos,
        limits: rec.limits,
        limit_action: rec.limit_action,
        has_executables: rec.has_executables,
        dl_limit_kib: rec.dl_limit_kib,
        ul_limit_kib: rec.ul_limit_kib,
        error: stats.error.clone(),
        checksum: rec
            .checksums
            .as_ref()
            .filter(|c| c.verified + c.mismatched_count > 0)
            .map(|c| c.mismatched_count == 0),
        suspicion: rec.suspicion(),
        // Filled in by the ticker.
        swarm_seeds: None,
        swarm_leechers: None,
        scrub: None,
        last_scrub: rec.last_scrub,
    }
}

async fn build_session(
    data_dir: &Path,
    s: &Settings,
    factory: &CachedStorageFactory,
    net: &NetworkMode,
) -> anyhow::Result<Arc<Session>> {
    let anon = s.anonymous_mode;
    let blocked = matches!(net, NetworkMode::Blocked);
    let vpn = match net {
        NetworkMode::Vpn(v) => Some(v),
        _ => None,
    };
    let listen_ip = vpn
        .map(|v| std::net::IpAddr::V4(v.local_ip))
        .or(s.listen_address);
    // On the VPN, tracker and DHT lookups go through the tunnel too.
    let resolver = vpn
        .map(|v| crate::dns::TunnelResolver::new(v.local_ip.into(), &crate::dns::SERVERS))
        .transpose()
        .context("setting up DNS through the VPN")?;
    crate::dns::install(resolver.as_ref());
    let dht = (s.dht && !anon && !blocked).then(|| DhtSessionConfig {
        persistence: Some(DhtPersistenceConfig {
            config_filename: Some(data_dir.join("dht.json")),
            ..Default::default()
        }),
        ..Default::default()
    });
    let listen = ListenerOptions {
        mode: match s.transport {
            Transport::Tcp => ListenerMode::TcpOnly,
            Transport::TcpAndUtp => ListenerMode::TcpAndUtp,
        },
        listen_addr: (
            listen_ip.unwrap_or(Ipv6Addr::UNSPECIFIED.into()),
            s.listen_port,
        )
            .into(),
        ipv4_only: listen_ip.is_some_and(|a| a.is_ipv4()),
        // UPnP would open a port on the home router.
        enable_upnp_port_forwarding: s.upnp && !anon && vpn.is_none(),
        announce_port: vpn.and_then(|v| v.forwarded_port),
        ..Default::default()
    };
    let (dl, ul) = s.effective_limits();
    let blocklist_file = match (&s.blocklist_file, &s.blocklist_url) {
        (Some(f), _) => Some(f.clone()),
        (None, Some(u)) => crate::blocklist::fetch(data_dir, u, net).await,
        (None, None) => None,
    };
    let opts = SessionOptions {
        dht,
        persistence: Some(SessionPersistenceConfig::Json {
            folder: Some(data_dir.join("session")),
        }),
        fastresume: true,
        listen: (!blocked).then_some(listen),
        connect: Some(ConnectionOptions {
            proxy_url: s.proxy_url.clone(),
            enable_tcp: true,
            peer_opts: None,
            encryption: match s.encryption {
                crate::settings::PeerEncryption::Off => librqbit::EncryptionPolicy::Disabled,
                crate::settings::PeerEncryption::Prefer => librqbit::EncryptionPolicy::Prefer,
                crate::settings::PeerEncryption::Require => librqbit::EncryptionPolicy::Require,
            },
        }),
        disable_trackers: blocked,
        // Pinned to loopback, so anything that slips through fails in the OS.
        bind_device_name: match net {
            NetworkMode::Vpn(v) => Some(v.adapter.clone()),
            NetworkMode::Blocked => crate::network::loopback_adapter(),
            NetworkMode::Normal => None,
        },
        bind_local_ip: vpn.map(|v| v.local_ip.into()),
        dns_resolver: resolver.as_ref().map(crate::dns::TunnelResolver::hook),
        ipv4_only: vpn.is_some(),
        default_storage_factory: Some(factory.clone().into_boxed()),
        ratelimits: LimitsConfig {
            download_bps: kib(dl),
            upload_bps: kib(ul),
        },
        // librqbit would fetch a URL itself, outside the tunnel.
        blocklist_url: None,
        blocklist_file,
        peer_limit: s.peer_limit,
        disable_local_service_discovery: !s.lsd || anon || vpn.is_some() || blocked,
        client_name_and_version: anon.then(|| "BitTorrent".to_string()),
        ..Default::default()
    };
    let session = Session::new_with_opts(s.download_dir.clone(), opts)
        .await
        .context("starting the torrent session")?;
    session.set_banned(s.banned_ips.iter().copied().collect());
    Ok(session)
}

fn spawn_ticker(engine: Weak<Engine>) {
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(TICK);
        iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            iv.tick().await;
            let Some(e) = engine.upgrade() else { break };
            e.tick().await;
            for (hash, dest) in e.take_queued_moves() {
                let e = e.clone();
                tokio::spawn(async move { e.finish_move(hash, dest).await });
            }
            for job in e.take_jobs() {
                let e = e.clone();
                tokio::spawn(async move { e.run_job(job).await });
            }
        }
    });
}

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;

#[derive(Clone, Copy, Debug, serde::Deserialize)]
pub struct RestoreOptions {
    pub settings: bool,
    pub torrents: bool,
    pub statistics: bool,
    /// Otherwise everything comes back stopped.
    pub resume_torrents: bool,
}

#[derive(Clone, Debug, Default, serde::Serialize)]
pub struct RestoreReport {
    pub added: usize,
    pub already_present: usize,
    pub settings_restored: bool,
    pub statistics_restored: bool,
    pub skipped: Vec<String>,
}

#[derive(Clone, Debug, serde::Serialize)]
pub struct BackupSummary {
    pub created: u64,
    pub app_version: String,
    pub password_protected: bool,
    pub torrents: usize,
    pub torrent_names: Vec<String>,
    pub save_folders: Vec<String>,
    pub categories: usize,
    pub all_time_upload: u64,
    pub all_time_download: u64,
}

fn summarize_backup(p: &backup::BackupPayload, protected: bool) -> BackupSummary {
    let mut folders: Vec<String> = p
        .torrents
        .iter()
        .map(|t| t.record.root.to_string_lossy().into_owned())
        .collect();
    folders.sort();
    folders.dedup();
    BackupSummary {
        created: p.created,
        app_version: p.app_version.chars().take(32).collect(),
        password_protected: protected,
        torrents: p.torrents.len(),
        torrent_names: p
            .torrents
            .iter()
            .take(50)
            .map(|t| t.record.name.chars().take(200).collect())
            .collect(),
        save_folders: folders.into_iter().take(50).collect(),
        categories: p.categories.len(),
        all_time_upload: p.statistics.all_time_upload,
        all_time_download: p.statistics.all_time_download,
    }
}

/// Only touches the limiter when the setting changed.
fn apply_speed_limits(l: &librqbit::limits::Limits, dl_kib: Option<u32>, ul_kib: Option<u32>) {
    let (dl, ul) = (kib(dl_kib), kib(ul_kib));
    if l.get_download_bps() != dl {
        l.set_download_bps(dl);
    }
    if l.get_upload_bps() != ul {
        l.set_upload_bps(ul);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn speed_limits_reach_the_live_limiter() {
        let l = librqbit::limits::Limits::new(Default::default());
        apply_speed_limits(&l, Some(512), Some(64));
        assert_eq!(l.get_download_bps().map(|v| v.get()), Some(512 * 1024));
        assert_eq!(l.get_upload_bps().map(|v| v.get()), Some(64 * 1024));
        apply_speed_limits(&l, None, Some(64));
        assert_eq!(l.get_download_bps(), None);
        assert_eq!(l.get_upload_bps().map(|v| v.get()), Some(64 * 1024));
    }
}
