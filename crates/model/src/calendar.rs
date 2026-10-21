//! Calendars and appointments. Times are seconds since the Unix epoch, UTC; the UI shows them
//! in the computer's time zone. An all-day appointment runs from local midnight to the next.
//!
//! A recurring appointment is stored once, with a [`Recurrence`]; its occurrences are computed
//! when a period is shown ([`Appointment::occurrences`]). Repetition follows the wall clock of
//! the time zone the caller passes, so a weekly 10:00 stays 10:00 across a daylight-saving
//! change.

use chrono::{DateTime, Datelike, Days, Local, Months, NaiveDateTime, TimeZone};
use serde::{Deserialize, Serialize};

use crate::clean_text;

/// How long the notes of one appointment may be, in characters.
pub const MAX_NOTES: usize = 10_000;
/// The most occurrences one series yields in one expansion, so a wide window stays cheap.
pub const MAX_OCCURRENCES: usize = 5_000;
/// The largest step between occurrences.
pub const MAX_INTERVAL: u32 = 999;
/// The earliest a reminder may come, in minutes before the start: four weeks.
pub const MAX_REMINDER: u32 = 4 * 7 * 24 * 60;

/// The longest calendar name accepted, in characters.
pub const MAX_CALENDAR_NAME: usize = 100;

/// The colours a calendar can be given, with their Dutch names, as Outlook offers a row of
/// them. Each stands out at least 3:1 against the window in light and in dark mode, so the dot
/// in the calendar list stays visible, on a card too.
pub const CALENDAR_COLORS: [(&str, [u8; 3]); 12] = [
    ("Blauw", [0x2A, 0x78, 0xD2]),
    ("Turkoois", [0x00, 0x83, 0x8F]),
    ("Groen", [0x2E, 0x8B, 0x57]),
    ("Olijf", [0x6F, 0x7F, 0x1A]),
    ("Goud", [0xA8, 0x84, 0x00]),
    ("Oranje", [0xC8, 0x66, 0x0F]),
    ("Rood", [0xD4, 0x46, 0x46]),
    ("Roze", [0xC8, 0x46, 0x8F]),
    ("Paars", [0x8A, 0x62, 0xC8]),
    ("Lavendel", [0x9A, 0x6F, 0xD0]),
    ("Bruin", [0xA0, 0x6A, 0x46]),
    ("Grijs", [0x6E, 0x76, 0x80]),
];

/// A calendar: local, or one of an account once calendars sync.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Calendar {
    pub id: String,
    pub name: String,
    /// `None` for a calendar that lives only on this computer.
    pub account_id: Option<String>,
    /// RGB, the colour of its appointments.
    pub color: [u8; 3],
}

