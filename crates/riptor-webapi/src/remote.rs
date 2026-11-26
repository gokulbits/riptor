//! Search, search plugins, RSS, the log and preferences in qBittorrent's API shapes.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::response::Response;
use riptor_core::{AddParams, Limit, PeerEncryption, ShareLimitAction, Source};
use riptor_search::{ProviderInfo, ProviderKind, SearchManager};
use riptor_services::Change;
use serde_json::{json, Map, Value};

use crate::api::{
    bad, conflict, json_body, limit_kib, ok, param, queue_adds, split_list, text, Ctx,
};

const MAX_INSTALL: usize = 20;
const NO_PLUGIN_INSTALL: &str =
    "Installing search plugins remotely is off. Turn it on in Riptor's remote control settings.";

fn not_ready(what: &str) -> Response {
    text(StatusCode::CONFLICT, format!("{what} is still starting"))
}

fn not_found(what: &str) -> Response {
    text(StatusCode::NOT_FOUND, format!("{what} not found"))
}

fn flag(p: &HashMap<String, String>, k: &str, default: bool) -> bool {
    match p.get(k).map(String::as_str) {
        Some("true") | Some("1") => true,
        Some("false") | Some("0") => false,
        _ => default,
    }
}

/// RFC 2822 in UTC, the way feeds date their items.
pub fn rfc2822(secs: u64) -> String {
    const DAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    // Days to a civil date (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{}, {:02} {} {} {:02}:{:02}:{:02} GMT",
        DAYS[(days.rem_euclid(7)) as usize],
        day,
        MONTHS[(month - 1) as usize],
        year,
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

// ---------------------------------------------------------------- search

/// The name clients use: a plugin's own id, others as Riptor names them.
fn engine_name(id: &str) -> &str {
    id.strip_prefix("plugin:").unwrap_or(id)
}

fn by_name(providers: &[ProviderInfo], name: &str) -> Option<String> {
    providers
        .iter()
        .find(|p| engine_name(&p.id) == name || p.id == name)
        .map(|p| p.id.clone())
}

fn title_case(c: &str) -> String {
    let mut s = c.to_string();
    if c == "tv" {
        return "TV shows".into();
    }
    if let Some(f) = s.get_mut(0..1) {
        f.make_ascii_uppercase();
    }
    s
}

fn plugins(s: &SearchManager) -> Value {
    Value::Array(
        s.providers()
            .iter()
            .map(|p| {
                json!({
                    "enabled": p.enabled,
                    "fullName": p.name,
                    "name": engine_name(&p.id),
                    "supportedCategories": p.categories.iter()
                        .map(|c| json!({ "id": c, "name": title_case(c) }))
                        .collect::<Vec<_>>(),
                    "url": p.url,
                    "version": "",
                    // Riptor's own: what kind it is and why it can't run.
                    "kind": match p.kind {
                        ProviderKind::Builtin => "builtin",
                        ProviderKind::Torznab => "torznab",
                        ProviderKind::Plugin => "plugin",
                    },
                    "error": p.error,
                })
            })
            .collect(),
    )
}

fn search_id(p: &HashMap<String, String>) -> Option<u64> {
    param(p, "id").trim().parse().ok()
}

fn status_word(done: bool) -> &'static str {
    if done {
        "Stopped"
    } else {
        "Running"
    }
}

fn results(ctx: &Ctx, s: &SearchManager, p: &HashMap<String, String>) -> Response {
    let Some(id) = search_id(p) else {
        return bad("missing id");
    };
    let total = match ctx.hub.searches.status(Some(id)).first() {
        Some(st) => st.total,
        None => return not_found("Search job"),
    };
    let offset: i64 = param(p, "offset").trim().parse().unwrap_or(0);
    // Negative counts back from the end.
    let offset = if offset < 0 {
        total.saturating_sub(offset.unsigned_abs() as usize)
    } else {
        offset as usize
    };
    let limit = param(p, "limit")
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|l| *l > 0)
        .map(|l| l as usize);
    let Some((job, total)) = ctx.hub.searches.results(id, offset, limit) else {
        return not_found("Search job");
    };
    let providers = s.providers();
    let site = |engine: &str| {
        providers
            .iter()
            .find(|p| p.id == engine)
            .map(|p| (p.url.clone(), p.name.clone()))
            .unwrap_or_default()
    };
    let list: Vec<Value> = job
        .results
        .iter()
        .map(|r| {
            let (url, name) = site(&r.engine);
            json!({
                "descrLink": r.desc_link.clone().unwrap_or_default(),
                "fileName": r.name,
                "fileSize": r.size.map_or(-1, |s| s as i64),
                "fileUrl": r.link,
                "nbLeechers": r.leechers.map_or(-1, i64::from),
                "nbSeeders": r.seeders.map_or(-1, i64::from),
                "siteUrl": url,
                "engineName": name,
                "pluginName": engine_name(&r.engine),
                "pubDate": r.pub_date.map_or(-1, |d| d as i64),
            })
        })
        .collect();
    json_body(json!({ "results": list, "status": status_word(job.done), "total": total }))
}

