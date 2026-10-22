//! Contacts as vCard files (`.vcf`), the format Outlook and phones exchange: "Opslaan als vCard"
//! on a contact card writes one to the Downloads folder, and "Importeren…" on the ribbon reads
//! one or more files into the local address book. Both use the vCard code the CardDAV sync
//! already has, so what goes out is what a server would get.

use crate::i18n::tr;
use std::path::Path;

use merel_dav::ical;
use merel_model::Contact;
use serde_json::json;

use crate::MailApp;
use crate::attachments::{downloads_dir, save_to};

pub fn export_label() -> String {
    tr("vcard-export")
}
pub fn import_label() -> String {
    tr("contacts-import")
}

/// The most a vCard file may be to be read; address books are small, and this keeps a wrong
/// file from being loaded whole.
pub const MAX_FILE: u64 = 8 * 1024 * 1024;

/// The contact as a vCard 3.0 text, with a fresh UID.
pub fn export(c: &Contact) -> String {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let mut card = ical::new_card(&format!("merel-{now}-{}", c.id.len()));
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
        let mut parts = vec![crate::i18n::tr_args("vcard-imported", &[("count", self.added.into())])];
        if self.skipped > 0 {
            parts.push(crate::i18n::tr_args("vcard-skipped", &[("count", self.skipped.into())]));
        }
        if self.failed > 0 {
            parts.push(crate::i18n::tr_args("vcard-failed", &[("count", self.failed.into())]));
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
                Ok(path) => crate::attachments::saved(&tr("noun-contact"), &path),
                Err(e) => crate::Notice::error(crate::i18n::tr_args("vcard-not-saved", &[("error", e.to_string().into())])),
            },
            None => crate::Notice::error(tr("vcard-no-downloads")),
        });
    }

    /// Reads vCard files into the local address book; a card whose first address is known
    /// already is skipped. A notice says how it went.
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
                use base64::Engine as _;
                let params = json!({ "name": c.name, "emails": c.emails, "phones": c.phones, "company": c.company, "notes": c.notes,
                    "job_title": c.job_title, "department": c.department, "office": c.office, "website": c.website, "birthday": c.birthday,
                    "places": c.places, "photo": c.photo.as_ref().map(|p| base64::engine::general_purpose::STANDARD.encode(&p.0)) });
                match self.engine.run("contact.save", &params) {
                    Ok(()) => out.added += 1,
                    Err(_) => out.failed += 1,
                }
            }
        }
        let tone = match out {
            Outcome { added: 0, failed: 0, .. } => crate::notice::Tone::Info,
            Outcome { failed: 0, .. } => crate::notice::Tone::Success,
            _ => crate::notice::Tone::Warning,
        };
        self.notice = Some(crate::Notice::new(tone, out.notice()));
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
            phones: vec![merel_model::Phone::new(merel_model::PhoneKind::Mobile, "06 12345678")],
            company: "Studio ArtCraft".into(),
            job_title: "Art director".into(),
            places: vec![merel_model::Place { kind: merel_model::PlaceKind::Work, city: "Amsterdam".into(), ..Default::default() }],
            photo: crate::contacts::picture_from_bytes(&tiny_png()),
            notes: "Ontwerpt de schetsen;\nbelt liever.".into(),
            ..Contact::default()
        };
        assert!(c.photo.is_some());
        let text = export(&c);
        assert!(
            text.starts_with("BEGIN:VCARD\r\nVERSION:3.0\r\n") && text.contains("FN:Sanne de Vries") && text.ends_with("END:VCARD\r\n"),
            "{text}"
        );
        let back = import(&text);
        assert_eq!(back.len(), 1);
        assert_eq!((back[0].name.as_str(), &back[0].emails, &back[0].phones), ("Sanne de Vries", &c.emails, &c.phones));
        assert_eq!((back[0].company.as_str(), back[0].notes.as_str()), ("Studio ArtCraft", "Ontwerpt de schetsen;\nbelt liever."));
        assert_eq!((&back[0].job_title, &back[0].places, &back[0].photo), (&c.job_title, &c.places, &c.photo), "the details and photo go along");
    }

    /// A small picture, as a file from the computer would be.
    fn tiny_png() -> Vec<u8> {
        let img = image::RgbImage::from_fn(300, 200, |x, _| image::Rgb([(x % 256) as u8, 90, 160]));
        let mut out = Vec::new();
        image::DynamicImage::ImageRgb8(img).write_to(&mut std::io::Cursor::new(&mut out), image::ImageFormat::Png).unwrap();
        out
    }

    #[test]
    fn a_chosen_picture_is_cut_square_and_made_small() {
        let photo = crate::contacts::picture_from_bytes(&tiny_png()).unwrap();
        assert_eq!(photo.mime(), Some("image/jpeg"));
        let img = image::load_from_memory(&photo.0).unwrap();
        assert_eq!((img.width(), img.height()), (200, 200), "a square from the middle, not larger than it was");
        let big = image::RgbImage::from_fn(1200, 900, |_, y| image::Rgb([20, (y % 256) as u8, 40]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(big).write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png).unwrap();
        let small = image::load_from_memory(&crate::contacts::picture_from_bytes(&png).unwrap().0).unwrap();
        assert_eq!((small.width(), small.height()), (256, 256));
        assert!(crate::contacts::picture_from_bytes(b"<svg xmlns='http://www.w3.org/2000/svg'/>").is_none());
    }

    #[test]
    fn a_file_with_several_cards_is_imported_once_and_known_addresses_are_skipped() {
        let dir = std::env::temp_dir().join(format!("merel-vcf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("export.vcf");
        std::fs::write(
            &file,
            "BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Ravi Patel\r\nEMAIL;TYPE=INTERNET:ravi.patel@example.net\r\nTEL:+31 6 1111\r\nEND:VCARD\r\n\
             BEGIN:VCARD\r\nVERSION:2.1\r\nN:de Vries;Sanne\r\nEMAIL:sanne@example.org\r\nEND:VCARD\r\n\
             BEGIN:VCARD\r\nVERSION:3.0\r\nFN:Zonder adres\r\nORG:Firma\r\nEND:VCARD\r\n",
        )
        .unwrap();
        let mut app = MailApp::new(merel_engine::Engine::sample());
        let before = app.engine.session.contacts.len();
        assert!(app.engine.session.contacts.iter().any(|c| c.emails.iter().any(|e| e == "sanne@example.org")), "the sample knows Sanne");
        let out = app.import_contacts(&[file.clone(), dir.join("missing.vcf")]);
        assert_eq!(out, Outcome { added: 2, skipped: 1, failed: 1 });
        assert_eq!(app.engine.session.contacts.len(), before + 2);
        assert!(app.engine.session.contacts.iter().any(|c| c.name == "Ravi Patel" && c.phones == [merel_model::Phone::from("+31 6 1111")]));
        assert!(app.engine.session.contacts.iter().any(|c| c.name == "Zonder adres" && c.company == "Firma"));
        assert_eq!(app.notice.as_deref(), Some("2 contacten geïmporteerd, 1 al bekend en overgeslagen, 1 bestand niet leesbaar."));
        // The same file again adds nothing; the card without an address is the exception, as
        // nothing identifies it.
        let again = app.import_contacts(&[file]);
        assert_eq!((again.added, again.skipped), (1, 2));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