impl Calendar {
    /// The calendar every installation starts with.
    pub fn local() -> Self {
        Calendar { id: "local".into(), name: "Mijn agenda".into(), account_id: None, color: CALENDAR_COLORS[0].1 }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Appointment {
    pub id: String,
    pub calendar_id: String,
    pub title: String,
    pub start: i64,
    /// Exclusive: an appointment from 10:00 to 11:00 ends at 11:00.
    pub end: i64,
    pub all_day: bool,
    #[serde(default)]
    pub location: String,
    #[serde(default)]
    pub notes: String,
    /// How the appointment repeats; `None` for a single one. `start` and `end` are then the
    /// first occurrence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub recurrence: Option<Recurrence>,
    /// Minutes before the start a reminder comes; `None` for no reminder. For a series, before
    /// every occurrence.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reminder: Option<u32>,
    /// Outlook's "Privé": the details stay hidden from anyone the calendar is shared with. Goes
    /// to the server as iCalendar `CLASS:PRIVATE`, Exchange and Microsoft 365 sensitivity
    /// `Private` and Google visibility `private`.
    #[serde(default)]
    pub private: bool,
    /// Outlook's "Weergeven als": how the time counts for whoever looks at your availability.
    #[serde(default)]
    pub show_as: ShowAs,
    /// Outlook's categories, by name: the coloured labels the person knows from mail (their
    /// colours are in the session's category list). Travels as iCalendar `CATEGORIES`,
    /// Exchange and Microsoft 365 categories, and a private extended property on Google.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub categories: Vec<String>,
    /// The formatting over `notes` (bold, lists, colours…), kept on this computer; servers get
    /// the notes as text. `None` for plain notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_rich: Option<crate::rich::Rich>,
    /// Outlook's meeting: who organises it and who is invited, with their answers. `None` for
    /// an appointment of your own.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub meeting: Option<Meeting>,
    /// The link to join it online, as the server names it (Microsoft 365's online meeting,
    /// Google Meet). A link in the location or notes is found without it; see
    /// [`Appointment::meeting_link`].
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub online: Option<String>,
}

/// The most people one meeting invites.
pub const MAX_ATTENDEES: usize = 500;

/// A meeting: the appointment as the organiser sent it out (RFC 5546, iTIP), and what every
/// attendee answered.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Meeting {
    /// The iCalendar `UID` the invitations carry; the answers name it.
    pub uid: String,
    /// The organiser's address.
    pub organizer: String,
    /// The organiser's name, when known.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub organizer_name: String,
    /// The iCalendar `SEQUENCE`: one up with every update sent that moves the meeting.
    #[serde(default)]
    pub sequence: u32,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub attendees: Vec<Attendee>,
}

/// One person invited to a meeting.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attendee {
    pub email: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    /// Outlook's "Optioneel": invited, but not needed.
    #[serde(default)]
    pub optional: bool,
    #[serde(default)]
    pub response: Response,
    /// When the answer was sent (its `DTSTAMP`), so an older answer read later does not undo a
    /// newer one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub replied: Option<i64>,
    /// Outlook's "Nieuwe tijd voorgesteld": the start and end the attendee asked for.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proposed: Option<(i64, i64)>,
}

/// An attendee's answer, iCalendar's `PARTSTAT`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Response {
    /// No answer yet (`NEEDS-ACTION`).
    #[default]
    None,
    Accepted,
    Tentative,
    Declined,
}

impl Response {
    /// The `PARTSTAT` value for this answer.
    pub fn partstat(self) -> &'static str {
        match self {
            Response::None => "NEEDS-ACTION",
            Response::Accepted => "ACCEPTED",
            Response::Tentative => "TENTATIVE",
            Response::Declined => "DECLINED",
        }
    }

    /// The answer a `PARTSTAT` value stands for; anything unknown (delegated, …) is no answer.
    pub fn of_partstat(value: &str) -> Response {
        match value.trim().to_ascii_uppercase().as_str() {
            "ACCEPTED" => Response::Accepted,
            "TENTATIVE" => Response::Tentative,
            "DECLINED" => Response::Declined,
            _ => Response::None,
        }
    }
}

impl Attendee {
    /// The name to show: the name, else the address.
    pub fn label(&self) -> &str {
        if self.name.trim().is_empty() { &self.email } else { self.name.trim() }
    }
}

impl Meeting {
    /// The attendee with this address, compared without case.
    pub fn attendee_mut(&mut self, email: &str) -> Option<&mut Attendee> {
        self.attendees.iter_mut().find(|a| a.email.eq_ignore_ascii_case(email.trim()))
    }

    /// Whether `email` organises the meeting.
    pub fn organized_by(&self, email: &str) -> bool {
        self.organizer.eq_ignore_ascii_case(email.trim())
    }

    /// How many attendees gave each answer: accepted, tentative, declined, none.
    pub fn tally(&self) -> [usize; 4] {
        let mut n = [0; 4];
        for a in &self.attendees {
            n[match a.response {
                Response::Accepted => 0,
                Response::Tentative => 1,
                Response::Declined => 2,
                Response::None => 3,
            }] += 1;
        }
        n
    }

