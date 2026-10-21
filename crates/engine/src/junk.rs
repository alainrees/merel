//! Blocked and safe senders, kept in `junk.json` next to the accounts file (one list for every
//! account) and applied to the messages a refresh finds new in an inbox. A blocked sender's
//! mail goes to the account's Junk folder the way "Ongewenst" does it by hand: the header
//! moves in the session at once and the server gets the job. A safe sender, or a contact, is
//! never filed, whatever the blocked list says. The server's own filter stays the first line.

use serde_json::Value;

use merel_model::{FolderKind, JunkLists, junk_entry};

use crate::{CommandError, Job, SAMPLE_ACCOUNT, Session, commands::move_message};
use merel_model::JunkLevel;

impl Session {
    pub fn set_junk(&mut self, lists: JunkLists) {
        self.junk = lists;
    }

    pub fn junk_data(&self) -> JunkLists {
        self.junk.clone()
    }

    /// Whether a command changed the lists since the last call.
    pub fn take_junk_dirty(&mut self) -> bool {
        std::mem::take(&mut self.junk_dirty)
    }

    /// Whether mail from `email` is let through whatever the blocked list says: a safe sender,
    /// or one of the contacts.
    pub fn sender_safe(&self, email: &str) -> bool {
        self.junk.is_safe(email) || self.contact_with_email(email).is_some()
    }

    /// Whether mail from `email` is filed as junk on arrival.
    pub fn sender_blocked(&self, email: &str) -> bool {
        !self.sender_safe(email) && self.junk.is_blocked(email)
    }

    /// Whether a message with this sender is filed as junk on arrival: a blocked sender, and
    /// beyond that what the level says. Safe senders, contacts and your own addresses never are.
    pub fn files_as_junk(&self, from: &merel_model::Address) -> bool {
        let email = from.email();
        let own: Vec<&str> = self.accounts.iter().map(|a| a.address.email()).collect();
        if self.sender_safe(email) || own.iter().any(|o| o.eq_ignore_ascii_case(email)) {
            return false;
        }
        if self.junk.is_blocked(email) {
            return true;
        }
        match self.junk.level {
            JunkLevel::Standard => false,
            JunkLevel::High => !merel_model::sender_warnings(from, &self.contacts, &own).is_empty(),
            JunkLevel::SafeOnly => true,
        }
    }

    /// A message new in an inbox that [`Self::files_as_junk`] goes to the account's Junk folder;
    /// true when it did (the rules then do not see it).
    pub(crate) fn apply_junk(&mut self, message_id: &str) -> bool {
        let Some(m) = self.message(message_id) else { return false };
        if !self.files_as_junk(&m.from) {
            return false;
        }
        let Some(folder) = self.folder(&m.folder_id) else { return false };
        if folder.kind != FolderKind::Inbox {
            return false;
        }
        let Some(junk) = self.folder_of_kind(&folder.account_id, FolderKind::Junk).map(|f| f.id.clone()) else { return false };
        move_message(self, message_id, &junk).is_ok()
    }
}

/// Queues the word to the server that `message_id` is junk (or not), for the stores that take
/// it (IMAP keywords, Exchange's `MarkAsJunk`); before any move, while the message is where
/// the session says. Sample mail has no server.
pub(crate) fn report(s: &mut Session, message_id: &str, junk: bool) {
    let Some(m) = s.message(message_id) else { return };
    let Some(folder) = s.folder(&m.folder_id) else { return };
    if folder.account_id == SAMPLE_ACCOUNT {
        return;
    }
    s.pending.push(Job::Junk { account_id: folder.account_id.clone(), path: folder.path.clone(), id: m.remote_id.clone(), junk });
}

/// The entry a command works on, and the message it came from: `entry` as typed, else the
/// sender of the given (else the selected) message.
fn entry_param(s: &Session, params: &Value) -> Result<(String, Option<String>), CommandError> {
    if let Some(text) = params.get("entry").and_then(Value::as_str) {
        return junk_entry(text).map(|e| (e, None)).ok_or(CommandError::Param("entry"));
    }
    let id = params.get("id").and_then(Value::as_str).map(str::to_string).or_else(|| s.selected_message.clone()).ok_or(CommandError::NoSelection)?;
    let m = s.message(&id).ok_or(CommandError::NotFound)?;
    let entry = junk_entry(m.from.email()).ok_or(CommandError::Param("entry"))?;
    Ok((entry, Some(id)))
}

