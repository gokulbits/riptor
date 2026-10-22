//! Riptor's data in qBittorrent's Web API shapes. "No date" is -1, as the API does.

use riptor_core::{
    FileEntry, FilePriority, GlobalStats, Limit, PeerInfo, TorrentState, TorrentSummary,
};
use serde_json::{json, Map, Value};

/// What `app/version` reports; 5.x makes clients use start/stop and the stopped states.
pub const VERSION: &str = "v5.0.0";
pub const WEBAPI_VERSION: &str = "2.11.2";
const NO_ETA: u64 = 8_640_000;

fn state(t: &TorrentSummary) -> &'static str {
    let done = t.progress >= 1.0;
    match t.state {
        TorrentState::Metadata => "metaDL",
        TorrentState::Checking if done => "checkingUP",
        TorrentState::Checking => "checkingDL",
        TorrentState::Downloading if t.forced => "forcedDL",
        TorrentState::Downloading => "downloading",
        TorrentState::StalledDl => "stalledDL",
        TorrentState::Seeding if t.forced => "forcedUP",
        TorrentState::Seeding => "uploading",
        TorrentState::StalledUp => "stalledUP",
        TorrentState::Queued if done => "queuedUP",
        TorrentState::Queued => "queuedDL",
        TorrentState::Stopped | TorrentState::NoSpace if done => "stoppedUP",
        TorrentState::Stopped | TorrentState::NoSpace => "stoppedDL",
        TorrentState::Error => "error",
    }
}

fn ratio_limit(l: Limit<f64>) -> f64 {
    match l {
        Limit::UseGlobal => -2.0,
        Limit::Unlimited => -1.0,
        Limit::Value(v) => v,
    }
}

fn minutes_limit(l: Limit<u64>) -> i64 {
    match l {
        Limit::UseGlobal => -2,
        Limit::Unlimited => -1,
        Limit::Value(v) => v.min(i64::MAX as u64) as i64,
    }
}

/// -2 means "the global limit", -1 none; out-of-range values are refused.
pub fn parse_ratio_limit(v: f64) -> Option<Limit<f64>> {
    if v == -2.0 {
        Some(Limit::UseGlobal)
    } else if v == -1.0 {
        Some(Limit::Unlimited)
    } else if v.is_finite() && (0.0..=10_000.0).contains(&v) {
        Some(Limit::Value(v))
    } else {
        None
    }
}

pub fn parse_minutes_limit(v: i64) -> Option<Limit<u64>> {
    match v {
        -2 => Some(Limit::UseGlobal),
        -1 => Some(Limit::Unlimited),
        0..=52_560_000 => Some(Limit::Value(v as u64)),
        _ => None,
    }
}

fn bytes_limit(kib: Option<u32>) -> i64 {
    kib.map_or(-1, |k| i64::from(k) * 1024)
}

pub fn torrent(t: &TorrentSummary, magnet: &str) -> Value {
    json!({
        "hash": t.id,
        "infohash_v1": t.id,
        "infohash_v2": "",
        "name": t.name,
        "state": state(t),
        "added_on": t.added_on,
        "completion_on": t.completed_on.map_or(-1, |c| c as i64),
        "completed": t.done,
        "size": t.size,
        "total_size": t.total_size,
        "eta": t.eta.unwrap_or(NO_ETA).min(NO_ETA),
        "dlspeed": t.dl_rate,
        "upspeed": t.ul_rate,
        "dl_limit": bytes_limit(t.dl_limit_kib),
        "up_limit": bytes_limit(t.ul_limit_kib),
        "progress": t.progress,
        "priority": t.queue_pos.map_or(0, |p| i64::from(p) + 1),
        "num_seeds": 0,
        "num_leechs": t.peers,
        "num_complete": t.swarm_seeds.unwrap_or(0),
        "num_incomplete": t.swarm_leechers.unwrap_or(0),
        "save_path": t.save_path,
        "content_path": t.save_path,
        "download_path": "",
        "category": t.category.clone().unwrap_or_default(),
        "tags": t.tags.join(", "),
        "seq_dl": false,
        "f_l_piece_prio": false,
        "auto_tmm": false,
        "force_start": t.forced,
        "super_seeding": false,
        "magnet_uri": magnet,
        "time_active": t.seeding_time,
        "downloaded": t.downloaded,
        "downloaded_session": 0,
        "uploaded": t.uploaded,
        "uploaded_session": 0,
        "ratio": if t.ratio.is_finite() { t.ratio } else { -1.0 },
        "last_activity": t.added_on,
        "seen_complete": -1,
        "ratio_limit": ratio_limit(t.limits.ratio),
        "seeding_time_limit": minutes_limit(t.limits.seeding_time),
        "inactive_seeding_time_limit": minutes_limit(t.limits.inactive_seeding_time),
        "seeding_time": t.seeding_time,
        "trackers_count": i32::from(t.tracker.is_some()),
        "tracker": t.tracker.as_ref().map(|h| format!("https://{h}")).unwrap_or_default(),
        "private": false,
        "popularity": -1,
        "availability": -1,
    })
}

