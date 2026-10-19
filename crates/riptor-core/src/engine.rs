//! The engine: one librqbit session plus everything Riptor adds on top —
//! hardened storage, persistent all-time counters, and qBittorrent-style
//! seeding/queueing applied once a second.

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
use crate::settings::{Settings, Transport};
use crate::stats::{self, Statistics, StatsReport};
use crate::{backup, motw, paths, state_file};

const TICK: Duration = Duration::from_secs(1);
const PERSIST_EVERY_TICKS: u64 = 15;
const METADATA_TIMEOUT: Duration = Duration::from_secs(180);
const MAX_PENDING_PREVIEWS: usize = 32;

pub struct EngineConfig {
    /// App data folder: settings, Riptor state, librqbit session and DHT files.
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
    at: Instant,
}

#[derive(Default)]
struct Runtime {
    last_fetched: u64,
    last_uploaded: u64,
    /// fetched - verified for this live session, last seen (finished torrents only).
    last_waste: u64,
    dl_rate: f64,
    ul_rate: f64,
    running_since: Option<Instant>,
}

pub struct Engine {
    data_dir: PathBuf,
    session: RwLock<Arc<Session>>,
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
    /// Serialises session rebuilds and torrent removals.
    admin: tokio::sync::Mutex<()>,
    statistics: Mutex<Statistics>,
    session_tally: Mutex<SessionTally>,
    last_tick_data: Mutex<(Vec<TorrentSummary>, GlobalStats)>,
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
        let data_dir = cfg.data_dir;
        let mut settings: Settings = state_file::load(&data_dir.join("settings.json"))?;
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
                        },
                    );
                    records.insert(h, r);
                }
                Err(e) => warn!("dropping corrupt torrent record: {e:#}"),
            }
        }

        let session = build_session(&data_dir, &settings, &factory).await?;
        let (events, _) = broadcast::channel(64);
        let engine = Arc::new(Self {
            data_dir,
            session: RwLock::new(session),
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
            admin: tokio::sync::Mutex::new(()),
            statistics: Mutex::new(statistics),
            session_tally: Mutex::new(SessionTally {
                started: Some(Instant::now()),
                ..Default::default()
            }),
            last_tick_data: Mutex::new(Default::default()),
        });
        engine.apply_limits();
        engine.forget_orphans().await;
        spawn_ticker(Arc::downgrade(&engine));
        Ok(engine)
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

    /// Torrents librqbit restored but Riptor has no record of (e.g. state file
    /// lost) cannot be placed safely; drop them without touching files.
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

    /// Resolve a source and describe it for the add dialog. Magnet links fetch
    /// metadata from peers, which can take a while.
    pub async fn preview(&self, src: Source) -> anyhow::Result<Preview> {
        let (add, mut trackers) = match input::resolve(&src, &self.http).await? {
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
                    path: rel,
                    size: f.len,
                    done: 0,
                    selected: lo.only_files.as_ref().is_none_or(|o| o.contains(&i)),
                }
            })
            .collect();
        let has_executables = files.iter().any(|f| f.executable);
        let already_added = self.records.read().contains_key(&lo.info_hash);

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
                at: Instant::now(),
            },
        );
        Ok(preview)
    }

    /// Commit a previewed torrent.
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
        let (root, owned) = if subfolder {
            (base.join(sanitize_component(&p.name)), true)
        } else {
            (base.clone(), false)
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

        self.factory.register(
            hash,
            TorrentRoot {
                path: root.clone(),
                owned,
            },
        );
        let opts = AddTorrentOptions {
            paused: true, // the queue decides when it actually starts
            overwrite: true,
            output_folder: Some(root_str),
            only_files: params.only_files.clone(),
            trackers: (!p.trackers.is_empty()).then(|| p.trackers.clone()),
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
                intent: if params.start_paused {
                    Intent::Stopped
                } else {
                    Intent::Started
                },
                has_executables: p.has_executables,
            },
        );
        drop(records);
        self.statistics.lock().torrents_added += 1;
        self.persist_now()?;
        info!("added {}", p.name);
        Ok(hash.as_string())
    }

    /// Preview then add with defaults (watched folder, tests).
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

    /// Back into the queue; the scheduler starts them when a slot is free.
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
        let result = self
            .session()
            .delete(TorrentIdOrHash::Hash(hash), delete_files)
            .await;
        if self.records.write().remove(&hash).is_some() {
            self.statistics.lock().torrents_removed += 1;
        }
        self.runtime.lock().remove(&hash);
        self.factory.unregister(&hash);
        self.persist_now()?;
        let _ = self.events.send(Event::Removed {
            id: hash.as_string(),
        });
        result
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

    /// Ids of every torrent in the list (for "pause all" / "resume all").
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
        self.persist_now()
    }

    pub fn remove_category_def(&self, name: &str) -> anyhow::Result<()> {
        self.categories.write().remove(name);
        for r in self.records.write().values_mut() {
            if r.category.as_deref() == Some(name) {
                r.category = None;
            }
        }
        self.persist_now()
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
        t.with_metadata(|m| -> anyhow::Result<Vec<FileEntry>> {
            let layout = layout_for(&root, m)?;
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
                        path: rel,
                        size: f.len,
                        done: stats.file_progress.get(i).copied().unwrap_or(0),
                        selected: only.as_ref().is_none_or(|o| o.contains(&i)),
                    }
                })
                .collect())
        })?
    }

    pub fn trackers(&self, id: &str) -> anyhow::Result<Vec<String>> {
        let t = self.handle(&parse_hash(id)?)?;
        let mut v: Vec<String> = t.shared().trackers.iter().map(|u| u.to_string()).collect();
        v.sort();
        Ok(v)
    }

    pub fn peers(&self, id: &str) -> anyhow::Result<serde_json::Value> {
        let t = self.handle(&parse_hash(id)?)?;
        match t.live() {
            Some(live) => Ok(serde_json::to_value(
                live.per_peer_stats_snapshot(Default::default()),
            )?),
            None => Ok(serde_json::Value::Null),
        }
    }

    /// Folder to reveal in the file manager for this torrent.
    pub fn content_path(&self, id: &str) -> anyhow::Result<PathBuf> {
        let hash = parse_hash(id)?;
        Ok(self.factory.root_of(&hash).context("unknown torrent")?.path)
    }

    // ------------------------------------------------------------ settings

    pub fn settings(&self) -> Settings {
        self.settings.read().clone()
    }

    pub async fn update_settings(&self, new: Settings) -> anyhow::Result<()> {
        new.validate()?;
        self.check_root(&new.download_dir)?;
        let old = self.settings();
        state_file::save(&self.data_dir.join("settings.json"), &new)?;
        *self.settings.write() = new.clone();
        self.storage.reconfigure(new.storage.clone());
        self.apply_limits();
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

    async fn restart_session(&self) -> anyhow::Result<()> {
        let _g = self.admin.lock().await;
        let old = self.session();
        old.stop().await;
        let settings = self.settings();
        let new = build_session(&self.data_dir, &settings, &self.factory).await?;
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

    fn persist_now(&self) -> anyhow::Result<()> {
        self.dirty.store(false, Ordering::Relaxed);
        let stats = self.statistics.lock().clone();
        state_file::save(&self.data_dir.join("statistics.json"), &stats)?;
        state_file::save(&self.data_dir.join("torrents.json"), &self.snapshot_state())
    }

    // -------------------------------------------------------------- backup

    fn check_root(&self, p: &Path) -> anyhow::Result<()> {
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

    /// Write an encrypted backup of settings, torrents, categories and statistics.
    pub fn create_backup(
        &self,
        path: &Path,
        password: Option<String>,
    ) -> anyhow::Result<BackupSummary> {
        Self::check_backup_path(path)?;
        // Never overwrite something that is not itself a Riptor backup.
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

    /// Restore from a backup. Its contents are treated as untrusted input:
    /// a password-less backup can be forged by anyone who extracts the
    /// built-in key, so every value is validated again before use.
    pub async fn restore_backup(
        &self,
        path: &Path,
        password: Option<String>,
        opts: RestoreOptions,
    ) -> anyhow::Result<RestoreReport> {
        let (payload, _) = Self::read_backup(path, password.as_deref())?;
        let mut report = RestoreReport::default();

        // Categories first (torrents may refer to them).
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
        self.persist_now()?;
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
        if !resume {
            rec.intent = Intent::Stopped;
        }
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

    /// Everything for the statistics page.
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
        let r = self.persist_now();
        self.session().stop().await;
        r
    }

    // ---------------------------------------------------------------- tick

    async fn tick(&self) {
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
                // Waste is only exact once nothing is in flight, i.e. the torrent is finished.
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
                    // Only tag files we actually downloaded, not a user's own data being seeded.
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
                    limits: rec.limits,
                    limit_action: rec.limit_action,
                });
                summaries.push(summarize(rec, &stats, r, peers, seen));
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

        for (hash, name, downloaded) in finished {
            let _ = self.events.send(Event::Finished {
                id: hash.as_string(),
                name,
            });
            self.dirty.store(true, Ordering::Relaxed);
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
        for a in actions {
            if let Err(e) = self.apply(a, &by_id).await {
                warn!("scheduler action failed: {e:#}");
            }
        }

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
            if let Err(e) = self.persist_now() {
                warn!("saving torrent state failed: {e:#}");
            }
        }
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

/// Categories and tags are shown in the UI and used in paths; keep them tame.
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

/// Keep queue positions dense (0..n) over unfinished torrents.
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
) -> TorrentSummary {
    let state = match stats.state {
        TorrentStatsState::Error => TorrentState::Error,
        TorrentStatsState::Initializing { .. } => TorrentState::Checking,
        TorrentStatsState::Paused => match rec.intent {
            Intent::Stopped => TorrentState::Stopped,
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
        added_on: rec.added_on,
        completed_on: rec.completed_on,
        seeding_time: rec.seeding_time,
        queue_pos: rec.queue_pos,
        limits: rec.limits,
        limit_action: rec.limit_action,
        has_executables: rec.has_executables,
        error: stats.error.clone(),
    }
}

async fn build_session(
    data_dir: &Path,
    s: &Settings,
    factory: &CachedStorageFactory,
) -> anyhow::Result<Arc<Session>> {
    let anon = s.anonymous_mode;
    let dht = (s.dht && !anon).then(|| DhtSessionConfig {
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
            s.listen_address.unwrap_or(Ipv6Addr::UNSPECIFIED.into()),
            s.listen_port,
        )
            .into(),
        ipv4_only: s.listen_address.is_some_and(|a| a.is_ipv4()),
        enable_upnp_port_forwarding: s.upnp && !anon,
        ..Default::default()
    };
    let (dl, ul) = s.effective_limits();
    let opts = SessionOptions {
        dht,
        persistence: Some(SessionPersistenceConfig::Json {
            folder: Some(data_dir.join("session")),
        }),
        fastresume: true,
        listen: Some(listen),
        connect: Some(ConnectionOptions {
            proxy_url: s.proxy_url.clone(),
            enable_tcp: true,
            peer_opts: None,
        }),
        default_storage_factory: Some(factory.clone().into_boxed()),
        ratelimits: LimitsConfig {
            download_bps: kib(dl),
            upload_bps: kib(ul),
        },
        blocklist_url: s.blocklist_url.clone(),
        peer_limit: s.peer_limit,
        disable_local_service_discovery: !s.lsd || anon,
        client_name_and_version: anon.then(|| "BitTorrent".to_string()),
        ..Default::default()
    };
    Session::new_with_opts(s.download_dir.clone(), opts)
        .await
        .context("starting the torrent session")
}

fn spawn_ticker(engine: Weak<Engine>) {
    tokio::spawn(async move {
        let mut iv = tokio::time::interval(TICK);
        iv.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            iv.tick().await;
            let Some(e) = engine.upgrade() else { break };
            e.tick().await;
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
    /// Keep each torrent's running/stopped state. When false every restored
    /// torrent starts stopped so the user can review its save folder first.
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
