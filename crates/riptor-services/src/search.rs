//! Searches that remote clients start and then poll for results.

use std::collections::HashMap;
use std::sync::Arc;

use anyhow::ensure;
use parking_lot::Mutex;
use riptor_core::{Engine, NetworkMode};
use riptor_search::{Route, SearchEvent, SearchManager, SearchResult};

/// Searches running at once, as qBittorrent allows.
pub const MAX_RUNNING: usize = 5;
/// Finished searches kept for reading; the oldest go first.
const MAX_KEPT: usize = 20;
const MAX_RESULTS: usize = 10_000;

/// Search traffic follows the torrents' network: direct, the tunnel, or held.
pub fn route(engine: &Engine, s: &SearchManager) {
    s.set_route(match engine.network() {
        NetworkMode::Normal => Route::Direct,
        NetworkMode::Blocked => Route::Held,
        // Never the system resolver while on the VPN.
        NetworkMode::Vpn(b) => {
            match riptor_core::dns::TunnelResolver::new(&b.end(), &riptor_core::dns::SERVERS) {
                Ok(r) => Route::Tunnel(riptor_search::scrape::Tunnel {
                    end: b.end(),
                    lookup: r.hook(),
                }),
                Err(_) => Route::Held,
            }
        }
    });
}

#[derive(Clone, Debug, Default)]
pub struct Job {
    pub query: String,
    pub results: Vec<SearchResult>,
    pub done: bool,
}

#[derive(Clone, Debug)]
pub struct Status {
    pub id: u64,
    pub done: bool,
    pub total: usize,
}

#[derive(Default)]
pub struct Jobs {
    jobs: Arc<Mutex<HashMap<u64, Job>>>,
}

fn record(jobs: &Mutex<HashMap<u64, Job>>, ev: SearchEvent) {
    let mut jobs = jobs.lock();
    match ev {
        SearchEvent::Results {
            search_id, results, ..
        } => {
            let job = jobs.entry(search_id).or_default();
            let room = MAX_RESULTS.saturating_sub(job.results.len());
            job.results.extend(results.into_iter().take(room));
        }
        SearchEvent::Counts { search_id, counts } => {
            if let Some(job) = jobs.get_mut(&search_id) {
                for c in counts {
                    for r in job.results.iter_mut().filter(|r| r.link == c.link) {
                        r.seeders = Some(c.seeders);
                        r.leechers = Some(c.leechers);
                    }
                }
            }
        }
        SearchEvent::LookupsDone { search_id } => jobs.entry(search_id).or_default().done = true,
        SearchEvent::ProviderDone { .. } | SearchEvent::Finished { .. } => {}
    }
}

impl Jobs {
    pub fn running(&self) -> usize {
        self.jobs.lock().values().filter(|j| !j.done).count()
    }

    pub async fn start(
        &self,
        engine: &Engine,
        s: &Arc<SearchManager>,
        query: &str,
        category: &str,
        providers: Option<Vec<String>>,
    ) -> anyhow::Result<u64> {
        ensure!(
            self.running() < MAX_RUNNING,
            "Unable to create more than {MAX_RUNNING} concurrent searches."
        );
        {
            let mut jobs = self.jobs.lock();
            while jobs.len() >= MAX_KEPT {
                let Some(oldest) = jobs.iter().filter(|(_, j)| j.done).map(|(id, _)| *id).min()
                else {
                    break;
                };
                jobs.remove(&oldest);
            }
        }
        route(engine, s);
        let jobs = Arc::downgrade(&self.jobs);
        let sink: Arc<dyn Fn(SearchEvent) + Send + Sync> = Arc::new(move |ev| {
            if let Some(jobs) = jobs.upgrade() {
                record(&jobs, ev);
            }
        });
        let id = s.start(query, category, providers, sink).await?;
        self.jobs.lock().entry(id).or_default().query = query.trim().to_string();
        Ok(id)
    }

    pub fn stop(&self, s: &SearchManager, id: u64) -> bool {
        s.cancel(id);
        match self.jobs.lock().get_mut(&id) {
            Some(j) => {
                j.done = true;
                true
            }
            None => false,
        }
    }

    pub fn delete(&self, s: &SearchManager, id: u64) -> bool {
        s.cancel(id);
        self.jobs.lock().remove(&id).is_some()
    }

    pub fn status(&self, id: Option<u64>) -> Vec<Status> {
        let mut v: Vec<Status> = self
            .jobs
            .lock()
            .iter()
            .filter(|(k, _)| id.is_none_or(|id| **k == id))
            .map(|(k, j)| Status {
                id: *k,
                done: j.done,
                total: j.results.len(),
            })
            .collect();
        v.sort_by_key(|s| s.id);
        v
    }

    /// A page of results, or `None` for an unknown search.
    pub fn results(&self, id: u64, offset: usize, limit: Option<usize>) -> Option<(Job, usize)> {
        let jobs = self.jobs.lock();
        let j = jobs.get(&id)?;
        let total = j.results.len();
        let page = j
            .results
            .iter()
            .skip(offset)
            .take(limit.unwrap_or(usize::MAX))
            .cloned()
            .collect();
        Some((
            Job {
                query: j.query.clone(),
                results: page,
                done: j.done,
            },
            total,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn result(link: &str) -> SearchResult {
        SearchResult {
            name: link.into(),
            link: link.into(),
            size: None,
            seeders: None,
            leechers: None,
            desc_link: None,
            pub_date: None,
            engine: "archive".into(),
            via_plugin: false,
        }
    }

    #[test]
    fn events_fill_a_job_until_lookups_end() {
        let jobs = Jobs::default();
        record(
            &jobs.jobs,
            SearchEvent::Results {
                search_id: 7,
                provider: "archive".into(),
                results: vec![result("magnet:?xt=a"), result("magnet:?xt=b")],
            },
        );
        record(
            &jobs.jobs,
            SearchEvent::Counts {
                search_id: 7,
                counts: vec![riptor_search::scrape::ResultCounts {
                    link: "magnet:?xt=b".into(),
                    seeders: 9,
                    leechers: 3,
                }],
            },
        );
        let s = jobs.status(Some(7));
        assert_eq!((s.len(), s[0].done, s[0].total), (1, false, 2));
        assert_eq!(jobs.running(), 1);
        let (page, total) = jobs.results(7, 1, Some(5)).unwrap();
        assert_eq!(total, 2);
        assert_eq!(page.results[0].seeders, Some(9));
        record(&jobs.jobs, SearchEvent::LookupsDone { search_id: 7 });
        assert!(jobs.status(None)[0].done);
        assert_eq!(jobs.running(), 0);
        assert!(jobs.results(8, 0, None).is_none());
    }

    #[test]
    fn a_flood_of_results_is_capped() {
        let jobs = Jobs::default();
        for _ in 0..3 {
            record(
                &jobs.jobs,
                SearchEvent::Results {
                    search_id: 1,
                    provider: "x".into(),
                    results: (0..MAX_RESULTS / 2 + 1)
                        .map(|i| result(&format!("magnet:?xt={i}")))
                        .collect(),
                },
            );
        }
        assert_eq!(jobs.status(Some(1))[0].total, MAX_RESULTS);
    }
}
