//! Appointments, contacts and tasks that live in an account, mirrored into the session.
//!
//! An account whose store keeps items (Exchange does) says so when it connects; the engine
//! then asks for its appointments in a window around today, its contacts and its tasks, and
//! again on `sync.refresh`. What comes back replaces the account's items in the session; the
//! local files hold only local items. A change to an account's item is applied to the session
//! at once and sent to the store; a new item carries a pending id until the store names it.
//!
//! Session ids of account items are `<account id>/<store id>`, like folder ids, so a session
//! id always says where an item lives.

use std::collections::HashMap;

use chrono::{Days, Local, NaiveTime, TimeZone};
use serde_json::Value;

use merel_mailbox::Item;
use merel_model::rich::Rich;
use merel_model::{Account, Appointment, Calendar, Contact, Meeting, Task};

use crate::{CommandError, Job, Session};

/// Local formatting of synced notes by item id, with the text it lies over.
type Formatted = HashMap<String, (String, Rich)>;

/// The formatting kept for the notes of a synced item: the local one while the server sent the
/// same text back, none once the notes changed elsewhere.
fn kept_formatting(formatted: &Formatted, id: &str, notes: &str) -> Option<Rich> {
    formatted.get(id).filter(|(text, _)| text == notes).map(|(_, rich)| rich.clone())
}

/// How far back and ahead appointments are fetched, in days.
pub const PAST_DAYS: i64 = 90;
pub const FUTURE_DAYS: i64 = 400;

/// Colours for account calendars, in the order accounts were added.
const PALETTE: [[u8; 3]; 6] =
    [[0x2E, 0x8B, 0x57], [0xB0, 0x4A, 0x8A], [0xD9, 0x7A, 0x1E], [0x3C, 0x7D, 0xB5], [0xA3, 0x3A, 0x3A], [0x6B, 0x5B, 0xA8]];

const PENDING: &str = "pending-";

/// The nearest local midnight to `ts`: what a store that keeps a day as midnight in its own
/// time zone meant, as long as the two zones are within twelve hours of each other.
pub fn snap_to_midnight(ts: i64) -> i64 {
    let Some(local) = Local.timestamp_opt(ts, 0).single() else { return ts };
    let mut date = local.date_naive();
    if local.time() >= NaiveTime::from_hms_opt(12, 0, 0).unwrap_or_default() {
        date = date.checked_add_days(Days::new(1)).unwrap_or(date);
    }
    Local.from_local_datetime(&date.and_time(NaiveTime::MIN)).earliest().map(|d| d.timestamp()).unwrap_or(ts)
}

fn now() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0)
}

/// The name of an account's own calendar, as Outlook calls it; the side pane shows it under
/// the account's address.
pub const ACCOUNT_CALENDAR: &str = "Agenda";

impl Session {
    /// The accounts whose store keeps appointments, contacts and tasks, in account order.
    pub fn item_accounts(&self) -> Vec<&Account> {
        self.accounts.iter().filter(|a| self.item_accounts.contains(&a.id)).collect()
    }

    /// The session id of the calendar of `account_id`.
    pub fn account_calendar_id(account_id: &str) -> String {
        format!("{account_id}/calendar")
    }

    /// The session id of the item the store of `account_id` calls `remote_id`.
    pub fn local_item_id(account_id: &str, remote_id: &str) -> String {
        format!("{account_id}/{remote_id}")
    }

    /// The store's id behind session id `id`; `None` while the store has not named it yet.
    pub fn remote_item_id(account_id: &str, id: &str) -> Option<String> {
        let rest = id.strip_prefix(account_id)?.strip_prefix('/')?;
        (!rest.is_empty() && !rest.starts_with(PENDING)).then(|| rest.to_string())
    }

    /// A session id for an item that is on its way to the store of `account_id`.
    pub(crate) fn pending_id(&mut self, account_id: &str) -> String {
        self.next_pending_id += 1;
        format!("{account_id}/{PENDING}{}-{}", now(), self.next_pending_id)
    }

