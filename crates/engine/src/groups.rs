//! Contact groups (distribution lists), kept in `contact_groups.json` next to the accounts
//! file. A group is a name for a set of addresses; the compose window offers its members when
//! the name is typed, and the contacts module lists, makes and changes groups.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use merel_model::{ContactGroup, ContactGroupError, parse_address_list};

use crate::Session;
use crate::commands::{CommandError, str_param};

/// What `contact_groups.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupsFile {
    pub groups: Vec<ContactGroup>,
}

impl Session {
    pub fn set_groups(&mut self, groups: Vec<ContactGroup>) {
        self.groups = groups;
    }

    pub fn groups_data(&self) -> GroupsFile {
        GroupsFile { groups: self.groups.clone() }
    }

    /// Whether a command changed the groups since the last call.
    pub fn take_groups_dirty(&mut self) -> bool {
        std::mem::take(&mut self.groups_dirty)
    }

    pub fn group(&self, id: &str) -> Option<&ContactGroup> {
        self.groups.iter().find(|g| g.id == id)
    }

    /// Every group, by name.
    pub fn groups_sorted(&self) -> Vec<&ContactGroup> {
        let mut list: Vec<&ContactGroup> = self.groups.iter().collect();
        list.sort_by_cached_key(|g| g.name.to_lowercase());
        list
    }

    /// The groups whose name starts like `prefix`, by name; for completing an address field.
    pub fn groups_matching(&self, prefix: &str) -> Vec<&ContactGroup> {
        self.groups_sorted().into_iter().filter(|g| g.matches_prefix(prefix)).collect()
    }

    fn new_group_id(&mut self) -> String {
        self.next_group_id += 1;
        let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        format!("g-{now}-{}", self.next_group_id)
    }
}

/// `group.save { id?, name, members }`: creates a group or replaces the one with `id`. `members`
/// is an address line as a person types it (`Naam <a@b.nl>; c@d.nl`, line breaks allowed) or a
/// list of such strings. A name that cannot be read is `Param("members")`.
pub fn save(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let existing = match str_param(params, "id") {
        Ok(id) => Some(s.group(id).ok_or(CommandError::NotFound)?.clone()),
        Err(_) => None,
    };
    let text = match params.get("members") {
        Some(Value::String(t)) => t.clone(),
        Some(Value::Array(items)) => items.iter().map(Value::as_str).collect::<Option<Vec<_>>>().ok_or(CommandError::Param("members"))?.join("; "),
        _ => return Err(CommandError::Param("members")),
    };
    let members = parse_address_list(&text).map_err(|_| CommandError::Param("members"))?;
    let group = ContactGroup {
        id: existing.as_ref().map(|g| g.id.clone()).unwrap_or_else(|| s.new_group_id()),
        name: str_param(params, "name").unwrap_or("").to_string(),
        members,
    }
    .validated()
    .map_err(|e| match e {
        ContactGroupError::NoName => CommandError::Param("name"),
        ContactGroupError::NoMembers | ContactGroupError::TooMany => CommandError::Param("members"),
    })?;
    let id = group.id.clone();
    match s.groups.iter_mut().find(|g| g.id == id) {
        Some(slot) => *slot = group,
        None => s.groups.push(group),
    }
    s.groups_dirty = true;
    Ok(())
}

/// `group.delete { id }`: removes the group; the contacts stay.
pub fn delete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id")?;
    let before = s.groups.len();
    s.groups.retain(|g| g.id != id);
    if s.groups.len() == before {
        return Err(CommandError::NotFound);
    }
    s.groups_dirty = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn a_group_is_saved_offered_while_typing_changed_and_removed() {
        let mut s = Session::default();
        s.run("group.save", &json!({ "name": "Team ontwerp", "members": "Sanne <sanne@example.org>\nravi@example.org; sanne@example.org" })).unwrap();
        assert!(s.take_groups_dirty() && !s.take_groups_dirty());
        let g = s.groups_sorted()[0].clone();
        assert_eq!((g.name.as_str(), g.members.len()), ("Team ontwerp", 2));
        assert_eq!(s.groups_matching("ont").len(), 1);
        assert!(s.groups_matching("sanne").is_empty(), "members do not match, names do");
        s.run("group.save", &json!({ "id": g.id, "name": "Team", "members": ["ravi@example.org"] })).unwrap();
        assert_eq!(s.groups.len(), 1);
        assert_eq!((s.group(&g.id).unwrap().name.as_str(), s.group(&g.id).unwrap().members.len()), ("Team", 1));
        assert_eq!(s.run("group.save", &json!({ "name": "Leeg", "members": "" })), Err(CommandError::Param("members")));
        assert_eq!(s.run("group.save", &json!({ "name": "Fout", "members": "sanne@" })), Err(CommandError::Param("members")));
        assert_eq!(s.run("group.save", &json!({ "name": " ", "members": "a@b.nl" })), Err(CommandError::Param("name")));
        assert_eq!(s.run("group.save", &json!({ "id": "nope", "name": "x", "members": "a@b.nl" })), Err(CommandError::NotFound));
        s.run("group.delete", &json!({ "id": g.id })).unwrap();
        assert!(s.groups.is_empty() && s.take_groups_dirty());
        assert_eq!(s.run("group.delete", &json!({ "id": g.id })), Err(CommandError::NotFound));
    }
}