async fn install_plugins(
    ctx: &Ctx,
    s: &Arc<SearchManager>,
    urls: Vec<String>,
) -> anyhow::Result<()> {
    anyhow::ensure!(!urls.is_empty(), "give a plugin's https address");
    anyhow::ensure!(urls.len() <= MAX_INSTALL, "too many plugins at once");
    for u in &urls {
        // A local path would let a remote client run any .py on this computer.
        anyhow::ensure!(
            u.starts_with("https://"),
            "plugins can only be installed from https addresses"
        );
    }
    for u in urls {
        let preview = s.preview_plugin(&u).await?;
        s.install_plugin(&preview.sha256).await?;
        tracing::info!(target: "riptor::remote", "remote control installed search plugin {}", preview.id);
    }
    ctx.hub.changed(Change::SearchEngines);
    Ok(())
}

async fn update_plugins(ctx: &Ctx, s: &Arc<SearchManager>) -> anyhow::Result<()> {
    let catalog = s.catalog(true).await;
    let urls: Vec<String> = catalog
        .entries
        .into_iter()
        .filter(|e| e.installed && e.download_url.starts_with("https://"))
        .map(|e| e.download_url)
        .take(MAX_INSTALL)
        .collect();
    if urls.is_empty() {
        s.refresh_plugins().await;
        return Ok(());
    }
    install_plugins(ctx, s, urls).await
}

async fn search_act(
    ctx: &Arc<Ctx>,
    s: Arc<SearchManager>,
    path: &str,
    p: &HashMap<String, String>,
) -> Option<Response> {
    let jobs = &ctx.hub.searches;
    Some(match path {
        "search/start" => {
            let category = match param(p, "category").trim() {
                "" => "all",
                c => c,
            };
            let providers = s.providers();
            let wanted = match param(p, "plugins").trim() {
                "" | "enabled" => None,
                "all" => Some(
                    providers
                        .iter()
                        .filter(|p| p.error.is_none())
                        .map(|p| p.id.clone())
                        .collect(),
                ),
                list => {
                    let ids: Vec<String> = split_list(list, '|')
                        .iter()
                        .filter_map(|n| by_name(&providers, n))
                        .collect();
                    if ids.is_empty() {
                        return Some(bad("no such search plugin"));
                    }
                    Some(ids)
                }
            };
            match jobs
                .start(&ctx.engine, &s, param(p, "pattern"), category, wanted)
                .await
            {
                Ok(id) => json_body(json!({ "id": id })),
                Err(e) => conflict(e),
            }
        }
        "search/stop" => match search_id(p) {
            Some(id) if jobs.stop(&s, id) => ok(),
            Some(_) => not_found("Search job"),
            None => bad("missing id"),
        },
        "search/delete" => match search_id(p) {
            Some(id) if jobs.delete(&s, id) => ok(),
            Some(_) => not_found("Search job"),
            None => bad("missing id"),
        },
        "search/enablePlugin" => {
            let on = flag(p, "enable", true);
            let providers = s.providers();
            for n in split_list(param(p, "names"), '|') {
                let Some(id) = by_name(&providers, &n) else {
                    return Some(bad("no such search plugin"));
                };
                if let Err(e) = s.set_enabled(&id, on) {
                    return Some(conflict(e));
                }
            }
            ctx.hub.changed(Change::SearchEngines);
            ok()
        }
        "search/uninstallPlugin" => {
            let providers = s.providers();
            for n in split_list(param(p, "names"), '|') {
                let Some(info) =
                    by_name(&providers, &n).and_then(|id| providers.iter().find(|p| p.id == id))
                else {
                    return Some(bad("no such search plugin"));
                };
                let r = match info.kind {
                    ProviderKind::Plugin => s.uninstall_plugin(&info.id).await,
                    ProviderKind::Torznab => s.remove_torznab(&info.id),
                    ProviderKind::Builtin => {
                        return Some(bad("Riptor's own search can be turned off, not removed"))
                    }
                };
                if let Err(e) = r {
                    return Some(conflict(e));
                }
            }
            ctx.hub.changed(Change::SearchEngines);
            ok()
        }
        "search/installPlugin" | "search/updatePlugins" => {
            if !ctx.cfg.allow_plugin_install {
                return Some(text(StatusCode::FORBIDDEN, NO_PLUGIN_INSTALL));
            }
            riptor_services::search::route(&ctx.engine, &s);
            let r = if path.ends_with("installPlugin") {
                install_plugins(ctx, &s, split_list(param(p, "sources"), '|')).await
            } else {
                update_plugins(ctx, &s).await
            };
            r.map_or_else(conflict, |()| ok())
        }
        "search/downloadTorrent" => {
            let url = param(p, "torrentUrl").trim().to_string();
            let plugin = param(p, "pluginName").trim();
            let providers = s.providers();
            let plugin_id = by_name(&providers, plugin).filter(|id| id.starts_with("plugin:"));
            let src = if let Some(id) = plugin_id.filter(|_| !url.starts_with("magnet:")) {
                riptor_services::search::route(&ctx.engine, &s);
                match s.plugin_download(&id, &url).await {
                    Ok(riptor_search::nova::Fetched::Magnet(m)) => Source::Magnet(m),
                    Ok(riptor_search::nova::Fetched::File(f)) => Source::File(f),
                    Err(e) => return Some(conflict(e)),
                }
            } else {
                match riptor_search::clean(riptor_search::SearchResult {
                    name: "link".into(),
                    link: url,
                    size: None,
                    seeders: None,
                    leechers: None,
                    desc_link: None,
                    pub_date: None,
                    engine: String::new(),
                    via_plugin: false,
                }) {
                    Some(r) if r.link.starts_with("magnet:") => Source::Magnet(r.link),
                    Some(r) => Source::Url(r.link),
                    None => return Some(bad("not a magnet or http(s) link")),
                }
            };
            if queue_adds(ctx, vec![src], Vec::new(), AddParams::default()) {
                ok()
            } else {
                conflict(anyhow::anyhow!("too many adds are still waiting"))
            }
        }
        _ => return None,
    })
}

