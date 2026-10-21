//! Outlook's "Doorsturen als bijlage": a new message with the selected one attached as an
//! `.eml` file, headers and all, so the reader sees it exactly as it arrived. The same source
//! goes to disk for "Opslaan als…" under Bestand, Outlook's "Save As".

use std::path::{Path, PathBuf};

use merel_model::{Address, FolderKind, MessageHeader, clean_text};

use crate::MailApp;
use crate::compose::{Compose, Kind, prefixed};

pub const FORWARD: &str = "Doorsturen als bijlage";
pub const NOT_FROM_JUNK: &str = "Uit de map Ongewenst wordt niets doorgestuurd.";
pub const SAVE_AS: &str = "Opslaan als…";
pub const NOTHING_TO_SAVE: &str = "Open eerst een bericht om het op te slaan.";

/// The attachment's file name: the subject, without the characters a file name cannot have,
/// with `.eml`; `bericht.eml` when the subject is empty.
pub fn file_name(subject: &str) -> String {
    let stem: String = subject.chars().map(|c| if c.is_control() || "/\\:*?\"<>|".contains(c) { ' ' } else { c }).collect();
    let stem: String = stem.split_whitespace().collect::<Vec<_>>().join(" ").chars().take(80).collect();
    let stem = stem.trim_end_matches('.');
    if stem.is_empty() { "bericht.eml".to_string() } else { format!("{stem}.eml") }
}

fn mailbox(a: &Address) -> String {
    match a.name.as_deref().map(clean_text).filter(|n| !n.trim().is_empty() && n != a.email()) {
        Some(n) => format!("\"{}\" <{}>", n.replace('"', "'"), a.email()),
        None => format!("<{}>", a.email()),
    }
}

/// A plain copy of the message for when its source is not at hand (sample mail, a message too
/// large for the cache): the envelope headers and the text, as a `text/plain` message.
pub fn plain_source(m: &MessageHeader, text: &str) -> Vec<u8> {
    let mut s = format!("From: {}\r\n", mailbox(&m.from));
    if !m.to.is_empty() {
        s += &format!("To: {}\r\n", m.to.iter().map(mailbox).collect::<Vec<_>>().join(", "));
    }
    if let Some(d) = chrono::DateTime::from_timestamp(m.date, 0) {
        s += &format!("Date: {}\r\n", d.to_rfc2822());
    }
    if let Some(id) = m.message_id.as_deref().filter(|id| !id.is_empty()) {
        let id = id.trim_matches(['<', '>']);
        s += &format!("Message-ID: <{}>\r\n", clean_text(id));
    }
    s += &format!("Subject: {}\r\n", clean_text(&m.subject));
    s += "MIME-Version: 1.0\r\nContent-Type: text/plain; charset=utf-8\r\nContent-Transfer-Encoding: 8bit\r\n\r\n";
    s += &text.replace("\r\n", "\n").replace('\n', "\r\n");
    s.into_bytes()
}

impl MailApp {
    /// The message as it arrived when the cache still holds its source, else a plain copy of
    /// its envelope and text.
    fn source_of(&self, m: &MessageHeader) -> Vec<u8> {
        match self.engine.raw_message(&m.id) {
            Some(raw) => raw.to_vec(),
            None => plain_source(m, self.engine.session.bodies.get(&m.id).map(|b| b.text.as_str()).unwrap_or(&m.preview)),
        }
    }

    /// Opens a new message with the selected one attached as an `.eml` file, Outlook's
    /// "Doorsturen als bijlage": nothing is quoted, the message travels whole. Not from the
    /// Junk folder, whose mail is not passed on.
    pub fn forward_as_attachment(&mut self) {
        let Some(m) = self.engine.session.selected().cloned() else { return };
        if self.engine.session.folder(&m.folder_id).is_some_and(|f| f.kind == FolderKind::Junk) {
            self.notice = Some(crate::Notice::warning(NOT_FROM_JUNK));
            return;
        }
        let bytes = self.source_of(&m);
        let session = &self.engine.session;
        let signature = session.current_account().map(|a| a.signature.clone()).unwrap_or_default();
        let account_id = session.current_account().map(|a| a.id.clone()).unwrap_or_default();
        let mut c = Compose::new(Kind::New, None).with_signature(&signature).opened_from(&account_id);
        c.subject = prefixed(&m.subject, "Fw:");
        if let Err(e) = c.add_attachment(&file_name(&m.subject), bytes) {
            c.error = Some(e);
        }
        self.compose = Some(c);
    }

