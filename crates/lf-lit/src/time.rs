//! Time encodings used by value controllers.

/// Timeline time and duration values count ticks at this rate ("flicks").
pub const TICKS_PER_SECOND: i64 = 705_600_000;

/// Seconds between 1601-01-01 (Windows FILETIME epoch) and 1970-01-01.
const FILETIME_UNIX_OFFSET: i64 = 11_644_473_600;
const FILETIME_PER_SECOND: i64 = 10_000_000;

/// Format a Windows FILETIME (100 ns units since 1601) as `YYYY-MM-DDTHH:MM:SSZ`,
/// dropping sub-second precision.
pub fn filetime_to_iso(filetime: i64) -> String {
    let unix = filetime.div_euclid(FILETIME_PER_SECOND) - FILETIME_UNIX_OFFSET;
    let (days, secs) = (unix.div_euclid(86_400), unix.rem_euclid(86_400));
    let (y, m, d) = civil_from_days(days);
    format!(
        "{y:04}-{m:02}-{d:02}T{:02}:{:02}:{:02}Z",
        secs / 3600,
        secs / 60 % 60,
        secs % 60
    )
}

/// Parse `YYYY-MM-DDTHH:MM:SSZ` into a Windows FILETIME.
pub fn iso_to_filetime(text: &str) -> Option<i64> {
    let b = text.as_bytes();
    if b.len() != 20 || b[4] != b'-' || b[7] != b'-' || b[10] != b'T' || b[13] != b':' {
        return None;
    }
    if b[16] != b':' || b[19] != b'Z' {
        return None;
    }
    let num = |r: std::ops::Range<usize>| text.get(r)?.parse::<i64>().ok();
    let (y, mo, d) = (num(0..4)?, num(5..7)?, num(8..10)?);
    let (h, mi, s) = (num(11..13)?, num(14..16)?, num(17..19)?);
    if !(1..=12).contains(&mo) || !(1..=31).contains(&d) || h > 23 || mi > 59 || s > 60 {
        return None;
    }
    let unix = days_from_civil(y, mo, d) * 86_400 + h * 3600 + mi * 60 + s;
    Some((unix + FILETIME_UNIX_OFFSET) * FILETIME_PER_SECOND)
}

// Proleptic Gregorian conversions (H. Hinnant, "chrono-Compatible Low-Level
// Date Algorithms").
fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filetime_round_trips_through_iso() {
        let ft = iso_to_filetime("2022-08-12T14:41:41Z").unwrap();
        assert_eq!(filetime_to_iso(ft), "2022-08-12T14:41:41Z");
        assert_eq!(
            filetime_to_iso(FILETIME_UNIX_OFFSET * FILETIME_PER_SECOND),
            "1970-01-01T00:00:00Z"
        );
    }

    #[test]
    fn leap_day() {
        let ft = iso_to_filetime("2020-02-29T23:59:59Z").unwrap();
        assert_eq!(filetime_to_iso(ft), "2020-02-29T23:59:59Z");
    }

    #[test]
    fn rejects_bad_dates() {
        assert_eq!(iso_to_filetime("2020-13-01T00:00:00Z"), None);
        assert_eq!(iso_to_filetime("2020-02-28 14:54:08"), None);
    }
}
