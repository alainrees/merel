//! "Opruimen", Outlook's Clean Up: of each conversation in a folder the older messages go to
//! Deleted Items and the newest stays. Merel goes by the headers it has, not by whether a
//! later reply quotes the whole earlier message, so it keeps every message that may hold
//! something the newest one does not: unread, flagged or categorised messages and messages
//! with attachments. The moves are ordinary moves, so nothing is lost and the server sees them
//! like any other.

use std::collections::HashMap;

use serde_json::Value;

use merel_model::{FolderKind, MessageHeader, conversation_key};

use crate::{CommandError, Session, commands::move_message};

impl Session {
    /// The messages Clean Up would take out of `folder_id`, by id: in each conversation there
    /// every message older than the newest one, except the unread, the flagged, the categorised
    /// and those with attachments. `key` limits it to one conversation. A subject that is
    /// nothing but prefixes has no conversation and is never touched.
    pub fn redundant_messages(&self, folder_id: &str, key: Option<&str>) -> Vec<String> {
        let mut by_key: HashMap<String, Vec<&MessageHeader>> = HashMap::new();
        for m in self.messages.iter().filter(|m| m.folder_id == folder_id) {
            let k = conversation_key(&m.subject);
            if k.is_empty() || key.is_some_and(|key| key != k) {
                continue;
            }
            by_key.entry(k).or_default().push(m);
        }
        let mut out = Vec::new();
        for list in by_key.values_mut() {
            // Newest first; of two with the same time the one listed first stays.
            list.sort_by_key(|m| std::cmp::Reverse(m.date));
            out.extend(
                list.iter()
                    .skip(1)
                    .filter(|m| m.flags.seen && !m.flags.flagged && !m.has_attachments && m.categories.is_empty())
                    .map(|m| m.id.clone()),
            );
        }
        out.sort();
        out
    }
}

/// Moves the redundant messages of `folder_id` (of one conversation when `key` is given) to
/// the account's Deleted Items; that folder itself is never cleaned up.
fn clean(s: &mut Session, folder_id: &str, key: Option<&str>) -> Result<(), CommandError> {
    let folder = s.folder(folder_id).cloned().ok_or(CommandError::NotFound)?;
    if folder.kind == FolderKind::Deleted {
        return Err(CommandError::Param("folder"));
    }
    let target = s.folder_of_kind(&folder.account_id, FolderKind::Deleted).map(|f| f.id.clone()).ok_or(CommandError::NoFolder("Deleted Items"))?;
    for id in s.redundant_messages(folder_id, key) {
        move_message(s, &id, &target)?;
    }
    Ok(())
}

/// `conversation.cleanup { id? }`: cleans up the conversation of the given (else the selected)
/// message, within its folder.
pub fn conversation(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = params.get("id").and_then(Value::as_str).map(str::to_string).or_else(|| s.selected_message.clone()).ok_or(CommandError::NoSelection)?;
    let m = s.message(&id).ok_or(CommandError::NotFound)?;
    let key = conversation_key(&m.subject);
    if key.is_empty() {
        return Err(CommandError::Param("subject"));
    }
    let folder_id = m.folder_id.clone();
    clean(s, &folder_id, Some(&key))
}

/// `folder.cleanup { id? }`: cleans up every conversation in the given (else the selected)
/// folder.
pub fn folder(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let id = params.get("id").and_then(Value::as_str).map(str::to_string).or_else(|| s.selected_folder.clone()).ok_or(CommandError::NoSelection)?;
    clean(s, &id, None)
}

#[cfg(test)]
mod tests {
    use merel_mailbox::{RemoteFolder, RemoteHeader};
    use serde_json::json;

    use super::*;
    use crate::{Event, HEADER_LIMIT, Job};

    fn header(n: u32, subject: &str, seen: bool, flagged: bool, has_attachments: bool) -> RemoteHeader {
        RemoteHeader {
            id: n.to_string(),
            seen,
            flagged,
            answered: false,
            size: 1,
            has_attachments,
            categories: Vec::new(),
            header: merel_mime::parse_header(format!("From: x@example.org\r\nSubject: {subject}\r\n\r\n").as_bytes()),
            date: i64::from(n),
        }
    }

    #[test]
    fn clean_up_keeps_the_newest_and_what_may_hold_more_and_moves_the_rest_to_deleted_items() {
        let mut s = Session::with_accounts(vec![crate::tests::account("a")]);
        s.apply(Event::Connected {
            account_id: "a".into(),
            folders: vec![
                RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), kind: FolderKind::Inbox, selectable: true, parent: None },
                RemoteFolder { path: "Trash".into(), name: "Trash".into(), kind: FolderKind::Deleted, selectable: true, parent: None },
            ],
            syncs_items: false,
        });
        s.apply(Event::Headers {
            folder_id: "a/INBOX".into(),
            headers: vec![
                header(1, "Offerte", true, false, false),
                header(2, "Re: Offerte", false, false, false),
                header(3, "RE: Offerte", true, true, false),
                header(4, "Re: offerte", true, false, true),
                header(5, "Fwd: Offerte", true, false, false),
                header(6, "Ander onderwerp", true, false, false),
                header(7, "Re: Ander onderwerp", true, false, false),
                header(8, "Re:", true, false, false),
            ],
            offset: 0,
            limit: HEADER_LIMIT,
        });
        s.take_jobs();
        let folder = |s: &Session, n: u32| s.message(&format!("a/INBOX#{n}")).map(|m| m.folder_id.clone()).unwrap_or_default();

        assert_eq!(s.redundant_messages("a/INBOX", Some("offerte")), ["a/INBOX#1"], "2 is unread, 3 flagged, 4 has attachments, 5 is the newest");
        assert_eq!(s.redundant_messages("a/INBOX", None), ["a/INBOX#1", "a/INBOX#6"]);

        assert_eq!(s.run("conversation.cleanup", &json!({ "id": "a/INBOX#8" })), Err(CommandError::Param("subject")));
        assert_eq!(s.run("conversation.cleanup", &Value::Null), Err(CommandError::NoSelection));
        s.run("conversation.cleanup", &json!({ "id": "a/INBOX#2" })).unwrap();
        assert_eq!(folder(&s, 1), "a/Trash");
        assert!([2, 3, 4, 5, 6, 7].iter().all(|n| folder(&s, *n) == "a/INBOX"));
        let jobs = s.take_jobs();
        assert!(jobs.iter().any(|j| matches!(j, Job::Move { dest, id, .. } if dest == "Trash" && id == "1")));

        s.run("folder.select", &json!({ "id": "a/INBOX" })).unwrap();
        s.run("folder.cleanup", &Value::Null).unwrap();
        assert_eq!(folder(&s, 6), "a/Trash");
        assert_eq!(folder(&s, 7), "a/INBOX");
        assert!(s.redundant_messages("a/INBOX", None).is_empty(), "a second pass finds nothing");
        assert_eq!(s.run("folder.cleanup", &json!({ "id": "a/Trash" })), Err(CommandError::Param("folder")), "Deleted Items is left alone");
    }
}