    /// Cleans the names, drops attendees without a valid address or listed twice, and the
    /// organiser from the attendees.
    fn validated(mut self) -> Self {
        self.uid = self.uid.chars().filter(|c| c.is_ascii_graphic()).take(255).collect();
        self.organizer = self.organizer.trim().to_string();
        self.organizer_name = clean_text(self.organizer_name.trim());
        let mut seen: Vec<String> = vec![self.organizer.to_lowercase()];
        self.attendees.retain_mut(|a| {
            a.email = a.email.trim().to_string();
            a.name = clean_text(a.name.trim());
            let key = a.email.to_lowercase();
            let ok = crate::Address::new(None, &a.email).is_ok() && !seen.contains(&key);
            seen.push(key);
            ok
        });
        self.attendees.truncate(MAX_ATTENDEES);
        self
    }
}

/// How an appointment shows in your availability, Outlook's "Weergeven als". Travels as the
/// server's own free/busy status: iCalendar `TRANSP` plus `X-MICROSOFT-CDO-BUSYSTATUS`, Exchange
/// `LegacyFreeBusyStatus`, Microsoft 365 `showAs`, Google `transparency` (free or busy only).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ShowAs {
    Free,
    Tentative,
    #[default]
    Busy,
    /// Out of office.
    Oof,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppointmentError {
    #[error("an appointment needs a title")]
    NoTitle,
    #[error("an appointment ends after it starts")]
    EndsBeforeStart,
    #[error("a recurring appointment must be shorter than its repeat interval")]
    LongerThanInterval,
}

/// The unit a recurrence counts in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Frequency {
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

fn one() -> u32 {
    1
}

/// How an appointment repeats: every `interval` days, weeks, months or years from its start,
/// on the given weekdays when weekly, until `until` or for `count` occurrences, with some
/// occurrences skipped. The vocabulary is RFC 5545's (`FREQ`, `INTERVAL`, `BYDAY`, `UNTIL`,
/// `COUNT`, `EXDATE`) so a series can later travel to a server as an `RRULE`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Recurrence {
    pub frequency: Frequency,
    /// Every so many units; at least 1.
    #[serde(default = "one")]
    pub interval: u32,
    /// Weekly only: the days it falls on, 0 = Monday to 6 = Sunday. Empty means the start's day.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub weekdays: Vec<u8>,
    /// The last moment an occurrence may start, inclusive.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub until: Option<i64>,
    /// How many occurrences there are, the skipped ones included.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub count: Option<u32>,
    /// Occurrences that were taken out, by the start they would have had.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub skipped: Vec<i64>,
}

impl Recurrence {
    /// A series that repeats every `frequency` without end.
    pub fn every(frequency: Frequency) -> Self {
        Recurrence { frequency, interval: 1, weekdays: Vec::new(), until: None, count: None, skipped: Vec::new() }
    }

    /// Puts the fields in range: interval 1..=999, weekdays 0..=6 without doubles, count at
    /// least 1, and the skipped list sorted without doubles.
    pub fn normalized(mut self) -> Self {
        self.interval = self.interval.clamp(1, MAX_INTERVAL);
        self.weekdays.retain(|d| *d < 7);
        self.weekdays.sort_unstable();
        self.weekdays.dedup();
        if self.frequency != Frequency::Weekly {
            self.weekdays.clear();
        }
        if self.count == Some(0) {
            self.count = None;
        }
        self.skipped.sort_unstable();
        self.skipped.dedup();
        self
    }