fn search_read(
    ctx: &Ctx,
    s: &SearchManager,
    path: &str,
    p: &HashMap<String, String>,
) -> Option<Response> {
    Some(match path {
        "search/plugins" => json_body(plugins(s)),
        "search/status" => {
            let id = search_id(p);
            let list = ctx.hub.searches.status(id);
            if id.is_some() && list.is_empty() {
                return Some(not_found("Search job"));
            }
            json_body(Value::Array(
                list.iter()
                    .map(|st| json!({ "id": st.id, "status": status_word(st.done), "total": st.total }))
                    .collect(),
            ))
        }
        "search/results" => results(ctx, s, p),
        _ => return None,
    })
}

// ------------------------------------------------------------------- RSS

fn feed_id(rss: &riptor_rss::Rss, path: &str) -> Option<u32> {
    let path = path.trim();
    let v = rss.view();
    v.feeds
        .iter()
        .find(|f| f.name == path)
        .or_else(|| {
            v.feeds
                .iter()
                .find(|f| f.url == path || f.id.to_string() == path)
        })
        .map(|f| f.id)
}

fn rss_items(rss: &riptor_rss::Rss, with_data: bool) -> Value {
    let mut m = Map::new();
    for f in rss.view().feeds {
        let mut item = json!({ "uid": format!("riptor-feed-{}", f.id), "url": f.url });
        if with_data {
            let articles: Vec<Value> = rss
                .articles(f.id)
                .unwrap_or_default()
                .iter()
                .map(|a| {
                    json!({
                        "id": a.id,
                        "title": a.title,
                        "date": a.date.map(rfc2822).unwrap_or_default(),
                        "link": a.page.clone().unwrap_or_else(|| a.torrent.clone()),
                        "torrentURL": a.torrent,
                        "description": "",
                        "isRead": a.read,
                        "size": a.size,
                    })
                })
                .collect();
            item["title"] = json!(f.name);
            item["lastBuildDate"] = json!(f.last_refresh.map(rfc2822).unwrap_or_default());
            item["isLoading"] = json!(false);
            item["hasError"] = json!(f.error.is_some());
            item["error"] = json!(f.error);
            item["articles"] = json!(articles);
        }
        // Names can repeat; a second one gets its id after it.
        let key = if m.contains_key(&f.name) {
            format!("{} ({})", f.name, f.id)
        } else {
            f.name.clone()
        };
        m.insert(key, item);
    }
    Value::Object(m)
}

fn rule_def(rss: &riptor_rss::Rss, r: &riptor_rss::Rule) -> Value {
    let view = rss.view();
    let urls: Vec<String> = view
        .feeds
        .iter()
        .filter(|f| r.feeds.is_empty() || r.feeds.contains(&f.id))
        .map(|f| f.url.clone())
        .collect();
    let save = r
        .save_path
        .as_ref()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    let cat = r.category.clone().unwrap_or_default();
    json!({
        "enabled": r.enabled,
        "mustContain": r.must_contain,
        "mustNotContain": r.must_not_contain,
        "useRegex": r.use_regex,
        "episodeFilter": "",
        "smartFilter": false,
        "previouslyMatchedEpisodes": [],
        "affectedFeeds": urls,
        "ignoreDays": 0,
        "lastMatch": r.last_match.map(rfc2822).unwrap_or_default(),
        "addPaused": r.start_paused,
        "assignedCategory": cat,
        "savePath": save,
        "torrentParams": { "category": cat, "save_path": save, "stopped": r.start_paused },
    })
}

