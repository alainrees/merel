//! Dates for the message list and reading pane, in UTC until time zones arrive with the calendar;
//! the date groups of the list ("Vandaag", "Gisteren", …) follow the computer's time zone.

use chrono::{DateTime, Datelike, Local, NaiveDate};

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

/// The group a message of `ts` falls in when the list is grouped by date, seen from `today`:
/// the headings Outlook puts over the list.
pub fn date_group(ts: i64, today: NaiveDate) -> &'static str {
    let Some(day) = DateTime::from_timestamp(ts, 0).map(|t| t.with_timezone(&Local).date_naive()) else { return "Ouder" };
    let week_start = today - chrono::Duration::days(i64::from(today.weekday().num_days_from_monday()));
    let month_start = today.with_day(1).unwrap_or(today);
    let last_month_start = month_start.pred_opt().and_then(|d| d.with_day(1)).unwrap_or(month_start);
    if day > today {
        "Later"
    } else if day == today {
        "Vandaag"
    } else if today.pred_opt() == Some(day) {
        "Gisteren"
    } else if day >= week_start {
        "Deze week"
    } else if day >= week_start - chrono::Duration::days(7) {
        "Vorige week"
    } else if day >= month_start {
        "Deze maand"
    } else if day >= last_month_start {
        "Vorige maand"
    } else {
        "Ouder"
    }
}

#[cfg(test)]
mod tests {
    use chrono::TimeZone;

    use super::*;

    #[test]
    fn the_date_groups_read_like_outlook_s() {
        let today = NaiveDate::from_ymd_opt(2026, 10, 21).unwrap(); // a Wednesday
        let on = |y, m, d| Local.with_ymd_and_hms(y, m, d, 10, 0, 0).single().unwrap().timestamp();
        assert_eq!(date_group(on(2026, 10, 22), today), "Later");
        assert_eq!(date_group(on(2026, 10, 21), today), "Vandaag");
        assert_eq!(date_group(on(2026, 10, 20), today), "Gisteren");
        assert_eq!(date_group(on(2026, 10, 19), today), "Deze week", "Monday of this week");
        assert_eq!(date_group(on(2026, 10, 18), today), "Vorige week", "the Sunday before");
        assert_eq!(date_group(on(2026, 10, 12), today), "Vorige week");
        assert_eq!(date_group(on(2026, 10, 11), today), "Deze maand");
        assert_eq!(date_group(on(2026, 10, 1), today), "Deze maand");
        assert_eq!(date_group(on(2026, 9, 30), today), "Vorige maand");
        assert_eq!(date_group(on(2026, 9, 1), today), "Vorige maand");
        assert_eq!(date_group(on(2026, 8, 31), today), "Ouder");
        let first = NaiveDate::from_ymd_opt(2026, 1, 1).unwrap();
        assert_eq!(date_group(on(2025, 12, 15), first), "Vorige maand", "across the turn of the year");
    }

    #[test]
    fn formats_known_dates() {
        assert_eq!(long(0), "1 jan 1970 00:00");
        assert_eq!(long(1_792_000_000), "14 okt 2026 17:46");
        assert_eq!(short(951_782_400), "29 feb 00:00");
    }
}
