//! Calendars and appointments. Times are seconds since the Unix epoch, UTC; the UI shows them
//! in the computer's time zone. An all-day appointment runs from local midnight to the next.

use serde::{Deserialize, Serialize};

use crate::clean_text;

/// How long the notes of one appointment may be, in characters.
pub const MAX_NOTES: usize = 10_000;

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
        Calendar { id: "local".into(), name: "Mijn agenda".into(), account_id: None, color: [0x1F, 0x6F, 0xC9] }
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
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum AppointmentError {
    #[error("an appointment needs a title")]
    NoTitle,
    #[error("an appointment ends after it starts")]
    EndsBeforeStart,
}

impl Appointment {
    /// Cleans the free text and checks the times; what comes back is safe to store and show.
    pub fn validated(mut self) -> Result<Self, AppointmentError> {
        self.title = clean_text(self.title.trim());
        self.location = clean_text(self.location.trim());
        self.notes = self.notes.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).take(MAX_NOTES).collect();
        if self.title.is_empty() {
            return Err(AppointmentError::NoTitle);
        }
        if self.end <= self.start {
            return Err(AppointmentError::EndsBeforeStart);
        }
        Ok(self)
    }

    /// Whether any part of the appointment falls in `[from, to)`.
    pub fn overlaps(&self, from: i64, to: i64) -> bool {
        self.start < to && self.end > from
    }
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
        }
    }

    #[test]
    fn validation_cleans_and_checks() {
        let ok = a(0, 3600).validated().unwrap();
        assert_eq!((ok.title.as_str(), ok.notes.as_str()), ("Lunch", "xy"));
        assert_eq!(a(3600, 3600).validated(), Err(AppointmentError::EndsBeforeStart));
        let mut untitled = a(0, 1);
        untitled.title = "  ".into();
        assert_eq!(untitled.validated(), Err(AppointmentError::NoTitle));
    }

    #[test]
    fn overlap_is_half_open() {
        let x = a(100, 200);
        assert!(x.overlaps(150, 160) && x.overlaps(0, 101) && x.overlaps(199, 300));
        assert!(!x.overlaps(200, 300) && !x.overlaps(0, 100));
    }
}
