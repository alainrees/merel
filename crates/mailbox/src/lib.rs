//! What every mail store looks like to the engine: folders, headers, bodies, flags, moves.
//!
//! IMAP and Exchange Web Services each implement [`Mailbox`]; the engine's worker holds a
//! `Box<dyn Mailbox>` and never knows which. Message and folder ids are opaque strings (an
//! IMAP UID, an EWS item id) that only the store that produced them interprets.

use mailcraft_mime::ParsedHeader;
use mailcraft_model::FolderKind;
use mailcraft_secrets::Secret;

/// How to log in.
#[derive(Clone, Debug)]
pub enum Credential {
    Password(Secret),
    /// An OAuth 2.0 access token (SASL XOAUTH2 on IMAP/SMTP, a Bearer token on EWS).
    OAuth2(Secret),
}

#[derive(Debug, thiserror::Error)]
pub enum MailboxError {
    #[error("could not connect to {host}: {reason}")]
    Connect { host: String, reason: String },
    #[error("the server refused the user name or password")]
    Login,
    /// The server wants an authentication scheme Mailcraft does not speak (NTLM, Negotiate).
    #[error("the server requires {0} authentication")]
    AuthScheme(String),
    #[error("the server reported an error: {0}")]
    Protocol(String),
    #[error("the name is not allowed: {0:?}")]
    BadName(String),
}

pub type Result<T> = std::result::Result<T, MailboxError>;

/// A folder as the server lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteFolder {
    /// What the store needs to open it: the IMAP mailbox name, the EWS folder id.
    pub path: String,
    /// What a person sees: the last path segment, the display name.
    pub name: String,
    pub kind: FolderKind,
    pub selectable: bool,
}

/// A message's envelope as the server lists it.
#[derive(Clone, Debug)]
pub struct RemoteHeader {
    /// The IMAP UID or the EWS item id, as a string.
    pub id: String,
    pub seen: bool,
    pub flagged: bool,
    pub answered: bool,
    pub size: u32,
    pub has_attachments: bool,
    pub header: ParsedHeader,
    /// Seconds since the epoch: the Date header, else when the server received it.
    pub date: i64,
}

/// A flag a person can set on a message.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Flag {
    Seen,
    Flagged,
}

/// A connected mail store. Every method may fail on a dropped connection; the worker then
/// reconnects once and retries.
pub trait Mailbox: Send {
    fn folders(&mut self) -> Result<Vec<RemoteFolder>>;
    /// The newest `limit` messages of `folder`, newest last.
    fn headers(&mut self, folder: &str, limit: u32) -> Result<Vec<RemoteHeader>>;
    /// The whole raw message, without marking it read.
    fn body(&mut self, folder: &str, id: &str) -> Result<Vec<u8>>;
    fn set_flag(&mut self, folder: &str, id: &str, flag: Flag, on: bool) -> Result<()>;
    fn move_to(&mut self, folder: &str, id: &str, dest: &str) -> Result<()>;
    /// Files a message that went out over SMTP, marked read.
    fn append(&mut self, folder: &str, message: &[u8]) -> Result<()>;
    /// Sends `message` through the store itself and files the copy; `Ok(false)` when this
    /// store does not send and the message has to go over SMTP.
    fn send(&mut self, message: &[u8]) -> Result<bool>;
    fn logout(self: Box<Self>);
}

/// Which special folder a name denotes, in English or Dutch; `Other` when it is an ordinary one.
pub fn kind_from_name(name: &str) -> FolderKind {
    let last = name.rsplit(['/', '.']).next().unwrap_or(name).to_ascii_lowercase();
    match last.as_str() {
        "inbox" | "postvak in" => FolderKind::Inbox,
        "sent" | "sent items" | "sent messages" | "verzonden items" | "verzonden" => FolderKind::Sent,
        "drafts" | "concepten" => FolderKind::Drafts,
        "junk" | "spam" | "junk e-mail" | "junk email" | "ongewenste e-mail" => FolderKind::Junk,
        "trash" | "deleted items" | "deleted messages" | "verwijderde items" | "prullenbak" => FolderKind::Deleted,
        "archive" | "archief" => FolderKind::Archive,
        _ => FolderKind::Other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_map_to_kinds() {
        assert_eq!(kind_from_name("INBOX"), FolderKind::Inbox);
        assert_eq!(kind_from_name("INBOX/Verzonden items"), FolderKind::Sent);
        assert_eq!(kind_from_name("Projecten"), FolderKind::Other);
    }
}
