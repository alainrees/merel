//! A small, blocking IMAP client for one account: folders, headers, bodies, flags, moves.
//!
//! TLS is mandatory. Implicit TLS (port 993) or STARTTLS, both through rustls with the
//! platform's certificate verifier; there is no way to turn verification off. Mailbox names
//! are quoted before they go on the wire, so a hostile folder name cannot inject a command.
//!
//! The connection is opened here rather than by the imap crate, so that every step has a
//! limit: a server that does not answer, or answers on the wrong port, is an error after
//! [`CONNECT_TIMEOUT`] or [`READ_TIMEOUT`] instead of a wait without end.

use std::io::{self, Read, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use imap::extensions::idle::SetReadTimeout;
use imap_proto::{BodyStructure, NameAttribute};
pub use merel_mailbox::{Credential, Flag, Mailbox, MailboxError, RemoteFolder, RemoteHeader};
use merel_model::{FolderKind, Server, Tls};
use merel_secrets::Secret;

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

/// How long a server gets to accept the connection.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(15);
/// How long a server may stay silent while Merel waits for its answer.
pub const READ_TIMEOUT: Duration = Duration::from_secs(60);

#[derive(Debug, thiserror::Error)]
pub enum ImapError {
    /// The connection could not be opened or secured; `reason` is for the person, in Dutch.
    #[error("could not connect to {host}: {reason}")]
    Connect { host: String, reason: String },
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

fn is_timeout(e: &io::Error) -> bool {
    matches!(e.kind(), io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock)
}

/// What went wrong while opening a connection, in words for the person.
fn reason_of(e: &io::Error) -> String {
    if is_timeout(e) {
        return merel_i18n::tr("imap-no-answer-tls");
    }
    if let Some(tls) = e.get_ref().and_then(|inner| inner.downcast_ref::<rustls::Error>()) {
        return match tls {
            rustls::Error::InvalidCertificate(c) => merel_i18n::tr_args("imap-untrusted-cert", &[("detail", format!("{c:?}").into())]),
            other => merel_i18n::tr_args("imap-tls-failed-port", &[("detail", other.to_string().into())]),
        };
    }
    match e.kind() {
        io::ErrorKind::ConnectionRefused => merel_i18n::tr("imap-refused"),
        io::ErrorKind::InvalidData | io::ErrorKind::UnexpectedEof | io::ErrorKind::ConnectionReset | io::ErrorKind::ConnectionAborted => {
            merel_i18n::tr("imap-tls-dropped")
        }
        _ => e.to_string(),
    }
}

/// The reason inside an imap error raised while connecting.
fn connect_reason(e: &imap::Error) -> String {
    match e {
        imap::Error::Io(io) => reason_of(io),
        other => other.to_string(),
    }
}

impl From<ImapError> for MailboxError {
    fn from(e: ImapError) -> Self {
        match e {
            ImapError::Connect { host, reason } => MailboxError::Connect { host, reason },
            ImapError::Login => MailboxError::Login,
            ImapError::Protocol(e) => match *e {
                imap::Error::Io(io) if is_timeout(&io) => MailboxError::Protocol(merel_i18n::tr("imap-no-longer-answers")),
                e => MailboxError::Protocol(e.to_string()),
            },
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

/// A TLS connection over TCP, in a shape the imap crate can drive.
struct TlsStream(rustls::StreamOwned<rustls::ClientConnection, TcpStream>);

impl Read for TlsStream {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.0.read(buf)
    }
}

impl Write for TlsStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.write(buf)
    }
    fn flush(&mut self) -> io::Result<()> {
        self.0.flush()
    }
}

impl SetReadTimeout for TlsStream {
    fn set_read_timeout(&mut self, timeout: Option<Duration>) -> imap::Result<()> {
        self.0.sock.set_read_timeout(timeout).map_err(imap::Error::Io)
    }
}

/// A TCP connection to `host:port`, tried address by address, each within
/// [`CONNECT_TIMEOUT`]; reads and writes on it give up after `wait`.
fn open_tcp(host: &str, port: u16, wait: Duration) -> std::result::Result<TcpStream, String> {
    let addrs: Vec<_> = (host, port).to_socket_addrs().map_err(|_| merel_i18n::tr("imap-host-not-found"))?.collect();
    if addrs.is_empty() {
        return Err(merel_i18n::tr("imap-host-not-found"));
    }
    let mut last = None;
    for addr in addrs {
        match TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT) {
            Ok(tcp) => {
                let limits = tcp.set_read_timeout(Some(wait)).and_then(|()| tcp.set_write_timeout(Some(wait)));
                return limits.map(|()| tcp).map_err(|e| reason_of(&e));
            }
            Err(e) => last = Some(e),
        }
    }
    Err(match last {
        Some(e) if is_timeout(&e) => merel_i18n::tr_args("imap-timeout", &[("seconds", (CONNECT_TIMEOUT.as_secs() as usize).into())]),
        Some(e) => reason_of(&e),
        None => merel_i18n::tr("imap-host-not-found"),
    })
}

/// Wraps `tcp` in TLS for `host`, checked by the platform's certificate verifier. The
/// handshake itself happens on the first read.
fn tls(host: &str, tcp: TcpStream) -> std::result::Result<TlsStream, String> {
    use rustls_platform_verifier::BuilderVerifierExt;
    let provider = Arc::new(rustls::crypto::aws_lc_rs::default_provider());
    let config = rustls::ClientConfig::builder_with_provider(provider)
        .with_safe_default_protocol_versions()
        .and_then(|b| b.with_platform_verifier())
        .map_err(|e| merel_i18n::tr_args("imap-tls-setup", &[("detail", e.to_string().into())]))?
        .with_no_client_auth();
    let name = rustls::pki_types::ServerName::try_from(host.to_string()).map_err(|_| merel_i18n::tr("imap-host-invalid"))?;
    let conn =
        rustls::ClientConnection::new(Arc::new(config), name).map_err(|e| merel_i18n::tr_args("tls-failed", &[("detail", e.to_string().into())]))?;
    Ok(TlsStream(rustls::StreamOwned::new(conn, tcp)))
}

/// One line from the server, read a byte at a time so nothing after it (the TLS handshake)
/// is taken from the socket.
fn read_line(tcp: &mut TcpStream) -> std::result::Result<String, String> {
    let mut line = Vec::new();
    let mut byte = [0u8; 1];
    while line.len() < 8192 {
        match tcp.read(&mut byte) {
            Ok(0) => return Err(merel_i18n::tr("imap-dropped")),
            Ok(_) if byte[0] == b'\n' => return Ok(String::from_utf8_lossy(&line).trim_end().to_string()),
            Ok(_) => line.push(byte[0]),
            Err(e) => return Err(reason_of(&e)),
        }
    }
    Err(merel_i18n::tr("imap-not-imap"))
}

/// Reads the plain greeting and asks for `STARTTLS`; the connection is ready for TLS after.
fn starttls(tcp: &mut TcpStream) -> std::result::Result<(), String> {
    let greeting = read_line(tcp)?;
    // PREAUTH before TLS would mean a session nobody secured; it is refused like anything else.
    if !greeting.starts_with("* OK") {
        return Err(merel_i18n::tr("imap-not-imap"));
    }
    tcp.write_all(b"m0 STARTTLS\r\n").map_err(|e| reason_of(&e))?;
    loop {
        let line = read_line(tcp)?;
        if let Some(status) = line.strip_prefix("m0 ") {
            return if status.get(..2).is_some_and(|s| s.eq_ignore_ascii_case("OK")) { Ok(()) } else { Err(merel_i18n::tr("imap-no-starttls")) };
        }
    }
}

/// An IMAP client on a secured connection, greeting read: TLS from the first byte, or a plain
/// greeting and `STARTTLS` first. `wait` is how long the server may stay silent.
fn open(server: &Server, wait: Duration) -> Result<imap::Client<Conn>> {
    let host = server.host.as_str();
    let failed = |reason: String| ImapError::Connect { host: host.to_string(), reason };
    let tcp = open_tcp(host, server.port, wait).map_err(failed)?;
    let stream = match server.tls {
        Tls::Implicit => tls(host, tcp).map_err(failed)?,
        Tls::StartTls => {
            let mut tcp = tcp;
            starttls(&mut tcp).map_err(failed)?;
            tls(host, tcp).map_err(failed)?
        }
    };
    let mut client = imap::Client::new(Box::new(stream) as Conn);
    if server.tls == Tls::Implicit {
        client.read_greeting().map_err(|e| failed(connect_reason(&e)))?;
    }
    Ok(client)
}

pub struct ImapClient {
    session: imap::Session<Conn>,
    selected: Option<String>,
}

const HEADER_FIELDS: &str =
    "BODY.PEEK[HEADER.FIELDS (FROM TO CC SUBJECT DATE MESSAGE-ID CONTENT-TYPE CHAT-VERSION CHAT-GROUP-ID CHAT-GROUP-NAME SECURE-JOIN)]";

/// A mailbox name as it may go to the imap crate, which quotes it itself (SELECT, MOVE,
/// CREATE, RENAME, DELETE, SUBSCRIBE). CR, LF and NUL can never be quoted, so they are refused.
fn checked(name: &str) -> Result<&str> {
    if name.chars().any(|c| matches!(c, '\r' | '\n' | '\0')) {
        return Err(ImapError::BadName(name.chars().take(40).collect()));
    }
    Ok(name)
}

/// The inside of an IMAP quoted string: `"` and `\` escaped. For APPEND, which the imap crate
/// puts between quotes without escaping.
fn escaped(name: &str) -> Result<String> {
    let mut out = String::with_capacity(name.len());
    for c in checked(name)?.chars() {
        if c == '"' || c == '\\' {
            out.push('\\');
        }
        out.push(c);
    }
    Ok(out)
}

/// A mailbox name as an IMAP quoted string, for the commands the imap crate passes on as they
/// are (LIST patterns, COPY).
fn quote(name: &str) -> Result<String> {
    Ok(format!("\"{}\"", escaped(name)?))
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
    merel_mailbox::kind_from_name(name)
}

fn is_special_use(a: &NameAttribute<'_>) -> bool {
    matches!(a, NameAttribute::Sent | NameAttribute::Drafts | NameAttribute::Junk | NameAttribute::Trash | NameAttribute::Archive)
}

/// Each special folder once: when two folders would both be the junk folder (a `\Junk`
/// "Junk" and a "spam" known only by its name, say), the one the server marks wins, else the
/// first listed, and the others keep their own names as ordinary folders. `bool` is whether the
/// server marked the folder with a SPECIAL-USE attribute.
fn one_per_kind(listed: Vec<(RemoteFolder, bool)>) -> Vec<RemoteFolder> {
    let winner = |kind: FolderKind| {
        let all: Vec<(usize, bool)> =
            listed.iter().enumerate().filter(|(_, (f, _))| f.kind == kind && f.selectable).map(|(i, (_, marked))| (i, *marked)).collect();
        all.iter().find(|(_, marked)| *marked).or(all.first()).map(|(i, _)| *i)
    };
    listed
        .iter()
        .enumerate()
        .map(|(i, (f, _))| {
            let mut f = f.clone();
            if f.kind != FolderKind::Other && winner(f.kind) != Some(i) {
                f.kind = FolderKind::Other;
            }
            f
        })
        .collect()
}

/// The parent path and the own name of mailbox `path`, split at the server's hierarchy
/// delimiter; without one (a flat server) the whole path is the name.
fn split_path<'a>(path: &'a str, delimiter: Option<&str>) -> (Option<&'a str>, &'a str) {
    match delimiter.filter(|d| !d.is_empty()).and_then(|d| path.rsplit_once(d)) {
        Some((parent, name)) if !parent.is_empty() && !name.is_empty() => (Some(parent), name),
        _ => (None, path),
    }
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
        let client = open(server, READ_TIMEOUT)?;
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
        let listed: Vec<(RemoteFolder, bool)> = names
            .iter()
            .map(|n| {
                let (parent, name) = split_path(n.name(), n.delimiter());
                let folder = RemoteFolder {
                    path: n.name().to_string(),
                    name: name.to_string(),
                    kind: kind_of(n.name(), n.attributes()),
                    selectable: !n.attributes().iter().any(|a| matches!(a, NameAttribute::NoSelect)),
                    parent: parent.map(str::to_string),
                };
                (folder, n.attributes().iter().any(is_special_use))
            })
            .collect();
        let mut out = one_per_kind(listed);
        if !out.iter().any(|f| f.kind == FolderKind::Inbox) {
            out.insert(0, RemoteFolder { path: "INBOX".into(), name: "INBOX".into(), kind: FolderKind::Inbox, selectable: true, parent: None });
        }
        Ok(out)
    }

    fn select(&mut self, folder: &str) -> Result<imap::types::Mailbox> {
        let mb = self.session.select(checked(folder)?)?;
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
        let uid = uid.to_string();
        // MOVE quotes the name itself; COPY sends it as given.
        if self.session.uid_mv(&uid, checked(dest)?).is_ok() {
            return Ok(());
        }
        self.session.uid_copy(&uid, quote(dest)?)?;
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
            |names: &[String]| names.iter().map(|n| merel_model::category_keyword(n)).filter(|k| !k.is_empty()).collect::<Vec<_>>().join(" ");
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

    /// Sets the junk keywords other clients and some servers' filters read (RFC 5788's `$Junk`
    /// and `$NotJunk`, and Thunderbird's `Junk` and `NonJunk`): the one pair on, the other
    /// off. A server without custom keywords refuses the `STORE`.
    pub fn mark_junk(&mut self, folder: &str, uid: u32, junk: bool) -> Result<()> {
        if self.selected.as_deref() != Some(folder) {
            self.select(folder)?;
        }
        let uid = uid.to_string();
        let (on, off) = if junk { ("$Junk Junk", "$NotJunk NonJunk") } else { ("$NotJunk NonJunk", "$Junk Junk") };
        self.session.uid_store(&uid, format!("-FLAGS.SILENT ({off})"))?;
        self.session.uid_store(&uid, format!("+FLAGS.SILENT ({on})"))?;
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
        let folder = escaped(folder)?;
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
        checked(folder)?;
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
        checked(folder)?;
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
    let header = merel_mime::parse_header(f.header().unwrap_or(b""));
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
    fn folders(&mut self) -> merel_mailbox::Result<Vec<RemoteFolder>> {
        Ok(ImapClient::folders(self)?)
    }

    fn headers(&mut self, folder: &str, offset: u32, limit: u32) -> merel_mailbox::Result<Vec<RemoteHeader>> {
        Ok(ImapClient::headers(self, folder, offset, limit)?)
    }

    fn search(&mut self, folder: &str, query: &str, limit: u32) -> merel_mailbox::Result<Vec<RemoteHeader>> {
        Ok(ImapClient::search(self, folder, query, limit)?)
    }

    fn body(&mut self, folder: &str, id: &str) -> merel_mailbox::Result<Vec<u8>> {
        Ok(ImapClient::body(self, folder, uid_of(id)?)?)
    }

    fn set_flag(&mut self, folder: &str, id: &str, flag: Flag, on: bool) -> merel_mailbox::Result<()> {
        Ok(ImapClient::set_flag(self, folder, uid_of(id)?, flag, on)?)
    }

    fn move_to(&mut self, folder: &str, id: &str, dest: &str) -> merel_mailbox::Result<()> {
        Ok(ImapClient::move_to(self, folder, uid_of(id)?, dest)?)
    }

    fn mark_junk(&mut self, folder: &str, id: &str, junk: bool) -> merel_mailbox::Result<()> {
        Ok(ImapClient::mark_junk(self, folder, uid_of(id)?, junk)?)
    }

    fn append(&mut self, folder: &str, message: &[u8]) -> merel_mailbox::Result<()> {
        Ok(ImapClient::append(self, folder, message)?)
    }

    fn send(&mut self, _message: &[u8]) -> merel_mailbox::Result<bool> {
        Ok(false)
    }

    fn create_folder(&mut self, parent: Option<&str>, name: &str) -> merel_mailbox::Result<()> {
        Ok(ImapClient::create_folder(self, parent, name)?)
    }

    fn rename_folder(&mut self, folder: &str, name: &str) -> merel_mailbox::Result<()> {
        Ok(ImapClient::rename_folder(self, folder, name)?)
    }

    fn delete_folder(&mut self, folder: &str) -> merel_mailbox::Result<()> {
        Ok(ImapClient::delete_folder(self, folder)?)
    }

    fn delete_message(&mut self, folder: &str, id: &str) -> merel_mailbox::Result<()> {
        Ok(ImapClient::delete_message(self, folder, uid_of(id)?)?)
    }

    fn empty_folder(&mut self, folder: &str) -> merel_mailbox::Result<()> {
        Ok(ImapClient::empty_folder(self, folder)?)
    }

    fn set_categories(&mut self, folder: &str, id: &str, categories: &[String], removed: &[String]) -> merel_mailbox::Result<()> {
        Ok(ImapClient::set_categories(self, folder, uid_of(id)?, categories, removed)?)
    }

    fn logout(self: Box<Self>) {
        ImapClient::logout(*self);
    }
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::time::Instant;

    /// A server on a free local port that runs `serve` on the first connection.
    fn server(serve: impl FnOnce(std::net::TcpStream) + Send + 'static) -> u16 {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        std::thread::spawn(move || {
            if let Ok((stream, _)) = listener.accept() {
                serve(stream);
            }
        });
        port
    }

    fn reason(tls: Tls, port: u16) -> String {
        let started = Instant::now();
        let at = Server { host: "localhost".into(), port, tls };
        let Err(e) = open(&at, Duration::from_millis(500)) else { panic!("connected to a server that should fail") };
        assert!(started.elapsed() < Duration::from_secs(10), "took {:?}", started.elapsed());
        match e {
            ImapError::Connect { host, reason } => {
                assert_eq!(host, "localhost");
                reason
            }
            other => panic!("expected a connection error, got {other}"),
        }
    }

    #[test]
    fn a_silent_server_is_an_error_not_a_wait() {
        // STARTTLS on a TLS port: both sides wait for the other to speak first.
        let port = server(|s| {
            std::thread::sleep(Duration::from_secs(3));
            drop(s);
        });
        assert!(reason(Tls::StartTls, port).contains("reageert niet"));
        // TLS to a server that takes the hello and never answers.
        let port = server(|mut s| {
            let _ = s.read(&mut [0u8; 512]);
            std::thread::sleep(Duration::from_secs(3));
        });
        assert!(reason(Tls::Implicit, port).contains("reageert niet"));
    }

    #[test]
    fn plain_imap_on_a_tls_connection_says_so() {
        let port = server(|mut s| {
            let _ = s.write_all(b"* OK IMAP4rev1 ready\r\n");
            let _ = s.read(&mut [0u8; 512]);
        });
        let why = reason(Tls::Implicit, port);
        assert!(why.contains("TLS en STARTTLS"), "{why}");
    }

    #[test]
    fn a_closed_port_and_an_unknown_name_are_told_apart() {
        let port = TcpListener::bind("127.0.0.1:0").unwrap().local_addr().unwrap().port();
        assert!(reason(Tls::Implicit, port).contains("weigert"));
        let at = Server { host: "bestaat-niet.invalid".into(), port: 993, tls: Tls::Implicit };
        match open(&at, Duration::from_millis(500)) {
            Err(ImapError::Connect { reason, .. }) => assert!(reason.contains("niet gevonden"), "{reason}"),
            other => panic!("expected a connection error, got {:?}", other.err()),
        }
    }

    #[test]
    fn a_server_refusing_starttls_says_so() {
        let port = server(|mut s| {
            let _ = s.write_all(b"* OK ready\r\n");
            let mut buf = [0u8; 512];
            let n = s.read(&mut buf).unwrap_or(0);
            let tag = String::from_utf8_lossy(&buf[..n]).split(' ').next().unwrap_or("a1").to_string();
            let _ = s.write_all(format!("{tag} BAD STARTTLS not supported\r\n").as_bytes());
            std::thread::sleep(Duration::from_millis(500));
        });
        assert!(reason(Tls::StartTls, port).contains("STARTTLS"));
    }

    use super::*;

    #[test]
    fn search_keys_require_every_word_and_can_go_as_literals() {
        assert_eq!(search_keys(&["factuur", "2026"], false).unwrap(), "TEXT \"factuur\" TEXT \"2026\"");
        assert_eq!(search_keys(&["a\"b"], false).unwrap(), "TEXT \"a\\\"b\"");
        assert_eq!(search_keys(&["café", "x"], true).unwrap(), "CHARSET UTF-8 TEXT {5+}\r\ncafé TEXT {1+}\r\nx");
    }

    /// A logged-in client on a fake server that answers every command with OK (and `+` to a
    /// literal) and hands back the lines it was sent.
    fn recording() -> (ImapClient, std::sync::mpsc::Receiver<String>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let port = server(move |mut s| {
            let _ = s.write_all(b"* OK ready\r\n");
            let mut pending = Vec::new();
            let mut buf = [0u8; 4096];
            while let Ok(n) = s.read(&mut buf) {
                if n == 0 {
                    break;
                }
                pending.extend_from_slice(&buf[..n]);
                while let Some(end) = pending.windows(2).position(|w| w == b"\r\n") {
                    let line = String::from_utf8_lossy(&pending[..end]).to_string();
                    pending.drain(..end + 2);
                    let reply = if line.ends_with('}') {
                        "+ go\r\n".to_string()
                    } else if let Some((tag, _)) = line.split_once(' ').filter(|(t, _)| t.starts_with('a')) {
                        format!("{tag} OK done\r\n")
                    } else {
                        // The message itself after a literal: the APPEND's tag was on its first line.
                        String::new()
                    };
                    let _ = tx.send(line);
                    let _ = s.write_all(reply.as_bytes());
                }
            }
        });
        let tcp = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
        // The fake server never ends an APPEND; its reply is not what the tests look at.
        tcp.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let mut client = imap::Client::new(Box::new(tcp) as Conn);
        client.read_greeting().unwrap();
        let session = client.login("a", "b").map_err(|(e, _)| e).unwrap();
        (ImapClient { session, selected: None }, rx)
    }

    #[test]
    fn a_mailbox_path_splits_into_its_parent_and_its_own_name() {
        assert_eq!(split_path("INBOX.HUIS", Some(".")), (Some("INBOX"), "HUIS"));
        assert_eq!(split_path("Prive/Florence/2026", Some("/")), (Some("Prive/Florence"), "2026"));
        assert_eq!(split_path("Gem. Ass", Some("/")), (None, "Gem. Ass"), "a dot is no delimiter here");
        assert_eq!(split_path("INBOX", None), (None, "INBOX"));
    }

    #[test]
    fn two_junk_folders_become_one_junk_folder_and_one_named_folder() {
        let f = |path: &str, kind| RemoteFolder {
            path: path.into(),
            name: path.rsplit('.').next().unwrap().into(),
            kind,
            selectable: true,
            parent: None,
        };
        let listed = vec![
            (f("INBOX", FolderKind::Inbox), false),
            (f("INBOX.spam", FolderKind::Junk), false),
            (f("INBOX.Junk", FolderKind::Junk), true),
            (f("INBOX.Trash", FolderKind::Deleted), false),
            (f("Trash", FolderKind::Deleted), false),
        ];
        let kinds: Vec<(String, FolderKind)> = one_per_kind(listed).into_iter().map(|f| (f.path, f.kind)).collect();
        assert_eq!(
            kinds,
            vec![
                ("INBOX".into(), FolderKind::Inbox),
                ("INBOX.spam".into(), FolderKind::Other),
                ("INBOX.Junk".into(), FolderKind::Junk),
                ("INBOX.Trash".into(), FolderKind::Deleted),
                ("Trash".into(), FolderKind::Other),
            ],
            "the marked one wins, else the first"
        );
    }

    #[test]
    fn mailbox_names_are_quoted_once() {
        let (mut c, rx) = recording();
        c.select("Trash").unwrap();
        c.move_to("INBOX", 7, "INBOX.Trash").unwrap();
        let _ = c.append("INBOX.Sent", b"Subject: x\r\n\r\nhoi\r\n");
        let sent: Vec<String> = rx.try_iter().collect();
        let has = |want: &str| sent.iter().any(|l| l.ends_with(want));
        assert!(has("SELECT \"Trash\""), "{sent:?}");
        assert!(has("UID MOVE 7 \"INBOX.Trash\""), "{sent:?}");
        assert!(sent.iter().any(|l| l.contains("APPEND \"INBOX.Sent\" ")), "{sent:?}");
        assert!(!sent.iter().any(|l| l.contains("\"\\\"")), "nothing is quoted twice: {sent:?}");
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
    //! Needs the network: `cargo test -p merel-imap -- --ignored`.
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
