//! Tasks: things to do, with an optional due day and a priority.

use chrono::{Days, Months, NaiveTime, TimeZone};
use serde::{Deserialize, Serialize};

use crate::calendar::Frequency;
use crate::{MAX_NOTES, clean_text};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Priority {
    Low,
    #[default]
    Normal,
    High,
}

impl Priority {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "low" => Priority::Low,
            "normal" => Priority::Normal,
            "high" => Priority::High,
            _ => return None,
        })
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub notes: String,
    /// Local midnight of the day it is due, as Unix seconds; `None` for no deadline.
    #[serde(default)]
    pub due: Option<i64>,
    #[serde(default)]
    pub priority: Priority,
    #[serde(default)]
    pub done: bool,
    /// When it was marked done, as Unix seconds.
    #[serde(default)]
    pub completed: Option<i64>,
    /// The account the task lives in; `None` for one kept only on this computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// How the task comes back once it is done, Outlook's "Terugkeerpatroon" on a task; only
    /// for tasks kept on this computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub repeat: Option<Repeat>,
    /// The formatting over `notes`, kept on this computer; servers get the notes as text.
    /// `None` for plain notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_rich: Option<crate::rich::Rich>,
}

fn one() -> u32 {
    1
}

/// A task's repeat: finishing it makes the next one, due `interval` days, weeks, months or
/// years after this one's due day.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Repeat {
    pub frequency: Frequency,
    /// Every so many units; at least 1.
    #[serde(default = "one")]
    pub interval: u32,
}

impl Repeat {
    pub fn every(frequency: Frequency) -> Self {
        Repeat { frequency, interval: 1 }
    }

    /// The due day of the next task after one due at `due` (midnight in `tz`), as midnight in
    /// `tz`. A month or a year later lands on the same day of the month, or the last day the
    /// month has.
    pub fn next_due<Tz: TimeZone>(&self, due: i64, tz: &Tz) -> Option<i64> {
        let n = self.interval.max(1);
        let day = tz.timestamp_opt(due, 0).single()?.date_naive();
        let next = match self.frequency {
            Frequency::Daily => day.checked_add_days(Days::new(u64::from(n)))?,
            Frequency::Weekly => day.checked_add_days(Days::new(7 * u64::from(n)))?,
            Frequency::Monthly => day.checked_add_months(Months::new(n))?,
            Frequency::Yearly => day.checked_add_months(Months::new(12 * n))?,
        };
        tz.from_local_datetime(&next.and_time(NaiveTime::MIN)).earliest().map(|d| d.timestamp())
    }

    /// "Elke week", "Om de 2 maanden".
    pub fn describe(&self) -> String {
        let (one, many) = match self.frequency {
            Frequency::Daily => ("dag", "dagen"),
            Frequency::Weekly => ("week", "weken"),
            Frequency::Monthly => ("maand", "maanden"),
            Frequency::Yearly => ("jaar", "jaar"),
        };
        if self.interval <= 1 { format!("Elke {one}") } else { format!("Om de {} {many}", self.interval) }
    }
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TaskError {
    #[error("a task needs a title")]
    NoTitle,
    #[error("a repeating task needs a due day and lives on this computer")]
    RepeatNeedsDue,
}

impl Task {
    /// Cleans the free text; what comes back is safe to store and show.
    pub fn validated(mut self) -> Result<Self, TaskError> {
        self.title = clean_text(self.title.trim());
        self.notes = self.notes.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).take(MAX_NOTES).collect();
        self.notes_rich = crate::rich::fitted(self.notes_rich.take(), &self.notes);
        if self.title.is_empty() {
            return Err(TaskError::NoTitle);
        }
        if !self.done {
            self.completed = None;
        }
        if self.repeat.is_some() && (self.due.is_none() || self.account_id.is_some()) {
            return Err(TaskError::RepeatNeedsDue);
        }
        Ok(self)
    }

    /// Whether the task is open and its due day has passed, `now` being the start of today.
    pub fn overdue(&self, today: i64) -> bool {
        !self.done && self.due.is_some_and(|d| d < today)
    }

    /// Whether every word of `query` (case-insensitive) occurs in the title or the notes.
    pub fn matches(&self, query: &str) -> bool {
        let text = format!("{}\n{}", self.title, self.notes).to_lowercase();
        query.split_whitespace().all(|w| text.contains(&w.to_lowercase()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_and_overdue() {
        let t = Task { title: " Offerte sturen ".into(), notes: "a\u{7}b".into(), due: Some(100), completed: Some(5), ..Task::default() }
            .validated()
            .unwrap();
        assert_eq!((t.title.as_str(), t.notes.as_str(), t.completed), ("Offerte sturen", "ab", None));
        assert!(t.overdue(200) && !t.overdue(100));
        assert_eq!(Task::default().validated(), Err(TaskError::NoTitle));
        assert_eq!(Priority::parse("high"), Some(Priority::High));
        assert!(Priority::Low < Priority::High);
    }

    #[test]
    fn a_repeat_needs_a_due_day_on_a_local_task() {
        let weekly = Some(Repeat::every(Frequency::Weekly));
        let t = Task { title: "x".into(), repeat: weekly, ..Task::default() };
        assert_eq!(t.clone().validated(), Err(TaskError::RepeatNeedsDue));
        assert_eq!(Task { due: Some(0), account_id: Some("a".into()), ..t.clone() }.validated(), Err(TaskError::RepeatNeedsDue));
        assert!(Task { due: Some(0), ..t }.validated().is_ok());
    }

    #[test]
    fn the_next_due_day_steps_by_the_unit_and_clamps_to_the_month() {
        use chrono::{NaiveDate, Utc};
        let midnight = |y, m, d| Utc.from_utc_datetime(&NaiveDate::from_ymd_opt(y, m, d).unwrap().and_time(NaiveTime::MIN)).timestamp();
        let next = |r: Repeat, from| r.next_due(from, &Utc);
        assert_eq!(next(Repeat { interval: 2, ..Repeat::every(Frequency::Weekly) }, midnight(2026, 10, 19)), Some(midnight(2026, 11, 2)));
        assert_eq!(next(Repeat::every(Frequency::Daily), midnight(2026, 12, 31)), Some(midnight(2027, 1, 1)));
        assert_eq!(next(Repeat::every(Frequency::Monthly), midnight(2026, 1, 31)), Some(midnight(2026, 2, 28)));
        assert_eq!(next(Repeat::every(Frequency::Yearly), midnight(2028, 2, 29)), Some(midnight(2029, 2, 28)));
        assert_eq!(Repeat::every(Frequency::Weekly).describe(), "Elke week");
        assert_eq!(Repeat { interval: 3, ..Repeat::every(Frequency::Monthly) }.describe(), "Om de 3 maanden");
    }
}
