//! The `/api/v2/...` routes qBitController and similar apps use, mapped onto the engine.
//! Every call except login needs a session; changes must be POSTs.

use std::collections::{BTreeSet, HashMap};
use std::net::{IpAddr, SocketAddr};
use std::path::{Path as FsPath, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use axum::body::Bytes;
use axum::extract::{ConnectInfo, DefaultBodyLimit, Multipart, Path, RawQuery, State};
use axum::http::{header, HeaderMap, HeaderValue, Method, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::routing::{any, get, post};
use axum::Router;
use riptor_core::settings::{cidr_contains, parse_cidr};
use riptor_core::{AddParams, Engine, FilePriority, QueueMove, ShareLimits, Source, WebUi};
use serde_json::{json, Value};

use crate::auth::{Auth, Login};
use crate::net;
use crate::qbt;

const MAX_FORM: usize = 1024 * 1024;
const MAX_UPLOAD: usize = 64 * 1024 * 1024;
const MAX_ADD_ITEMS: usize = 50;
const MAX_PIECES: usize = 200_000;

pub struct Ctx {
    pub engine: Arc<Engine>,
    pub auth: Auth,
    pub data_dir: PathBuf,
    pub cfg: WebUi,
    rid: AtomicU64,
    whitelist: Vec<(IpAddr, u8)>,
    proxies: Vec<(IpAddr, u8)>,
    headers: Vec<(header::HeaderName, HeaderValue)>,
}

impl Ctx {
    pub fn new(engine: Arc<Engine>, auth: Auth, data_dir: PathBuf, cfg: WebUi) -> Self {
        let cidrs = |list: &mut dyn Iterator<Item = &str>| list.filter_map(parse_cidr).collect();
        let whitelist = cidrs(&mut cfg.subnet_whitelist.iter().map(String::as_str));
        let proxies = cidrs(&mut cfg.trusted_proxies.split(';'));
        let mut headers = Vec::new();
        if cfg.clickjacking_protection {
            headers.push((
                header::X_FRAME_OPTIONS,
                HeaderValue::from_static("SAMEORIGIN"),
            ));
            headers.push((
                header::CONTENT_SECURITY_POLICY,
                HeaderValue::from_static("frame-ancestors 'self'"),
            ));
        }
        headers.push((
            header::X_CONTENT_TYPE_OPTIONS,
            HeaderValue::from_static("nosniff"),
        ));
        headers.push((
            header::REFERRER_POLICY,
            HeaderValue::from_static("same-origin"),
        ));
        if cfg.custom_headers_enabled {
            for line in cfg
                .custom_headers
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
            {
                if let Some((name, value)) = line.split_once(':') {
                    if let (Ok(n), Ok(v)) = (
                        header::HeaderName::from_bytes(name.trim().as_bytes()),
                        HeaderValue::from_str(value.trim()),
                    ) {
                        headers.push((n, v));
                    }
                }
            }
        }
        Self {
            engine,
            auth,
            data_dir,
            cfg,
            rid: AtomicU64::new(0),
            whitelist,
            proxies,
            headers,
        }
    }

    /// The real client: behind a trusted reverse proxy, the last untrusted X-Forwarded-For hop.
    fn client_ip(&self, peer: SocketAddr, headers: &HeaderMap) -> IpAddr {
        let trusted = |ip: IpAddr| self.proxies.iter().any(|c| cidr_contains(*c, ip));
        if !self.cfg.reverse_proxy || !trusted(peer.ip()) {
            return peer.ip();
        }
        let hops: Vec<IpAddr> = headers
            .get_all("x-forwarded-for")
            .iter()
            .filter_map(|v| v.to_str().ok())
            .flat_map(|v| v.split(','))
            .filter_map(|h| h.trim().parse().ok())
            .collect();
        hops.into_iter()
            .rev()
            .find(|ip| !trusted(*ip))
            .unwrap_or(peer.ip())
    }

    fn bypassed(&self, ip: IpAddr) -> bool {
        (self.cfg.bypass_localhost && ip.is_loopback())
            || (self.cfg.bypass_subnets && self.whitelist.iter().any(|c| cidr_contains(*c, ip)))
    }

    fn signed_in(&self, ip: IpAddr, headers: &HeaderMap) -> bool {
        if self.bypassed(ip) {
            return true;
        }
        let bearer = headers
            .get(header::AUTHORIZATION)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "));
        session_id(headers).is_some_and(|sid| self.auth.check(sid))
            || bearer.is_some_and(|k| self.auth.api_key_from(ip, k.trim()))
    }

    /// CSRF (Origin/Referer) and Host checks, as configured.
    fn request_ok(&self, headers: &HeaderMap) -> bool {
        (!self.cfg.csrf_protection || same_origin(headers))
            && (!self.cfg.host_validation || host_allowed(&self.cfg.server_domains, headers))
    }
}

/// IP addresses and localhost always pass; names must match a server domain (`*` wildcards).
fn host_allowed(domains: &str, headers: &HeaderMap) -> bool {
    let Some(host) = headers.get(header::HOST).and_then(|h| h.to_str().ok()) else {
        return false;
    };
    let name = match host.strip_prefix('[') {
        Some(v6) => v6.split(']').next().unwrap_or(""),
        None => host.rsplit_once(':').map_or(host, |(h, _)| h),
    };
    if name.parse::<IpAddr>().is_ok() || name.eq_ignore_ascii_case("localhost") {
        return true;
    }
    domains
        .split(';')
        .map(str::trim)
        .filter(|d| !d.is_empty())
        .any(|d| wildcard(&d.to_ascii_lowercase(), &name.to_ascii_lowercase()))
}

fn wildcard(pattern: &str, s: &str) -> bool {
    match pattern.split_once('*') {
        None => pattern == s,
        Some((head, tail)) => s.strip_prefix(head).is_some_and(|rest| {
            (0..=rest.len()).any(|i| rest.is_char_boundary(i) && wildcard(tail, &rest[i..]))
        }),
    }
}

async fn security_headers(State(ctx): State<Arc<Ctx>>, mut r: Response) -> Response {
    for (n, v) in &ctx.headers {
        r.headers_mut().insert(n.clone(), v.clone());
    }
    r
}

pub fn router(ctx: Arc<Ctx>) -> Router {
    Router::new()
        .route("/api/v2/auth/login", post(login))
        .route(
            "/api/v2/torrents/add",
            post(add).layer(DefaultBodyLimit::max(MAX_UPLOAD)),
        )
        .route("/api/v2/{*path}", any(dispatch))
        // No web pages here, only the API; say so to someone checking from a browser.
        .route(
            "/",
            get(|| async {
                text(
                    StatusCode::OK,
                    "Riptor remote control is running. Connect with an app made for qBittorrent's Web UI, such as qBitController.",
                )
            }),
        )
        .fallback(|| async { (StatusCode::NOT_FOUND, "Not Found") })
        .layer(DefaultBodyLimit::max(MAX_FORM))
        .layer(axum::middleware::map_response_with_state(
            ctx.clone(),
            security_headers,
        ))
        .with_state(ctx)
}

fn text(code: StatusCode, body: impl Into<String>) -> Response {
    (
        code,
        [(header::CONTENT_TYPE, "text/plain; charset=UTF-8")],
        body.into(),
    )
        .into_response()
}

fn ok() -> Response {
    text(StatusCode::OK, "")
}

fn json_body(v: Value) -> Response {
    (
        StatusCode::OK,
        [(header::CONTENT_TYPE, "application/json")],
        v.to_string(),
    )
        .into_response()
}

fn conflict(e: anyhow::Error) -> Response {
    text(StatusCode::CONFLICT, format!("{e:#}"))
}

fn bad(msg: &str) -> Response {
    text(StatusCode::BAD_REQUEST, msg)
}

/// A browser on another site can't drive the API: Origin and Referer, when sent, must match Host.
fn same_origin(headers: &HeaderMap) -> bool {
    let host = headers.get(header::HOST).and_then(|h| h.to_str().ok());
    [header::ORIGIN, header::REFERER].iter().all(|name| {
        let Some(v) = headers.get(name).and_then(|v| v.to_str().ok()) else {
            return true;
        };
        let Ok(u) = url::Url::parse(v) else {
            return false;
        };
        let origin_host = match (u.host_str(), u.port()) {
            (Some(h), Some(p)) => format!("{h}:{p}"),
            (Some(h), None) => h.to_string(),
            _ => return false,
        };
        host.is_some_and(|h| h.eq_ignore_ascii_case(&origin_host))
    })
}

fn session_id(headers: &HeaderMap) -> Option<&str> {
    headers
        .get_all(header::COOKIE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .flat_map(|v| v.split(';'))
        .find_map(|kv| kv.trim().strip_prefix("SID="))
}

fn form(bytes: &[u8]) -> HashMap<String, String> {
    url::form_urlencoded::parse(bytes).into_owned().collect()
}

async fn login(
    State(ctx): State<Arc<Ctx>>,
    ConnectInfo(net::Client(remote)): ConnectInfo<net::Client>,
    headers: HeaderMap,
    body: Bytes,
) -> Response {
    if !ctx.request_ok(&headers) {
        return text(StatusCode::UNAUTHORIZED, "");
    }
    let ip = ctx.client_ip(remote, &headers);
    let p = form(&body);
    let (user, pass) = (
        p.get("username").map_or("", String::as_str),
        p.get("password").map_or("", String::as_str),
    );
    let result = if ctx.bypassed(ip) {
        Login::Ok(ctx.auth.new_session())
    } else {
        ctx.auth.login(ip, user, pass).await
    };
    let outcome = match &result {
        Login::Ok(_) => "signed in",
        Login::Fails => "wrong user name or password",
        Login::Banned => "refused, banned after failed sign-ins",
        Login::Paused => "refused, sign-ins are paused after many failures",
    };
    crate::net::note(
        ip,
        outcome,
        format_args!("remote control sign-in from {ip}: {outcome}"),
    );
    match result {
        Login::Ok(sid) => {
            let secure = if ctx.cfg.https && ctx.cfg.secure_cookie {
                "; Secure"
            } else {
                ""
            };
            let cookie = format!("SID={sid}; HttpOnly; SameSite=Strict; path=/{secure}");
            let mut r = text(StatusCode::OK, "Ok.");
            if let Ok(v) = HeaderValue::from_str(&cookie) {
                r.headers_mut().insert(header::SET_COOKIE, v);
            }
            r
        }
        Login::Fails => text(StatusCode::OK, "Fails."),
        Login::Banned => text(
            StatusCode::FORBIDDEN,
            "Your IP address has been banned after too many failed authentication attempts.",
        ),
        Login::Paused => text(
            StatusCode::FORBIDDEN,
            "Too many failed sign-ins from different addresses; sign-ins are paused for a while.",
        ),
    }
}

/// `hashes` (or `hash`): `all`, or info hashes separated by `|`.
fn ids(ctx: &Ctx, p: &HashMap<String, String>) -> Result<Vec<String>, Box<Response>> {
    let raw = p
        .get("hashes")
        .or_else(|| p.get("hash"))
        .ok_or_else(|| Box::new(bad("missing hashes")))?;
    let (torrents, _) = ctx.engine.snapshot();
    if raw == "all" {
        return Ok(torrents.into_iter().map(|t| t.id).collect());
    }
    let known: BTreeSet<String> = torrents.into_iter().map(|t| t.id).collect();
    let wanted: Vec<String> = raw
        .split('|')
        .map(|h| h.trim().to_ascii_lowercase())
        .filter(|h| known.contains(h))
        .collect();
    if wanted.is_empty() {
        return Err(Box::new(text(StatusCode::NOT_FOUND, "Torrent not found")));
    }
    Ok(wanted)
}

fn one(ctx: &Ctx, p: &HashMap<String, String>) -> Result<String, Box<Response>> {
    let mut v = ids(ctx, p)?;
    if v.len() != 1 {
        return Err(Box::new(bad("one hash expected")));
    }
    Ok(v.remove(0))
}

fn summary(ctx: &Ctx, id: &str) -> Option<riptor_core::TorrentSummary> {
    ctx.engine.snapshot().0.into_iter().find(|t| t.id == id)
}

fn split_list(s: &str, sep: char) -> Vec<String> {
    s.split(sep)
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(String::from)
        .collect()
}

fn param<'a>(p: &'a HashMap<String, String>, k: &str) -> &'a str {
    p.get(k).map_or("", String::as_str)
}

fn engine_result(r: anyhow::Result<()>) -> Response {
    r.map_or_else(conflict, |()| ok())
}

async fn dispatch(
    State(ctx): State<Arc<Ctx>>,
    ConnectInfo(net::Client(remote)): ConnectInfo<net::Client>,
    method: Method,
    headers: HeaderMap,
    Path(path): Path<String>,
    RawQuery(query): RawQuery,
    body: Bytes,
) -> Response {
    if !ctx.request_ok(&headers) {
        return text(StatusCode::UNAUTHORIZED, "");
    }
    let signed_in = ctx.signed_in(ctx.client_ip(remote, &headers), &headers);
    if path == "auth/logout" {
        if let Some(sid) = session_id(&headers) {
            ctx.auth.logout(sid);
        }
        return ok();
    }
    if !signed_in {
        return text(StatusCode::FORBIDDEN, "Forbidden");
    }
    let mut p = form(query.unwrap_or_default().as_bytes());
    let is_form = headers
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .is_some_and(|v| v.starts_with("application/x-www-form-urlencoded"));
    if is_form {
        p.extend(form(&body));
    }
    match read(&ctx, &path, &p).await {
        Some(r) => r,
        None if method != Method::POST => text(StatusCode::METHOD_NOT_ALLOWED, ""),
        None => act(&ctx, &path, &p).await,
    }
}

/// Calls that only read; `None` for anything else.
async fn read(ctx: &Ctx, path: &str, p: &HashMap<String, String>) -> Option<Response> {
    let e = &ctx.engine;
    Some(match path {
        "app/version" => text(StatusCode::OK, qbt::VERSION),
        "app/webapiVersion" => text(StatusCode::OK, qbt::WEBAPI_VERSION),
        "app/defaultSavePath" => text(StatusCode::OK, e.settings().download_dir.to_string_lossy()),
        "sync/maindata" => maindata(ctx),
        "sync/torrentPeers" => match one(ctx, p) {
            Ok(id) => match e.peers(&id) {
                Ok(peers) => json_body(qbt::peers(
                    &peers,
                    ctx.rid.fetch_add(1, Ordering::Relaxed) + 1,
                )),
                Err(err) => conflict(err),
            },
            Err(r) => *r,
        },
        "torrents/info" => {
            let (torrents, _) = e.snapshot();
            let wanted: Option<BTreeSet<String>> = p
                .get("hashes")
                .map(|h| h.split('|').map(|s| s.to_ascii_lowercase()).collect());
            let category = p.get("category");
            let list = torrents
                .iter()
                .filter(|t| wanted.as_ref().is_none_or(|w| w.contains(&t.id)))
                .filter(|t| category.is_none_or(|c| t.category.as_deref().unwrap_or("") == c))
                .map(|t| qbt::torrent(t, &magnet(ctx, &t.id)))
                .collect();
            json_body(Value::Array(list))
        }
        "torrents/properties" => match one(ctx, p) {
            Ok(id) => properties(ctx, &id),
            Err(r) => *r,
        },
        "torrents/files" => match one(ctx, p) {
            Ok(id) => e
                .files(&id)
                .map_or_else(conflict, |f| json_body(qbt::files(&f))),
            Err(r) => *r,
        },
        "torrents/trackers" => match one(ctx, p) {
            Ok(id) => e
                .trackers(&id)
                .map_or_else(conflict, |t| json_body(qbt::trackers(&t))),
            Err(r) => *r,
        },
        "torrents/webseeds" => json_body(json!([])),
        "torrents/pieceStates" => match one(ctx, p) {
            Ok(id) => piece_states(ctx, &id),
            Err(r) => *r,
        },
        "torrents/categories" => json_body(qbt::categories(&e.categories())),
        "torrents/tags" => {
            let tags: BTreeSet<String> = e.snapshot().0.into_iter().flat_map(|t| t.tags).collect();
            json_body(json!(tags))
        }
        "torrents/export" => match one(ctx, p) {
            Ok(id) => export(ctx, &id),
            Err(r) => *r,
        },
        "transfer/info" => {
            let (_, g) = e.snapshot();
            let s = e.settings();
            let totals = qbt::Totals::from(&e.statistics());
            let state = qbt::server_state(&g, &s, totals, 0);
            json_body(json!({
                "dl_info_speed": state["dl_info_speed"], "up_info_speed": state["up_info_speed"],
                "dl_info_data": state["dl_info_data"], "up_info_data": state["up_info_data"],
                "dl_rate_limit": state["dl_rate_limit"], "up_rate_limit": state["up_rate_limit"],
                "dht_nodes": state["dht_nodes"], "connection_status": state["connection_status"],
            }))
        }
        "transfer/speedLimitsMode" => text(
            StatusCode::OK,
            if e.snapshot().1.alt_speed { "1" } else { "0" },
        ),
        // Riptor's logs stay on this computer; search and RSS aren't offered remotely.
        "log/main" | "log/peers" | "search/plugins" => json_body(json!([])),
        "rss/items" | "rss/rules" => json_body(json!({})),
        _ => return None,
    })
}

fn magnet(ctx: &Ctx, id: &str) -> String {
    ctx.engine
        .magnet_link(id)
        .map(|m| m.link)
        .unwrap_or_default()
}

fn maindata(ctx: &Ctx) -> Response {
    let e = &ctx.engine;
    let (torrents, global) = e.snapshot();
    let settings = e.settings();
    let totals = qbt::Totals::from(&e.statistics());
    let free = e.free_space(&settings.download_dir).unwrap_or(0);
    let mut map = serde_json::Map::new();
    let mut tags = BTreeSet::new();
    for t in &torrents {
        tags.extend(t.tags.iter().cloned());
        map.insert(t.id.clone(), qbt::torrent(t, &magnet(ctx, &t.id)));
    }
    // Always a full update: simple, and clients replace their copy.
    json_body(json!({
        "rid": ctx.rid.fetch_add(1, Ordering::Relaxed) + 1,
        "full_update": true,
        "server_state": qbt::server_state(&global, &settings, totals, free),
        "torrents": map,
        "categories": qbt::categories(&e.categories()),
        "tags": tags,
        "trackers": {},
    }))
}

fn properties(ctx: &Ctx, id: &str) -> Response {
    let Some(t) = summary(ctx, id) else {
        return text(StatusCode::NOT_FOUND, "Torrent not found");
    };
    let pieces = ctx.engine.piece_map(id, 1).ok();
    let kib = |k: Option<u32>| k.map_or(-1, |k| i64::from(k) * 1024);
    json_body(json!({
        "save_path": t.save_path,
        "creation_date": -1,
        "piece_size": -1,
        "comment": "",
        "total_wasted": 0,
        "total_uploaded": t.uploaded,
        "total_downloaded": t.downloaded,
        "up_limit": kib(t.ul_limit_kib),
        "dl_limit": kib(t.dl_limit_kib),
        "time_elapsed": t.seeding_time,
        "seeding_time": t.seeding_time,
        "nb_connections": t.peers,
        "nb_connections_limit": -1,
        "share_ratio": if t.ratio.is_finite() { t.ratio } else { -1.0 },
        "addition_date": t.added_on,
        "completion_date": t.completed_on.map_or(-1, |c| c as i64),
        "created_by": "",
        "dl_speed_avg": 0,
        "dl_speed": t.dl_rate,
        "eta": t.eta.unwrap_or(8_640_000),
        "last_seen": -1,
        "peers": t.peers,
        "peers_total": t.peers_seen,
        "pieces_have": pieces.as_ref().map_or(0, |m| m.have),
        "pieces_num": pieces.as_ref().map_or(-1, |m| m.total as i64),
        "reannounce": 0,
        "seeds": 0,
        "seeds_total": t.swarm_seeds.unwrap_or(0),
        "total_size": t.total_size,
        "up_speed_avg": 0,
        "up_speed": t.ul_rate,
        "isPrivate": false,
    }))
}

fn piece_states(ctx: &Ctx, id: &str) -> Response {
    let total = match ctx.engine.piece_map(id, 1) {
        Ok(m) => m.total,
        Err(e) => return conflict(e),
    };
    if total == 0 || total > MAX_PIECES {
        return json_body(json!([]));
    }
    // One bucket per piece: full ones are downloaded (2), the rest not yet (0).
    match ctx.engine.piece_map(id, total) {
        Ok(m) => json_body(json!(m
            .buckets
            .iter()
            .map(|b| if *b == u8::MAX { 2 } else { 0 })
            .collect::<Vec<_>>())),
        Err(e) => conflict(e),
    }
}

fn temp_path(ctx: &Ctx, what: &str) -> PathBuf {
    let mut n = [0u8; 8];
    rand::fill(&mut n);
    let tag: String = n.iter().map(|b| format!("{b:02x}")).collect();
    ctx.data_dir.join(format!("webui-{what}-{tag}.torrent"))
}

fn export(ctx: &Ctx, id: &str) -> Response {
    let path = temp_path(ctx, "export");
    let result = ctx
        .engine
        .export_torrent(id, &path, false)
        .and_then(|()| Ok(std::fs::read(&path)?));
    let _ = std::fs::remove_file(&path);
    match result {
        Ok(bytes) => (
            StatusCode::OK,
            [(header::CONTENT_TYPE, "application/x-bittorrent")],
            bytes,
        )
            .into_response(),
        Err(e) => conflict(e),
    }
}

/// Calls that change something; POST only.
async fn act(ctx: &Ctx, path: &str, p: &HashMap<String, String>) -> Response {
    let e = &ctx.engine;
    macro_rules! hashes {
        () => {
            match ids(ctx, p) {
                Ok(v) => v,
                Err(r) => return *r,
            }
        };
    }
    macro_rules! hash {
        () => {
            match one(ctx, p) {
                Ok(v) => v,
                Err(r) => return *r,
            }
        };
    }
    match path {
        "torrents/stop" | "torrents/pause" => engine_result(e.stop(&hashes!()).await),
        "torrents/start" | "torrents/resume" => engine_result(e.resume(&hashes!())),
        "torrents/setForceStart" => {
            let ids = hashes!();
            if param(p, "value") == "true" {
                engine_result(e.force_start(&ids))
            } else {
                engine_result(e.resume(&ids))
            }
        }
        "torrents/delete" => {
            let ids = hashes!();
            engine_result(e.remove(&ids, param(p, "deleteFiles") == "true").await)
        }
        "torrents/recheck" => engine_result(e.recheck(&hashes!()).await),
        "torrents/reannounce" => engine_result(e.reannounce(&hashes!()).await),
        "torrents/setCategory" => {
            let ids = hashes!();
            let c = param(p, "category").trim();
            engine_result(e.set_category(&ids, (!c.is_empty()).then(|| c.to_string())))
        }
        "torrents/addTags" | "torrents/removeTags" => {
            let ids = hashes!();
            let given = split_list(param(p, "tags"), ',');
            let adding = path.ends_with("addTags");
            for t in e.snapshot().0.into_iter().filter(|t| ids.contains(&t.id)) {
                let tags: Vec<String> = if adding {
                    let mut all: BTreeSet<String> = t.tags.into_iter().collect();
                    all.extend(given.iter().cloned());
                    all.into_iter().collect()
                } else if given.is_empty() {
                    Vec::new()
                } else {
                    t.tags.into_iter().filter(|x| !given.contains(x)).collect()
                };
                if let Err(err) = e.set_tags(std::slice::from_ref(&t.id), tags) {
                    return conflict(err);
                }
            }
            ok()
        }
        // Riptor's tags exist only on torrents, so there's nothing to create up front.
        "torrents/createTags" => ok(),
        "torrents/deleteTags" => {
            let gone = split_list(param(p, "tags"), ',');
            for t in e.snapshot().0 {
                if t.tags.iter().any(|x| gone.contains(x)) {
                    let keep = t.tags.into_iter().filter(|x| !gone.contains(x)).collect();
                    if let Err(err) = e.set_tags(std::slice::from_ref(&t.id), keep) {
                        return conflict(err);
                    }
                }
            }
            ok()
        }
        "torrents/createCategory" | "torrents/editCategory" => {
            let name = param(p, "category").trim();
            let path = param(p, "savePath").trim();
            let folder = (!path.is_empty()).then(|| PathBuf::from(path));
            engine_result(e.set_category_def(name, folder))
        }
        "torrents/removeCategories" => {
            for c in split_list(param(p, "categories"), '\n') {
                if let Err(err) = e.remove_category_def(&c) {
                    return conflict(err);
                }
            }
            ok()
        }
        "torrents/setLocation" => {
            let ids = hashes!();
            engine_result(
                e.move_torrents(&ids, FsPath::new(param(p, "location").trim()))
                    .await,
            )
        }
        "torrents/filePrio" => file_priority(ctx, &hash!(), p).await,
        "torrents/renameFile" | "torrents/renameFolder" => {
            let id = hash!();
            let (old, new) = (param(p, "oldPath"), param(p, "newPath"));
            let files = match e.files(&id) {
                Ok(f) => f,
                Err(err) => return conflict(err),
            };
            let changes: Vec<(usize, String)> = if path.ends_with("renameFile") {
                files
                    .iter()
                    .filter(|f| f.path == old)
                    .map(|f| (f.index, new.to_string()))
                    .collect()
            } else {
                let prefix = format!("{}/", old.trim_end_matches('/'));
                files
                    .iter()
                    .filter_map(|f| {
                        let rest = f.path.strip_prefix(&prefix)?;
                        Some((f.index, format!("{}/{rest}", new.trim_end_matches('/'))))
                    })
                    .collect()
            };
            if changes.is_empty() {
                return bad("no such file or folder");
            }
            engine_result(e.rename_files(&id, &changes).await)
        }
        "torrents/setShareLimits" => {
            let ids = hashes!();
            let ratio = param(p, "ratioLimit")
                .parse::<f64>()
                .ok()
                .and_then(qbt::parse_ratio_limit);
            let mins = |k: &str| {
                param(p, k).parse::<i64>().ok().map_or(
                    Some(riptor_core::Limit::UseGlobal),
                    qbt::parse_minutes_limit,
                )
            };
            let (Some(ratio), Some(seeding), Some(inactive)) = (
                ratio,
                mins("seedingTimeLimit"),
                mins("inactiveSeedingTimeLimit"),
            ) else {
                return bad("invalid share limits");
            };
            let limits = ShareLimits {
                ratio,
                seeding_time: seeding,
                inactive_seeding_time: inactive,
            };
            engine_result(e.set_share_limits(&ids, limits, None))
        }
        "torrents/setDownloadLimit" | "torrents/setUploadLimit" => {
            let ids = hashes!();
            let kib = limit_kib(param(p, "limit"));
            let down = path.ends_with("setDownloadLimit");
            for t in e.snapshot().0.into_iter().filter(|t| ids.contains(&t.id)) {
                let (dl, ul) = if down {
                    (kib, t.ul_limit_kib)
                } else {
                    (t.dl_limit_kib, kib)
                };
                if let Err(err) = e.set_speed_limits(std::slice::from_ref(&t.id), dl, ul) {
                    return conflict(err);
                }
            }
            ok()
        }
        "torrents/topPrio" => engine_result(e.queue_move(&hashes!(), QueueMove::Top)),
        "torrents/bottomPrio" => engine_result(e.queue_move(&hashes!(), QueueMove::Bottom)),
        "torrents/increasePrio" => engine_result(e.queue_move(&hashes!(), QueueMove::Up)),
        "torrents/decreasePrio" => engine_result(e.queue_move(&hashes!(), QueueMove::Down)),
        "torrents/addTrackers" | "torrents/editTracker" | "torrents/removeTrackers" => {
            let id = hash!();
            let mut list = match e.trackers(&id) {
                Ok(l) => l,
                Err(err) => return conflict(err),
            };
            match path {
                "torrents/addTrackers" => {
                    for u in split_list(param(p, "urls"), '\n') {
                        if !list.contains(&u) {
                            list.push(u);
                        }
                    }
                }
                "torrents/editTracker" => {
                    let (from, to) = (param(p, "origUrl"), param(p, "newUrl").trim());
                    let Some(slot) = list.iter_mut().find(|u| *u == from) else {
                        return bad("no such tracker");
                    };
                    *slot = to.to_string();
                }
                _ => {
                    let gone = split_list(param(p, "urls"), '|');
                    list.retain(|u| !gone.contains(u));
                }
            }
            engine_result(e.set_trackers(&id, &list).await)
        }
        "torrents/addPeers" => {
            let ids = hashes!();
            for id in &ids {
                for peer in split_list(param(p, "peers"), '|').iter().take(100) {
                    let _ = e.add_peer(id, peer);
                }
            }
            ok()
        }
        "transfer/setDownloadLimit" | "transfer/setUploadLimit" => {
            let mut s = e.settings();
            let kib = limit_kib(param(p, "limit"));
            if path.ends_with("setDownloadLimit") {
                s.dl_limit_kib = kib;
            } else {
                s.ul_limit_kib = kib;
            }
            engine_result(e.update_settings(s).await)
        }
        "transfer/toggleSpeedLimitsMode" => {
            let on = e.snapshot().1.alt_speed;
            engine_result(e.set_alt_speed(!on))
        }
        "transfer/banPeers" => {
            for peer in split_list(param(p, "peers"), '|').iter().take(100) {
                let ip = peer
                    .parse::<SocketAddr>()
                    .map(|a| a.ip())
                    .or_else(|_| peer.parse::<IpAddr>());
                if let Ok(ip) = ip {
                    if let Err(err) = e.ban_peer(ip).await {
                        return conflict(err);
                    }
                }
            }
            ok()
        }
        // Not something a remote app should be able to do to this computer.
        "app/shutdown" | "app/getDirectoryContent" | "app/setPreferences" => {
            text(StatusCode::FORBIDDEN, "Not allowed over remote control")
        }
        _ => text(StatusCode::CONFLICT, "Riptor doesn't support this"),
    }
}

/// Bytes/s from the API; 0 or less means no limit.
fn limit_kib(v: &str) -> Option<u32> {
    let b = v.trim().parse::<i64>().ok()?;
    (b > 0).then(|| u32::try_from(b.saturating_add(1023) / 1024).unwrap_or(u32::MAX))
}

async fn file_priority(ctx: &Ctx, id: &str, p: &HashMap<String, String>) -> Response {
    let e = &ctx.engine;
    let files = match e.files(id) {
        Ok(f) => f,
        Err(err) => return conflict(err),
    };
    let picked: BTreeSet<usize> = param(p, "id")
        .split('|')
        .filter_map(|s| s.trim().parse().ok())
        .filter(|i| *i < files.len())
        .collect();
    if picked.is_empty() {
        return bad("invalid file id");
    }
    let prio = param(p, "priority");
    let mut selected: BTreeSet<usize> = files
        .iter()
        .filter(|f| f.selected)
        .map(|f| f.index)
        .collect();
    let level = match prio {
        "0" => {
            selected.retain(|i| !picked.contains(i));
            None
        }
        "1" => Some(FilePriority::Normal),
        "6" | "7" => Some(FilePriority::High),
        _ => return bad("invalid priority"),
    };
    if let Some(level) = level {
        selected.extend(picked.iter().copied());
        let list: Vec<usize> = picked.iter().copied().collect();
        if let Err(err) = e.set_file_priority(id, &list, level) {
            return conflict(err);
        }
    }
    engine_result(e.set_files(id, selected.into_iter().collect()).await)
}

async fn add(
    State(ctx): State<Arc<Ctx>>,
    ConnectInfo(net::Client(remote)): ConnectInfo<net::Client>,
    headers: HeaderMap,
    mut multipart: Multipart,
) -> Response {
    if !ctx.request_ok(&headers) {
        return text(StatusCode::UNAUTHORIZED, "");
    }
    if !ctx.signed_in(ctx.client_ip(remote, &headers), &headers) {
        return text(StatusCode::FORBIDDEN, "Forbidden");
    }
    let mut fields: HashMap<String, String> = HashMap::new();
    let mut files: Vec<Bytes> = Vec::new();
    loop {
        let field = match multipart.next_field().await {
            Ok(Some(f)) => f,
            Ok(None) => break,
            Err(_) => return bad("invalid upload"),
        };
        let name = field.name().unwrap_or_default().to_string();
        let is_file = name == "torrents" || field.file_name().is_some();
        let Ok(data) = field.bytes().await else {
            return bad("invalid upload");
        };
        if is_file {
            if files.len() >= MAX_ADD_ITEMS || data.len() > riptor_core::input::MAX_TORRENT_BYTES {
                return bad("too many or too large torrent files");
            }
            files.push(data);
        } else if data.len() <= MAX_FORM {
            fields.insert(name, String::from_utf8_lossy(&data).into_owned());
        }
    }
    let flag = |k: &str| fields.get(k).is_some_and(|v| v == "true");
    let save = fields
        .get("savepath")
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());
    let category = fields
        .get("category")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty());
    let params = AddParams {
        save_path: save.map(PathBuf::from),
        category,
        tags: fields
            .get("tags")
            .map(|t| split_list(t, ','))
            .unwrap_or_default(),
        start_paused: flag("stopped") || flag("paused"),
        ..Default::default()
    };
    let mut sources = Vec::new();
    for line in fields
        .get("urls")
        .map(|u| split_list(u, '\n'))
        .unwrap_or_default()
        .into_iter()
        .take(MAX_ADD_ITEMS)
    {
        let lower = line.to_ascii_lowercase();
        if lower.starts_with("magnet:") {
            sources.push(Source::Magnet(line));
        } else if lower.starts_with("http://") || lower.starts_with("https://") {
            sources.push(Source::Url(line));
        } else if line.len() == 40 && line.bytes().all(|b| b.is_ascii_hexdigit()) {
            sources.push(Source::Magnet(format!("magnet:?xt=urn:btih:{line}")));
        }
    }
    let mut added = 0;
    let mut temp = Vec::new();
    for bytes in files {
        let path = temp_path(&ctx, "upload");
        if riptor_core::state_file::save_bytes(&path, &bytes).is_ok() {
            sources.push(Source::File(path.clone()));
            temp.push(path);
        }
    }
    for src in sources {
        if ctx.engine.add_direct(src, params.clone()).await.is_ok() {
            added += 1;
        }
    }
    for path in temp {
        let _ = std::fs::remove_file(path);
    }
    text(StatusCode::OK, if added > 0 { "Ok." } else { "Fails." })
}
