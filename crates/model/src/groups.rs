//! Contact groups, as Outlook's "Contactgroep" (a distribution list): a name for a set of
//! addresses, so that a message to the whole team is one name typed. Groups live on this
//! computer only; the contacts themselves stay where they are.

use serde::{Deserialize, Serialize};

use crate::{Address, clean_text};

/// The longest name a group may have.
pub const MAX_GROUP_NAME: usize = 60;
/// The most members a group may have.
pub const MAX_GROUP_MEMBERS: usize = 200;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContactGroup {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub members: Vec<Address>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum ContactGroupError {
    #[error("a group needs a name")]
    NoName,
    #[error("a group needs at least one member")]
    NoMembers,
    #[error("a group holds at most {MAX_GROUP_MEMBERS} members")]
    TooMany,
}

impl ContactGroup {
    /// Cleans the name, drops members with the same address and checks the group has a name
    /// and at least one member.
    pub fn validated(mut self) -> Result<Self, ContactGroupError> {
        self.name = clean_text(&self.name).trim().chars().take(MAX_GROUP_NAME).collect();
        if self.name.is_empty() {
            return Err(ContactGroupError::NoName);
        }
        let mut members: Vec<Address> = Vec::new();
        for m in self.members {
            if !members.iter().any(|x| x.email().eq_ignore_ascii_case(m.email())) {
                members.push(m);
            }
        }
        if members.is_empty() {
            return Err(ContactGroupError::NoMembers);
        }
        if members.len() > MAX_GROUP_MEMBERS {
            return Err(ContactGroupError::TooMany);
        }
        self.members = members;
        Ok(self)
    }

    /// Whether the name, or a word of it, starts with `prefix`; for completing an address field.
    pub fn matches_prefix(&self, prefix: &str) -> bool {
        let q = prefix.trim().to_lowercase();
        if q.is_empty() {
            return false;
        }
        let name = self.name.to_lowercase();
        name.starts_with(&q) || name.split_whitespace().any(|w| w.starts_with(&q))
    }

    /// The members as one address line: `Naam <adres>; adres`.
    pub fn address_line(&self) -> String {
        self.members.iter().map(mailbox).collect::<Vec<_>>().join("; ")
    }
}

/// `Naam <adres>`, or the bare address when there is no name.
pub fn mailbox(a: &Address) -> String {
    if a.display() == a.email() { a.email().to_string() } else { format!("{} <{}>", a.display(), a.email()) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_group_needs_a_name_and_members_and_drops_doubles() {
        let sanne = Address::new(Some("Sanne"), "sanne@example.org").unwrap();
        let again = Address::new(None, "SANNE@example.org").unwrap();
        let ravi = Address::new(None, "ravi@example.org").unwrap();
        let g = ContactGroup { id: "g1".into(), name: " Team ontwerp \u{7}".into(), members: vec![sanne.clone(), again, ravi.clone()] };
        let ok = g.clone().validated().unwrap();
        assert_eq!(ok.name, "Team ontwerp");
        assert_eq!(ok.members, vec![sanne, ravi], "the same address twice is kept once");
        assert_eq!(ok.address_line(), "Sanne <sanne@example.org>; ravi@example.org");
        assert!(ok.matches_prefix("te") && ok.matches_prefix("Ontw") && !ok.matches_prefix("x") && !ok.matches_prefix(" "));
        assert_eq!(ContactGroup { name: "  ".into(), ..g.clone() }.validated(), Err(ContactGroupError::NoName));
        assert_eq!(ContactGroup { members: Vec::new(), ..g.clone() }.validated(), Err(ContactGroupError::NoMembers));
        let many: Vec<Address> = (0..=MAX_GROUP_MEMBERS).map(|i| Address::new(None, &format!("p{i}@example.org")).unwrap()).collect();
        assert_eq!(ContactGroup { members: many, ..g }.validated(), Err(ContactGroupError::TooMany));
    }
}
