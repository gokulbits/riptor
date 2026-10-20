//! Just enough ISO 8601 and RFC 2822 parsing for search results.

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = (m + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

fn to_unix(y: i64, mo: i64, d: i64, h: i64, mi: i64, s: i64, offset_secs: i64) -> Option<u64> {
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let t = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s - offset_secs;
    u64::try_from(t).ok()
}

/// `2012-01-01T10:20:30Z`, `2012-01-01T10:20:30+02:00`, or `2012-01-01`.
pub fn parse_iso8601(s: &str) -> Option<u64> {
    let s = s.trim();
    let num = |a: usize, b: usize| s.get(a..b)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0, 4)?, num(5, 7)?, num(8, 10)?);
    if s.len() == 10 {
        return to_unix(y, mo, d, 0, 0, 0, 0);
    }
    let (h, mi, se) = (num(11, 13)?, num(14, 16)?, num(17, 19).unwrap_or(0));
    let tz = s[19..].trim_start_matches(|c: char| c == '.' || c.is_ascii_digit());
    let off = match tz.as_bytes().first() {
        None | Some(b'Z') => 0,
        Some(b'+') | Some(b'-') => {
            let sign = if tz.starts_with('-') { -1 } else { 1 };
            let hh = tz.get(1..3)?.parse::<i64>().ok()?;
            let mm = tz.get(tz.len() - 2..)?.parse::<i64>().ok()?;
            sign * (hh * 3600 + mm * 60)
        }
        _ => return None,
    };
    to_unix(y, mo, d, h, mi, se, off)
}

/// `Sat, 18 Oct 2026 10:00:00 +0000` (the weekday is optional).
pub fn parse_rfc2822(s: &str) -> Option<u64> {
    let s = s.trim();
    let s = s.split_once(',').map_or(s, |(_, rest)| rest).trim();
    let mut it = s.split_whitespace();
    let d = it.next()?.parse::<i64>().ok()?;
    let mon = it.next()?;
    let mo = [
        "jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec",
    ]
    .iter()
    .position(|m| mon.get(..3).is_some_and(|x| x.eq_ignore_ascii_case(m)))? as i64
        + 1;
    let y = it.next()?.parse::<i64>().ok()?;
    let mut hms = it.next()?.split(':').map(|p| p.parse::<i64>().ok());
    let (h, mi, se) = (
        hms.next()??,
        hms.next()??,
        hms.next().flatten().unwrap_or(0),
    );
    let off = match it.next() {
        Some(z) if z.len() == 5 && (z.starts_with('+') || z.starts_with('-')) => {
            let sign = if z.starts_with('-') { -1 } else { 1 };
            sign * (z[1..3].parse::<i64>().ok()? * 3600 + z[3..5].parse::<i64>().ok()? * 60)
        }
        _ => 0, // GMT/UT/missing
    };
    to_unix(y, mo, d, h, mi, se, off)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iso() {
        assert_eq!(parse_iso8601("1970-01-01T00:00:00Z"), Some(0));
        assert_eq!(parse_iso8601("2026-10-18"), Some(1_792_281_600));
        assert_eq!(
            parse_iso8601("2026-10-18T02:00:00+02:00"),
            Some(1_792_281_600)
        );
        assert_eq!(
            parse_iso8601("2026-10-18T00:00:00.123Z"),
            Some(1_792_281_600)
        );
        assert_eq!(parse_iso8601("garbage"), None);
        assert_eq!(parse_iso8601("2026-13-18"), None);
    }

    #[test]
    fn rfc2822() {
        assert_eq!(
            parse_rfc2822("Sun, 18 Oct 2026 00:00:00 +0000"),
            Some(1_792_281_600)
        );
        assert_eq!(
            parse_rfc2822("18 Oct 2026 01:00:00 +0100"),
            Some(1_792_281_600)
        );
        assert_eq!(
            parse_rfc2822("Sun, 18 Oct 2026 00:00:00 GMT"),
            Some(1_792_281_600)
        );
        assert_eq!(parse_rfc2822("nonsense"), None);
    }
}