    /// Asks the store of `account_id` for its items; nothing happens while a request is out.
    pub fn request_sync(&mut self, account_id: &str) {
        let key = format!("sync/{account_id}");
        if !self.item_accounts.contains(account_id) || self.loading.contains(&key) {
            return;
        }
        self.loading.insert(key);
        let today = now();
        self.pending.push(Job::Sync { account_id: account_id.to_string(), from: today - PAST_DAYS * 86_400, to: today + FUTURE_DAYS * 86_400 });
    }

    /// Queues `item` (carrying its session id) for the store of `account_id`. A new item is
    /// created there; an existing one is replaced, unless the store has not named it yet.
    pub(crate) fn save_remote(&mut self, account_id: &str, is_new: bool, mut item: Item) -> Result<(), CommandError> {
        let local_id = item.id().to_string();
        let remote = Self::remote_item_id(account_id, &local_id);
        if !is_new && remote.is_none() {
            return Err(CommandError::Store(merel_i18n::tr("sync-still-saving")));
        }
        item.set_id(remote.unwrap_or_default());
        self.pending.push(Job::SaveItem { account_id: account_id.to_string(), local_id, item: Box::new(item) });
        Ok(())
    }

    /// Queues the deletion of the item behind session id `local_id` in its store.
    pub(crate) fn delete_remote(&mut self, account_id: &str, local_id: &str) {
        if let Some(id) = Self::remote_item_id(account_id, local_id) {
            self.pending.push(Job::DeleteItem { account_id: account_id.to_string(), id });
        }
    }

    /// How a calendar is named where it stands without its account above it, as in the
    /// appointment window's calendar list: "Agenda (alain@example.org)" for an account's.
    pub fn calendar_label(&self, c: &Calendar) -> String {
        match c.account_id.as_deref().and_then(|id| self.accounts.iter().find(|a| a.id == id)) {
            Some(a) => format!("{} ({})", c.name, a.address.email()),
            None => c.name.clone(),
        }
    }

    /// The account an appointment's calendar belongs to, if any.
    pub fn appointment_account(&self, calendar_id: &str) -> Option<String> {
        self.calendar(calendar_id).and_then(|c| c.account_id.clone())
    }

    fn ensure_calendar(&mut self, account_id: &str) -> String {
        let id = Self::account_calendar_id(account_id);
        match self.calendars.iter_mut().find(|c| c.id == id) {
            // Earlier versions named it after the account, which reads as a person.
            Some(c) => c.name = ACCOUNT_CALENDAR.into(),
            None => {
                let index = self.accounts.iter().position(|a| a.id == account_id).unwrap_or(0);
                let color = self.calendar_colors.get(&id).copied().unwrap_or(PALETTE[index % PALETTE.len()]);
                self.calendars.push(Calendar { id: id.clone(), name: ACCOUNT_CALENDAR.into(), account_id: Some(account_id.to_string()), color });
            }
        }
        id
    }