/// A qBittorrent rule definition onto Riptor's rule, refusing what Riptor can't do.
fn parse_rule(
    ctx: &Ctx,
    rss: &riptor_rss::Rss,
    mut rule: riptor_rss::Rule,
    def: &Value,
) -> anyhow::Result<riptor_rss::Rule> {
    let s = |k: &str| def.get(k).and_then(Value::as_str);
    let b = |k: &str| def.get(k).and_then(Value::as_bool);
    let tp = def.get("torrentParams");
    let tps = |k: &str| tp.and_then(|t| t.get(k)).and_then(Value::as_str);
    anyhow::ensure!(
        s("episodeFilter").unwrap_or("").trim().is_empty() && !b("smartFilter").unwrap_or(false),
        "Riptor's rules don't support episode filters"
    );
    anyhow::ensure!(
        def.get("ignoreDays").and_then(Value::as_i64).unwrap_or(0) == 0,
        "Riptor's rules don't support ignoring matches for some days"
    );
    if let Some(v) = b("enabled") {
        rule.enabled = v;
    }
    if let Some(v) = s("mustContain") {
        rule.must_contain = v.to_string();
    }
    if let Some(v) = s("mustNotContain") {
        rule.must_not_contain = v.to_string();
    }
    if let Some(v) = b("useRegex") {
        rule.use_regex = v;
    }
    if let Some(list) = def.get("affectedFeeds").and_then(Value::as_array) {
        let view = rss.view();
        let ids: Vec<u32> = list
            .iter()
            .filter_map(Value::as_str)
            .filter_map(|u| view.feeds.iter().find(|f| f.url == u).map(|f| f.id))
            .collect();
        // Every feed listed is the same as "all feeds".
        rule.feeds = if ids.len() == view.feeds.len() {
            Vec::new()
        } else {
            ids
        };
    }
    if let Some(c) = tps("category").or_else(|| s("assignedCategory")) {
        rule.category = Some(c.trim().to_string()).filter(|c| !c.is_empty());
    }
    if let Some(p) = tps("save_path").or_else(|| s("savePath")) {
        let p = p.trim();
        rule.save_path = if p.is_empty() {
            None
        } else {
            let p = PathBuf::from(p);
            ctx.engine.check_root(&p)?;
            Some(p)
        };
    }
    let stopped = tp
        .and_then(|t| t.get("stopped"))
        .and_then(Value::as_bool)
        .or_else(|| b("addPaused"));
    if let Some(v) = stopped {
        rule.start_paused = v;
    }
    Ok(rule)
}

fn rule_named(rss: &riptor_rss::Rss, name: &str) -> Option<riptor_rss::Rule> {
    rss.view().rules.into_iter().find(|r| r.name == name)
}

fn rss_read(rss: &riptor_rss::Rss, path: &str, p: &HashMap<String, String>) -> Option<Response> {
    Some(match path {
        "rss/items" => json_body(rss_items(rss, flag(p, "withData", false))),
        "rss/rules" => {
            let mut m = Map::new();
            for r in rss.view().rules {
                m.insert(r.name.clone(), rule_def(rss, &r));
            }
            json_body(Value::Object(m))
        }
        "rss/matchingArticles" => {
            let Some(rule) = rule_named(rss, param(p, "ruleName")) else {
                return Some(not_found("Rule"));
            };
            match rss.preview_rule(&rule) {
                Ok(hits) => {
                    let mut m: Map<String, Value> = Map::new();
                    for h in hits {
                        let list = m.entry(h.feed).or_insert_with(|| json!([]));
                        if let Some(a) = list.as_array_mut() {
                            a.push(json!(h.title));
                        }
                    }
                    json_body(Value::Object(m))
                }
                Err(e) => conflict(e),
            }
        }
        _ => return None,
    })
}

fn refresh_later(ctx: &Ctx, rss: Arc<riptor_rss::Rss>, only: Option<Vec<u32>>) {
    let (engine, hub) = (ctx.engine.clone(), ctx.hub.clone());
    tokio::spawn(async move {
        if riptor_services::rss::refresh(&engine, &rss, true, only).await {
            hub.changed(Change::Rss);
        }
    });
}

fn rss_act(
    ctx: &Ctx,
    rss: Arc<riptor_rss::Rss>,
    path: &str,
    p: &HashMap<String, String>,
) -> Option<Response> {
    let done = |r: anyhow::Result<()>| {
        r.map_or_else(conflict, |()| {
            ctx.hub.changed(Change::Rss);
            ok()
        })
    };
    let feed =
        || feed_id(&rss, param(p, "itemPath").trim()).or_else(|| feed_id(&rss, param(p, "path")));
    Some(match path {
        "rss/addFeed" => {
            let name = param(p, "path").trim();
            if name.contains('\\') {
                return Some(conflict(anyhow::anyhow!(
                    "Riptor doesn't support RSS folders"
                )));
            }
            match rss.add_feed(param(p, "url"), Some(name).filter(|n| !n.is_empty())) {
                Ok(id) => {
                    refresh_later(ctx, rss.clone(), Some(vec![id]));
                    done(Ok(()))
                }
                Err(e) => conflict(e),
            }
        }
        "rss/addFolder" => conflict(anyhow::anyhow!("Riptor doesn't support RSS folders")),
        "rss/setFeedURL" => conflict(anyhow::anyhow!(
            "Riptor can't change a feed's address; remove it and add the new one"
        )),
        "rss/removeItem" => match feed() {
            Some(id) => done(rss.remove_feed(id)),
            None => not_found("Feed"),
        },
        "rss/moveItem" => {
            let dest = param(p, "destPath").trim();
            if dest.contains('\\') {
                return Some(conflict(anyhow::anyhow!(
                    "Riptor doesn't support RSS folders"
                )));
            }
            match feed_id(&rss, param(p, "itemPath")) {
                Some(id) => done(rss.rename_feed(id, dest)),
                None => not_found("Feed"),
            }
        }
        "rss/refreshItem" => {
            let only = match param(p, "itemPath").trim() {
                "" => None,
                path => match feed_id(&rss, path) {
                    Some(id) => Some(vec![id]),
                    None => return Some(not_found("Feed")),
                },
            };
            refresh_later(ctx, rss.clone(), only);
            ok()
        }
        "rss/markAsRead" => match feed() {
            Some(id) => {
                let article = p
                    .get("articleId")
                    .map(|a| a.as_str())
                    .filter(|a| !a.is_empty());
                done(rss.mark_read(id, article))
            }
            None => not_found("Feed"),
        },
        "rss/setRule" => {
            let name = param(p, "ruleName").trim();
            let def: Value = match serde_json::from_str(param(p, "ruleDef")) {
                Ok(v @ Value::Object(_)) => v,
                _ => return Some(bad("invalid rule definition")),
            };
            let base = rule_named(&rss, name).unwrap_or_else(|| riptor_rss::Rule {
                name: name.to_string(),
                ..Default::default()
            });
            match parse_rule(ctx, &rss, base, &def) {
                Ok(rule) => done(rss.save_rule(rule).map(|_| ())),
                Err(e) => conflict(e),
            }
        }
        "rss/renameRule" => match rule_named(&rss, param(p, "ruleName")) {
            Some(mut r) => {
                r.name = param(p, "newRuleName").trim().to_string();
                done(rss.save_rule(r).map(|_| ()))
            }
            None => not_found("Rule"),
        },
        "rss/removeRule" => match rule_named(&rss, param(p, "ruleName")) {
            Some(r) => done(rss.delete_rule(r.id)),
            None => not_found("Rule"),
        },
        _ => return None,
    })
}

