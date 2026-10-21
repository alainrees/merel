//! Snoozed mail, Outlook's "Sluimeren": a message leaves the list until a moment of choice
//! (later today, tomorrow morning, the weekend, next week) and comes back unread when that
//! moment has passed. The list lives on this computer in `snoozed.json`, filed under the
//! message's Message-ID so a move between folders keeps it; the server is told nothing but the
//! unread flag when the message returns. The "Gesluimerd" filter on the Beeld tab shows what
//! is put off.

use chrono::{DateTime, Datelike, Days, Local, TimeZone};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use merel_model::{MessageHeader, Snooze};

use crate::{CommandError, Session, commands, followup};

/// What `snoozed.json` holds.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SnoozeFile {
    #[serde(default)]
    pub snoozes: Vec<Snooze>,
}

/// Outlook's choices, as `message.snooze` takes them and the menu shows them.
pub const WHEN: [(&str, &str); 4] =
    [("later_today", "Later vandaag"), ("tomorrow", "Morgen"), ("weekend", "Dit weekend"), ("next_week", "Volgende week")];

/// The hour a snoozed message comes back on a later day.
const MORNING: u32 = 8;

/// The moment a choice means, seen from `now`: three hours on for "later_today", eight in the
/// morning for the other days ("weekend" is the coming Saturday, "next_week" the coming Monday).
pub fn until_for(when: &str, now: DateTime<Local>) -> Option<i64> {
    let today = now.date_naive();
    let morning = |day: chrono::NaiveDate| day.and_hms_opt(MORNING, 0, 0).and_then(|t| Local.from_local_datetime(&t).single()).map(|t| t.timestamp());
    let days_until = |weekday: u32| {
        let d = (weekday + 7 - now.weekday().num_days_from_monday()) % 7;
        u64::from(if d == 0 { 7 } else { d })
    };
    match when {
        "later_today" => Some(now.timestamp() + 3 * 3600),
        "tomorrow" => today.checked_add_days(Days::new(1)).and_then(morning),
        "weekend" => today.checked_add_days(Days::new(days_until(5))).and_then(morning),
        "next_week" => today.checked_add_days(Days::new(days_until(0))).and_then(morning),
        _ => None,
    }
}

pub(crate) fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

impl Session {
    pub fn set_snoozes(&mut self, list: Vec<Snooze>) {
        self.snoozes = list;
    }

    pub fn snoozes_data(&self) -> SnoozeFile {
        SnoozeFile { snoozes: self.snoozes.clone() }
    }

    /// Whether a command changed the list since the last call.
    pub fn take_snoozes_dirty(&mut self) -> bool {
        std::mem::take(&mut self.snoozes_dirty)
    }

    /// The moment the message comes back, when it is snoozed and that moment has not come.
    pub fn snoozed_until(&self, m: &MessageHeader, now: i64) -> Option<i64> {
        let k = followup::key(m);
        self.snoozes.iter().find(|s| s.key == k && s.until > now).map(|s| s.until)
    }

    /// The snoozed messages whose moment has come, newest first; `message.wake_snoozed` brings
    /// them back.
    pub fn snoozes_due(&self, now: i64) -> Vec<MessageHeader> {
        let mut due: Vec<MessageHeader> =
            self.messages.iter().filter(|m| self.snoozes.iter().any(|s| s.until <= now && s.key == followup::key(m))).cloned().collect();
        due.sort_by_key(|m| std::cmp::Reverse(m.date));
        due
    }
}

/// `message.snooze { when, id? }`: puts the given message (else the whole selection) out of
/// the list until the moment `when` names ("later_today", "tomorrow", "weekend", "next_week");
/// `until` (Unix seconds, in the future) names the moment itself.
pub fn snooze(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let now = now();
    let until = match params.get("until").and_then(Value::as_i64) {
        Some(t) if t > now => t,
        Some(_) => return Err(CommandError::Param("until")),
        None => until_for(commands::str_param(params, "when")?, Local::now()).ok_or(CommandError::Param("when"))?,
    };
    let ids = match params.get("id").and_then(Value::as_str) {
        Some(id) => vec![id.to_string()],
        None => s.selection(),
    };
    if ids.is_empty() {
        return Err(CommandError::NoSelection);
    }
    for id in ids {
        let m = s.message(&id).cloned().ok_or(CommandError::NotFound)?;
        let k = followup::key(&m);
        s.snoozes.retain(|x| x.key != k);
        s.snoozes.push(Snooze { key: k, until, subject: m.subject });
        s.snoozes_dirty = true;
    }
    Ok(())
}

