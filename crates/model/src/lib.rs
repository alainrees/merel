//! The mail model: accounts, folders, message headers and addresses.
//!
//! This crate does no I/O and knows nothing of the UI. Everything that crosses into it from the
//! network (an address, a subject, a folder name) is validated or bounded here, so the layers
//! above can trust what they hold.

mod address;

pub use address::{Address, AddressError, parse_address_list};

use serde::{Deserialize, Serialize};

/// Longest subject or display name kept, in characters; a hostile server can send megabytes.
pub const MAX_TEXT: usize = 998;

/// How an account talks to its server. IMAP/SMTP comes first; the others follow.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Protocol {
    ImapSmtp,
    Gmail,
    Microsoft365,
    Exchange,
}

/// Transport security for a connection. There is deliberately no plain-text variant.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Tls {
    /// TLS from the first byte (IMAPS 993, SMTPS 465).
    Implicit,
    /// STARTTLS, required: the connection is dropped if the server does not offer it.
    StartTls,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Server {
    pub host: String,
    pub port: u16,
    pub tls: Tls,
}

/// An account. Secrets (passwords, OAuth tokens) are never stored here: they live in the
/// system keychain, looked up by `id`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Account {
    pub id: String,
    pub name: String,
    pub address: Address,
    /// The login name, usually the address itself.
    pub username: String,
    pub protocol: Protocol,
    pub incoming: Option<Server>,
    pub outgoing: Option<Server>,
}

/// The folders Outlook treats specially; everything else is `Other`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FolderKind {
    Inbox,
    Drafts,
    Sent,
    Archive,
    Junk,
    Deleted,
    Other,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Folder {
    pub id: String,
    pub account_id: String,
    /// What the list shows: the last path segment, or the Dutch name of a special folder.
    pub name: String,
    /// The mailbox name on the server, as given to SELECT.
    pub path: String,
    pub kind: FolderKind,
    pub unread: u32,
}

impl FolderKind {
    /// The name Outlook (Dutch) gives a special folder; `None` for ordinary ones.
    pub fn dutch_name(self) -> Option<&'static str> {
        Some(match self {
            FolderKind::Inbox => "Postvak IN",
            FolderKind::Drafts => "Concepten",
            FolderKind::Sent => "Verzonden items",
            FolderKind::Archive => "Archief",
            FolderKind::Junk => "Ongewenste e-mail",
            FolderKind::Deleted => "Verwijderde items",
            FolderKind::Other => return None,
        })
    }

    /// Order in the folder pane: Inbox first, then the other special folders, then the rest.
    pub fn rank(self) -> u8 {
        match self {
            FolderKind::Inbox => 0,
            FolderKind::Drafts => 1,
            FolderKind::Sent => 2,
            FolderKind::Archive => 3,
            FolderKind::Junk => 4,
            FolderKind::Deleted => 5,
            FolderKind::Other => 6,
        }
    }
}

/// Message flags, as IMAP has them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Flags {
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
}

/// What the message list shows: the envelope, not the body.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct MessageHeader {
    pub id: String,
    pub folder_id: String,
    /// The IMAP UID; 0 for sample mail.
    pub uid: u32,
    pub from: Address,
    pub to: Vec<Address>,
    pub subject: String,
    /// Seconds since the Unix epoch, UTC.
    pub date: i64,
    pub preview: String,
    pub flags: Flags,
    pub has_attachments: bool,
    /// The Message-ID header, for replies.
    pub message_id: Option<String>,
}

/// A message being written.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Draft {
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
    pub subject: String,
    pub body: String,
    /// The Message-ID this answers, if any.
    pub in_reply_to: Option<String>,
}

/// `s` cut to [`MAX_TEXT`] characters with control characters (other than tab) removed, so a
/// header cannot smuggle line breaks or terminal escapes into the UI or a log.
pub fn clean_text(s: &str) -> String {
    s.chars().filter(|c| !c.is_control() || *c == '\t').take(MAX_TEXT).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_text_drops_controls_and_bounds_length() {
        assert_eq!(clean_text("Hi\r\nBcc: x\u{1b}[31m"), "HiBcc: x[31m");
        assert_eq!(clean_text(&"a".repeat(5000)).chars().count(), MAX_TEXT);
    }
}
