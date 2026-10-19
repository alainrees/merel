//! The command registry.

use serde_json::Value;

use mailcraft_mailbox::Flag;
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
    #[error("could not save: {0}")]
    Store(String),
}

pub static COMMANDS: &[CommandSpec] = &[
    CommandSpec { id: "folder.select", label: "Open folder", run: folder_select },
    CommandSpec { id: "folder.refresh", label: "Refresh folder", run: folder_refresh },
    CommandSpec { id: "folder.older", label: "Load older messages", run: folder_older },
    CommandSpec { id: "folder.create", label: "New folder", run: crate::folders::create },
    CommandSpec { id: "folder.rename", label: "Rename folder", run: crate::folders::rename },
    CommandSpec { id: "folder.delete", label: "Delete folder", run: crate::folders::delete },
    CommandSpec { id: "folder.empty", label: "Empty folder", run: crate::folders::empty },
    CommandSpec { id: "folder.mark_all_read", label: "Mark all as read", run: crate::folders::mark_all_read },
    CommandSpec { id: "message.categorize", label: "Categorize", run: crate::categories::categorize },
    CommandSpec { id: "message.clear_categories", label: "Clear all categories", run: crate::categories::clear },
    CommandSpec { id: "category.save", label: "Save category", run: crate::categories::save },
    CommandSpec { id: "category.delete", label: "Delete category", run: crate::categories::delete },
    CommandSpec { id: "view.filter", label: "Show only…", run: crate::view::filter },
    CommandSpec { id: "view.sort", label: "Sort by…", run: crate::view::sort },
    CommandSpec { id: "view.category", label: "Show category…", run: crate::view::category },
    CommandSpec {
        id: "search.server",
        label: "Search on the server",
        run: |s, _| {
            s.request_server_search();
            Ok(())
        },
    },
    CommandSpec { id: "message.select", label: "Open message", run: message_select },
    CommandSpec { id: "message.select_more", label: "Select this message too", run: message_select_more },
    CommandSpec { id: "message.select_many", label: "Select these messages", run: message_select_many },
    CommandSpec { id: "message.mark_read", label: "Mark as Read", run: |s, _| set_seen(s, true) },
    CommandSpec { id: "message.mark_unread", label: "Mark as Unread", run: |s, _| set_seen(s, false) },
    CommandSpec { id: "message.flag", label: "Flag", run: toggle_flag },
    CommandSpec { id: "message.follow_up", label: "Follow up", run: crate::followup::follow_up },
    CommandSpec { id: "message.move", label: "Move to folder", run: message_move },
    CommandSpec { id: "message.delete", label: "Delete", run: message_delete },
    CommandSpec { id: "message.archive", label: "Archive", run: |s, _| move_selected(s, FolderKind::Archive, "Archive") },
    CommandSpec { id: "message.junk", label: "Junk", run: message_junk },
    CommandSpec { id: "junk.block", label: "Block sender", run: crate::junk::block },
    CommandSpec { id: "junk.trust", label: "Trust sender", run: crate::junk::trust },
    CommandSpec { id: "junk.forget", label: "Forget sender", run: crate::junk::forget },
    CommandSpec { id: "junk.level", label: "Junk filter level", run: crate::junk::level },
    CommandSpec { id: "conversation.ignore", label: "Ignore conversation", run: crate::ignore::ignore },
    CommandSpec { id: "conversation.unignore", label: "Stop ignoring conversation", run: crate::ignore::unignore },
    CommandSpec { id: "appointment.select", label: "Open appointment", run: crate::calendar::select },
    CommandSpec { id: "appointment.save", label: "Save appointment", run: crate::calendar::save },
    CommandSpec { id: "appointment.skip", label: "Skip one occurrence", run: crate::calendar::skip },
    CommandSpec { id: "appointment.move", label: "Move appointment", run: crate::calendar::move_to },
    CommandSpec { id: "appointment.resize", label: "Change appointment end", run: crate::calendar::resize },
    CommandSpec { id: "rule.save", label: "Save rule", run: crate::rules::save },
    CommandSpec { id: "rule.delete", label: "Delete rule", run: crate::rules::delete },
    CommandSpec { id: "rule.run", label: "Run rules now", run: crate::rules::run },
    CommandSpec { id: "quickstep.save", label: "Save quick step", run: crate::quicksteps::save },
    CommandSpec { id: "quickstep.delete", label: "Delete quick step", run: crate::quicksteps::delete },
    CommandSpec { id: "quickstep.run", label: "Run quick step", run: crate::quicksteps::run },
    CommandSpec { id: "reminder.dismiss", label: "Close a reminder", run: crate::reminders::dismiss },
    CommandSpec { id: "reminder.snooze", label: "Put a reminder off", run: crate::reminders::snooze },
    CommandSpec { id: "appointment.delete", label: "Delete appointment", run: crate::calendar::delete },
    CommandSpec { id: "contact.select", label: "Open contact", run: crate::contacts::select },
    CommandSpec { id: "contact.save", label: "Save contact", run: crate::contacts::save },
    CommandSpec { id: "contact.delete", label: "Delete contact", run: crate::contacts::delete },
    CommandSpec { id: "task.select", label: "Open task", run: crate::tasks::select },
    CommandSpec { id: "task.save", label: "Save task", run: crate::tasks::save },
    CommandSpec { id: "task.complete", label: "Complete task", run: crate::tasks::complete },
    CommandSpec { id: "task.delete", label: "Delete task", run: crate::tasks::delete },
    CommandSpec { id: "sync.refresh", label: "Refresh calendar, contacts and tasks", run: crate::sync::refresh },
    CommandSpec { id: "draft.save", label: "Save draft", run: crate::drafts::save },
    CommandSpec { id: "draft.delete", label: "Delete draft", run: crate::drafts::delete },
    CommandSpec { id: "draft.schedule", label: "Send draft later", run: crate::drafts::schedule },
];

