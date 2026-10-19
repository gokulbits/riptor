/// Ratios are capped here; the UI may show this as "∞".
pub const MAX_RATIO: f64 = 9999.0;

/// Share ratio (the conventional "real ratio").
///
/// If the recorded download is under 1% of the data we have (for example a
/// torrent that was added already complete, or whose stats were lost), the
/// data we have is used as the denominator instead.
pub fn real_ratio(all_time_upload: u64, all_time_download: u64, total_done: u64) -> f64 {
    let download = if (all_time_download as f64) < (total_done as f64) * 0.01 {
        total_done
    } else {
        all_time_download
    };
    if download == 0 {
        return if all_time_upload == 0 { 0.0 } else { MAX_RATIO };
    }
    let ratio = all_time_upload as f64 / download as f64;
    ratio.min(MAX_RATIO)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cases() {
        // (upload, download, done, expected)
        let cases = [
            (0, 0, 0, 0.0),
            (100, 0, 0, MAX_RATIO),
            (500, 1000, 1000, 0.5),
            (2000, 1000, 1000, 2.0),
            // Added complete: download 0 but 1000 bytes done -> use done.
            (500, 0, 1000, 0.5),
            // Download just under 1% of done -> use done.
            (500, 9, 1000, 0.5),
            // Download at exactly 1% -> use download.
            (500, 10, 1000, 50.0),
            // Huge ratios are capped.
            (u64::MAX / 2, 1, 1, MAX_RATIO),
        ];
        for (up, dl, done, want) in cases {
            assert_eq!(
                real_ratio(up, dl, done),
                want,
                "up={up} dl={dl} done={done}"
            );
        }
    }
}
