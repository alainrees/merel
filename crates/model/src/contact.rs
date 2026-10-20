//! Contacts: people and organisations with their addresses, kept locally until they sync.

use serde::{Deserialize, Serialize};

use crate::{Address, MAX_NOTES, clean_text};

/// How long a phone number may be, in characters.
const MAX_PHONE: usize = 40;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Contact {
    pub id: String,
    pub name: String,
    /// E-mail addresses, the first one being the one mail goes to.
    #[serde(default)]
    pub emails: Vec<String>,
    #[serde(default)]
    pub phones: Vec<String>,
    #[serde(default)]
    pub company: String,
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
        self.phones =
            self.phones.iter().map(|p| clean_text(p.trim()).chars().take(MAX_PHONE).collect::<String>()).filter(|p| !p.is_empty()).collect();
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
            account_id: None,
            notes_rich: None,
        }
        .validated()
        .unwrap();
        assert_eq!(c.name, "Sanne de Vries");
        assert_eq!(c.emails, vec!["sanne@example.org"]);
        assert_eq!(c.phones, vec!["06-12345678"]);
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
}
