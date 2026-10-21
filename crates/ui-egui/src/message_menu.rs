//! The right-click menu on a message row: the everyday actions of the "Start" tab, and a task
//! or an appointment made from the message, with the subject as title and the sender, the date
//! and the text as notes, the way Outlook turns a mail into a to-do or a meeting. "Beantwoorden
//! met vergaderverzoek" goes one step further, as in Outlook: the appointment window opens with
//! the sender and the other recipients as attendees, ready to send.

use crate::i18n::tr;
use chrono::Timelike;
use egui::Response;
use merel_engine::Session;
use merel_model::{Address, MessageHeader, mailbox};
use serde_json::{Value, json};

use crate::{ComposeKind, MailApp, Module, calendar, dates, tasks};

/// What the menu asked for; it runs once the menu has closed, on the clicked message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Pick {
    Compose(ComposeKind),
    /// A new message with this one attached as an `.eml` file.
    ForwardAttached,
    /// The message as a new one, with the same recipients, subject, text and attachments.
    Resend,
    Command(&'static str),
    /// A follow-up flag for the day named as `message.follow_up` takes it.
    FollowUp(&'static str),
    /// Out of the list until the moment named as `message.snooze` takes it.
    Snooze(&'static str),
    Move,
    Task,
    Appointment,
    /// The appointment window with the sender and the other recipients as attendees.
    Meeting,
    /// The sender's card, or the form for a new contact with the sender's name and address.
    Sender,
    /// The rules window with a new rule for the sender's address filled in.
    Rule,
    /// The search box set to the sender's address, over every folder.
    FromSender,
    /// The question before cleaning up the message's conversation.
    Cleanup,
}

/// How much of a message's text goes into the notes of a task or an appointment.
const NOTES_CHARS: usize = 2000;

/// Outlook's "Beantwoorden met vergaderverzoek".
pub fn meeting() -> String {
    tr("menu-reply-meeting")
}

/// The sender and the other recipients of `m`, without `me`, as the "Genodigden" line.
pub fn attendee_line(m: &MessageHeader, me: &str) -> String {
    let mut list: Vec<&Address> = Vec::new();
    for a in std::iter::once(&m.from).chain(m.to.iter()) {
        if !a.email().eq_ignore_ascii_case(me) && !list.iter().any(|x| x.email().eq_ignore_ascii_case(a.email())) {
            list.push(a);
        }
    }
    list.into_iter().map(mailbox).collect::<Vec<_>>().join("; ")
}

/// Outlook's "Sluimeren", the submenu's name.
pub fn snooze() -> String {
    tr("menu-snooze")
}
pub fn unsnooze() -> String {
    tr("menu-unsnooze")
}

/// Draws the menu on `r` for message `m`; what was clicked comes back once, to be run by
/// [`MailApp::act_on_message`]. `known_sender` says whether the sender is in the contacts,
/// `in_junk` whether the message sits in the Junk folder, `ignored` whether its conversation
/// is ignored, `snoozed` whether it is put off with "Sluimeren".
pub fn menu(r: &Response, m: &MessageHeader, known_sender: bool, in_junk: bool, ignored: bool, snoozed: bool) -> Option<Pick> {
    let mut pick = None;
    r.context_menu(|ui| {
        let read = if m.flags.seen { ("menu-mark-unread", "message.mark_unread") } else { ("view-mark-read", "message.mark_read") };
        let sender = if known_sender { "menu-sender-open" } else { "menu-sender-add" };
        let junk = if in_junk { ("junk-not-junk", "junk.trust") } else { ("block-sender", "junk.block") };
        let ignore = if ignored { ("menu-unignore", "conversation.unignore") } else { ("menu-ignore", "conversation.ignore") };
        let items: [(String, Pick); 19] = [
            (tr("mail-reply"), Pick::Compose(ComposeKind::Reply)),
            (tr("mail-reply-all"), Pick::Compose(ComposeKind::ReplyAll)),
            (tr("mail-forward"), Pick::Compose(ComposeKind::Forward)),
            (crate::eml::forward(), Pick::ForwardAttached),
            (crate::resend::resend(), Pick::Resend),
            (meeting(), Pick::Meeting),
            (tr(read.0), Pick::Command(read.1)),
            (tr("menu-move-to"), Pick::Move),
            (tr("mail-archive"), Pick::Command("message.archive")),
            (tr("mail-junk"), Pick::Command("message.junk")),
            (tr(junk.0), Pick::Command(junk.1)),
            (tr("delete"), Pick::Command("message.delete")),
            (tr(ignore.0), Pick::Command(ignore.1)),
            (tr("menu-clean-up-conversation"), Pick::Cleanup),
            (tr("menu-make-task"), Pick::Task),
            (tr("menu-make-appointment"), Pick::Appointment),
            (tr(sender), Pick::Sender),
            (tr("menu-find-from-sender"), Pick::FromSender),
            (tr("menu-rule-for-sender"), Pick::Rule),
        ];
        for (i, (label, p)) in items.into_iter().enumerate() {
            if matches!(i, 6 | 14 | 17) {
                ui.separator();
            }
            if i == 7 {
                // Outlook's "Opvolgen": the flag with a due day, under its own arrow.
                ui.menu_button(tr("menu-follow-up"), |ui| {
                    for (when, _) in merel_engine::followup::WHEN {
                        if ui.button(tr(&format!("followup-when-{when}"))).clicked() {
                            pick = Some(Pick::FollowUp(when));
                        }
                    }
                    if ui.button(tr("follow-up-no-date")).clicked() {
                        pick = Some(Pick::FollowUp("none"));
                    }
                    if m.flags.flagged {
                        ui.separator();
                        if ui.button(tr("follow-up-clear")).clicked() {
                            pick = Some(Pick::Command("message.flag"));
                        }
                    }
                    if pick.is_some() {
                        egui::Popup::close_all(ui.ctx());
                    }
                });
                // Outlook's "Sluimeren": out of the list until a moment of choice.
                ui.menu_button(snooze(), |ui| {
                    for (when, _) in merel_engine::snooze::WHEN {
                        if ui.button(tr(&format!("snooze-{when}"))).clicked() {
                            pick = Some(Pick::Snooze(when));
                        }
                    }
                    if snoozed {
                        ui.separator();
                        if ui.button(unsnooze()).clicked() {
                            pick = Some(Pick::Command("message.unsnooze"));
                        }
                    }
                    if pick.is_some() {
                        egui::Popup::close_all(ui.ctx());
                    }
                });
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
    format!("{}\n\n{text}", crate::i18n::tr_args("notes-from", &[("from", from.into()), ("date", dates::long(m.date).into())]))
}

impl MailApp {
    /// Runs what the menu picked on `m`, selecting it first so the commands and the compose
    /// window work on the message that was clicked.
    pub fn act_on_message(&mut self, m: &MessageHeader, pick: Pick) {
        if self.engine.session.selected_message.as_deref() != Some(m.id.as_str()) {
            self.select_message(&m.id);
        }
        match pick {
            Pick::Compose(kind) => self.open_compose(kind),
            Pick::ForwardAttached => self.forward_as_attachment(),
            Pick::Resend => self.resend_message(m),
            Pick::Command(id) => self.run(id, Value::Null),
            Pick::FollowUp(when) => self.run("message.follow_up", json!({ "when": when })),
            Pick::Snooze(when) => self.run("message.snooze", json!({ "when": when })),
            Pick::Move => self.move_to = true,
            Pick::Task => self.task_from_message(m),
            Pick::Appointment => self.appointment_from_message(m),
            Pick::Meeting => self.meeting_from_message(m),
            Pick::Sender => self.open_or_add_sender(&m.from),
            Pick::Rule => self.open_rules(true),
            Pick::Cleanup => self.ask_cleanup_conversation(&m.id),
            Pick::FromSender => {
                // Outlook's "related messages from sender": the search box takes the address,
                // over every folder of every account.
                self.drafts_for = None;
                self.engine.session.search = m.from.email().to_string();
                self.engine.session.search_all = true;
            }
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

    /// Outlook's "Beantwoorden met vergaderverzoek": the appointment window for a new meeting on
    /// the next full hour, named after the message, with the sender and the other recipients as
    /// attendees and the message as notes, in the calendar of the account the message came to.
    pub fn meeting_from_message(&mut self, m: &MessageHeader) {
        self.appointment_from_message(m);
        let session = &self.engine.session;
        let account = session.folder(&m.folder_id).map(|f| f.account_id.clone());
        let me = account
            .as_deref()
            .and_then(|id| session.accounts.iter().find(|a| a.id == id))
            .map(|a| a.address.email().to_string())
            .or_else(|| session.current_account().map(|a| a.address.email().to_string()))
            .unwrap_or_default();
        let calendar = account.map(|id| Session::account_calendar_id(&id)).filter(|c| session.calendar(c).is_some());
        if let Some(e) = self.calendar.editor.as_mut() {
            e.attendees = attendee_line(m, &me);
            e.calendar_id = calendar;
        }
    }
}

#[cfg(test)]
mod tests {
    use merel_engine::Engine;

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
        assert_eq!(e.attendees, "", "a plain appointment invites nobody");
    }

    #[test]
    fn reply_with_meeting_invites_the_sender_and_the_other_recipients_but_not_me() {
        let mut app = MailApp::new(Engine::sample());
        let mut m = app.engine.session.message("m1").expect("sample mail").clone();
        m.to.push(merel_model::Address::new(Some("Ravi"), "ravi@example.org").unwrap());
        m.to.push(merel_model::Address::new(None, "SANNE@example.org").unwrap());
        app.act_on_message(&m, Pick::Meeting);
        let e = app.calendar.editor.as_ref().expect("appointment window");
        assert_eq!(e.title, "Planning voor donderdag");
        assert_eq!(e.attendees, "Sanne de Vries <sanne@example.org>; Ravi <ravi@example.org>", "me (jij@example.org) left out, Sanne once");
        assert!(e.notes.starts_with("Van: Sanne de Vries"));
        assert_eq!(e.calendar_id, None, "the sample account has no calendar of its own; the window picks the default");
    }

    #[test]
    fn sluimeren_takes_the_message_out_of_the_list_until_its_moment() {
        let mut app = MailApp::new(Engine::sample());
        let m = app.engine.session.message("m1").expect("sample mail").clone();
        app.act_on_message(&m, Pick::Snooze("tomorrow"));
        assert!(!app.engine.session.visible_messages().iter().any(|h| h.id == "m1"), "out of the list");
        let now = chrono::Local::now().timestamp();
        let until = app.engine.session.snoozed_until(&m, now).expect("a moment");
        assert!(until > now && until - now <= 48 * 3600);
        assert_eq!(app.engine.session.snoozes_due(now).len(), 0);
        app.act_on_message(&m, Pick::Command("message.unsnooze"));
        assert!(app.engine.session.visible_messages().iter().any(|h| h.id == "m1"), "back by hand");
    }

    #[test]
    fn the_sender_items_search_the_address_everywhere_and_start_a_rule_for_it() {
        let mut app = MailApp::new(Engine::sample());
        let m = app.engine.session.message("m1").expect("sample mail").clone();
        app.act_on_message(&m, Pick::FromSender);
        assert_eq!(app.engine.session.search, "sanne@example.org");
        assert!(app.engine.session.search_all);
        assert!(app.engine.session.visible_messages().iter().all(|h| h.from.email() == "sanne@example.org"));
        app.act_on_message(&m, Pick::Rule);
        let form = app.rules.as_ref().and_then(|r| r.form.as_ref()).expect("the rules window with a new rule");
        assert_eq!((form.contains.as_str(), form.field), ("sanne@example.org", merel_model::RuleField::From));
    }
}
