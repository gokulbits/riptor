//! Owns provider configuration, installed plugins, and running searches.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use anyhow::{bail, ensure, Context};
use futures::StreamExt;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::catalog::{self, Catalog};
use crate::nova::{self, PluginCaps, PluginPreview, Python};
use crate::{
    archive, clean, dedupe_key, pyembed, scrape, secrets, torznab, ProviderInfo, ProviderKind,
    SearchEvent, SearchResult,
};

const PROVIDER_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TorznabEndpoint {
    pub id: String,
    pub name: String,
    pub url: String,
}

/// Persisted in `search/config.json`. API keys are *not* stored here.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub torznab: Vec<TorznabEndpoint>,
    pub disabled: BTreeSet<String>,
    /// Ask trackers for seed/peer counts that providers didn't send.
    pub scrape: bool,
}

impl Default for SearchConfig {
    fn default() -> Self {
        Self {
            torznab: Vec::new(),
            disabled: BTreeSet::new(),
            scrape: true,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct SearchOptions {
    pub scrape: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct PythonStatus {
    pub available: bool,
    pub version: Option<String>,
    pub sandboxed: bool,
    /// The interpreter in use is the one Riptor set up.
    pub bundled: bool,
    /// Riptor's own Python is present (whether or not it is the one in use).
    pub bundled_installed: bool,
    /// Riptor can set up Python by itself on this platform.
    pub can_bundle: bool,
    pub bundle_version: &'static str,
    pub bundle_size: Option<u64>,
}

/// How long a fetched plugin catalog is reused.
const CATALOG_TTL: Duration = Duration::from_secs(60 * 60);

pub struct SearchManager {
    dir: PathBuf,
    http: reqwest::Client,
    scrape_net: scrape::Net,
    config: RwLock<SearchConfig>,
    python: tokio::sync::Mutex<Option<Option<Python>>>,
    python_setup: tokio::sync::Mutex<()>,
    plugins: RwLock<Vec<PluginCaps>>,
    pending: Mutex<HashMap<String, (PluginPreview, Vec<u8>)>>,
    catalog: tokio::sync::Mutex<Option<(Instant, Catalog)>>,
    active: Mutex<HashMap<u64, CancellationToken>>,
    next_id: AtomicU64,
    /// Set while Riptor's VPN or kill switch is in charge: tracker lookups
    /// would go out on the normal connection, so they are skipped.
    lookups_held: std::sync::atomic::AtomicBool,
}

fn slug(name: &str) -> String {
    let s: String = name
        .to_ascii_lowercase()
        .chars()
        .map(|c| if c.is_ascii_alphanumeric() { c } else { '-' })
        .collect();
    let s = s.trim_matches('-').chars().take(40).collect::<String>();
    if s.is_empty() {
        "indexer".into()
    } else {
        s
    }
}

impl SearchManager {
    pub fn new(data_dir: &Path, http: reqwest::Client) -> anyhow::Result<Arc<Self>> {
        let dir = data_dir.join("search");
        nova::install_shim(&dir)?;
        let config = match std::fs::read(dir.join("config.json")) {
            Ok(b) => serde_json::from_slice(&b).unwrap_or_else(|e| {
                warn!("search config unreadable ({e}); starting fresh");
                SearchConfig::default()
            }),
            Err(_) => SearchConfig::default(),
        };
        Ok(Arc::new(Self {
            dir,
            http,
            scrape_net: scrape::Net::new(false)?,
            config: RwLock::new(config),
            python: tokio::sync::Mutex::new(None),
            python_setup: tokio::sync::Mutex::new(()),
            plugins: RwLock::new(Vec::new()),
            pending: Mutex::new(HashMap::new()),
            catalog: tokio::sync::Mutex::new(None),
            active: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
            lookups_held: std::sync::atomic::AtomicBool::new(false),
        }))
    }

    pub fn options(&self) -> SearchOptions {
        SearchOptions {
            scrape: self.config.read().scrape,
        }
    }

    /// Hold tracker lookups (seed/peer counts) regardless of the user setting.
    pub fn hold_lookups(&self, held: bool) {
        self.lookups_held.store(held, Ordering::SeqCst);
    }

    pub fn set_scrape(&self, on: bool) -> anyhow::Result<()> {
        self.config.write().scrape = on;
        self.save_config()
    }

    fn engines_dir(&self) -> PathBuf {
        self.dir.join("engines")
    }

    fn save_config(&self) -> anyhow::Result<()> {
        let body = serde_json::to_vec_pretty(&*self.config.read())?;
        let tmp = self.dir.join("config.json.tmp");
        std::fs::write(&tmp, body)?;
        std::fs::rename(tmp, self.dir.join("config.json"))?;
        Ok(())
    }

    async fn python(&self) -> Option<Python> {
        let mut g = self.python.lock().await;
        if g.is_none() {
            *g = Some(nova::detect(&[pyembed::exe(&self.dir)]).await);
        }
        g.clone().flatten()
    }

    pub async fn python_status(&self, recheck: bool) -> PythonStatus {
        if recheck {
            *self.python.lock().await = None;
        }
        let py = self.python().await;
        let bundled_dir = pyembed::dir(&self.dir);
        PythonStatus {
            available: py.is_some(),
            bundled: py.as_ref().is_some_and(|p| p.exe.starts_with(&bundled_dir)),
            version: py.map(|p| p.version),
            sandboxed: cfg!(target_os = "linux") && nova::find_in_path("bwrap").is_some(),
            bundled_installed: pyembed::installed(&self.dir),
            can_bundle: pyembed::RELEASE.is_some(),
            bundle_version: pyembed::VERSION,
            bundle_size: pyembed::RELEASE.as_ref().map(|r| r.size),
        }
    }

    /// Download and set up Riptor's own Python (Windows), then use it if no
    /// system Python is found.
    pub async fn install_python(&self) -> anyhow::Result<PythonStatus> {
        let _one_at_a_time = self
            .python_setup
            .try_lock()
            .map_err(|_| anyhow::anyhow!("Python is already being set up"))?;
        pyembed::install(&self.dir).await?;
        let status = self.python_status(true).await;
        self.refresh_plugins().await;
        Ok(status)
    }

    pub async fn remove_python(&self) -> anyhow::Result<PythonStatus> {
        let _one_at_a_time = self
            .python_setup
            .try_lock()
            .map_err(|_| anyhow::anyhow!("Python is being set up"))?;
        pyembed::remove(&self.dir)?;
        let status = self.python_status(true).await;
        self.refresh_plugins().await;
        Ok(status)
    }

    /// Plugins available from GitHub, marked with which are installed.
    pub async fn catalog(&self, refresh: bool) -> Catalog {
        let mut cache = self.catalog.lock().await;
        let fresh = cache
            .as_ref()
            .is_some_and(|(at, c)| at.elapsed() < CATALOG_TTL && c.errors.is_empty());
        if refresh || !fresh {
            *cache = Some((Instant::now(), catalog::fetch(&self.http).await));
        }
        let mut c = cache.as_ref().map(|(_, c)| c.clone()).unwrap_or_default();
        let engines = self.engines_dir();
        for e in &mut c.entries {
            e.installed = e.problem.is_none() && engines.join(format!("{}.py", e.id)).is_file();
        }
        c
    }

    /// Re-read installed plugins' names and categories.
    pub async fn refresh_plugins(&self) {
        let has_files = std::fs::read_dir(self.engines_dir())
            .map(|d| {
                d.flatten()
                    .any(|e| e.path().extension().is_some_and(|x| x == "py"))
            })
            .unwrap_or(false);
        let caps = match (has_files, self.python().await) {
            (true, Some(py)) => nova::capabilities(&py, &self.dir)
                .await
                .unwrap_or_else(|e| {
                    warn!("reading search plugins failed: {e:#}");
                    Vec::new()
                }),
            (true, None) => std::fs::read_dir(self.engines_dir())
                .map(|d| {
                    d.flatten()
                        .filter_map(|e| {
                            e.file_name()
                                .to_str()?
                                .strip_suffix(".py")
                                .map(str::to_string)
                        })
                        .filter(|id| nova::validate_id(id).is_ok())
                        .map(|id| PluginCaps {
                            name: id.clone(),
                            id,
                            url: String::new(),
                            categories: vec![],
                            error: Some("Python 3.9 or newer is needed to run this plugin".into()),
                        })
                        .collect()
                })
                .unwrap_or_default(),
            _ => Vec::new(),
        };
        *self.plugins.write() = caps;
    }

    pub fn providers(&self) -> Vec<ProviderInfo> {
        let cfg = self.config.read();
        let on = |id: &str| !cfg.disabled.contains(id);
        let mut v = vec![ProviderInfo {
            id: archive::ID.into(),
            name: archive::NAME.into(),
            kind: ProviderKind::Builtin,
            url: archive::URL.into(),
            enabled: on(archive::ID),
            categories: archive::categories(),
            error: None,
        }];
        for t in &cfg.torznab {
            let id = format!("torznab:{}", t.id);
            v.push(ProviderInfo {
                enabled: on(&id),
                id,
                name: t.name.clone(),
                kind: ProviderKind::Torznab,
                url: t.url.clone(),
                categories: torznab::categories(),
                error: None,
            });
        }
        for p in self.plugins.read().iter() {
            let id = format!("plugin:{}", p.id);
            v.push(ProviderInfo {
                enabled: on(&id),
                id,
                name: p.name.clone(),
                kind: ProviderKind::Plugin,
                url: p.url.clone(),
                categories: p.categories.clone(),
                error: p.error.clone(),
            });
        }
        v
    }

    pub fn set_enabled(&self, id: &str, enabled: bool) -> anyhow::Result<()> {
        ensure!(
            self.providers().iter().any(|p| p.id == id),
            "unknown search provider"
        );
        {
            let mut cfg = self.config.write();
            if enabled {
                cfg.disabled.remove(id);
            } else {
                cfg.disabled.insert(id.to_string());
            }
        }
        self.save_config()
    }

    // ------------------------------------------------------------ torznab

    pub fn add_torznab(&self, name: &str, url: &str, api_key: &str) -> anyhow::Result<()> {
        let name = name.trim();
        ensure!(
            !name.is_empty() && name.chars().count() <= 64,
            "give the indexer a name (up to 64 characters)"
        );
        let url = torznab::validate_endpoint(url)?.to_string();
        ensure!(
            !api_key.trim().is_empty() && api_key.len() <= 256,
            "enter the indexer's API key"
        );
        let mut id = slug(name);
        {
            let cfg = self.config.read();
            let mut n = 2;
            let base = id.clone();
            while cfg.torznab.iter().any(|t| t.id == id) {
                id = format!("{base}-{n}");
                n += 1;
            }
        }
        secrets::store_api_key(&id, api_key.trim())?;
        self.config.write().torznab.push(TorznabEndpoint {
            id,
            name: name.into(),
            url,
        });
        self.save_config()
    }

    pub fn remove_torznab(&self, id: &str) -> anyhow::Result<()> {
        let short = id.strip_prefix("torznab:").unwrap_or(id).to_string();
        {
            let mut cfg = self.config.write();
            cfg.torznab.retain(|t| t.id != short);
            cfg.disabled.remove(&format!("torznab:{short}"));
        }
        secrets::delete_api_key(&short);
        self.save_config()
    }

    // ------------------------------------------------------------ plugins

    /// Fetch or read a plugin and describe it. Nothing is installed or run.
    pub async fn preview_plugin(&self, source: &str) -> anyhow::Result<PluginPreview> {
        let (file_name, bytes) = if source.starts_with("https://") {
            let u = url::Url::parse(source)?;
            let name = u
                .path_segments()
                .and_then(|mut s| s.next_back())
                .unwrap_or_default()
                .to_string();
            let resp = self.http.get(u).send().await?.error_for_status()?;
            ensure!(
                resp.url().scheme() == "https",
                "the plugin download was redirected off https"
            );
            if let Some(len) = resp.content_length() {
                ensure!(
                    len as usize <= nova::MAX_PLUGIN_BYTES,
                    "plugin file is too large"
                );
            }
            let mut body = Vec::new();
            let mut stream = resp.bytes_stream();
            while let Some(chunk) = stream.next().await {
                let chunk = chunk?;
                ensure!(
                    body.len() + chunk.len() <= nova::MAX_PLUGIN_BYTES,
                    "plugin file is too large"
                );
                body.extend_from_slice(&chunk);
            }
            (name, body)
        } else if source.starts_with("http://") {
            bail!("plugins can only be downloaded over https");
        } else {
            let p = Path::new(source);
            ensure!(p.is_absolute(), "choose a plugin file");
            let len = std::fs::metadata(p)
                .with_context(|| format!("opening {source}"))?
                .len();
            ensure!(
                len as usize <= nova::MAX_PLUGIN_BYTES,
                "plugin file is too large"
            );
            let name = p
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or_default()
                .to_string();
            (name, std::fs::read(p)?)
        };
        let preview = nova::inspect(&file_name, &bytes, source, &self.engines_dir())?;
        let mut pending = self.pending.lock();
        // Room for a reviewed batch from the catalog (the UI allows 20).
        if pending.len() >= 64 {
            pending.clear();
        }
        pending.insert(preview.sha256.clone(), (preview.clone(), bytes));
        Ok(preview)
    }

    /// Install exactly the bytes the user reviewed (identified by hash).
    pub async fn install_plugin(&self, sha256: &str) -> anyhow::Result<()> {
        let (preview, bytes) = self
            .pending
            .lock()
            .remove(sha256)
            .context("review the plugin again before installing")?;
        let path = self.engines_dir().join(format!("{}.py", preview.id));
        let tmp = path.with_extension("py.tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &path)?;
        self.refresh_plugins().await;
        Ok(())
    }

    pub async fn uninstall_plugin(&self, id: &str) -> anyhow::Result<()> {
        let short = id.strip_prefix("plugin:").unwrap_or(id);
        nova::validate_id(short)?;
        let path = self.engines_dir().join(format!("{short}.py"));
        if path.exists() {
            std::fs::remove_file(path)?;
        }
        self.config
            .write()
            .disabled
            .remove(&format!("plugin:{short}"));
        self.save_config()?;
        self.refresh_plugins().await;
        Ok(())
    }

    /// For plugin results that must be fetched through the plugin (cookies etc).
    pub async fn plugin_download(&self, provider: &str, url: &str) -> anyhow::Result<PathBuf> {
        let short = provider
            .strip_prefix("plugin:")
            .context("not a plugin result")?;
        let py = self.python().await.context("Python is not installed")?;
        nova::download(&py, &self.dir, short, url).await
    }

    // ------------------------------------------------------------- search

    pub fn cancel(&self, search_id: u64) {
        if let Some(t) = self.active.lock().remove(&search_id) {
            t.cancel();
        }
    }

    /// Start a search over the given providers (or every enabled one).
    /// Events are delivered through `sink` until `LookupsDone`.
    pub async fn start(
        self: &Arc<Self>,
        query: &str,
        category: &str,
        providers: Option<Vec<String>>,
        sink: Arc<dyn Fn(SearchEvent) + Send + Sync>,
    ) -> anyhow::Result<u64> {
        let query = crate::validate_query(query)?;
        crate::validate_category(category)?;
        let all = self.providers();
        let chosen: Vec<ProviderInfo> = all
            .into_iter()
            .filter(|p| p.error.is_none())
            .filter(|p| match &providers {
                Some(ids) => ids.contains(&p.id),
                None => p.enabled,
            })
            .filter(|p| category == "all" || p.categories.iter().any(|c| c == category))
            .collect();
        ensure!(
            !chosen.is_empty(),
            "no search engine supports this category; enable one in Search engines"
        );

        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let token = CancellationToken::new();
        self.active.lock().insert(id, token.clone());
        let seen: Arc<Mutex<HashSet<String>>> = Arc::default();
        let category = category.to_string();

        // Results without counts are queued for tracker lookups, which run
        // alongside the providers and may finish after them.
        let (lookup_tx, lookup_rx) = tokio::sync::mpsc::unbounded_channel::<scrape::Job>();
        let lookups_on = self.config.read().scrape && !self.lookups_held.load(Ordering::SeqCst);

        let me = self.clone();
        tokio::spawn(async move {
            let lookups = {
                let (net, sink, token) = (me.scrape_net.clone(), sink.clone(), token.clone());
                tokio::spawn(async move {
                    let emit = |counts| {
                        sink(SearchEvent::Counts {
                            search_id: id,
                            counts,
                        })
                    };
                    tokio::select! {
                        _ = net.run(lookup_rx, emit) => {}
                        _ = token.cancelled() => {}
                    }
                })
            };
            let tasks = chosen.into_iter().map(|p| {
                let (me, sink, seen, token, query, category, lookup_tx) =
                    (me.clone(), sink.clone(), seen.clone(), token.clone(), query.clone(), category.clone(), lookup_tx.clone());
                async move {
                    let emit = |batch: Vec<SearchResult>| {
                        let mut s = seen.lock();
                        let fresh: Vec<SearchResult> =
                            batch.into_iter().filter_map(clean).filter(|r| s.insert(dedupe_key(r))).collect();
                        drop(s);
                        if lookups_on {
                            for job in fresh.iter().filter_map(scrape::job_for) {
                                let _ = lookup_tx.send(job);
                            }
                        }
                        if !fresh.is_empty() {
                            sink(SearchEvent::Results { search_id: id, provider: p.id.clone(), results: fresh });
                        }
                    };
                    let result = tokio::select! {
                        r = tokio::time::timeout(PROVIDER_TIMEOUT, me.run_provider(&p, &query, &category, token.clone(), emit)) =>
                            r.unwrap_or_else(|_| Err(anyhow::anyhow!("timed out"))),
                        _ = token.cancelled() => Ok(0),
                    };
                    let (count, error) = match result {
                        Ok(n) => (n, None),
                        Err(e) => (0, Some(format!("{e:#}").chars().take(300).collect())),
                    };
                    sink(SearchEvent::ProviderDone { search_id: id, provider: p.id.clone(), count, error });
                }
            });
            futures::future::join_all(tasks).await;
            drop(lookup_tx);
            sink(SearchEvent::Finished { search_id: id });
            let _ = lookups.await;
            me.active.lock().remove(&id);
            sink(SearchEvent::LookupsDone { search_id: id });
        });
        Ok(id)
    }

    async fn run_provider(
        &self,
        p: &ProviderInfo,
        query: &str,
        category: &str,
        token: CancellationToken,
        mut emit: impl FnMut(Vec<SearchResult>),
    ) -> anyhow::Result<usize> {
        let results = match p.kind {
            ProviderKind::Builtin => archive::search(&self.http, query, category).await?,
            ProviderKind::Torznab => {
                let short = p.id.strip_prefix("torznab:").unwrap_or(&p.id);
                let key = secrets::api_key(short)?;
                torznab::search(&self.http, &p.url, &key, &p.id, query, category).await?
            }
            ProviderKind::Plugin => {
                let py = self
                    .python()
                    .await
                    .context("Python 3.9 or newer is needed for plugins")?;
                let short = p.id.strip_prefix("plugin:").unwrap_or(&p.id).to_string();
                return nova::search(&py, &self.dir, &short, category, query, token, |mut b| {
                    for r in &mut b {
                        r.engine = p.id.clone();
                    }
                    emit(b)
                })
                .await;
            }
        };
        let n = results.len();
        emit(results);
        Ok(n)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn config_and_providers() {
        let d = tempfile::tempdir().unwrap();
        let m = SearchManager::new(d.path(), reqwest::Client::new()).unwrap();
        assert!(d.path().join("search/nova2.py").exists());
        let p = m.providers();
        assert_eq!(p[0].id, "archive");
        m.set_enabled("archive", false).unwrap();
        assert!(!m.providers()[0].enabled);
        assert!(m.set_enabled("nope", false).is_err());
        // Reload from disk.
        let m2 = SearchManager::new(d.path(), reqwest::Client::new()).unwrap();
        assert!(!m2.providers()[0].enabled);
        // Plain-http remote indexers are refused before any key is stored.
        assert!(m
            .add_torznab("x", "http://jackett.example.com/api", "k")
            .is_err());
        assert_eq!(slug("My Jackett!"), "my-jackett");
    }

    #[tokio::test]
    async fn plugin_install_requires_review() {
        let d = tempfile::tempdir().unwrap();
        let m = SearchManager::new(d.path(), reqwest::Client::new()).unwrap();
        let f = d.path().join("demo.py");
        std::fs::write(
            &f,
            "class demo(object):\n    name = 'Demo'\n    url = 'https://d.example'\n",
        )
        .unwrap();
        assert!(m.install_plugin("0".repeat(64).as_str()).await.is_err());
        let p = m.preview_plugin(f.to_str().unwrap()).await.unwrap();
        assert_eq!(p.declared_name.as_deref(), Some("Demo"));
        assert!(m.preview_plugin("http://plain.example/x.py").await.is_err());
        m.install_plugin(&p.sha256).await.unwrap();
        assert!(d.path().join("search/engines/demo.py").exists());
        // A second install of the same review token is refused.
        assert!(m.install_plugin(&p.sha256).await.is_err());
        m.uninstall_plugin("plugin:demo").await.unwrap();
        assert!(!d.path().join("search/engines/demo.py").exists());
        assert!(m.uninstall_plugin("plugin:../x").await.is_err());
    }

    #[tokio::test]
    async fn rejects_bad_queries_before_searching() {
        let d = tempfile::tempdir().unwrap();
        let m = SearchManager::new(d.path(), reqwest::Client::new()).unwrap();
        let sink: Arc<dyn Fn(SearchEvent) + Send + Sync> = Arc::new(|_| {});
        assert!(m.start("", "all", None, sink.clone()).await.is_err());
        assert!(m.start("x", "nope", None, sink.clone()).await.is_err());
        m.set_enabled("archive", false).unwrap();
        assert!(
            m.start("x", "all", None, sink).await.is_err(),
            "no providers enabled"
        );
    }
}
