//! Tasks: things to do, with an optional due day and a priority.

use serde::{Deserialize, Serialize};

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
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum TaskError {
    #[error("a task needs a title")]
    NoTitle,
}

impl Task {
    /// Cleans the free text; what comes back is safe to store and show.
    pub fn validated(mut self) -> Result<Self, TaskError> {
        self.title = clean_text(self.title.trim());
        self.notes = self.notes.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).take(MAX_NOTES).collect();
        if self.title.is_empty() {
            return Err(TaskError::NoTitle);
        }
        if !self.done {
            self.completed = None;
        }
        Ok(self)
    }

    /// Whether the task is open and its due day has passed, `now` being the start of today.
    pub fn overdue(&self, today: i64) -> bool {
        !self.done && self.due.is_some_and(|d| d < today)
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
}