    /// The series as the value of an iCalendar `RRULE` (without the `RRULE:` name), as CalDAV
    /// and Google Calendar take it. `UNTIL` is a local date for an all-day appointment, else a
    /// UTC time, as the standard wants next to a `DTSTART` of that kind. The skipped
    /// occurrences are not part of the rule.
    pub fn rrule(&self, all_day: bool) -> String {
        const DAYS: [&str; 7] = ["MO", "TU", "WE", "TH", "FR", "SA", "SU"];
        let mut parts = vec![format!(
            "FREQ={}",
            match self.frequency {
                Frequency::Daily => "DAILY",
                Frequency::Weekly => "WEEKLY",
                Frequency::Monthly => "MONTHLY",
                Frequency::Yearly => "YEARLY",
            }
        )];
        if self.interval > 1 {
            parts.push(format!("INTERVAL={}", self.interval));
        }
        if self.frequency == Frequency::Weekly && !self.weekdays.is_empty() {
            let days: Vec<&str> = self.weekdays.iter().filter_map(|d| DAYS.get(usize::from(*d))).copied().collect();
            parts.push(format!("BYDAY={}", days.join(",")));
        }
        if let Some(u) = self.until {
            let t = DateTime::from_timestamp(u, 0).unwrap_or_default();
            parts.push(if all_day {
                format!("UNTIL={}", t.with_timezone(&Local).format("%Y%m%d"))
            } else {
                format!("UNTIL={}", t.format("%Y%m%dT%H%M%SZ"))
            });
        } else if let Some(c) = self.count {
            parts.push(format!("COUNT={c}"));
        }
        parts.join(";")
    }

    /// Whether the occurrence that would start at `start` was taken out.
    pub fn skips(&self, start: i64) -> bool {
        self.skipped.binary_search(&start).is_ok()
    }

    /// The occurrence starts of a series beginning at `first` (a wall-clock moment in `tz`),
    /// in series order, as long as they start before `to` and the end conditions allow; a
    /// closure sees each one and says whether to go on. Skipped occurrences are reported
    /// too, flagged, since they count towards `count`.
    fn walk<Tz: TimeZone>(&self, first: NaiveDateTime, to: i64, tz: &Tz, mut visit: impl FnMut(i64, bool) -> bool) {
        let mut produced = 0usize;
        let mut n: u32 = 0;
        let max = self.count.map_or(MAX_OCCURRENCES, |c| (c as usize).min(MAX_OCCURRENCES));
        let mut emit = |local: NaiveDateTime| -> Option<bool> {
            let start = tz.from_local_datetime(&local).earliest().or_else(|| tz.from_local_datetime(&local).latest())?.timestamp();
            if start >= to || self.until.is_some_and(|u| start > u) {
                return Some(false);
            }
            produced += 1;
            Some(visit(start, self.skips(start)) && produced < max)
        };
        match self.frequency {
            Frequency::Weekly => {
                let mut days = self.weekdays.clone();
                if days.is_empty() {
                    days.push(first.weekday().num_days_from_monday() as u8);
                }
                let week = first.checked_sub_days(Days::new(u64::from(first.weekday().num_days_from_monday())));
                let Some(week) = week else { return };
                loop {
                    let Some(base) = week.checked_add_days(Days::new(7 * u64::from(self.interval) * u64::from(n))) else { return };
                    for d in &days {
                        let Some(local) = base.checked_add_days(Days::new(u64::from(*d))) else { return };
                        if local < first {
                            continue;
                        }
                        match emit(local) {
                            Some(true) => {}
                            _ => return,
                        }
                    }
                    n += 1;
                    if n > MAX_OCCURRENCES as u32 {
                        return;
                    }
                }
            }
            Frequency::Daily | Frequency::Monthly | Frequency::Yearly => loop {
                let steps = self.interval.saturating_mul(n);
                let local = match self.frequency {
                    Frequency::Daily => first.checked_add_days(Days::new(u64::from(steps))),
                    Frequency::Monthly => first.checked_add_months(Months::new(steps)),
                    _ => first.checked_add_months(Months::new(steps.saturating_mul(12))),
                };
                let Some(local) = local else { return };
                match emit(local) {
                    Some(true) => {}
                    _ => return,
                }
                n += 1;
                if n > MAX_OCCURRENCES as u32 {
                    return;
                }
            },
        }
    }