/// Moves `id` to the account's folder of `kind` unless it is there already.
fn file_as(s: &mut Session, id: &str, kind: FolderKind, what: &'static str) -> Result<(), CommandError> {
    let m = s.message(id).ok_or(CommandError::NotFound)?;
    let folder = s.folder(&m.folder_id).ok_or(CommandError::NotFound)?;
    if folder.kind == kind {
        return Ok(());
    }
    let target = s.folder_of_kind(&folder.account_id, kind).map(|f| f.id.clone()).ok_or(CommandError::NoFolder(what))?;
    move_message(s, id, &target)
}

/// `junk.block { entry? | id? }`: blocks an address or a domain ("example.org" covers every
/// address there). Without `entry`, the sender of the given (else the selected) message is
/// blocked and the message goes to the Junk folder. The entry leaves the safe list.
pub fn block(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let (entry, message) = entry_param(s, params)?;
    s.junk.safe.retain(|e| e != &entry);
    if !s.junk.blocked.contains(&entry) {
        s.junk.blocked.push(entry);
    }
    s.junk_dirty = true;
    match message {
        Some(id) => {
            report(s, &id, true);
            file_as(s, &id, FolderKind::Junk, "Junk")
        }
        None => Ok(()),
    }
}

/// `junk.trust { entry? | id? }`: a safe sender, never filed as junk. Without `entry`, the
/// sender of the given (else the selected) message, and a message in the Junk folder comes
/// back to the inbox ("Geen ongewenste e-mail"). The entry leaves the blocked list.
pub fn trust(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let (entry, message) = entry_param(s, params)?;
    s.junk.blocked.retain(|e| e != &entry);
    if !s.junk.safe.contains(&entry) {
        s.junk.safe.push(entry);
    }
    s.junk_dirty = true;
    if let Some(id) = message
        && s.message(&id).and_then(|m| s.folder(&m.folder_id)).is_some_and(|f| f.kind == FolderKind::Junk)
    {
        report(s, &id, false);
        file_as(s, &id, FolderKind::Inbox, "Inbox")?;
    }
    Ok(())
}

/// `junk.level { level }`: how far the filing goes ("standard", "high" or "safe_only").
pub fn level(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let level = params.get("level").and_then(Value::as_str).and_then(JunkLevel::from_id).ok_or(CommandError::Param("level"))?;
    if s.junk.level != level {
        s.junk.level = level;
        s.junk_dirty = true;
    }
    Ok(())
}

