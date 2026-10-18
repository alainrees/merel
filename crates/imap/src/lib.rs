//! A small, blocking IMAP client for one account: folders, headers, bodies, flags, moves.
//!
//! TLS is mandatory. Implicit TLS (port 993) or STARTTLS, both through rustls with the
//! platform's certificate verifier; there is no way to turn verification off. Mailbox names
//! are quoted before they go on the wire, so a hostile folder name cannot inject a command.

use imap_proto::{BodyStructure, NameAttribute};
pub use mailcraft_mailbox::{Credential, Flag, Mailbox, MailboxError, RemoteFolder, RemoteHeader};
use mailcraft_model::{FolderKind, Server, Tls};
use mailcraft_secrets::Secret;

struct XOAuth2<'a> {
    user: &'a str,
    token: &'a Secret,
}

impl imap::Authenticator for XOAuth2<'_> {
    type Response = String;
    fn process(&self, _challenge: &[u8]) -> String {
        format!("user={}\x01auth=Bearer {}\x01\x01", self.user, self.token.expose())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ImapError {
    #[error("could not connect to {host}: {source}")]
    Connect { host: String, source: Box<imap::Error> },
    #[error("the server refused the user name or password")]
    Login,
    #[error("IMAP error: {0}")]
    Protocol(Box<imap::Error>),
    #[error("mailbox name is not allowed: {0:?}")]
    BadName(String),
}

impl From<imap::Error> for ImapError {
    fn from(e: imap::Error) -> Self {
        ImapError::Protocol(Box::new(e))
    }
}

impl From<ImapError> for MailboxError {
    fn from(e: ImapError) -> Self {
        match e {
            ImapError::Connect { host, source } => MailboxError::Connect { host, reason: source.to_string() },
            ImapError::Login => MailboxError::Login,
            ImapError::Protocol(e) => MailboxError::Protocol(e.to_string()),
            ImapError::BadName(n) => MailboxError::BadName(n),
        }
    }
}

pub type Result<T> = std::result::Result<T, ImapError>;

/// An item id as the engine passes it back, which for IMAP is the UID in decimal.
fn uid_of(id: &str) -> Result<u32> {
    id.parse::<u32>().ok().filter(|u| *u > 0).ok_or_else(|| ImapError::BadName(id.chars().take(40).collect()))
}

type Conn = imap::Connection;

pub struct ImapClient {
    session: imap::Session<Conn>,
    selected: Option<String>,
}

const HEADER_FIELDS: &str = "BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID)]";

/// A mailbox name as an IMAP quoted string. CR, LF and NUL can never be quoted, so they are
/// refused outright.
fn quote(name: &str) -> Result<String> {
    if name.chars().any(|c| matches!(c, '\r' | '\n' | '\0')) {
        return Err(ImapError::BadName(name.chars().take(40).collect()));
    }
    let mut out = String::with_capacity(name.len() + 2);
    out.push('"');
    for c in name.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    out.push('"');
    Ok(out)
}

/// Which special folder a mailbox is, from its SPECIAL-USE attributes, else from its name.
fn kind_of(name: &str, attrs: &[NameAttribute<'_>]) -> FolderKind {
    for a in attrs {
        match a {
            NameAttribute::Sent => return FolderKind::Sent,
            NameAttribute::Drafts => return FolderKind::Drafts,
            NameAttribute::Junk => return FolderKind::Junk,
            NameAttribute::Trash => return FolderKind::Deleted,
            NameAttribute::Archive => return FolderKind::Archive,
            _ => {}
        }
    }
    mailcraft_mailbox::kind_from_name(name)
}

/// Whether a BODYSTRUCTURE holds anything a person would call an attachment.
fn has_attachment(b: &BodyStructure<'_>) -> bool {
    let disposition_attachment =
        |common: &imap_proto::BodyContentCommon<'_>| common.disposition.as_ref().is_some_and(|d| d.ty.eq_ignore_ascii_case("attachment"));
    match b {
        BodyStructure::Multipart { bodies, .. } => bodies.iter().any(has_attachment),
        BodyStructure::Text { common, .. } => disposition_attachment(common),
        BodyStructure::Basic { common, .. } => disposition_attachment(common) || !common.ty.ty.eq_ignore_ascii_case("text"),
        BodyStructure::Message { .. } => true,
    }
}

impl ImapClient {
    /// Connects, negotiates TLS and logs in.
    pub fn connect(server: &Server, username: &str, credential: &Credential) -> Result<Self> {
        let mode = match server.tls {
            Tls::Implicit => imap::ConnectionMode::Tls,
            Tls::StartTls => imap::ConnectionMode::StartTls,
        };
        let client = imap::ClientBuilder::new(server.host.as_str(), server.port)
            .mode(mode)
            .connect()
            .map_err(|source| ImapError::Connect { host: server.host.clone(), source: Box::new(source) })?;
        let refused = |(e, _): (imap::Error, imap::Client<Conn>)| match e {
            imap::Error::No(_) | imap::Error::Bad(_) => ImapError::Login,
            e => ImapError::Protocol(Box::new(e)),
        };
        let session = match credential {
            Credential::Password(p) => client.login(username, p.expose()).map_err(refused)?,
            Credential::OAuth2(token) => client.authenticate("XOAUTH2", &XOAuth2 { user: username, token }).map_err(refused)?,
        };
        Ok(ImapClient { session, selected: None })
    }

    pub fn folders(&mut self) -> Result<Vec<RemoteFolder>> {
        let names = self.session.list(None, Some("*"))?;
        let mut out: Vec<RemoteFolder> = names
            .iter()
            .map(|n| RemoteFolder {
                path: n.name().to_string(),
                name: n.name().rsplit(['/', '.']).next().unwrap_or(n.name()).to_string(),
                kind: kind_of(n.name(), n.attributes()),
                selectable: !n.attributes().iter().any(|a| matches!(a, NameAttribute::NoSelect)),
            })
            .collect();
        if !out.iter().any(|f| f.kind == FolderKind::Inbox) {
            out.insert(0, RemoteFolder { path: "INBOX".into(), name: "INBOX".into(), kind: FolderKind::Inbox, selectable: true });
        }
        Ok(out)
    }

    fn select(&mut self, folder: &str) -> Result<imap::types::Mailbox> {
        let mb = self.session.select(quote(folder)?)?;
        self.selected = Some(folder.to_string());
        Ok(mb)
    }

    /// The newest `limit` messages of `folder`, newest last.
    pub fn headers(&mut self, folder: &str, limit: u32) -> Result<Vec<RemoteHeader>> {
        let mb = self.select(folder)?;
        if mb.exists == 0 {
            return Ok(Vec::new());
        }
        let first = mb.exists.saturating_sub(limit.max(1) - 1).max(1);
        let query = format!("(UID FLAGS INTERNALDATE RFC822.SIZE BODYSTRUCTURE {HEADER_FIELDS})");
        let fetches = self.session.fetch(format!("{first}:*"), query)?;
        let mut out = Vec::with_capacity(fetches.len());
        for f in fetches.iter() {
            let Some(uid) = f.uid else { continue };
            let header = mailcraft_mime::parse_header(f.header().unwrap_or(b""));
            let internal = f.internal_date().map(|d| d.timestamp());
            let date = header.date.or(internal).unwrap_or(0);
            let flags = f.flags();
            out.push(RemoteHeader {
                id: uid.to_string(),
                seen: flags.contains(&imap::types::Flag::Seen),
                flagged: flags.contains(&imap::types::Flag::Flagged),
                answered: flags.contains(&imap::types::Flag::Answered),
                size: f.size.unwrap_or(0),
                has_attachments: f.bodystructure().is_some_and(has_attachment),
                header,
                date,
            });
        }
        Ok(out)
    }

    /// The whole raw message, without setting \Seen.
    pub fn body(&mut self, folder: &str, uid: u32) -> Result<Vec<u8>> {
        if self.selected.as_deref() != Some(folder) {
            self.select(folder)?;
        }
        let fetches = self.session.uid_fetch(uid.to_string(), "BODY.PEEK[]")?;
        Ok(fetches.iter().find_map(|f| f.body().map(<[u8]>::to_vec)).unwrap_or_default())
    }

    /// Sets or clears `\Seen` or `\Flagged`.
    pub fn set_flag(&mut self, folder: &str, uid: u32, flag: Flag, on: bool) -> Result<()> {
        let flag = match flag {
            Flag::Seen => "\\Seen",
            Flag::Flagged => "\\Flagged",
        };
        if self.selected.as_deref() != Some(folder) {
            self.select(folder)?;
        }
        let op = if on { "+FLAGS.SILENT" } else { "-FLAGS.SILENT" };
        self.session.uid_store(uid.to_string(), format!("{op} ({flag})"))?;
        Ok(())
    }

    /// Moves a message; falls back to copy, delete and expunge when the server lacks MOVE.
    pub fn move_to(&mut self, folder: &str, uid: u32, dest: &str) -> Result<()> {
        if self.selected.as_deref() != Some(folder) {
            self.select(folder)?;
        }
        let dest = quote(dest)?;
        let uid = uid.to_string();
        if self.session.uid_mv(&uid, &dest).is_ok() {
            return Ok(());
        }
        self.session.uid_copy(&uid, &dest)?;
        self.session.uid_store(&uid, "+FLAGS.SILENT (\\Deleted)")?;
        self.session.uid_expunge(&uid)?;
        Ok(())
    }

    /// Files a sent message in `folder`, marked read.
    pub fn append(&mut self, folder: &str, message: &[u8]) -> Result<()> {
        let folder = quote(folder)?;
        self.session.append(&folder, message).flag(imap::types::Flag::Seen).finish()?;
        Ok(())
    }

    pub fn logout(mut self) {
        let _ = self.session.logout();
    }
}

impl Mailbox for ImapClient {
    fn folders(&mut self) -> mailcraft_mailbox::Result<Vec<RemoteFolder>> {
        Ok(ImapClient::folders(self)?)
    }

    fn headers(&mut self, folder: &str, limit: u32) -> mailcraft_mailbox::Result<Vec<RemoteHeader>> {
        Ok(ImapClient::headers(self, folder, limit)?)
    }

    fn body(&mut self, folder: &str, id: &str) -> mailcraft_mailbox::Result<Vec<u8>> {
        Ok(ImapClient::body(self, folder, uid_of(id)?)?)
    }

    fn set_flag(&mut self, folder: &str, id: &str, flag: Flag, on: bool) -> mailcraft_mailbox::Result<()> {
        Ok(ImapClient::set_flag(self, folder, uid_of(id)?, flag, on)?)
    }

    fn move_to(&mut self, folder: &str, id: &str, dest: &str) -> mailcraft_mailbox::Result<()> {
        Ok(ImapClient::move_to(self, folder, uid_of(id)?, dest)?)
    }

    fn append(&mut self, folder: &str, message: &[u8]) -> mailcraft_mailbox::Result<()> {
        Ok(ImapClient::append(self, folder, message)?)
    }

    fn send(&mut self, _message: &[u8]) -> mailcraft_mailbox::Result<bool> {
        Ok(false)
    }

    fn logout(self: Box<Self>) {
        ImapClient::logout(*self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quoting_escapes_and_refuses_line_breaks() {
        assert_eq!(quote("INBOX").unwrap(), "\"INBOX\"");
        assert_eq!(quote("a\"b\\c").unwrap(), "\"a\\\"b\\\\c\"");
        assert!(quote("INBOX\r\nA1 DELETE INBOX").is_err());
    }

    #[test]
    fn folder_kinds_from_names() {
        assert_eq!(kind_of("INBOX", &[]), FolderKind::Inbox);
        assert_eq!(kind_of("INBOX/Verzonden items", &[]), FolderKind::Sent);
        assert_eq!(kind_of("Projecten", &[]), FolderKind::Other);
        assert_eq!(kind_of("Weird", &[NameAttribute::Trash]), FolderKind::Deleted);
    }
}

#[cfg(test)]
mod network_tests {
    //! Needs the network: `cargo test -p mailcraft-imap -- --ignored`.
    use super::*;

    #[test]
    #[ignore]
    fn tls_handshake_and_login_refusal_against_a_public_server() {
        let server = Server { host: "imap.gmail.com".into(), port: 993, tls: Tls::Implicit };
        let r = ImapClient::connect(&server, "nobody@example.org", &Credential::Password(Secret::new("wrong")));
        match r {
            Err(ImapError::Login) => {}
            Err(e) => panic!("expected a login refusal, got {e}"),
            Ok(_) => panic!("logged in with a bogus password"),
        }
    }
}