    /// The shortest time between two occurrences, in seconds, for the overlap check.
    fn shortest_gap(&self) -> i64 {
        let interval = i64::from(self.interval.max(1));
        match self.frequency {
            Frequency::Daily => 86_400 * interval,
            // Several days a week can be next to each other, whatever the interval.
            Frequency::Weekly if self.weekdays.len() > 1 => 86_400,
            Frequency::Weekly => 7 * 86_400 * interval,
            Frequency::Monthly => 28 * 86_400 * interval,
            Frequency::Yearly => 365 * 86_400 * interval,
        }
    }
}

impl Appointment {
    /// Whether every word of `query` (case-insensitive) occurs in the title, the location or
    /// the notes.
    pub fn matches(&self, query: &str) -> bool {
        let text = format!("{}\n{}\n{}", self.title, self.location, self.notes).to_lowercase();
        query.split_whitespace().all(|w| text.contains(&w.to_lowercase()))
    }

    /// The way to join it online: the link the server named, else the first link to a meeting
    /// service in the location and then the notes.
    pub fn meeting_link(&self) -> Option<crate::MeetingLink> {
        self.online
            .as_deref()
            .and_then(crate::meeting_link)
            .or_else(|| crate::find_meeting_link(&self.location))
            .or_else(|| crate::find_meeting_link(&self.notes))
    }

    /// Cleans the free text and checks the times; what comes back is safe to store and show.
    pub fn validated(mut self) -> Result<Self, AppointmentError> {
        self.title = clean_text(self.title.trim());
        self.location = clean_text(self.location.trim());
        self.notes = self.notes.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).take(MAX_NOTES).collect();
        self.notes_rich = crate::rich::fitted(self.notes_rich.take(), &self.notes);
        if self.title.is_empty() {
            return Err(AppointmentError::NoTitle);
        }
        if self.end <= self.start {
            return Err(AppointmentError::EndsBeforeStart);
        }
        self.reminder = self.reminder.map(|m| m.min(MAX_REMINDER));
        self.categories = crate::clean_categories(std::mem::take(&mut self.categories));
        self.meeting = self.meeting.take().map(Meeting::validated);
        self.online = self.online.take().and_then(|u| crate::online::meeting_link(&u)).map(|l| l.url);
        if let Some(r) = self.recurrence.take() {
            let r = r.normalized();
            if self.end - self.start > r.shortest_gap() {
                return Err(AppointmentError::LongerThanInterval);
            }
            self.recurrence = Some(r);
        }
        Ok(self)
    }

    /// Whether any part of the appointment itself (the first occurrence, when it repeats)
    /// falls in `[from, to)`.
    pub fn overlaps(&self, from: i64, to: i64) -> bool {
        self.start < to && self.end > from
    }

    /// The occurrences touching `[from, to)`, as copies of the appointment with their own
    /// `start` and `end`, earliest first. A single appointment is its own one occurrence.
    /// `tz` is the time zone the series keeps its wall-clock time in.
    pub fn occurrences<Tz: TimeZone>(&self, from: i64, to: i64, tz: &Tz) -> Vec<Appointment> {
        let Some(r) = &self.recurrence else {
            return if self.overlaps(from, to) { vec![self.clone()] } else { Vec::new() };
        };
        let Some(first) = chrono::DateTime::from_timestamp(self.start, 0).map(|d| d.with_timezone(tz).naive_local()) else { return Vec::new() };
        let Some(first_end) = chrono::DateTime::from_timestamp(self.end, 0).map(|d| d.with_timezone(tz).naive_local()) else {
            return Vec::new();
        };
        let length = first_end - first;
        let mut out = Vec::new();
        r.walk(first, to, tz, |start, skipped| {
            if skipped {
                return true;
            }
            let end = chrono::DateTime::from_timestamp(start, 0)
                .map(|d| d.with_timezone(tz).naive_local())
                .and_then(|s| s.checked_add_signed(length))
                .and_then(|e| tz.from_local_datetime(&e).earliest().or_else(|| tz.from_local_datetime(&e).latest()))
                .map_or(start + (self.end - self.start), |e| e.timestamp())
                .max(start + 1);
            if end > from {
                out.push(Appointment { start, end, ..self.clone() });
            }
            true
        });
        out
    }

    /// The start of the occurrence on or after `at`, if the series has one.
    pub fn next_occurrence<Tz: TimeZone>(&self, at: i64, tz: &Tz) -> Option<i64> {
        self.occurrences(at, i64::MAX / 4, tz).iter().map(|a| a.start).find(|s| *s >= at)
    }

    /// Takes the occurrence that would start at `start` out of the series. A single
    /// appointment is left as it is.
    pub fn skip(&mut self, start: i64) {
        if let Some(r) = &mut self.recurrence
            && !r.skips(start)
        {
            r.skipped.push(start);
            r.skipped.sort_unstable();
        }
    }
}

