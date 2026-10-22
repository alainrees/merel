//! Contacts: people and organisations with their addresses, kept locally until they sync.

use serde::{Deserialize, Serialize};

use crate::{Address, MAX_NOTES, clean_text};

/// How long a phone number may be, in characters.
const MAX_PHONE: usize = 40;
/// How large a contact's photo may be, in bytes: a server's photos are a few dozen
/// kilobytes, and a picture chosen here is made smaller first.
pub const MAX_PHOTO: usize = 512 * 1024;
/// How long one line of a postal address may be, in characters.
const MAX_LINE: usize = 200;

/// What a phone number is for, as Outlook sorts them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PhoneKind {
    Mobile,
    Work,
    Home,
    #[default]
    Other,
}

/// A phone number with what it is for.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Phone {
    pub kind: PhoneKind,
    pub number: String,
}

impl Phone {
    pub fn new(kind: PhoneKind, number: &str) -> Phone {
        Phone { kind, number: number.to_string() }
    }
}

/// A number of no particular kind.
impl From<&str> for Phone {
    fn from(number: &str) -> Phone {
        Phone::new(PhoneKind::Other, number)
    }
}

/// Caches written before phones had a kind hold them as plain text.
impl<'de> Deserialize<'de> for Phone {
    fn deserialize<D: serde::Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        #[derive(Deserialize)]
        #[serde(untagged)]
        enum Either {
            Text(String),
            Full {
                #[serde(default)]
                kind: PhoneKind,
                number: String,
            },
        }
        Ok(match Either::deserialize(d)? {
            Either::Text(number) => Phone { kind: PhoneKind::Other, number },
            Either::Full { kind, number } => Phone { kind, number },
        })
    }
}

/// What a postal address is.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PlaceKind {
    Work,
    Home,
    #[default]
    Other,
}

/// A postal address, in the parts Outlook and vCard keep.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Place {
    pub kind: PlaceKind,
    /// Street and number; may run over more than one line.
    pub street: String,
    pub postal_code: String,
    pub city: String,
    /// The province or state.
    pub region: String,
    pub country: String,
}

impl Place {
    pub fn is_empty(&self) -> bool {
        [&self.street, &self.postal_code, &self.city, &self.region, &self.country].iter().all(|p| p.trim().is_empty())
    }

    /// The address as lines: the street, then postal code and city, then region and country.
    pub fn lines(&self) -> Vec<String> {
        let join = |parts: &[&str]| parts.iter().map(|p| p.trim()).filter(|p| !p.is_empty()).collect::<Vec<_>>().join(" ");
        let mut out: Vec<String> = self.street.lines().map(str::trim).filter(|l| !l.is_empty()).map(str::to_string).collect();
        out.push(join(&[&self.postal_code, &self.city]));
        out.push([self.region.trim(), self.country.trim()].into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join(", "));
        out.retain(|l| !l.is_empty());
        out
    }

    fn cleaned(mut self) -> Place {
        let line = |s: &str| clean_text(s.trim()).chars().take(MAX_LINE).collect::<String>();
        self.street = self.street.lines().map(line).filter(|l| !l.is_empty()).take(4).collect::<Vec<_>>().join("\n");
        self.postal_code = line(&self.postal_code);
        self.city = line(&self.city);
        self.region = line(&self.region);
        self.country = line(&self.country);
        self
    }
}

/// A contact's photo: the image file as it came, only ever from the server or the computer,
/// never fetched from a link.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Photo(#[serde(with = "crate::base64_bytes")] pub Vec<u8>);

impl std::fmt::Debug for Photo {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Photo({} bytes)", self.0.len())
    }
}

impl Photo {
    /// The photo, when `bytes` are a JPEG, PNG, GIF or WebP image of at most [`MAX_PHOTO`].
    pub fn new(bytes: Vec<u8>) -> Option<Photo> {
        let photo = Photo(bytes);
        (photo.0.len() <= MAX_PHOTO && photo.mime().is_some()).then_some(photo)
    }