// ------------------------------------------------------------------- log

fn log_main(p: &HashMap<String, String>) -> Response {
    let after = param(p, "last_known_id")
        .trim()
        .parse::<i64>()
        .ok()
        .filter(|i| *i >= 0)
        .map(|i| i as u64);
    let wanted = |kind: u8| match kind {
        1 => flag(p, "normal", true),
        2 => flag(p, "info", true),
        4 => flag(p, "warning", true),
        _ => flag(p, "critical", true),
    };
    let list: Vec<Value> = riptor_services::logs::since(after)
        .into_iter()
        .filter_map(|l| {
            let kind = match l.level {
                "error" => 8,
                "warn" => 4,
                "info" => 1,
                _ => 2,
            };
            wanted(kind).then(|| {
                json!({
                    "id": l.id,
                    "message": format!("{}: {}", l.target, l.message),
                    "timestamp": l.time / 1000,
                    "type": kind,
                })
            })
        })
        .collect();
    json_body(Value::Array(list))
}

// ----------------------------------------------------------- preferences

fn kib_bytes(k: Option<u32>) -> i64 {
    k.map_or(0, |k| i64::from(k) * 1024)
}

fn opt_count(v: Option<u32>) -> i64 {
    v.map_or(-1, i64::from)
}

fn days_code(mask: u8) -> u8 {
    match mask & 0x7f {
        0x1f => 1,
        0x60 => 2,
        m if m.count_ones() == 1 => 3 + m.trailing_zeros() as u8,
        _ => 0,
    }
}

fn days_mask(code: i64) -> Option<u8> {
    match code {
        0 => Some(0x7f),
        1 => Some(0x1f),
        2 => Some(0x60),
        3..=9 => Some(1 << (code - 3)),
        _ => None,
    }
}

