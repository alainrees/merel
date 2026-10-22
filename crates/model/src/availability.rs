//! Whether someone is free now, and their working hours, as Outlook shows on a contact card.
//! Only Microsoft 365 answers this, for people whose calendar it may look at.

use serde::{Deserialize, Serialize};

/// What someone's calendar says about now.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Presence {
    Free,
    Tentative,
    Busy,
    /// Out of office.
    Away,
    WorkingElsewhere,
    /// Outside their working hours.
    OffHours,
}

/// The days and hours someone works, in their own time zone.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkingHours {
    /// The working days, 0 for Monday to 6 for Sunday.
    pub days: Vec<u8>,
    /// Start and end as `HH:MM`.
    pub start: String,
    pub end: String,
    /// Their time zone by IANA name, when it is known.
    pub zone: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Availability {
    pub now: Presence,
    /// When `now` ends, in seconds since the Unix epoch: the end of a meeting, the start of the
    /// next one, or the start of the working day. `None` when nothing changes soon.
    pub until: Option<i64>,
    pub working: Option<WorkingHours>,
    /// Their time zone's distance from UTC now, in seconds; `None` when it is not known.
    pub offset: Option<i32>,
}

/// Windows time zone names, as Outlook and Exchange write them, with an IANA zone that keeps
/// the same clock.
pub const WINDOWS_ZONES: &[(&str, &str)] = &[
    ("W. Europe Standard Time", "Europe/Amsterdam"),
    ("Romance Standard Time", "Europe/Paris"),
    ("Central Europe Standard Time", "Europe/Budapest"),
    ("Central European Standard Time", "Europe/Warsaw"),
    ("GMT Standard Time", "Europe/London"),
    ("Greenwich Standard Time", "Atlantic/Reykjavik"),
    ("GTB Standard Time", "Europe/Athens"),
    ("FLE Standard Time", "Europe/Helsinki"),
    ("E. Europe Standard Time", "Europe/Chisinau"),
    ("Turkey Standard Time", "Europe/Istanbul"),
    ("Russian Standard Time", "Europe/Moscow"),
    ("South Africa Standard Time", "Africa/Johannesburg"),
    ("Egypt Standard Time", "Africa/Cairo"),
    ("Israel Standard Time", "Asia/Jerusalem"),
    ("Arabian Standard Time", "Asia/Dubai"),
    ("India Standard Time", "Asia/Kolkata"),
    ("China Standard Time", "Asia/Shanghai"),
    ("Singapore Standard Time", "Asia/Singapore"),
    ("Tokyo Standard Time", "Asia/Tokyo"),
    ("AUS Eastern Standard Time", "Australia/Sydney"),
    ("New Zealand Standard Time", "Pacific/Auckland"),
    ("Eastern Standard Time", "America/New_York"),
    ("Central Standard Time", "America/Chicago"),
    ("Mountain Standard Time", "America/Denver"),
    ("Pacific Standard Time", "America/Los_Angeles"),
    ("Atlantic Standard Time", "America/Halifax"),
    ("E. South America Standard Time", "America/Sao_Paulo"),
    ("UTC", "UTC"),
    ("Coordinated Universal Time", "UTC"),
];

/// The IANA name of a Windows zone name; `None` for one not in [`WINDOWS_ZONES`].
pub fn iana_of_windows(name: &str) -> Option<&'static str> {
    WINDOWS_ZONES.iter().find(|(w, _)| w.eq_ignore_ascii_case(name.trim())).map(|(_, iana)| *iana)
}
