//! The command registry.

use serde_json::Value;

use mailcraft_model::FolderKind;

use crate::{Job, Session};

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
    #[error("this account has no {0} folder")]
    NoFolder(&'static str),
}

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec { id: "folder.select", label: "Open folder", run: folder_select },
    CommandSpec { id: "folder.refresh", label: "Refresh folder", run: folder_refresh },
    CommandSpec { id: "message.select", label: "Open message", run: message_select },
    CommandSpec { id: "message.mark_read", label: "Mark as Read", run: |s, _| set_seen(s, true) },
    CommandSpec { id: "message.mark_unread", label: "Mark as Unread", run: |s, _| set_seen(s, false) },
    CommandSpec { id: "message.flag", label: "Flag", run: toggle_flag },
    CommandSpec { id: "message.delete", label: "Delete", run: |s, _| move_selected(s, FolderKind::Deleted, "Deleted Items") },
    CommandSpec { id: "message.archive", label: "Archive", run: |s, _| move_selected(s, FolderKind::Archive, "Archive") },
    CommandSpec { id: "message.junk", label: "Junk", run: |s, _| move_selected(s, FolderKind::Junk, "Junk") },
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
    let id = str_param(params, "id")?.to_string();
    s.folder(&id).ok_or(CommandError::NotFound)?;
    s.selected_folder = Some(id.clone());
    s.selected_message = None;
    let loaded = s.messages.iter().any(|m| m.folder_id == id);
    if !loaded {
        s.request_headers(&id);
    }
    Ok(())
}

fn folder_refresh(s: &mut Session, _: &Value) -> Result<(), CommandError> {
    let id = s.selected_folder.clone().ok_or(CommandError::NoSelection)?;
    s.request_headers(&id);
    Ok(())
}

fn message_select(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id")?.to_string();
    s.message(&id).ok_or(CommandError::NotFound)?;
    s.selected_message = Some(id.clone());
    s.request_body(&id);
    set_seen(s, true)
}

fn selected_mut(s: &mut Session) -> Result<&mut mailcraft_model::MessageHeader, CommandError> {
    let id = s.selected_message.clone().ok_or(CommandError::NoSelection)?;
    s.messages.iter_mut().find(|m| m.id == id).ok_or(CommandError::NoSelection)
}

/// The account id, mailbox path and UID of the selected message, for a remote job.
fn remote(s: &Session) -> Option<(String, String, u32)> {
    let m = s.selected()?;
    let f = s.folder(&m.folder_id)?;
    (f.account_id != crate::SAMPLE_ACCOUNT).then(|| (f.account_id.clone(), f.path.clone(), m.uid))
}

fn set_flag(s: &mut Session, flag: &'static str, on: bool) {
    if let Some((account_id, path, uid)) = remote(s) {
        s.pending.push(Job::Flag { account_id, path, uid, flag, on });
    }
}

fn set_seen(s: &mut Session, seen: bool) -> Result<(), CommandError> {
    let m = selected_mut(s)?;
    if m.flags.seen == seen {
        return Ok(());
    }
    m.flags.seen = seen;
    set_flag(s, "\\Seen", seen);
    Ok(())
}

fn toggle_flag(s: &mut Session, _: &Value) -> Result<(), CommandError> {
    let m = selected_mut(s)?;
    m.flags.flagged = !m.flags.flagged;
    let on = m.flags.flagged;
    set_flag(s, "\\Flagged", on);
    Ok(())
}

fn move_selected(s: &mut Session, kind: FolderKind, what: &'static str) -> Result<(), CommandError> {
    let m = s.selected().ok_or(CommandError::NoSelection)?;
    let account = s.folder(&m.folder_id).map(|f| f.account_id.clone()).ok_or(CommandError::NotFound)?;
    let target = s.folder_of_kind(&account, kind).cloned().ok_or(CommandError::NoFolder(what))?;
    if let Some((account_id, path, uid)) = remote(s) {
        s.pending.push(Job::Move { account_id, path, uid, dest: target.path.clone() });
    }
    selected_mut(s)?.folder_id = target.id;
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
        assert!(s.take_jobs().is_empty(), "sample mail never talks to a server");
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

    #[test]
    fn real_accounts_leave_jobs_behind() {
        use mailcraft_imap::{RemoteFolder, RemoteHeader};
        use mailcraft_model::{Account, Address, Protocol, Server, Tls};
        let account = Account {
            id: "acc".into(),
            name: "Acc".into(),
            address: Address::new(None, "a@b.nl").unwrap(),
            username: "a@b.nl".into(),
            protocol: Protocol::ImapSmtp,
            incoming: Some(Server { host: "imap.b.nl".into(), port: 993, tls: Tls::Implicit }),
            outgoing: None,
        };
        let mut s = Session::with_accounts(vec![account]);
        assert_eq!(s.take_jobs(), vec![Job::Connect { account_id: "acc".into() }]);
        s.apply(crate::Event::Connected {
            account_id: "acc".into(),
            folders: vec![
                RemoteFolder { name: "Trash".into(), kind: FolderKind::Deleted, selectable: true },
                RemoteFolder { name: "INBOX".into(), kind: FolderKind::Inbox, selectable: true },
                RemoteFolder { name: "[Gmail]".into(), kind: FolderKind::Other, selectable: false },
            ],
        });
        assert_eq!(s.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Postvak IN", "Verwijderde items"]);
        assert_eq!(s.selected_folder.as_deref(), Some("acc/INBOX"));
        assert_eq!(s.take_jobs(), vec![Job::Headers { account_id: "acc".into(), folder_id: "acc/INBOX".into(), path: "INBOX".into() }]);
        let header = mailcraft_mime::parse_header(b"From: x@y.nl\r\nSubject: Hoi\r\n\r\n");
        s.apply(crate::Event::Headers {
            folder_id: "acc/INBOX".into(),
            headers: vec![RemoteHeader { uid: 7, seen: false, flagged: false, answered: false, size: 10, has_attachments: false, header, date: 5 }],
        });
        assert_eq!(s.folder("acc/INBOX").unwrap().unread, 1);
        s.run("message.select", &json!({ "id": "acc/INBOX#7" })).unwrap();
        let jobs = s.take_jobs();
        assert!(jobs.contains(&Job::Body { account_id: "acc".into(), message_id: "acc/INBOX#7".into(), path: "INBOX".into(), uid: 7 }));
        assert!(jobs.contains(&Job::Flag { account_id: "acc".into(), path: "INBOX".into(), uid: 7, flag: "\\Seen", on: true }));
        s.run("message.delete", &Value::Null).unwrap();
        assert_eq!(s.take_jobs(), vec![Job::Move { account_id: "acc".into(), path: "INBOX".into(), uid: 7, dest: "Trash".into() }]);
        assert_eq!(s.message("acc/INBOX#7").unwrap().folder_id, "acc/Trash");
    }
}