impl Session {
    /// Runs command `id` with `params` (a JSON object, or null when it takes none).
    pub fn run(&mut self, id: &str, params: &Value) -> Result<(), CommandError> {
        let spec = COMMANDS.iter().find(|c| c.id == id).ok_or_else(|| CommandError::Unknown(id.chars().take(64).collect()))?;
        let result = (spec.run)(self, params);
        self.prune_marked();
        self.recount_unread();
        result
    }
}

pub(crate) fn str_param<'a>(params: &'a Value, key: &'static str) -> Result<&'a str, CommandError> {
    params.get(key).and_then(Value::as_str).ok_or(CommandError::Param(key))
}

fn folder_select(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id")?.to_string();
    s.folder(&id).ok_or(CommandError::NotFound)?;
    s.selected_folder = Some(id.clone());
    s.selected_message = None;
    s.marked.clear();
    let loaded = s.messages.iter().any(|m| m.folder_id == id);
    if !loaded {
        s.request_headers(&id);
    }
    Ok(())
}

/// `folder.older`: the page of mail below what the selected folder lists.
fn folder_older(s: &mut Session, _: &Value) -> Result<(), CommandError> {
    let id = s.selected_folder.clone().ok_or(CommandError::NoSelection)?;
    s.request_older(&id);
    Ok(())
}

fn folder_refresh(s: &mut Session, _: &Value) -> Result<(), CommandError> {
    let id = s.selected_folder.clone().ok_or(CommandError::NoSelection)?;
    s.request_headers(&id);
    Ok(())
}

/// `message.select { id }`: the one selected message; whatever was marked with the previous
/// one is let go.
fn message_select(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id")?.to_string();
    s.message(&id).ok_or(CommandError::NotFound)?;
    s.marked.clear();
    s.selected_message = Some(id.clone());
    s.request_body(&id);
    set_seen_one(s, &id, true)
}

/// `message.select_more { id }` (Ctrl-click): the message joins the selection, or leaves it
/// when it was in it. Without a selected message it simply becomes the selected one.
fn message_select_more(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = str_param(params, "id")?.to_string();
    s.message(&id).ok_or(CommandError::NotFound)?;
    if s.selected_message.is_none() {
        return message_select(s, params);
    }
    if s.selected_message.as_deref() == Some(id.as_str()) {
        // The selected one leaves: the first marked message takes its place, if there is one.
        if !s.marked.is_empty() {
            let next = s.marked.remove(0);
            s.selected_message = Some(next.clone());
            s.request_body(&next);
        }
    } else if let Some(i) = s.marked.iter().position(|m| *m == id) {
        s.marked.remove(i);
    } else {
        s.marked.push(id);
    }
    Ok(())
}

