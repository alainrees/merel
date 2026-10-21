//! What Merel tells the computer, as Outlook does: the unread count of every inbox on the app
//! icon, and a notification in the corner of the screen for new mail and for reminders while
//! Merel is in the background. A click on one brings the message, appointment or task up.
//!
//! The notification shows no sender, subject or title unless the person switches that on
//! ([`Prefs::details`]): a notification can be read off a locked screen or over a shoulder.

use egui::{RichText, Ui};
use merel_model::MessageHeader;
use merel_notify::{Alert, Kind};
use serde::{Deserialize, Serialize};
use serde_json::json;

use crate::{MailApp, Module, Tokens, helder};

/// The choices on Bestand > Meldingen.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Prefs {
    /// The unread count of every inbox on the app icon.
    pub badge: bool,
    /// A notification for new mail in an inbox.
    pub mail: bool,
    /// A notification for reminders of appointments, tasks and flagged mail.
    pub reminders: bool,
    pub sound: bool,
    /// Sender and subject, or the title of an appointment or task, in the notification.
    pub details: bool,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs { badge: true, mail: true, reminders: true, sound: true, details: false }
    }
}

pub const PAGE: &str = "Meldingen";
pub const BADGE: &str = "Aantal ongelezen berichten op het app-icoon";
pub const MAIL: &str = "Bij nieuwe e-mail in Postvak IN";
pub const REMINDERS: &str = "Bij herinneringen voor afspraken en taken";
pub const SOUND: &str = "Geluid afspelen";
pub const DETAILS: &str = "Afzender, onderwerp en titel in de melding tonen";
pub const TRY: &str = "Proefmelding sturen";

/// More new messages than this at once come as one notification with the count.
const ONE_BY_ONE: usize = 3;

/// The notification for new mail: one per message, each opening its message.
pub fn mail_alert(prefs: &Prefs, m: &MessageHeader) -> Alert {
    let (title, body) = if prefs.details {
        let from = m.from.display().to_string();
        let subject = if m.subject.trim().is_empty() { "(geen onderwerp)".to_string() } else { m.subject.clone() };
        (from, subject)
    } else {
        ("Nieuw bericht".to_string(), "Je hebt een nieuw bericht in Postvak IN.".to_string())
    };
    Alert { id: format!("mail:{}", m.id), title, body, kind: Kind::Mail, sound: prefs.sound }
}

/// One notification for many new messages at once, which opens the mail.
pub fn many_alert(prefs: &Prefs, count: usize) -> Alert {
    Alert { id: "inbox".into(), title: format!("{count} nieuwe berichten"), body: "In Postvak IN.".into(), kind: Kind::Mail, sound: prefs.sound }
}

/// The notification for a reminder that just came due. `id` is what a click hands back (see
/// [`open`]); `title` and `details` only show with [`Prefs::details`] on.
pub fn reminder_alert(prefs: &Prefs, id: String, kind: &str, title: &str, details: &str) -> Alert {
    let (title, body) = if prefs.details {
        (title.to_string(), if details.is_empty() { kind.to_string() } else { format!("{kind} · {details}") })
    } else {
        ("Herinnering".to_string(), format!("{kind}: open Merel om te zien wat er is."))
    };
    Alert { id, title, body, kind: Kind::Reminder, sound: prefs.sound }
}

impl MailApp {
    /// Hands the notification to the computer, if Merel is not in front: then the window and
    /// its own notice say it already. Every one is kept in [`MailApp::alerts`] too.
    pub fn alert(&mut self, ctx: &egui::Context, alert: Alert, always: bool) {
        let in_front = ctx.input(|i| i.viewport().focused) == Some(true);
        if in_front && !always {
            return;
        }
        if let Some(n) = self.notifier.as_mut() {
            n.show(&alert);
        }
        self.alerts.push(alert);
        let extra = self.alerts.len().saturating_sub(20);
        self.alerts.drain(..extra);
    }

    /// Notifications for the messages that just came in, still unread in an inbox (a rule or
    /// the junk filter may have moved them on).
    pub fn alert_new_mail(&mut self, ctx: &egui::Context, ids: &[String]) {
        if !self.view.notifications.mail {
            return;
        }
        let s = &self.engine.session;
        let fresh: Vec<MessageHeader> = ids
            .iter()
            .filter_map(|id| s.message(id))
            .filter(|m| !m.flags.seen && s.folder(&m.folder_id).is_some_and(|f| f.kind == merel_model::FolderKind::Inbox))
            .cloned()
            .collect();
        let prefs = self.view.notifications.clone();
        if fresh.len() > ONE_BY_ONE {
            self.alert(ctx, many_alert(&prefs, fresh.len()), false);
        } else {
            for m in &fresh {
                self.alert(ctx, mail_alert(&prefs, m), false);
            }
        }
    }

    /// Every frame: the count on the icon, and what was clicked in a notification.
    pub fn tick_notifications(&mut self, ctx: &egui::Context) {
        let count = if self.view.notifications.badge { self.engine.session.inbox_unread() } else { 0 };
        if let Some(n) = self.notifier.as_mut() {
            n.set_badge(count);
        }
        for id in merel_notify::take_clicked() {
            ctx.send_viewport_cmd(egui::ViewportCommand::Visible(true));
            ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
            ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
            open(self, &id);
        }
    }
}