    /// Replaces the items of `account_id` with what its store reported.
    pub(crate) fn apply_synced(&mut self, account_id: &str, appointments: Vec<Appointment>, contacts: Vec<Contact>, tasks: Vec<Task>) {
        self.loading.remove(&format!("sync/{account_id}"));
        if !self.accounts.iter().any(|a| a.id == account_id) {
            return;
        }
        self.item_accounts.insert(account_id.to_string());
        let calendar_id = self.ensure_calendar(account_id);
        // Servers keep notes as text; the formatting stays on this computer for as long as the
        // text it lies over comes back unchanged.
        let formatted: Formatted = self
            .appointments
            .iter()
            .filter(|a| a.calendar_id == calendar_id)
            .filter_map(|a| a.notes_rich.clone().map(|r| (a.id.clone(), (a.notes.clone(), r))))
            .collect();
        // Meetings Merel sent keep their attendees and answers here when the server does
        // not track them (Exchange, Microsoft 365, Google); what a server does track wins, with
        // the details of the answers filed here.
        let meetings: HashMap<String, Meeting> =
            self.appointments.iter().filter(|a| a.calendar_id == calendar_id).filter_map(|a| a.meeting.clone().map(|m| (a.id.clone(), m))).collect();
        let formatted_contacts: Formatted = self
            .contacts
            .iter()
            .filter(|c| c.account_id.as_deref() == Some(account_id))
            .filter_map(|c| c.notes_rich.clone().map(|r| (c.id.clone(), (c.notes.clone(), r))))
            .collect();
        let formatted_tasks: Formatted = self
            .tasks
            .iter()
            .filter(|t| t.account_id.as_deref() == Some(account_id))
            .filter_map(|t| t.notes_rich.clone().map(|r| (t.id.clone(), (t.notes.clone(), r))))
            .collect();
        self.appointments.retain(|a| a.calendar_id != calendar_id);
        self.appointments.extend(appointments.into_iter().map(|mut a| {
            a.id = Self::local_item_id(account_id, &a.id);
            a.calendar_id = calendar_id.clone();
            a.notes_rich = kept_formatting(&formatted, &a.id, &a.notes);
            if let Some(ours) = meetings.get(&a.id) {
                match &mut a.meeting {
                    None => a.meeting = Some(ours.clone()),
                    Some(theirs) => {
                        for t in &mut theirs.attendees {
                            if let Some(o) = ours.attendees.iter().find(|o| o.email.eq_ignore_ascii_case(&t.email)) {
                                t.replied = o.replied;
                                t.proposed = o.proposed.filter(|_| t.response == o.response);
                            }
                        }
                    }
                }
            }
            if a.all_day {
                a.start = snap_to_midnight(a.start);
                a.end = snap_to_midnight(a.end).max(a.start + 86_400);
            }
            a
        }));
        self.contacts.retain(|c| c.account_id.as_deref() != Some(account_id));
        self.contacts.extend(contacts.into_iter().map(|mut c| {
            c.id = Self::local_item_id(account_id, &c.id);
            c.account_id = Some(account_id.to_string());
            c.notes_rich = kept_formatting(&formatted_contacts, &c.id, &c.notes);
            c
        }));
        self.tasks.retain(|t| t.account_id.as_deref() != Some(account_id));
        self.tasks.extend(tasks.into_iter().map(|mut t| {
            t.id = Self::local_item_id(account_id, &t.id);
            t.account_id = Some(account_id.to_string());
            t.due = t.due.map(snap_to_midnight);
            t.notes_rich = kept_formatting(&formatted_tasks, &t.id, &t.notes);
            t
        }));
        if self.selected_appointment().is_none() {
            self.selected_appointment = None;
        }
        if self.selected_contact().is_none() {
            self.selected_contact = None;
        }
        if self.selected_task().is_none() {
            self.selected_task = None;
        }
    }

    /// The store of `account_id` named the item the session knew as `local_id`.
    pub(crate) fn apply_saved(&mut self, account_id: &str, local_id: &str, remote_id: &str) {
        let id = Self::local_item_id(account_id, remote_id);
        if let Some(a) = self.appointments.iter_mut().find(|a| a.id == local_id) {
            a.id = id.clone();
        }
        if let Some(c) = self.contacts.iter_mut().find(|c| c.id == local_id) {
            c.id = id.clone();
        }
        if let Some(t) = self.tasks.iter_mut().find(|t| t.id == local_id) {
            t.id = id.clone();
        }
        for selected in [&mut self.selected_appointment, &mut self.selected_contact, &mut self.selected_task] {
            if selected.as_deref() == Some(local_id) {
                *selected = Some(id.clone());
            }
        }
    }
}

