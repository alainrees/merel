//! "Melden als phishing of spam": a message that got past the checks goes to the Merel team, so
//! the checks can learn from it. Only the click sends it, from the person's own account, as a
//! message with the source attached as an `.eml` file. The person's names and addresses are
//! taken out first (see [`merel_mime::redact`]); afterwards the message goes to Junk.

use crate::i18n::tr;
use merel_engine::SendState;
use merel_mime::Redaction;
use merel_model::{Address, Draft, DraftAttachment, FolderKind, MessageHeader};

use crate::MailApp;

/// Where reports go. Mail through the person's own account for now: no server, nothing to
/// pay. A report box on Merel's own server can take the same `.eml` later.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Destination {
    Mail(&'static str),
}

/// The team's address for reports.
pub const DESTINATION: Destination = Destination::Mail("meldingen@merel.invalid");

/// The message to the team: `source` without the person in it, attached.
pub fn draft(source: &[u8], who: &Redaction) -> Result<Draft, String> {
    let Destination::Mail(to) = DESTINATION;
    let to = Address::new(None, to).map_err(|e| e.to_string())?;
    let body = format!(
        "Gemeld als phishing of spam met Merel {}.\r\nDe gemelde mail zit als bijlage bij, zonder de naam en het adres van de ontvanger.\r\n",
        env!("CARGO_PKG_VERSION")
    );
    Ok(Draft {
        to: vec![to],
        subject: "Merel-melding: phishing of spam".to_string(),
        body,
        attachments: vec![DraftAttachment::new("gemeld.eml", merel_mime::redact(source, who))],
        ..Draft::default()
    })
}

/// The names and addresses of the person's accounts: what is taken out.
fn redaction(app: &MailApp) -> Redaction {
    let mut who = Redaction::default();
    for a in &app.engine.session.accounts {
        who.addresses.push(a.address.email().to_string());
        who.names.extend(a.address.name.clone().filter(|n| !n.trim().is_empty()));
        if !a.name.trim().is_empty() && !a.name.contains('@') {
            who.names.push(a.name.clone());
        }
    }
    who
}

impl MailApp {
    /// Sends `m` to the Merel team from the account it came in on, and files it as junk.
    pub fn report_message(&mut self, m: &MessageHeader) {
        let Some(folder) = self.engine.session.folder(&m.folder_id) else { return };
        let (account, in_junk) = (folder.account_id.clone(), folder.kind == FolderKind::Junk);
        let draft = match draft(&self.source_of(m), &redaction(self)) {
            Ok(d) => d,
            Err(e) => {
                self.notice = Some(crate::Notice::error(e));
                return;
            }
        };
        let id = self.engine.send(&account, draft);
        if let Some(SendState::Failed(e)) = self.engine.session.outbox.get(&id) {
            self.notice = Some(crate::Notice::error(e.clone()));
            return;
        }
        if !in_junk {
            self.select_message(&m.id);
            self.run("message.junk", serde_json::Value::Null);
        }
        if !self.notice.as_ref().is_some_and(|n| n.tone == crate::notice::Tone::Error) {
            self.notice = Some(crate::Notice::success(tr("report-sent")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::kittest::Queryable;

    #[test]
    fn the_report_goes_to_the_team_with_the_source_attached_and_the_reader_taken_out() {
        let source = b"To: \"Sanne de Vries\" <sanne@example.org>\r\nSubject: Beste Sanne\r\n\r\nHoi\r\n";
        let who = Redaction { addresses: vec!["sanne@example.org".into()], names: vec!["Sanne de Vries".into()] };
        let d = draft(source, &who).expect("a draft");
        let Destination::Mail(to) = DESTINATION;
        assert_eq!(d.to.iter().map(|a| a.email()).collect::<Vec<_>>(), [to]);
        assert_eq!(d.attachments[0].name, "gemeld.eml");
        let sent = String::from_utf8(d.attachments[0].bytes.to_vec()).expect("text");
        assert!(sent.contains("Subject: Beste Ontvanger") && !sent.contains("Sanne") && !sent.contains("sanne"), "{sent}");
    }

    #[test]
    fn the_button_over_the_message_sends_the_report_at_once() {
        let mut app = MailApp::new(merel_engine::Engine::sample());
        let m = app.engine.session.visible_messages().first().cloned().cloned().expect("sample mail");
        app.select_message(&m.id);
        let mut h = egui_kittest::Harness::builder().with_size(egui::vec2(1280.0, 900.0)).build_ui_state(|ui, app: &mut MailApp| app.ui(ui), app);
        h.run_steps(2);
        let before = h.state().engine.session.outbox.len();
        h.get_by_label(&tr("report-button")).click();
        h.run_steps(2);
        assert_eq!(h.state().engine.session.outbox.len(), before + 1, "on its way with the click");
        // Sample mail has no account to send from: the failure shows and the message stays put.
        assert_eq!(h.state().notice.as_ref().map(|n| n.tone), Some(crate::notice::Tone::Error));
        let s = &h.state().engine.session;
        assert_eq!(s.message(&m.id).and_then(|m| s.folder(&m.folder_id)).map(|f| f.kind), Some(FolderKind::Inbox), "not moved when nothing went");
    }
}
