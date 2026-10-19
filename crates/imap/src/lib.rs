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
    pub fn headers(&mut self, folder: &str, offset: u32, limit: u32) -> Result<Vec<RemoteHeader>> {
        let mb = self.select(folder)?;
        let Some((first, last)) = Self::fetch_range(mb.exists, offset, limit) else { return Ok(Vec::new()) };
        let fetches = self.session.fetch(format!("{first}:{last}"), header_query())?;
        Ok(fetches.iter().filter_map(header_of).collect())
    }

    /// The newest `limit` messages of `folder` that match every word of `query`, by the
    /// server's own `SEARCH`. Words are sent quoted; when the server refuses that (words beyond
    /// ASCII on a server without UTF-8 support) they go once more as UTF-8 literals.
    pub fn search(&mut self, folder: &str, query: &str, limit: u32) -> Result<Vec<RemoteHeader>> {
        let words: Vec<&str> = query.split_whitespace().filter(|w| !w.chars().any(|c| matches!(c, '\r' | '\n' | '\0'))).collect();
        if words.is_empty() {
            return Ok(Vec::new());
        }
        self.select(folder)?;
        let uids = match self.session.uid_search(search_keys(&words, false)?) {
            Ok(uids) => uids,
            Err(e) if words.iter().any(|w| !w.is_ascii()) => match self.session.uid_search(search_keys(&words, true)?) {
                Ok(uids) => uids,
                Err(_) => return Err(e.into()),
            },
            Err(e) => return Err(e.into()),
        };
        let mut uids: Vec<u32> = uids.into_iter().collect();
        uids.sort_unstable_by(|a, b| b.cmp(a));
        uids.truncate(limit.max(1) as usize);
        if uids.is_empty() {
            return Ok(Vec::new());
        }
        let set = uids.iter().map(u32::to_string).collect::<Vec<_>>().join(",");
        let fetches = self.session.uid_fetch(set, header_query())?;
        let mut out: Vec<RemoteHeader> = fetches.iter().filter_map(header_of).collect();
        out.sort_by_key(|h| h.id.parse::<u32>().unwrap_or(0));
        Ok(out)
    }

    /// The sequence numbers to fetch for the page `offset` messages below the newest, `limit`
    /// long: `None` when the folder has nothing there.
    fn fetch_range(exists: u32, offset: u32, limit: u32) -> Option<(u32, u32)> {
        let last = exists.checked_sub(offset).filter(|l| *l > 0)?;
        let first = last.saturating_sub(limit.max(1) - 1).max(1);
        Some((first, last))
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

    /// Sets the keywords for `categories` and takes those for `removed` off. A server that
    /// does not keep custom keywords (`PERMANENTFLAGS` without `\*`) refuses the `STORE`.
    pub fn set_categories(&mut self, folder: &str, uid: u32, categories: &[String], removed: &[String]) -> Result<()> {
        if self.selected.as_deref() != Some(folder) {
            self.select(folder)?;
        }
        let uid = uid.to_string();
        let keywords =
            |names: &[String]| names.iter().map(|n| mailcraft_model::category_keyword(n)).filter(|k| !k.is_empty()).collect::<Vec<_>>().join(" ");
        let off = keywords(removed);
        if !off.is_empty() {
            self.session.uid_store(&uid, format!("-FLAGS.SILENT ({off})"))?;
        }
        let on = keywords(categories);
        if !on.is_empty() {
            self.session.uid_store(&uid, format!("+FLAGS.SILENT ({on})"))?;
        }
        Ok(())
    }

    /// Removes one message for good: `\Deleted`, then expunged (the whole folder's deleted
    /// messages when the server lacks UIDPLUS).
    pub fn delete_message(&mut self, folder: &str, uid: u32) -> Result<()> {
        if self.selected.as_deref() != Some(folder) {
            self.select(folder)?;
        }
        let uid = uid.to_string();
        self.session.uid_store(&uid, "+FLAGS.SILENT (\\Deleted)")?;
        if self.session.uid_expunge(&uid).is_err() {
            self.session.expunge()?;
        }
        Ok(())
    }

    /// Removes every message in `folder` for good.
    pub fn empty_folder(&mut self, folder: &str) -> Result<()> {
        let mb = self.select(folder)?;
        if mb.exists == 0 {
            return Ok(());
        }
        self.session.store("1:*", "+FLAGS.SILENT (\\Deleted)")?;
        self.session.expunge()?;
        Ok(())
    }

    /// Files a sent message in `folder`, marked read.
    pub fn append(&mut self, folder: &str, message: &[u8]) -> Result<()> {
        let folder = quote(folder)?;
        self.session.append(&folder, message).flag(imap::types::Flag::Seen).finish()?;
        Ok(())
    }

    /// The character between a folder and its subfolders, as the server lists `folder` (the
    /// top level when empty); "/" when the server names none.
    fn delimiter(&mut self, folder: &str) -> Result<String> {
        let names = if folder.is_empty() { self.session.list(Some(""), Some("\"\""))? } else { self.session.list(None, Some(&quote(folder)?))? };
        Ok(names.iter().find_map(|n| n.delimiter()).unwrap_or("/").to_string())
    }

    /// Leaves `folder` when it is the selected one, so it can be renamed or removed.
    fn leave(&mut self, folder: &str) -> Result<()> {
        if self.selected.as_deref() == Some(folder) {
            self.selected = None;
            self.session.select("INBOX")?;
        }
        Ok(())
    }

    /// Makes `name` under `parent`, or at the top level, and subscribes to it for clients
    /// that list subscriptions only.
    pub fn create_folder(&mut self, parent: Option<&str>, name: &str) -> Result<()> {
        let delimiter = self.delimiter(parent.unwrap_or(""))?;
        if name.is_empty() || name.contains(&delimiter) || name.chars().any(|c| matches!(c, '\r' | '\n' | '\0' | '%' | '*')) {
            return Err(ImapError::BadName(name.chars().take(40).collect()));
        }
        let path = match parent {
            Some(p) => format!("{p}{delimiter}{name}"),
            None => name.to_string(),
        };
        self.session.create(&path)?;
        if let Err(e) = self.session.subscribe(&path) {
            log::debug!("could not subscribe to {path}: {e}");
        }
        Ok(())
    }

    /// Renames the last segment of `folder` to `name`; subfolders move along, as RFC 3501 has it.
    pub fn rename_folder(&mut self, folder: &str, name: &str) -> Result<()> {
        let delimiter = self.delimiter(folder)?;
        if name.is_empty() || name.contains(&delimiter) || name.chars().any(|c| matches!(c, '\r' | '\n' | '\0' | '%' | '*')) {
            return Err(ImapError::BadName(name.chars().take(40).collect()));
        }
        quote(folder)?;
        let path = match folder.rsplit_once(delimiter.as_str()) {
            Some((parent, _)) => format!("{parent}{delimiter}{name}"),
            None => name.to_string(),
        };
        self.leave(folder)?;
        self.session.rename(folder, &path)?;
        if let Err(e) = self.session.subscribe(&path) {
            log::debug!("could not subscribe to {path}: {e}");
        }
        Ok(())
    }

    /// Removes `folder` and the messages in it; subfolders stay, as the protocol has it.
    pub fn delete_folder(&mut self, folder: &str) -> Result<()> {
        quote(folder)?;
        self.leave(folder)?;
        if let Err(e) = self.session.unsubscribe(folder) {
            log::debug!("could not unsubscribe from {folder}: {e}");
        }
        self.session.delete(folder)?;
        Ok(())
    }

    pub fn logout(mut self) {
        let _ = self.session.logout();
    }
}

