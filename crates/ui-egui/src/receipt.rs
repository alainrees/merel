//! Read receipts, Outlook's "Leesbevestiging": asking for one in the compose window, and the
//! strip over a message whose sender asked for one. Nothing goes back by itself: the person
//! presses the button, or does not, and the sender learns nothing otherwise.

use egui::{RichText, Ui};
use mailcraft_engine::SendState;
use mailcraft_model::{Address, Draft, MessageHeader};

use crate::{MailApp, Tokens};

/// The checkbox in the compose window.
pub const ASK: &str = "Leesbevestiging vragen";
/// The strip over a message that asks for one.
pub const STRIP: &str = "De afzender vraagt een leesbevestiging.";
pub const SEND: &str = "Leesbevestiging sturen";
/// What the strip says once the receipt went out.
pub const SENT: &str = "Leesbevestiging verstuurd.";

/// The receipt as a plain message any mail program can show: which message, and when it was
/// read. It answers the message, so it lands in the sender's conversation.
pub fn draft(m: &MessageHeader, to: &Address, read_at: i64) -> Draft {
    let subject = m.subject.trim();
    let body = format!(
        "Dit is een leesbevestiging voor je bericht \"{subject}\" van {sent}.\n\nHet bericht is gelezen op {read}.\n\nDeze bevestiging zegt alleen dat het bericht is geopend, niet dat het is beantwoord.\n",
        sent = crate::compose::when_text(m.date),
        read = crate::compose::when_text(read_at),
    );
    Draft { to: vec![to.clone()], subject: format!("Gelezen: {subject}"), body, in_reply_to: m.message_id.clone(), ..Draft::default() }
}

/// The strip over the message; true when the person asks for the receipt to go out.
pub fn strip(ui: &mut Ui, t: &Tokens, to: &Address, sent: bool) -> bool {
    let mut send = false;
    egui::Frame::new().fill(t.checked).corner_radius(4).inner_margin(8).show(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            if sent {
                ui.label(RichText::new(SENT).color(t.text));
            } else {
                ui.label(RichText::new(STRIP).color(t.text));
                send = ui
                    .small_button(SEND)
                    .on_hover_text(format!(
                        "Stuurt een korte e-mail aan {} dat je dit bericht hebt gelezen. Er gaat nooit iets vanzelf; als je dit niet doet, merkt de afzender niets.",
                        to.email()
                    ))
                    .clicked();
            }
        });
    });
    ui.add_space(8.0);
    send
}

impl MailApp {
    /// Sends the receipt the sender asked for, from the account the message came in on.
    pub fn send_read_receipt(&mut self, message_id: &str) {
        let Some(m) = self.engine.session.message(message_id).cloned() else {
            self.notice = Some("Het bericht is niet meer te vinden.".to_string());
            return;
        };
        let Some(to) = self.engine.session.bodies.get(message_id).and_then(|b| b.receipt_to.clone()) else { return };
        let Some(account) = self.engine.session.folder(&m.folder_id).map(|f| f.account_id.clone()) else { return };
        let id = self.engine.send(&account, draft(&m, &to, chrono::Local::now().timestamp()));
        match self.engine.session.outbox.get(&id) {
            Some(SendState::Failed(e)) => self.notice = Some(e.clone()),
            _ => {
                self.receipts_sent.insert(message_id.to_string());
                self.notice = Some(format!("Leesbevestiging verstuurd aan {}.", to.email()));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use mailcraft_model::Flags;

    use super::*;

    #[test]
    fn the_receipt_answers_the_message_and_names_when_it_was_read() {
        let m = MessageHeader {
            id: "m".into(),
            folder_id: "f".into(),
            remote_id: "1".into(),
            from: Address::new(Some("Sanne"), "sanne@example.org").unwrap(),
            to: Vec::new(),
            subject: " Planning ".into(),
            date: 1_760_000_000,
            preview: String::new(),
            flags: Flags::default(),
            has_attachments: false,
            message_id: Some("<x@example.org>".into()),
            categories: Vec::new(),
            importance: Default::default(),
        };
        let to = Address::new(None, "receipts@example.org").unwrap();
        let d = draft(&m, &to, 1_760_100_000);
        assert_eq!((d.subject.as_str(), d.in_reply_to.as_deref(), d.to.len()), ("Gelezen: Planning", Some("<x@example.org>"), 1));
        assert!(d.body.contains("\"Planning\" van") && d.body.contains("gelezen op"), "{}", d.body);
        assert!(!d.read_receipt, "a receipt asks for no receipt");
    }
}
