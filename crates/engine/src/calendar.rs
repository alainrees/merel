//! Calendars and appointments in the session, and the commands that change them.
//!
//! Local appointments live in `calendar.json` next to the accounts file; an account's live in
//! its store and are mirrored here (see [`crate::sync`]). Every change goes through a command,
//! which marks the session dirty or leaves a job for the store; the engine saves right after
//! the command returns.
//!
//! A recurring appointment is one entry with a [`Recurrence`]; [`Session::appointments_between`]
//! expands it into the occurrences of the period asked for, on the computer's clock. In an
//! account's calendar a new appointment can start a series too: the rule goes to the server once,
//! which then owns the series and hands over single occurrences, edited one by one.

use chrono::Local;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use mailcraft_mailbox::Item;
use mailcraft_model::{Appointment, AppointmentError, Calendar, Recurrence};

use crate::{CommandError, Session};

/// What `calendar.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarFile {
    pub calendars: Vec<Calendar>,
    pub appointments: Vec<Appointment>,
    /// Reminders closed for good, of any calendar's appointments.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dismissed: Vec<crate::reminders::ReminderKey>,
}

impl Session {
    /// Replaces the calendars and appointments; there is always a local calendar.
    pub fn set_calendar_data(&mut self, data: CalendarFile) {
        self.calendars = data.calendars;
        self.appointments = data.appointments;
        self.dismissed_reminders = data.dismissed;
        if self.calendars.is_empty() {
            self.calendars.push(Calendar::local());
        }
        self.selected_appointment = None;
    }

    /// What goes to the file: the local calendars and their appointments.
    pub fn calendar_data(&self) -> CalendarFile {
        let calendars: Vec<Calendar> = self.calendars.iter().filter(|c| c.account_id.is_none()).cloned().collect();
        let appointments = self.appointments.iter().filter(|a| calendars.iter().any(|c| c.id == a.calendar_id)).cloned().collect();
        CalendarFile { calendars, appointments, dismissed: self.dismissed_reminders.clone() }
    }

    pub fn calendar(&self, id: &str) -> Option<&Calendar> {
        self.calendars.iter().find(|c| c.id == id)
    }

    pub fn appointment(&self, id: &str) -> Option<&Appointment> {
        self.appointments.iter().find(|a| a.id == id)
    }

    pub fn selected_appointment(&self) -> Option<&Appointment> {
        self.selected_appointment.as_deref().and_then(|id| self.appointment(id))
    }

    /// The appointments touching `[from, to)`, earliest first. A recurring appointment appears
    /// once per occurrence in the period, each a copy with that occurrence's `start` and `end`
    /// and the series' id.
    pub fn appointments_between(&self, from: i64, to: i64) -> Vec<Appointment> {
        let mut list: Vec<Appointment> = self.appointments.iter().flat_map(|a| a.occurrences(from, to, &Local)).collect();
        list.sort_by(|a, b| (a.start, a.end, &a.id).cmp(&(b.start, b.end, &b.id)));
        list
    }

    /// The occurrences in `[from, to)` of the appointments whose title, location or notes hold
    /// every word of `query`, earliest first; see [`Self::appointments_between`].
    pub fn appointments_matching(&self, query: &str, from: i64, to: i64) -> Vec<Appointment> {
        let mut list: Vec<Appointment> =
            self.appointments.iter().filter(|a| a.matches(query)).flat_map(|a| a.occurrences(from, to, &Local)).collect();
        list.sort_by(|a, b| (a.start, a.end, &a.id).cmp(&(b.start, b.end, &b.id)));
        list
    }

    /// Whether a command changed the calendar since the last call.
    pub fn take_calendar_dirty(&mut self) -> bool {
        std::mem::take(&mut self.calendar_dirty)
    }

    fn new_appointment_id(&mut self) -> String {
        self.next_appointment_id += 1;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        format!("a-{now}-{}", self.next_appointment_id)
    }
}

fn str_param<'a>(params: &'a Value, key: &'static str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

