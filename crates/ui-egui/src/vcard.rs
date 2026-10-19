//! Contacts as vCard files (`.vcf`), the format Outlook and phones exchange: "Opslaan als vCard"
//! on a contact card writes one to the Downloads folder, and "Importeren…" on the ribbon reads
//! one or more files into the local address book. Both use the vCard code the CardDAV sync
//! already has, so what goes out is what a server would get.

use std::path::Path;

use mailcraft_dav::ical;
use mailcraft_model::Contact;
use serde_json::json;

use crate::MailApp;
use crate::attachments::{downloads_dir, save_to};

pub const EXPORT: &str = "Opslaan als vCard";
pub const IMPORT: &str = "Importeren…";

/// The most a vCard file may be to be read; address books are small, and this keeps a wrong
/// file from being loaded whole.
pub const MAX_FILE: u64 = 8 * 1024 * 1024;

/// The contact as a vCard 3.0 text, with a fresh UID.
pub fn export(c: &Contact) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut card = ical::new_card(&format!("mailcraft-{now}-{}", c.id.len()));
    ical::write_contact(&mut card, c);
    card.serialize()
}

/// Every readable contact in a vCard text; a file may hold many cards.
pub fn import(text: &str) -> Vec<Contact> {
    ical::parse(text).iter().filter(|r| r.name == "VCARD").filter_map(|card| ical::contact_of(card, String::new())).collect()
}

/// What an import did, for the notice.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Outcome {
    pub added: usize,
    /// Cards whose first address the address book already had.
    pub skipped: usize,
    /// Files that could not be read or held no card.
    pub failed: usize,
}

impl Outcome {
    pub fn notice(&self) -> String {
        let mut parts = vec![match self.added {
            1 => "1 contact geïmporteerd".to_string(),
            n => format!("{n} contacten geïmporteerd"),
        }];
        if self.skipped > 0 {
            parts.push(format!("{} al bekend en overgeslagen", self.skipped));
        }
        if self.failed > 0 {
            parts.push(format!("{} bestand(en) niet leesbaar", self.failed));
        }
        format!("{}.", parts.join(", "))
    }
}

impl MailApp {
    /// Writes the selected contact to `<naam>.vcf` in the Downloads folder.
    pub fn export_selected_contact(&mut self) {
        let Some(c) = self.engine.session.selected_contact().cloned() else { return };
        let text = export(&c);
        let name = format!("{}.vcf", if c.name.trim().is_empty() { "contact" } else { c.name.trim() });
        self.notice = Some(match downloads_dir() {
            Some(dir) => match save_to(&dir, &name, text.as_bytes()) {
                Ok(path) => format!("Contact opgeslagen als {}", path.display()),
                Err(e) => format!("Contact niet opgeslagen: {e}"),
            },
            None => "Contact niet opgeslagen: geen map Downloads gevonden.".into(),
        });
    }

    /// Reads vCard files into the local address book; a card whose first address is known
    /// already is skipped. The outcome shows in the status bar.
    pub fn import_contacts(&mut self, paths: &[impl AsRef<Path>]) -> Outcome {
        let mut out = Outcome::default();
        for path in paths {
            let path = path.as_ref();
            let text = match std::fs::metadata(path) {
                Ok(m) if m.is_file() && m.len() <= MAX_FILE => std::fs::read(path).ok().map(|b| String::from_utf8_lossy(&b).into_owned()),
                _ => None,
            };
            let cards = text.map(|t| import(&t)).unwrap_or_default();
            if cards.is_empty() {
                out.failed += 1;
                continue;
            }
            for c in cards {
                let known =
                    c.emails.first().is_some_and(|e| self.engine.session.contacts.iter().any(|k| k.emails.iter().any(|x| x.eq_ignore_ascii_case(e))));
                if known {
                    out.skipped += 1;
                    continue;
                }
                let params = json!({ "name": c.name, "emails": c.emails, "phones": c.phones, "company": c.company, "notes": c.notes });
                match self.engine.run("contact.save", &params) {
                    Ok(()) => out.added += 1,
                    Err(_) => out.failed += 1,
                }
            }
        }
        self.notice = Some(out.notice());
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_contact_survives_the_trip_through_a_vcard() {
        let c = Contact {
            id: "c1".into(),
            name: "Sanne de Vries".into(),
            emails: vec!["sanne@example.org".into(), "s.devries@example.com".into()],
            phones: vec!["06 12345678".into()],
            company: "Studio ArtCraft".into(),
            notes: "Ontwerpt de schetsen;\nbelt liever.".into(),
            account_id: None,
        };
        let text = export(&c);
        assert!(
            text.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n") && text.contains("FN:Sanne de Vries") && text.ends_with("END:VCARD\r\n"),
            "{text}"
        );
        let back = import(&text);
        assert_eq!(back.len(), 1);
        assert_eq!((back[0].name.as_str(), &back[0].emails, &back[0].phones), ("Sanne de Vries", &c.emails, &c.phones));
        assert_eq!((back[0].company.as_str(), back[0].notes.as_str()), ("Studio ArtCraft", "Ontwerpt de schetsen;\nbelt liever."));
    }

    #[test]
    fn a_file_with_several_cards_is_imported_once_and_known_addresses_are_skipped() {
        let dir = std::env::temp_dir().join(format!("mailcraft-vcf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("export.vcf");
        std::fs::write(
            &file,
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ravi Patel\r\nEMAIL;TYPE=INTERNET:ravi.patel@example.net\r\nTEL:+31 6 1111\r\nEND:VCARD\r\n\
             BEGIN:VCARD\r\nVERSION:2.1\r\nN:de Vries;Sanne\r\nEMAIL:sanne@example.org\r\nEND:VCARD\r\n\
             BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Zonder adres\r\nORG:Firma\r\nEND:VCARD\r\n",
        )
        .unwrap();
        let mut app = MailApp::new(mailcraft_engine::Engine::sample());
        let before = app.engine.session.contacts.len();
        assert!(app.engine.session.contacts.iter().any(|c| c.emails.iter().any(|e| e == "sanne@example.org")), "the sample knows Sanne");
        let out = app.import_contacts(&[file.clone(), dir.join("missing.vcf")]);
        assert_eq!(out, Outcome { added: 2, skipped: 1, failed: 1 });
        assert_eq!(app.engine.session.contacts.len(), before + 2);
        assert!(app.engine.session.contacts.iter().any(|c| c.name == "Ravi Patel" && c.phones == ["+31 6 1111"]));
        assert!(app.engine.session.contacts.iter().any(|c| c.name == "Zonder adres" && c.company == "Firma"));
        assert_eq!(app.notice.as_deref(), Some("2 contacten geïmporteerd, 1 al bekend en overgeslagen, 1 bestand(en) niet leesbaar."));
        // The same file again adds nothing; the card without an address is the exception, as
        // nothing identifies it.
        let again = app.import_contacts(&[file]);
        assert_eq!((again.added, again.skipped), (1, 2));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