    /// The image's type by its first bytes; `None` for anything that is not one of the four.
    pub fn mime(&self) -> Option<&'static str> {
        let b = self.0.as_slice();
        if b.starts_with(&[0xFF, 0xD8, 0xFF]) {
            Some("image/jpeg")
        } else if b.starts_with(b"\x89PNG\r\n\x1a\n") {
            Some("image/png")
        } else if b.starts_with(b"GIF87a") || b.starts_with(b"GIF89a") {
            Some("image/gif")
        } else if b.len() > 12 && b.starts_with(b"RIFF") && &b[8..12] == b"WEBP" {
            Some("image/webp")
        } else {
            None
        }
    }

    /// A short fingerprint, so a picture shown once is not decoded again. It samples a few
    /// hundred bytes and the length, cheap enough to take every frame.
    pub fn fingerprint(&self) -> u64 {
        let step = self.0.len() / 512 + 1;
        let sampled = self.0.iter().step_by(step).chain(self.0.iter().rev().take(64));
        sampled.fold(0xcbf2_9ce4_8422_2325u64 ^ self.0.len() as u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0100_0000_01b3))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub name: String,
    /// E-mail addresses, the first one being the one mail goes to.
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub phones: Vec<Phone>,
    #[serde(default)]
    pub company: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub job_title: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub department: String,
    /// The office or room, Outlook's "Kantoor".
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub office: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub places: Vec<Place>,
    /// A web address, shown as text and opened only when clicked.
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub website: String,
    /// The day of birth as `YYYY-MM-DD`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub birthday: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub photo: Option<Photo>,
    #[serde(default)]
    pub notes: String,
    /// The account the contact lives in; `None` for one kept only on this computer.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub account_id: Option<String>,
    /// The formatting over `notes`, kept on this computer; servers get the notes as text.
    /// `None` for plain notes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub notes_rich: Option<crate::rich::Rich>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContactError {
    #[error("a contact needs a name")]
    NoName,
    #[error("not an e-mail address: {0:?}")]
    BadEmail(String),
}

impl Contact {
    /// Cleans every field and checks the addresses; what comes back is safe to store and show.
    pub fn validated(mut self) -> Result<Self, ContactError> {
        self.name = clean_text(self.name.trim());
        self.company = clean_text(self.company.trim());
        self.notes = self.notes.chars().filter(|c| !c.is_control() || matches!(c, '\n' | '\t')).take(MAX_NOTES).collect();
        self.notes_rich = crate::rich::fitted(self.notes_rich.take(), &self.notes);
        let mut emails = Vec::new();
        for e in &self.emails {
            let e = e.trim().to_ascii_lowercase();
            if e.is_empty() || emails.contains(&e) {
                continue;
            }
            Address::new(None, &e).map_err(|_| ContactError::BadEmail(e.chars().take(60).collect()))?;
            emails.push(e);
        }
        self.emails = emails;
        let mut phones: Vec<Phone> = Vec::new();
        for p in self.phones.drain(..) {
            let number: String = clean_text(p.number.trim()).chars().take(MAX_PHONE).collect();
            if !number.is_empty() && !phones.iter().any(|q| q.kind == p.kind && q.number == number) {
                phones.push(Phone { kind: p.kind, number });
            }
        }
        self.phones = phones;
        for field in [&mut self.job_title, &mut self.department, &mut self.office] {
            *field = clean_text(field.trim()).chars().take(MAX_LINE).collect();
        }
        self.website = clean_text(self.website.trim()).chars().filter(|c| !c.is_whitespace()).take(MAX_LINE).collect();
        self.places = self.places.drain(..).map(Place::cleaned).filter(|p| !p.is_empty()).collect();
        self.birthday =
            self.birthday.take().and_then(|b| chrono::NaiveDate::parse_from_str(b.trim(), "%Y-%m-%d").ok()).map(|d| d.format("%Y-%m-%d").to_string());
        self.photo = self.photo.take().and_then(|p| Photo::new(p.0));
        if self.name.is_empty() {
            // A contact made from an address alone is named after it.
            self.name = self.emails.first().cloned().ok_or(ContactError::NoName)?;
        }
        Ok(self)
    }

    /// Whether `query` (case-insensitive) occurs in the name, an address or the company.
    pub fn matches(&self, query: &str) -> bool {
        let q = query.trim().to_lowercase();
        q.is_empty()
            || self.name.to_lowercase().contains(&q)
            || self.company.to_lowercase().contains(&q)
            || self.emails.iter().any(|e| e.contains(&q))
    }

    /// The contact's addresses, named after the contact.
    pub fn addresses(&self) -> Vec<Address> {
        self.emails.iter().filter_map(|e| Address::new(Some(&self.name), e).ok()).collect()
    }

    /// The first phone number of `kind`.
    pub fn phone(&self, kind: PhoneKind) -> Option<&str> {
        self.phones.iter().find(|p| p.kind == kind).map(|p| p.number.as_str())
    }

    /// The first address of `kind`.
    pub fn place(&self, kind: PlaceKind) -> Option<&Place> {
        self.places.iter().find(|p| p.kind == kind)
    }

    /// The web address as a link to open: only `http` and `https`, with `https://` put in
    /// front of a bare "example.org".
    pub fn website_link(&self) -> Option<String> {
        let w = self.website.trim();
        if w.is_empty() {
            return None;
        }
        let lower = w.to_ascii_lowercase();
        if lower.starts_with("https://") || lower.starts_with("http://") {
            Some(w.to_string())
        } else if !w.contains(':') && w.contains('.') {
            Some(format!("https://{w}"))
        } else {
            None
        }
    }

