//! The command registry.

use serde_json::Value;

use mailcraft_model::FolderKind;

use crate::Session;

/// One command: its id, the name the UI shows, and the function that runs it.
pub struct CommandSpec {
    pub id: &'static str,
    pub label: &'static str,
    run: fn(&mut Session, &Value) -> Result<(), CommandError>,
}

#[derive(Debug, PartialEq, Eq, thiserror::Error)]
pub enum CommandError {
    #[error("unknown command {0:?}")]
    Unknown(String),
    #[error("missing or invalid parameter {0:?}")]
    Param(&'static str),
    #[error("nothing is selected")]
    NoSelection,
    #[error("no such item")]
    NotFound,
}

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec { id: "folder.select", label: "Open folder", run: folder_select },
    CommandSpec { id: "message.select", label: "Open message", run: message_select },
    CommandSpec { id: "message.mark_read", label: "Mark as Read", run: |s, _| set_seen(s, true) },
    CommandSpec { id: "message.mark_unread", label: "Mark as Unread", run: |s, _| set_seen(s, false) },
    CommandSpec { id: "message.flag", label: "Flag", run: toggle_flag },
    CommandSpec { id: "message.delete", label: "Delete", run: |s, _| move_selected(s, FolderKind::Deleted) },
    CommandSpec { id: "message.archive", label: "Archive", run: |s, _| move_selected(s, FolderKind::Archive) },
    CommandSpec { id: "message.junk", label: "Junk", run: |s, _| move_selected(s, FolderKind::Junk) },
];

impl Session {
    /// Runs command `id` with `params` (a JSON object, or null when it takes none).
    pub fn run(&mut self, id: &str, params: &Value) -> Result<(), CommandError> {
        let spec = COMMANDS.iter().find(|c| c.id == id).ok_or_else(|| CommandError::Unknown(id.chars().take(64).collect()))?;
        (spec.run)(self, params)?;
        self.recount_unread();
        Ok(())
    }
}

fn str_param<'a>(params: &'a Value, key: &'static str) -> Result<&'a str, CommandError> {
    params.get(key).and_then(Value::as_str).ok_or(CommandError::Param(key))
}

fn folder_select(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id")?;
    s.folder(id).ok_or(CommandError::NotFound)?;
    s.selected_folder = Some(id.to_string());
    s.selected_message = None;
    Ok(())
}

fn message_select(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id")?;
    let m = s.messages.iter_mut().find(|m| m.id == id).ok_or(CommandError::NotFound)?;
    m.flags.seen = true;
    s.selected_message = Some(id.to_string());
    Ok(())
}

fn selected_mut(s: &mut Session) -> Result<&mut mailcraft_model::MessageHeader, CommandError> {
    let id = s.selected_message.clone().ok_or(CommandError::NoSelection)?;
    s.messages.iter_mut().find(|m| m.id == id).ok_or(CommandError::NoSelection)
}

fn set_seen(s: &mut Session, seen: bool) -> Result<(), CommandError> {
    selected_mut(s)?.flags.seen = seen;
    Ok(())
}

fn toggle_flag(s: &mut Session, _: &Value) -> Result<(), CommandError> {
    let m = selected_mut(s)?;
    m.flags.flagged = !m.flags.flagged;
    Ok(())
}

fn move_selected(s: &mut Session, kind: FolderKind) -> Result<(), CommandError> {
    let m = s.selected().ok_or(CommandError::NoSelection)?;
    let account = s.folder(&m.folder_id).map(|f| f.account_id.clone()).ok_or(CommandError::NotFound)?;
    let target = s.folder_of_kind(&account, kind).map(|f| f.id.clone()).ok_or(CommandError::NotFound)?;
    selected_mut(s)?.folder_id = target;
    s.selected_message = None;
    Ok(())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn opening_a_message_marks_it_read() {
        let mut s = Session::sample();
        let inbox_unread = |s: &Session| s.folder(s.selected_folder.as_deref().unwrap()).unwrap().unread;
        let before = inbox_unread(&s);
        let id = s.visible_messages().iter().find(|m| !m.flags.seen).unwrap().id.clone();
        s.run("message.select", &json!({ "id": id })).unwrap();
        assert_eq!(inbox_unread(&s), before - 1);
    }

    #[test]
    fn delete_moves_to_deleted_items() {
        let mut s = Session::sample();
        let id = s.visible_messages()[0].id.clone();
        s.run("message.select", &json!({ "id": id })).unwrap();
        s.run("message.delete", &Value::Null).unwrap();
        let folder = s.message(&id).unwrap().folder_id.clone();
        assert_eq!(s.folder(&folder).unwrap().kind, FolderKind::Deleted);
        assert!(s.selected_message.is_none());
    }

    #[test]
    fn bad_input_is_an_error_not_a_panic() {
        let mut s = Session::sample();
        assert!(matches!(s.run("nope", &Value::Null), Err(CommandError::Unknown(_))));
        assert_eq!(s.run("message.flag", &Value::Null), Err(CommandError::NoSelection));
        assert_eq!(s.run("folder.select", &json!({ "id": 7 })), Err(CommandError::Param("id")));
        assert_eq!(s.run("message.select", &json!({ "id": "missing" })), Err(CommandError::NotFound));
        for c in COMMANDS {
            for p in [Value::Null, json!({}), json!({ "id": "" }), json!([1, 2])] {
                let _ = s.run(c.id, &p);
            }
        }
    }
}
