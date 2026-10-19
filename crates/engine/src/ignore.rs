//! Conversations the person ignores, Outlook's "Negeren": the messages of the conversation in
//! the inbox go to Deleted Items, and so does every message that arrives in it afterwards,
//! until the person stops ignoring it. A conversation is the [`conversation_key`] of the
//! subject, per account; the list lives in `ignored.json`. Nothing is told to the server:
//! the moves are ordinary moves.

use serde::{Deserialize, Serialize};
use serde_json::Value;

use mailcraft_model::{FolderKind, IgnoredConversation, conversation_key};

use crate::{CommandError, Session, commands::move_message};

/// What `ignored.json` holds.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct IgnoredFile {
    #[serde(default)]
    pub ignored: Vec<IgnoredConversation>,
}

impl Session {
    pub fn set_ignored(&mut self, list: Vec<IgnoredConversation>) {
        self.ignored = list;
    }

    pub fn ignored_data(&self) -> IgnoredFile {
        IgnoredFile { ignored: self.ignored.clone() }
    }

    /// Whether a command changed the list since the last call.
    pub fn take_ignored_dirty(&mut self) -> bool {
        std::mem::take(&mut self.ignored_dirty)
    }

    /// Whether the conversation `subject` belongs to is ignored for `account_id`.
    pub fn conversation_ignored(&self, account_id: &str, subject: &str) -> bool {
        let key = conversation_key(subject);
        !key.is_empty() && self.ignored.iter().any(|i| i.account_id == account_id && i.key == key)
    }

    /// Whether the message's conversation is ignored.
    pub fn message_ignored(&self, message_id: &str) -> bool {
        self.message(message_id)
            .and_then(|m| self.folder(&m.folder_id).map(|f| (f, m)))
            .is_some_and(|(f, m)| self.conversation_ignored(&f.account_id, &m.subject))
    }

    /// A message new in an inbox whose conversation is ignored goes to Deleted Items; true when
    /// it did (the rules then do not see it).
    pub(crate) fn apply_ignored(&mut self, message_id: &str) -> bool {
        let Some(m) = self.message(message_id) else { return false };
        let Some(folder) = self.folder(&m.folder_id) else { return false };
        if folder.kind != FolderKind::Inbox || !self.conversation_ignored(&folder.account_id, &m.subject) {
            return false;
        }
        let Some(deleted) = self.folder_of_kind(&folder.account_id, FolderKind::Deleted).map(|f| f.id.clone()) else { return false };
        move_message(self, message_id, &deleted).is_ok()
    }

    /// Moves every message of the conversation `key` that sits in the account's folder of
    /// `from` to its folder of `to`.
    fn move_conversation(&mut self, account_id: &str, key: &str, from: FolderKind, to: FolderKind) -> Result<(), CommandError> {
        let target = self.folder_of_kind(account_id, to).map(|f| f.id.clone()).ok_or(CommandError::NoFolder(match to {
            FolderKind::Inbox => "Inbox",
            _ => "Deleted Items",
        }))?;
        let ids: Vec<String> = self
            .messages
            .iter()
            .filter(|m| conversation_key(&m.subject) == key)
            .filter(|m| self.folder(&m.folder_id).is_some_and(|f| f.account_id == account_id && f.kind == from))
            .map(|m| m.id.clone())
            .collect();
        for id in ids {
            move_message(self, &id, &target)?;
        }
        Ok(())
    }
}

/// The message a command works on (`id`, else the selected one) with its account and
/// conversation key; a subject that is nothing but prefixes has no conversation.
fn target(s: &Session, params: &Value) -> Result<(String, String, String), CommandError> {
    let id = params.get("id").and_then(Value::as_str).map(str::to_string).or_else(|| s.selected_message.clone()).ok_or(CommandError::NoSelection)?;
    let m = s.message(&id).ok_or(CommandError::NotFound)?;
    let account = s.folder(&m.folder_id).map(|f| f.account_id.clone()).ok_or(CommandError::NotFound)?;
    let key = conversation_key(&m.subject);
    if key.is_empty() {
        return Err(CommandError::Param("subject"));
    }
    Ok((account, key, m.subject.clone()))
}

/// `conversation.ignore { id? }`: ignores the conversation of the given (else the selected)
/// message: its messages in the inbox go to Deleted Items now, later ones on arrival.
pub fn ignore(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let (account_id, key, subject) = target(s, params)?;
    if !s.ignored.iter().any(|i| i.account_id == account_id && i.key == key) {
        s.ignored.push(IgnoredConversation { account_id: account_id.clone(), key: key.clone(), subject });
        s.ignored_dirty = true;
    }
    s.move_conversation(&account_id, &key, FolderKind::Inbox, FolderKind::Deleted)
}

