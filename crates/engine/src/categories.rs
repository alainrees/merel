//! Categories, the coloured labels on mail: the person's list of them (`categories.json`,
//! starting as Outlook's six) and the commands that put them on messages. The names travel to
//! the server with `Job::Categories`; the colours stay on this computer.

use merel_model::{Category, CategoryColor, MAX_CATEGORY_NAME, clean_text};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::commands::{CommandError, str_param};
use crate::{Job, SAMPLE_ACCOUNT, Session};

/// What `categories.json` holds. A missing file gives Outlook's six; an emptied list stays empty.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CategoriesFile {
    pub categories: Vec<Category>,
}

impl Default for CategoriesFile {
    fn default() -> Self {
        CategoriesFile { categories: Category::defaults() }
    }
}

impl Session {
    pub fn set_categories(&mut self, categories: Vec<Category>) {
        self.categories = categories;
    }

    pub fn categories_data(&self) -> CategoriesFile {
        CategoriesFile { categories: self.categories.clone() }
    }

    pub fn take_categories_dirty(&mut self) -> bool {
        std::mem::take(&mut self.categories_dirty)
    }

    /// The colour of a category by name; grey for one this computer does not know (it came
    /// from the server or another client).
    pub fn category_color(&self, name: &str) -> CategoryColor {
        self.categories.iter().find(|c| c.name.eq_ignore_ascii_case(name)).map(|c| c.color).unwrap_or(CategoryColor::Gray)
    }
}

/// The message `id` names, else the selected one.
fn target(s: &Session, params: &Value) -> Result<String, CommandError> {
    params.get("id").and_then(Value::as_str).map(str::to_string).or_else(|| s.selected_message.clone()).ok_or(CommandError::NoSelection)
}

/// Changes a message's categories in the session and hands the server the new set.
pub(crate) fn set_message_categories(s: &mut Session, message_id: &str, categories: Vec<String>, removed: Vec<String>) -> Result<(), CommandError> {
    let m = s.message(message_id).cloned().ok_or(CommandError::NotFound)?;
    let folder = s.folder(&m.folder_id).cloned().ok_or(CommandError::NotFound)?;
    if folder.account_id != SAMPLE_ACCOUNT {
        s.pending.push(Job::Categories {
            account_id: folder.account_id,
            path: folder.path,
            id: m.remote_id,
            categories: categories.clone(),
            removed,
        });
    }
    if let Some(h) = s.messages.iter_mut().find(|h| h.id == message_id) {
        h.categories = categories;
    }
    Ok(())
}

/// `message.categorize { name, id? }`: puts the category on the message, or takes it off when
/// it is there. The name must be a known category, or one the message carries already.
pub fn categorize(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let name = clean_text(str_param(params, "name")?).trim().to_string();
    let id = target(s, params)?;
    let m = s.message(&id).cloned().ok_or(CommandError::NotFound)?;
    let known = s.categories.iter().find(|c| c.name.eq_ignore_ascii_case(&name)).map(|c| c.name.clone());
    let carried = m.categories.iter().find(|c| c.eq_ignore_ascii_case(&name)).cloned();
    let name = carried.clone().or(known).ok_or(CommandError::Param("name"))?;
    let mut categories = m.categories.clone();
    let removed = match carried {
        Some(c) => {
            categories.retain(|x| x != &c);
            vec![c]
        }
        None => {
            categories.push(name);
            Vec::new()
        }
    };
    set_message_categories(s, &id, categories, removed)
}

/// `message.clear_categories { id? }`: every category off the message.
pub fn clear(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = target(s, params)?;
    let removed = s.message(&id).ok_or(CommandError::NotFound)?.categories.clone();
    if removed.is_empty() {
        return Ok(());
    }
    set_message_categories(s, &id, Vec::new(), removed)
}

/// `category.save { name, color }`: a new category, or a new colour for one that exists.
pub fn save(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let name = clean_text(str_param(params, "name")?).trim().to_string();
    if name.is_empty() || name.chars().count() > MAX_CATEGORY_NAME {
        return Err(CommandError::Param("name"));
    }
    let color: CategoryColor =
        serde_json::from_value(params.get("color").cloned().unwrap_or(Value::Null)).map_err(|_| CommandError::Param("color"))?;
    match s.categories.iter_mut().find(|c| c.name.eq_ignore_ascii_case(&name)) {
        Some(c) => c.color = color,
        None => s.categories.push(Category { name, color }),
    }
    s.categories_dirty = true;
    Ok(())
}

