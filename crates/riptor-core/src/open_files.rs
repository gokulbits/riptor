//! Linux gives desktop apps 1024 open files unless they ask; a big list of torrents needs more.

#[cfg(unix)]
const WANTED: u64 = 65536;

pub fn raise_limit() {
    #[cfg(unix)]
    match rlimit::increase_nofile_limit(WANTED) {
        Ok(limit) => tracing::info!("open file limit: {limit}"),
        Err(e) => tracing::warn!("couldn't raise the open file limit: {e}"),
    }
}

/// Warns once when nearly out, so a failure that follows has an explanation in the logs.
pub fn check_usage() {
    #[cfg(target_os = "linux")]
    {
        use std::sync::atomic::{AtomicBool, Ordering};
        static WARNED: AtomicBool = AtomicBool::new(false);
        let Ok((limit, _)) = rlimit::Resource::NOFILE.get() else {
            return;
        };
        let used = match open_now() {
            Ok(used) => used,
            Err(e) => {
                if !WARNED.swap(true, Ordering::Relaxed) {
                    tracing::warn!("can't count open files (limit {limit}): {e}");
                }
                return;
            }
        };
        if nearly_out(used, limit) {
            if !WARNED.swap(true, Ordering::Relaxed) {
                tracing::warn!("{used} files and connections open, the limit is {limit}");
            }
        } else {
            WARNED.store(false, Ordering::Relaxed);
        }
    }
}

#[cfg(target_os = "linux")]
fn open_now() -> std::io::Result<u64> {
    Ok(std::fs::read_dir("/proc/self/fd")?.count() as u64)
}

#[cfg(target_os = "linux")]
fn nearly_out(used: u64, limit: u64) -> bool {
    used.saturating_mul(5) >= limit.saturating_mul(4)
}

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use super::*;

    #[test]
    fn raises_the_limit_and_counts_what_is_open() {
        raise_limit();
        let (soft, hard) = rlimit::Resource::NOFILE.get().unwrap();
        assert!(soft >= WANTED.min(hard));
        let before = open_now().unwrap();
        let _f = std::fs::File::open("/proc/self/stat").unwrap();
        assert!(open_now().unwrap() >= before);
        assert!(before > 0);
        assert!(nearly_out(80, 100));
        assert!(!nearly_out(79, 100));
    }
}