fn i64_param(params: &Value, key: &'static str) -> Result<i64, CommandError> {
    params.get(key).and_then(Value::as_i64).ok_or(CommandError::Param(key))
}

/// `appointment.select { id }`.
pub fn select(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id").ok_or(CommandError::Param("id"))?;
    s.appointment(id).ok_or(CommandError::NotFound)?;
    s.selected_appointment = Some(id.to_string());
    Ok(())
}

/// `appointment.save { id?, calendar_id?, title, start, end, all_day?, location?, notes?, recurrence?, reminder?, private?, show_as? }`:
/// creates the appointment, or replaces the one with `id`. Times are Unix seconds. `recurrence`
/// is a [`Recurrence`] object (or null for a single appointment); in an account's calendar only
/// a new appointment takes one (the server then owns the series and lists its occurrences one
/// by one), on this computer the occurrences a series had skipped stay skipped unless the
/// object names its own `skipped` list. `reminder` is minutes before the start, null for none;
/// left out, an existing appointment keeps its reminder. `private` marks the appointment as
/// Outlook's "Privé"; false when left out. `show_as` is `free`, `tentative`, `busy` or `oof`;
/// left out, an existing appointment keeps its status and a new one is busy.
pub fn save(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let existing = match str_param(params, "id") {
        Some(id) => Some(s.appointment(id).ok_or(CommandError::NotFound)?.clone()),
        None => None,
    };
    let calendar_id = match str_param(params, "calendar_id") {
        Some(c) => s.calendar(c).ok_or(CommandError::Param("calendar_id"))?.id.clone(),
        None => existing
            .as_ref()
            .map(|a| a.calendar_id.clone())
            .or_else(|| s.calendars.first().map(|c| c.id.clone()))
            .ok_or(CommandError::Param("calendar_id"))?,
    };
    let account = s.appointment_account(&calendar_id);
    let recurrence = match params.get("recurrence") {
        None | Some(Value::Null) => None,
        Some(r) => {
            if account.is_some() && existing.is_some() {
                return Err(CommandError::Param("recurrence"));
            }
            let mut r: Recurrence = serde_json::from_value(r.clone()).map_err(|_| CommandError::Param("recurrence"))?;
            if r.skipped.is_empty() && params.get("recurrence").and_then(|r| r.get("skipped")).is_none() {
                r.skipped = existing.as_ref().and_then(|a| a.recurrence.as_ref()).map(|old| old.skipped.clone()).unwrap_or_default();
            }
            Some(r)
        }
    };
    let reminder = match params.get("reminder") {
        None => existing.as_ref().and_then(|a| a.reminder),
        Some(Value::Null) => None,
        Some(v) => Some(v.as_u64().and_then(|m| u32::try_from(m).ok()).ok_or(CommandError::Param("reminder"))?),
    };
    let show_as = match params.get("show_as") {
        None => existing.as_ref().map(|a| a.show_as).unwrap_or_default(),
        Some(v) => serde_json::from_value::<mailcraft_model::ShowAs>(v.clone()).map_err(|_| CommandError::Param("show_as"))?,
    };
    let id = match (&existing, &account) {
        (Some(a), _) => a.id.clone(),
        (None, Some(account)) => s.pending_id(account),
        (None, None) => s.new_appointment_id(),
    };
    let appointment = Appointment {
        id: id.clone(),
        calendar_id,
        title: str_param(params, "title").unwrap_or("").to_string(),
        start: i64_param(params, "start")?,
        end: i64_param(params, "end")?,
        all_day: params.get("all_day").and_then(Value::as_bool).unwrap_or(false),
        location: str_param(params, "location").unwrap_or("").to_string(),
        notes: str_param(params, "notes").unwrap_or("").to_string(),
        recurrence,
        reminder,
        private: params.get("private").and_then(Value::as_bool).unwrap_or(false),
        show_as,
    }
    .validated()
    .map_err(|e| match e {
        AppointmentError::NoTitle => CommandError::Param("title"),
        AppointmentError::EndsBeforeStart => CommandError::Param("end"),
        AppointmentError::LongerThanInterval => CommandError::Param("recurrence"),
    })?;
    match &account {
        Some(account) => s.save_remote(account, existing.is_none(), Item::Appointment(appointment.clone()))?,
        None => s.calendar_dirty = true,
    }
    s.appointments.retain(|a| a.id != id);
    s.appointments.push(appointment);
    s.selected_appointment = Some(id);
    Ok(())
}