/// `message.select_many { ids }` (Shift-click, Ctrl+A): exactly these messages are selected;
/// the selected message stays selected when it is among them, else the first one takes over.
fn message_select_many(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let ids: Vec<String> = params
        .get("ids")
        .and_then(Value::as_array)
        .ok_or(CommandError::Param("ids"))?
        .iter()
        .map(|v| v.as_str().map(str::to_string).ok_or(CommandError::Param("ids")))
        .collect::<Result<_, _>>()?;
    if ids.iter().any(|id| s.message(id).is_none()) {
        return Err(CommandError::NotFound);
    }
    let Some(first) = ids.first().cloned() else { return Err(CommandError::Param("ids")) };
    if !s.selected_message.as_ref().is_some_and(|sel| ids.contains(sel)) {
        s.selected_message = Some(first.clone());
        s.request_body(&first);
        set_seen_one(s, &first, true)?;
    }
    s.marked.clear();
    for id in ids {
        if s.selected_message.as_deref() != Some(id.as_str()) && !s.marked.contains(&id) {
            s.marked.push(id);
        }
    }
    Ok(())
}

/// The selected message and the ones marked with it, or `NoSelection`.
fn selection(s: &Session) -> Result<Vec<String>, CommandError> {
    let ids = s.selection();
    if ids.is_empty() { Err(CommandError::NoSelection) } else { Ok(ids) }
}

fn header_mut<'a>(s: &'a mut Session, id: &str) -> Result<&'a mut mailcraft_model::MessageHeader, CommandError> {
    s.messages.iter_mut().find(|m| m.id == id).ok_or(CommandError::NoSelection)
}

/// The account id, folder path and server id of a message, for a remote job.
fn remote(s: &Session, message_id: &str) -> Option<(String, String, String)> {
    let m = s.message(message_id)?;
    let f = s.folder(&m.folder_id)?;
    (f.account_id != crate::SAMPLE_ACCOUNT).then(|| (f.account_id.clone(), f.path.clone(), m.remote_id.clone()))
}

fn set_flag(s: &mut Session, message_id: &str, flag: Flag, on: bool) {
    if let Some((account_id, path, id)) = remote(s, message_id) {
        s.pending.push(Job::Flag { account_id, path, id, flag, on });
    }
}

fn set_seen_one(s: &mut Session, message_id: &str, seen: bool) -> Result<(), CommandError> {
    let m = header_mut(s, message_id)?;
    if m.flags.seen == seen {
        return Ok(());
    }
    m.flags.seen = seen;
    set_flag(s, message_id, Flag::Seen, seen);
    Ok(())
}

/// Marks the whole selection read or unread.
fn set_seen(s: &mut Session, seen: bool) -> Result<(), CommandError> {
    for id in selection(s)? {
        set_seen_one(s, &id, seen)?;
    }
    Ok(())
}

/// Puts the flag on one message or takes it off, the server included; off takes the
/// follow-up date with it.
pub(crate) fn flag_message(s: &mut Session, message_id: &str, on: bool) -> Result<(), CommandError> {
    let m = header_mut(s, message_id)?;
    if m.flags.flagged != on {
        m.flags.flagged = on;
        set_flag(s, message_id, Flag::Flagged, on);
    }
    if !on {
        s.clear_follow_up(message_id);
    }
    Ok(())
}

/// `message.flag { id? }`: flags the given message, else the whole selection; takes the flag
/// off instead when the message (the selected one, for a selection) has it, as Outlook does.
fn toggle_flag(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let ids = match params.get("id").and_then(Value::as_str) {
        Some(id) => vec![id.to_string()],
        None => selection(s)?,
    };
    let first = ids.first().ok_or(CommandError::NoSelection)?;
    let on = !s.message(first).ok_or(CommandError::NotFound)?.flags.flagged;
    for id in ids {
        flag_message(s, &id, on)?;
    }
    Ok(())
}

/// `message.delete`: the selection goes to the Deleted folder; a message that is there already
/// goes for good, the way Outlook has it (the window asks first).
fn message_delete(s: &mut Session, _: &Value) -> Result<(), CommandError> {
    for id in selection(s)? {
        let in_deleted = s.message(&id).and_then(|m| s.folder(&m.folder_id)).is_some_and(|f| f.kind == FolderKind::Deleted);
        if in_deleted {
            delete_message(s, &id)?;
        } else {
            move_to_kind(s, &id, FolderKind::Deleted, "Deleted Items")?;
        }
    }
    Ok(())
}

