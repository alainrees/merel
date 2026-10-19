//! Outlook's "Dit bericht opnieuw verzenden": the message opens as a new one with the same
//! recipients, subject, text and attachments, to send again, to someone else, or corrected.

use mailcraft_model::{FolderKind, MessageHeader};

use crate::MailApp;
use crate::compose::{Compose, Kind, join};

pub const RESEND: &str = "Opnieuw verzenden";

impl MailApp {
    /// Opens `m` as a new message: the same recipients, subject, text, urgency and attachments,
    /// from the account the message belongs to. Nothing is quoted and no prefix is added;
    /// nothing goes until Send is pressed. Attachments from the Junk folder stay behind.
    pub fn resend_message(&mut self, m: &MessageHeader) {
        let session = &self.engine.session;
        let folder = session.folder(&m.folder_id);
        let in_junk = folder.is_some_and(|f| f.kind == FolderKind::Junk);
        let account_id = folder.map(|f| f.account_id.clone()).or_else(|| session.current_account().map(|a| a.id.clone())).unwrap_or_default();
        let body = session.bodies.get(&m.id);
        let text = body.map(|b| b.text.clone()).unwrap_or_else(|| m.preview.clone());
        let attachments: Vec<(String, Vec<u8>)> = match body {
            Some(b) if !in_junk => b.attachments.iter().map(|a| (a.name.clone(), a.bytes.to_vec())).collect(),
            _ => Vec::new(),
        };
        let mut c = Compose::new(Kind::New, None);
        c.to = join(&m.to);
        c.subject = m.subject.clone();
        c.body = text;
        c.importance = m.importance;
        for (name, bytes) in attachments {
            if let Err(e) = c.add_attachment(&name, bytes) {
                c.error = Some(e);
            }
        }
        self.compose = Some(c.opened_from(&account_id));
    }
}

#[cfg(test)]
mod tests {
    use mailcraft_engine::Engine;

    use super::*;

    #[test]
    fn opnieuw_verzenden_opens_the_message_as_a_new_one_with_the_same_recipients_and_text() {
        let mut app = MailApp::new(Engine::sample());
        let m = app.engine.session.message("m1").cloned().expect("sample mail");
        app.resend_message(&m);
        let c = app.compose.as_ref().expect("a message opens");
        assert_eq!((c.to.as_str(), c.subject.as_str()), (join(&m.to).as_str(), m.subject.as_str()), "no prefix, the same recipients");
        assert_eq!(c.body, m.preview, "the preview stands in for a text not fetched; nothing is quoted");
        assert!(c.in_reply_to.is_none() && c.attachments.is_empty());
        let folder = app.engine.session.folder(&m.folder_id).expect("the folder");
        assert_eq!(c.account_id, folder.account_id, "it goes out from the account the message belongs to");
    }
}