/// `appointment.move { id, start }`: shifts the appointment so it begins at `start` (Unix
/// seconds), keeping its length and everything else; the UI uses it when a block is dragged to
/// another day or time. A recurring appointment is refused with `Param("id")`: a series is
/// changed through its window. In an account's calendar the move goes to the server like a save.
pub fn move_to(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id").ok_or(CommandError::Param("id"))?.to_string();
    let start = i64_param(params, "start")?;
    let a = s.appointment(&id).ok_or(CommandError::NotFound)?.clone();
    if a.recurrence.is_some() {
        return Err(CommandError::Param("id"));
    }
    let moved = Appointment { start, end: start + (a.end - a.start), ..a };
    match s.appointment_account(&moved.calendar_id) {
        Some(account) => s.save_remote(&account, false, Item::Appointment(moved.clone()))?,
        None => s.calendar_dirty = true,
    }
    s.appointments.retain(|x| x.id != id);
    s.appointments.push(moved);
    s.selected_appointment = Some(id);
    Ok(())
}

/// `appointment.resize { id, end }`: gives the appointment a new end (Unix seconds), keeping
/// its start and everything else; the UI uses it when the bottom edge of a block is dragged. An
/// end at or before the start is `Param("end")`; a recurring appointment is refused with
/// `Param("id")`. In an account's calendar the change goes to the server like a save.
pub fn resize(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id").ok_or(CommandError::Param("id"))?.to_string();
    let end = i64_param(params, "end")?;
    let a = s.appointment(&id).ok_or(CommandError::NotFound)?.clone();
    if a.recurrence.is_some() {
        return Err(CommandError::Param("id"));
    }
    if end <= a.start {
        return Err(CommandError::Param("end"));
    }
    let resized = Appointment { end, ..a };
    match s.appointment_account(&resized.calendar_id) {
        Some(account) => s.save_remote(&account, false, Item::Appointment(resized.clone()))?,
        None => s.calendar_dirty = true,
    }
    s.appointments.retain(|x| x.id != id);
    s.appointments.push(resized);
    s.selected_appointment = Some(id);
    Ok(())
}

/// `appointment.skip { id?, start }`: takes one occurrence, the one starting at `start`, out of
/// a recurring appointment (the given one, else the selected one). The series stays.
pub fn skip(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id").map(str::to_string).or_else(|| s.selected_appointment.clone()).ok_or(CommandError::NoSelection)?;
    let start = i64_param(params, "start")?;
    let a = s.appointments.iter_mut().find(|a| a.id == id).ok_or(CommandError::NotFound)?;
    if a.recurrence.is_none() {
        return Err(CommandError::Param("id"));
    }
    a.skip(start);
    s.calendar_dirty = true;
    Ok(())
}