/// Removes one message for good: out of the session and, for a real account, off the server.
pub(crate) fn delete_message(s: &mut Session, message_id: &str) -> Result<(), CommandError> {
    let m = s.message(message_id).cloned().ok_or(CommandError::NotFound)?;
    let from = s.folder(&m.folder_id).cloned().ok_or(CommandError::NotFound)?;
    if from.account_id != crate::SAMPLE_ACCOUNT {
        s.pending.push(Job::Delete { account_id: from.account_id, path: from.path, id: m.remote_id });
    }
    s.messages.retain(|h| h.id != message_id);
    s.bodies.remove(message_id);
    s.marked.retain(|id| id != message_id);
    if s.selected_message.as_deref() == Some(message_id) {
        s.selected_message = None;
    }
    s.recount_unread();
    Ok(())
}

/// Moves one message to its account's folder of `kind`.
fn move_to_kind(s: &mut Session, message_id: &str, kind: FolderKind, what: &'static str) -> Result<(), CommandError> {
    let m = s.message(message_id).ok_or(CommandError::NotFound)?;
    let account = s.folder(&m.folder_id).map(|f| f.account_id.clone()).ok_or(CommandError::NotFound)?;
    let target = s.folder_of_kind(&account, kind).map(|f| f.id.clone()).ok_or(CommandError::NoFolder(what))?;
    move_message(s, message_id, &target)
}

/// Moves the whole selection to each message's folder of `kind`.
fn move_selected(s: &mut Session, kind: FolderKind, what: &'static str) -> Result<(), CommandError> {
    for id in selection(s)? {
        move_to_kind(s, &id, kind, what)?;
    }
    Ok(())
}

/// `message.junk`: the selection goes to the Junk folder, and the server hears that the person
/// called it junk, so its own filter learns.
fn message_junk(s: &mut Session, _: &Value) -> Result<(), CommandError> {
    for id in selection(s)? {
        let account = s.message(&id).and_then(|m| s.folder(&m.folder_id)).map(|f| f.account_id.clone()).ok_or(CommandError::NotFound)?;
        s.folder_of_kind(&account, FolderKind::Junk).ok_or(CommandError::NoFolder("Junk"))?;
        crate::junk::report(s, &id, true);
        move_to_kind(s, &id, FolderKind::Junk, "Junk")?;
    }
    Ok(())
}

/// `message.move { folder_id, id? }`: moves the given message (else the whole selection) to a
/// folder of its own account. The folder it is in already is refused.
fn message_move(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let target = str_param(params, "folder_id")?.to_string();
    let ids = match params.get("id").and_then(Value::as_str) {
        Some(id) => vec![id.to_string()],
        None => selection(s)?,
    };
    for id in ids {
        move_message(s, &id, &target)?;
    }
    Ok(())
}

