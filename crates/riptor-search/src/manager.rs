//! Owns provider configuration, installed plugins, and running searches.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::{bail, ensure, Context};
use futures::StreamExt;
use parking_lot::{Mutex, RwLock};
use serde::{Deserialize, Serialize};
use tokio_util::sync::CancellationToken;
use tracing::warn;

use crate::nova::{self, PluginCaps, PluginPreview, Python};
use crate::{
    archive, clean, dedupe_key, secrets, torznab, ProviderInfo, ProviderKind, SearchEvent,
    SearchResult,
};

const PROVIDER_TIMEOUT: Duration = Duration::from_secs(45);

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TorznabEndpoint {
    pub id: String,
    pub name: String,
    pub url: String,
}

/// Persisted in `search/config.json`. API keys are *not* stored here.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct SearchConfig {
    pub torznab: Vec<TorznabEndpoint>,
    pub disabled: BTreeSet<String>,
}

#[derive(Clone, Debug, Serialize)]
pub struct PythonStatus {
    pub available: bool,
    pub version: Option<String>,
    pub sandboxed: bool,
}

pub struct SearchManager {
    dir: PathBuf,
    http: reqwest::Client,
    config: RwLock<SearchConfig>,
    python: tokio::sync::Mutex<Option<Option<Python>>>,
    plugins: RwLock<Vec<PluginCaps>>,
    pending: Mutex<HashMap<String, (PluginPreview, Vec<u8>)>>,
    active: Mutex<HashMap<u64, CancellationToken>>,
    next_id: AtomicU64,
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
            config: RwLock::new(config),
            python: tokio::sync::Mutex::new(None),
            plugins: RwLock::new(Vec::new()),
            pending: Mutex::new(HashMap::new()),
            active: Mutex::new(HashMap::new()),
            next_id: AtomicU64::new(1),
        }))
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
            *g = Some(nova::detect().await);
        }
        g.clone().flatten()
    }

    pub async fn python_status(&self, recheck: bool) -> PythonStatus {
        if recheck {
            *self.python.lock().await = None;
        }
        let py = self.python().await;
        PythonStatus {
            available: py.is_some(),
            version: py.map(|p| p.version),
            sandboxed: cfg!(target_os = "linux") && nova::find_in_path("bwrap").is_some(),
        }
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
        if pending.len() > 16 {
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
    /// Events are delivered through `sink` until `Finished`.
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

        let me = self.clone();
        tokio::spawn(async move {
            let tasks = chosen.into_iter().map(|p| {
                let (me, sink, seen, token, query, category) =
                    (me.clone(), sink.clone(), seen.clone(), token.clone(), query.clone(), category.clone());
                async move {
                    let emit = |batch: Vec<SearchResult>| {
                        let mut s = seen.lock();
                        let fresh: Vec<SearchResult> =
                            batch.into_iter().filter_map(clean).filter(|r| s.insert(dedupe_key(r))).collect();
                        drop(s);
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
            me.active.lock().remove(&id);
            sink(SearchEvent::Finished { search_id: id });
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
