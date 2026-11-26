//! Search, RSS and the log buffer, shared by the desktop app, `riptord` and remote control.
#![forbid(unsafe_code)]

pub mod logs;
pub mod rss;
pub mod search;

use std::sync::{Arc, OnceLock};

use riptor_rss::Rss;
use riptor_search::SearchManager;

/// What changed behind the app's back, so its windows can reload.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Change {
    Rss,
    SearchEngines,
}

type OnChange = Arc<dyn Fn(Change) + Send + Sync>;

/// Filled in as each part starts; remote control answers "not ready" until then.
#[derive(Default)]
pub struct Hub {
    search: OnceLock<Arc<SearchManager>>,
    rss: OnceLock<Arc<Rss>>,
    pub searches: search::Jobs,
    on_change: OnceLock<OnChange>,
}

impl Hub {
    pub fn set_search(&self, s: Arc<SearchManager>) {
        let _ = self.search.set(s);
    }

    pub fn set_rss(&self, r: Arc<Rss>) {
        let _ = self.rss.set(r);
    }

    pub fn on_change(&self, f: OnChange) {
        let _ = self.on_change.set(f);
    }

    pub fn search(&self) -> Option<Arc<SearchManager>> {
        self.search.get().cloned()
    }

    pub fn rss(&self) -> Option<Arc<Rss>> {
        self.rss.get().cloned()
    }

    pub fn changed(&self, c: Change) {
        if let Some(f) = self.on_change.get() {
            f(c);
        }
    }
}
