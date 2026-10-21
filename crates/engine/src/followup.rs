//! Follow-up flags with a date, Outlook's "Opvolgen": a flagged message can carry the day it
//! is due (today, tomorrow, this week, next week or a day of choice). The flag goes to the
//! server as before; the date stays on this computer in `follow_ups.json`, filed under the
//! message's Message-ID so a move between folders keeps it. Taking the flag off takes the
//! date with it. From the due day on, the reminders window names the message, as Outlook
//! reminds of flagged items; closing that reminder is kept in the file, putting it off is not.

use chrono::{Datelike, Days, NaiveDate};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use merel_model::{FollowUp, MessageHeader};

use crate::{CommandError, Session, commands};

/// What `follow_ups.json` holds.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct FollowUpFile {
    #[serde(default)]
    pub follow_ups: Vec<FollowUp>,
}

/// Outlook's choices for the due day, as `message.follow_up` takes them and the menu shows them.
pub const WHEN: [(&str, &str); 4] = [("today", "Vandaag"), ("tomorrow", "Morgen"), ("this_week", "Deze week"), ("next_week", "Volgende week")];

/// What a message's follow-up is filed under: its Message-ID, else its id.
pub fn key(m: &MessageHeader) -> String {
    m.message_id.clone().filter(|k| !k.is_empty()).unwrap_or_else(|| m.id.clone())
}

/// The day a choice means, seen from `today`: "this_week" is this week's Friday (today, when
/// Friday has passed), "next_week" next week's; "YYYY-MM-DD" names a day itself.
pub fn due_for(when: &str, today: NaiveDate) -> Option<NaiveDate> {
    match when {
        "today" => Some(today),
        "tomorrow" => today.checked_add_days(Days::new(1)),
        "this_week" => friday_of(today).map(|f| f.max(today)),
        "next_week" => friday_of(today).and_then(|f| f.checked_add_days(Days::new(7))),
        day => NaiveDate::parse_from_str(day, "%Y-%m-%d").ok(),
    }
}

fn friday_of(day: NaiveDate) -> Option<NaiveDate> {
    day.checked_sub_days(Days::new(u64::from(day.weekday().num_days_from_monday()))).and_then(|monday| monday.checked_add_days(Days::new(4)))
}

/// The ISO day of a follow-up, parsed.
pub fn due_day(f: &FollowUp) -> Option<NaiveDate> {
    NaiveDate::parse_from_str(&f.due, "%Y-%m-%d").ok()
}

impl Session {
    pub fn set_follow_ups(&mut self, list: Vec<FollowUp>) {
        self.follow_ups = list;
    }

    pub fn follow_ups_data(&self) -> FollowUpFile {
        FollowUpFile { follow_ups: self.follow_ups.clone() }
    }

    /// Whether a command changed the list since the last call.
    pub fn take_follow_ups_dirty(&mut self) -> bool {
        std::mem::take(&mut self.follow_ups_dirty)
    }

    /// The follow-up of a message, when it has a date.
    pub fn follow_up(&self, message_id: &str) -> Option<&FollowUp> {
        let m = self.message(message_id)?;
        let k = key(m);
        self.follow_ups.iter().find(|f| f.key == k)
    }

    /// The flagged messages whose follow-up day has come (seen from `today`) and whose reminder
    /// was neither closed nor put off past `now`: the earliest day first, then the newest mail.
    pub fn due_follow_ups(&self, today: NaiveDate, now: i64) -> Vec<(MessageHeader, NaiveDate)> {
        let mut seen: Vec<String> = Vec::new();
        let mut due: Vec<(MessageHeader, NaiveDate)> = self
            .messages
            .iter()
            .filter(|m| m.flags.flagged)
            .filter_map(|m| {
                let k = key(m);
                let f = self.follow_ups.iter().find(|f| f.key == k && !f.dismissed)?;
                let day = due_day(f).filter(|d| *d <= today)?;
                if self.snoozed_follow_ups.get(&k).is_some_and(|until| *until > now) || seen.contains(&k) {
                    return None;
                }
                seen.push(k);
                Some((m.clone(), day))
            })
            .collect();
        due.sort_by(|(a, da), (b, db)| da.cmp(db).then(b.date.cmp(&a.date)));
        due
    }