/// `conversation.unignore { id? }`: stops ignoring the conversation of the given (else the
/// selected) message; its messages in Deleted Items come back to the inbox.
pub fn unignore(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let (account_id, key, _) = target(s, params)?;
    let before = s.ignored.len();
    s.ignored.retain(|i| !(i.account_id == account_id && i.key == key));
    if s.ignored.len() == before {
        return Err(CommandError::NotFound);
    }
    s.ignored_dirty = true;
    s.move_conversation(&account_id, &key, FolderKind::Deleted, FolderKind::Inbox)
}

#[cfg(test)]
mod tests {
    use mailcraft_mailbox::{RemoteFolder, RemoteHeader};
    use serde_json::json;

    use super::*;
    use crate::{Event, HEADER_LIMIT, Job};

    fn header(n: u32, subject: &str) -> RemoteHeader {
        RemoteHeader {
            id: n.to_string(),
            seen: false,
            flagged: false,
            answered: false,
            size: 1,
            has_attachments: false,
            categories: Vec::new(),
            header: mailcraft_mime::parse_header(format!("From: x@example.org\r\nSubject: {subject}\r\n\r\n").as_bytes()),
            date: i64::from(n),
        }
    }

    #[test]
    fn an_ignored_conversation_goes_to_deleted_items_now_and_on_arrival_until_it_is_not() {
        let mut s = Session::with_accounts(vec![crate::tests::account("a")]);
        s.apply(Event::Connected {
            account_id: "a".into(),
            folders: vec![
                RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), kind: FolderKind::Inbox, selectable: true },
                RemoteFolder { path: "Trash".into(), name: "Trash".into(), kind: FolderKind::Deleted, selectable: true },
            ],
            syncs_items: false,
        });
        let list = |headers: Vec<RemoteHeader>| Event::Headers { folder_id: "a/INBOX".into(), headers, offset: 0, limit: HEADER_LIMIT };
        s.apply(list(vec![header(1, "Re: Nieuwsbrief"), header(2, "Ander onderwerp"), header(3, "Re:")]));
        let folder = |s: &Session, n: u32| s.message(&format!("a/INBOX#{n}")).map(|m| m.folder_id.clone()).unwrap_or_default();
        s.take_jobs();

        assert_eq!(s.run("conversation.ignore", &json!({ "id": "a/INBOX#3" })), Err(CommandError::Param("subject")), "nothing to go by");
        assert_eq!(s.run("conversation.unignore", &json!({ "id": "a/INBOX#1" })), Err(CommandError::NotFound));
        s.run("message.select", &json!({ "id": "a/INBOX#1" })).unwrap();
        s.run("conversation.ignore", &Value::Null).unwrap();
        assert!(s.take_ignored_dirty());
        assert_eq!(s.ignored.len(), 1);
        assert_eq!((s.ignored[0].key.as_str(), s.ignored[0].subject.as_str()), ("nieuwsbrief", "Re: Nieuwsbrief"));
        assert_eq!((folder(&s, 1), folder(&s, 2)), ("a/Trash".to_string(), "a/INBOX".to_string()));
        assert!(s.message_ignored("a/INBOX#1") && !s.message_ignored("a/INBOX#2"));
        assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Move { dest, id, .. } if dest == "Trash" && id == "1")));
        s.run("conversation.ignore", &json!({ "id": "a/INBOX#1" })).unwrap();
        assert!(!s.take_ignored_dirty(), "ignoring it again changes nothing");

        // New mail in the conversation is filed on arrival; the rest is not.
        s.apply(list(vec![header(2, "Ander onderwerp"), header(4, "RE: nieuwsbrief"), header(5, "Fwd: Ander onderwerp")]));
        assert_eq!((folder(&s, 4), folder(&s, 5)), ("a/Trash".to_string(), "a/INBOX".to_string()));
        assert!(s.take_jobs().iter().any(|j| matches!(j, Job::Move { dest, id, .. } if dest == "Trash" && id == "4")));

        // Stopping brings the conversation back from Deleted Items.
        s.run("conversation.unignore", &json!({ "id": "a/INBOX#4" })).unwrap();
        assert!(s.take_ignored_dirty() && s.ignored.is_empty());
        assert_eq!((folder(&s, 1), folder(&s, 4)), ("a/INBOX".to_string(), "a/INBOX".to_string()));
        s.apply(list(vec![header(2, "Ander onderwerp"), header(6, "Re: Nieuwsbrief")]));
        assert_eq!(folder(&s, 6), "a/INBOX");

        // Removing the account drops its list.
        s.run("conversation.ignore", &json!({ "id": "a/INBOX#6" })).unwrap();
        s.take_ignored_dirty();
        s.remove_account("a");
        assert!(s.ignored.is_empty() && s.take_ignored_dirty(), "the last account gone, the file is still rewritten");
    }
}