/// Moves one message: the header changes folder in the session and the server gets the job.
pub(crate) fn move_message(s: &mut Session, message_id: &str, target_id: &str) -> Result<(), CommandError> {
    let m = s.message(message_id).cloned().ok_or(CommandError::NotFound)?;
    let from = s.folder(&m.folder_id).cloned().ok_or(CommandError::NotFound)?;
    let target = s.folder(target_id).cloned().ok_or(CommandError::Param("folder_id"))?;
    if target.account_id != from.account_id || target.id == from.id {
        return Err(CommandError::Param("folder_id"));
    }
    if from.account_id != crate::SAMPLE_ACCOUNT {
        s.pending.push(Job::Move { account_id: from.account_id, path: from.path, id: m.remote_id, dest: target.path.clone() });
    }
    if let Some(h) = s.messages.iter_mut().find(|h| h.id == message_id) {
        h.folder_id = target.id;
    }
    s.marked.retain(|id| id != message_id);
    if s.selected_message.as_deref() == Some(message_id) {
        s.selected_message = None;
    }
    s.recount_unread();
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
    fn several_messages_are_selected_together_and_the_commands_take_them_all() {
        let mut s = Session::sample();
        let ids: Vec<String> = s.visible_messages().iter().take(4).map(|m| m.id.clone()).collect();
        s.run("message.select", &json!({ "id": ids[0] })).unwrap();
        s.run("message.select_more", &json!({ "id": ids[1] })).unwrap();
        assert_eq!(s.selection(), [ids[0].clone(), ids[1].clone()]);
        // Ctrl-click on a marked message takes it out again; on the selected one, the next steps up.
        s.run("message.select_more", &json!({ "id": ids[1] })).unwrap();
        assert_eq!(s.selection(), [ids[0].clone()]);
        s.run("message.select_many", &json!({ "ids": [ids[0], ids[1], ids[2]] })).unwrap();
        s.run("message.select_more", &json!({ "id": ids[0] })).unwrap();
        assert_eq!(s.selected_message.as_deref(), Some(ids[1].as_str()));
        assert_eq!(s.marked, [ids[2].clone()]);
        // A range that leaves the selected message out makes the first of it the selected one.
        s.run("message.select_many", &json!({ "ids": [ids[2], ids[3]] })).unwrap();
        assert_eq!(s.selection(), [ids[2].clone(), ids[3].clone()]);
        assert!(s.message(&ids[2]).unwrap().flags.seen, "opening it marks it read");

        s.run("message.mark_unread", &Value::Null).unwrap();
        assert!([&ids[2], &ids[3]].iter().all(|id| !s.message(id).unwrap().flags.seen));
        s.run("message.flag", &Value::Null).unwrap();
        assert!([&ids[2], &ids[3]].iter().all(|id| s.message(id).unwrap().flags.flagged));
        s.run("message.flag", &Value::Null).unwrap();
        assert!([&ids[2], &ids[3]].iter().all(|id| !s.message(id).unwrap().flags.flagged), "the second press takes every flag off");
        s.run("message.flag", &json!({ "id": ids[0] })).unwrap();
        assert!(s.message(&ids[0]).unwrap().flags.flagged && !s.message(&ids[2]).unwrap().flags.flagged, "an id flags that one message alone");
        s.run("message.flag", &json!({ "id": ids[0] })).unwrap();
        assert!(!s.message(&ids[0]).unwrap().flags.flagged);
        assert_eq!(s.run("message.flag", &json!({ "id": "missing" })), Err(CommandError::NotFound));
        s.run("message.delete", &Value::Null).unwrap();
        for id in [&ids[2], &ids[3]] {
            let folder = s.message(id).unwrap().folder_id.clone();
            assert_eq!(s.folder(&folder).unwrap().kind, FolderKind::Deleted);
        }
        assert!(s.selected_message.is_none() && s.marked.is_empty(), "nothing stays selected after the move");
        // A plain click lets the others go.
        s.run("message.select_many", &json!({ "ids": [ids[0], ids[1]] })).unwrap();
        s.run("message.select", &json!({ "id": ids[1] })).unwrap();
        assert_eq!(s.selection(), [ids[1].clone()]);
        assert_eq!(s.run("message.select_many", &json!({ "ids": ["missing"] })), Err(CommandError::NotFound));
        assert_eq!(s.run("message.select_many", &json!({ "ids": [] })), Err(CommandError::Param("ids")));
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
        use mailcraft_mailbox::{RemoteFolder, RemoteHeader};
        use mailcraft_model::{Account, Address, Protocol, Server, Tls};
        let account = Account {
            id: "acc".into(),
            name: "Acc".into(),
            address: Address::new(None, "a@b.nl").unwrap(),
            username: "a@b.nl".into(),
            protocol: Protocol::ImapSmtp,
            auth: mailcraft_model::Auth::Password,
            incoming: Some(Server { host: "imap.b.nl".into(), port: 993, tls: Tls::Implicit }),
            outgoing: None,
            dav: None,
            signature: String::new(),
        };
        let mut s = Session::with_accounts(vec![account]);
        assert_eq!(s.take_jobs(), vec![Job::Connect { account_id: "acc".into() }]);
        s.apply(crate::Event::Connected {
            account_id: "acc".into(),
            folders: vec![
                RemoteFolder { path: "Trash".into(), name: "Trash".into(), kind: FolderKind::Deleted, selectable: true },
                RemoteFolder { path: "INBOX".into(), name: "INBOX".into(), kind: FolderKind::Inbox, selectable: true },
                RemoteFolder { path: "[Gmail]".into(), name: "[Gmail]".into(), kind: FolderKind::Other, selectable: false },
            ],
            syncs_items: false,
        });
        assert_eq!(s.folders.iter().map(|f| f.name.as_str()).collect::<Vec<_>>(), ["Postvak IN", "Verwijderde items"]);
        assert_eq!(s.selected_folder.as_deref(), Some("acc/INBOX"));
        assert_eq!(
            s.take_jobs(),
            vec![Job::Headers {
                account_id: "acc".into(),
                folder_id: "acc/INBOX".into(),
                path: "INBOX".into(),
                offset: 0,
                limit: crate::HEADER_LIMIT
            }]
        );
        let header = mailcraft_mime::parse_header(b"From: x@y.nl\r\nSubject: Hoi\r\n\r\n");
        s.apply(crate::Event::Headers {
            folder_id: "acc/INBOX".into(),
            headers: vec![RemoteHeader {
                id: "7".into(),
                seen: false,
                flagged: false,
                answered: false,
                size: 10,
                has_attachments: false,
                categories: Vec::new(),
                header,
                date: 5,
            }],
            offset: 0,
            limit: crate::HEADER_LIMIT,
        });
        assert_eq!(s.folder("acc/INBOX").unwrap().unread, 1);
        s.run("message.select", &json!({ "id": "acc/INBOX#7" })).unwrap();
        let jobs = s.take_jobs();
        assert!(jobs.contains(&Job::Body { account_id: "acc".into(), message_id: "acc/INBOX#7".into(), path: "INBOX".into(), id: "7".into() }));
        assert!(jobs.contains(&Job::Flag { account_id: "acc".into(), path: "INBOX".into(), id: "7".into(), flag: Flag::Seen, on: true }));
        s.run("message.delete", &Value::Null).unwrap();
        assert_eq!(s.take_jobs(), vec![Job::Move { account_id: "acc".into(), path: "INBOX".into(), id: "7".into(), dest: "Trash".into() }]);
        assert_eq!(s.message("acc/INBOX#7").unwrap().folder_id, "acc/Trash");

        let draft = mailcraft_model::Draft {
            to: vec![Address::new(None, "x@y.nl").unwrap()],
            subject: "Hoi".into(),
            body: "Dag".into(),
            ..Default::default()
        };
        let id = s.send("acc", draft.clone());
        assert_eq!(s.outbox.get(&id), Some(&crate::SendState::Sending));
        assert_eq!(s.take_jobs(), vec![Job::Send { account_id: "acc".into(), draft_id: id, draft: Box::new(draft), sent_path: None }]);
        s.apply(crate::Event::Sent { account_id: "acc".into(), draft_id: id, result: Err("nee".into()) });
        assert_eq!(s.outbox.get(&id), Some(&crate::SendState::Failed("nee".into())));
    }

    #[test]
    fn a_message_moves_to_any_folder_of_its_own_account() {
        let mut s = Session::sample();
        let archive = s.folders.iter().find(|f| f.kind == FolderKind::Archive).map(|f| f.id.clone()).unwrap();
        let inbox = s.folders.iter().find(|f| f.kind == FolderKind::Inbox).map(|f| f.id.clone()).unwrap();
        let id = s.messages.iter().find(|m| m.folder_id == inbox).map(|m| m.id.clone()).unwrap();
        assert_eq!(s.run("message.move", &json!({ "folder_id": archive })), Err(CommandError::NoSelection));
        s.run("message.select", &json!({ "id": id })).unwrap();
        assert_eq!(s.run("message.move", &json!({ "folder_id": inbox })), Err(CommandError::Param("folder_id")), "already there");
        assert_eq!(s.run("message.move", &json!({ "folder_id": "nope" })), Err(CommandError::Param("folder_id")));
        s.run("message.move", &json!({ "folder_id": archive })).unwrap();
        assert_eq!(s.message(&id).unwrap().folder_id, archive);
        assert!(s.selected_message.is_none());
        assert!(s.take_jobs().is_empty(), "sample mail has no server");
        s.run("message.move", &json!({ "id": id, "folder_id": inbox })).unwrap();
        assert_eq!(s.message(&id).unwrap().folder_id, inbox);
    }

    #[test]
    fn sample_account_cannot_send() {
        let mut s = Session::sample();
        let id = s.send(crate::SAMPLE_ACCOUNT, mailcraft_model::Draft::default());
        assert!(matches!(s.outbox.get(&id), Some(crate::SendState::Failed(_))));
        assert!(s.take_jobs().is_empty());
    }
}