pub fn server_state(
    g: &GlobalStats,
    s: &riptor_core::Settings,
    all_dl: u64,
    all_ul: u64,
    free: u64,
) -> Value {
    let kib = |k: Option<u32>| k.map_or(0, |k| i64::from(k) * 1024);
    let (dl, ul) = if g.alt_speed {
        (s.alt_dl_limit_kib, s.alt_ul_limit_kib)
    } else {
        (s.dl_limit_kib, s.ul_limit_kib)
    };
    json!({
        "alltime_dl": all_dl,
        "alltime_ul": all_ul,
        "use_subcategories": false,
        "global_ratio": if all_dl > 0 { format!("{:.2}", all_ul as f64 / all_dl as f64) } else { "0.00".into() },
        "total_wasted_session": 0,
        "total_peer_connections": 0,
        "read_cache_hits": "0",
        "total_buffers_size": g.write_buffer_used,
        "write_cache_overload": "0",
        "read_cache_overload": "0",
        "queued_io_jobs": 0,
        "average_time_queue": 0,
        "total_queued_size": 0,
        "dl_info_data": 0,
        "dl_info_speed": g.dl_rate,
        "dl_rate_limit": kib(dl),
        "up_info_data": 0,
        "up_info_speed": g.ul_rate,
        "up_rate_limit": kib(ul),
        "use_alt_speed_limits": g.alt_speed,
        "queueing": s.seeding.queue.enabled,
        "free_space_on_disk": free,
        "connection_status": if g.network == "blocked" { "disconnected" } else { "connected" },
        "dht_nodes": g.dht_nodes.unwrap_or(0),
    })
}

pub fn categories(cats: &std::collections::BTreeMap<String, Option<std::path::PathBuf>>) -> Value {
    let mut m = Map::new();
    for (name, path) in cats {
        let path = path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        m.insert(name.clone(), json!({ "name": name, "savePath": path }));
    }
    Value::Object(m)
}

fn file_priority(f: &FileEntry) -> u8 {
    match (f.selected, f.priority) {
        (false, _) => 0,
        (true, FilePriority::High) => 7,
        (true, _) => 1,
    }
}

pub fn files(files: &[FileEntry]) -> Value {
    Value::Array(
        files
            .iter()
            .map(|f| {
                json!({
                    "index": f.index,
                    "name": f.path,
                    "size": f.size,
                    "progress": if f.size == 0 { 1.0 } else { f.done as f64 / f.size as f64 },
                    "priority": file_priority(f),
                    "is_seed": f.done >= f.size,
                    "availability": -1,
                })
            })
            .collect(),
    )
}

pub fn trackers(urls: &[String]) -> Value {
    Value::Array(
        urls.iter()
            .enumerate()
            .map(|(i, u)| {
                json!({
                    "url": u, "tier": i, "status": 0, "num_peers": -1, "num_seeds": -1,
                    "num_leeches": -1, "num_downloaded": -1, "msg": "",
                })
            })
            .collect(),
    )
}

pub fn peers(list: &[PeerInfo], rid: u64) -> Value {
    let mut m = Map::new();
    for p in list {
        let (ip, port) = match p.addr.rsplit_once(':') {
            Some((ip, port)) => (
                ip.trim_matches(['[', ']']).to_string(),
                port.parse::<u16>().unwrap_or(0),
            ),
            None => (p.addr.clone(), 0),
        };
        m.insert(
            p.addr.clone(),
            json!({
                "connection": match p.conn { Some("utp") => "μTP", _ => "BT" },
                "country_code": p.country.as_deref().map(str::to_ascii_lowercase).unwrap_or_default(),
                "ip": ip,
                "port": port,
                "flags": "",
                "client": p.client.clone().unwrap_or_default(),
                "peer_id_client": "",
                "dl_speed": 0,
                "up_speed": 0,
                "downloaded": p.downloaded,
                "uploaded": p.uploaded,
                "progress": 0,
                "relevance": 0,
                "files": "",
            }),
        );
    }
    json!({ "full_update": true, "rid": rid, "peers": m, "show_flags": true })
}