/// `sync.refresh`: fetches the items of every account that keeps them.
pub fn refresh(s: &mut Session, _: &Value) -> Result<(), CommandError> {
    let ids: Vec<String> = s.item_accounts.iter().cloned().collect();
    for id in ids {
        s.request_sync(&id);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use merel_model::{Address, Auth, Priority, Protocol, Server, Tls};

    use super::*;
    use crate::Event;

    fn account(id: &str, name: &str) -> Account {
        Account {
            id: id.into(),
            name: name.into(),
            address: Address::new(None, "a@example.org").unwrap(),
            username: "a".into(),
            protocol: Protocol::Exchange,
            auth: Auth::Password,
            incoming: Some(Server { host: "mail.example.org".into(), port: 443, tls: Tls::Implicit }),
            outgoing: None,
            dav: None,
            signature: String::new(),
        }
    }

    fn connected(s: &mut Session) {
        s.apply(Event::Connected { account_id: "acc".into(), folders: Vec::new(), syncs_items: true });
    }

    fn appointment(id: &str, start: i64, end: i64, all_day: bool) -> Appointment {
        Appointment {
            id: id.into(),
            calendar_id: String::new(),
            title: "Overleg".into(),
            start,
            end,
            all_day,
            location: String::new(),
            notes: String::new(),
            notes_rich: None,
            recurrence: None,
            reminder: None,
            private: false,
            show_as: merel_model::ShowAs::Busy,
            categories: Vec::new(),
            meeting: None,
        }
    }

    #[test]
    fn an_account_calendar_named_after_the_account_is_renamed_agenda() {
        let mut s = Session::with_accounts(vec![account("acc", "Alain Rees")]);
        s.calendars.push(Calendar { id: "acc/calendar".into(), name: "Alain Rees".into(), account_id: Some("acc".into()), color: [1, 2, 3] });
        connected(&mut s);
        s.apply(Event::Synced { account_id: "acc".into(), appointments: Vec::new(), contacts: Vec::new(), tasks: Vec::new() });
        let cal = s.calendar("acc/calendar").unwrap();
        assert_eq!((cal.name.as_str(), cal.color), (ACCOUNT_CALENDAR, [1, 2, 3]), "renamed, colour kept");
        assert_eq!(s.calendar_label(&Calendar::local()), "Mijn agenda", "a local calendar keeps its own name");
    }

    #[test]
    fn a_connected_store_is_asked_for_its_items_and_mirrored() {
        let mut s = Session::with_accounts(vec![account("acc", "Werk")]);
        s.take_jobs();
        connected(&mut s);
        assert_eq!(s.item_accounts().len(), 1);
        let jobs = s.take_jobs();
        assert!(
            matches!(jobs.as_slice(), [Job::Sync { account_id, from, to }] if account_id == "acc" && to - from == (PAST_DAYS + FUTURE_DAYS) * 86_400)
        );
        s.run("sync.refresh", &Value::Null).unwrap();
        assert!(s.take_jobs().is_empty(), "no second request while one is out");

        let midnight = snap_to_midnight(1_792_000_000);
        let contact = Contact { id: "C1".into(), name: "Sanne".into(), emails: vec!["s@example.org".into()], ..Contact::default() };
        let task = Task { id: "T1".into(), title: "Offerte".into(), due: Some(midnight - 7200), priority: Priority::High, ..Task::default() };
        s.apply(Event::Synced {
            account_id: "acc".into(),
            appointments: vec![appointment("A1", 1_792_000_000, 1_792_003_600, false), appointment("A2", midnight - 7200, midnight - 7200, true)],
            contacts: vec![contact],
            tasks: vec![task],
        });
        let cal = s.calendar("acc/calendar").unwrap();
        assert_eq!((cal.name.as_str(), cal.account_id.as_deref()), (ACCOUNT_CALENDAR, Some("acc")));
        assert_eq!(s.calendar_label(cal), "Agenda (a@example.org)", "away from the side pane the address says whose it is");
        let a2 = s.appointment("acc/A2").unwrap();
        assert_eq!((a2.start, a2.end, a2.calendar_id.as_str()), (midnight, midnight + 86_400, "acc/calendar"));
        assert!(s.appointment("acc/A1").is_some());
        assert_eq!(s.contact("acc/C1").unwrap().account_id.as_deref(), Some("acc"));
        assert_eq!(s.task("acc/T1").unwrap().due, Some(midnight));
        assert!(!s.has_pending());

        // Only local items go to the files.
        assert!(s.calendar_data().appointments.is_empty() && s.calendar_data().calendars.iter().all(|c| c.account_id.is_none()));
        assert!(s.contacts_data().contacts.is_empty() && s.tasks_data().tasks.is_empty());

        // A second sync replaces the account's items and keeps a selection that survives.
        s.run("contact.select", &json!({ "id": "acc/C1" })).unwrap();
        s.run("sync.refresh", &Value::Null).unwrap();
        s.apply(Event::Synced { account_id: "acc".into(), appointments: Vec::new(), contacts: Vec::new(), tasks: Vec::new() });
        assert!(s.appointments.is_empty() && s.contacts.is_empty() && s.selected_contact.is_none());

        // A colour picked for the account's calendar is saved locally and survives the
        // calendar being made again, after a restart or when the account signs in anew.
        s.run("calendar.color", &json!({ "id": "acc/calendar", "color": [1, 2, 3] })).unwrap();
        let saved = s.calendar_data();
        assert_eq!(saved.colors.get("acc/calendar"), Some(&[1, 2, 3]));
        s.calendars.retain(|c| c.account_id.is_none());
        s.set_calendar_data(saved);
        s.apply(Event::Synced { account_id: "acc".into(), appointments: Vec::new(), contacts: Vec::new(), tasks: Vec::new() });
        assert_eq!(s.calendar("acc/calendar").unwrap().color, [1, 2, 3]);
        s.remove_account("acc");
        assert!(s.calendar_colors.is_empty(), "the colour goes with the account");
    }

    #[test]
    fn formatted_notes_survive_a_sync_while_their_text_is_unchanged() {
        let mut s = Session::with_accounts(vec![account("acc", "Werk")]);
        connected(&mut s);
        let mut a = appointment("A1", 1_792_000_000, 1_792_003_600, false);
        a.notes = "Agenda".into();
        s.apply(Event::Synced { account_id: "acc".into(), appointments: vec![a.clone()], contacts: Vec::new(), tasks: Vec::new() });
        let mut rich = Rich::plain("Agenda");
        rich.apply(0..6, |st| st.bold = true);
        if let Some(local) = s.appointments.iter_mut().find(|x| x.id == "acc/A1") {
            local.notes_rich = Some(rich.clone());
        }
        s.apply(Event::Synced { account_id: "acc".into(), appointments: vec![a.clone()], contacts: Vec::new(), tasks: Vec::new() });
        assert_eq!(s.appointment("acc/A1").and_then(|x| x.notes_rich.clone()), Some(rich.clone()), "same text, same formatting");
        a.notes = "Agenda, gewijzigd op de server".into();
        s.apply(Event::Synced { account_id: "acc".into(), appointments: vec![a], contacts: Vec::new(), tasks: Vec::new() });
        assert_eq!(s.appointment("acc/A1").and_then(|x| x.notes_rich.clone()), None, "changed elsewhere: the text wins");

        // Contacts and tasks the same way.
        let contact = Contact { id: "C1".into(), name: "Noor".into(), notes: "Agenda".into(), ..Contact::default() };
        let task = Task { id: "T1".into(), title: "Bellen".into(), notes: "Agenda".into(), ..Task::default() };
        let synced = |s: &mut Session| {
            s.apply(Event::Synced { account_id: "acc".into(), appointments: Vec::new(), contacts: vec![contact.clone()], tasks: vec![task.clone()] })
        };
        synced(&mut s);
        for c in s.contacts.iter_mut().filter(|c| c.id == "acc/C1") {
            c.notes_rich = Some(rich.clone());
        }
        for t in s.tasks.iter_mut().filter(|t| t.id == "acc/T1") {
            t.notes_rich = Some(rich.clone());
        }
        synced(&mut s);
        assert!(s.contacts.iter().any(|c| c.id == "acc/C1" && c.notes_rich.as_ref() == Some(&rich)));
        assert!(s.tasks.iter().any(|t| t.id == "acc/T1" && t.notes_rich.as_ref() == Some(&rich)));
    }

    #[test]
    fn a_meeting_keeps_its_people_when_the_server_does_not_track_them() {
        let mut s = Session::with_accounts(vec![account("acc", "Werk")]);
        connected(&mut s);
        let a = appointment("A1", 1_792_000_000, 1_792_003_600, false);
        s.apply(Event::Synced { account_id: "acc".into(), appointments: vec![a.clone()], contacts: Vec::new(), tasks: Vec::new() });
        let sanne = merel_model::Attendee {
            email: "sanne@example.org".into(),
            response: merel_model::Response::Accepted,
            replied: Some(5),
            ..Default::default()
        };
        let ours = Meeting { uid: "m1".into(), organizer: "a@example.org".into(), attendees: vec![sanne.clone()], ..Meeting::default() };
        if let Some(local) = s.appointments.iter_mut().find(|x| x.id == "acc/A1") {
            local.meeting = Some(ours.clone());
        }
        s.apply(Event::Synced { account_id: "acc".into(), appointments: vec![a.clone()], contacts: Vec::new(), tasks: Vec::new() });
        assert_eq!(s.appointment("acc/A1").and_then(|x| x.meeting.clone()), Some(ours.clone()), "the server had none, ours stays");
        // A server that tracks the answers wins, with the time of our answer kept.
        let theirs = Meeting { attendees: vec![merel_model::Attendee { replied: None, ..sanne }], ..ours };
        let tracked = Appointment { meeting: Some(theirs), ..a };
        s.apply(Event::Synced { account_id: "acc".into(), appointments: vec![tracked], contacts: Vec::new(), tasks: Vec::new() });
        let kept = s.appointment("acc/A1").and_then(|x| x.meeting.clone()).expect("a meeting");
        assert_eq!(kept.attendees[0].replied, Some(5));
    }

    #[test]
    fn changes_to_account_items_go_to_the_store() {
        let mut s = Session::with_accounts(vec![account("acc", "Werk")]);
        connected(&mut s);
        s.apply(Event::Synced { account_id: "acc".into(), appointments: Vec::new(), contacts: Vec::new(), tasks: Vec::new() });
        s.take_jobs();

        s.run("appointment.save", &json!({ "calendar_id": "acc/calendar", "title": "Lunch", "start": 1000, "end": 4600 })).unwrap();
        assert!(!s.take_calendar_dirty());
        let pending = s.selected_appointment().unwrap().id.clone();
        assert!(pending.starts_with("acc/pending-"));
        let jobs = s.take_jobs();
        assert!(
            matches!(&jobs[..], [Job::SaveItem { account_id, local_id, item }] if account_id == "acc" && *local_id == pending && matches!(&**item, Item::Appointment(a) if a.id.is_empty()))
        );
        // Editing before the store has named it is refused rather than duplicated.
        assert!(matches!(
            s.run("appointment.save", &json!({ "id": pending, "title": "Lunch", "start": 1000, "end": 4600 })),
            Err(CommandError::Store(_))
        ));

        s.apply(Event::ItemSaved { account_id: "acc".into(), local_id: pending.clone(), remote_id: "A9".into() });
        assert!(s.appointment("acc/A9").is_some() && s.appointment(&pending).is_none());
        assert_eq!(s.selected_appointment.as_deref(), Some("acc/A9"));
        s.run("appointment.save", &json!({ "id": "acc/A9", "title": "Lunch", "start": 1000, "end": 4600, "location": "Kantine" })).unwrap();
        assert!(
            matches!(&s.take_jobs()[..], [Job::SaveItem { item, .. }] if matches!(&**item, Item::Appointment(a) if a.id == "A9" && a.location == "Kantine"))
        );
        s.run("appointment.delete", &Value::Null).unwrap();
        assert!(matches!(&s.take_jobs()[..], [Job::DeleteItem { account_id, id }] if account_id == "acc" && id == "A9"));
        assert!(s.appointments.is_empty() && !s.take_calendar_dirty());

        s.run("contact.save", &json!({ "name": "Noor", "emails": ["noor@example.org"], "account_id": "acc" })).unwrap();
        let c = s.selected_contact().unwrap().clone();
        assert_eq!(c.account_id.as_deref(), Some("acc"));
        assert!(!s.take_contacts_dirty());
        assert!(
            matches!(&s.take_jobs()[..], [Job::SaveItem { item, .. }] if matches!(&**item, Item::Contact(c) if c.id.is_empty() && c.name == "Noor"))
        );
        assert_eq!(s.run("contact.save", &json!({ "name": "X", "account_id": "nope" })), Err(CommandError::Param("account_id")));
        s.apply(Event::ItemSaved { account_id: "acc".into(), local_id: c.id, remote_id: "C5".into() });
        s.run("contact.delete", &json!({ "id": "acc/C5" })).unwrap();
        assert!(matches!(&s.take_jobs()[..], [Job::DeleteItem { id, .. }] if id == "C5"));

        s.run("task.save", &json!({ "title": "Bellen", "account_id": "acc" })).unwrap();
        let t = s.selected_task().unwrap().clone();
        s.apply(Event::ItemSaved { account_id: "acc".into(), local_id: t.id, remote_id: "T5".into() });
        s.take_jobs();
        s.run("task.complete", &Value::Null).unwrap();
        assert!(matches!(&s.take_jobs()[..], [Job::SaveItem { item, .. }] if matches!(&**item, Item::Task(t) if t.id == "T5" && t.done)));
        assert!(!s.take_tasks_dirty());

        // Local items still go to the files, not to a store.
        s.run("task.save", &json!({ "title": "Thuis" })).unwrap();
        assert!(s.take_tasks_dirty() && s.take_jobs().is_empty());
        assert_eq!(s.tasks_data().tasks.len(), 1);
    }

    #[test]
    fn an_account_that_signs_in_again_is_replaced_not_doubled() {
        let mut s = Session::with_accounts(vec![account("acc", "Werk")]);
        s.take_jobs();
        let mut renamed = account("acc", "Werk (nieuw)");
        renamed.auth = Auth::OAuth2 { client_id: "c".into(), auth_url: "a".into(), token_url: "t".into(), scope: "wide".into() };
        s.add_account(renamed);
        assert_eq!(s.accounts.len(), 1);
        assert_eq!(s.accounts[0].name, "Werk (nieuw)");
        assert_eq!(s.take_jobs(), vec![Job::Connect { account_id: "acc".into() }]);
        assert_eq!(s.status.get("acc"), Some(&crate::AccountStatus::Connecting));
    }

    #[test]
    fn ids_say_where_an_item_lives() {
        assert_eq!(Session::remote_item_id("acc", "acc/AAMk=/x+"), Some("AAMk=/x+".to_string()));
        assert_eq!(Session::remote_item_id("acc", "acc/pending-1-2"), None);
        assert_eq!(Session::remote_item_id("acc", "other/A"), None);
        assert_eq!(Session::remote_item_id("acc", "acc/"), None);
        let noon = snap_to_midnight(1_792_000_000);
        assert_eq!(snap_to_midnight(noon + 3600), noon);
        assert_eq!(snap_to_midnight(noon - 3600), noon);
        assert_eq!(snap_to_midnight(noon + 13 * 3600), noon + 86_400);
    }
}