/// One line that says how a series repeats, in Dutch, for the window and the chips.
pub fn describe_recurrence(r: &Recurrence) -> String {
    const DAYS: [&str; 7] = ["maandag", "dinsdag", "woensdag", "donderdag", "vrijdag", "zaterdag", "zondag"];
    let every = |one: &str, many: &str| if r.interval <= 1 { format!("Elke {one}") } else { format!("Om de {} {many}", r.interval) };
    let mut text = match r.frequency {
        Frequency::Daily => every("dag", "dagen"),
        Frequency::Weekly => {
            let names: Vec<&str> = r.weekdays.iter().filter_map(|d| DAYS.get(*d as usize).copied()).collect();
            let days = match names.len() {
                0 => String::new(),
                1 => format!(" op {}", names[0]),
                n => format!(" op {} en {}", names[..n - 1].join(", "), names[n - 1]),
            };
            format!("{}{days}", every("week", "weken"))
        }
        Frequency::Monthly => every("maand", "maanden"),
        Frequency::Yearly => every("jaar", "jaar"),
    };
    if let Some(c) = r.count {
        text.push_str(&format!(", {c} keer"));
    }
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    fn a(start: i64, end: i64) -> Appointment {
        Appointment {
            id: "a".into(),
            calendar_id: "local".into(),
            title: " Lunch ".into(),
            start,
            end,
            all_day: false,
            location: String::new(),
            notes: "x\u{7}y".into(),
            notes_rich: None,
            recurrence: None,
            reminder: None,
            private: false,
            show_as: ShowAs::Busy,
            categories: Vec::new(),
            meeting: None,
            online: None,
        }
    }

    /// 2026-10-19 (a Monday) 10:00 UTC.
    const MONDAY: i64 = 1_792_404_000;

    #[test]
    fn validation_cleans_and_checks() {
        let ok = a(0, 3600).validated().unwrap();
        assert_eq!((ok.title.as_str(), ok.notes.as_str()), ("Lunch", "xy"));
        let mut early = a(0, 3600);
        early.reminder = Some(u32::MAX);
        assert_eq!(early.validated().unwrap().reminder, Some(MAX_REMINDER));
        assert_eq!(a(3600, 3600).validated(), Err(AppointmentError::EndsBeforeStart));
        let mut untitled = a(0, 1);
        untitled.title = "  ".into();
        assert_eq!(untitled.validated(), Err(AppointmentError::NoTitle));
    }

    #[test]
    fn a_series_is_expanded_on_the_wall_clock_and_honours_its_end_and_skips() {
        use chrono::Utc;
        let day = 86_400;
        let mut weekly = a(MONDAY, MONDAY + 3600);
        weekly.recurrence = Some(Recurrence { weekdays: vec![0, 2], ..Recurrence::every(Frequency::Weekly) });
        let weekly = weekly.validated().unwrap();
        let starts: Vec<i64> = weekly.occurrences(MONDAY, MONDAY + 15 * day, &Utc).iter().map(|o| o.start).collect();
        assert_eq!(starts, vec![MONDAY, MONDAY + 2 * day, MONDAY + 7 * day, MONDAY + 9 * day, MONDAY + 14 * day]);
        assert!(weekly.occurrences(0, MONDAY, &Utc).is_empty(), "nothing before the first");
        // A window that starts mid-occurrence still shows it.
        assert_eq!(weekly.occurrences(MONDAY + 1800, MONDAY + 1900, &Utc).len(), 1);

        let mut counted = weekly.clone();
        counted.recurrence = Some(Recurrence { count: Some(3), ..counted.recurrence.clone().unwrap() });
        assert_eq!(counted.occurrences(0, MONDAY + 100 * day, &Utc).len(), 3);
        counted.skip(MONDAY + 2 * day);
        assert_eq!(counted.occurrences(0, MONDAY + 100 * day, &Utc).len(), 2, "a skipped occurrence still counts");
        assert_eq!(counted.next_occurrence(MONDAY + 1, &Utc), Some(MONDAY + 7 * day));

        let mut until = weekly.clone();
        until.recurrence = Some(Recurrence { until: Some(MONDAY + 8 * day), ..until.recurrence.clone().unwrap() });
        assert_eq!(until.occurrences(0, MONDAY + 100 * day, &Utc).len(), 3);

        // Monthly on the 31st lands on the last day of shorter months; yearly keeps the date.
        let jan31 = 1_769_817_600; // 2026-01-31 00:00 UTC
        let mut monthly = a(jan31, jan31 + day);
        monthly.all_day = true;
        monthly.recurrence = Some(Recurrence::every(Frequency::Monthly));
        let starts: Vec<i64> = monthly.occurrences(0, jan31 + 70 * day, &Utc).iter().map(|o| o.start).collect();
        assert_eq!(starts, vec![jan31, jan31 + 28 * day, jan31 + (28 + 31) * day]);
        let mut every_other_day = a(MONDAY, MONDAY + 3600);
        every_other_day.recurrence = Some(Recurrence { interval: 2, ..Recurrence::every(Frequency::Daily) });
        assert_eq!(every_other_day.occurrences(0, MONDAY + 5 * day, &Utc).len(), 3);

        // Across a daylight-saving change the local time stays, so the UTC gap is not a week.
        let ams = chrono_tz::Europe::Amsterdam;
        let mut dst = a(MONDAY, MONDAY + 3600);
        dst.recurrence = Some(Recurrence::every(Frequency::Weekly));
        let starts: Vec<i64> = dst.occurrences(0, MONDAY + 15 * day, &ams).iter().map(|o| o.start).collect();
        assert_eq!(starts[1] - starts[0], 7 * day + 3600, "the clock went back on 25 October 2026");
        assert_eq!(starts[2] - starts[1], 7 * day);

        assert_eq!(
            a(MONDAY, MONDAY + 2 * day).validated().map(|_| ()).and_then(|()| {
                let mut long = a(MONDAY, MONDAY + 2 * day);
                long.recurrence = Some(Recurrence::every(Frequency::Daily));
                long.validated().map(|_| ())
            }),
            Err(AppointmentError::LongerThanInterval)
        );
        let messy =
            Recurrence { interval: 0, weekdays: vec![9, 3, 3], count: Some(0), skipped: vec![5, 1, 5], ..Recurrence::every(Frequency::Weekly) }
                .normalized();
        assert_eq!((messy.interval, messy.weekdays.as_slice(), messy.count, messy.skipped.as_slice()), (1, &[3][..], None, &[1, 5][..]));
        assert_eq!(
            describe_recurrence(&Recurrence { weekdays: vec![0, 2, 4], count: Some(10), ..Recurrence::every(Frequency::Weekly) }),
            "Elke week op maandag, woensdag en vrijdag, 10 keer"
        );
        assert_eq!(describe_recurrence(&Recurrence { interval: 3, ..Recurrence::every(Frequency::Monthly) }), "Om de 3 maanden");
    }

    #[test]
    fn overlap_is_half_open() {
        let x = a(100, 200);
        assert!(x.overlaps(150, 160) && x.overlaps(0, 101) && x.overlaps(199, 300));
        assert!(!x.overlaps(200, 300) && !x.overlaps(0, 100));
    }
}