/// What a listing fetches per message: flags, size, structure and the envelope headers.
fn header_query() -> String {
    format!("(UID FLAGS INTERNALDATE RFC822.SIZE BODYSTRUCTURE {HEADER_FIELDS})")
}

/// One fetched message as a header; `None` for a reply without a UID.
fn header_of(f: &imap::types::Fetch) -> Option<RemoteHeader> {
    let uid = f.uid?;
    let header = mailcraft_mime::parse_header(f.header().unwrap_or(b""));
    let internal = f.internal_date().map(|d| d.timestamp());
    let date = header.date.or(internal).unwrap_or(0);
    let flags = f.flags();
    Some(RemoteHeader {
        id: uid.to_string(),
        seen: flags.contains(&imap::types::Flag::Seen),
        flagged: flags.contains(&imap::types::Flag::Flagged),
        answered: flags.contains(&imap::types::Flag::Answered),
        size: f.size.unwrap_or(0),
        has_attachments: f.bodystructure().is_some_and(has_attachment),
        categories: categories_of(flags),
        header,
        date,
    })
}

/// The keywords (custom flags) that stand for categories: everything a client may set, minus
/// the `$`-prefixed ones and the junk markers other clients use among themselves.
fn categories_of(flags: &[imap::types::Flag<'_>]) -> Vec<String> {
    flags
        .iter()
        .filter_map(|f| match f {
            imap::types::Flag::Custom(k) => Some(k.as_ref()),
            _ => None,
        })
        .filter(|k| !k.starts_with('$') && !["NonJunk", "Junk", "Forwarded", "Redirected"].iter().any(|s| s.eq_ignore_ascii_case(k)))
        .map(|k| k.replace('_', " "))
        .collect()
}

/// The `SEARCH` keys for `words`: a `TEXT` key per word, every word required. Quoted strings
/// by default; with `literals`, `CHARSET UTF-8` and a non-synchronising literal (RFC 7888)
/// per word, for servers that do not take UTF-8 inside quotes.
fn search_keys(words: &[&str], literals: bool) -> Result<String> {
    let mut out = String::new();
    if literals {
        out.push_str("CHARSET UTF-8");
    }
    for w in words {
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str("TEXT ");
        if literals {
            out.push_str(&format!("{{{}+}}\r\n{w}", w.len()));
        } else {
            out.push_str(&quote(w)?);
        }
    }
    Ok(out)
}

impl Mailbox for ImapClient {
    fn folders(&mut self) -> mailcraft_mailbox::Result<Vec<RemoteFolder>> {
        Ok(ImapClient::folders(self)?)
    }

    fn headers(&mut self, folder: &str, offset: u32, limit: u32) -> mailcraft_mailbox::Result<Vec<RemoteHeader>> {
        Ok(ImapClient::headers(self, folder, offset, limit)?)
    }

    fn search(&mut self, folder: &str, query: &str, limit: u32) -> mailcraft_mailbox::Result<Vec<RemoteHeader>> {
        Ok(ImapClient::search(self, folder, query, limit)?)
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

    fn create_folder(&mut self, parent: Option<&str>, name: &str) -> mailcraft_mailbox::Result<()> {
        Ok(ImapClient::create_folder(self, parent, name)?)
    }

    fn rename_folder(&mut self, folder: &str, name: &str) -> mailcraft_mailbox::Result<()> {
        Ok(ImapClient::rename_folder(self, folder, name)?)
    }

    fn delete_folder(&mut self, folder: &str) -> mailcraft_mailbox::Result<()> {
        Ok(ImapClient::delete_folder(self, folder)?)
    }

    fn delete_message(&mut self, folder: &str, id: &str) -> mailcraft_mailbox::Result<()> {
        Ok(ImapClient::delete_message(self, folder, uid_of(id)?)?)
    }

    fn empty_folder(&mut self, folder: &str) -> mailcraft_mailbox::Result<()> {
        Ok(ImapClient::empty_folder(self, folder)?)
    }

    fn set_categories(&mut self, folder: &str, id: &str, categories: &[String], removed: &[String]) -> mailcraft_mailbox::Result<()> {
        Ok(ImapClient::set_categories(self, folder, uid_of(id)?, categories, removed)?)
    }

    fn logout(self: Box<Self>) {
        ImapClient::logout(*self);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn search_keys_require_every_word_and_can_go_as_literals() {
        assert_eq!(search_keys(&["factuur", "2026"], false).unwrap(), "TEXT \"factuur\" TEXT \"2026\"");
        assert_eq!(search_keys(&["a\"b"], false).unwrap(), "TEXT \"a\\\"b\"");
        assert_eq!(search_keys(&["café", "x"], true).unwrap(), "CHARSET UTF-8 TEXT {5+}\r\ncafé TEXT {1+}\r\nx");
    }

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

    #[test]
    fn pages_count_down_from_the_newest_message() {
        assert_eq!(ImapClient::fetch_range(0, 0, 200), None);
        assert_eq!(ImapClient::fetch_range(5, 0, 200), Some((1, 5)));
        assert_eq!(ImapClient::fetch_range(450, 0, 200), Some((251, 450)));
        assert_eq!(ImapClient::fetch_range(450, 200, 200), Some((51, 250)));
        assert_eq!(ImapClient::fetch_range(450, 400, 200), Some((1, 50)));
        assert_eq!(ImapClient::fetch_range(450, 450, 200), None);
        assert_eq!(ImapClient::fetch_range(3, 1, 0), Some((2, 2)), "a limit of 0 still fetches one");
    }
}
