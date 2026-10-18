//! Contacts in the session, the commands that change them, and address suggestions for the
//! compose window. Contacts live in `contacts.json` next to the accounts file until they sync.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use mailcraft_model::{Address, Contact, ContactError};

use crate::{CommandError, Session};

/// What `contacts.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactsFile {
    pub contacts: Vec<Contact>,
}

impl Session {
    pub fn set_contacts(&mut self, contacts: Vec<Contact>) {
        self.contacts = contacts;
        self.selected_contact = None;
    }

    pub fn contacts_data(&self) -> ContactsFile {
        ContactsFile { contacts: self.contacts.clone() }
    }

    pub fn contact(&self, id: &str) -> Option<&Contact> {
        self.contacts.iter().find(|c| c.id == id)
    }

    pub fn selected_contact(&self) -> Option<&Contact> {
        self.selected_contact.as_deref().and_then(|id| self.contact(id))
    }

    /// The contacts matching `query` (all of them when it is empty), by name.
    pub fn contacts_matching(&self, query: &str) -> Vec<&Contact> {
        let mut list: Vec<&Contact> = self.contacts.iter().filter(|c| c.matches(query)).collect();
        list.sort_by_cached_key(|c| c.name.to_lowercase());
        list
    }

    /// Addresses that start like `prefix`, for completing the To and CC fields: contacts
    /// first, then people who wrote or received the mail in the session. No more than `limit`.
    pub fn suggest_addresses(&self, prefix: &str, limit: usize) -> Vec<Address> {
        let q = prefix.trim().to_lowercase();
        if q.is_empty() {
            return Vec::new();
        }
        let hit = |a: &Address| a.email().to_lowercase().starts_with(&q) || a.display().to_lowercase().split_whitespace().any(|w| w.starts_with(&q));
        let mut out: Vec<Address> = Vec::new();
        let mut push = |a: Address| {
            if out.len() < limit && hit(&a) && !out.iter().any(|o| o.email().eq_ignore_ascii_case(a.email())) {
                out.push(a);
            }
        };
        for c in self.contacts_matching("") {
            for a in c.addresses() {
                push(a);
            }
        }
        for m in &self.messages {
            push(m.from.clone());
            for a in &m.to {
                push(a.clone());
            }
        }
        out
    }

    /// Whether a command changed the contacts since the last call.
    pub fn take_contacts_dirty(&mut self) -> bool {
        std::mem::take(&mut self.contacts_dirty)
    }

    fn new_contact_id(&mut self) -> String {
        self.next_contact_id += 1;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        format!("c-{now}-{}", self.next_contact_id)
    }
}

fn str_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
    params.get(key).and_then(Value::as_str)
}

fn list_param(params: &Value, key: &str) -> Vec<String> {
    params.get(key).and_then(Value::as_array).map(|a| a.iter().filter_map(Value::as_str).map(str::to_string).collect()).unwrap_or_default()
}

/// `contact.select { id }`.
pub fn select(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id").ok_or(CommandError::Param("id"))?;
    s.contact(id).ok_or(CommandError::NotFound)?;
    s.selected_contact = Some(id.to_string());
    Ok(())
}

/// `contact.save { id?, name, emails: [..], phones: [..], company?, notes? }`: creates the
/// contact, or replaces the one with `id`.
pub fn save(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = match str_param(params, "id") {
        Some(id) => s.contact(id).ok_or(CommandError::NotFound)?.id.clone(),
        None => s.new_contact_id(),
    };
    let contact = Contact {
        id: id.clone(),
        name: str_param(params, "name").unwrap_or("").to_string(),
        emails: list_param(params, "emails"),
        phones: list_param(params, "phones"),
        company: str_param(params, "company").unwrap_or("").to_string(),
        notes: str_param(params, "notes").unwrap_or("").to_string(),
    }
    .validated()
    .map_err(|e| match e {
        ContactError::NoName => CommandError::Param("name"),
        ContactError::BadEmail(_) => CommandError::Param("emails"),
    })?;
    s.contacts.retain(|c| c.id != id);
    s.contacts.push(contact);
    s.selected_contact = Some(id);
    s.contacts_dirty = true;
    Ok(())
}

/// `contact.delete { id? }`: the given contact, else the selected one.
pub fn delete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id").map(str::to_string).or_else(|| s.selected_contact.clone()).ok_or(CommandError::NoSelection)?;
    let before = s.contacts.len();
    s.contacts.retain(|c| c.id != id);
    if s.contacts.len() == before {
        return Err(CommandError::NotFound);
    }
    if s.selected_contact.as_deref() == Some(id.as_str()) {
        s.selected_contact = None;
    }
    s.contacts_dirty = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn contacts_are_saved_found_and_deleted() {
        let mut s = Session::sample();
        let n = s.contacts.len();
        s.run("contact.save", &json!({ "name": "Noor Bakker", "emails": ["Noor@Example.org"], "phones": ["06 1"], "company": "Bakkerij" })).unwrap();
        assert!(s.take_contacts_dirty());
        let c = s.selected_contact().unwrap().clone();
        assert_eq!((c.name.as_str(), c.emails[0].as_str()), ("Noor Bakker", "noor@example.org"));
        assert_eq!(s.contacts.len(), n + 1);
        assert_eq!(s.contacts_matching("bakker").len(), 1);
        assert_eq!(s.contacts_matching("").first().map(|c| c.name.as_str()), Some("Noor Bakker"));

        s.run("contact.save", &json!({ "id": c.id, "name": "Noor Bakker-Jansen", "emails": ["noor@example.org"] })).unwrap();
        assert_eq!(s.contacts.len(), n + 1);
        assert_eq!(s.contact(&c.id).unwrap().name, "Noor Bakker-Jansen");

        assert_eq!(s.run("contact.save", &json!({ "name": "", "emails": [] })), Err(CommandError::Param("name")));
        assert_eq!(s.run("contact.save", &json!({ "name": "x", "emails": ["nope"] })), Err(CommandError::Param("emails")));
        assert_eq!(s.run("contact.select", &json!({ "id": "nope" })), Err(CommandError::NotFound));

        s.run("contact.delete", &Value::Null).unwrap();
        assert_eq!(s.contacts.len(), n);
        assert!(s.selected_contact.is_none());
    }

    #[test]
    fn suggestions_come_from_contacts_then_mail() {
        let s = Session::sample();
        let names: Vec<String> = s.suggest_addresses("sa", 5).iter().map(|a| a.display().to_string()).collect();
        assert_eq!(names, vec!["Sanne de Vries"]);
        assert!(s.suggest_addresses("", 5).is_empty());
        // "jij@example.org" is only known from the sample mail's recipients.
        assert_eq!(s.suggest_addresses("jij@", 5).len(), 1);
        assert!(s.suggest_addresses("example", 5).is_empty(), "matching is on the start of a word or address");
        assert_eq!(s.suggest_addresses("s", 1).len(), 1);
    }
}