/// `category.delete { name }`: the category leaves the list; messages that carry the name keep
/// it (in grey), as Outlook has it.
pub fn delete(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let name = str_param(params, "name")?;
    let before = s.categories.len();
    s.categories.retain(|c| !c.name.eq_ignore_ascii_case(name));
    if s.categories.len() == before {
        return Err(CommandError::NotFound);
    }
    s.categories_dirty = true;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn categories_go_on_and_off_a_message_and_the_list_is_kept() {
        let mut s = Session::sample();
        assert_eq!(s.categories.len(), 6);
        s.run("message.select", &json!({ "id": "m0" })).expect("select");
        s.run("message.categorize", &json!({ "name": "Rode categorie" })).expect("on");
        s.run("message.categorize", &json!({ "name": "blauwe categorie" })).expect("on, whatever the case");
        assert_eq!(s.message("m0").map(|m| m.categories.clone()), Some(vec!["Rode categorie".to_string(), "Blauwe categorie".to_string()]));
        assert!(matches!(s.run("message.categorize", &json!({ "name": "Onbekend" })), Err(CommandError::Param("name"))));
        s.run("message.categorize", &json!({ "name": "Rode categorie" })).expect("off");
        assert_eq!(s.message("m0").map(|m| m.categories.clone()), Some(vec!["Blauwe categorie".to_string()]));
        s.run("message.clear_categories", &json!({})).expect("clear");
        assert!(s.message("m0").is_some_and(|m| m.categories.is_empty()));
        assert_eq!(s.message("m1").map(|m| m.categories.len()), Some(1), "the sample has one");

        s.run("category.save", &json!({ "name": " Klanten ", "color": "purple" })).expect("new");
        assert_eq!(s.categories.last().map(|c| (c.name.as_str(), c.color)), Some(("Klanten", CategoryColor::Purple)));
        s.run("category.save", &json!({ "name": "klanten", "color": "gray" })).expect("recolour");
        assert_eq!(s.categories.len(), 7);
        assert_eq!(s.category_color("Klanten"), CategoryColor::Gray);
        assert!(matches!(s.run("category.save", &json!({ "name": "X", "color": "pink" })), Err(CommandError::Param("color"))));
        s.run("category.delete", &json!({ "name": "Klanten" })).expect("delete");
        assert_eq!(s.categories.len(), 6);
        assert!(s.take_categories_dirty());
        assert!(s.take_jobs().is_empty(), "the sample account has no server");
    }

    #[test]
    fn a_real_account_gets_the_whole_new_set_and_what_came_off() {
        use merel_mailbox::RemoteFolder;
        use merel_model::FolderKind;

        let mut s = Session::with_accounts(vec![crate::tests::account("acc")]);
        let folders = vec![RemoteFolder { path: "INBOX".into(), name: "INBOX".into(), kind: FolderKind::Inbox, selectable: true }];
        s.apply(crate::Event::Connected { account_id: "acc".into(), folders, syncs_items: false });
        let mut h = crate::tests::header(7);
        h.categories = vec!["Werk".into()];
        s.apply(crate::Event::Headers { folder_id: "acc/INBOX".into(), headers: vec![h], offset: 0, limit: crate::HEADER_LIMIT });
        s.take_jobs();
        assert_eq!(s.category_color("Werk"), CategoryColor::Gray, "a server category this computer does not know");
        s.run("message.categorize", &json!({ "id": "acc/INBOX#7", "name": "Rode categorie" })).expect("on");
        s.run("message.categorize", &json!({ "id": "acc/INBOX#7", "name": "Werk" })).expect("off");
        let jobs = s.take_jobs();
        assert_eq!(
            jobs,
            vec![
                Job::Categories {
                    account_id: "acc".into(),
                    path: "INBOX".into(),
                    id: "7".into(),
                    categories: vec!["Werk".into(), "Rode categorie".into()],
                    removed: vec![]
                },
                Job::Categories {
                    account_id: "acc".into(),
                    path: "INBOX".into(),
                    id: "7".into(),
                    categories: vec!["Rode categorie".into()],
                    removed: vec!["Werk".into()]
                },
            ]
        );
        assert!(jobs[0].is_change());
    }
}