    /// Outlook's "Opslaan als": the selected message as an `.eml` file in the Downloads folder,
    /// named after its subject, never over an existing file. A notice says where it went.
    pub fn save_as_file(&mut self) {
        let Some(dir) = crate::attachments::downloads_dir() else {
            self.notice = Some(crate::Notice::error("Bericht niet opgeslagen: geen map Downloads gevonden."));
            return;
        };
        self.save_as_into(&dir);
    }

    /// Writes the selected message into `dir` as an `.eml` file; the path when it went in.
    pub(crate) fn save_as_into(&mut self, dir: &Path) -> Option<PathBuf> {
        let Some(m) = self.engine.session.selected().cloned() else {
            self.notice = Some(crate::Notice::warning(NOTHING_TO_SAVE));
            return None;
        };
        let bytes = self.source_of(&m);
        match crate::attachments::save_to(dir, &file_name(&m.subject), &bytes) {
            Ok(path) => {
                self.notice = Some(crate::attachments::saved("Bericht", &path));
                Some(path)
            }
            Err(e) => {
                self.notice = Some(crate::Notice::error(format!("Bericht niet opgeslagen: {e}")));
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MailApp;

    #[test]
    fn opslaan_als_writes_the_selected_message_as_an_eml_file_and_never_over_another() {
        let mut app = MailApp::new(merel_engine::Engine::sample());
        let dir = std::env::temp_dir().join(format!("merel-save-as-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("temp dir");

        assert!(app.save_as_into(&dir).is_none(), "nothing is selected yet");
        assert_eq!(app.notice.as_deref(), Some(NOTHING_TO_SAVE));

        let id = app.engine.session.visible_messages().first().map(|m| m.id.clone()).expect("sample mail");
        app.select_message(&id);
        let m = app.engine.session.selected().cloned().expect("selected");
        let first = app.save_as_into(&dir).expect("the file is written");
        assert_eq!(first.file_name().and_then(|n| n.to_str()), Some(file_name(&m.subject).as_str()));
        let source = std::fs::read_to_string(&first).expect("readable");
        assert!(source.contains(&format!("Subject: {}", m.subject)) && source.contains(m.from.email()), "{source}");
        let notice = app.notice.clone().expect("a notice");
        assert!(
            notice.starts_with("Bericht opgeslagen in ") && !notice.contains(&*dir.to_string_lossy()),
            "only the names, not the path: {notice:?}"
        );
        assert_eq!((notice.tone, notice.reveal.as_ref()), (crate::notice::Tone::Success, Some(&first)), "good news, with a way to the file");

        let second = app.save_as_into(&dir).expect("a second copy gets its own name");
        assert_ne!(first, second);
        assert!(second.to_string_lossy().ends_with(" (2).eml"), "{}", second.display());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_file_is_named_after_the_subject_without_what_a_file_name_cannot_hold() {
        assert_eq!(file_name("Offerte: drukwerk / 2026?"), "Offerte drukwerk 2026.eml");
        assert_eq!(file_name("  "), "bericht.eml");
        assert_eq!(file_name("Re: Planning..."), "Re Planning.eml");
    }

    #[test]
    fn the_plain_copy_carries_the_envelope_and_the_text_with_crlf() {
        let session = merel_engine::Session::sample();
        let mut m = session.messages.first().cloned().expect("sample mail");
        m.from = Address::new(Some("Sanne \"de\" Vries"), "sanne@example.org").expect("address");
        m.to = vec![Address::new(None, "ik@example.org").expect("address")];
        m.subject = "Hallo\r\nBcc: x".into();
        m.date = 1_760_000_000;
        m.message_id = Some("<abc@example.org>".into());
        let s = String::from_utf8(plain_source(&m, "Regel een\nRegel twee")).expect("utf-8");
        assert!(s.starts_with("From: \"Sanne 'de' Vries\" <sanne@example.org>\r\nTo: <ik@example.org>\r\nDate: "), "{s}");
        assert!(s.contains("\r\nMessage-ID: <abc@example.org>\r\nSubject: HalloBcc: x\r\nMIME-Version: 1.0\r\n"), "{s}");
        assert!(s.ends_with("\r\n\r\nRegel een\r\nRegel twee"), "{s}");
    }
}
