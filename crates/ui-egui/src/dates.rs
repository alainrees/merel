//! Dates for the message list and reading pane, in UTC until time zones arrive with the calendar.

const MONTHS: [&str; 12] = ["jan", "feb", "mrt", "apr", "mei", "jun", "jul", "aug", "sep", "okt", "nov", "dec"];

/// (year, month 1–12, day) from days since 1970-01-01 (Howard Hinnant's civil_from_days).
fn civil(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (yoe + era * 400 + i64::from(m <= 2), m, d)
}

fn parts(ts: i64) -> (i64, u32, u32, i64, i64) {
    let (y, m, d) = civil(ts.div_euclid(86_400));
    let secs = ts.rem_euclid(86_400);
    (y, m, d, secs / 3600, secs % 3600 / 60)
}

fn month(m: u32) -> &'static str {
    MONTHS.get(m.saturating_sub(1) as usize).copied().unwrap_or("")
}

/// "18 okt 14:05".
pub fn short(ts: i64) -> String {
    let (_, m, d, h, min) = parts(ts);
    format!("{d} {} {h:02}:{min:02}", month(m))
}

/// "18 okt 2026 14:05".
pub fn long(ts: i64) -> String {
    let (y, m, d, h, min) = parts(ts);
    format!("{d} {} {y} {h:02}:{min:02}", month(m))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_known_dates() {
        assert_eq!(long(0), "1 jan 1970 00:00");
        assert_eq!(long(1_792_000_000), "14 okt 2026 17:46");
        assert_eq!(short(951_782_400), "29 feb 00:00");
    }
}