/// Opens what a clicked notification was about.
pub fn open(app: &mut MailApp, id: &str) {
    app.tab = crate::Tab::Home;
    if let Some(message) = id.strip_prefix("mail:") {
        app.open_message(message);
    } else if let Some(rest) = id.strip_prefix("appt:")
        && let Some((start, appointment)) = rest.split_once(':')
        && let Ok(start) = start.parse::<i64>()
    {
        app.module = Module::Calendar;
        app.calendar.focus = crate::calendar::to_local(start).date();
        app.open_appointment(appointment, start);
    } else if let Some(task) = id.strip_prefix("task:") {
        app.module = Module::Tasks;
        app.run("task.select", json!({ "id": task }));
    } else {
        app.module = Module::Mail;
    }
}

/// Bestand > Meldingen.
pub fn page(app: &mut MailApp, ui: &mut Ui, t: &Tokens) {
    let hint = |ui: &mut Ui, text: &str| {
        ui.horizontal(|ui| {
            ui.add_space(44.0);
            ui.add(egui::Label::new(RichText::new(text).size(12.0).color(t.text_dim)).wrap());
        });
    };
    let section = |ui: &mut Ui, text: &str| {
        ui.add_space(10.0);
        ui.label(RichText::new(text).size(14.0).strong().color(t.text));
    };
    ui.label(RichText::new(PAGE).size(helder::TITLE_TEXT).strong().color(t.text));
    ui.label(RichText::new("Zoals in Outlook: een teller op het Merel-icoon en een melding van je computer als er iets binnenkomt terwijl Merel op de achtergrond staat.").size(13.0).color(t.text_dim));
    let before = app.view.notifications.clone();
    let p = &mut app.view.notifications;
    section(ui, "Op het app-icoon");
    helder::toggle(ui, t, &mut p.badge, BADGE);
    hint(ui, "Alle accounts samen, alleen Postvak IN. De titel van het venster toont het ook: \"Merel (3)\".");
    section(ui, "Meldingen van je computer");
    helder::toggle(ui, t, &mut p.mail, MAIL);
    helder::toggle(ui, t, &mut p.reminders, REMINDERS);
    helder::toggle(ui, t, &mut p.sound, SOUND);
    section(ui, "Privacy");
    helder::toggle(ui, t, &mut p.details, DETAILS);
    hint(
        ui,
        "Uit: de melding zegt alleen \"Nieuw bericht\" of \"Herinnering\", zodat niemand over je schouder of op een vergrendeld scherm meeleest.",
    );
    ui.add_space(10.0);
    let try_it = ui.button(TRY).clicked();
    #[cfg(target_os = "macos")]
    hint(ui, "Zie je niets? Kijk in Systeeminstellingen > Berichtgeving > Merel of meldingen aan staan.");
    if app.view.notifications != before {
        if (app.view.notifications.mail || app.view.notifications.reminders)
            && let Some(n) = app.notifier.as_mut()
        {
            n.prepare();
        }
        app.save_view(ui.ctx());
    }
    if try_it {
        let prefs = app.view.notifications.clone();
        let title = if prefs.details { "Merel" } else { "Nieuw bericht" };
        let alert = Alert {
            id: "inbox".into(),
            title: title.into(),
            body: "Zo ziet een melding van Merel eruit.".into(),
            kind: Kind::Mail,
            sound: prefs.sound,
        };
        app.alert(&ui.ctx().clone(), alert, true);
        app.notice = Some(crate::Notice::info("Proefmelding verstuurd."));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use merel_model::{Address, Flags};

    fn message() -> MessageHeader {
        MessageHeader {
            id: "inbox#7".into(),
            folder_id: "inbox".into(),
            remote_id: String::new(),
            from: Address::new(Some("Sanne Visser"), "sanne@example.org").unwrap(),
            to: Vec::new(),
            subject: "Planning voor donderdag".into(),
            date: 0,
            preview: String::new(),
            flags: Flags::default(),
            has_attachments: false,
            message_id: None,
            categories: Vec::new(),
            importance: Default::default(),
            chat: None,
        }
    }

    #[test]
    fn mail_notifications_keep_sender_and_subject_back_unless_asked() {
        let quiet = mail_alert(&Prefs::default(), &message());
        assert_eq!((quiet.title.as_str(), quiet.id.as_str()), ("Nieuw bericht", "mail:inbox#7"));
        assert!(!quiet.body.contains("Planning") && !quiet.title.contains("Sanne"));
        let open = mail_alert(&Prefs { details: true, ..Prefs::default() }, &message());
        assert!(open.title.contains("Sanne Visser"));
        assert_eq!(open.body, "Planning voor donderdag");
    }

    #[test]
    fn reminder_notifications_hide_the_title_unless_asked() {
        let quiet = reminder_alert(&Prefs::default(), "appt:1:a".into(), "Afspraak", "Tandarts", "Begint over 15 minuten");
        assert_eq!(quiet.title, "Herinnering");
        assert!(!quiet.body.contains("Tandarts"));
        let open = reminder_alert(
            &Prefs { details: true, sound: false, ..Prefs::default() },
            "appt:1:a".into(),
            "Afspraak",
            "Tandarts",
            "Begint over 15 minuten",
        );
        assert_eq!((open.title.as_str(), open.body.as_str(), open.sound), ("Tandarts", "Afspraak · Begint over 15 minuten", false));
    }
}
