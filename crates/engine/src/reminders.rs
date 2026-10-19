//! Reminders: the pop-up before an appointment starts. Each appointment carries how many
//! minutes before its start the reminder comes ([`Appointment::reminder`]); the session works
//! out which reminders are due right now, and remembers which ones were closed or put off.
//!
//! A closed reminder is kept in `calendar.json` (for an account's appointment too, since the
//! servers do not record this), by the id and the start of the occurrence, and forgotten once
//! that occurrence is a day in the past. A put-off reminder lives in memory: after a restart
//! it comes back, which is what Outlook does too.

use std::collections::HashMap;

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use mailcraft_model::{Appointment, MAX_REMINDER};

use crate::{CommandError, Session};

/// One occurrence's reminder: the appointment's id and the start of that occurrence.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ReminderKey {
    pub id: String,
    pub start: i64,
}

/// How long after its start an occurrence keeps reminding when nobody closed it: a day.
pub const REMINDER_GRACE: i64 = 86_400;

/// Reminders put off until a moment, in memory.
pub type Snoozed = HashMap<ReminderKey, i64>;

impl Session {
    /// The occurrences whose reminder is due at `now` and was neither closed nor put off,
    /// earliest first. Each is a copy of its appointment with that occurrence's times.
    pub fn due_reminders(&self, now: i64) -> Vec<Appointment> {
        let from = now - REMINDER_GRACE;
        let to = now + i64::from(MAX_REMINDER) * 60 + 1;
        let mut due: Vec<Appointment> = self
            .appointments
            .iter()
            .filter(|a| a.reminder.is_some())
            .flat_map(|a| a.occurrences(from, to, &Local))
            .filter(|o| {
                let lead = i64::from(o.reminder.unwrap_or(0)) * 60;
                let key = ReminderKey { id: o.id.clone(), start: o.start };
                o.start - lead <= now
                    && o.start >= from
                    && !self.dismissed_reminders.contains(&key)
                    && !self.snoozed_reminders.get(&key).is_some_and(|until| *until > now)
            })
            .collect();
        due.sort_by(|a, b| (a.start, &a.id).cmp(&(b.start, &b.id)));
        due
    }

    /// Forgets closed reminders of occurrences more than a day in the past, so the file does
    /// not grow with every appointment there ever was.
    pub fn forget_old_reminders(&mut self, now: i64) {
        self.dismissed_reminders.retain(|k| k.start >= now - REMINDER_GRACE);
        self.snoozed_reminders.retain(|k, _| k.start >= now - REMINDER_GRACE);
    }
}

fn key_of(s: &Session, params: &Value) -> Result<ReminderKey, CommandError> {
    let id =
        params.get("id").and_then(Value::as_str).map(str::to_string).or_else(|| s.selected_appointment.clone()).ok_or(CommandError::NoSelection)?;
    s.appointment(&id).ok_or(CommandError::NotFound)?;
    let start = params.get("start").and_then(Value::as_i64).ok_or(CommandError::Param("start"))?;
    Ok(ReminderKey { id, start })
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `reminder.dismiss { id?, start }`: closes the reminder of the occurrence starting at `start`
/// for good.
pub fn dismiss(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let key = key_of(s, params)?;
    s.snoozed_reminders.remove(&key);
    if !s.dismissed_reminders.contains(&key) {
        s.dismissed_reminders.push(key);
    }
    s.forget_old_reminders(now());
    s.calendar_dirty = true;
    Ok(())
}

/// `reminder.snooze { id?, start, minutes }`: puts the reminder off for `minutes` from now.
pub fn snooze(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let key = key_of(s, params)?;
    let minutes =
        params.get("minutes").and_then(Value::as_u64).filter(|m| (1..=u64::from(MAX_REMINDER)).contains(m)).ok_or(CommandError::Param("minutes"))?;
    s.snoozed_reminders.insert(key, now() + minutes as i64 * 60);
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::calendar::CalendarFile;

    #[test]
    fn reminders_come_due_can_be_closed_or_put_off_and_closing_is_kept() {
        let mut s = Session::default();
        s.set_calendar_data(CalendarFile::default());
        let t = now();
        let start = t + 600;
        s.run("appointment.save", &json!({ "title": "Tandarts", "start": start, "end": start + 1800, "reminder": 15 })).unwrap();
        let id = s.selected_appointment().unwrap().id.clone();
        s.run("appointment.save", &json!({ "title": "Later", "start": t + 7200, "end": t + 9000, "reminder": 15 })).unwrap();
        s.run("appointment.save", &json!({ "title": "Stil", "start": start, "end": start + 1800 })).unwrap();
        let due = s.due_reminders(t);
        assert_eq!(
            due.iter().map(|a| a.title.as_str()).collect::<Vec<_>>(),
            vec!["Tandarts"],
            "due within its lead, not the one two hours off, not the one without"
        );
        assert_eq!(s.due_reminders(t + 7200 - 15 * 60).len(), 2);
        assert!(s.due_reminders(start + REMINDER_GRACE + 1).iter().all(|a| a.title == "Later"), "a day after its start a reminder gives up");

        s.run("reminder.snooze", &json!({ "id": id, "start": start, "minutes": 5 })).unwrap();
        assert!(s.due_reminders(t).is_empty());
        assert_eq!(s.due_reminders(t + 301).len(), 1, "and it is back after five minutes");
        assert_eq!(s.run("reminder.snooze", &json!({ "id": id, "start": start, "minutes": 0 })), Err(CommandError::Param("minutes")));
        assert_eq!(s.run("reminder.dismiss", &json!({ "id": "nope", "start": start })), Err(CommandError::NotFound));
        assert_eq!(s.run("reminder.dismiss", &json!({ "id": id })), Err(CommandError::Param("start")));

        s.take_calendar_dirty();
        s.run("reminder.dismiss", &json!({ "id": id, "start": start })).unwrap();
        assert!(s.take_calendar_dirty());
        assert!(s.due_reminders(t + 301).is_empty(), "closed for good");
        let saved = s.calendar_data();
        assert_eq!(saved.dismissed, vec![ReminderKey { id: id.clone(), start }]);
        let mut again = Session::default();
        again.set_calendar_data(saved);
        assert!(again.due_reminders(t + 301).is_empty(), "the file remembers what was closed");

        // A series reminds before every occurrence, each closed on its own.
        s.run("appointment.save", &json!({ "title": "Weekstart", "start": t - 86_400 * 7 + 300, "end": t - 86_400 * 7 + 2100, "reminder": 10, "recurrence": { "frequency": "weekly" } })).unwrap();
        let weekly = s.selected_appointment().unwrap().id.clone();
        let due = s.due_reminders(t);
        assert_eq!(due.iter().filter(|a| a.id == weekly).count(), 1);
        let this_week = due.iter().find(|a| a.id == weekly).unwrap().start;
        assert!(this_week > t && this_week - t <= 600);
        s.run("reminder.dismiss", &json!({ "id": weekly, "start": this_week })).unwrap();
        assert!(s.due_reminders(t).iter().all(|a| a.id != weekly));
        assert_eq!(s.due_reminders(t + 86_400 * 7).iter().filter(|a| a.id == weekly).count(), 1, "next week's is its own reminder");
        s.forget_old_reminders(t + 86_400 * 3);
        assert!(s.dismissed_reminders.iter().all(|k| k.start >= t + 86_400 * 2), "old closings are forgotten");
    }
}
