//! Calendars and appointments in the session, and the commands that change them.
//!
//! Appointments live in `calendar.json` next to the accounts file until they sync with a
//! server. Every change goes through a command, which marks the session dirty; the engine
//! saves right after the command returns.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use mailcraft_model::{Appointment, AppointmentError, Calendar};

use crate::{CommandError, Session};

/// What `calendar.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarFile {
    pub calendars: Vec<Calendar>,
    pub appointments: Vec<Appointment>,
}

impl Session {
    /// Replaces the calendars and appointments; there is always a local calendar.
    pub fn set_calendar_data(&mut self, data: CalendarFile) {
        self.calendars = data.calendars;
        self.appointments = data.appointments;
        if self.calendars.is_empty() {
            self.calendars.push(Calendar::local());
        }
        self.selected_appointment = None;
    }

    pub fn calendar_data(&self) -> CalendarFile {
        CalendarFile { calendars: self.calendars.clone(), appointments: self.appointments.clone() }
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

    /// The appointments touching `[from, to)`, earliest first.
    pub fn appointments_between(&self, from: i64, to: i64) -> Vec<&Appointment> {
        let mut list: Vec<&Appointment> = self.appointments.iter().filter(|a| a.overlaps(from, to)).collect();
        list.sort_by_key(|a| (a.start, a.end));
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

/// `appointment.save { id?, calendar_id?, title, start, end, all_day?, location?, notes? }`:
/// creates the appointment, or replaces the one with `id`. Times are Unix seconds.
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
    let id = existing.as_ref().map(|a| a.id.clone()).unwrap_or_else(|| s.new_appointment_id());
    let appointment = Appointment {
        id: id.clone(),
        calendar_id,
        title: str_param(params, "title").unwrap_or("").to_string(),
        start: i64_param(params, "start")?,
        end: i64_param(params, "end")?,
        all_day: params.get("all_day").and_then(Value::as_bool).unwrap_or(false),
        location: str_param(params, "location").unwrap_or("").to_string(),
        notes: str_param(params, "notes").unwrap_or("").to_string(),
    }
    .validated()
    .map_err(|e| match e {
        AppointmentError::NoTitle => CommandError::Param("title"),
        AppointmentError::EndsBeforeStart => CommandError::Param("end"),
    })?;
    s.appointments.retain(|a| a.id != id);
    s.appointments.push(appointment);
    s.selected_appointment = Some(id);
    s.calendar_dirty = true;
    Ok(())
}

/// `appointment.delete { id? }`: the given appointment, else the selected one.
pub fn delete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id").map(str::to_string).or_else(|| s.selected_appointment.clone()).ok_or(CommandError::NoSelection)?;
    let before = s.appointments.len();
    s.appointments.retain(|a| a.id != id);
    if s.appointments.len() == before {
        return Err(CommandError::NotFound);
    }
    if s.selected_appointment.as_deref() == Some(id.as_str()) {
        s.selected_appointment = None;
    }
    s.calendar_dirty = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

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

        s.run("appointment.save", &json!({ "id": a.id, "title": "Lunch met Sanne", "start": 1000, "end": 4600 })).unwrap();
        assert_eq!(s.appointments.len(), 1);
        assert_eq!(s.appointment(&a.id).unwrap().title, "Lunch met Sanne");

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
}