/// `appointment.delete { id? }`: the given appointment, else the selected one; a recurring
/// appointment goes with all its occurrences.
pub fn delete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id").map(str::to_string).or_else(|| s.selected_appointment.clone()).ok_or(CommandError::NoSelection)?;
    let calendar_id = s.appointment(&id).ok_or(CommandError::NotFound)?.calendar_id.clone();
    s.appointments.retain(|a| a.id != id);
    if s.selected_appointment.as_deref() == Some(id.as_str()) {
        s.selected_appointment = None;
    }
    match s.appointment_account(&calendar_id) {
        Some(account) => s.delete_remote(&account, &id),
        None => s.calendar_dirty = true,
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_search_lists_the_matching_occurrences_in_the_period() {
        let mut s = Session::default();
        s.set_calendar_data(CalendarFile::default());
        s.run("appointment.save", &json!({ "title": "Tandarts", "start": 3_600, "end": 7_200, "location": "Centrum" })).unwrap();
        s.run("appointment.save", &json!({ "title": "Weekstart", "start": 10_000, "end": 11_800, "recurrence": { "frequency": "weekly" } })).unwrap();
        let week = 7 * 86_400;
        let titles = |list: Vec<Appointment>| list.into_iter().map(|a| (a.title, a.start)).collect::<Vec<_>>();
        assert_eq!(titles(s.appointments_matching("centrum", 0, 3 * week)), [("Tandarts".to_string(), 3_600)], "the location counts");
        assert_eq!(titles(s.appointments_matching("WEEK", 0, 3 * week)).len(), 3, "one hit per occurrence in the period");
        assert_eq!(titles(s.appointments_matching("week start", 0, week)), [("Weekstart".to_string(), 10_000)], "every word must occur");
        assert!(s.appointments_matching("lunch", 0, 3 * week).is_empty());
    }

    #[test]
    fn a_dragged_appointment_keeps_its_length_but_a_series_stays_put() {
        let mut s = Session::default();
        s.set_calendar_data(CalendarFile::default());
        s.run("appointment.save", &json!({ "title": "Tandarts", "start": 3_600, "end": 7_200, "location": "Centrum" })).unwrap();
        let id = s.selected_appointment().unwrap().id.clone();
        s.take_calendar_dirty();
        s.run("appointment.move", &json!({ "id": id, "start": 90_000 })).unwrap();
        let a = s.appointment(&id).unwrap();
        assert_eq!((a.start, a.end, a.location.as_str()), (90_000, 93_600, "Centrum"));
        assert!(s.take_calendar_dirty(), "the move is saved");
        s.run("appointment.save", &json!({ "title": "Weekstart", "start": 10_000, "end": 11_800, "recurrence": { "frequency": "weekly" } })).unwrap();
        let series = s.selected_appointment().unwrap().id.clone();
        assert!(matches!(s.run("appointment.move", &json!({ "id": series, "start": 20_000 })), Err(CommandError::Param("id"))));
        assert_eq!(s.appointment(&series).unwrap().start, 10_000);
        assert!(matches!(s.run("appointment.move", &json!({ "id": "nope", "start": 1 })), Err(CommandError::NotFound)));
        assert!(matches!(s.run("appointment.move", &json!({ "id": id })), Err(CommandError::Param("start"))));
    }

    #[test]
    fn a_resized_appointment_keeps_its_start_and_cannot_end_before_it() {
        let mut s = Session::default();
        s.set_calendar_data(CalendarFile::default());
        s.run("appointment.save", &json!({ "title": "Tandarts", "start": 3_600, "end": 7_200, "location": "Centrum" })).unwrap();
        let id = s.selected_appointment().unwrap().id.clone();
        s.take_calendar_dirty();
        s.run("appointment.resize", &json!({ "id": id, "end": 9_000 })).unwrap();
        let a = s.appointment(&id).unwrap();
        assert_eq!((a.start, a.end, a.location.as_str()), (3_600, 9_000, "Centrum"));
        assert!(s.take_calendar_dirty(), "the new end is saved");
        s.run("appointment.resize", &json!({ "id": id, "end": 5_400 })).unwrap();
        assert_eq!(s.appointment(&id).unwrap().end, 5_400, "shorter works too");
        assert!(matches!(s.run("appointment.resize", &json!({ "id": id, "end": 3_600 })), Err(CommandError::Param("end"))));
        assert!(matches!(s.run("appointment.resize", &json!({ "id": id })), Err(CommandError::Param("end"))));
        s.run("appointment.save", &json!({ "title": "Weekstart", "start": 10_000, "end": 11_800, "recurrence": { "frequency": "weekly" } })).unwrap();
        let series = s.selected_appointment().unwrap().id.clone();
        assert!(matches!(s.run("appointment.resize", &json!({ "id": series, "end": 20_000 })), Err(CommandError::Param("id"))));
        assert!(matches!(s.run("appointment.resize", &json!({ "id": "nope", "end": 1 })), Err(CommandError::NotFound)));
    }

    #[test]
    fn appointments_are_saved_selected_and_deleted() {
        let mut s = Session::default();
        s.set_calendar_data(CalendarFile::default());
        assert_eq!(s.calendars, vec![Calendar::local()]);
        s.run("appointment.save", &json!({ "title": "Lunch", "start": 1000, "end": 4600, "location": "Kantine" })).unwrap();
        assert!(s.take_calendar_dirty());
        let a = s.selected_appointment().unwrap().clone();
        assert_eq!((a.title.as_str(), a.calendar_id.as_str(), a.location.as_str()), ("Lunch", "local", "Kantine"));
        assert_eq!(s.appointments_between(0, 1001).len(), 1);
        assert!(s.appointments_between(4600, 9000).is_empty());

        s.run("appointment.save", &json!({ "id": a.id, "title": "Lunch met Sanne", "start": 1000, "end": 4600, "reminder": 30 })).unwrap();
        assert_eq!(s.appointments.len(), 1);
        assert_eq!(s.appointment(&a.id).unwrap().title, "Lunch met Sanne");
        s.run("appointment.save", &json!({ "id": a.id, "title": "Lunch met Sanne", "start": 1000, "end": 4600 })).unwrap();
        assert_eq!(s.appointment(&a.id).unwrap().reminder, Some(30), "left out keeps the reminder");
        s.run("appointment.save", &json!({ "id": a.id, "title": "Lunch met Sanne", "start": 1000, "end": 4600, "reminder": null })).unwrap();
        assert_eq!(s.appointment(&a.id).unwrap().reminder, None);
        assert!(!s.appointment(&a.id).unwrap().private);
        s.run("appointment.save", &json!({ "id": a.id, "title": "Dokter", "start": 1000, "end": 4600, "private": true })).unwrap();
        assert!(s.appointment(&a.id).unwrap().private, "Privé is kept");
        s.run("appointment.save", &json!({ "id": a.id, "title": "Dokter", "start": 1000, "end": 4600 })).unwrap();
        assert!(!s.appointment(&a.id).unwrap().private, "left out means not private");
        assert_eq!(s.appointment(&a.id).unwrap().show_as, mailcraft_model::ShowAs::Busy, "busy unless said otherwise");
        s.run("appointment.save", &json!({ "id": a.id, "title": "Dokter", "start": 1000, "end": 4600, "show_as": "free" })).unwrap();
        assert_eq!(s.appointment(&a.id).unwrap().show_as, mailcraft_model::ShowAs::Free);
        s.run("appointment.save", &json!({ "id": a.id, "title": "Dokter", "start": 1000, "end": 4600 })).unwrap();
        assert_eq!(s.appointment(&a.id).unwrap().show_as, mailcraft_model::ShowAs::Free, "left out keeps the status");
        assert_eq!(
            s.run("appointment.save", &json!({ "title": "x", "start": 1, "end": 2, "show_as": "maybe" })),
            Err(CommandError::Param("show_as"))
        );
        assert_eq!(
            s.run("appointment.save", &json!({ "title": "x", "start": 1, "end": 2, "reminder": "soon" })),
            Err(CommandError::Param("reminder"))
        );

        assert_eq!(s.run("appointment.save", &json!({ "title": "", "start": 1, "end": 2 })), Err(CommandError::Param("title")));
        assert_eq!(s.run("appointment.save", &json!({ "title": "x", "start": 2, "end": 2 })), Err(CommandError::Param("end")));
        assert_eq!(s.run("appointment.save", &json!({ "id": "nope", "title": "x", "start": 1, "end": 2 })), Err(CommandError::NotFound));
        assert_eq!(s.run("appointment.select", &json!({ "id": "nope" })), Err(CommandError::NotFound));

        s.run("appointment.delete", &Value::Null).unwrap();
        assert!(s.appointments.is_empty() && s.selected_appointment.is_none());
        assert_eq!(s.run("appointment.delete", &Value::Null), Err(CommandError::NoSelection));
        let data = s.calendar_data();
        assert_eq!(data.calendars.len(), 1);
    }

    #[test]
    fn a_recurring_appointment_is_expanded_skipped_and_kept_off_account_calendars() {
        let mut s = Session::default();
        s.set_calendar_data(CalendarFile::default());
        let day = 86_400;
        let start = 1_792_404_000; // Monday 2026-10-19 10:00 UTC
        let weekly = json!({ "frequency": "weekly", "weekdays": [0, 3] });
        s.run("appointment.save", &json!({ "title": "Weekstart", "start": start, "end": start + 1800, "recurrence": weekly })).unwrap();
        let id = s.selected_appointment().unwrap().id.clone();
        assert_eq!(s.appointments.len(), 1, "a series is one entry");
        let shown = s.appointments_between(start - day, start + 14 * day);
        assert_eq!(shown.len(), 4);
        assert!(shown.iter().all(|a| a.id == id && a.recurrence.is_some() && a.end - a.start == 1800));
        assert_eq!(shown[1].start - shown[0].start, 3 * day);
        assert!(s.calendar_data().appointments[0].recurrence.is_some(), "the file keeps the rule, not the occurrences");

        s.run("appointment.skip", &json!({ "id": id, "start": shown[1].start })).unwrap();
        assert!(s.take_calendar_dirty());
        assert_eq!(s.appointments_between(start - day, start + 14 * day).len(), 3);
        // Saving the series again without a skipped list keeps the skip; naming one replaces it.
        s.run("appointment.save", &json!({ "id": id, "title": "Weekstart", "start": start, "end": start + 1800, "recurrence": weekly })).unwrap();
        assert_eq!(s.appointment(&id).unwrap().recurrence.as_ref().unwrap().skipped, vec![shown[1].start]);
        s.run("appointment.save", &json!({ "id": id, "title": "Weekstart", "start": start, "end": start + 1800, "recurrence": { "frequency": "weekly", "weekdays": [0, 3], "skipped": [] } })).unwrap();
        assert!(s.appointment(&id).unwrap().recurrence.as_ref().unwrap().skipped.is_empty());
        // Dropping the rule makes it a single appointment again.
        s.run("appointment.save", &json!({ "id": id, "title": "Weekstart", "start": start, "end": start + 1800, "recurrence": null })).unwrap();
        assert!(s.appointment(&id).unwrap().recurrence.is_none());
        assert_eq!(s.run("appointment.skip", &json!({ "id": id, "start": start })), Err(CommandError::Param("id")));

        assert_eq!(
            s.run("appointment.save", &json!({ "title": "Lang", "start": start, "end": start + 2 * day, "recurrence": { "frequency": "daily" } })),
            Err(CommandError::Param("recurrence"))
        );
        assert_eq!(
            s.run("appointment.save", &json!({ "title": "Raar", "start": start, "end": start + 1, "recurrence": { "frequency": "fortnightly" } })),
            Err(CommandError::Param("recurrence"))
        );
        s.calendars.push(Calendar { id: "acc/cal".into(), name: "Werk".into(), account_id: Some("acc".into()), color: [0; 3] });
        s.take_jobs();
        s.run(
            "appointment.save",
            &json!({ "calendar_id": "acc/cal", "title": "Op de server", "start": start, "end": start + 1, "recurrence": weekly }),
        )
        .expect("a new appointment in an account calendar may start a series");
        let jobs = s.take_jobs();
        assert!(
            matches!(&jobs[..], [crate::Job::SaveItem { item: mailcraft_mailbox::Item::Appointment(a), .. }] if a.recurrence.is_some()),
            "the server gets the rule: {jobs:?}"
        );
        let id = s.selected_appointment.clone().expect("selected");
        assert_eq!(
            s.run("appointment.save", &json!({ "id": id, "title": "Op de server", "start": start, "end": start + 1, "recurrence": weekly })),
            Err(CommandError::Param("recurrence")),
            "the server owns the series once it exists"
        );
    }
}