    /// Drops the date of a message whose flag came off.
    pub(crate) fn clear_follow_up(&mut self, message_id: &str) {
        let Some(k) = self.message(message_id).map(key) else { return };
        let before = self.follow_ups.len();
        self.follow_ups.retain(|f| f.key != k);
        if self.follow_ups.len() != before {
            self.follow_ups_dirty = true;
        }
    }
}

/// `message.follow_up { when, id? }`: flags the given message (else the whole selection) for
/// follow-up on the day `when` names ("today", "tomorrow", "this_week", "next_week" or
/// "YYYY-MM-DD"); "none" flags it without a date.
pub fn follow_up(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let when = commands::str_param(params, "when")?;
    let due = match when {
        "none" => None,
        w => Some(due_for(w, chrono::Local::now().date_naive()).ok_or(CommandError::Param("when"))?),
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
        commands::flag_message(s, &id, true)?;
        let k = key(&m);
        s.follow_ups.retain(|f| f.key != k);
        s.snoozed_follow_ups.remove(&k);
        if let Some(due) = due {
            s.follow_ups.push(FollowUp { key: k, due: due.format("%Y-%m-%d").to_string(), subject: m.subject, dismissed: false });
        }
        s.follow_ups_dirty = true;
    }
    Ok(())
}

/// The follow-up a reminder command is about: the message `id` names, with a date.
fn entry_key(s: &Session, params: &Value) -> Result<String, CommandError> {
    let id = commands::str_param(params, "id")?;
    let m = s.message(id).ok_or(CommandError::NotFound)?;
    let k = key(m);
    s.follow_ups.iter().find(|f| f.key == k).map(|_| k).ok_or(CommandError::NotFound)
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// `followup.dismiss { id }`: closes the reminder of the message's follow-up; the flag and the
/// day stay. A new day (another `message.follow_up`) opens it again.
pub fn dismiss(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let k = entry_key(s, params)?;
    s.snoozed_follow_ups.remove(&k);
    if let Some(f) = s.follow_ups.iter_mut().find(|f| f.key == k && !f.dismissed) {
        f.dismissed = true;
        s.follow_ups_dirty = true;
    }
    Ok(())
}

/// `followup.snooze { id, minutes }`: puts the message's follow-up reminder off for `minutes`.
pub fn snooze(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let k = entry_key(s, params)?;
    let minutes = params
        .get("minutes")
        .and_then(Value::as_u64)
        .filter(|m| (1..=u64::from(merel_model::MAX_REMINDER)).contains(m))
        .ok_or(CommandError::Param("minutes"))?;
    s.snoozed_follow_ups.insert(k, now() + minutes as i64 * 60);
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    fn day(s: &str) -> NaiveDate {
        NaiveDate::parse_from_str(s, "%Y-%m-%d").unwrap()
    }

    #[test]
    fn the_choices_land_on_outlooks_days() {
        let wed = day("2026-10-21");
        assert_eq!(due_for("today", wed), Some(wed));
        assert_eq!(due_for("tomorrow", wed), Some(day("2026-10-22")));
        assert_eq!(due_for("this_week", wed), Some(day("2026-10-23")), "this week's Friday");
        assert_eq!(due_for("next_week", wed), Some(day("2026-10-30")));
        let sat = day("2026-10-24");
        assert_eq!(due_for("this_week", sat), Some(sat), "Friday has passed, so today");
        assert_eq!(due_for("next_week", sat), Some(day("2026-10-30")));
        assert_eq!(due_for("2026-12-01", wed), Some(day("2026-12-01")));
        assert_eq!(due_for("someday", wed), None);
    }

    #[test]
    fn a_follow_up_reminds_from_its_day_until_closed_or_put_off() {
        let mut s = Session::sample();
        let ids: Vec<String> = s.visible_messages().iter().take(2).map(|m| m.id.clone()).collect();
        let today = chrono::Local::now().date_naive();
        let t = now();
        s.run("message.follow_up", &json!({ "when": "today", "id": ids[0] })).unwrap();
        s.run("message.follow_up", &json!({ "when": "tomorrow", "id": ids[1] })).unwrap();
        let due = s.due_follow_ups(today, t);
        assert_eq!(due.iter().map(|(m, _)| m.id.as_str()).collect::<Vec<_>>(), [ids[0].as_str()], "tomorrow's is not due yet");
        assert_eq!(due[0].1, today);
        assert_eq!(s.due_follow_ups(today.succ_opt().unwrap(), t).len(), 2, "a passed day keeps reminding");

        s.run("followup.snooze", &json!({ "id": ids[0], "minutes": 5 })).unwrap();
        assert!(s.due_follow_ups(today, t).is_empty());
        assert_eq!(s.due_follow_ups(today, t + 301).len(), 1, "and it is back after five minutes");
        assert_eq!(s.run("followup.snooze", &json!({ "id": ids[0], "minutes": 0 })), Err(CommandError::Param("minutes")));
        assert_eq!(s.run("followup.dismiss", &json!({ "id": "missing" })), Err(CommandError::NotFound));

        s.take_follow_ups_dirty();
        s.run("followup.dismiss", &json!({ "id": ids[0] })).unwrap();
        assert!(s.take_follow_ups_dirty(), "closing is kept in the file");
        assert!(s.due_follow_ups(today, t + 301).is_empty());
        assert!(s.message(&ids[0]).unwrap().flags.flagged && s.follow_up(&ids[0]).is_some(), "the flag and the day stay");
        s.run("message.follow_up", &json!({ "when": "today", "id": ids[0] })).unwrap();
        assert_eq!(s.due_follow_ups(today, t).len(), 1, "a new day opens the reminder again");
        s.run("message.flag", &json!({ "id": ids[0] })).unwrap();
        assert!(s.due_follow_ups(today, t).is_empty(), "no flag, no reminder");
    }

    #[test]
    fn a_follow_up_flags_the_selection_with_a_day_and_leaves_with_the_flag() {
        let mut s = Session::sample();
        let ids: Vec<String> = s.visible_messages().iter().take(2).map(|m| m.id.clone()).collect();
        assert_eq!(s.run("message.follow_up", &json!({ "when": "tomorrow" })), Err(CommandError::NoSelection));
        s.run("message.select_many", &json!({ "ids": ids })).unwrap();
        assert_eq!(s.run("message.follow_up", &json!({ "when": "someday" })), Err(CommandError::Param("when")));
        s.run("message.follow_up", &json!({ "when": "tomorrow" })).unwrap();
        assert!(s.take_follow_ups_dirty());
        let tomorrow = chrono::Local::now().date_naive().checked_add_days(Days::new(1)).unwrap();
        for id in &ids {
            assert!(s.message(id).unwrap().flags.flagged);
            assert_eq!(s.follow_up(id).and_then(due_day), Some(tomorrow));
        }
        // A day of choice on one message, by id; "none" keeps the flag but drops the day.
        s.run("message.follow_up", &json!({ "when": "2026-12-01", "id": ids[0] })).unwrap();
        assert_eq!(s.follow_up(&ids[0]).map(|f| f.due.as_str()), Some("2026-12-01"));
        s.run("message.follow_up", &json!({ "when": "none", "id": ids[0] })).unwrap();
        assert!(s.follow_up(&ids[0]).is_none() && s.message(&ids[0]).unwrap().flags.flagged);
        // The flag off (on the whole selection) takes the dates with it.
        s.take_follow_ups_dirty();
        s.run("message.flag", &Value::Null).unwrap();
        assert!(ids.iter().all(|id| !s.message(id).unwrap().flags.flagged && s.follow_up(id).is_none()));
        assert!(s.take_follow_ups_dirty());
        assert!(s.follow_ups.is_empty());
    }
}