fn preferences(ctx: &Ctx) -> Value {
    let s = ctx.engine.settings();
    let q = &s.seeding.queue;
    let w = &s.web_ui;
    let rss = ctx.hub.rss().map(|r| r.view().options).unwrap_or_default();
    let path = |p: &Option<PathBuf>| {
        p.as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default()
    };
    let scan: Map<String, Value> = s
        .watched_folders
        .iter()
        .map(|f| {
            let to = f
                .save_path
                .as_ref()
                .map_or(json!(1), |p| json!(p.to_string_lossy()));
            (f.path.to_string_lossy().into_owned(), to)
        })
        .collect();
    let a = &s.alt_schedule;
    json!({
        "locale": "en",
        "save_path": s.download_dir.to_string_lossy(),
        "temp_path_enabled": s.incomplete_dir.is_some(),
        "temp_path": path(&s.incomplete_dir),
        "scan_dirs": scan,
        "start_paused_enabled": s.add_stopped,
        "add_stopped_enabled": s.add_stopped,
        "dl_limit": kib_bytes(s.dl_limit_kib),
        "up_limit": kib_bytes(s.ul_limit_kib),
        "alt_dl_limit": kib_bytes(s.alt_dl_limit_kib),
        "alt_up_limit": kib_bytes(s.alt_ul_limit_kib),
        "scheduler_enabled": a.enabled,
        "schedule_from_hour": a.from / 60,
        "schedule_from_min": a.from % 60,
        "schedule_to_hour": a.to / 60,
        "schedule_to_min": a.to % 60,
        "scheduler_days": days_code(a.days),
        "listen_port": s.listen_port,
        "random_port": s.listen_port == 0,
        "upnp": s.upnp,
        "dht": s.dht,
        "lsd": s.lsd,
        "pex": true,
        "encryption": match s.encryption {
            PeerEncryption::Prefer => 0,
            PeerEncryption::Require => 1,
            PeerEncryption::Off => 2,
        },
        "anonymous_mode": s.anonymous_mode,
        "max_connec": s.peer_limit.map_or(-1, |n| n as i64),
        "max_connec_per_torrent": s.peer_limit.map_or(-1, |n| n as i64),
        "queueing_enabled": q.enabled,
        "max_active_downloads": opt_count(q.max_active_downloads),
        "max_active_uploads": opt_count(q.max_active_uploads),
        "max_active_torrents": opt_count(q.max_active_torrents),
        "dont_count_slow_torrents": q.dont_count_slow,
        "slow_torrent_dl_rate_threshold": q.dl_rate_threshold / 1024,
        "slow_torrent_ul_rate_threshold": q.ul_rate_threshold / 1024,
        "slow_torrent_inactive_timer": q.inactivity_timer_secs,
        "max_ratio_enabled": s.seeding.max_ratio.is_some(),
        "max_ratio": s.seeding.max_ratio.unwrap_or(-1.0),
        "max_seeding_time_enabled": s.seeding.max_seeding_minutes.is_some(),
        "max_seeding_time": s.seeding.max_seeding_minutes.map_or(-1, |m| m as i64),
        "max_inactive_seeding_time_enabled": s.seeding.max_inactive_seeding_minutes.is_some(),
        "max_inactive_seeding_time": s.seeding.max_inactive_seeding_minutes.map_or(-1, |m| m as i64),
        "max_ratio_act": match s.seeding.limit_action {
            ShareLimitAction::Stop => 0,
            ShareLimitAction::Remove => 1,
            ShareLimitAction::RemoveWithFiles => 3,
        },
        "rss_processing_enabled": true,
        "rss_auto_downloading_enabled": rss.auto_download,
        "rss_refresh_interval": rss.refresh_minutes,
        // Read only over remote control.
        "web_ui_address": w.address,
        "web_ui_port": w.port,
        "web_ui_username": w.username,
        "use_https": w.https,
        "web_ui_upnp": w.upnp,
        "bypass_local_auth": w.bypass_localhost,
        "bypass_auth_subnet_whitelist_enabled": w.bypass_subnets,
        "bypass_auth_subnet_whitelist": w.subnet_whitelist.join("
    "),
        "web_ui_max_auth_fail_count": w.max_failures,
        "web_ui_ban_duration": w.ban_secs,
        "web_ui_session_timeout": w.session_timeout_secs,
        "web_ui_clickjacking_protection_enabled": w.clickjacking_protection,
        "web_ui_csrf_protection_enabled": w.csrf_protection,
        "web_ui_secure_cookie_enabled": w.secure_cookie,
        "web_ui_host_header_validation_enabled": w.host_validation,
        "web_ui_domain_list": w.server_domains,
        "web_ui_reverse_proxy_enabled": w.reverse_proxy,
        "web_ui_reverse_proxies_list": w.trusted_proxies,
        "dyndns_enabled": w.ddns.enabled,
        "dyndns_domain": w.ddns.domain,
        "alternative_webui_enabled": false,
    })
}

/// Settings a remote client may change; everything else it sends must match what it read.
const SETTABLE: &[&str] = &[
    "save_path",
    "temp_path_enabled",
    "temp_path",
    "start_paused_enabled",
    "add_stopped_enabled",
    "dl_limit",
    "up_limit",
    "alt_dl_limit",
    "alt_up_limit",
    "scheduler_enabled",
    "schedule_from_hour",
    "schedule_from_min",
    "schedule_to_hour",
    "schedule_to_min",
    "scheduler_days",
    "listen_port",
    "random_port",
    "upnp",
    "dht",
    "lsd",
    "encryption",
    "anonymous_mode",
    "max_connec",
    "max_connec_per_torrent",
    "queueing_enabled",
    "max_active_downloads",
    "max_active_uploads",
    "max_active_torrents",
    "dont_count_slow_torrents",
    "slow_torrent_dl_rate_threshold",
    "slow_torrent_ul_rate_threshold",
    "slow_torrent_inactive_timer",
    "max_ratio_enabled",
    "max_ratio",
    "max_seeding_time_enabled",
    "max_seeding_time",
    "max_inactive_seeding_time_enabled",
    "max_inactive_seeding_time",
    "max_ratio_act",
    "rss_auto_downloading_enabled",
    "rss_refresh_interval",
];

fn int(v: &Value) -> Option<i64> {
    v.as_i64()
        .or_else(|| v.as_f64().filter(|f| f.fract() == 0.0).map(|f| f as i64))
        .or_else(|| v.as_str()?.trim().parse().ok())
}

fn boolean(v: &Value) -> Option<bool> {
    v.as_bool().or_else(|| match v.as_str()? {
        "true" => Some(true),
        "false" => Some(false),
        _ => None,
    })
}

fn count(v: &Value) -> Option<Option<u32>> {
    match int(v)? {
        n if n < 0 => Some(None),
        n => u32::try_from(n).ok().map(Some),
    }
}

/// Bytes/s with 0 or less meaning none, as KiB/s.
fn speed(v: &Value) -> Option<Option<u32>> {
    int(v).map(|b| limit_kib(&b.to_string()))
}

fn apply_preferences(
    ctx: &Ctx,
    prefs: &Map<String, Value>,
) -> Result<riptor_core::Settings, Response> {
    let current = preferences(ctx);
    for (k, v) in prefs {
        if !SETTABLE.contains(&k.as_str()) && current.get(k).is_some_and(|c| c != v) {
            return Err(text(
                StatusCode::FORBIDDEN,
                format!("{k} can't be changed over remote control"),
            ));
        }
    }
    let mut s = ctx.engine.settings();
    let mut rss = ctx.hub.rss().map(|r| r.view().options);
    let bad_value = |k: &str| Err(bad(&format!("invalid value for {k}")));
    let mut temp_on = s.incomplete_dir.is_some();
    let mut temp_path = s.incomplete_dir.clone();
    let mut sched = (s.alt_schedule.from, s.alt_schedule.to);
    for (k, v) in prefs {
        let k = k.as_str();
        macro_rules! get {
            ($f:ident) => {
                match $f(v) {
                    Some(x) => x,
                    None => return bad_value(k),
                }
            };
        }
        match k {
            "save_path" => match v.as_str() {
                Some(p) if !p.trim().is_empty() => s.download_dir = PathBuf::from(p.trim()),
                _ => return bad_value(k),
            },
            "temp_path_enabled" => temp_on = get!(boolean),
            "temp_path" => {
                let p = v.as_str().unwrap_or("").trim();
                temp_path = (!p.is_empty()).then(|| PathBuf::from(p));
            }
            "start_paused_enabled" | "add_stopped_enabled" => s.add_stopped = get!(boolean),
            "dl_limit" => s.dl_limit_kib = get!(speed),
            "up_limit" => s.ul_limit_kib = get!(speed),
            "alt_dl_limit" => s.alt_dl_limit_kib = get!(speed),
            "alt_up_limit" => s.alt_ul_limit_kib = get!(speed),
            "scheduler_enabled" => s.alt_schedule.enabled = get!(boolean),
            "schedule_from_hour" | "schedule_from_min" | "schedule_to_hour" | "schedule_to_min" => {
                let n = get!(int);
                let max = if k.ends_with("hour") { 23 } else { 59 };
                if !(0..=max).contains(&n) {
                    return bad_value(k);
                }
                let n = n as u16;
                let slot = if k.contains("from") {
                    &mut sched.0
                } else {
                    &mut sched.1
                };
                *slot = if k.ends_with("hour") {
                    n * 60 + *slot % 60
                } else {
                    *slot / 60 * 60 + n
                };
            }
            "scheduler_days" => match days_mask(get!(int)) {
                Some(m) => s.alt_schedule.days = m,
                None => return bad_value(k),
            },
            "listen_port" => match u16::try_from(get!(int)) {
                Ok(p) => s.listen_port = p,
                Err(_) => return bad_value(k),
            },
            "random_port" => {
                if get!(boolean) {
                    s.listen_port = 0;
                }
            }
            "upnp" => s.upnp = get!(boolean),
            "dht" => s.dht = get!(boolean),
            "lsd" => s.lsd = get!(boolean),
            "encryption" => {
                s.encryption = match get!(int) {
                    0 => PeerEncryption::Prefer,
                    1 => PeerEncryption::Require,
                    2 => PeerEncryption::Off,
                    _ => return bad_value(k),
                }
            }
            "anonymous_mode" => s.anonymous_mode = get!(boolean),
            "max_connec" => s.peer_limit = get!(count).map(|n| n as usize),
            // Riptor has one peer limit; the global one wins.
            "max_connec_per_torrent" => {}
            "queueing_enabled" => s.seeding.queue.enabled = get!(boolean),
            "max_active_downloads" => s.seeding.queue.max_active_downloads = get!(count),
            "max_active_uploads" => s.seeding.queue.max_active_uploads = get!(count),
            "max_active_torrents" => s.seeding.queue.max_active_torrents = get!(count),
            "dont_count_slow_torrents" => s.seeding.queue.dont_count_slow = get!(boolean),
            "slow_torrent_dl_rate_threshold" => {
                s.seeding.queue.dl_rate_threshold = (get!(int).max(0) as u64).saturating_mul(1024)
            }
            "slow_torrent_ul_rate_threshold" => {
                s.seeding.queue.ul_rate_threshold = (get!(int).max(0) as u64).saturating_mul(1024)
            }
            "slow_torrent_inactive_timer" => {
                s.seeding.queue.inactivity_timer_secs = get!(int).clamp(0, 86_400) as u64
            }
            "max_ratio_enabled" => {
                if !get!(boolean) {
                    s.seeding.max_ratio = None;
                } else if s.seeding.max_ratio.is_none() {
                    s.seeding.max_ratio = Some(1.0);
                }
            }
            "max_ratio" => {
                let r = v.as_f64().unwrap_or(-1.0);
                s.seeding.max_ratio = match qbt_ratio(r) {
                    Some(l) => l,
                    None => return bad_value(k),
                };
            }
            "max_seeding_time_enabled" | "max_inactive_seeding_time_enabled" => {
                let slot = if k.contains("inactive") {
                    &mut s.seeding.max_inactive_seeding_minutes
                } else {
                    &mut s.seeding.max_seeding_minutes
                };
                if !get!(boolean) {
                    *slot = None;
                } else if slot.is_none() {
                    *slot = Some(1440);
                }
            }
            "max_seeding_time" | "max_inactive_seeding_time" => {
                let n = get!(int);
                let m = match crate::qbt::parse_minutes_limit(n) {
                    Some(Limit::Value(m)) => Some(m),
                    Some(_) => None,
                    None => return bad_value(k),
                };
                if k.contains("inactive") {
                    s.seeding.max_inactive_seeding_minutes = m;
                } else {
                    s.seeding.max_seeding_minutes = m;
                }
            }
            "max_ratio_act" => {
                s.seeding.limit_action = match get!(int) {
                    0 => ShareLimitAction::Stop,
                    1 => ShareLimitAction::Remove,
                    3 => ShareLimitAction::RemoveWithFiles,
                    _ => return bad_value(k),
                }
            }
            "rss_auto_downloading_enabled" => {
                if let Some(o) = rss.as_mut() {
                    o.auto_download = get!(boolean);
                }
            }
            "rss_refresh_interval" => {
                if let Some(o) = rss.as_mut() {
                    o.refresh_minutes = match u32::try_from(get!(int)) {
                        Ok(m) => m,
                        Err(_) => return bad_value(k),
                    };
                }
            }
            _ => {}
        }
    }
    s.alt_schedule.from = sched.0;
    s.alt_schedule.to = sched.1;
    s.incomplete_dir = if temp_on { temp_path } else { None };
    if temp_on && s.incomplete_dir.is_none() {
        return Err(bad("choose a folder for incomplete downloads"));
    }
    if let Some(d) = &s.incomplete_dir {
        ctx.engine.check_root(d).map_err(conflict)?;
    }
    if let (Some(o), Some(r)) = (rss, ctx.hub.rss()) {
        if o != r.view().options {
            r.set_options(o).map_err(conflict)?;
            ctx.hub.changed(Change::Rss);
        }
    }
    Ok(s)
}

fn qbt_ratio(r: f64) -> Option<Option<f64>> {
    match crate::qbt::parse_ratio_limit(r)? {
        Limit::Value(v) => Some(Some(v)),
        _ => Some(None),
    }
}

// ------------------------------------------------------------ dispatch

/// Reads for search, RSS, the log and preferences; `None` for paths this doesn't know.
pub async fn read(ctx: &Ctx, path: &str, p: &HashMap<String, String>) -> Option<Response> {
    if path.starts_with("search/") {
        let s = ctx.hub.search();
        return match (s, path) {
            (Some(s), _) => search_read(ctx, &s, path, p),
            (None, "search/plugins" | "search/status") => Some(json_body(json!([]))),
            (None, "search/results") => Some(not_found("Search job")),
            _ => None,
        };
    }
    if path.starts_with("rss/") {
        return match ctx.hub.rss() {
            Some(r) => rss_read(&r, path, p),
            None if matches!(path, "rss/items" | "rss/rules") => Some(json_body(json!({}))),
            None => None,
        };
    }
    Some(match path {
        "log/main" => log_main(p),
        // Riptor keeps no log of peers.
        "log/peers" => json_body(json!([])),
        "app/preferences" => json_body(preferences(ctx)),
        _ => return None,
    })
}

/// Changes for search, RSS and preferences; `None` for paths this doesn't know.
pub async fn act(ctx: &Arc<Ctx>, path: &str, p: &HashMap<String, String>) -> Option<Response> {
    if path.starts_with("search/") {
        return match ctx.hub.search() {
            Some(s) => search_act(ctx, s, path, p).await,
            None => Some(not_ready("Search")),
        };
    }
    if path.starts_with("rss/") {
        return match ctx.hub.rss() {
            Some(r) => rss_act(ctx, r, path, p),
            None => Some(not_ready("RSS")),
        };
    }
    if path == "app/setPreferences" {
        let prefs: Map<String, Value> = match serde_json::from_str(param(p, "json")) {
            Ok(Value::Object(m)) => m,
            _ => return Some(bad("invalid preferences")),
        };
        return Some(match apply_preferences(ctx, &prefs) {
            Ok(s) => crate::api::engine_result(ctx.engine.update_settings(s).await),
            Err(r) => r,
        });
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_read_like_feed_dates() {
        assert_eq!(rfc2822(0), "Thu, 01 Jan 1970 00:00:00 GMT");
        assert_eq!(rfc2822(951_782_400), "Tue, 29 Feb 2000 00:00:00 GMT");
        assert_eq!(rfc2822(1_767_225_599), "Wed, 31 Dec 2025 23:59:59 GMT");
    }

    #[test]
    fn schedule_days_round_trip() {
        for code in 0..=9 {
            assert_eq!(days_code(days_mask(code).unwrap()), code as u8);
        }
        assert_eq!(days_mask(10), None);
        assert_eq!(days_code(0b101), 0);
    }

    #[test]
    fn plugin_names_drop_the_kind() {
        assert_eq!(engine_name("plugin:eztv"), "eztv");
        assert_eq!(engine_name("torznab:home"), "torznab:home");
        assert_eq!(engine_name("archive"), "archive");
        assert_eq!(title_case("tv"), "TV shows");
        assert_eq!(title_case("movies"), "Movies");
    }
}