/// `junk.forget { entry }`: takes an entry off both lists.
pub fn forget(s: &mut Session, params: &Value) -> Result<(), CommandError> {
    let entry = params.get("entry").and_then(Value::as_str).and_then(junk_entry).ok_or(CommandError::Param("entry"))?;
    let before = s.junk.blocked.len() + s.junk.safe.len();
    s.junk.blocked.retain(|e| e != &entry);
    s.junk.safe.retain(|e| e != &entry);
    if s.junk.blocked.len() + s.junk.safe.len() != before {
        s.junk_dirty = true;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use merel_mailbox::{RemoteFolder, RemoteHeader};
    use serde_json::json;

    use super::*;
    use crate::{Event, HEADER_LIMIT};

    #[test]
    fn a_sender_is_blocked_from_a_message_and_trusted_back() {
        let mut s = Session::sample();
        s.run("junk.block", &json!({ "id": "m3" })).unwrap();
        assert_eq!(s.junk_data().blocked, ["news@example.org"]);
        assert!(s.take_junk_dirty());
        assert_eq!(s.message("m3").map(|m| m.folder_id.as_str()), Some("junk"), "the message is filed");
        assert!(s.sender_blocked("NEWS@example.org"));
        assert!(!s.sender_blocked("sanne@example.org"));

        s.run("junk.block", &json!({ "entry": " @Example.org " })).unwrap();
        assert_eq!(s.junk_data().blocked, ["news@example.org", "example.org"]);
        assert!(s.sender_blocked("x@mail.example.org"), "a domain covers its subdomains");
        assert!(!s.sender_blocked("sanne@example.org"), "a contact is never blocked");
        assert_eq!(s.message("m1").map(|m| m.folder_id.as_str()), Some("inbox"), "blocking by entry moves nothing");

        s.run("message.select", &json!({ "id": "m3" })).unwrap();
        s.run("junk.trust", &Value::Null).unwrap();
        let lists = s.junk_data();
        assert_eq!((lists.blocked.as_slice(), lists.safe.as_slice()), (&["example.org".to_string()][..], &["news@example.org".to_string()][..]));
        assert_eq!(s.message("m3").map(|m| m.folder_id.as_str()), Some("inbox"), "not junk: back to the inbox");
        assert!(!s.sender_blocked("news@example.org"), "safe wins over a blocked domain");

        s.run("junk.forget", &json!({ "entry": "example.org" })).unwrap();
        assert_eq!(s.junk_data().blocked.len(), 0);
        assert!(s.take_junk_dirty());
        s.run("junk.forget", &json!({ "entry": "nobody@nowhere.example" })).unwrap();
        assert!(!s.take_junk_dirty(), "forgetting what is not listed changes nothing");

        assert_eq!(s.run("junk.block", &json!({ "entry": "not an address" })), Err(CommandError::Param("entry")));
        assert_eq!(s.run("junk.forget", &Value::Null), Err(CommandError::Param("entry")));
        assert_eq!(s.run("junk.block", &json!({ "id": "nope" })), Err(CommandError::NotFound));
        s.selected_message = None;
        assert_eq!(s.run("junk.block", &Value::Null), Err(CommandError::NoSelection));
    }

    fn header(n: u32, from: &str) -> RemoteHeader {
        RemoteHeader {
            id: n.to_string(),
            seen: false,
            flagged: false,
            answered: false,
            size: 1,
            has_attachments: false,
            categories: Vec::new(),
            header: merel_mime::parse_header(format!("From: {from}\r\nSubject: Hallo\r\n\r\n").as_bytes()),
            date: i64::from(n),
        }
    }

    #[test]
    fn new_inbox_mail_from_a_blocked_sender_is_filed_as_junk_with_a_job_for_the_server() {
        let mut s = Session::with_accounts(vec![crate::tests::account("a")]);
        s.apply(Event::Connected {
            account_id: "a".into(),
            folders: vec![
                RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), kind: FolderKind::Inbox, selectable: true, parent: None },
                RemoteFolder { path: "Junk".into(), name: "Junk".into(), kind: FolderKind::Junk, selectable: true, parent: None },
            ],
            syncs_items: false,
        });
        s.take_jobs();
        s.run("junk.block", &json!({ "entry": "spam.example" })).unwrap();
        let list = |headers: Vec<RemoteHeader>| Event::Headers { folder_id: "a/INBOX".into(), headers, offset: 0, limit: HEADER_LIMIT };
        // The first listing is not "new mail": nothing is filed.
        s.apply(list(vec![header(1, "old@spam.example")]));
        assert_eq!(s.message("a/INBOX#1").map(|m| m.folder_id.as_str()), Some("a/INBOX"));
        s.apply(list(vec![header(1, "old@spam.example"), header(2, "new@spam.example"), header(3, "friend@example.org")]));
        assert_eq!(s.message("a/INBOX#2").map(|m| m.folder_id.as_str()), Some("a/Junk"), "the arrival from the blocked domain is filed");
        assert_eq!(s.message("a/INBOX#3").map(|m| m.folder_id.as_str()), Some("a/INBOX"));
        let jobs = s.take_jobs();
        assert!(jobs.iter().any(|j| matches!(j, Job::Move { dest, id, .. } if dest == "Junk" && id == "2")), "{jobs:?}");
        assert!(!jobs.iter().any(|j| matches!(j, Job::Junk { .. })), "filing by the list is not the person's word");

        // The person's own word goes to the server first, then the move; and back again.
        s.run("junk.block", &json!({ "id": "a/INBOX#3" })).unwrap();
        let jobs = s.take_jobs();
        let junk = |j: &Job| matches!(j, Job::Junk { path, id, junk: true, .. } if path == "INBOX" && id == "3");
        let moved = |j: &Job| matches!(j, Job::Move { dest, id, .. } if dest == "Junk" && id == "3");
        assert!(jobs.iter().position(junk) < jobs.iter().position(moved) && jobs.iter().any(moved), "{jobs:?}");
        s.run("junk.trust", &json!({ "id": "a/INBOX#3" })).unwrap();
        let jobs = s.take_jobs();
        assert!(jobs.iter().any(|j| matches!(j, Job::Junk { path, junk: false, .. } if path == "Junk")), "{jobs:?}");
        assert!(jobs.iter().any(|j| matches!(j, Job::Move { dest, .. } if dest == "INBOX")), "{jobs:?}");
        assert_eq!(s.message("a/INBOX#3").map(|m| m.folder_id.as_str()), Some("a/INBOX"));

        // "Ongewenst" by hand says the same.
        s.run("message.select", &json!({ "id": "a/INBOX#1" })).unwrap();
        s.run("message.junk", &Value::Null).unwrap();
        let jobs = s.take_jobs();
        assert!(
            jobs.iter().position(|j| matches!(j, Job::Junk { junk: true, .. })) < jobs.iter().position(|j| matches!(j, Job::Move { .. })),
            "{jobs:?}"
        );
        assert!(jobs.iter().any(|j| matches!(j, Job::Move { .. })));
    }

    #[test]
    fn the_level_says_how_far_the_filing_goes() {
        let mut s = Session::with_accounts(vec![crate::tests::account("a")]);
        s.apply(Event::Connected {
            account_id: "a".into(),
            folders: vec![
                RemoteFolder { path: "INBOX".into(), name: "Inbox".into(), kind: FolderKind::Inbox, selectable: true, parent: None },
                RemoteFolder { path: "Junk".into(), name: "Junk".into(), kind: FolderKind::Junk, selectable: true, parent: None },
            ],
            syncs_items: false,
        });
        s.contacts.push(merel_model::Contact {
            id: "c1".into(),
            name: "Sanne de Vries".into(),
            emails: vec!["sanne@example.org".into()],
            ..Default::default()
        });
        let list = |headers: Vec<RemoteHeader>| Event::Headers { folder_id: "a/INBOX".into(), headers, offset: 0, limit: HEADER_LIMIT };
        s.apply(list(vec![header(1, "old@example.org")]));
        let folder = |s: &Session, n: u32| s.message(&format!("a/INBOX#{n}")).map(|m| m.folder_id.clone()).unwrap_or_default();

        // Standard: only the blocked list.
        s.apply(list(vec![header(1, "old@example.org"), header(2, "\"Sanne de Vries\" <sanne@examp1e.org>"), header(3, "shop@store.example")]));
        assert_eq!((folder(&s, 2), folder(&s, 3)), ("a/INBOX".to_string(), "a/INBOX".to_string()));

        assert_eq!(s.run("junk.level", &json!({ "level": "loud" })), Err(CommandError::Param("level")));
        s.run("junk.level", &json!({ "level": "high" })).unwrap();
        assert!(s.take_junk_dirty());
        s.run("junk.level", &json!({ "level": "high" })).unwrap();
        assert!(!s.take_junk_dirty(), "the same level again changes nothing");
        // High: a sender with signs of phishing in the header, not an unknown shop.
        s.apply(list(vec![header(1, "old@example.org"), header(4, "\"Sanne de Vries\" <sanne@examp1e.org>"), header(5, "shop@store.example")]));
        assert_eq!((folder(&s, 4), folder(&s, 5)), ("a/Junk".to_string(), "a/INBOX".to_string()));

        // Safe lists only: everything not from a contact, a safe sender or yourself.
        s.run("junk.level", &json!({ "level": "safe_only" })).unwrap();
        s.run("junk.trust", &json!({ "entry": "store.example" })).unwrap();
        s.apply(list(vec![
            header(1, "old@example.org"),
            header(6, "sanne@example.org"),
            header(7, "shop@store.example"),
            header(8, "A@voorbeeld.nl"),
            header(9, "stranger@elsewhere.example"),
        ]));
        assert_eq!(folder(&s, 6), "a/INBOX", "a contact");
        assert_eq!(folder(&s, 7), "a/INBOX", "a safe sender");
        assert_eq!(folder(&s, 8), "a/INBOX", "your own address");
        assert_eq!(folder(&s, 9), "a/Junk", "a stranger");
        assert!(!s.take_jobs().iter().any(|j| matches!(j, Job::Junk { .. })), "filing by the level is not the person's word");
    }
}