    /// The first letter of the name, for grouping; "#" when it is not a letter.
    pub fn initial(&self) -> char {
        self.name.chars().next().filter(|c| c.is_alphabetic()).map(|c| c.to_uppercase().next().unwrap_or(c)).unwrap_or('#')
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validation_cleans_and_checks_addresses() {
        let c = Contact {
            id: "c".into(),
            name: " Sanne de Vries ".into(),
            emails: vec!["Sanne@Example.org".into(), " ".into(), "sanne@example.org".into()],
            phones: vec!["06-12345678".into(), "".into()],
            company: "Studio".into(),
            notes: "a\u{1}b".into(),
            ..Contact::default()
        }
        .validated()
        .unwrap();
        assert_eq!(c.name, "Sanne de Vries");
        assert_eq!(c.emails, vec!["sanne@example.org"]);
        assert_eq!(c.phones, vec![Phone::from("06-12345678")]);
        assert_eq!(c.notes, "ab");
        assert_eq!(c.initial(), 'S');
        assert!(c.matches("vries") && c.matches("SANNE@") && c.matches("studio") && !c.matches("ravi"));
        assert_eq!(c.addresses()[0].display(), "Sanne de Vries");

        let bad = Contact { emails: vec!["not an address".into()], name: "x".into(), ..Contact::default() };
        assert_eq!(bad.validated(), Err(ContactError::BadEmail("not an address".into())));
        assert_eq!(Contact::default().validated(), Err(ContactError::NoName));
        let unnamed = Contact { emails: vec!["a@b.nl".into()], ..Contact::default() }.validated().unwrap();
        assert_eq!(unnamed.name, "a@b.nl");
    }

    #[test]
    fn the_details_are_cleaned_and_old_phones_still_read() {
        let png = [b"\x89PNG\r\n\x1a\n".as_slice(), &[0; 8]].concat();
        let c = Contact {
            name: "Sanne".into(),
            phones: vec![Phone::new(PhoneKind::Mobile, " 06 1 "), Phone::new(PhoneKind::Mobile, "06 1"), Phone::new(PhoneKind::Work, "020 1")],
            job_title: " Ontwerper\u{7} ".into(),
            places: vec![
                Place {
                    kind: PlaceKind::Work,
                    street: "Wibautstraat 3\n\n2e etage".into(),
                    postal_code: "1091 GH".into(),
                    city: "Amsterdam".into(),
                    region: "Noord-Holland".into(),
                    country: "Nederland".into(),
                },
                Place { kind: PlaceKind::Home, street: "  ".into(), ..Place::default() },
            ],
            website: " example.org ".into(),
            birthday: Some("1980-02-30".into()),
            photo: Some(Photo(png.clone())),
            ..Contact::default()
        }
        .validated()
        .unwrap();
        assert_eq!(c.phones, vec![Phone::new(PhoneKind::Mobile, "06 1"), Phone::new(PhoneKind::Work, "020 1")]);
        assert_eq!((c.phone(PhoneKind::Work), c.phone(PhoneKind::Home)), (Some("020 1"), None));
        assert_eq!(c.job_title, "Ontwerper");
        assert_eq!(c.places.len(), 1, "an empty address goes");
        assert_eq!(c.place(PlaceKind::Work).unwrap().lines(), ["Wibautstraat 3", "2e etage", "1091 GH Amsterdam", "Noord-Holland, Nederland"]);
        assert_eq!(c.website_link().as_deref(), Some("https://example.org"));
        assert_eq!(c.birthday, None, "no 30 February");
        assert_eq!(c.photo.as_ref().and_then(Photo::mime), Some("image/png"));

        let not_an_image = Contact { name: "x".into(), photo: Some(Photo(b"<svg/>".to_vec())), ..Contact::default() }.validated().unwrap();
        assert!(not_an_image.photo.is_none(), "only real images are kept");
        assert!(Photo::new(vec![0xFF; MAX_PHOTO + 1]).is_none());
        let link = |w: &str| Contact { website: w.into(), ..Contact::default() }.website_link();
        assert_eq!(link("javascript:alert(1)"), None);
        assert_eq!(link("http://a.nl").as_deref(), Some("http://a.nl"));

        let old: Contact = serde_json::from_str(r#"{"id":"c","name":"Ravi","phones":["06 2", {"kind":"home","number":"010 3"}]}"#).unwrap();
        assert_eq!(old.phones, vec![Phone::from("06 2"), Phone::new(PhoneKind::Home, "010 3")]);
        let again: Contact = serde_json::from_str(&serde_json::to_string(&c).unwrap()).unwrap();
        assert_eq!(again, c);
    }
}
