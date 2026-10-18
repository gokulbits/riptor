use serde::{Deserialize, Serialize};

use crate::{Action, Intent, Prefs, TorrentView, MAX_RATIO};

/// A per-torrent override. qBittorrent stores these as -2 / -1 / value.
#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mode", content = "value", rename_all = "snake_case")]
pub enum Limit<T> {
    #[default]
    UseGlobal,
    Unlimited,
    Value(T),
}

impl<T: Copy> Limit<T> {
    /// Resolve against the global setting (`None` = no global limit).
    pub fn resolve(self, global: Option<T>) -> Option<T> {
        match self {
            Limit::UseGlobal => global,
            Limit::Unlimited => None,
            Limit::Value(v) => Some(v),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct ShareLimits {
    pub ratio: Limit<f64>,
    /// Minutes.
    pub seeding_time: Limit<u64>,
    /// Minutes.
    pub inactive_seeding_time: Limit<u64>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShareLimitAction {
    #[default]
    Stop,
    Remove,
    RemoveWithFiles,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LimitKind {
    Ratio,
    SeedingTime,
    InactiveSeedingTime,
}

/// Which limit (if any) a torrent has reached. Mirrors the order and
/// conditions of qBittorrent's `SessionImpl::processShareLimits()`: only
/// finished, non-forced torrents are checked, ratio first, then seeding
/// time, then inactive seeding time. Stopped torrents are skipped so a
/// Remove action never deletes something the user paused.
pub fn reached(t: &TorrentView, prefs: &Prefs) -> Option<LimitKind> {
    if !t.finished || t.intent != Intent::Started || t.busy_or_error {
        return None;
    }
    if let Some(max) = t.limits.ratio.resolve(prefs.max_ratio) {
        let ratio = t.ratio();
        if max >= 0.0 && ratio <= MAX_RATIO && ratio >= max {
            return Some(LimitKind::Ratio);
        }
    }
    if let Some(min) = t.limits.seeding_time.resolve(prefs.max_seeding_minutes) {
        if t.seeding_time >= min * 60 {
            return Some(LimitKind::SeedingTime);
        }
    }
    if let Some(min) = t
        .limits
        .inactive_seeding_time
        .resolve(prefs.max_inactive_seeding_minutes)
    {
        if t.time_since_activity >= min * 60 {
            return Some(LimitKind::InactiveSeedingTime);
        }
    }
    None
}

pub fn check_all(torrents: &[TorrentView], prefs: &Prefs) -> Vec<Action> {
    torrents
        .iter()
        .filter_map(|t| {
            reached(t, prefs).map(|reason| Action::ShareLimit {
                id: t.id,
                action: t.limit_action.unwrap_or(prefs.limit_action),
                reason,
            })
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_util::seed;

    fn prefs() -> Prefs {
        Prefs {
            max_ratio: Some(2.0),
            max_seeding_minutes: Some(60),
            max_inactive_seeding_minutes: Some(30),
            ..Prefs::default()
        }
    }

    #[test]
    fn table() {
        type Case = (&'static str, fn(&mut TorrentView), Option<LimitKind>);
        let cases: &[Case] = &[
            ("fresh seed", |_| {}, None),
            (
                "ratio reached",
                |t| t.all_time_upload = 2000,
                Some(LimitKind::Ratio),
            ),
            ("ratio just under", |t| t.all_time_upload = 1999, None),
            (
                "seeding time",
                |t| t.seeding_time = 3600,
                Some(LimitKind::SeedingTime),
            ),
            ("seeding time under", |t| t.seeding_time = 3599, None),
            (
                "inactive",
                |t| t.time_since_activity = 1800,
                Some(LimitKind::InactiveSeedingTime),
            ),
            (
                "ratio wins over time",
                |t| {
                    t.all_time_upload = 5000;
                    t.seeding_time = 99999;
                },
                Some(LimitKind::Ratio),
            ),
            (
                "still downloading",
                |t| {
                    t.finished = false;
                    t.all_time_upload = 5000;
                },
                None,
            ),
            (
                "forced is exempt",
                |t| {
                    t.intent = Intent::Forced;
                    t.all_time_upload = 5000;
                },
                None,
            ),
            (
                "stopped is exempt",
                |t| {
                    t.intent = Intent::Stopped;
                    t.all_time_upload = 5000;
                },
                None,
            ),
            (
                "per-torrent unlimited ratio",
                |t| {
                    t.limits.ratio = Limit::Unlimited;
                    t.all_time_upload = 5000;
                },
                None,
            ),
            (
                "per-torrent custom ratio",
                |t| {
                    t.limits.ratio = Limit::Value(0.5);
                    t.all_time_upload = 500;
                },
                Some(LimitKind::Ratio),
            ),
            (
                "custom zero seeding time stops immediately",
                |t| t.limits.seeding_time = Limit::Value(0),
                Some(LimitKind::SeedingTime),
            ),
            (
                "upload with nothing downloaded is max ratio",
                |t| {
                    t.all_time_download = 0;
                    t.total_done = 0;
                    t.all_time_upload = 1;
                },
                Some(LimitKind::Ratio),
            ),
        ];
        for (name, setup, want) in cases {
            let mut t = seed(1);
            setup(&mut t);
            assert_eq!(reached(&t, &prefs()), *want, "{name}");
        }
    }

    #[test]
    fn no_global_limits_means_nothing_fires() {
        let mut t = seed(1);
        t.all_time_upload = u64::MAX / 4;
        t.seeding_time = u64::MAX / 4;
        t.time_since_activity = u64::MAX / 4;
        assert_eq!(reached(&t, &Prefs::default()), None);
    }

    #[test]
    fn per_torrent_action_overrides_global() {
        let mut t = seed(7);
        t.all_time_upload = 5000;
        t.limit_action = Some(ShareLimitAction::RemoveWithFiles);
        assert_eq!(
            check_all(&[t], &prefs()),
            vec![Action::ShareLimit {
                id: 7,
                action: ShareLimitAction::RemoveWithFiles,
                reason: LimitKind::Ratio
            }]
        );
    }

    #[test]
    fn limit_serde_shape() {
        let s = serde_json::to_string(&Limit::Value(1.5f64)).unwrap();
        assert_eq!(s, r#"{"mode":"value","value":1.5}"#);
        let s = serde_json::to_string(&Limit::<f64>::UseGlobal).unwrap();
        assert_eq!(s, r#"{"mode":"use_global"}"#);
    }
}
