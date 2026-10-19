//! The right-click menu on a message row: the everyday actions of the "Start" tab, and a task
//! or an appointment made from the message, with the subject as title and the sender, the date
//! and the text as notes, the way Outlook turns a mail into a to-do or a meeting.

use chrono::Timelike;
use egui::Response;
use mailcraft_engine::Session;
use mailcraft_model::MessageHeader;
use serde_json::{Value, json};

use crate::{ComposeKind, MailApp, Module, calendar, dates, tasks};

/// What the menu asked for; it runs once the menu has closed, on the clicked message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Compose(ComposeKind),
    Command(&'static str),
    Move,
    Task,
    Appointment,
    /// The sender's card, or the form for a new contact with the sender's name and address.
    Sender,
}

/// How much of a message's text goes into the notes of a task or an appointment.
const NOTES_CHARS: usize = 2000;

/// Draws the menu on `r` for message `m`; what was clicked comes back once, to be run by
/// [`MailApp::act_on_message`].
pub fn menu(r: &Response, m: &MessageHeader, known_sender: bool) -> Option<Pick> {
    let mut pick = None;
    r.context_menu(|ui| {
        let read = if m.flags.seen { ("Als ongelezen markeren", "message.mark_unread") } else { ("Als gelezen markeren", "message.mark_read") };
        let flag = if m.flags.flagged { "Vlag weghalen" } else { "Vlag" };
        let sender = if known_sender { "Afzender openen in contacten" } else { "Afzender toevoegen aan contacten" };
        let items: [(&str, Pick); 12] = [
            ("Beantwoorden", Pick::Compose(ComposeKind::Reply)),
            ("Allen beantwoorden", Pick::Compose(ComposeKind::ReplyAll)),
            ("Doorsturen", Pick::Compose(ComposeKind::Forward)),
            (read.0, Pick::Command(read.1)),
            (flag, Pick::Command("message.flag")),
            ("Verplaatsen naar…", Pick::Move),
            ("Archiveren", Pick::Command("message.archive")),
            ("Ongewenst", Pick::Command("message.junk")),
            ("Verwijderen", Pick::Command("message.delete")),
            ("Taak maken van dit bericht", Pick::Task),
            ("Afspraak maken van dit bericht", Pick::Appointment),
            (sender, Pick::Sender),
        ];
        for (i, (label, p)) in items.into_iter().enumerate() {
            if matches!(i, 3 | 9) {
                ui.separator();
            }
            if ui.button(label).clicked() {
                pick = Some(p);
                ui.close();
            }
        }
    });
    pick
}

/// The notes a task or an appointment made from `m` starts with: the sender, the date and the
/// message text (the preview when the text has not been fetched), cut to [`NOTES_CHARS`].
pub fn notes_from(session: &Session, m: &MessageHeader) -> String {
    let (name, email) = (m.from.display(), m.from.email());
    let from = if name == email { email.to_string() } else { format!("{name} <{email}>") };
    let text = session.bodies.get(&m.id).map(|b| b.text.as_str()).unwrap_or(m.preview.as_str());
    let text: String = text.trim().chars().take(NOTES_CHARS).collect();
    format!("Van: {from}\nDatum: {}\n\n{text}", dates::long(m.date))
}

impl MailApp {
    /// Runs what the menu picked on `m`, selecting it first so the commands and the compose
    /// window work on the message that was clicked.
    pub fn act_on_message(&mut self, m: &MessageHeader, pick: Pick) {
        if self.engine.session.selected_message.as_deref() != Some(m.id.as_str()) {
            self.run("message.select", json!({ "id": m.id }));
        }
        match pick {
            Pick::Compose(kind) => self.open_compose(kind),
            Pick::Command(id) => self.run(id, Value::Null),
            Pick::Move => self.move_to = true,
            Pick::Task => self.task_from_message(m),
            Pick::Appointment => self.appointment_from_message(m),
            Pick::Sender => self.open_or_add_sender(&m.from),
        }
    }

    /// Opens the tasks module with a new task named after the message, its notes filled in.
    pub fn task_from_message(&mut self, m: &MessageHeader) {
        let notes = notes_from(&self.engine.session, m);
        self.module = Module::Tasks;
        self.tasks.editor = Some(tasks::Editor { title: m.subject.clone(), notes, ..tasks::Editor::default() });
    }

    /// Opens the appointment window for a new appointment on the next full hour, named after
    /// the message, its notes filled in.
    pub fn appointment_from_message(&mut self, m: &MessageHeader) {
        let now = chrono::Local::now().naive_local();
        let start = now.date().and_hms_opt((now.hour() + 1).min(23), 0, 0).unwrap_or(now);
        let mut editor = calendar::Editor::new_at(start);
        editor.title = m.subject.clone();
        editor.notes = notes_from(&self.engine.session, m);
        self.calendar.editor = Some(editor);
    }
}

#[cfg(test)]
mod tests {
    use mailcraft_engine::Engine;

    use super::*;

    #[test]
    fn the_notes_name_the_sender_and_the_date_and_fall_back_to_the_preview() {
        let engine = Engine::sample();
        let m = engine.session.message("m1").expect("sample mail").clone();
        let notes = notes_from(&engine.session, &m);
        assert!(notes.starts_with("Van: Sanne de Vries <sanne@example.org>\nDatum: "), "{notes}");
        assert!(notes.ends_with(&format!("\n\n{}", m.preview)), "the preview stands in for the text: {notes}");
    }

    #[test]
    fn a_task_and_an_appointment_take_the_subject_and_the_notes() {
        let mut app = MailApp::new(Engine::sample());
        let m = app.engine.session.message("m1").expect("sample mail").clone();
        app.act_on_message(&m, Pick::Task);
        assert_eq!(app.module, Module::Tasks);
        let e = app.tasks.editor.as_ref().expect("task form");
        assert_eq!((e.id.as_deref(), e.title.as_str()), (None, "Planning voor donderdag"));
        assert!(e.notes.contains(&m.preview));
        assert_eq!(app.engine.session.selected_message.as_deref(), Some("m1"), "the message was selected first");
        app.act_on_message(&m, Pick::Appointment);
        let e = app.calendar.editor.as_ref().expect("appointment window");
        assert_eq!((e.id.as_deref(), e.title.as_str(), e.all_day), (None, "Planning voor donderdag", false));
        assert!(e.notes.starts_with("Van: "));
    }
}