/// `message.unsnooze { id? }`: brings the given message (else the selection) back now.
pub fn unsnooze(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let ids = match params.get("id").and_then(Value::as_str) {
        Some(id) => vec![id.to_string()],
        None => s.selection(),
    };
    if ids.is_empty() {
        return Err(CommandError::NoSelection);
    }
    for id in ids {
        let m = s.message(&id).cloned().ok_or(CommandError::NotFound)?;
        let k = followup::key(&m);
        let before = s.snoozes.len();
        s.snoozes.retain(|x| x.key != k);
        if s.snoozes.len() != before {
            s.snoozes_dirty = true;
        }
    }
    Ok(())
}

/// `message.wake_snoozed {}`: brings back every message whose moment has come, marked unread
/// as Outlook does, and forgets snoozes whose message is gone.
pub fn wake(s: &mut Session, _params: &Value) -> Result<(), CommandError> {
    let now = now();
    let due = s.snoozes_due(now);
    for m in &due {
        commands::set_seen_one(s, &m.id, false)?;
    }
    let before = s.snoozes.len();
    s.snoozes.retain(|x| x.until > now);
    if s.snoozes.len() != before {
        s.snoozes_dirty = true;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::view::Filter;

    #[test]
    fn the_moments_are_three_hours_on_or_eight_in_the_morning() {
        let wednesday = Local.with_ymd_and_hms(2026, 10, 21, 14, 30, 0).single().unwrap();
        let at = |d: u32, h: u32| Local.with_ymd_and_hms(2026, 10, d, h, 0, 0).single().unwrap().timestamp();
        assert_eq!(until_for("later_today", wednesday), Some(wednesday.timestamp() + 3 * 3600));
        assert_eq!(until_for("tomorrow", wednesday), Some(at(22, 8)));
        assert_eq!(until_for("weekend", wednesday), Some(at(24, 8)), "Saturday");
        assert_eq!(until_for("next_week", wednesday), Some(at(26, 8)), "Monday");
        let saturday = Local.with_ymd_and_hms(2026, 10, 24, 10, 0, 0).single().unwrap();
        assert_eq!(until_for("weekend", saturday), Some(at(31, 8)), "on a Saturday the weekend means the next one");
        assert_eq!(until_for("someday", wednesday), None);
    }

    #[test]
    fn a_snoozed_message_leaves_the_list_shows_under_the_filter_and_comes_back_unread() {
        let mut s = crate::sample::session();
        let id = "m1".to_string();
        s.selected_folder = s.message(&id).map(|m| m.folder_id.clone());
        assert!(s.visible_messages().iter().any(|m| m.id == id), "the sample inbox shows m1");
        s.run("message.select", &json!({ "id": id })).unwrap();
        s.run("message.mark_read", &Value::Null).unwrap();
        assert_eq!(s.run("message.snooze", &json!({ "when": "someday" })), Err(CommandError::Param("when")));
        assert_eq!(s.run("message.snooze", &json!({ "until": 5 })), Err(CommandError::Param("until")));
        s.run("message.snooze", &json!({ "when": "tomorrow" })).unwrap();
        assert!(s.take_snoozes_dirty() && !s.take_snoozes_dirty());
        let m = s.message(&id).unwrap().clone();
        assert!(s.snoozed_until(&m, now()).is_some());
        assert!(!s.visible_messages().iter().any(|m| m.id == id), "out of the list");
        s.filter = Filter::Snoozed;
        let shown: Vec<&str> = s.visible_messages().iter().map(|m| m.id.as_str()).collect();
        assert_eq!(shown, [id.as_str()], "the filter shows only what is put off");
        s.filter = Filter::All;

        // Nothing is due yet; a moment in the past brings it back unread.
        assert!(s.snoozes_due(now()).is_empty());
        s.run("message.wake_snoozed", &Value::Null).unwrap();
        assert_eq!(s.snoozes.len(), 1);
        s.snoozes[0].until = now() - 60;
        assert_eq!(s.snoozes_due(now()).len(), 1);
        s.run("message.wake_snoozed", &Value::Null).unwrap();
        assert!(s.snoozes.is_empty() && s.take_snoozes_dirty());
        let m = s.message(&id).unwrap();
        assert!(!m.flags.seen, "back as unread");
        assert!(s.visible_messages().iter().any(|m| m.id == id));

        s.run("message.snooze", &json!({ "when": "next_week", "id": id })).unwrap();
        s.run("message.unsnooze", &json!({ "id": id })).unwrap();
        assert!(s.snoozes.is_empty());
        assert_eq!(s.run("message.snooze", &json!({ "when": "tomorrow", "id": "nope" })), Err(CommandError::NotFound));
    }
}
